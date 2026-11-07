//! Bitmap representation for JBIG2 decoding.
//!
//! "The variable whose value is the result of this decoding procedure is shown
//! in Table 3." (6.2.3)
//!
//! "GBREG - The decoded region bitmap." (Table 3)

use crate::decode::CombinationOperator;
use crate::error::{OverflowError, Result, bail};
use alloc::vec;
use alloc::vec::Vec;

// To guard better against malicious files, we limit bitmap dimensions
// to `u16::MAX`, which should be more than enough.
pub(crate) const MAX_DIMENSION: u32 = u16::MAX as u32;

/// PdfCraft patch: the most pixels one bitmap may have (2^28, the renderer's
/// `MAX_IMAGE_PIXELS`). Each side alone may be 65535, so a single region could be 4.3 billion
/// pixels, decoded one by one.
pub(crate) const MAX_PIXELS: u64 = 1 << 28;

/// PdfCraft patch: the most cells a halftone grid may have, which still gives the largest page
/// (see [`MAX_PIXELS`]) cells of 3 × 3, unrotated. Each cell takes a 4-byte value while the grid decodes, so
/// the pixel bound alone let a 98-byte segment take 1 GiB; this one leaves 128 MiB.
pub(crate) const MAX_GRID_CELLS: u64 = 1 << 25;

/// PdfCraft patch: the decoding work a stream may ask for, in pixels decoded from their code: four
/// of the largest pages, about 6 s on data the decoder predicts and about 17 s on data it can't, so
/// a render finishes inside its 20 s watchdog. Each bitmap is bounded on its own, but nothing
/// bounded how many there are: 1,140 bytes of generic regions took over 40 s, a 795-byte PDF's one
/// halftone 265 s, and a 923-byte PDF's regions would have taken days.
pub(crate) const MAX_WORK: u64 = 4 * MAX_PIXELS;

/// PdfCraft patch: the work of reading one integer of an item, what an arithmetic-coded one takes
/// at worst in decisions (see [`MAX_WORK`]), so that a decision costs no more than a pixel. An item
/// with no pixels still reads its integers: charged a step each, a 22,515-byte PDF of refined text
/// instances of no size stayed inside `MAX_WORK` and took 730 s.
pub(crate) const INTEGER_WORK: u64 = 38;

/// PdfCraft patch: the work of a line of a Huffman table built for symbols, about 50 ns (see
/// [`MAX_WORK`]).
pub(crate) const HUFFMAN_LINE_WORK: u64 = 10;

/// PdfCraft patch: the work of decoding a bitmap pixel by pixel (see [`MAX_WORK`]). A row costs up
/// to 21 ns however few pixels it has, about 4 pixels' worth, so one of no width isn't free: 16,384
/// regions of 0 × 65535 took 9 s at a cost of nothing.
pub(crate) fn decode_work(width: u32, height: u32) -> u64 {
    u64::from(height) * (u64::from(width) + 4)
}

/// PdfCraft patch: the work of copying a bitmap (see [`MAX_WORK`]): a word at a time, and a row is
/// a copy of its own, so a tall thin bitmap costs its rows, not its pixels.
pub(crate) fn copy_work(width: u32, height: u32) -> u64 {
    u64::from(height) * (u64::from(width.div_ceil(WORD_BITS)) + 1)
}

/// The underlying word type for packed pixel storage.
/// Change this to `u32` to switch back to 32-bit words.
pub(crate) type Word = u32;

pub(crate) const WORD_BITS: u32 = Word::BITS;
pub(crate) const WORD_SHIFT: u32 = WORD_BITS - 1;

/// PdfCraft patch: the words a [`Bitmap`] takes before its pixels. A symbol of width 0 has no
/// pixels and still costs this: 5 KB of zero-width dictionaries held 781 MB.
pub(crate) const BITMAP_WORDS: usize = size_of::<Bitmap>() / size_of::<Word>();

/// PdfCraft patch: refuse a page past the bounds on a bitmap (see `MAX_PIXELS`). A stream is held
/// to this when it is opened as well as when its page bitmap is sized: a caller sizes its own
/// output from the page header in between, and a 743-byte PDF's Indexed image asked for 137 GB.
pub(crate) fn check_page_size(width: u32, height: u32) -> Result<()> {
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(OverflowError::BitmapDimension.into());
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(OverflowError::BitmapDimension.into());
    }
    Ok(())
}

