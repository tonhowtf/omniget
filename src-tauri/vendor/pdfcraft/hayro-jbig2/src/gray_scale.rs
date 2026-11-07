//! Gray-scale image decoding procedure (Annex C).

use alloc::vec;
use alloc::vec::Vec;

use crate::ScratchBuffers;
use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};
use crate::bitmap::{Bitmap, MAX_DIMENSION, WORD_BITS};
use crate::decode::generic::{ContextGatherer, decode_bitmap_mmr};
use crate::decode::{AdaptiveTemplatePixel, Template};
use crate::error::{OverflowError, Result, bail};
use crate::simd::{self, Level, Simd, u32x8};

/// Input parameters to the gray-scale image decoding procedure (Table C.1).
#[derive(Debug, Clone)]
pub(crate) struct GrayScaleParams<'a> {
    /// `GSMMR` - Specifies whether MMR is used.
    pub(crate) use_mmr: bool,
    /// `GSBPP` - The number of bits per gray-scale value.
    pub(crate) bits_per_pixel: u32,
    /// `GSW` - The width of the gray-scale image.
    pub(crate) width: u32,
    /// `GSH` - The height of the gray-scale image.
    pub(crate) height: u32,
    /// `GSTEMPLATE` - The template used to code the gray-scale bitplanes.
    pub(crate) template: Template,
    /// `GSKIP` - A mask indicating which values should be skipped.
    /// None if `GSUSESKIP` = 0.
    pub(crate) skip_mask: Option<&'a [u32]>,
}

/// The gray-scale image decoding procedure (Annex C, C.5).
///
/// Returns `GSVALS`: the decoded gray-scale image array, GSW × GSH pixels.
#[inline(always)]
pub(crate) fn decode_gray_scale_image(
    data: &[u8],
    params: &GrayScaleParams<'_>,
    ctx: &mut ScratchBuffers,
) -> Result<Vec<u32>> {
    // Table C.1: "GSMMR specifies whether MMR is used."
    if params.use_mmr {
        decode_mmr(data, params)
    } else {
        decode_arithmetic(data, params, ctx)
    }
}

/// The gray-scale image decoding procedure using MMR (Annex C, C.5).
///
/// Table C.4: "MMR = GSMMR"
fn decode_mmr(data: &[u8], params: &GrayScaleParams<'_>) -> Result<Vec<u32>> {
    // `GSW` - The width of the gray-scale image.
    let width = params.width;
    // `GSH` - The height of the gray-scale image.
    let height = params.height;
    // `GSBPP` - The number of bits per gray-scale value.
    let bits_per_pixel = params.bits_per_pixel;
    let stride = width.div_ceil(WORD_BITS);

    let mut offset = 0;
    decode_bitplanes(width, height, stride, bits_per_pixel, |_, bitplane| {
        // Table C.4: "GBW = GSW, GBH = GSH"
        offset += decode_bitmap_mmr(bitplane, &data[offset..])?;

        Ok(())
    })
}

/// The gray-scale image decoding procedure using arithmetic coding (Annex C, C.5).
///
/// Table C.4: "GBTEMPLATE = GSTEMPLATE, TPGDON = 0, USESKIP = GSUSESKIP, SKIP = GSKIP"
fn decode_arithmetic(
    data: &[u8],
    params: &GrayScaleParams<'_>,
    ctx: &mut ScratchBuffers,
) -> Result<Vec<u32>> {
    // `GSW` - The width of the gray-scale image.
    let width = params.width;
    // `GSH` - The height of the gray-scale image.
    let height = params.height;

    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        bail!(OverflowError::BitmapDimension);
    }
    // PdfCraft patch: the product is bounded too (see `bitmap::MAX_GRID_CELLS`).
    if u64::from(width) * u64::from(height) > crate::bitmap::MAX_GRID_CELLS {
        bail!(OverflowError::BitmapDimension);
    }

    // `GSBPP` - The number of bits per gray-scale value.
    let bits_per_pixel = params.bits_per_pixel;
    let stride = width.div_ceil(WORD_BITS);
    // `GSKIP` - The skip mask (if GSUSESKIP = 1).
    let skip_mask = params.skip_mask;
    let skip_stride = width.div_ceil(32);
    // `GSTEMPLATE` - The template used for bitplane decoding.
    let template = params.template;

    // Table C.4: Adaptive template pixel positions.
    let at_pixels: [AdaptiveTemplatePixel; 4] = match template {
        Template::Template0 => [
            AdaptiveTemplatePixel { x: 3, y: -1 },
            AdaptiveTemplatePixel { x: -3, y: -1 },
            AdaptiveTemplatePixel { x: 2, y: -2 },
            AdaptiveTemplatePixel { x: -2, y: -2 },
        ],
        Template::Template1 => [
            AdaptiveTemplatePixel { x: 3, y: -1 },
            // Unused.
            AdaptiveTemplatePixel::default(),
            AdaptiveTemplatePixel::default(),
            AdaptiveTemplatePixel::default(),
        ],
        Template::Template2 | Template::Template3 => [
            AdaptiveTemplatePixel { x: 2, y: -1 },
            // Unused.
            AdaptiveTemplatePixel::default(),
            AdaptiveTemplatePixel::default(),
            AdaptiveTemplatePixel::default(),
        ],
    };

    let mut decoder = ArithmeticDecoder::new(data);
    ctx.contexts.clear();
    ctx.contexts.resize(
        1 << template.context_bits(),
        ArithmeticDecoderContext::default(),
    );

    macro_rules! gs_decode_loop {
        ($gather:expr) => {
            decode_bitplanes(width, height, stride, bits_per_pixel, |_, bitplane| {
                // Table C.4: "GBW = GSW, GBH = GSH, TPGDON = 0"
                let mut gatherer = ContextGatherer::new(template, &at_pixels);

                for y in 0..height {
                    gatherer.start_row(bitplane, y);
                    for x in 0..width {
                        gatherer.maybe_reload_buffers(bitplane, x);

                        // Table C.4: "USESKIP = GSUSESKIP, SKIP = GSKIP"
                        if let Some(mask) = skip_mask {
                            let word_idx = (y * skip_stride + x / 32) as usize;
                            let bit_pos = 31 - (x % 32);
                            if (mask[word_idx] >> bit_pos) & 1 != 0 {
                                // Still need to update the context.
                                let _ = ($gather)(&mut gatherer, bitplane, x);
                                gatherer.update_current_row(x, 0);
                                continue;
                            }
                        }

                        let context = ($gather)(&mut gatherer, bitplane, x);
                        let pixel = decoder.read_bit(&mut ctx.contexts[context as usize]);
                        let value = pixel as u8;

                        bitplane.set_pixel(x, y, value);
                        gatherer.update_current_row(x, value);
                    }
                }

                Ok(())
            })
        };
    }

    match template {
        Template::Template0 => gs_decode_loop!(ContextGatherer::gather_template0_default),
        Template::Template1 => gs_decode_loop!(ContextGatherer::gather_template1_default),
        Template::Template2 => gs_decode_loop!(ContextGatherer::gather_template2_default),
        Template::Template3 => gs_decode_loop!(ContextGatherer::gather_template3_default),
    }
}

