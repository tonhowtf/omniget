/*!
A memory-safe, pure-Rust JBIG2 decoder.

`hayro-jbig2` decodes JBIG2 images as specified in ITU-T T.88 (also known as
ISO/IEC 14492). JBIG2 is a bi-level image compression standard commonly used
in PDF documents for compressing scanned text documents.

The crate is `no_std` compatible but requires an allocator to be available.

# Safety
This crate forbids unsafe code via a crate-level attribute.
*/

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![allow(missing_docs)]

extern crate alloc;

use alloc::vec::Vec;

use crate::arithmetic_decoder::ArithmeticDecoderContext;

/// A reusable context for decoding JBIG2 images.
#[derive(Default)]
pub struct DecoderContext {
    pub(crate) page_state: PageState,
    pub(crate) scratch_buffers: ScratchBuffers,
    pub(crate) page_bitmap: Bitmap,
}

#[derive(Default)]
pub(crate) struct ScratchBuffers {
    pub(crate) contexts: Vec<ArithmeticDecoderContext>,
}

/// A decoder for JBIG2 images.
pub trait Decoder {
    /// Push a single pixel to the output.
    fn push_pixel(&mut self, black: bool);
    /// Push multiple chunks of 8 pixels of the same color.
    ///
    /// The `chunk_count` parameter indicates how many 8-pixel chunks to push.
    /// For example, if this method is called with `white = true` and
    /// `chunk_count = 10`, 80 white pixels are pushed (10 × 8 = 80).
    ///
    /// You can assume that this method is only called if the number of already
    /// pushed pixels is a multiple of 8 (i.e. byte-aligned).
    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32);
    /// Called when a row has been completed.
    fn next_line(&mut self);
}

#[cfg(feature = "image")]
pub mod integration;

mod arithmetic_decoder;
mod bitmap;
mod decode;
mod error;
mod file;
mod gray_scale;
mod huffman_table;
mod integer_decoder;
mod lazy;
mod page_info;
mod reader;
mod segment;
mod simd;
mod symbol_id_decoder;

use error::bail;
pub use error::{
    DecodeError, FormatError, HuffmanError, OverflowError, ParseError, RegionError, Result,
    SegmentError, SymbolError, TemplateError,
};

use crate::file::{SegmentCount, parse_segments_sequential};
use bitmap::Bitmap;
use decode::CombinationOperator;
use decode::generic;
use decode::generic_refinement;
use decode::halftone;
use decode::pattern;
use decode::pattern::PatternDictionary;
use decode::symbol;
use decode::symbol::SymbolDictionary;
use decode::text;
use file::parse_file;
use huffman_table::{HuffmanTable, StandardHuffmanTables};
use page_info::{PageInformation, parse_page_information};
use reader::Reader;
use segment::SegmentType;

/// A JBIG2 image.
pub struct Image<'a> {
    /// The parsed segments.
    segments: Vec<segment::Segment<'a>>,
    /// The width of the image in pixels.
    width: u32,
    /// The height of the image in pixels.
    height: u32,
    /// The height determined from `EndOfStripe` segments, if applicable.
    height_from_stripes: Option<u32>,
}

impl<'a> Image<'a> {
    /// Parse a JBIG2 file from the given data.
    ///
    /// The file is expected to use the sequential or random-access organization,
    /// as defined in Annex D.1 and D.2.
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let file = parse_file(data)?;
        Self::from_segments(file.segments)
    }

    /// Parse an embedded JBIG2 image with optional global segments.
    ///
    /// The file is expected to use the embedded organization defined in
    /// Annex D.3.
    pub fn new_embedded(data: &'a [u8], globals: Option<&'a [u8]>) -> Result<Self> {
        let mut segments = Vec::new();
        let mut count = SegmentCount::default();
        if let Some(globals_data) = globals {
            let mut reader = Reader::new(globals_data);
            parse_segments_sequential(&mut reader, &mut segments, &mut count)?;
        };

        let mut reader = Reader::new(data);
        parse_segments_sequential(&mut reader, &mut segments, &mut count)?;

        segments.sort_by_key(|seg| seg.header.segment_number);

        Self::from_segments(segments)
    }

    fn from_segments(segments: Vec<segment::Segment<'a>>) -> Result<Self> {
        // Pre-scan for stripe height from EndOfStripe segments.
        let height_from_stripes = segments
            .iter()
            .filter(|seg| seg.header.segment_type == SegmentType::EndOfStripe)
            .filter_map(|seg| u32::from_be_bytes(seg.data.try_into().ok()?).checked_add(1))
            .max();

        // Find and parse page information to extract dimensions.
        let page_info_seg = segments
            .iter()
            .find(|s| s.header.segment_type == SegmentType::PageInformation)
            .ok_or(FormatError::MissingPageInfo)?;

        let mut reader = Reader::new(page_info_seg.data);
        let page_info = parse_page_information(&mut reader)?;

        // "A page's bitmap height may be declared in its page information segment
        // to be unknown (by specifying a height of 0xFFFFFFFF). In this case, the
        // page must be striped." (7.4.8.2)
        let height = if page_info.height == 0xFFFF_FFFF {
            height_from_stripes.ok_or(FormatError::UnknownPageHeight)?
        } else {
            page_info.height
        };

        if page_info.width == 0 || height == 0 {
            bail!(FormatError::EmptyPage);
        }
        bitmap::check_page_size(page_info.width, height)?;

        Ok(Self {
            segments,
            width: page_info.width,
            height,
            height_from_stripes,
        })
    }

    /// The width of the image in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height of the image in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Decode the image data through the given [`Decoder`].
    pub fn decode<D: Decoder>(&self, decoder: &mut D) -> Result<()> {
        let mut ctx = DecoderContext::default();

        self.decode_with(decoder, &mut ctx)
    }

    /// Decode the image data through the given [`Decoder`] and [`DecoderContext`].
    ///
    /// This is useful in case you want to convert multiple JBIG2 images,
    /// as it allows `hayro-jbig2` to reuse allocations during decoding.
    pub fn decode_with<D: Decoder>(&self, decoder: &mut D, ctx: &mut DecoderContext) -> Result<()> {
        let mut work_left = bitmap::MAX_WORK;
        decode_segments(&self.segments, self.height_from_stripes, ctx, &mut work_left)?;
        emit_bitmap(&ctx.page_bitmap, decoder);

        Ok(())
    }

    /// PdfCraft patch: [`Image::decode`], spending from `work`, what decoding may still cost
    /// across a run of images. An image spends what it took, failing or not, starting with its
    /// page's pixels, which whoever asked for them will go through, and its segments, so images
    /// that decode nothing add up too. On top of that, it takes at most [`bitmap::MAX_WORK`]
    /// whatever `work` says. One that `work` can't pay for spends all of it, so a run ends rather
    /// than trying every image it has left. Reading the stream comes before this and is charged
    /// by [`charge_reading`].
    pub fn decode_within<D: Decoder>(&self, decoder: &mut D, work: &mut u64) -> Result<()> {
        let start = bitmap::decode_work(self.width, self.height).saturating_add(self.segments.len() as u64);
        if charge_work(work, start).is_err() {
            *work = 0;
            bail!(OverflowError::DecodingWork);
        }
        let allowed = bitmap::MAX_WORK.min(*work);
        let mut left = allowed;
        let mut ctx = DecoderContext::default();
        let decoded = decode_segments(&self.segments, self.height_from_stripes, &mut ctx, &mut left);
        let refused = decoded == Err(OverflowError::DecodingWork.into());
        *work = if refused && allowed < bitmap::MAX_WORK { 0 } else { *work - (allowed - left) };
        decoded?;
        emit_bitmap(&ctx.page_bitmap, &mut *decoder);

        Ok(())
    }
}