/// A decoded bitmap with position information.
///
/// "Pixels decoded by the MMR decoder having the value 'black' shall be treated
/// as having the value 1. Pixels decoded by the MMR decoder having the value
/// 'white' shall be treated as having the value 0." (6.2.6)
#[derive(Debug, Clone, Default)]
pub(crate) struct Bitmap {
    /// Width in pixels.
    pub(crate) width: u32,
    /// Height in pixels.
    pub(crate) height: u32,
    /// Number of words per row.
    pub(crate) stride: u32,
    /// Packed pixel data, one bit per pixel, row-major order.
    /// Each row is padded to a word boundary.
    ///
    /// PdfCraft patch: the padding is 0s, as pixels past the edge are (6.2.5.2, 6.3.5.2). Generic
    /// and refinement decoding read the rows above their own pixels a word at a time, padding
    /// included, and a refinement reads its reference the same way: whatever writes whole words
    /// keeps the padding 0 (see `clear_padding`).
    pub(crate) data: Vec<Word>,
    /// "This four-byte field gives the horizontal offset in pixels of the bitmap
    /// encoded in this segment relative to the page bitmap." (7.4.1.3)
    pub(crate) x_location: u32,
    /// "This four-byte field gives the vertical offset in pixels of the bitmap
    /// encoded in this segment relative to the page bitmap." (7.4.1.4)
    pub(crate) y_location: u32,
}

impl Bitmap {
    /// PdfCraft patch: the words this bitmap holds, its header included.
    pub(crate) fn words(&self) -> usize {
        self.data.len().saturating_add(BITMAP_WORDS)
    }

    /// Create a new bitmap filled with white pixels (false).
    ///
    /// The bitmap is positioned at (0, 0).
    pub(crate) fn new(width: u32, height: u32) -> Result<Self> {
        Self::new_with(width, height, 0, 0, false)
    }

