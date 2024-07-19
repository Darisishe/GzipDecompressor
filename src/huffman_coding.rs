#![forbid(unsafe_code)]

use std::{collections::HashMap, convert::TryFrom, io::BufRead};

use anyhow::{anyhow, bail, ensure, Context, Result};

use crate::bit_reader::{BitReader, BitSequence};

////////////////////////////////////////////////////////////////////////////////

pub fn decode_litlen_distance_trees<T: BufRead>(
    bit_reader: &mut BitReader<T>,
) -> Result<(HuffmanCoding<LitLenToken>, HuffmanCoding<DistanceToken>)> {
    let litlen_codes_count = (bit_reader
        .read_bits(5)
        .context("Failed to read HLIT bits")?
        .bits()
        + 257) as usize;
    let dist_codes_count = (bit_reader
        .read_bits(5)
        .context("Failed to read HDIST bits")?
        .bits()
        + 1) as usize;
    let codelen_codes_count = bit_reader
        .read_bits(4)
        .context("Failed to read HCLEN bits")?
        .bits()
        + 4;

    let codelen_coding = build_codelen_coding(bit_reader, codelen_codes_count)
        .context("Failed to build codelen coding")?;

    let mut code_lengths = Vec::<u8>::with_capacity(litlen_codes_count + dist_codes_count);
    while code_lengths.len() < litlen_codes_count + dist_codes_count {
        let token = codelen_coding.read_symbol(bit_reader)?;
        match token {
            TreeCodeToken::Length(val) => code_lengths.push(val),
            TreeCodeToken::CopyPrev => {
                let offset = bit_reader.read_bits(2)?.bits() as usize;
                let prev = *code_lengths.last().context("No code length to copy!")?;
                code_lengths.extend(vec![prev; 3 + offset]);
            }
            TreeCodeToken::RepeatZero { base, extra_bits } => {
                let offset = bit_reader.read_bits(extra_bits)?.bits() as usize;
                code_lengths.extend(vec![0; (base as usize) + offset])
            }
        }
    }

    if code_lengths.len() > litlen_codes_count + dist_codes_count {
        bail!("Number of codes exceeded!");
    }

    Ok((
        HuffmanCoding::from_lengths(&code_lengths[0..litlen_codes_count])?,
        HuffmanCoding::from_lengths(&code_lengths[litlen_codes_count..])?,
    ))
}

pub fn build_fixed_trees() -> Result<(HuffmanCoding<LitLenToken>, HuffmanCoding<DistanceToken>)> {
    let mut litlen_tree_lengths = vec![8u8; 144];
    litlen_tree_lengths.extend([9u8; 112]);
    litlen_tree_lengths.extend([7u8; 24]);
    litlen_tree_lengths.extend([8u8; 8]);

    let distance_tree_lengths = [5u8; 32];

    Ok((
        HuffmanCoding::from_lengths(&litlen_tree_lengths)?,
        HuffmanCoding::from_lengths(&distance_tree_lengths)?,
    ))
}

fn read_codelen_length<T: BufRead>(bit_reader: &mut BitReader<T>) -> Result<u8> {
    Ok(bit_reader
        .read_bits(3)
        .context("Failed to read length for codelen")?
        .bits() as u8)
}

fn build_codelen_coding<T: BufRead>(
    bit_reader: &mut BitReader<T>,
    codelen_codes_count: u16,
) -> Result<HuffmanCoding<TreeCodeToken>> {
    let mut codelen_code_lengths = [0u8; 19];
    codelen_code_lengths[16] = read_codelen_length(bit_reader)?;
    codelen_code_lengths[17] = read_codelen_length(bit_reader)?;
    codelen_code_lengths[18] = read_codelen_length(bit_reader)?;
    codelen_code_lengths[0] = read_codelen_length(bit_reader)?;

    for i in 0..(codelen_codes_count - 4) {
        let j = (if i % 2 == 0 { 8 + i / 2 } else { 7 - i / 2 }) as usize;
        codelen_code_lengths[j] = read_codelen_length(bit_reader)?
    }

    HuffmanCoding::<TreeCodeToken>::from_lengths(&codelen_code_lengths)
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Copy, Debug)]
pub enum TreeCodeToken {
    Length(u8),
    CopyPrev,
    RepeatZero { base: u16, extra_bits: u8 },
}

