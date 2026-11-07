//! Generic refinement region segment parsing and decoding (7.4.7, 6.3).

use alloc::vec::Vec;

use super::{
    AdaptiveTemplatePixel, RefinementTemplate, RegionBitmap, RegionSegmentInfo,
    parse_refinement_at_pixels, parse_region_segment_info,
};
use crate::ScratchBuffers;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::{Bitmap, WORD_BITS, WORD_SHIFT, Word};
use crate::decode::generic::ContextGatherer;
use crate::error::{ParseError, RegionError, Result, bail};
use crate::reader::Reader;

/// Generic refinement region decoding procedure (6.3).
pub(crate) fn decode(
    header: &GenericRefinementRegionHeader<'_>,
    reference: &Bitmap,
    ctx: &mut ScratchBuffers,
) -> Result<RegionBitmap> {
    let mut region = Bitmap::new_with(
        header.region_info.width,
        header.region_info.height,
        header.region_info.x_location,
        header.region_info.y_location,
        false,
    )?;

    decode_into(header, reference, &mut region, ctx)?;

    Ok(RegionBitmap {
        bitmap: region,
        combination_operator: header.region_info.combination_operator,
    })
}

/// Decode a generic refinement region directly into an existing bitmap.
pub(crate) fn decode_into(
    header: &GenericRefinementRegionHeader<'_>,
    reference: &Bitmap,
    region: &mut Bitmap,
    ctx: &mut ScratchBuffers,
) -> Result<()> {
    let data = header.data;

    // Validate that the region fits within the reference bitmap.
    // When referring to another segment, dimensions must match exactly (7.4.7.5).
    // PdfCraft patch: a refinement of the page refines the page's area of its own size (7.4.7.4,
    // `Bitmap::area`), which it always fits.
    if header.region_info.width > reference.width || header.region_info.height > reference.height {
        bail!(RegionError::InvalidDimension);
    }

    // PdfCraft patch: "GRREFERENCEDX 0, GRREFERENCEDY 0" (Table 35). The reference is another
    // region, which a refinement refines where it is (their locations should be equal, 7.4.7.5),
    // or the page restricted to the refinement's area (7.4.7.4, `Bitmap::area`). Offsetting it by
    // the difference of the locations refused one past `i32::MAX` as an overflow, and read the
    // page around a refinement of it.
    let (reference_dx, reference_dy) = (0, 0);

    let mut decoder = ArithmeticDecoder::new(data);
    let num_context_bits = header.template.context_bits();
    ctx.contexts.clear();
    ctx.contexts
        .resize(1 << num_context_bits, ArithmeticDecoderContext::default());

    decode_bitmap(
        &mut decoder,
        &mut ctx.contexts,
        region,
        reference,
        reference_dx,
        reference_dy,
        header.template,
        &header.adaptive_template_pixels,
        header.tpgron,
    )?;

    Ok(())
}

/// Parsed generic refinement region segment header (7.4.7.1).
#[derive(Debug, Clone)]
pub(crate) struct GenericRefinementRegionHeader<'a> {
    pub(crate) region_info: RegionSegmentInfo,
    pub(crate) template: RefinementTemplate,
    pub(crate) tpgron: bool,
    pub(crate) adaptive_template_pixels: Vec<AdaptiveTemplatePixel>,
    pub(crate) data: &'a [u8],
}

/// Parse a generic refinement region segment header (7.4.7.1).
pub(crate) fn parse<'a>(reader: &mut Reader<'a>) -> Result<GenericRefinementRegionHeader<'a>> {
    let region_info = parse_region_segment_info(reader)?;
    let flags = reader.read_byte().ok_or(ParseError::UnexpectedEof)?;
    let template = RefinementTemplate::from_byte(flags);
    let tpgron = flags & 0x02 != 0;
    let adaptive_template_pixels = if template == RefinementTemplate::Template0 {
        parse_refinement_at_pixels(reader)?
    } else {
        Vec::new()
    };
    let data = reader.tail().ok_or(ParseError::UnexpectedEof)?;

    Ok(GenericRefinementRegionHeader {
        region_info,
        template,
        tpgron,
        adaptive_template_pixels,
        data,
    })
}

fn has_default_at_pixels(at_pixels: &[AdaptiveTemplatePixel]) -> bool {
    at_pixels.len() >= 2
        && at_pixels[0].x == -1
        && at_pixels[0].y == -1
        && at_pixels[1].x == -1
        && at_pixels[1].y == -1
}

