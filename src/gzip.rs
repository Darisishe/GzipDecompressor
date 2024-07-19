#![forbid(unsafe_code)]

use std::io::{BufRead, Write};

use anyhow::{bail, ensure, Context, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use crc::Crc;

use crate::{bit_reader::BitReader, deflate::DeflateReader, tracking_writer::TrackingWriter};

////////////////////////////////////////////////////////////////////////////////

const ID1: u8 = 0x1f;
const ID2: u8 = 0x8b;

const CM_DEFLATE: u8 = 8;

const FTEXT_OFFSET: u8 = 0;
const FHCRC_OFFSET: u8 = 1;
const FEXTRA_OFFSET: u8 = 2;
const FNAME_OFFSET: u8 = 3;
const FCOMMENT_OFFSET: u8 = 4;

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct MemberHeader {
    pub compression_method: CompressionMethod,
    pub modification_time: u32,
    pub extra: Option<Vec<u8>>,
    pub name: Option<String>,
    pub comment: Option<String>,
    pub extra_flags: u8,
    pub os: u8,
    pub has_crc: bool,
    pub is_text: bool,
}

impl MemberHeader {
    pub fn crc16(&self) -> u16 {
        let crc = Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
        let mut digest = crc.digest();

        digest.update(&[ID1, ID2, self.compression_method.into(), self.flags().0]);
        digest.update(&self.modification_time.to_le_bytes());
        digest.update(&[self.extra_flags, self.os]);

        if let Some(extra) = &self.extra {
            digest.update(&(extra.len() as u16).to_le_bytes());
            digest.update(extra);
        }

        if let Some(name) = &self.name {
            digest.update(name.as_bytes());
            digest.update(&[0]);
        }

        if let Some(comment) = &self.comment {
            digest.update(comment.as_bytes());
            digest.update(&[0]);
        }

        (digest.finalize() & 0xffff) as u16
    }

    pub fn flags(&self) -> MemberFlags {
        let mut flags = MemberFlags(0);
        flags.set_is_text(self.is_text);
        flags.set_has_crc(self.has_crc);
        flags.set_has_extra(self.extra.is_some());
        flags.set_has_name(self.name.is_some());
        flags.set_has_comment(self.comment.is_some());
        flags
    }
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Copy, Debug)]
pub enum CompressionMethod {
    Deflate,
    Unknown(u8),
}

impl From<u8> for CompressionMethod {
    fn from(value: u8) -> Self {
        match value {
            CM_DEFLATE => Self::Deflate,
            x => Self::Unknown(x),
        }
    }
}

