use crate::bit_reader::{BitWriter, bit_mask};
use crate::filter::FilterResult;
use crate::object::stream::{ImageColorSpace, ImageData, ImageDecodeParams};
use alloc::borrow::Cow;
use alloc::vec;
use alloc::vec::Vec;
use hayro_jpeg2000::{ColorSpace, DecodeSettings};

impl ImageColorSpace {
    fn num_components(&self) -> u8 {
        match self {
            Self::Gray => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
            Self::Unknown(num) => *num,
        }
    }
}

pub(crate) fn decode(data: &[u8], params: &ImageDecodeParams) -> Option<FilterResult<'static>> {
    use crate::object::stream::ImageColorSpace;

    let settings = DecodeSettings {
        // PdfCraft patch: (#3179) with no /ColorSpace in the image dictionary the colours come from
        // the JP2 header (ISO 32000-2 §7.4.9), so its palette (`pclr`) is applied here; with a
        // /ColorSpace the header colour data is ignored, and an /Indexed space takes the raw indices.
        // A /ColorSpace that does not resolve also leaves `num_components` unset: the header's
        // colours are then the only ones there are, so the palette is applied too (raw indices
        // would be drawn as gray).
        resolve_palette_indices: params.num_components.is_none(),
        strict: false,
        target_resolution: params.target_dimension,
    };

    // PdfCraft patch (21): a JP2 colour specification naming an enumerated colour space the
    // decoder refuses (CIEJab, enum 19, in a veraPDF fixture; YCbCr, PhotoYCC, …) made the whole
    // image fail, so the page drew blank (#3306). Then it is decoded as if it had no colour
    // specification, as MuPDF and Poppler draw it: grey, RGB or CMYK by its component count.
    let repaired;
    let image = match hayro_jpeg2000::Image::new(data, &settings) {
        Ok(image) => image,
        Err(_) => {
            repaired = without_refused_enumerated_colour(data)?;
            hayro_jpeg2000::Image::new(&repaired, &settings).ok()?
        }
    };

    let width = image.width();
    let height = image.height();
    // PdfCraft patch: (#3179) a resolved palette gives colours, decoded to 8 bits like every
    // sample; the codestream's precision is that of the indices (1 bit, say), and rescaling the
    // colours to it kept only 0 or 255 per channel (review on #3184).
    let palette_resolved = settings.resolve_palette_indices && has_palette(data);
    let bpc = params
        .bpc
        .unwrap_or(if palette_resolved { 8 } else { image.original_bit_depth() });
    let cs = match image.color_space() {
        ColorSpace::Gray => ImageColorSpace::Gray,
        ColorSpace::RGB => ImageColorSpace::Rgb,
        ColorSpace::CMYK => ImageColorSpace::Cmyk,
        ColorSpace::Unknown { num_channels } => ImageColorSpace::Unknown(*num_channels),
        ColorSpace::Icc {
            num_channels: num_components,
            ..
        } => match num_components {
            1 => ImageColorSpace::Gray,
            3 => ImageColorSpace::Rgb,
            4 => ImageColorSpace::Cmyk,
            _ => return None,
        },
    };
    let has_alpha = image.has_alpha();
    let bitmap = image.decode().ok()?;

    let (mut data, mut alpha) = if !has_alpha {
        (bitmap, None)
    } else {
        // Extract the alpha channel.
        let total_channels = cs.num_components() + 1;
        let mut color_channels = Vec::with_capacity(
            (bitmap.len() / total_channels as usize) * cs.num_components() as usize,
        );
        let mut alpha_channel = Vec::with_capacity(bitmap.len() / total_channels as usize);

        for sample in bitmap.chunks_exact(total_channels as usize) {
            let (alpha, color) = sample.split_last()?;
            alpha_channel.push(*alpha);
            color_channels.extend_from_slice(color);
        }

        (color_channels, Some(alpha_channel))
    };

    // The decoded image is always 8-bit, so if necessary we have to rescale
    // ourselves.
    if bpc != 8 {
        data = scale(&data, bpc, cs.num_components(), width, height)?;
        alpha = alpha.and_then(|alpha| scale(&alpha, bpc, cs.num_components(), width, height));
    }

    Some(FilterResult {
        data: Cow::Owned(data),
        image_data: Some(ImageData {
            alpha,
            color_space: Some(cs),
            bits_per_component: bpc,
            width,
            height,
        }),
    })
}

