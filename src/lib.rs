#![forbid(unsafe_code)]

use std::io::{BufRead, Write};

use anyhow::Result;
use deflate::{
    DeflateBlock, DeflateReader,
    NextBlock::{BlockOrError, Footer},
};
use gzip::GzipFooter;
use log::*;

use gzip::GzipReader;

mod bit_reader;
mod deflate;
mod gzip;
mod huffman_coding;
mod tracking_writer;

fn process_gzip_footer<R: BufRead, W: Write>(
    gzip_footer: GzipFooter<R, W>,
) -> Result<GzipReader<R, W>> {
    info!("Processing Gzip footer...");

    match gzip_footer.read_footer() {
        Ok((footer, gzip_reader)) => {
            trace!("Gzip footer: {:?}", footer);

            info!("Finished reading Gzip footer!");

            Ok(gzip_reader)
        }

        Err(error) => {
            error!("Failed while processing Gzip footer!");

            Err(error)
        }
    }
}

fn process_deflate_block<R: BufRead, W: Write>(
    block: DeflateBlock<R, W>,
) -> Result<DeflateReader<R, W>> {
    trace!("Deflate block header: {:?}", block.get_header());

    match block.read_content() {
        Ok(deflate_reader) => {
            info!("Finished reading deflate block!");

            Ok(deflate_reader)
        }
        Err(error) => {
            error!("Failed while processing deflate block!");

            Err(error)
        }
    }
}

fn process_compressed_data<R: BufRead, W: Write>(
    mut deflate_reader: DeflateReader<R, W>,
) -> Result<GzipReader<R, W>> {
    info!("Starting to process Deflate part of file...");

    loop {
        match deflate_reader.next_block() {
            BlockOrError(maybe_block) => match maybe_block {
                Ok(block) => {
                    deflate_reader = process_deflate_block(block)?;
                }

                Err(error) => {
                    error!("Failure during deflate header reading!");

                    return Err(error);
                }
            },

            Footer(reader, writer) => {
                return process_gzip_footer(GzipFooter::new(reader, writer));
            }
        }
    }
}

pub fn decompress<R: BufRead, W: Write>(input: R, output: W) -> Result<()> {
    let mut gzip_reader = GzipReader::new(input, output);

    info!("Decompression started!");
    while !gzip_reader.is_empty()? {
        info!("Starting to process member...");

        match gzip_reader.next_member() {
            Ok((header, deflate_reader)) => {
                trace!("Gzip member header: {:?}", header);

                // gzip_reader may be reused in case of multiple compressed files in one gzip
                gzip_reader = process_compressed_data(deflate_reader)?;

                info!("Member decompression finished successfully!");
            }

            Err(error) => {
                error!("Unable to read Gzip member header!");
                return Err(error);
            }
        }
    }

    info!("All Gzip members decompressed successfully!");

    Ok(())
}
