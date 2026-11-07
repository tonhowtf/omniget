//! Pattern dictionary segment parsing and decoding (7.4.4, 6.7).

use super::{AdaptiveTemplatePixel, Template, generic};
use crate::ScratchBuffers;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::{Bitmap, WORD_BITS, Word, copy_work};
use crate::error::{OverflowError, ParseError, Result};
use crate::reader::Reader;
use alloc::vec::Vec;

/// PdfCraft patch: the most words of shifted copies a dictionary keeps, 16 MiB, where 256 patterns
/// of 32 rows take 2 MiB. A 48-byte dictionary of 65,535 such patterns asked for 539 MB.
const MAX_SHIFTED_WORDS: u64 = 1 << 22;

/// Decode a pattern dictionary segment (7.4.4.2, 6.7).
pub(crate) fn decode(
    header: &PatternDictionaryHeader<'_>,
    scratch: &mut ScratchBuffers,
    room: u64,
) -> Result<PatternDictionary> {
    let pattern_width = header.pattern_width as u32;
    let pattern_height = header.pattern_height as u32;
    let num_patterns = header
        .num_patterns
        .checked_add(1)
        .ok_or(OverflowError::BitmapDimension)?;

    // "1) Create a bitmap B_HDC. The height of this bitmap is HDPH. The width
    // of the bitmap is (GRAYMAX + 1) × HDPW. This bitmap contains all the
    // patterns concatenated left to right." (6.7.5)
    let collective_width = num_patterns
        .checked_mul(pattern_width)
        .ok_or(OverflowError::BitmapDimension)?;

    let mut collective_bitmap = Bitmap::new(collective_width, pattern_height)?;

    // "2) Decode the collective bitmap using a generic region decoding procedure
    // as described in 6.2." (6.7.5)
    if header.mmr {
        let _ = generic::decode_bitmap_mmr(&mut collective_bitmap, header.data)?;
    } else {
        let at_pixels = match header.template {
            Template::Template0 => [
                AdaptiveTemplatePixel {
                    x: -(pattern_width as i8),
                    y: 0,
                },
                AdaptiveTemplatePixel { x: -3, y: -1 },
                AdaptiveTemplatePixel { x: 2, y: -2 },
                AdaptiveTemplatePixel { x: -2, y: -2 },
            ],
            Template::Template1 | Template::Template2 | Template::Template3 => [
                AdaptiveTemplatePixel {
                    x: -(pattern_width as i8),
                    y: 0,
                },
                // Unused.
                AdaptiveTemplatePixel::default(),
                AdaptiveTemplatePixel::default(),
                AdaptiveTemplatePixel::default(),
            ],
        };

        let mut decoder = ArithmeticDecoder::new(header.data);
        scratch.contexts.clear();
        scratch.contexts.resize(
            1 << header.template.context_bits(),
            ArithmeticDecoderContext::default(),
        );

        generic::decode_bitmap_arithmetic_coding(
            &mut collective_bitmap,
            &mut decoder,
            &mut scratch.contexts,
            header.template,
            false,
            &at_pixels,
        )?;
    }

    // "3) Set: GRAY = 0" (6.7.5)
    // "4) While GRAY ≤ GRAYMAX:" (6.7.5)
    let mut patterns = Vec::with_capacity(num_patterns as usize);

    let src_stride = collective_bitmap.stride;

    for gray in 0..num_patterns {
        // "a) Let the subimage of B_HDC consisting of HPH rows and columns
        // HDPW × GRAY through HDPW × (GRAY + 1) − 1 be denoted B_P. Set:
        // HDPATS[GRAY] = B_P" (6.7.5)"
        let start_x = gray * pattern_width;
        let mut pattern = Bitmap::new(pattern_width, pattern_height)?;

        let src_word_idx = start_x / WORD_BITS;
        let src_bit_offset = start_x % WORD_BITS;
        let dst_stride = pattern.stride;

        for y in 0..pattern_height {
            let src_row = (y * src_stride + src_word_idx) as usize;
            let dst_row = (y * dst_stride) as usize;

            for w in 0..dst_stride {
                let s1 = collective_bitmap.data[src_row + w as usize];
                let s2 = collective_bitmap
                    .data
                    .get(src_row + w as usize + 1)
                    .copied()
                    .unwrap_or(0);

                let word: Word = if src_bit_offset == 0 {
                    s1
                } else {
                    (s1 << src_bit_offset) | (s2 >> (WORD_BITS - src_bit_offset))
                };

                pattern.data[dst_row + w as usize] = word;
            }

            // Clear trailing bits in the last word beyond `pattern_width`.
            let trailing = pattern_width % WORD_BITS;
            if trailing != 0 {
                let last = dst_row + (dst_stride - 1) as usize;
                pattern.data[last] &= !0 << (WORD_BITS - trailing);
            }
        }

        patterns.push(pattern);
    }

    // It turns out that when rendering patterns, the `Bitmap::combine` operation
    // can often be a huge bottleneck. This is because we need to do a lot of
    // fiddling to compose the patterns in the right position. And since
    // if a page uses such patterns, there's going to be a lot of them,
    // this takes up a lot of time.
    //
    // The key insight for our optimization is that patterns are usually
    // very small. Therefore, we precompute a new version of each pattern
    // for all possible 32-bit alignments, which later on allows us to
    // very easily blit it into the backdrop, significantly speeding up
    // the whole process.
    //
    // PdfCraft patch: only when the table is small, and fits what the page may still hold
    // (`room`), so that a speed-up never gets an image refused. It can be left out:
    // `shifted_pattern` then returns `None` and the patterns are combined the slow way.
    let shifted_words = (patterns.len() as u64)
        .saturating_mul(32 * 2)
        .saturating_mul(u64::from(pattern_height));
    let pattern_words = patterns.iter().map(|p| p.words() as u64).fold(0, u64::saturating_add);
    let shifted_patterns = if pattern_width <= WORD_BITS
        && pattern_height <= WORD_BITS
        && shifted_words <= MAX_SHIFTED_WORDS
        && shifted_words.saturating_add(pattern_words) <= room
    {
        let h = pattern_height as usize;
        let mut data = Vec::with_capacity(patterns.len() * 32 * h * 2);
        for pattern in &patterns {
            for k in 0..32_u32 {
                for y in 0..h {
                    let word = pattern.data[y];
                    if k == 0 {
                        data.push(word);
                        data.push(0);
                    } else {
                        data.push(word >> k);
                        data.push(word << (WORD_BITS - k));
                    }
                }
            }
        }
        data
    } else {
        Vec::new()
    };

    Ok(PatternDictionary {
        patterns,
        pattern_width,
        pattern_height,
        shifted_patterns,
    })
}