pub(crate) struct RefinementContextGatherer<'a> {
    template: RefinementTemplate,
    at_pixels: &'a [AdaptiveTemplatePixel],
    use_default_at: bool,
    reference_dx: i32,
    reference_dy: i32,
    reg_y: u32,
    reg_cur_x: u32,
    reg_m1: Word,
    reg_cur: Word,
    ref_cur_x: u32,
    ref_m1: Word,
    ref_cur: Word,
    ref_p1: Word,
    ctx: u16,
}

const SHIFT_MASK_T0_DEFAULT: u16 = 0b1_1001_1011_0110;
const SHIFT_MASK_T0_CUSTOM: u16 = 0b0_1000_1011_0110;
const SHIFT_MASK_T1: u16 = 0b11_0001_1010;

impl<'a> RefinementContextGatherer<'a> {
    pub(crate) fn new(
        template: RefinementTemplate,
        at_pixels: &'a [AdaptiveTemplatePixel],
        reference_dx: i32,
        reference_dy: i32,
    ) -> Self {
        let use_default_at =
            template == RefinementTemplate::Template0 && has_default_at_pixels(at_pixels);
        Self {
            template,
            at_pixels,
            use_default_at,
            reference_dx,
            reference_dy,
            reg_y: 0,
            reg_cur_x: 0,
            reg_m1: 0,
            reg_cur: 0,
            ref_cur_x: 0,
            ref_m1: 0,
            ref_cur: 0,
            ref_p1: 0,
            ctx: 0,
        }
    }

    pub(crate) fn start_row(&mut self, region: &Bitmap, reference: &Bitmap, y: u32) {
        self.reg_y = y;
        // The offsets can be anywhere an `i32` reaches, so coordinates in the reference wrap here and
        // below. Wrapped, they are off the reference, whose pixels there read as 0.
        let ref_y = (y as i32).wrapping_sub(self.reference_dy);
        // This will on purpose wrap to a very large `u32` for negative values, such that
        // they resolve to 0 in `ContextGatherer::load_word`.
        let ref_x0 = self.reference_dx.wrapping_neg() as u32;

        self.reg_cur_x = 0;
        self.reg_m1 = if y >= 1 {
            ContextGatherer::load_word(region, y - 1, 0)
        } else {
            0
        };
        self.reg_cur = 0;

        // PdfCraft patch: the buffers start a column left of the reference column the row starts
        // at, as `maybe_reload_buffers` would load them, or at column 0 where that is left of the
        // reference. The row's first context takes in that column and the one left of it, which
        // from column 32 on lay outside a word loaded from column 0 and read as 0: with the
        // reference offset 32 left of the region its first column, and from 33 both (GRREFERENCEDX
        // below −31, which a refined text instance has from a narrower bitmap or an RDX below −31,
        // and a refined symbol from such an RDX). The region decoded to other pixels than were
        // coded, or failed.
        self.ref_cur_x = self
            .reference_dx
            .checked_neg()
            .and_then(|column| u32::try_from(column).ok())
            .map_or(0, |column| column.saturating_sub(1));
        self.ref_m1 =
            ContextGatherer::load_word(reference, ref_y.wrapping_sub(1) as u32, self.ref_cur_x);
        self.ref_cur = ContextGatherer::load_word(reference, ref_y as u32, self.ref_cur_x);
        self.ref_p1 =
            ContextGatherer::load_word(reference, ref_y.wrapping_add(1) as u32, self.ref_cur_x);

        self.ctx = match self.template {
            RefinementTemplate::Template1 => {
                let rbx = ref_x0.wrapping_sub(self.ref_cur_x);

                let reg_m1_0 = ContextGatherer::get_buf_pixel(self.reg_m1, 0);
                let ref_cur_m1 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx.wrapping_sub(1));
                let ref_cur_0 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx);
                let ref_p1_0 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx);