/// The bitplane decoding and gray value computation procedure (C.5).
///
/// The closure `decode_next` is called for each bitplane, receiving the bitplane
/// index (`GSBPP`-1 down to 0) and a zeroed bitmap to decode into.
fn decode_bitplanes<F>(
    width: u32,
    height: u32,
    stride: u32,
    bits_per_pixel: u32,
    mut decode_next: F,
) -> Result<Vec<u32>>
where
    F: FnMut(u32, &mut Bitmap) -> Result<()>,
{
    let size = width as usize * height as usize;
    // `GSVALS` - The decoded gray-scale image array.
    let mut values = vec![0_u32; size];

    let mut bitplane = Bitmap::new(width, height)?;
    let mut prev_plane = vec![0; bitplane.data.len()];

    simd::dispatch!(Level::new(), simd => {
        for j in (0..bits_per_pixel).rev() {
            bitplane.data.fill(0);
            decode_next(j, &mut bitplane)?;

            // Done every time except the first time.
            if j < bits_per_pixel - 1 {
                // C.5 step 3b: "GSPLANES[J][x, y] = GSPLANES[J + 1][x, y] XOR GSPLANES[J][x, y]"
                xor_planes(simd, &mut bitplane.data, &prev_plane);
            }

            // C.5 step 4: "GSVALS[x, y] = sum(J=0 to GSBPP-1) GSPLANES[J][x, y] * 2^J"
            extract_bits(simd, &bitplane.data, &mut values, width, height, stride, j);

            // C.5 step 3c: "Set J = J - 1."
            prev_plane.copy_from_slice(&bitplane.data);
        }

        Ok(values)
    })
}

#[inline(always)]
fn xor_planes<S: Simd>(simd: S, plane: &mut [u32], prev: &[u32]) {
    let simd_end = plane.len() & !7;

    for i in (0..simd_end).step_by(simd::SIMD_WIDTH) {
        let chunk = &mut plane[i..i + simd::SIMD_WIDTH];
        let mut a = u32x8::from_slice(simd, chunk);
        let b = u32x8::from_slice(simd, &prev[i..i + simd::SIMD_WIDTH]);
        a ^= b;
        a.store(chunk);
    }

    for i in simd_end..plane.len() {
        plane[i] ^= prev[i];
    }
}

#[inline(always)]
fn extract_bits<S: Simd>(
    simd: S,
    data: &[u32],
    values: &mut [u32],
    width: u32,
    height: u32,
    stride: u32,
    j: u32,
) {
    /// Bit masks for extracting individual bits from a packed u32 word (MSB-first).
    /// `BIT_MASKS[i] = 1 << (31 - i)`.
    const BIT_MASKS: [u32; 32] = {
        let mut masks = [0_u32; 32];
        let mut i = 0;
        while i < 32 {
            masks[i] = 1 << (31 - i);
            i += 1;
        }
        masks
    };

    let bit_value = u32x8::splat(simd, 1_u32 << j);
    let zero = u32x8::splat(simd, 0);
    let simd_end = (width as usize) & !7;

    for y in 0..height as usize {
        let row_data = y * stride as usize;
        let row_vals = y * width as usize;

        // Handle in chunks of 8.
        for x in (0..simd_end).step_by(simd::SIMD_WIDTH) {
            let word = data[row_data + x / 32];
            let bit_offset = x % 32;

            let splatted = u32x8::splat(simd, word);
            let masks =
                u32x8::from_slice(simd, &BIT_MASKS[bit_offset..bit_offset + simd::SIMD_WIDTH]);
            let extracted = splatted & masks;
            let nonzero = extracted.simd_gt(zero);
            let contribution = nonzero.select(bit_value, zero);
            let vals = &mut values[row_vals + x..row_vals + x + simd::SIMD_WIDTH];
            let current = u32x8::from_slice(simd, vals);
            (current | contribution).store(vals);
        }

        // Handle the tail.
        for x in simd_end..width as usize {
            let word_idx = row_data + x / 32;
            let bit_pos = 31 - (x % 32);
            if (data[word_idx] >> bit_pos) & 1 != 0 {
                values[row_vals + x] |= 1 << j;
            }
        }
    }
}