fn emit_bitmap<D: Decoder>(bitmap: &Bitmap, decoder: &mut D) {
    let width = bitmap.width;
    let bytes_per_row = width.div_ceil(8) as usize;

    for row in bitmap.data.chunks_exact(bitmap.stride as usize) {
        let mut x = 0_u32;
        let mut chunk_byte: Option<u8> = None;
        let mut chunk_count = 0_u32;

        let bytes = row.iter().flat_map(|w| w.to_be_bytes()).take(bytes_per_row);

        for byte in bytes {
            let remaining = width - x;

            if remaining >= 8 && (byte == 0x00 || byte == 0xFF) {
                // Continue the previous chunk.
                if chunk_byte == Some(byte) {
                    chunk_count += 1;
                    x += 8;
                    continue;
                }

                // Flush previous chunk if any, then start new one.
                if let Some(b) = chunk_byte {
                    decoder.push_pixel_chunk(b == 0xFF, chunk_count);
                }

                chunk_byte = Some(byte);
                chunk_count = 1;
                x += 8;

                continue;
            }

            // Can't continue/start chunk, flush any existing chunk first.
            if let Some(b) = chunk_byte.take() {
                decoder.push_pixel_chunk(b == 0xFF, chunk_count);
                chunk_count = 0;
            }

            // Emit individual pixels.
            let count = remaining.min(8);
            for i in 0..count {
                decoder.push_pixel((byte >> (7 - i)) & 1 != 0);
            }
            x += count;
        }

        // Flush any remaining chunk at end of row.
        if let Some(b) = chunk_byte {
            decoder.push_pixel_chunk(b == 0xFF, chunk_count);
        }

        decoder.next_line();
    }
}

/// PdfCraft patch: count `work` more decoding work, refusing past what is `left` of
/// [`bitmap::MAX_WORK`] or a run's allowance before what asks for it is decoded.
fn charge_work(left: &mut u64, work: u64) -> Result<()> {
    *left = left.checked_sub(work).ok_or(OverflowError::DecodingWork)?;
    Ok(())
}

/// PdfCraft patch: the steps of reading a byte of a stream or its globals: parsing and sorting
/// segments, and building their tables, measured at about 3 a byte at the worst.
const READ_WORK: u64 = 4;

/// PdfCraft patch: charge `work`, what a run of images may still cost, for reading `bytes` of a stream
/// and its globals. This comes before anything is parsed, as it counts whether or not they turn out
/// to decode: an image that fails has still cost its reading, and it is read again for each use. What
/// `work` can't pay for spends all of it.
pub fn charge_reading(work: &mut u64, bytes: usize) -> Result<()> {
    if charge_work(work, (bytes as u64).saturating_mul(READ_WORK)).is_err() {
        *work = 0;
        bail!(OverflowError::DecodingWork);
    }
    Ok(())
}

/// PdfCraft patch: the work of a region decoded pixel by pixel.
fn region_work(region: &decode::RegionSegmentInfo) -> u64 {
    bitmap::decode_work(region.width, region.height)
}

/// PdfCraft patch: the page restricted to a refinement's area, its reference when it refers to no
/// other region (7.4.7.4). Copying it a word at a time costs less than the region's own decoding,
/// already charged, and it is held, as large as the region, only while the region decodes.
fn page_area(page: &Bitmap, region: &decode::RegionSegmentInfo) -> Result<Bitmap> {
    page.area(region.x_location, region.y_location, region.width, region.height)
}