    /// Create a new bitmap with full configuration.
    pub(crate) fn new_with(
        width: u32,
        height: u32,
        x_location: u32,
        y_location: u32,
        default_pixel: bool,
    ) -> Result<Self> {
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            bail!(OverflowError::BitmapDimension);
        }
        // PdfCraft patch: see `MAX_PIXELS`.
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            bail!(OverflowError::BitmapDimension);
        }

        let stride = width.div_ceil(WORD_BITS);
        let default_word: Word = if default_pixel { !0 } else { 0 };
        let data = vec![default_word; stride as usize * height as usize];
        let mut bitmap = Self {
            width,
            height,
            stride,
            data,
            x_location,
            y_location,
        };
        if default_pixel {
            bitmap.clear_padding();
        }
        Ok(bitmap)
    }

    /// PdfCraft patch: clear the bits past `width` in each row's last word. "All pixels lying
    /// outside the bounds of the actual bitmap or the reference bitmap have the value 0" (6.3.5.2),
    /// and a refinement reads its reference a word at a time, those bits included: filled whole
    /// with a default pixel of 1, they read as 1s, so a refinement of the page, or of an
    /// intermediate text or halftone region, decoded to other pixels than were coded.
    fn clear_padding(&mut self) {
        let trailing = self.width % WORD_BITS;
        // A width of 0 has no words to clear, and `chunks_exact_mut` would take a stride of 0.
        if trailing == 0 {
            return;
        }
        let mask: Word = !0 << (WORD_BITS - trailing);
        for row in self.data.chunks_exact_mut(self.stride as usize) {
            if let Some(last) = row.last_mut() {
                *last &= mask;
            }
        }
    }

    pub(crate) fn reinitialize(
        &mut self,
        width: u32,
        height: u32,
        default_pixel: bool,
    ) -> Result<()> {
        check_page_size(width, height)?;

        let stride = width.div_ceil(WORD_BITS);
        let default_word: Word = if default_pixel { !0 } else { 0 };
        self.width = width;
        self.height = height;
        self.stride = stride;
        self.x_location = 0;
        self.y_location = 0;
        self.data.clear();
        self.data
            .resize(stride as usize * height as usize, default_word);
        if default_pixel {
            self.clear_padding();
        }

        Ok(())
    }

    /// Get a pixel value at (x, y).
    #[inline]
    pub(crate) fn get_pixel(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }

        let word_idx = (y * self.stride + x / WORD_BITS) as usize;
        let bit_pos = WORD_SHIFT - (x % WORD_BITS);
        ((self.data[word_idx] >> bit_pos) & 1) as u8
    }

    /// Set a pixel value at (x, y).
    #[inline]
    pub(crate) fn set_pixel(&mut self, x: u32, y: u32, value: u8) {
        if x >= self.width || y >= self.height {
            return;
        }

        let word_idx = (y * self.stride + x / WORD_BITS) as usize;
        let bit_pos = WORD_SHIFT - (x % WORD_BITS);
        self.data[word_idx] |= (value as Word) << bit_pos;
    }

    /// PdfCraft patch: combine a decoded region into this page at the region's own location.
    ///
    /// "Unless otherwise specified, all multi-bit values shall be treated as unsigned values"
    /// (5.4.4), and a region's location (7.4.1.3, 7.4.1.4) is not specified otherwise. One past
    /// `i32::MAX` is past any page. Cast to a signed coordinate it read as left of the page or above
    /// it, and one within a region's size of 2^32 came back onto it.
    pub(crate) fn combine_region(&mut self, region: &Self, operator: CombinationOperator) {
        let signed = |location: u32| i32::try_from(location).unwrap_or(i32::MAX);
        self.combine(region, signed(region.x_location), signed(region.y_location), operator);
    }

    /// Combine another bitmap into this one at a specific location.
    ///
    /// "These operators describe how the segment's bitmap is to be combined with
    /// the page bitmap." (7.4.1.5)
    ///
    /// Pixels outside the destination bitmap are ignored.
    pub(crate) fn combine(&mut self, other: &Self, x: i32, y: i32, operator: CombinationOperator) {
        // TODO: Figure out how we can optimize this, especially since this is a
        // bottleneck for symbols/patterns.
        let dest_x_start = x.max(0);
        // Saturating: a location can be anywhere an `i32` reaches, and past the end it is off the page.
        let dest_x_end = x.saturating_add(other.width as i32).min(self.width as i32);

        if dest_x_start >= dest_x_end {
            return;
        }

        let src_x_start = (dest_x_start - x) as u32;
        let dest_x_start = dest_x_start as u32;
        let dest_x_end = dest_x_end as u32;

        for src_y in 0..other.height {
            let dest_y = y.saturating_add(src_y as i32);
            if dest_y < 0 || dest_y >= self.height as i32 {
                continue;
            }

            let dest_y = dest_y as u32;

            let first_word = dest_x_start / WORD_BITS;
            let last_word = (dest_x_end - 1) / WORD_BITS;

            for word_idx in first_word..=last_word {
                let word_start_x = word_idx * WORD_BITS;
                let word_end_x = word_start_x + WORD_BITS;

                let px_start = dest_x_start.max(word_start_x);
                let px_end = dest_x_end.min(word_end_x);

                let bit_start = px_start - word_start_x;
                let bit_end = px_end - word_start_x;

                let mask: Word = if bit_end == WORD_BITS {
                    !0 >> bit_start
                } else {
                    (!0 >> bit_start) & !(!0 >> bit_end)
                };

                let src_x_for_range = src_x_start + (px_start - dest_x_start);
                let src_word_idx = src_x_for_range / WORD_BITS;
                let src_bit_offset = src_x_for_range % WORD_BITS;

                let src_word1 = other.get_word(src_y, src_word_idx);
                let src_word2 = other.get_word(src_y, src_word_idx + 1);

                let src_raw = if src_bit_offset == 0 {
                    src_word1
                } else {
                    (src_word1 << src_bit_offset) | (src_word2 >> (WORD_BITS - src_bit_offset))
                };
                let src_aligned = src_raw >> bit_start;

                let dest_idx = (dest_y * self.stride + word_idx) as usize;
                let dest_word = self.data[dest_idx];

                let result = match operator {
                    CombinationOperator::Or => dest_word | src_aligned,
                    CombinationOperator::And => dest_word & src_aligned,
                    CombinationOperator::Xor => dest_word ^ src_aligned,
                    CombinationOperator::Xnor => !(dest_word ^ src_aligned),
                    CombinationOperator::Replace => src_aligned,
                };

                self.data[dest_idx] = (dest_word & !mask) | (result & mask);
            }
        }
    }

    #[inline]
    pub(crate) fn get_word(&self, row: u32, word_idx: u32) -> Word {
        if row >= self.height || word_idx >= self.stride {
            return 0;
        }

        self.data[(row * self.stride + word_idx) as usize]
    }

    /// PdfCraft patch: the `width` × `height` area of this bitmap at `x`, `y`, as a bitmap there.
    /// It is what a refinement of the page refines: "a bitmap containing the current contents of
    /// the page buffer, restricted to the area of the page buffer specified by this segment's
    /// region segment information field" (7.4.7.4). Off this bitmap the area is 0, past its width
    /// too whatever the padding of its rows holds, and outside the area the refinement reads 0
    /// (6.3.5.2), where reading the page itself took in the pixels around the area.
    pub(crate) fn area(&self, x: u32, y: u32, width: u32, height: u32) -> Result<Self> {
        let mut area = Self::new_with(width, height, x, y, false)?;
        for row in 0..height {
            let Some(from_row) = y.checked_add(row).filter(|r| *r < self.height) else {
                continue;
            };
            for word in 0..area.stride {
                // The first column of this word of the area, in this bitmap; past its width,
                // or past where a column can be, the rest of the row is 0.
                let Some(from) = x.checked_add(word * WORD_BITS).filter(|c| *c < self.width) else {
                    break;
                };
                let (index, shift) = (from / WORD_BITS, from % WORD_BITS);
                let mut bits = self.get_word(from_row, index) << shift;
                if shift > 0 {
                    bits |= self.get_word(from_row, index + 1) >> (WORD_BITS - shift);
                }
                // Both are above 0: `from` is inside this bitmap, and `word` inside the area.
                let ours = self.width - from;
                let theirs = width - word * WORD_BITS;
                let kept = ours.min(theirs);
                if kept < WORD_BITS {
                    bits &= !0 << (WORD_BITS - kept);
                }
                if let Some(slot) = area.data.get_mut((row * area.stride + word) as usize) {
                    *slot = bits;
                }
            }
        }
        Ok(area)
    }
}
