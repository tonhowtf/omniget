//! Generic region segment parsing and decoding (7.4.6, 6.2).

use super::{
    AdaptiveTemplatePixel, RegionBitmap, RegionSegmentInfo, Template, parse_region_segment_info,
};
use crate::ScratchBuffers;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::{Bitmap, WORD_BITS, WORD_SHIFT, Word};
use crate::error::{ParseError, RegionError, Result, TemplateError, bail};
use crate::reader::Reader;

/// Generic region decoding procedure (6.2).
pub(crate) fn decode(
    header: &GenericRegionHeader<'_>,
    ctx: &mut ScratchBuffers,
) -> Result<RegionBitmap> {
    let mut bitmap = Bitmap::new_with(
        header.region_info.width,
        header.region_info.height,
        header.region_info.x_location,
        header.region_info.y_location,
        false,
    )?;

    decode_into(header, &mut bitmap, ctx)?;

    Ok(RegionBitmap {
        bitmap,
        combination_operator: header.region_info.combination_operator,
    })
}

pub(crate) fn decode_into(
    header: &GenericRegionHeader<'_>,
    bitmap: &mut Bitmap,
    ctx: &mut ScratchBuffers,
) -> Result<()> {
    let data = header.data;

    if header.mmr {
        // "6.2.6 Decoding using MMR coding"
        let _ = decode_bitmap_mmr(bitmap, data)?;
    } else {
        let mut decoder = ArithmeticDecoder::new(data);
        ctx.contexts.clear();
        ctx.contexts.resize(
            1 << header.template.context_bits(),
            ArithmeticDecoderContext::default(),
        );

        // "6.2.5 Decoding using a template and arithmetic coding"
        decode_bitmap_arithmetic_coding(
            bitmap,
            &mut decoder,
            &mut ctx.contexts,
            header.template,
            header.tpgdon,
            &header.adaptive_template_pixels,
        )?;
    }

    Ok(())
}

/// Parsed generic region segment header (7.4.6.1).
#[derive(Debug, Clone)]
pub(crate) struct GenericRegionHeader<'a> {
    pub(crate) region_info: RegionSegmentInfo,
    pub(crate) mmr: bool,
    pub(crate) template: Template,
    pub(crate) tpgdon: bool,
    pub(crate) adaptive_template_pixels: [AdaptiveTemplatePixel; 4],
    pub(crate) data: &'a [u8],
}

/// Parse a generic region segment header (7.4.6.1).
pub(crate) fn parse<'a>(
    reader: &mut Reader<'a>,
    had_unknown_length: bool,
) -> Result<GenericRegionHeader<'a>> {
    let mut region_info = parse_region_segment_info(reader)?;
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let mmr = flags & 0x01 != 0;
    let template = Template::from_byte(flags >> 1);
    let tpgdon = flags & 0x08 != 0;
    let ext_template = flags & 0x10 != 0;
    let adaptive_template_pixels = if mmr {
        [AdaptiveTemplatePixel::default(); 4]
    } else {
        parse_adaptive_template_pixels(reader, template, ext_template)?
    };
    let mut data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    // "As a special case, as noted in 7.2.7, an immediate generic region segment
    // may have an unknown length. In this case, it also indicates the height of
    // the generic region (i.e. the number of rows that have been decoded in this
    // segment; it must be no greater than the region segment bitmap height value
    // in the segment's region segment information field." (7.4.6.4)
    if had_unknown_length {
        // Length has already been validated during segment parsing.
        let (head, tail) = data.split_at(data.len() - 4);
        let row_count = u32::from_be_bytes(tail.try_into().unwrap());

        if row_count > region_info.height {
            bail!(RegionError::InvalidDimension);
        }

        region_info.height = row_count;
        data = head;
    }

    Ok(GenericRegionHeader {
        region_info,
        mmr,
        template,
        tpgdon,
        adaptive_template_pixels,
        data,
    })
}