fn decode_segments(
    segments: &[segment::Segment<'_>],
    height_from_stripes: Option<u32>,
    decoder_ctx: &mut DecoderContext,
    work_left: &mut u64,
) -> Result<()> {
    // Find and parse page information segment first.
    if let Some(page_info) = segments
        .iter()
        .find(|s| s.header.segment_type == SegmentType::PageInformation)
    {
        let mut reader = Reader::new(page_info.data);
        init_page(
            &mut reader,
            height_from_stripes,
            &mut decoder_ctx.page_state,
            &mut decoder_ctx.page_bitmap,
        )?;
    } else {
        bail!(FormatError::MissingPageInfo);
    }

    let page_bitmap = &mut decoder_ctx.page_bitmap;
    let page_state = &mut decoder_ctx.page_state;
    let scratch_buffers = &mut decoder_ctx.scratch_buffers;

    // Process all segments.
    for seg in segments {
        let mut reader = Reader::new(seg.data);

        match seg.header.segment_type {
            SegmentType::PageInformation => {
                // Already processed above, skip.
            }
            SegmentType::ImmediateGenericRegion | SegmentType::ImmediateLosslessGenericRegion => {
                let had_unknown_length = seg.header.data_length.is_none();
                let header = generic::parse(&mut reader, had_unknown_length)?;
                charge_work(work_left, region_work(&header.region_info))?;

                if page_state.can_decode_directly(page_bitmap, &header.region_info, false) {
                    generic::decode_into(&header, page_bitmap, scratch_buffers)?;
                } else {
                    let region = generic::decode(&header, scratch_buffers)?;
                    page_bitmap.combine_region(&region.bitmap, region.combination_operator);
                }
                page_state.page_pristine = false;
            }
            SegmentType::IntermediateGenericRegion => {
                // Intermediate segments cannot have unknown length.
                let header = generic::parse(&mut reader, false)?;
                charge_work(work_left, region_work(&header.region_info))?;
                let region = generic::decode(&header, scratch_buffers)?;
                page_state.store_region(seg.header.segment_number, region.bitmap)?;
            }
            SegmentType::PatternDictionary => {
                let header = pattern::parse(&mut reader)?;
                charge_work(work_left, header.work())?;
                let dictionary = pattern::decode(&header, scratch_buffers, page_state.retained_room())?;
                page_state.store_pattern_dictionary(seg.header.segment_number, dictionary)?;
            }
            SegmentType::SymbolDictionary => {
                // "1) Concatenate all the input symbol dictionaries to form SDINSYMS."
                // (6.5.5, step 1)
                // Collect references to avoid cloning; symbols are only cloned if re-exported.
                let input_symbols = page_state.referred_symbols(&seg.header.referred_to_segments)?;
                charge_work(work_left, input_symbols.len() as u64)?;

                // Collect Huffman tables from referred table segments.
                let referred_tables: Vec<HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| page_state.get_huffman_table(num))
                    .cloned()
                    .collect();

                // Get retained contexts from the last referred symbol dictionary (7.4.2.2 step 3).
                let retained_contexts = seg
                    .header
                    .referred_to_segments
                    .last()
                    .and_then(|&num| page_state.get_symbol_dictionary(num))
                    .and_then(|dict| dict.retained_contexts.as_ref());

                let header = symbol::parse(&mut reader)?;
                let dictionary = symbol::decode(
                    &header,
                    &input_symbols,
                    &referred_tables,
                    &page_state.standard_tables,
                    retained_contexts,
                    page_state.retained_room(),
                    work_left,
                )?;
                page_state.store_symbol_dictionary(seg.header.segment_number, dictionary)?;
            }
            SegmentType::ImmediateTextRegion | SegmentType::ImmediateLosslessTextRegion => {
                // Collect symbols from referred symbol dictionaries (SBSYMS).
                let symbols = page_state.referred_symbols(&seg.header.referred_to_segments)?;
                charge_work(work_left, symbols.len() as u64)?;

                // Collect Huffman tables from referred table segments.
                // "These user-supplied Huffman decoding tables may be supplied either
                // as a Tables segment..." (7.4.3.1.6)
                let referred_tables: Vec<HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| page_state.get_huffman_table(num))
                    .cloned()
                    .collect();

                let header = text::parse(&mut reader, symbols.len() as u32)?;
                charge_work(work_left, bitmap::copy_work(header.region_info.width, header.region_info.height))?;
                if header.flags.use_huffman {
                    // Its symbol ID table has a line for each symbol.
                    charge_work(work_left, (symbols.len() as u64).saturating_mul(bitmap::HUFFMAN_LINE_WORK))?;
                }

                if page_state.can_decode_directly(
                    page_bitmap,
                    &header.region_info,
                    header.flags.default_pixel,
                ) {
                    text::decode_into(
                        &header,
                        &symbols,
                        &referred_tables,
                        &page_state.standard_tables,
                        page_bitmap,
                        scratch_buffers,
                        work_left,
                    )?;
                } else {
                    let region = text::decode(
                        &header,
                        &symbols,
                        &referred_tables,
                        &page_state.standard_tables,
                        scratch_buffers,
                        work_left,
                    )?;
                    page_bitmap.combine_region(&region.bitmap, region.combination_operator);
                }
                page_state.page_pristine = false;
            }
            SegmentType::IntermediateTextRegion => {
                // Collect symbols from referred symbol dictionaries (SBSYMS).
                let symbols = page_state.referred_symbols(&seg.header.referred_to_segments)?;
                charge_work(work_left, symbols.len() as u64)?;

                // Collect Huffman tables from referred table segments.
                let referred_tables: Vec<HuffmanTable> = seg
                    .header
                    .referred_to_segments
                    .iter()
                    .filter_map(|&num| page_state.get_huffman_table(num))
                    .cloned()
                    .collect();

                let header = text::parse(&mut reader, symbols.len() as u32)?;
                charge_work(work_left, bitmap::copy_work(header.region_info.width, header.region_info.height))?;
                if header.flags.use_huffman {
                    // Its symbol ID table has a line for each symbol.
                    charge_work(work_left, (symbols.len() as u64).saturating_mul(bitmap::HUFFMAN_LINE_WORK))?;
                }
                let region = text::decode(
                    &header,
                    &symbols,
                    &referred_tables,
                    &page_state.standard_tables,
                    scratch_buffers,
                    work_left,
                )?;
                page_state.store_region(seg.header.segment_number, region.bitmap)?;
            }
            SegmentType::ImmediateHalftoneRegion | SegmentType::ImmediateLosslessHalftoneRegion => {
                let pattern_dict = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| page_state.get_pattern_dictionary(num))
                    .ok_or(SegmentError::MissingPatternDictionary)?;

                let header = halftone::parse(&mut reader)?;
                charge_work(work_left, halftone::work(&header, pattern_dict))?;

                if page_state.can_decode_directly(
                    page_bitmap,
                    &header.region_info,
                    header.flags.initial_pixel_color,
                ) {
                    halftone::decode_into(&header, pattern_dict, page_bitmap, scratch_buffers)?;
                } else {
                    let region = halftone::decode(&header, pattern_dict, scratch_buffers)?;
                    page_bitmap.combine_region(&region.bitmap, region.combination_operator);
                }
                page_state.page_pristine = false;
            }
            SegmentType::IntermediateHalftoneRegion => {
                let pattern_dict = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| page_state.get_pattern_dictionary(num))
                    .ok_or(SegmentError::MissingPatternDictionary)?;

                let header = halftone::parse(&mut reader)?;
                charge_work(work_left, halftone::work(&header, pattern_dict))?;
                let region = halftone::decode(&header, pattern_dict, scratch_buffers)?;
                page_state.store_region(seg.header.segment_number, region.bitmap)?;
            }
            SegmentType::IntermediateGenericRefinementRegion => {
                // Same logic as immediate refinement, but store result instead of combining.
                let referred_segment = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| page_state.get_referred_segment(num));

                let header = generic_refinement::parse(&mut reader)?;
                charge_work(work_left, region_work(&header.region_info))?;
                // PdfCraft patch: a refinement of the page refines its area (`Bitmap::area`).
                let area;
                let reference = match referred_segment {
                    Some(region) => region,
                    None => {
                        area = page_area(page_bitmap, &header.region_info)?;
                        &area
                    }
                };
                let region = generic_refinement::decode(&header, reference, scratch_buffers)?;
                page_state.store_region(seg.header.segment_number, region.bitmap)?;
            }
            SegmentType::ImmediateGenericRefinementRegion
            | SegmentType::ImmediateLosslessGenericRefinementRegion => {
                // "3) Determine the buffer associated with the region segment that
                // this segment refers to." (7.4.7.5)
                //
                // With no region segment referred to, the reference is the page restricted to
                // this segment's area (7.4.7.4).
                let referred_segment = seg
                    .header
                    .referred_to_segments
                    .first()
                    .and_then(|&num| page_state.get_referred_segment(num));

                let header = generic_refinement::parse(&mut reader)?;
                charge_work(work_left, region_work(&header.region_info))?;

                if let Some(referred_segment) = referred_segment
                    && page_state.can_decode_directly(page_bitmap, &header.region_info, false)
                {
                    generic_refinement::decode_into(
                        &header,
                        referred_segment,
                        page_bitmap,
                        scratch_buffers,
                    )?;
                } else {
                    // PdfCraft patch: a refinement of the page refines its area (`Bitmap::area`).
                    let area;
                    let reference = match referred_segment {
                        Some(region) => region,
                        None => {
                            area = page_area(page_bitmap, &header.region_info)?;
                            &area
                        }
                    };
                    let region = generic_refinement::decode(&header, reference, scratch_buffers)?;
                    page_bitmap.combine_region(&region.bitmap, region.combination_operator);
                }
                page_state.page_pristine = false;
            }
            SegmentType::Tables => {
                // "Tables – see 7.4.13." (type 53)
                // "This segment contains data which defines one or more user-supplied
                // Huffman coding tables." (7.4.13)
                let table = HuffmanTable::read_custom(&mut reader)?;
                page_state.store_huffman_table(seg.header.segment_number, table)?;
            }
            SegmentType::EndOfPage | SegmentType::EndOfFile => {
                break;
            }
            // Other segment types not yet implemented.
            _ => {}
        }
    }

    Ok(())
}