                (reg_m1_0 << 7) | (ref_cur_m1 << 3) | (ref_cur_0 << 2) | (ref_p1_0)
            }
            RefinementTemplate::Template0 if self.use_default_at => {
                let rbx = ref_x0.wrapping_sub(self.ref_cur_x);
                let reg_m1_0 = ContextGatherer::get_buf_pixel(self.reg_m1, 0);
                let ref_m1_m1 = ContextGatherer::get_buf_pixel(self.ref_m1, rbx.wrapping_sub(1));
                let ref_m1_0 = ContextGatherer::get_buf_pixel(self.ref_m1, rbx);
                let ref_cur_m1 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx.wrapping_sub(1));
                let ref_cur_0 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx);
                let ref_p1_m1 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx.wrapping_sub(1));
                let ref_p1_0 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx);

                (reg_m1_0 << 10)
                    | (ref_m1_m1 << 7)
                    | (ref_m1_0 << 6)
                    | (ref_cur_m1 << 4)
                    | (ref_cur_0 << 3)
                    | (ref_p1_m1 << 1)
                    | ref_p1_0
            }
            RefinementTemplate::Template0 => {
                let rbx = ref_x0.wrapping_sub(self.ref_cur_x);
                let reg_m1_0 = ContextGatherer::get_buf_pixel(self.reg_m1, 0);
                let ref_m1_0 = ContextGatherer::get_buf_pixel(self.ref_m1, rbx);
                let ref_cur_m1 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx.wrapping_sub(1));
                let ref_cur_0 = ContextGatherer::get_buf_pixel(self.ref_cur, rbx);
                let ref_p1_m1 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx.wrapping_sub(1));
                let ref_p1_0 = ContextGatherer::get_buf_pixel(self.ref_p1, rbx);
                (reg_m1_0 << 10)
                    | (ref_m1_0 << 6)
                    | (ref_cur_m1 << 4)
                    | (ref_cur_0 << 3)
                    | (ref_p1_m1 << 1)
                    | ref_p1_0
            }
        };
    }

    #[inline(always)]
    pub(crate) fn maybe_reload_buffers(&mut self, region: &Bitmap, reference: &Bitmap, x: u32) {
        if x + 1 >= self.reg_cur_x + WORD_BITS {
            let new_start = x.saturating_sub(1);
            self.reg_cur_x = new_start;
            self.reg_m1 = if self.reg_y >= 1 {
                ContextGatherer::load_word(region, self.reg_y - 1, new_start)
            } else {
                0
            };
            self.reg_cur = ContextGatherer::load_word(region, self.reg_y, new_start);
        }

        // PdfCraft patch: left of the reference the buffers keep what `start_row` loaded, its words
        // from column 0. The context takes in the reference a column to the right of the current
        // one (Figures 12 and 13), so at column −1 it reads column 0, and from 0 on the buffers are
        // the reference's own. They were zeroed here and, at column 0, not loaded again: with the
        // reference offset right of the region (GRREFERENCEDX above 0, which a refined text
        // instance or symbol has from a wider bitmap or an RDX above 0), its first 32 columns read
        // as 0, and the region decoded to other pixels than were coded, or as garbage that failed.
        let ref_x_signed = (x as i32).wrapping_sub(self.reference_dx);
        if let Ok(ref_x) = u32::try_from(ref_x_signed)
            && (ref_x + 1 >= self.ref_cur_x + WORD_BITS || ref_x < self.ref_cur_x)
        {
            let new_start = ref_x.saturating_sub(1);
            let ref_y = (self.reg_y as i32).wrapping_sub(self.reference_dy);
            self.ref_cur_x = new_start;
            self.ref_m1 =
                ContextGatherer::load_word(reference, ref_y.wrapping_sub(1) as u32, new_start);
            self.ref_cur = ContextGatherer::load_word(reference, ref_y as u32, new_start);
            self.ref_p1 =
                ContextGatherer::load_word(reference, ref_y.wrapping_add(1) as u32, new_start);
        }
    }

    #[inline(always)]
    pub(crate) fn tpgr_all_same(&self, x: u32) -> bool {
        let ref_x = (x as i32).wrapping_sub(self.reference_dx) as u32;
        let rbx = ref_x.wrapping_sub(self.ref_cur_x);

        #[inline(always)]
        fn extract_3bits(buf: Word, rbx: u32) -> u32 {
            if rbx == 0 || rbx >= WORD_BITS - 1 {
                // Near word boundary, fall back to individual extraction.
                let b0 = ContextGatherer::get_buf_pixel(buf, rbx.wrapping_sub(1)) as u32;
                let b1 = ContextGatherer::get_buf_pixel(buf, rbx) as u32;
                let b2 = ContextGatherer::get_buf_pixel(buf, rbx.wrapping_add(1)) as u32;
                (b0 << 2) | (b1 << 1) | b2
            } else {
                // Fast path: shift and mask 3 bits at once.
                #[allow(trivial_numeric_casts)]
                #[allow(clippy::unnecessary_cast)]
                {
                    ((buf >> (WORD_SHIFT - rbx - 1)) & 0b111) as u32
                }
            }
        }

        let m1 = extract_3bits(self.ref_m1, rbx);
        let cur = extract_3bits(self.ref_cur, rbx);
        let p1 = extract_3bits(self.ref_p1, rbx);

        // Check that all 9 pixels are the same.
        m1 == cur && cur == p1 && (cur == 0 || cur == 0b111)
    }

    #[inline(always)]
    pub(crate) fn ref_center_pixel(&self, x: u32) -> u8 {
        let ref_x = (x as i32).wrapping_sub(self.reference_dx) as u32;
        let rbx = ref_x.wrapping_sub(self.ref_cur_x);

        ContextGatherer::get_buf_pixel(self.ref_cur, rbx) as u8
    }

    #[inline(always)]
    pub(crate) fn update_current_row(&mut self, x: u32, value: u8) {
        let bx = x - self.reg_cur_x;
        debug_assert!(bx < WORD_BITS);
        let bit_pos = WORD_SHIFT - bx;
        #[allow(trivial_numeric_casts)]
        let mask = (1 as Word) << bit_pos;
        self.reg_cur = (self.reg_cur & !mask) | ((value as Word) << bit_pos);
    }

    #[inline(always)]
    fn gather_template0_default(&mut self, _region: &Bitmap, _reference: &Bitmap, x: u32) -> u16 {
        let bx = x - self.reg_cur_x;
        let ref_x = (x as i32).wrapping_sub(self.reference_dx) as u32;
        let rbx = ref_x.wrapping_sub(self.ref_cur_x);

        let new_pixels = (ContextGatherer::get_buf_pixel(self.reg_m1, bx + 1) << 10)
            | (ContextGatherer::get_buf_pixel(self.reg_cur, bx.wrapping_sub(1)) << 9)
            | (ContextGatherer::get_buf_pixel(self.ref_m1, rbx.wrapping_add(1)) << 6)
            | (ContextGatherer::get_buf_pixel(self.ref_cur, rbx.wrapping_add(1)) << 3)
            | ContextGatherer::get_buf_pixel(self.ref_p1, rbx.wrapping_add(1));

        self.ctx = ((self.ctx << 1) & SHIFT_MASK_T0_DEFAULT) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template0_custom(&mut self, region: &Bitmap, reference: &Bitmap, x: u32) -> u16 {
        let bx = x - self.reg_cur_x;
        let ref_x_i = (x as i32).wrapping_sub(self.reference_dx);
        let rbx = ref_x_i as u32 - self.ref_cur_x;

        let xi = x as i32;
        let yi = self.reg_y as i32;
        let ref_y = yi.wrapping_sub(self.reference_dy);
        let at1 = self.at_pixels[0];
        let at2 = self.at_pixels[1];

        let new_pixels = ((region.get_pixel((xi + at1.x as i32) as u32, (yi + at1.y as i32) as u32)
            as u16)
            << 12)
            | (ContextGatherer::get_buf_pixel(self.reg_m1, bx + 1) << 10)
            | (ContextGatherer::get_buf_pixel(self.reg_cur, bx.wrapping_sub(1)) << 9)
            | ((reference.get_pixel(
                ref_x_i.wrapping_add(at2.x as i32) as u32,
                ref_y.wrapping_add(at2.y as i32) as u32,
            ) as u16)
                << 8)
            | (ContextGatherer::get_buf_pixel(self.ref_m1, rbx.wrapping_add(1)) << 6)
            | (ContextGatherer::get_buf_pixel(self.ref_cur, rbx.wrapping_add(1)) << 3)
            | ContextGatherer::get_buf_pixel(self.ref_p1, rbx.wrapping_add(1));

        self.ctx = ((self.ctx << 1) & SHIFT_MASK_T0_CUSTOM) | new_pixels;
        self.ctx
    }

    #[inline(always)]
    fn gather_template1(&mut self, _region: &Bitmap, _reference: &Bitmap, x: u32) -> u16 {
        let bx = x - self.reg_cur_x;
        let ref_x = (x as i32).wrapping_sub(self.reference_dx) as u32;
        let rbx = ref_x.wrapping_sub(self.ref_cur_x);

        let new_pixels = (ContextGatherer::get_buf_pixel(self.reg_m1, bx + 1) << 7)
            | (ContextGatherer::get_buf_pixel(self.reg_cur, bx.wrapping_sub(1)) << 6)
            | (ContextGatherer::get_buf_pixel(self.ref_m1, rbx) << 5)
            | (ContextGatherer::get_buf_pixel(self.ref_cur, rbx.wrapping_add(1)) << 2)
            | ContextGatherer::get_buf_pixel(self.ref_p1, rbx.wrapping_add(1));

        self.ctx = ((self.ctx << 1) & SHIFT_MASK_T1) | new_pixels;
        self.ctx
    }
}