impl TryFrom<HuffmanCodeWord> for TreeCodeToken {
    type Error = anyhow::Error;

    fn try_from(value: HuffmanCodeWord) -> Result<Self> {
        match value.0 {
            0..=15 => Ok(TreeCodeToken::Length(value.0 as u8)),
            16 => Ok(TreeCodeToken::CopyPrev),
            17 => Ok(TreeCodeToken::RepeatZero {
                base: 3,
                extra_bits: 3,
            }),
            18 => Ok(TreeCodeToken::RepeatZero {
                base: 11,
                extra_bits: 7,
            }),
            _ => Err(anyhow!("Not a code: {}", value.0)),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Copy, Debug)]
pub enum LitLenToken {
    Literal(u8),
    EndOfBlock,
    Length { base: u16, extra_bits: u8 },
}

impl TryFrom<HuffmanCodeWord> for LitLenToken {
    type Error = anyhow::Error;

    fn try_from(value: HuffmanCodeWord) -> Result<Self> {
        match value.0 {
            0..=255 => Ok(LitLenToken::Literal(value.0 as u8)),
            256 => Ok(LitLenToken::EndOfBlock),
            257..=264 => Ok(LitLenToken::Length {
                base: value.0 - 254,
                extra_bits: 0,
            }),
            265..=268 => Ok(LitLenToken::Length {
                base: 11 + 2 * (value.0 - 265),
                extra_bits: 1,
            }),
            269..=272 => Ok(LitLenToken::Length {
                base: 19 + 4 * (value.0 - 269),
                extra_bits: 2,
            }),
            273..=276 => Ok(LitLenToken::Length {
                base: 35 + 8 * (value.0 - 273),
                extra_bits: 3,
            }),
            277..=280 => Ok(LitLenToken::Length {
                base: 67 + 16 * (value.0 - 277),
                extra_bits: 4,
            }),
            281..=284 => Ok(LitLenToken::Length {
                base: 131 + 32 * (value.0 - 281),
                extra_bits: 5,
            }),
            285 => Ok(LitLenToken::Length {
                base: 258,
                extra_bits: 0,
            }),
            286..=287 => Err(anyhow!("Reserved code: {}", value.0)),
            _ => Err(anyhow!("Not a code: {}", value.0)),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Copy, Debug)]
pub struct DistanceToken {
    pub base: u16,
    pub extra_bits: u8,
}

impl TryFrom<HuffmanCodeWord> for DistanceToken {
    type Error = anyhow::Error;