/// PdfCraft patch: the words a stored item costs in its list, at least, apart from what it holds.
const ENTRY_WORDS: u64 = 32;

/// PdfCraft patch: the most words a page's segments may hold for later ones, 64 MiB: two square
/// bitmaps of 16,384 a side (`bitmap::MAX_PIXELS`). Each stored item is bounded on its own, but without a
/// bound on all of them a stream of a few dozen bytes a segment held 33 MB each (a 16,384-sample
/// square region of typical rows), and a few kilobytes could exhaust memory. A segment being
/// decoded takes more for a while, within its own bounds: the page bitmap isn't counted, nor is
/// what a region or a grid uses until it is stored or freed.
const MAX_RETAINED_WORDS: u64 =
    2 * (bitmap::MAX_PIXELS / bitmap::WORD_BITS as u64 + bitmap::BITMAP_WORDS as u64 + ENTRY_WORDS);

/// PdfCraft patch: the most symbols one segment may refer to, 2^20, far beyond the 65,535 a
/// dictionary may export. References may repeat, so each names the same symbols again: without
/// this, 65,535 references to one dictionary in a 78 KB stream collected 17 GB of pointers.
const MAX_REFERRED_SYMBOLS: u64 = 1 << 20;

/// Page-level decoding state for a JBIG2 page.
#[derive(Default)]
pub(crate) struct PageState {
    /// The parsed page information.
    pub(crate) page_info: PageInformation,
    /// Whether the page bitmap is still in its initial state (not yet painted to).
    pub(crate) page_pristine: bool,
    /// Decoded intermediate regions, stored as (`segment_number`, region) pairs.
    pub(crate) referred_segments: Vec<(u32, Bitmap)>,
    /// Decoded pattern dictionaries, stored as (`segment_number`, dictionary) pairs.
    pub(crate) pattern_dictionaries: Vec<(u32, PatternDictionary)>,
    /// Decoded symbol dictionaries, stored as (`segment_number`, dictionary) pairs.
    pub(crate) symbol_dictionaries: Vec<(u32, SymbolDictionary)>,
    /// Decoded Huffman tables from table segments, stored as (`segment_number`, table) pairs.
    /// "Tables – see 7.4.13." (type 53)
    pub(crate) huffman_tables: Vec<(u32, HuffmanTable)>,
    /// PdfCraft patch: the words the regions, dictionaries and tables above hold, at most
    /// [`MAX_RETAINED_WORDS`].
    pub(crate) retained_words: u64,
    /// Standard Huffman tables (`TABLE_A` through `TABLE_O`).
    pub(crate) standard_tables: StandardHuffmanTables,
}

impl PageState {
    fn reset(&mut self, page_info: PageInformation) {
        self.page_info = page_info;
        self.page_pristine = true;
        self.referred_segments.clear();
        self.pattern_dictionaries.clear();
        self.symbol_dictionaries.clear();
        self.huffman_tables.clear();
        self.retained_words = 0;
        // Standard tables are lazily built and reused across images.
    }

    /// Check if an immediate region can be decoded directly into the page bitmap.
    fn can_decode_directly(
        &self,
        page_bitmap: &Bitmap,
        region_info: &decode::RegionSegmentInfo,
        region_default_pixel: bool,
    ) -> bool {
        if !self.page_pristine {
            return false;
        }

        let covers_page = region_info.x_location == 0
            && region_info.y_location == 0
            && region_info.width == page_bitmap.width
            && region_info.height == page_bitmap.height;

        if !covers_page {
            return false;
        }

        let page_default_is_zero = self.page_info.flags.default_pixel == 0;

        if region_default_pixel == page_default_is_zero {
            return false;
        }

        let op = region_info.combination_operator;
        match op {
            CombinationOperator::Replace => true,
            CombinationOperator::Or | CombinationOperator::Xor => page_default_is_zero,
            CombinationOperator::And | CombinationOperator::Xnor => !page_default_is_zero,
        }
    }