/// Parse adaptive template pixel positions (7.4.6.3).
pub(crate) fn parse_adaptive_template_pixels(
    reader: &mut Reader<'_>,
    template: Template,
    // TODO: Find a test with this flag.
    _ext_template: bool,
) -> Result<[AdaptiveTemplatePixel; 4]> {
    let num_pixels = template.adaptive_template_pixels() as usize;

    let mut pixels = [AdaptiveTemplatePixel::default(); 4];

    for pixel in pixels.iter_mut().take(num_pixels) {
        let x = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;
        let y = reader.read_byte().ok_or(ParseError::UnexpectedEof)? as i8;

        // Validate AT pixel location (6.2.5.4, Figure 7).
        // AT pixels must reference already-decoded pixels:
        // - y must be <= 0 (current row or above)
        // - if y == 0, x must be < 0 (strictly to the left of current pixel)
        if y > 0 || (y == 0 && x >= 0) {
            bail!(TemplateError::InvalidAtPixel);
        }

        *pixel = AdaptiveTemplatePixel { x, y };
    }

    Ok(pixels)
}

/// Whether the adaptive template pixels correspond to the default ones.
/// See Table 5.
fn has_default_at_pixels(template: Template, at_pixels: &[AdaptiveTemplatePixel; 4]) -> bool {
    match template {
        Template::Template0 => {
            at_pixels[0].x == 3
                && at_pixels[0].y == -1
                && at_pixels[1].x == -3
                && at_pixels[1].y == -1
                && at_pixels[2].x == 2
                && at_pixels[2].y == -2
                && at_pixels[3].x == -2
                && at_pixels[3].y == -2
        }
        Template::Template1 => at_pixels[0].x == 3 && at_pixels[0].y == -1,
        Template::Template2 => at_pixels[0].x == 2 && at_pixels[0].y == -1,
        Template::Template3 => at_pixels[0].x == 2 && at_pixels[0].y == -1,
    }
}

/// Decode a bitmap using MMR coding (6.2.6).
pub(crate) fn decode_bitmap_mmr(bitmap: &mut Bitmap, data: &[u8]) -> Result<usize> {
    /// A decoder sink that writes decoded pixels into a `Bitmap`.
    struct BitmapDecoder<'a> {
        bitmap: &'a mut Bitmap,
        x: u32,
        y: u32,
        /// Precomputed start index into `bitmap.data` for the current row.
        row_start: usize,
        /// Accumulator for pixels emitted by `push_pixel`.
        buf: u8,
        /// Number of bits accumulated in `buf`.
        buf_len: u8,
    }

    impl<'a> BitmapDecoder<'a> {
        fn new(bitmap: &'a mut Bitmap) -> Self {
            Self {
                bitmap,
                x: 0,
                y: 0,
                row_start: 0,
                buf: 0,
                buf_len: 0,
            }
        }

        #[inline]
        fn flush_buf(&mut self) {
            if self.buf_len == 0 {
                return;
            }

            let start_x = self.x - self.buf_len as u32;
            if start_x < self.bitmap.width {
                let word_idx = (start_x / WORD_BITS) as usize;
                let bit_in_word = start_x % WORD_BITS;
                let shift = WORD_SHIFT - bit_in_word - (self.buf_len as u32 - 1);
                self.bitmap.data[self.row_start + word_idx] |= (self.buf as Word) << shift;
            }
            self.buf = 0;
            self.buf_len = 0;
        }
    }

    impl hayro_ccitt::Decoder for BitmapDecoder<'_> {
        #[inline]
        fn push_pixel(&mut self, white: bool) {
            self.buf = (self.buf << 1) | white as u8;
            self.buf_len += 1;
            self.x += 1;

            if self.buf_len == 8 {
                self.flush_buf();
            }
        }

        #[inline]
        fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
            const WORD_BYTES: usize = (WORD_BITS / 8) as usize;
            const BYTE_MASKS: [Word; WORD_BYTES] = {
                #[allow(trivial_numeric_casts)]
                let mut masks = [0 as Word; WORD_BYTES];
                let mut i = 0;
                while i < WORD_BYTES {
                    #[allow(trivial_numeric_casts)]
                    {
                        masks[i] = (0xFF as Word) << ((WORD_BYTES - 1 - i) * 8);
                    }
                    i += 1;
                }
                masks
            };

            let row_start = self.row_start;
            let end_x = (self.x + chunk_count * 8).min(self.bitmap.width);
            let white_mask = (white as Word).wrapping_neg();

            let start = (self.x / 8) as usize;
            let end = (end_x / 8) as usize;
            let first_full = start.div_ceil(WORD_BYTES);
            let last_full = end / WORD_BYTES;

            for b in start..(first_full * WORD_BYTES).min(end) {
                self.bitmap.data[row_start + b / WORD_BYTES] |=
                    BYTE_MASKS[b % WORD_BYTES] & white_mask;
            }

            if last_full > first_full {
                self.bitmap.data[row_start + first_full..row_start + last_full].fill(white_mask);
            }

            for b in (first_full.max(last_full) * WORD_BYTES)..end {
                self.bitmap.data[row_start + b / WORD_BYTES] |=
                    BYTE_MASKS[b % WORD_BYTES] & white_mask;
            }

            self.x = end_x;
        }

        #[inline]
        fn next_line(&mut self) {
            self.flush_buf();

            self.x = 0;
            self.y += 1;
            self.row_start = (self.y * self.bitmap.stride) as usize;
        }
    }

    let width = bitmap.width;
    let height = bitmap.height;
    let mut decoder = BitmapDecoder::new(bitmap);

    let settings = hayro_ccitt::DecodeSettings {
        columns: width,
        rows: height,
        // "If the number of bytes contained in the encoded bitmap is known in
        // advance, then it is permissible for the data stream not to contain
        // an EOFB" (6.2.6). But it _can_ contain it, which is what this
        // flag indicates.
        end_of_block: true,
        end_of_line: false,
        rows_are_byte_aligned: false,
        encoding: hayro_ccitt::EncodingMode::Group4,
        // "Pixels decoded by the MMR decoder having the value 'black' shall be
        // treated as having the value 1. Pixels decoded by the MMR decoder
        // having the value 'white' shall be treated as having the value 0."
        // (6.2.6)
        //
        // hayro-ccitt uses 1 for white, 0 for black by default, so we need to
        // invert to match JBIG2 convention.
        invert_black: true,
    };

    // "An invocation of the generic region decoding procedure with MMR equal to
    // 1 shall consume an integral number of bytes, beginning and ending on a
    // byte boundary. This may involve skipping over some bits in the last byte
    // read." (6.2.6)
    //
    // hayro-ccitt already aligns to the byte boundary before returning, so
    // nothing else to do here.
    let mut context = hayro_ccitt::DecoderContext::new(settings);
    Ok(hayro_ccitt::decode(data, &mut decoder, &mut context)
        .map_err(|_| RegionError::InvalidMmrData)?)
}