/// PdfCraft patch: (#3179) whether JP2 file `data` has a palette box (`pclr`) in its header box
/// (`jp2h`, ISO 15444-1 I.5.3.4). A raw codestream has none.
fn has_palette(data: &[u8]) -> bool {
    boxes(data)
        .find(|&(kind, _, _)| kind == *b"jp2h")
        .and_then(|(_, start, end)| data.get(start..end))
        .is_some_and(|header| boxes(header).any(|(kind, _, _)| kind == *b"pclr"))
}

/// The enumerated colour spaces (`EnumCS`, ISO 15444-1 I.5.3.3 and 15444-2 M.11.7.2) the decoder
/// handles: CMYK, CIELab, sRGB, greyscale, sYCC, e-sRGB and ROMM-RGB.
const DECODED_ENUMERATED: [u32; 7] = [12, 14, 16, 17, 18, 20, 21];

/// PdfCraft patch (21): a copy of JP2 file `data` whose colour specification boxes (`colr`, in
/// the `jp2h` header box) that name an enumerated colour space outside [`DECODED_ENUMERATED`]
/// name the space the decoder assumes without one instead: greyscale, sRGB or CMYK for 1, 3 or
/// 4 components (`ihdr`). `None` when there is nothing to change or the boxes can't be read.
pub(crate) fn without_refused_enumerated_colour(data: &[u8]) -> Option<Vec<u8>> {
    let header = boxes(data).find(|&(kind, _, _)| kind == *b"jp2h")?;
    let (_, start, end) = header;
    let inner = data.get(start..end)?;
    let (_, ih_start, _) = boxes(inner).find(|&(kind, _, _)| kind == *b"ihdr")?;
    // ihdr: height (4), width (4), number of components (2).
    let components = u16::from_be_bytes(inner.get(ih_start + 8..ih_start + 10)?.try_into().ok()?);
    let replacement: u32 = match components {
        1 => 17,
        3 => 16,
        4 => 12,
        _ => return None,
    };
    // Where each refused enum is, found before anything is copied: a stream the decoder refused
    // for another reason isn't copied for nothing.
    let mut refused = Vec::new();
    for (kind, body, body_end) in boxes(inner) {
        if kind != *b"colr" {
            continue;
        }
        // colr: METH (1), PREC (1), APPROX (1), then EnumCS (4) when METH is 1.
        let colr = inner.get(body..body_end)?;
        if colr.first() != Some(&1) {
            continue;
        }
        let enumerated = u32::from_be_bytes(colr.get(3..7)?.try_into().ok()?);
        if DECODED_ENUMERATED.contains(&enumerated) {
            continue;
        }
        refused.push(start.checked_add(body)?.checked_add(3)?);
    }
    if refused.is_empty() {
        return None;
    }
    let mut out = data.to_vec();
    for at in refused {
        out.get_mut(at..at.checked_add(4)?)?.copy_from_slice(&replacement.to_be_bytes());
    }
    Some(out)
}

/// The boxes in `data` (ISO 15444-1 I.4): each one's type and the start and end of its contents.
/// Stops at the first box whose length doesn't fit.
fn boxes(data: &[u8]) -> impl Iterator<Item = ([u8; 4], usize, usize)> + '_ {
    let mut at = 0usize;
    core::iter::from_fn(move || {
        let length = u32::from_be_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?);
        let kind: [u8; 4] = data.get(at + 4..at.checked_add(8)?)?.try_into().ok()?;
        let (header, size) = match length {
            // The rest of the data.
            0 => (8, data.len().checked_sub(at)?),
            // An 8-byte length follows the type.
            1 => {
                let large = u64::from_be_bytes(data.get(at + 8..at.checked_add(16)?)?.try_into().ok()?);
                (16, usize::try_from(large).ok()?)
            }
            n => (8, usize::try_from(n).ok()?),
        };
        let end = at.checked_add(size)?;
        if size < header || end > data.len() {
            return None;
        }
        let item = (kind, at + header, end);
        at = end;
        Some(item)
    })
}