    /// PdfCraft patch: the words the page may still hold for later segments.
    fn retained_room(&self) -> u64 {
        MAX_RETAINED_WORDS.saturating_sub(self.retained_words)
    }

    /// PdfCraft patch: count a stored item holding `words` for later segments, refusing past
    /// [`MAX_RETAINED_WORDS`].
    fn retain(&mut self, words: usize) -> Result<()> {
        self.retained_words = self.retained_words.saturating_add(words as u64).saturating_add(ENTRY_WORDS);
        if self.retained_words > MAX_RETAINED_WORDS {
            bail!(OverflowError::RetainedBitmaps);
        }
        Ok(())
    }

    /// Store a decoded region for later reference.
    fn store_region(&mut self, segment_number: u32, region: Bitmap) -> Result<()> {
        self.retain(region.words())?;
        self.referred_segments.push((segment_number, region));
        Ok(())
    }

    /// Look up a referred segment by number.
    fn get_referred_segment(&self, segment_number: u32) -> Option<&Bitmap> {
        self.referred_segments
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.referred_segments[idx].1)
    }

    /// Store a decoded pattern dictionary for later reference.
    fn store_pattern_dictionary(&mut self, segment_number: u32, dictionary: PatternDictionary) -> Result<()> {
        self.retain(dictionary.words())?;
        self.pattern_dictionaries.push((segment_number, dictionary));
        Ok(())
    }

    /// Look up a pattern dictionary by segment number.
    fn get_pattern_dictionary(&self, segment_number: u32) -> Option<&PatternDictionary> {
        self.pattern_dictionaries
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.pattern_dictionaries[idx].1)
    }

    /// Store a decoded symbol dictionary for later reference.
    fn store_symbol_dictionary(&mut self, segment_number: u32, dictionary: SymbolDictionary) -> Result<()> {
        self.retain(dictionary.words())?;
        self.symbol_dictionaries.push((segment_number, dictionary));
        Ok(())
    }

    /// PdfCraft patch: the symbols of the dictionaries in `segments`, in order, refusing past
    /// [`MAX_REFERRED_SYMBOLS`] before anything is collected.
    fn referred_symbols(&self, segments: &[u32]) -> Result<Vec<&Bitmap>> {
        let dictionaries = || segments.iter().filter_map(|&num| self.get_symbol_dictionary(num));
        let count = dictionaries()
            .map(|dictionary| dictionary.exported_symbols.len() as u64)
            .fold(0, u64::saturating_add);
        if count > MAX_REFERRED_SYMBOLS {
            bail!(OverflowError::SymbolCount);
        }
        Ok(dictionaries().flat_map(|dictionary| dictionary.exported_symbols.iter()).collect())
    }

    /// Look up a symbol dictionary by segment number.
    fn get_symbol_dictionary(&self, segment_number: u32) -> Option<&SymbolDictionary> {
        self.symbol_dictionaries
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.symbol_dictionaries[idx].1)
    }

    /// Store a decoded Huffman table for later reference.
    fn store_huffman_table(&mut self, segment_number: u32, table: HuffmanTable) -> Result<()> {
        self.retain(table.words())?;
        self.huffman_tables.push((segment_number, table));
        Ok(())
    }

    /// Look up a Huffman table by segment number.
    fn get_huffman_table(&self, segment_number: u32) -> Option<&HuffmanTable> {
        self.huffman_tables
            .binary_search_by_key(&segment_number, |(num, _)| *num)
            .ok()
            .map(|idx| &self.huffman_tables[idx].1)
    }
}