// I'm not sure why, but I was getting very weird codegen (with bad performance)
// when attempting to do this via generics. Hence why we use a macro for that.
macro_rules! decode_loop {
    ($bitmap:expr, $decoder:expr, $contexts:expr, $ctx_gatherer:expr,
     $tpgdon:expr, $sltp_context:expr, $gather:expr) => {{
        let bitmap: &mut Bitmap = $bitmap;
        let decoder: &mut ArithmeticDecoder<'_> = $decoder;
        let contexts: &mut [ArithmeticDecoderContext] = $contexts;
        let ctx_gatherer: &mut ContextGatherer<'_> = $ctx_gatherer;
        let width = bitmap.width;
        let height = bitmap.height;

        // "1) Set: LTP = 0" (6.2.5.7)
        let mut ltp = false;

        // "3) Decode each row as follows:" (6.2.5.7)
        for y in 0..height {
            // "b) If TPGDON is 1, then decode a bit using the arithmetic entropy
            // coder" (6.2.5.7)
            if $tpgdon {
                let sltp = decoder.read_bit(&mut contexts[$sltp_context as usize]);
                // "Let SLTP be the value of this bit. Set: LTP = LTP XOR SLTP" (6.2.5.7)
                ltp = ltp != (sltp != 0);
            }

            // "c) If LTP = 1 then set every pixel of the current row of GBREG equal
            // to the corresponding pixel of the row immediately above." (6.2.5.7)
            if ltp {
                // If y == 0, pixels remain the same.
                if y > 0 {
                    let stride = bitmap.stride as usize;
                    let src = (y as usize - 1) * stride;
                    bitmap
                        .data
                        .copy_within(src..src + stride, y as usize * stride);
                }
            } else {
                // "d) If LTP = 0 then, from left to right, decode each pixel of the
                // current row of GBREG." (6.2.5.7)
                ctx_gatherer.start_row(bitmap, y);

                for x in 0..width {
                    ctx_gatherer.maybe_reload_buffers(bitmap, x);
                    let context_bits = ($gather)(ctx_gatherer, bitmap, x) as usize;
                    let pixel = decoder.read_bit(&mut contexts[context_bits]);
                    let value = pixel as u8;
                    bitmap.set_pixel(x, y, value);
                    ctx_gatherer.update_current_row(x, value);
                }
            }
        }
    }};
}

