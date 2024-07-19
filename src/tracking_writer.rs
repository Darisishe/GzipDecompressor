#![forbid(unsafe_code)]

use std::{
    cmp::min,
    collections::VecDeque,
    io::{self, Write},
};

use anyhow::{bail, Context, Result};
use crc::{Crc, Digest};

////////////////////////////////////////////////////////////////////////////////

const HISTORY_SIZE: usize = 32768;
static CRC_ALGORITHM: Crc<u32> = Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);

pub struct TrackingWriter<T> {
    inner: T,
    history: VecDeque<u8>,
    digest: Digest<'static, u32>,
    byte_count: usize,
}

impl<T: Write> Write for TrackingWriter<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;

        self.history.extend(&buf[..written]);
        if self.history.len() > HISTORY_SIZE {
            self.history.drain(..(self.history.len() - HISTORY_SIZE));
        }

        self.digest.update(&buf[..written]);
        self.byte_count += written;

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: Write> TrackingWriter<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            history: VecDeque::<u8>::with_capacity(HISTORY_SIZE),
            digest: CRC_ALGORITHM.digest(),
            byte_count: 0,
        }
    }

    /// Write a sequence of `len` bytes written `dist` bytes ago.
    pub fn write_previous(&mut self, dist: usize, len: usize) -> Result<()> {
        if dist > self.history.len() || len == 0 || dist == 0 {
            bail!("Wrong write_previous() arguments provided: dist={}, len={} (current buffer size={})", dist, len, self.history.len());
        }

        let slice_start = self.history.len() - dist;
        let slice_end = min(slice_start + len, self.history.len());

        // using .cycle() in case of len > dist
        let history_slice: Vec<u8> = self
            .history
            .range(slice_start..slice_end)
            .copied()
            .cycle()
            .take(len)
            .collect();

        self.write_all(&history_slice)
            .context("Unable to write all slice of history bytes!")
    }

    pub fn byte_count(&self) -> usize {
        self.byte_count
    }

    // returns crc32 and underlying writer
    pub fn crc32(self) -> (u32, T) {
        (self.digest.finalize(), self.inner)
    }
}

////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;

    #[test]
    fn write() -> Result<()> {
        let mut buf: &mut [u8] = &mut [0u8; 10];
        let mut writer = TrackingWriter::new(&mut buf);

        assert_eq!(writer.write(&[1, 2, 3, 4])?, 4);
        assert_eq!(writer.byte_count(), 4);

        assert_eq!(writer.write(&[4, 8, 15, 16, 23])?, 5);
        assert_eq!(writer.byte_count(), 9);

        assert_eq!(writer.write(&[0, 0, 123])?, 1);
        assert_eq!(writer.byte_count(), 10);

        assert_eq!(writer.write(&[42, 124, 234, 27])?, 0);
        assert_eq!(writer.byte_count(), 10);
        assert_eq!(writer.crc32().0, 2992191065);

        Ok(())
    }

    #[test]
    fn write_previous() -> Result<()> {
        let mut buf: &mut [u8] = &mut [0u8; 512];
        let mut writer = TrackingWriter::new(&mut buf);

        for i in 0..=255 {
            writer.write_u8(i)?;
        }

        writer.write_previous(192, 128)?;
        assert_eq!(writer.byte_count(), 384);

        assert!(writer.write_previous(10000, 20).is_err());
        assert_eq!(writer.byte_count(), 384);

        assert!(writer.write_previous(256, 256).is_err());
        assert_eq!(writer.byte_count(), 512);

        assert!(writer.write_previous(1, 1).is_err());
        assert_eq!(writer.byte_count(), 512);
        assert_eq!(writer.crc32().0, 2733545866);

        Ok(())
    }

    #[test]
    fn write_previous_overlapped() -> Result<()> {
        let mut buf: &mut [u8] = &mut [0u8; 10];
        let mut writer = TrackingWriter::new(&mut buf);
        writer.write_u8(0b11110000)?;
        writer.write_u8(0b00001111)?;

        assert!(writer.write_previous(2, 8).is_ok());
        assert_eq!(writer.byte_count(), 10);
        assert_eq!(writer.crc32().0, 3148311779);

        Ok(())
    }
}