fn scale(
    data: &[u8],
    bit_per_component: u8,
    num_components: u8,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    if bit_per_component == 0
        || bit_per_component > 32
        || num_components == 0
        || width == 0
        || height == 0
    {
        return None;
    }

    // PdfCraft patch: normalize all supported bit widths without an overflowing
    // shift or floating-point endpoint rounding, including no_std builds.
    let max_sample = u64::from(bit_mask(bit_per_component));

    let bits_per_row = (width as usize)
        .checked_mul(num_components as usize)?
        .checked_mul(bit_per_component as usize)?;
    let input_len = bits_per_row.div_ceil(8).checked_mul(height as usize)?;
    let mut input = vec![0; input_len];
    let mut writer = BitWriter::new(&mut input, bit_per_component)?;
    let components_per_row = (num_components as usize).checked_mul(width as usize)?;

    for bytes in data.chunks_exact(components_per_row) {
        for byte in bytes {
            // The numerator is at most 255 * u32::MAX + 127, which fits in
            // u64. The rounded quotient is <= max_sample, so the cast is exact.
            let scaled = ((u64::from(*byte) * max_sample + 127) / 255) as u32;
            writer.write(scaled)?;
        }

        writer.align();
    }

    let final_pos = writer.cur_pos();
    input.truncate(final_pos);

    Some(input)
}

#[cfg(test)]
mod tests {
    use super::{decode, scale, without_refused_enumerated_colour};
    use crate::bit_reader::{BitReader, bit_mask};
    use crate::object::stream::{ImageColorSpace, ImageDecodeParams};
    use alloc::vec::Vec;

    #[test]
    fn jpx_scale_accepts_32_bit_samples() {
        assert_eq!(
            scale(&[0, 255], 32, 1, 2, 1).unwrap(),
            [0, 0, 0, 0, 255, 255, 255, 255]
        );
    }

    #[test]
    fn jpx_scale_keeps_supported_bit_depths() {
        for bpc in 1_u8..=32 {
            let bytes = scale(&[0, 255], bpc, 1, 2, 1).unwrap();
            assert_eq!(bytes.len(), (2 * usize::from(bpc)).div_ceil(8));
            let mut reader = BitReader::new(&bytes);
            assert_eq!(reader.read(bpc), Some(0));
            assert_eq!(reader.read(bpc), Some(bit_mask(bpc)));
        }
    }

    #[test]
    fn jpx_scale_rejects_out_of_range_bit_depths() {
        for bpc in [0, 33, 255] {
            assert!(scale(&[255], bpc, 1, 1, 1).is_none());
        }
    }

    #[test]
    fn jpx_scale_rejects_zero_dimensions_and_components() {
        for (components, width, height) in [(0, 1, 1), (1, 0, 1), (1, 1, 0)] {
            assert!(scale(&[255], 32, components, width, height).is_none());
        }
    }

    #[test]
    fn jpx_scale_preserves_one_eight_and_sixteen_bit_samples() {
        let samples: Vec<u8> = (0..=255).collect();
        for bpc in [1, 8, 16] {
            let bytes = scale(&samples, bpc, 1, 256, 1).unwrap();
            let mut reader = BitReader::new(&bytes);
            for byte in &samples {
                let expected = match bpc {
                    1 => u32::from(*byte >= 128),
                    8 => u32::from(*byte),
                    16 => u32::from(*byte) * 257,
                    _ => unreachable!(),
                };
                assert_eq!(reader.read(bpc), Some(expected), "{bpc}-bit sample {byte}");
            }
        }
    }