/// Decode a bitmap using arithmetic coding (6.2.5).
pub(crate) fn decode_bitmap_arithmetic_coding(
    bitmap: &mut Bitmap,
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [ArithmeticDecoderContext],
    template: Template,
    tpgdon: bool,
    adaptive_template_pixels: &[AdaptiveTemplatePixel; 4],
) -> Result<()> {
    let mut ctx_gatherer = ContextGatherer::new(template, adaptive_template_pixels);

    // See Figure 8 - 11.
    let sltp_context: u16 = match template {
        Template::Template0 => 0b1001101100100101,
        Template::Template1 => 0b0011110010101,
        Template::Template2 => 0b0011100101,
        Template::Template3 => 0b0110010101,
    };

    if ctx_gatherer.use_default_at {
        match template {
            Template::Template0 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template0_default
            ),
            Template::Template1 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template1_default
            ),
            Template::Template2 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template2_default
            ),
            Template::Template3 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template3_default
            ),
        }
    } else {
        match template {
            Template::Template0 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template0_custom
            ),
            Template::Template1 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template1_custom
            ),
            Template::Template2 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template2_custom
            ),
            Template::Template3 => decode_loop!(
                bitmap,
                decoder,
                contexts,
                &mut ctx_gatherer,
                tpgdon,
                sltp_context,
                ContextGatherer::gather_template3_custom
            ),
        }
    }

    Ok(())
}

pub(crate) struct ContextGatherer<'a> {
    template: Template,
    at_pixels: &'a [AdaptiveTemplatePixel; 4],
    use_default_at: bool,
    /// Used in `maybe_reload_buffers` to determine how far to the right
    /// we might access pixels.
    max_right: u32,
    /// Current position.
    cur_y: u32,
    cur_x: u32,
    /// Pre-fetched pixel buffers for rows y-2, y-1, y.
    buf_m2: Word,
    buf_m1: Word,
    buf_cur: Word,
    /// The current context for all 3 rows.
    ctx: u16,
}

impl<'a> ContextGatherer<'a> {
    pub(crate) fn new(template: Template, at_pixels: &'a [AdaptiveTemplatePixel; 4]) -> Self {
        let use_default_at = has_default_at_pixels(template, at_pixels);
        let max_right = match template {
            Template::Template0 | Template::Template1 => {
                if use_default_at {
                    3
                } else {
                    2
                }
            }
            Template::Template2 | Template::Template3 => {
                if use_default_at {
                    2
                } else {
                    1
                }
            }
        };
        Self {
            template,
            at_pixels,
            use_default_at,
            max_right,
            cur_y: 0,
            buf_m2: 0,
            buf_m1: 0,
            buf_cur: 0,
            cur_x: 0,
            ctx: 0,
        }
    }

    const SHIFT_MASK_T0: u16 = 0b0110_0011_1100_1110;
    const SHIFT_MASK_T1: u16 = 0b0001_1101_1110_0110;
    const SHIFT_MASK_T2: u16 = 0b0000_0011_0111_0010;
    const SHIFT_MASK_T3: u16 = 0b0000_0011_1100_1110;

    const SHIFT_MASK_T0_DEFAULT: u16 = 0b1111_0111_1110_1110;
    const SHIFT_MASK_T1_DEFAULT: u16 = 0b0001_1101_1111_0110;
    const SHIFT_MASK_T2_DEFAULT: u16 = 0b0000_0011_0111_1010;
    const SHIFT_MASK_T3_DEFAULT: u16 = 0b0000_0011_1110_1110;