impl From<CompressionMethod> for u8 {
    fn from(method: CompressionMethod) -> u8 {
        match method {
            CompressionMethod::Deflate => CM_DEFLATE,
            CompressionMethod::Unknown(x) => x,
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct MemberFlags(u8);

#[allow(unused)]
impl MemberFlags {
    fn bit(&self, n: u8) -> bool {
        (self.0 >> n) & 1 != 0
    }

    fn set_bit(&mut self, n: u8, value: bool) {
        if value {
            self.0 |= 1 << n;
        } else {
            self.0 &= !(1 << n);
        }
    }

    pub fn is_text(&self) -> bool {
        self.bit(FTEXT_OFFSET)
    }

    pub fn set_is_text(&mut self, value: bool) {
        self.set_bit(FTEXT_OFFSET, value)
    }

    pub fn has_crc(&self) -> bool {
        self.bit(FHCRC_OFFSET)
    }

    pub fn set_has_crc(&mut self, value: bool) {
        self.set_bit(FHCRC_OFFSET, value)
    }

    pub fn has_extra(&self) -> bool {
        self.bit(FEXTRA_OFFSET)
    }

    pub fn set_has_extra(&mut self, value: bool) {
        self.set_bit(FEXTRA_OFFSET, value)
    }

    pub fn has_name(&self) -> bool {
        self.bit(FNAME_OFFSET)
    }

    pub fn set_has_name(&mut self, value: bool) {
        self.set_bit(FNAME_OFFSET, value)
    }

    pub fn has_comment(&self) -> bool {
        self.bit(FCOMMENT_OFFSET)
    }

    pub fn set_has_comment(&mut self, value: bool) {
        self.set_bit(FCOMMENT_OFFSET, value)
    }
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct MemberFooter {
    pub data_crc32: u32,
    pub data_size: u32,
}

////////////////////////////////////////////////////////////////////////////////

pub struct GzipReader<R, W> {
    reader: R,
    underlying_writer: W,
}

impl<R: BufRead, W: Write> GzipReader<R, W> {
    pub fn new(reader: R, underlying_writer: W) -> Self {
        Self {
            reader,
            underlying_writer,
        }
    }

    // reads Gzip header and transforms to DeflateReader
    pub fn next_member(mut self) -> Result<(MemberHeader, DeflateReader<R, W>)> {
        let header = self
            .read_header()
            .context("Failure while reading header!")?;

        match header.compression_method {
            CompressionMethod::Unknown(x) => bail!("unsupported compression method: {x}"),
            CompressionMethod::Deflate => Ok((
                header,
                DeflateReader::new(
                    BitReader::new(self.reader),
                    TrackingWriter::new(self.underlying_writer),
                ),
            )),
        }
    }

    pub fn is_empty(&mut self) -> Result<bool> {
        Ok(self.reader.fill_buf()?.is_empty())
    }

    fn read_header(&mut self) -> Result<MemberHeader> {
        let id1 = self.reader.read_u8().context("Failed reading ID1!")?;
        let id2 = self.reader.read_u8().context("Failed reading ID1!")?;
        ensure!(id1 == ID1 && id2 == ID2, "wrong id values!");

        let compression_method =
            CompressionMethod::from(self.reader.read_u8().context("Failed reading CM!")?);

        let member_flags = MemberFlags(self.reader.read_u8().context("Failed reading FLG!")?);

        let header = MemberHeader {
            compression_method,
            modification_time: self.read_modification_time()?,
            extra_flags: self.reader.read_u8().context("Failed reading XFL!")?,
            os: self.reader.read_u8().context("Failed reading OS!")?,
            extra: self.read_extra(member_flags.has_extra())?,
            name: self.read_name(member_flags.has_name())?,
            comment: self.read_comment(member_flags.has_comment())?,
            has_crc: member_flags.has_crc(),
            is_text: member_flags.is_text(),
        };

        if member_flags.has_crc() {
            let crc16 = self
                .reader
                .read_u16::<LittleEndian>()
                .context("Failed reading CRC16!")?;

            ensure!(header.crc16() == crc16, "header crc16 check failed!");
        }

        Ok(header)
    }

    fn read_null_term_string(&mut self) -> Result<String> {
        let mut buffer = Vec::new();
        self.reader.read_until(0, &mut buffer)?;

        ensure!(!buffer.is_empty(), "No null-terminator!");

        Ok(String::from_utf8(buffer)?)
    }

    fn read_modification_time(&mut self) -> Result<u32> {
        self.reader
            .read_u32::<LittleEndian>()
            .context("Failed reading MTIME!")
    }

    fn read_extra(&mut self, has_extra: bool) -> Result<Option<Vec<u8>>> {
        if !has_extra {
            return Ok(None);
        }

        let len = self
            .reader
            .read_u16::<LittleEndian>()
            .context("Failed reading XLEN!")?;

        let mut buf = vec![0u8; len as usize];
        self.reader
            .read_exact(&mut buf)
            .context("Failed to read extra field!")?;

        Ok(Some(buf))
    }

    fn read_name(&mut self, has_name: bool) -> Result<Option<String>> {
        if !has_name {
            return Ok(None);
        }

        Ok(Some(
            self.read_null_term_string()
                .context("Failed reading file name!")?,
        ))
    }

    fn read_comment(&mut self, has_comment: bool) -> Result<Option<String>> {
        if !has_comment {
            return Ok(None);
        }

        Ok(Some(
            self.read_null_term_string()
                .context("Failed reading comment!")?,
        ))
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct GzipFooter<R, W> {
    reader: R,
    writer: TrackingWriter<W>,
}

impl<R: BufRead, W: Write> GzipFooter<R, W> {
    pub fn new(reader: R, writer: TrackingWriter<W>) -> Self {
        GzipFooter { reader, writer }
    }

    pub fn read_footer(mut self) -> Result<(MemberFooter, GzipReader<R, W>)> {
        let data_crc32 = self
            .reader
            .read_u32::<LittleEndian>()
            .context("Failed reading CRC32!")?;

        let data_size = self
            .reader
            .read_u32::<LittleEndian>()
            .context("Failed reading ISIZE!")?;

        let footer = MemberFooter {
            data_crc32,
            data_size,
        };

        if self.writer.byte_count() != (footer.data_size as usize) {
            bail!("length check failed!");
        }

        let (crc32, underlying) = self.writer.crc32();

        if crc32 != footer.data_crc32 {
            bail!("crc32 check failed!");
        }

        Ok((footer, GzipReader::new(self.reader, underlying)))
    }
}