/// Decode a refinement bitmap (6.3.5.6).
pub(crate) fn decode_bitmap(
    decoder: &mut ArithmeticDecoder<'_>,
    contexts: &mut [ArithmeticDecoderContext],
    region: &mut Bitmap,
    reference: &Bitmap,
    reference_dx: i32,
    reference_dy: i32,
    gr_template: RefinementTemplate,
    adaptive_template_pixels: &[AdaptiveTemplatePixel],
    tpgron: bool,
) -> Result<()> {
    macro_rules! refinement_decode_loop {
        ($gatherer:expr, $tpgron:expr, $sltp_context:expr, $gather:expr) => {{
            let gatherer: &mut RefinementContextGatherer<'_> = $gatherer;
            let width = region.width;
            let height = region.height;

            // "1) Set LTP = 0." (6.3.5.6)
            let mut ltp = false;

            // "3) Decode each row as follows:" (6.3.5.6)
            for y in 0..height {
                // "b) If TPGRON is 1, then decode a bit using the arithmetic entropy
                // coder" (6.3.5.6)
                if $tpgron {
                    let sltp = decoder.read_bit(&mut contexts[$sltp_context as usize]);
                    // "Let SLTP be the value of this bit. Set: LTP = LTP XOR SLTP"
                    ltp = ltp != (sltp != 0);
                }

                gatherer.start_row(region, reference, y);

                // "c) If LTP = 0 then, from left to right, explicitly decode all pixels
                // of the current row of GRREG." (6.3.5.6)
                if !ltp {
                    for x in 0..width {
                        gatherer.maybe_reload_buffers(region, reference, x);
                        let context = ($gather)(gatherer, region, reference, x) as usize;
                        let pixel = decoder.read_bit(&mut contexts[context]);
                        region.set_pixel(x, y, pixel as u8);
                        gatherer.update_current_row(x, pixel as u8);
                    }
                } else {
                    // "d) If LTP = 1 then, from left to right, implicitly decode certain
                    // pixels of the current row of GRREG, and explicitly decode the rest."
                    // (6.3.5.6)
                    for x in 0..width {
                        gatherer.maybe_reload_buffers(region, reference, x);

                        let context = ($gather)(gatherer, region, reference, x) as usize;

                        // "i) Set TPGRPIX equal to 1 if:
                        //    - TPGRON is 1 AND;
                        //    - a 3 × 3 pixel array in the reference bitmap (Figure 16),
                        //      centred at the location corresponding to the current pixel,
                        //      contains pixels all of the same value." (6.3.5.6)
                        if gatherer.tpgr_all_same(x) {
                            // "ii) If TPGRPIX is 1 then implicitly decode the current pixel
                            // by setting it equal to its predicted value (TPGRVAL)." (6.3.5.6)
                            //
                            // "When TPGRPIX is set to 1, set TPGRVAL equal to the current pixel
                            // predicted value, which is the common value of the nine adjacent
                            // pixels in the 3 × 3 array." (6.3.5.6)
                            let val = gatherer.ref_center_pixel(x);
                            region.set_pixel(x, y, val);
                            gatherer.update_current_row(x, val);
                        } else {
                            // "iii) Otherwise, explicitly decode the current pixel using the
                            // methodology of steps 3 c) i) through 3 c) iii) above." (6.3.5.6)
                            let pixel = decoder.read_bit(&mut contexts[context]);
                            region.set_pixel(x, y, pixel as u8);
                            gatherer.update_current_row(x, pixel as u8);
                        }
                    }
                }
            }
        }};
    }

    let mut gatherer = RefinementContextGatherer::new(
        gr_template,
        adaptive_template_pixels,
        reference_dx,
        reference_dy,
    );

    // Context for SLTP depends on template (Figures 14, 15).
    let sltp_context: u16 = match gr_template {
        RefinementTemplate::Template0 => 0b0000000010000,
        RefinementTemplate::Template1 => 0b0000001000,
    };

    match gr_template {
        RefinementTemplate::Template0 if gatherer.use_default_at => refinement_decode_loop!(
            &mut gatherer,
            tpgron,
            sltp_context,
            RefinementContextGatherer::gather_template0_default
        ),
        RefinementTemplate::Template0 => refinement_decode_loop!(
            &mut gatherer,
            tpgron,
            sltp_context,
            RefinementContextGatherer::gather_template0_custom
        ),
        RefinementTemplate::Template1 => refinement_decode_loop!(
            &mut gatherer,
            tpgron,
            sltp_context,
            RefinementContextGatherer::gather_template1
        ),
    }

    Ok(())
}