    pub(crate) fn start_row(&mut self, bitmap: &Bitmap, y: u32) {
        self.cur_y = y;
        self.cur_x = 0;

        self.buf_m2 = if y >= 2 {
            Self::load_word(bitmap, y - 2, 0)
        } else {
            0
        };
        self.buf_m1 = if y >= 1 {
            Self::load_word(bitmap, y - 1, 0)
        } else {
            0
        };
        self.buf_cur = 0;

        // Start initializing the contexts. Note that this won't load all initial
        // pixels yet, those will only be loaded after our first call to `gather`.
        // See 6.2.5.3 for the pixel positions.
        self.ctx = if self.use_default_at {
            self.init_context_default()
        } else {
            self.init_context_custom()
        };
    }

    #[inline]
    fn init_context_custom(&self) -> u16 {
        match self.template {
            Template::Template0 => {
                let m2 = Self::get_buf_pixel(self.buf_m2, 0);
                let m1 = (Self::get_buf_pixel(self.buf_m1, 0) << 1)
                    | Self::get_buf_pixel(self.buf_m1, 1);
                (m2 << 12) | (m1 << 5)
            }
            Template::Template1 => {
                let m2 = (Self::get_buf_pixel(self.buf_m2, 0) << 1)
                    | Self::get_buf_pixel(self.buf_m2, 1);
                let m1 = (Self::get_buf_pixel(self.buf_m1, 0) << 1)
                    | Self::get_buf_pixel(self.buf_m1, 1);
                (m2 << 9) | (m1 << 4)
            }
            Template::Template2 => {
                let m2 = Self::get_buf_pixel(self.buf_m2, 0);
                let m1 = Self::get_buf_pixel(self.buf_m1, 0);
                (m2 << 7) | (m1 << 3)
            }
            Template::Template3 => {
                let m1 = Self::get_buf_pixel(self.buf_m1, 0);
                m1 << 5
            }
        }
    }

    #[inline]
    fn init_context_default(&self) -> u16 {
        match self.template {
            Template::Template0 => {
                (Self::get_buf_pixel(self.buf_m2, 0) << 12)
                    | (Self::get_buf_pixel(self.buf_m2, 1) << 11)
                    | (Self::get_buf_pixel(self.buf_m1, 0) << 6)
                    | (Self::get_buf_pixel(self.buf_m1, 1) << 5)
                    | (Self::get_buf_pixel(self.buf_m1, 2) << 4)
            }
            Template::Template1 => {
                (Self::get_buf_pixel(self.buf_m2, 0) << 10)
                    | (Self::get_buf_pixel(self.buf_m2, 1) << 9)
                    | (Self::get_buf_pixel(self.buf_m1, 0) << 5)
                    | (Self::get_buf_pixel(self.buf_m1, 1) << 4)
                    | (Self::get_buf_pixel(self.buf_m1, 2) << 3)
            }
            Template::Template2 => {
                (Self::get_buf_pixel(self.buf_m2, 0) << 7)
                    | (Self::get_buf_pixel(self.buf_m1, 0) << 3)
                    | (Self::get_buf_pixel(self.buf_m1, 1) << 2)
            }
            Template::Template3 => {
                (Self::get_buf_pixel(self.buf_m1, 0) << 5)
                    | (Self::get_buf_pixel(self.buf_m1, 1) << 4)
            }
        }
    }

    #[inline(always)]
    pub(crate) fn load_word(bitmap: &Bitmap, row_y: u32, start_x: u32) -> Word {
        let word_idx = start_x / WORD_BITS;

        if start_x.is_multiple_of(WORD_BITS) {
            bitmap.get_word(row_y, word_idx)
        } else {
            let bit_offset = start_x % WORD_BITS;
            let word1 = bitmap.get_word(row_y, word_idx);
            let word2 = bitmap.get_word(row_y, word_idx + 1);
            (word1 << bit_offset) | (word2 >> (WORD_BITS - bit_offset))
        }
    }

    #[inline]
    pub(crate) fn get_buf_pixel(buf: Word, pos: u32) -> u16 {
        if pos < WORD_BITS {
            ((buf >> (WORD_SHIFT - pos)) & 1) as u16
        } else {
            0
        }
    }

