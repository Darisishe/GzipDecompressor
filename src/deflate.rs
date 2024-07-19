#![forbid(unsafe_code)]

use std::io::{BufRead, Write};

use anyhow::{bail, ensure, Context, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::huffman_coding::{DistanceToken, HuffmanCoding, LitLenToken};
use crate::tracking_writer::TrackingWriter;
use crate::{
    bit_reader::BitReader,
    huffman_coding::{build_fixed_trees, decode_litlen_distance_trees},
};

////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub struct BlockHeader {
    pub is_final: bool,
    pub compression_type: CompressionType,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CompressionType {
    Uncompressed = 0,
    FixedTree = 1,
    DynamicTree = 2,
    Reserved = 3,
}

////////////////////////////////////////////////////////////////////////////////

pub enum NextBlock<R, W> {
    /// indicates start of Footer of outer format (example: GZIP)
    /// and contains IOs of DeflateReader
    Footer(R, TrackingWriter<W>),

    BlockOrError(Result<DeflateBlock<R, W>>),
}

pub struct DeflateReader<R, W> {
    bit_reader: BitReader<R>,
    writer: TrackingWriter<W>,
    is_exhausted: bool,
}

impl<R: BufRead, W: Write> DeflateReader<R, W> {
    pub fn new(bit_reader: BitReader<R>, writer: TrackingWriter<W>) -> Self {
        Self {
            bit_reader,
            writer,
            is_exhausted: false,
        }
    }

    // reads header and transforms to DeflateBlock
    pub fn next_block(mut self) -> NextBlock<R, W> {
        if self.is_exhausted {
            return NextBlock::Footer(self.bit_reader.into_inner(), self.writer);
        }

        match self.read_header() {
            Ok(header) => NextBlock::BlockOrError(Ok(DeflateBlock {
                bit_reader: self.bit_reader,
                writer: self.writer,
                header,
            })),
            Err(error) => NextBlock::BlockOrError(Err(error)),
        }
    }

    fn read_header(&mut self) -> Result<BlockHeader> {
        let bfinal = self
            .bit_reader
            .read_bits(1)
            .context("Failed to read BFINAL in header!")?
            .bits();

        let btype = self
            .bit_reader
            .read_bits(2)
            .context("Failed reading BTYPE in header!")?
            .bits();

        let compression_type = match btype {
            0 => CompressionType::Uncompressed,
            1 => CompressionType::FixedTree,
            2 => CompressionType::DynamicTree,
            3 => CompressionType::Reserved,
            _ => unreachable!(),
        };

        Ok(BlockHeader {
            is_final: (bfinal == 1),
            compression_type,
        })
    }
}

// extracts data from block
pub struct DeflateBlock<R, W> {
    bit_reader: BitReader<R>,
    writer: TrackingWriter<W>,
    header: BlockHeader,
}

impl<R: BufRead, W: Write> DeflateBlock<R, W> {
    pub fn get_header(&self) -> &BlockHeader {
        &self.header
    }

    // reads block content to writer and transforms DeflateBlock back to DeflateReader
    pub fn read_content(mut self) -> Result<DeflateReader<R, W>> {
        if self.header.compression_type == CompressionType::Reserved {
            bail!("unsupported block type!");
        } else if self.header.compression_type == CompressionType::Uncompressed {
            self.process_uncompressed()?;
        } else {
            let (litlen_tree, distance_tree) =
                if self.header.compression_type == CompressionType::FixedTree {
                    build_fixed_trees()
                } else {
                    decode_litlen_distance_trees(&mut self.bit_reader)
                }
                .context("Failed to build trees!")?;

            self.process_with_trees(litlen_tree, distance_tree)?;
        }

        Ok(DeflateReader {
            bit_reader: self.bit_reader,
            writer: self.writer,
            is_exhausted: self.header.is_final,
        })
    }

    fn process_uncompressed(&mut self) -> Result<()> {
        let reader = self.bit_reader.borrow_reader_from_boundary();
        let len = reader
            .read_u16::<LittleEndian>()
            .context("Failed to read LEN!")?;

        let nlen = reader
            .read_u16::<LittleEndian>()
            .context("Failed to read NLEN!")?;

        ensure!(len == !nlen, "nlen check failed!");

        let mut buf = vec![0u8; len as usize];
        reader
            .read_exact(&mut buf)
            .context("Failed to read the content of uncompressed block!")?;

        self.writer
            .write_all(&buf)
            .context("Failed to write the content of uncompressed block!")?;

        Ok(())
    }

    fn process_with_trees(
        &mut self,
        litlen_tree: HuffmanCoding<LitLenToken>,
        distance_tree: HuffmanCoding<DistanceToken>,
    ) -> Result<()> {
        loop {
            let token = litlen_tree
                .read_symbol(&mut self.bit_reader)
                .context("literal/length token expected!")?;

            match token {
                LitLenToken::Literal(byte) => {
                    self.writer
                        .write_u8(byte)
                        .context("Failed to write Literal!")?;
                }

                LitLenToken::Length { base, extra_bits } => {
                    self.process_length_token(base, extra_bits, &distance_tree)?;
                }

                LitLenToken::EndOfBlock => break,
            }
        }

        Ok(())
    }

    fn process_length_token(
        &mut self,
        len_base: u16,
        len_extra_bits: u8,
        distance_tree: &HuffmanCoding<DistanceToken>,
    ) -> Result<()> {
        let len_offset = self
            .bit_reader
            .read_bits(len_extra_bits)
            .context("Failed to read Length extra bits!")?
            .bits();

        let len = (len_base + len_offset) as usize;

        let distance_token = distance_tree
            .read_symbol(&mut self.bit_reader)
            .context("distance token expected!")?;

        let dist_offset = self
            .bit_reader
            .read_bits(distance_token.extra_bits)
            .context("Failed to read Distance extra bits!")?
            .bits();

        let dist = (distance_token.base + dist_offset) as usize;

        self.writer
            .write_previous(dist, len)
            .context("Wrong Length/Distance!")?;

        Ok(())
    }
}