    fn try_from(value: HuffmanCodeWord) -> Result<Self> {
        if value.0 <= 1 {
            Ok(DistanceToken {
                base: value.0 + 1,
                extra_bits: 0,
            })
        } else if value.0 <= 29 {
            let extra_bits = (value.0 / 2 - 1) as u8;
            let base = ((value.0 % 2 + 2) << extra_bits) + 1;

            Ok(DistanceToken { base, extra_bits })
        } else if value.0 <= 31 {
            Err(anyhow!("Reserved code: {}", value.0))
        } else {
            Err(anyhow!("Not a code: {}", value.0))
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

const MAX_BITS: usize = 15;

pub struct HuffmanCodeWord(pub u16);

pub struct HuffmanCoding<T> {
    map: HashMap<BitSequence, T>,
}

impl<T> HuffmanCoding<T>
where
    T: Copy + TryFrom<HuffmanCodeWord, Error = anyhow::Error>,
{
    pub fn new(map: HashMap<BitSequence, T>) -> Self {
        Self { map }
    }

    #[allow(unused)]
    pub fn decode_symbol(&self, seq: BitSequence) -> Option<T> {
        self.map.get(&seq).copied()
    }

    pub fn read_symbol<U: BufRead>(&self, bit_reader: &mut BitReader<U>) -> Result<T> {
        let mut code = BitSequence::new(0, 0);
        for _ in 0..MAX_BITS {
            let new_bit = bit_reader.read_bits(1).context("Failed to read a bit")?;
            code = code.concat(new_bit);
            if let Some(&token) = self.map.get(&code) {
                return Ok(token);
            }
        }

        bail!("Failed to read a symbol");
    }

    pub fn from_lengths(code_lengths: &[u8]) -> Result<Self> {
        if code_lengths
            .iter()
            .max()
            .is_some_and(|&x| x as usize > MAX_BITS)
        {
            bail!("Length greater than {MAX_BITS} found!");
        }

        let mut bl_count = [0usize; MAX_BITS + 1];
        for &length in code_lengths {
            bl_count[length as usize] += 1;
        }

        let mut code = 0;
        bl_count[0] = 0;
        let mut next_code = [0; MAX_BITS + 1];

        for length in 1..=MAX_BITS {
            code = (code + bl_count[length - 1]) << 1;
            next_code[length] = code;
        }

        let mut map = HashMap::new();

        for (i, &length) in code_lengths.iter().enumerate() {
            if length != 0 {
                ensure!(
                    next_code[length as usize] < (1 << (length + 1)),
                    "Couldn't build coding, incorrect lengths provided!"
                );

                let bits = next_code[length as usize] as u16;

                let word = HuffmanCodeWord(u16::try_from(i).context("code_lengths is too large!")?);
                let token = T::try_from(word).context("Couldn't create a token from word!")?;

                map.insert(BitSequence::new(bits, length), token);

                next_code[length as usize] += 1;
            }
        }

        Ok(HuffmanCoding::new(map))
    }
}

////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Value(u16);

    impl TryFrom<HuffmanCodeWord> for Value {
        type Error = anyhow::Error;

        fn try_from(x: HuffmanCodeWord) -> Result<Self> {
            Ok(Self(x.0))
        }
    }

    #[test]
    fn from_lengths() -> Result<()> {
        let code = HuffmanCoding::<Value>::from_lengths(&[2, 3, 4, 3, 3, 4, 2])?;

        assert_eq!(
            code.decode_symbol(BitSequence::new(0b00, 2)),
            Some(Value(0)),
        );
        assert_eq!(
            code.decode_symbol(BitSequence::new(0b100, 3)),
            Some(Value(1)),
        );
        assert_eq!(
            code.decode_symbol(BitSequence::new(0b1110, 4)),
            Some(Value(2)),
        );
        assert_eq!(
            code.decode_symbol(BitSequence::new(0b101, 3)),
            Some(Value(3)),
        );
        assert_eq!(
            code.decode_symbol(BitSequence::new(0b110, 3)),
            Some(Value(4)),
        );
        assert_eq!(
            code.decode_symbol(BitSequence::new(0b1111, 4)),
            Some(Value(5)),
        );
        assert_eq!(
            code.decode_symbol(BitSequence::new(0b01, 2)),
            Some(Value(6)),
        );

        assert_eq!(code.decode_symbol(BitSequence::new(0b0, 1)), None);
        assert_eq!(code.decode_symbol(BitSequence::new(0b10, 2)), None);
        assert_eq!(code.decode_symbol(BitSequence::new(0b111, 3)), None,);

        Ok(())
    }

    #[test]
    fn read_symbol() -> Result<()> {
        let code = HuffmanCoding::<Value>::from_lengths(&[2, 3, 4, 3, 3, 4, 2])?;
        let mut data: &[u8] = &[0b10111001, 0b11001010, 0b11101101];
        let mut reader = BitReader::new(&mut data);

        assert_eq!(code.read_symbol(&mut reader)?, Value(1));
        assert_eq!(code.read_symbol(&mut reader)?, Value(2));
        assert_eq!(code.read_symbol(&mut reader)?, Value(3));
        assert_eq!(code.read_symbol(&mut reader)?, Value(6));
        assert_eq!(code.read_symbol(&mut reader)?, Value(0));
        assert_eq!(code.read_symbol(&mut reader)?, Value(2));
        assert_eq!(code.read_symbol(&mut reader)?, Value(4));
        assert!(code.read_symbol(&mut reader).is_err());

        Ok(())
    }

    #[test]
    fn from_lengths_with_zeros() -> Result<()> {
        let lengths = [3, 4, 5, 5, 0, 0, 6, 6, 4, 0, 6, 0, 7];
        let code = HuffmanCoding::<Value>::from_lengths(&lengths)?;
        let mut data: &[u8] = &[
            0b00100000, 0b00100001, 0b00010101, 0b10010101, 0b00110101, 0b00011101,
        ];
        let mut reader = BitReader::new(&mut data);

        assert_eq!(code.read_symbol(&mut reader)?, Value(0));
        assert_eq!(code.read_symbol(&mut reader)?, Value(1));
        assert_eq!(code.read_symbol(&mut reader)?, Value(2));
        assert_eq!(code.read_symbol(&mut reader)?, Value(3));
        assert_eq!(code.read_symbol(&mut reader)?, Value(6));
        assert_eq!(code.read_symbol(&mut reader)?, Value(7));
        assert_eq!(code.read_symbol(&mut reader)?, Value(8));
        assert_eq!(code.read_symbol(&mut reader)?, Value(10));
        assert_eq!(code.read_symbol(&mut reader)?, Value(12));
        assert!(code.read_symbol(&mut reader).is_err());

        Ok(())
    }

    #[test]
    fn from_lengths_additional() -> Result<()> {
        let lengths = [
            9, 10, 10, 8, 8, 8, 5, 6, 4, 5, 4, 5, 4, 5, 4, 4, 5, 4, 4, 5, 4, 5, 4, 5, 5, 5, 4, 6, 6,
        ];
        let code = HuffmanCoding::<Value>::from_lengths(&lengths)?;
        let mut data: &[u8] = &[
            0b11111000, 0b10111100, 0b01010001, 0b11111111, 0b00110101, 0b11111001, 0b11011111,
            0b11100001, 0b01110111, 0b10011111, 0b10111111, 0b00110100, 0b10111010, 0b11111111,
            0b11111101, 0b10010100, 0b11001110, 0b01000011, 0b11100111, 0b00000010,
        ];
        let mut reader = BitReader::new(&mut data);

        assert_eq!(code.read_symbol(&mut reader)?, Value(10));
        assert_eq!(code.read_symbol(&mut reader)?, Value(7));
        assert_eq!(code.read_symbol(&mut reader)?, Value(27));
        assert_eq!(code.read_symbol(&mut reader)?, Value(22));
        assert_eq!(code.read_symbol(&mut reader)?, Value(9));
        assert_eq!(code.read_symbol(&mut reader)?, Value(0));
        assert_eq!(code.read_symbol(&mut reader)?, Value(11));
        assert_eq!(code.read_symbol(&mut reader)?, Value(15));
        assert_eq!(code.read_symbol(&mut reader)?, Value(2));
        assert_eq!(code.read_symbol(&mut reader)?, Value(20));
        assert_eq!(code.read_symbol(&mut reader)?, Value(8));
        assert_eq!(code.read_symbol(&mut reader)?, Value(4));
        assert_eq!(code.read_symbol(&mut reader)?, Value(23));
        assert_eq!(code.read_symbol(&mut reader)?, Value(24));
        assert_eq!(code.read_symbol(&mut reader)?, Value(5));
        assert_eq!(code.read_symbol(&mut reader)?, Value(26));
        assert_eq!(code.read_symbol(&mut reader)?, Value(18));
        assert_eq!(code.read_symbol(&mut reader)?, Value(12));
        assert_eq!(code.read_symbol(&mut reader)?, Value(25));
        assert_eq!(code.read_symbol(&mut reader)?, Value(1));
        assert_eq!(code.read_symbol(&mut reader)?, Value(3));
        assert_eq!(code.read_symbol(&mut reader)?, Value(6));
        assert_eq!(code.read_symbol(&mut reader)?, Value(13));
        assert_eq!(code.read_symbol(&mut reader)?, Value(14));
        assert_eq!(code.read_symbol(&mut reader)?, Value(16));
        assert_eq!(code.read_symbol(&mut reader)?, Value(17));
        assert_eq!(code.read_symbol(&mut reader)?, Value(19));
        assert_eq!(code.read_symbol(&mut reader)?, Value(21));

        Ok(())
    }
}