    #[inline(always)]
    pub(crate) fn maybe_reload_buffers(&mut self, bitmap: &Bitmap, x: u32) {
        if x + self.max_right >= self.cur_x + WORD_BITS {
            let new_start = x.saturating_sub(4);
            self.cur_x = new_start;
            self.buf_m2 = if self.cur_y >= 2 {
                Self::load_word(bitmap, self.cur_y - 2, new_start)
            } else {
                0
            };
            self.buf_m1 = if self.cur_y >= 1 {
                Self::load_word(bitmap, self.cur_y - 1, new_start)
            } else {
                0
            };
            self.buf_cur = Self::load_word(bitmap, self.cur_y, new_start);
        }
    }

    #[inline(always)]
    fn gather_template0_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 12)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 5)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            // Note that the cast from i32 to u32 is deliberate here: Negative positions
            // should be resolved to 0, by casting negative i32 to u32 we end up with
            // a very large positive number which is guaranteed to be OOB. The same
            // applies to all other occurrences of this pattern (also in `generic_refinement.rs`).
            | ((bitmap.get_pixel((xi + self.at_pixels[3].x as i32) as u32, (yi + self.at_pixels[3].y as i32) as u32) as u16) << 15)
            | ((bitmap.get_pixel((xi + self.at_pixels[2].x as i32) as u32, (yi + self.at_pixels[2].y as i32) as u32) as u16) << 11)
            | ((bitmap.get_pixel((xi + self.at_pixels[1].x as i32) as u32, (yi + self.at_pixels[1].y as i32) as u32) as u16) << 10)
            | ((bitmap.get_pixel((xi + self.at_pixels[0].x as i32) as u32, (yi + self.at_pixels[0].y as i32) as u32) as u16) << 4);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T0) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template1_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 9)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | ((bitmap.get_pixel(
                (xi + self.at_pixels[0].x as i32) as u32,
                (yi + self.at_pixels[0].y as i32) as u32,
            ) as u16)
                << 3);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T1) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template2_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 7)
            | (Self::get_buf_pixel(self.buf_m1, bx + 1) << 3)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | ((bitmap.get_pixel(
                (xi + self.at_pixels[0].x as i32) as u32,
                (yi + self.at_pixels[0].y as i32) as u32,
            ) as u16)
                << 2);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T2) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template3_custom(&mut self, bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let xi = x as i32;
        let yi = self.cur_y as i32;

        let new_pixels = (Self::get_buf_pixel(self.buf_m1, bx + 1) << 5)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1))
            | ((bitmap.get_pixel(
                (xi + self.at_pixels[0].x as i32) as u32,
                (yi + self.at_pixels[0].y as i32) as u32,
            ) as u16)
                << 4);

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T3) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    pub(crate) fn gather_template0_default(&mut self, _bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 11)
            | (Self::get_buf_pixel(self.buf_m1, bx + 3) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T0_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    pub(crate) fn gather_template1_default(&mut self, _bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 2) << 9)
            | (Self::get_buf_pixel(self.buf_m1, bx + 3) << 3)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T1_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    pub(crate) fn gather_template2_default(&mut self, _bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m2, bx + 1) << 7)
            | (Self::get_buf_pixel(self.buf_m1, bx + 2) << 2)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T2_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    pub(crate) fn gather_template3_default(&mut self, _bitmap: &Bitmap, x: u32) -> u16 {
        let bx = x - self.cur_x;
        let new_pixels = (Self::get_buf_pixel(self.buf_m1, bx + 2) << 4)
            | Self::get_buf_pixel(self.buf_cur, bx.wrapping_sub(1));

        self.ctx = ((self.ctx << 1) & Self::SHIFT_MASK_T3_DEFAULT) | new_pixels;
        self.ctx
    }

    /// Note: The caller must ensure that `gather` has been called for this `x`
    /// first.
    #[inline(always)]
    pub(crate) fn update_current_row(&mut self, x: u32, value: u8) {
        debug_assert!(x >= self.cur_x && x < self.cur_x + WORD_BITS);

        let bit_pos = WORD_SHIFT - (x - self.cur_x);
        #[allow(trivial_numeric_casts)]
        let mask = (1 as Word) << bit_pos;
        self.buf_cur = (self.buf_cur & !mask) | ((value as Word) << bit_pos);
    }
}