/// Parse page information and initialize the page bitmap.
fn init_page(
    reader: &mut Reader<'_>,
    height_from_stripes: Option<u32>,
    page: &mut PageState,
    bitmap: &mut Bitmap,
) -> Result<()> {
    let page_info = parse_page_information(reader)?;

    // "A page's bitmap height may be declared in its page information segment
    // to be unknown (by specifying a height of 0xFFFFFFFF). In this case, the
    // page must be striped." (7.4.8.2)
    let height = if page_info.height == 0xFFFF_FFFF {
        height_from_stripes.ok_or(FormatError::UnknownPageHeight)?
    } else {
        page_info.height
    };

    // "Bit 2: Page default pixel value. This bit contains the initial value
    // for every pixel in the page, before any region segments are decoded
    // or drawn." (7.4.8.5)
    bitmap.reinitialize(page_info.width, height, page_info.flags.default_pixel != 0)?;

    page.reset(page_info);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page past the bounds is refused when the stream is opened, since callers size their
    /// output from it before decoding starts. 16,384 squared is exactly the bound on pixels.
    #[test]
    fn a_page_past_the_bounds_is_refused_when_opened() {
        let stream = |width: u32, height: u32| {
            let mut stream = vec![0, 0, 0, 0, 48, 0, 1, 0, 0, 0, 19];
            stream.extend_from_slice(&width.to_be_bytes());
            stream.extend_from_slice(&height.to_be_bytes());
            stream.extend_from_slice(&[0; 11]); // resolution, flags, striping
            stream
        };
        assert!(Image::new_embedded(&stream(16384, 16384), None).is_ok());
        assert!(Image::new_embedded(&stream(16385, 16384), None).is_err());
        assert!(Image::new_embedded(&stream(65536, 1), None).is_err(), "too wide, though few pixels");
        assert!(Image::new_embedded(&stream(1 << 20, 1 << 20), None).is_err());
    }

    /// Segment `number` of `kind` on page 1, referring to `refers_to`.
    fn segment(number: u8, kind: u8, refers_to: &[u8], data: &[u8]) -> Vec<u8> {
        let mut segment = vec![0, 0, 0, number, kind, (refers_to.len() as u8) << 5];
        segment.extend_from_slice(refers_to);
        segment.push(1);
        segment.extend_from_slice(&(data.len() as u32).to_be_bytes());
        segment.extend_from_slice(data);
        segment
    }

    /// A blank page of 8 × 8: resolution unknown, flags, not striped.
    fn page_of_8() -> Vec<u8> {
        segment(0, 48, &[], &[0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0x42, 0, 0])
    }

    /// Region information (7.4.1) of 4 × 4 at `x`, `y`, then the region's own `header`, then
    /// arbitrary coded data.
    fn region_of_4(x: u32, y: u32, operator: u8, header: &[u8]) -> Vec<u8> {
        let mut data: Vec<u8> = [4u32, 4, x, y].iter().flat_map(|v| v.to_be_bytes()).collect();
        data.push(operator);
        data.extend_from_slice(header);
        data.extend_from_slice(&[0x5a; 16]);
        data
    }

    const GENERIC: &[u8] = &[0, 3, 0xFF, 0xFD, 0xFF, 2, 0xFE, 0xFE, 0xFE]; // template 0, AT
    const REFINEMENT: &[u8] = &[0, 0xFF, 0xFF, 0xFF, 0xFF]; // template 0, AT
    const OR: u8 = 0;
    const REPLACE: u8 = 4;

    /// PdfCraft patch: a region's location is unsigned (5.4.4), so one past `i32::MAX` is off the
    /// page. Taken for a signed one, a region at 0xFFFFFFFE drew back onto the page two pixels left
    /// of it, and a refinement there was refused, and the rest of the stream with it. Each region
    /// here is 4 × 4 on a blank page of 8 × 8, its coded data arbitrary: on the page it draws, off
    /// it nothing.
    #[test]
    fn a_region_located_past_i32_max_is_off_the_page() {
        struct Ink(u32);
        impl Decoder for Ink {
            fn push_pixel(&mut self, black: bool) {
                self.0 += u32::from(black);
            }
            fn push_pixel_chunk(&mut self, black: bool, chunks: u32) {
                self.0 += if black { 8 * chunks } else { 0 };
            }
            fn next_line(&mut self) {}
        }
        let ink = |x: u32, y: u32| {
            let forms = [
                // An immediate generic region.
                vec![segment(1, 38, &[], &region_of_4(x, y, OR, GENERIC))],
                // An immediate refinement of the page, which reads the page at its own location (7.4.7.4).
                vec![segment(1, 42, &[], &region_of_4(x, y, REPLACE, REFINEMENT))],
                // An immediate refinement of an intermediate region at the same place (7.4.7.5).
                vec![
                    segment(1, 36, &[], &region_of_4(x, y, OR, GENERIC)),
                    segment(2, 42, &[1], &region_of_4(x, y, OR, REFINEMENT)),
                ],
            ];
            forms.map(|form| {
                let stream = [page_of_8(), form.concat()].concat();
                let mut ink = Ink(0);
                Image::new_embedded(&stream, None).and_then(|image| image.decode(&mut ink)).map(|()| ink.0)
            })
        };
        for (x, y) in [(0, 0), (2, 2)] {
            assert!(ink(x, y).iter().all(|ink| matches!(ink, Ok(n) if *n > 0)), "{x}, {y}: {:?}", ink(x, y));
        }
        for (x, y) in [(0xFFFF_FFFE, 0), (0, 0xFFFF_FFFE), (0x8000_0000, 0x8000_0000), (u32::MAX, u32::MAX)] {
            assert_eq!(ink(x, y).map(|ink| ink.ok()), [Some(0); 3], "{x:#x}, {y:#x}");
        }
    }

    /// PdfCraft patch: a refinement of a region refines it where it is, "GRREFERENCEDX 0,
    /// GRREFERENCEDY 0" (Table 35), not the page at the refinement's location. A region and its
    /// refinement, directly or through an intermediate refinement, decode otherwise than the same
    /// refinements of the page, and at 2, 3 to the pixels they decode to at 0, 0, moved.
    #[test]
    fn a_refinement_of_a_region_decodes_the_same_wherever_they_are() {
        #[derive(Default)]
        struct Pixels(Vec<bool>);
        impl Decoder for Pixels {
            fn push_pixel(&mut self, black: bool) {
                self.0.push(black);
            }
            fn push_pixel_chunk(&mut self, black: bool, chunks: u32) {
                self.0.extend((0..8 * chunks).map(|_| black));
            }
            fn next_line(&mut self) {}
        }
        // The first refinement refers to the region before it, or `of_page`, to nothing: the page.
        let decode = |x: u32, y: u32, through_intermediate: bool, of_page: bool| {
            let first: &[u8] = if of_page { &[] } else { &[1] };
            let mut form = vec![segment(1, 36, &[], &region_of_4(x, y, OR, GENERIC))];
            if through_intermediate {
                form.push(segment(2, 40, first, &region_of_4(x, y, OR, REFINEMENT)));
                form.push(segment(3, 42, &[2], &region_of_4(x, y, OR, REFINEMENT)));
            } else {
                form.push(segment(2, 42, first, &region_of_4(x, y, OR, REFINEMENT)));
            }
            let stream = [page_of_8(), form.concat()].concat();
            let mut pixels = Pixels::default();
            Image::new_embedded(&stream, None).unwrap().decode(&mut pixels).unwrap();
            pixels.0
        };
        for through_intermediate in [false, true] {
            let at_origin = decode(0, 0, through_intermediate, false);
            assert!(at_origin.contains(&true), "draws");
            assert_ne!(at_origin, decode(0, 0, through_intermediate, true), "the page refined instead");
            for (i, &black) in decode(2, 3, through_intermediate, false).iter().enumerate() {
                let (x, y) = (i % 8, i / 8);
                let moved = x >= 2 && y >= 3 && at_origin[(y - 3) * 8 + x - 2];
                assert_eq!(black, moved, "{x}, {y}, through an intermediate refinement: {through_intermediate}");
            }
        }
    }

    /// PdfCraft patch: a refinement of the page refines the page restricted to its area (7.4.7.4),
    /// reading 0 around it, and a reference 32 or more left of a refinement is read from where each
    /// row starts in it. So that a context read wrong changes what decodes, the refinements are
    /// large, and their pictures come from the hash they were coded from: a 64 × 16 refinement at
    /// 41, 4 of a 112 × 24 page, and a 70 × 12 text instance refined from a 140 × 12 symbol 35
    /// left of it.
    #[test]
    fn a_refinement_refines_the_page_in_its_area_and_any_reference_where_it_is() {
        #[derive(Default)]
        struct Rows(Vec<Vec<u8>>, Vec<u8>);
        impl Decoder for Rows {
            fn push_pixel(&mut self, black: bool) {
                self.1.push(u8::from(black));
            }
            fn push_pixel_chunk(&mut self, black: bool, chunks: u32) {
                for _ in 0..8 * chunks {
                    self.push_pixel(black);
                }
            }
            fn next_line(&mut self) {
                self.0.push(core::mem::take(&mut self.1));
            }
        }
        fn bit(seed: u32, x: u32, y: u32) -> u8 {
            let mut h = x.wrapping_mul(0x9E37_79B1).wrapping_add(y.wrapping_mul(0x85EB_CA77)).wrapping_add(seed.wrapping_mul(0xC2B2_AE3D));
            h ^= h >> 15;
            h = h.wrapping_mul(0x2C1B_3C6D);
            h ^= h >> 12;
            h = h.wrapping_mul(0x297A_2D39);
            h ^= h >> 15;
            ((h >> 7) & 1) as u8
        }
        // About one pixel in eight: where a refinement differs from its reference.
        let flipped = |seed: u32, x: u32, y: u32| bit(seed, x, y) & bit(seed + 1, x, y) & bit(seed + 2, x, y);
        // A page of bit(1, ..), and over it at 41, 4 the area refined from it.
        const PAGE: &str = "000000003000010000001300000070000000180000000000000000420000000000012600010000016f00000070000000180000000000000000040003fffdff02fefefefb5d354db053c2a984c01af66c619b02c245055be23a2a431b04c60325c88676f6ff1b0a68c873ad9011a27a89fc99abc7db2e8f529ecf198ff8c6a726ddcb746dc51f3812e365e74ee668fa02f59182cae39f0ef43b3201da1bcd43a21f449889e15ba0ba1a9344da1bdb9dcb4015279a39d944509febc622a235fa2c623acb443f0c10dabf7fba5e9d6fef2f1289dfd0a6a3801d036f21a62045074f5d3158ab0373ad2334102ef867dc3132dc8d02fe517771c419cdbad47d67d409ec89d0425d1f51a48af6ba3bee6e0701185bfa5f46b347b0ec39e744b665d1e8aac868838d3db25162d65534ff4771d9c0d657bd48d5814fa7cc184d0e961defab2d75a79a88345cdae5546a54221a22ddfe9c8a0514f32ac9de806fbc5fb4f5275c79d679f0258f77900daaa273cee50e50c958f323414670d7761418639342fdb2082eac5e7934f1a06169991452b1a042eaee7871ffac000000022a000100000091000000400000001000000029000000040400ffffffffd24567306e52f57232cefa9800db00fb60c21b30b0e53d9fac8a1f863a580a7cc49ab401934ac8be3a18e2dc0f01774b76c3e9196283cb5504742c763a867bc1aae8c48d74d194dacda03ce7e91904897ab61add273d9188fda65dc6fefb995d85f48142e43c1970bedb9c5f4be3cb00d458e9f4ee9714a61fffac";
        let page: Vec<Vec<u8>> = (0..24)
            .map(|y| (0..112).map(|x| bit(1, x, y) ^ if (41..105).contains(&x) && (4..20).contains(&y) { flipped(10, x - 41, y - 4) } else { 0 }).collect())
            .collect();
        // A symbol of bit(3, ..), and a text instance refined from it, 70 narrower with RDX 0.
        const TEXT: &str = "0000000030000100000013000000460000000c000000000000000042000000000001000001000000ed000003fffdff02fefefe00000001000000013f4efc0ce48e6b5c00246a648400fb605a44d542ed29da2b9b55c01f4e5af9842dd70c88f4913c7f552503ba52a5d7b5d9fd08ff5794b4dbfb3df7bafcf9c43d04154c1cefa181ca1aba5da826a4e4229858bd0950442df69c098ab3b61a32f71b136974b96bcb5306f3e75af1a4ef51b8f56a355acb8b69915cfa623f5a21933be8c056a348783a116ea914e886af2c64bf3b23e5610726c981092220f05411857529861fbf4c630a15fd00b081d2351279e06602f4408b25dc1d01c223027ecb8c87c4b1d76e3882a759c1adaf749f0e20735f8bd5a7306fffac000000020620010100000085000000460000000c0000000000000000040012ffffffff00000001a967c0b129999147062a4766ce10bc3dae7f1daf43dda1427d31203a1bc9fa021e0504e0a4b01739952970b54ad11ac08c0684d2f556a1843eb01dc75b963be6237b7f02fdfe17ccf7f061de310a936fd08c5ddea47609789e55f7cbca81ae38219c4abc5f9eff47ffac";
        let text: Vec<Vec<u8>> = (0..12).map(|y| (0..70).map(|x| bit(3, x + 35, y) ^ flipped(30, x, y)).collect()).collect();
        for (stream, picture) in [(PAGE, page), (TEXT, text)] {
            let stream: Vec<u8> = (0..stream.len() / 2).map(|i| u8::from_str_radix(&stream[i * 2..i * 2 + 2], 16).unwrap()).collect();
            let mut rows = Rows::default();
            Image::new_embedded(&stream, None).unwrap().decode(&mut rows).unwrap();
            assert_eq!(rows.0, picture);
        }
    }

    /// PdfCraft patch: the area a refinement of the page refines is the page inside it and 0
    /// outside: off the page's rows and columns, past where a column can be, and past the page's
    /// width and the area's own whatever the padding of the page's rows holds. `new_with` clears
    /// that padding (patch 13), so the page's words are set to all 1s, padding included.
    #[test]
    fn a_page_area_is_the_page_inside_it_and_0_outside() {
        let mut page = Bitmap::new_with(40, 3, 0, 0, true).unwrap();
        page.data.fill(!0);
        let area = |x, y, width, height| page.area(x, y, width, height).unwrap().data;
        assert_eq!(area(30, 1, 20, 3), [0xFFC0_0000, 0xFFC0_0000, 0], "10 columns and 2 rows of the page");
        assert_eq!(area(0, 0, 33, 1), [!0, 0x8000_0000], "the whole of a word, and none of the area's padding");
        assert_eq!(area(36, 0, 8, 1), [0xF000_0000], "4 columns, and none of the page's padding");
        assert_eq!(area(5, 0, 8, 1), [0xFF00_0000], "inside the page");
        assert_eq!(area(40, 0, 8, 1), [0], "right of the page");
        assert_eq!(area(u32::MAX - 2, 0, 40, 1), [0, 0], "past where a column can be");
        assert_eq!(area(0, u32::MAX, 8, 2), [0, 0], "below the page");
        assert!(area(3, 0, 0, 2).is_empty(), "no width");
    }

    /// A stream and its globals are one stream of segments, so they have one count.
    #[test]
    fn the_segments_of_a_stream_and_its_globals_count_together() {
        let segment: &[u8] = &[0, 0, 0, 0, 62, 0, 1, 0, 0, 0, 0]; // an empty extension
        let half = segment.repeat(1 << 19);
        let more = segment.repeat((1 << 19) + 1);
        let refused = |data: &[u8]| {
            let error = Image::new_embedded(data, Some(&half)).err().map(|e| e.to_string());
            error.as_deref() == Some("too many segments or references in a stream")
        };
        assert!(!refused(&half), "2^20 segments are allowed");
        assert!(refused(&more), "one more isn't");
    }

    /// PdfCraft patch: a refinement whose reference is offset right of it reads the reference from
    /// its first column, where it read the first 32 as 0. So that a context read wrong changes what
    /// decodes, the refinement is large, and its picture comes from the hash it was coded from: a
    /// 104 × 12 text instance refined from a 100 × 12 symbol 2 right of it.
    #[test]
    fn a_refinement_reads_a_reference_right_of_it_from_its_first_column() {
        #[derive(Default)]
        struct Rows(Vec<Vec<u8>>, Vec<u8>);
        impl Decoder for Rows {
            fn push_pixel(&mut self, black: bool) {
                self.1.push(u8::from(black));
            }
            fn push_pixel_chunk(&mut self, black: bool, chunks: u32) {
                for _ in 0..8 * chunks {
                    self.push_pixel(black);
                }
            }
            fn next_line(&mut self) {
                self.0.push(core::mem::take(&mut self.1));
            }
        }
        fn bit(seed: u32, x: u32, y: u32) -> u8 {
            let mut h = x.wrapping_mul(0x9E37_79B1).wrapping_add(y.wrapping_mul(0x85EB_CA77)).wrapping_add(seed.wrapping_mul(0xC2B2_AE3D));
            h ^= h >> 15;
            h = h.wrapping_mul(0x2C1B_3C6D);
            h ^= h >> 12;
            h = h.wrapping_mul(0x297A_2D39);
            h ^= h >> 15;
            ((h >> 7) & 1) as u8
        }
        // About one pixel in eight: where a refinement differs from its reference.
        let flipped = |seed: u32, x: u32, y: u32| bit(seed, x, y) & bit(seed + 1, x, y) & bit(seed + 2, x, y);
        // A symbol of bit(2, ..), and a text instance refined from it, 4 wider with RDX 0.
        const TEXT: &str = "0000000030000100000013000000680000000c000000000000000042000000000001000001000000b1000003fffdff02fefefe00000001000000013f525ad7118f669a303f1b11efa5210e9765b834b1831c650f3a3d325dbd7317379ea4fb573b09a20c362f065d8f9301e74cbbc50c69910641c0e0eea7c2fd1e831ca1459a943be55ed8e074b426ad7ff1e8d8ff6f1aa44684de5da4b4c3483911604412e00e96a0db6ec7103feb2bc4301ae98d3f9c23078ae72dc46f76e9dfea209d39187f1194f7b9285cd6e7b2dc7abb5def43ff0abe2df554fc55ffac0000000206200101000000af000000680000000c0000000000000000040012ffffffff00000001a96b5febebec5a68693b51f5db25ddf176d4aae89904a71d1690f12888e10b038db723ac59941f76c2844b739ce524d9563f4005e1edbabf2e323e8e9ac1a3b9060587c3010e7d6bcf90c00dbf9000a2f17b1d9b598e01b9158dc16a63f4388a6f647b99dc7118aa09e08d59fba1bc24e3a0f1ebb0cae636b6bedbb65b01945228d3e5d2980a0ad6290c13d2f34f5c861415ffac";
        let picture: Vec<Vec<u8>> = (0..12)
            .map(|y| (0..104).map(|x| (if (2..102).contains(&x) { bit(2, x - 2, y) } else { 0 }) ^ flipped(20, x, y)).collect())
            .collect();
        let stream: Vec<u8> = (0..TEXT.len() / 2).map(|i| u8::from_str_radix(&TEXT[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let mut rows = Rows::default();
        Image::new_embedded(&stream, None).unwrap().decode(&mut rows).unwrap();
        assert_eq!(rows.0, picture);
    }

    /// A dictionary being decoded has what the page may still hold, not all of it: refused only
    /// once it is stored, its copies would already have been made.
    #[test]
    fn the_room_left_shrinks_as_segments_are_stored() {
        let mut page = PageState::default();
        assert_eq!(page.retained_room(), MAX_RETAINED_WORDS);
        page.retain(1000).unwrap();
        assert_eq!(page.retained_room(), MAX_RETAINED_WORDS - 1000 - ENTRY_WORDS);
        page.retained_words = MAX_RETAINED_WORDS;
        assert_eq!(page.retained_room(), 0);
    }

    /// Pixels past a bitmap's edge are 0 (6.3.5.2), and a refinement reads its reference a word at
    /// a time, so a bitmap filled with 1s has none past its width: made, and a page made again for
    /// the next image.
    #[test]
    fn a_bitmap_of_1s_has_nothing_past_its_width() {
        assert_eq!(Bitmap::new_with(33, 2, 0, 0, true).unwrap().data, [!0, 1 << 31, !0, 1 << 31]);
        let mut page = Bitmap::new_with(40, 1, 0, 0, false).unwrap();
        page.reinitialize(7, 2, true).unwrap();
        assert_eq!(page.data, [0xFE00_0000, 0xFE00_0000]);
        assert_eq!(Bitmap::new_with(64, 1, 0, 0, true).unwrap().data, [!0, !0], "words filled whole");
        assert!(Bitmap::new_with(0, 3, 0, 0, true).unwrap().data.is_empty(), "no width");
    }
}