/// A decoded pattern dictionary.
#[derive(Debug, Clone)]
pub(crate) struct PatternDictionary {
    pub(crate) patterns: Vec<Bitmap>,
    pub(crate) pattern_width: u32,
    pub(crate) pattern_height: u32,
    shifted_patterns: Vec<Word>,
}

impl PatternDictionary {
    #[inline(always)]
    pub(crate) fn shifted_pattern(&self, pattern_idx: usize, bit_offset: u32) -> Option<&[Word]> {
        let h = self.pattern_height as usize;
        let len = h * 2;
        let start = (pattern_idx * 32 + bit_offset as usize) * len;
        self.shifted_patterns.get(start..start + len)
    }

    /// PdfCraft patch: the words the dictionary holds, its patterns and their shifted copies.
    pub(crate) fn words(&self) -> usize {
        self.patterns.iter().map(Bitmap::words).fold(self.shifted_patterns.len(), usize::saturating_add)
    }
}

/// Parsed pattern dictionary segment header (7.4.4.1).
#[derive(Debug, Clone)]
pub(crate) struct PatternDictionaryHeader<'a> {
    pub(crate) mmr: bool,
    pub(crate) template: Template,
    /// `HDPW`
    pub(crate) pattern_width: u8,
    /// `HDPH`
    pub(crate) pattern_height: u8,
    /// `GRAYMAX`
    pub(crate) num_patterns: u32,
    pub(crate) data: &'a [u8],
}

impl PatternDictionaryHeader<'_> {
    /// PdfCraft patch: the work of decoding the patterns, side by side in one bitmap, and of
    /// taking each out of it (see `bitmap::MAX_WORK`).
    pub(crate) fn work(&self) -> u64 {
        let (width, height) = (u32::from(self.pattern_width), u32::from(self.pattern_height));
        let patterns = u64::from(self.num_patterns) + 1;
        let side_by_side = u64::from(height) * (patterns.saturating_mul(u64::from(width)) + 4);
        side_by_side.saturating_add(patterns.saturating_mul(copy_work(width, height)))
    }
}

/// Parse a pattern dictionary segment header (7.4.4.1).
pub(crate) fn parse<'a>(reader: &mut Reader<'a>) -> Result<PatternDictionaryHeader<'a>> {
    let flags_byte = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let mmr = flags_byte & 0x01 != 0;
    let template = Template::from_byte(flags_byte >> 1);
    let pattern_width = reader
        .read_nonzero_byte()
        .ok_or(ParseError::UnexpectedEof)?;
    let pattern_height = reader
        .read_nonzero_byte()
        .ok_or(ParseError::UnexpectedEof)?;
    let num_patterns = reader.read_u32().ok_or(ParseError::UnexpectedEof)?;
    let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    Ok(PatternDictionaryHeader {
        mmr,
        template,
        pattern_width,
        pattern_height,
        num_patterns,
        data,
    })
}