    /// A JP2 header (`jp2h`) holding `ihdr` for `components` and the given `colr` bodies, with
    /// a codestream box after it.
    fn jp2(components: u16, colrs: &[&[u8]]) -> Vec<u8> {
        let jp2_box = |kind: &[u8], body: &[u8]| {
            let mut b = ((8 + body.len()) as u32).to_be_bytes().to_vec();
            b.extend_from_slice(kind);
            b.extend_from_slice(body);
            b
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1]);
        ihdr.extend_from_slice(&components.to_be_bytes());
        ihdr.extend_from_slice(&[7, 7, 0, 0]);
        let mut header = jp2_box(b"ihdr", &ihdr);
        for c in colrs {
            header.extend(jp2_box(b"colr", c));
        }
        let mut out = jp2_box(b"ftyp", b"jp2 \0\0\0\0jp2 ");
        out.extend(jp2_box(b"jp2h", &header));
        out.extend(jp2_box(b"jp2c", &[0xff, 0x4f]));
        out
    }

    fn enumerated(e: u32) -> Vec<u8> {
        let mut c = alloc::vec![1, 0, 0];
        c.extend_from_slice(&e.to_be_bytes());
        c
    }

    /// The enumerated colour spaces found in the copy's `colr` boxes, in order.
    fn enums_in(data: &[u8]) -> Vec<u32> {
        data.windows(4)
            .enumerate()
            .filter(|(_, w)| *w == b"colr")
            .filter(|(i, _)| data.get(i + 4) == Some(&1))
            .map(|(i, _)| u32::from_be_bytes(data[i + 7..i + 11].try_into().unwrap()))
            .collect()
    }

    #[test]
    fn refused_enumerated_colour_spaces_become_the_component_counts_default() {
        // CIEJab (19) and other refused spaces, by component count.
        for (components, refused, expected) in [(3, 19, 16), (3, 22, 16), (1, 15, 17), (4, 13, 12), (3, 1, 16)] {
            let data = jp2(components, &[&enumerated(refused)]);
            let out = without_refused_enumerated_colour(&data).unwrap();
            assert_eq!(enums_in(&out), [expected], "{refused} with {components} component(s)");
            assert_eq!(out.len(), data.len(), "only the enum changes");
        }
        // Several colour specifications: each refused one, and only those.
        let out = without_refused_enumerated_colour(&jp2(3, &[&enumerated(19), &enumerated(14), &enumerated(24)])).unwrap();
        assert_eq!(enums_in(&out), [16, 14, 16]);
        // An 8-byte box length (LBox 1) is read too.
        let mut long = jp2(3, &[&enumerated(19)]);
        let at = long.windows(4).position(|w| w == b"jp2h").unwrap() - 4;
        let len = u32::from_be_bytes(long[at..at + 4].try_into().unwrap()) as u64 + 8;
        long.splice(at..at + 4, [0, 0, 0, 1]);
        long.splice(at + 8..at + 8, (len).to_be_bytes());
        assert_eq!(enums_in(&without_refused_enumerated_colour(&long).unwrap()), [16]);
    }

    #[test]
    fn what_the_decoder_handles_or_cant_be_read_is_left_alone() {
        for known in [12, 14, 16, 17, 18, 20, 21] {
            assert_eq!(without_refused_enumerated_colour(&jp2(3, &[&enumerated(known)])), None, "{known}");
        }
        // An ICC profile (METH 2), a component count with no default, no header at all.
        // (Its bytes where an enum would be read say 19: only METH 1 is an enum.)
        assert_eq!(without_refused_enumerated_colour(&jp2(3, &[&[2, 0, 0, 0, 0, 0, 19, 0, 0]])), None);
        assert_eq!(without_refused_enumerated_colour(&jp2(2, &[&enumerated(19)])), None);
        assert_eq!(without_refused_enumerated_colour(&[0xff, 0x4f, 0xff, 0x51]), None);
        // Damaged lengths: none of these may panic, and none is changed.
        let good = jp2(3, &[&enumerated(19)]);
        for cut in 0..good.len() {
            let _ = without_refused_enumerated_colour(&good[..cut]);
        }
        for bad in [0x0000_0004_u32, 0x7fff_ffff, 0xffff_ffff] {
            let mut data = good.clone();
            let at = data.windows(4).position(|w| w == b"colr").unwrap() - 4;
            data[at..at + 4].copy_from_slice(&bad.to_be_bytes());
            assert_eq!(without_refused_enumerated_colour(&data), None, "{bad:#x}");
        }
    }

    /// A 4x2 JP2 written for this test: one 8-bit component whose samples (0, 1) index a
    /// two-entry sRGB palette (`pclr` + `cmap`): red, then blue. Rows: 0 1 0 1 / 1 0 1 0.
    const PALETTE_JP2: &[u8] = &[
        0x00, 0x00, 0x00, 0x0c, 0x6a, 0x50, 0x20, 0x20, 0x0d, 0x0a, 0x87, 0x0a, 0x00, 0x00, 0x00,
        0x14, 0x66, 0x74, 0x79, 0x70, 0x6a, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6a, 0x70,
        0x32, 0x20, 0x00, 0x00, 0x00, 0x55, 0x6a, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69,
        0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01, 0x07, 0x07,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x63, 0x6f, 0x6c, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x10, 0x00, 0x00, 0x00, 0x14, 0x70, 0x63, 0x6c, 0x72, 0x00, 0x02, 0x03, 0x07, 0x07,
        0x07, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x14, 0x63, 0x6d, 0x61, 0x70,
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00,
        0x8c, 0x6a, 0x70, 0x32, 0x63, 0xff, 0x4f, 0xff, 0x51, 0x00, 0x29, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x04, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x07, 0x01, 0x01, 0xff, 0x52, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01,
        0x04, 0x04, 0x00, 0x01, 0xff, 0x5c, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xff, 0x64,
        0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20,
        0x4f, 0x70, 0x65, 0x6e, 0x4a, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f,
        0x6e, 0x20, 0x32, 0x2e, 0x35, 0x2e, 0x34, 0xff, 0x90, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x17, 0x00, 0x01, 0xff, 0x93, 0xcf, 0xb4, 0x08, 0x08, 0x67, 0x90, 0x07, 0x41, 0x08,
        0xff, 0xd9,
    ];

    /// #3179: with no `/ColorSpace` in the image dictionary, the JP2 palette gives the colours. The
    /// raw indices (0 and 1) used to come back as a gray image, drawn as noise.
    #[test]
    fn jpx_palette_is_applied_without_a_color_space() {
        let params = ImageDecodeParams {
            width: 4,
            height: 2,
            ..ImageDecodeParams::default()
        };
        let result = decode(PALETTE_JP2, &params).unwrap();
        let image = result.image_data.unwrap();
        assert!(matches!(image.color_space, Some(ImageColorSpace::Rgb)));
        let red = [255, 0, 0];
        let blue = [0, 0, 255];
        let expected: Vec<u8> = [red, blue, red, blue, blue, red, blue, red].concat();
        assert_eq!(&*result.data, &expected[..]);
    }

    /// With an `/Indexed` colour space in the dictionary, the PDF supplies the palette, so the raw
    /// indices are what the image holds.
    #[test]
    fn jpx_palette_indices_stay_raw_for_an_indexed_color_space() {
        let params = ImageDecodeParams {
            is_indexed: true,
            num_components: Some(1),
            width: 4,
            height: 2,
            ..ImageDecodeParams::default()
        };
        let result = decode(PALETTE_JP2, &params).unwrap();
        assert_eq!(&*result.data, &[0, 1, 0, 1, 1, 0, 1, 0][..]);
    }

    /// A 1x1, 1-bit codestream (one empty packet, so its sample is the midpoint 1) as a JP2 file
    /// whose header holds `extra` boxes after `ihdr` and `colr`.
    fn one_bit_jp2(extra: &[u8]) -> Vec<u8> {
        let jp2_box = |kind: &[u8], body: &[u8]| {
            let mut b = ((8 + body.len()) as u32).to_be_bytes().to_vec();
            b.extend_from_slice(kind);
            b.extend_from_slice(body);
            b
        };
        let mut codestream = alloc::vec![0xff, 0x4f, 0xff, 0x51, 0, 41, 0, 0];
        for value in [1_u32, 1, 0, 0, 1, 1, 0, 0] {
            codestream.extend_from_slice(&value.to_be_bytes());
        }
        codestream.extend_from_slice(&[0, 1, 0, 1, 1]);
        codestream.extend_from_slice(&[0xff, 0x52, 0, 12, 0, 0, 0, 1, 0, 0, 4, 4, 0, 1]);
        codestream.extend_from_slice(&[0xff, 0x5c, 0, 4, 0x40, 0x40]);
        codestream.extend_from_slice(&[0xff, 0x90, 0, 10, 0, 0, 0, 0, 0, 15, 0, 1, 0xff, 0x93, 0]);
        codestream.extend_from_slice(&[0xff, 0xd9]);
        let mut header = jp2_box(b"ihdr", &[0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 0, 7, 0, 0]);
        header.extend(jp2_box(b"colr", &[1, 0, 0, 0, 0, 0, 16]));
        header.extend_from_slice(extra);
        let mut out = alloc::vec![0, 0, 0, 12, b'j', b'P', b' ', b' ', 0x0d, 0x0a, 0x87, 0x0a];
        out.extend(jp2_box(b"ftyp", b"jp2 \0\0\0\0jp2 "));
        out.extend(jp2_box(b"jp2h", &header));
        out.extend(jp2_box(b"jp2c", &codestream));
        out
    }

    /// Review on #3184: resolved palette colours are 8-bit samples whatever the indices'
    /// precision; an image without a palette keeps its own (1 bit here).
    #[test]
    fn palette_colours_are_eight_bit_and_other_samples_keep_their_precision() {
        let params = ImageDecodeParams { width: 1, height: 1, ..ImageDecodeParams::default() };
        let bpc = |data: &[u8]| decode(data, &params).and_then(|r| r.image_data).map(|d| d.bits_per_component);
        let mut palette = Vec::new();
        for (kind, body) in [(&b"pclr"[..], &[0, 2, 3, 7, 7, 7, 255, 0, 0, 64, 128, 192][..]), (b"cmap", &[0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 2])] {
            palette.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
            palette.extend_from_slice(kind);
            palette.extend_from_slice(body);
        }
        assert_eq!(bpc(&one_bit_jp2(&palette)), Some(8));
        assert_eq!(bpc(&one_bit_jp2(&[])), Some(1));
    }
}
