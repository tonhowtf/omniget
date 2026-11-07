//! Sample helpers and RGBA8 convenience extraction.
//!
//! These are conveniences for previews and tests; they perform no color
//! management. 16-bit samples are scaled with rounding, 32-bit float samples
//! are clamped to 0..=1 (no tone mapping or gamma), CMYK is converted
//! naively (`r = c' * k' / 255` using the stored, inverted values).

use crate::error::{PsdError, Result};
use crate::file::PsdFile;
use crate::header::{ColorMode, Header, Version};
use crate::layer::{CHANNEL_REAL_USER_MASK, CHANNEL_TRANSPARENCY, CHANNEL_USER_MASK, LayerRecord, Rect};
use std::borrow::Cow;

/// Interprets planar big-endian bytes as u16 samples.
pub fn samples_u16(bytes: &[u8]) -> Vec<u16> {
    bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect()
}

/// Interprets planar big-endian bytes as f32 samples.
pub fn samples_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.as_chunks::<4>().0.iter().map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Encodes u16 samples as big-endian bytes.
pub fn u16_to_bytes(samples: &[u16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_be_bytes()).collect()
}

/// Encodes f32 samples as big-endian bytes.
pub fn f32_to_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_be_bytes()).collect()
}

/// Allocates `len` bytes, reporting a machine without that much memory instead of aborting.
fn try_buffer(len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| PsdError::OutOfMemory("not enough memory for the converted plane"))?;
    Ok(out)
}

/// Expands a 1-bit plane (MSB first, rows padded to bytes) to one byte per
/// pixel (0 or 1). Errors with [`PsdError::LimitExceeded`] when the plane
/// size overflows or cannot be allocated.
pub fn unpack_bits(bytes: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    unpack_bits_with(bytes, width, height, |bit| bit)
}

/// Shared 1-bit expansion; `map` turns each bit (0 or 1) into the output byte.
fn unpack_bits_with(bytes: &[u8], width: usize, height: usize, map: impl Fn(u8) -> u8) -> Result<Vec<u8>> {
    let len = width.checked_mul(height).ok_or(PsdError::LimitExceeded("plane size overflow"))?;
    let rb = width.div_ceil(8);
    let mut out = try_buffer(len)?;
    for y in 0..height {
        for x in 0..width {
            let b = y.checked_mul(rb).and_then(|row| bytes.get(row + x / 8)).copied().unwrap_or(0);
            out.push(map((b >> (7 - (x % 8))) & 1));
        }
    }
    Ok(out)
}

/// Converts one plane of samples at `depth` to 8-bit.
pub fn plane_to_u8(bytes: &[u8], depth: u16, width: usize, height: usize) -> Result<Vec<u8>> {
    Ok(match depth {
        1 => unpack_bits_with(bytes, width, height, |b| if b == 1 { 0 } else { 255 })?,
        8 => {
            let mut out = try_buffer(bytes.len())?;
            out.extend_from_slice(bytes);
            out
        }
        16 => {
            let chunks = bytes.as_chunks::<2>().0;
            let mut out = try_buffer(chunks.len())?;
            out.extend(chunks.iter().map(|b| ((u32::from(u16::from_be_bytes(*b)) * 255 + 32767) / 65535) as u8));
            out
        }
        32 => {
            let chunks = bytes.as_chunks::<4>().0;
            let mut out = try_buffer(chunks.len())?;
            out.extend(chunks.iter().map(|b| {
                let v = f32::from_be_bytes(*b);
                if v.is_nan() { 0 } else { (v.clamp(0.0, 1.0) * 255.0).round() as u8 }
            }));
            out
        }
        d => return Err(PsdError::Unsupported(format!("depth {d}"))),
    })
}

/// An 8-bit RGBA image positioned in document coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// Left edge in document coordinates.
    pub left: i32,
    /// Top edge in document coordinates.
    pub top: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Interleaved RGBA, row-major.
    pub data: Vec<u8>,
}

/// An 8-bit single-channel image (e.g. a layer mask).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrayImage {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Samples.
    pub data: Vec<u8>,
    /// Value outside the rectangle (masks only).
    pub default_value: u8,
}

fn interleave(mode: ColorMode, color: &[Cow<'_, [u8]>], alpha: Option<&[u8]>, palette: &[u8], n: usize) -> Result<Vec<u8>> {
    if color.len() != color_channel_count(mode)? || color.iter().any(|p| p.len() != n) || alpha.is_some_and(|p| p.len() != n) {
        return Err(PsdError::invalid("channel size mismatch"));
    }
    let len = n
        .checked_mul(4)
        .filter(|&len| len as u64 <= crate::compression::MAX_DECODED_BYTES)
        .ok_or(PsdError::LimitExceeded("RGBA output exceeds MAX_DECODED_BYTES"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| PsdError::OutOfMemory("not enough memory for RGBA output"))?;
    out.resize(len, 255);
    if mode == ColorMode::Rgb {
        let [r, g, b] = color else { return Err(PsdError::invalid("RGB requires three channels")) };
        let rgb = r.iter().zip(g.iter()).zip(b.iter());
        match alpha {
            Some(alpha) => {
                for (rgba, (((r, g), b), a)) in out.as_chunks_mut::<4>().0.iter_mut().zip(rgb.zip(alpha)) {
                    *rgba = [*r, *g, *b, *a];
                }
            }
            None => {
                for (rgba, ((r, g), b)) in out.as_chunks_mut::<4>().0.iter_mut().zip(rgb) {
                    *rgba = [*r, *g, *b, 255];
                }
            }
        }
        return Ok(out);
    }
    for i in 0..n {
        let (r, g, b) = match mode {
            ColorMode::Rgb => (color[0][i], color[1][i], color[2][i]),
            ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => (color[0][i], color[0][i], color[0][i]),
            ColorMode::Indexed => {
                let idx = usize::from(color[0][i]);
                let p = |k: usize| palette.get(k * 256 + idx).copied().unwrap_or(0);
                (p(0), p(1), p(2))
            }
            ColorMode::Cmyk => {
                let k = u32::from(color[3][i]);
                let f = |c: u8| ((u32::from(c) * k + 127) / 255) as u8;
                (f(color[0][i]), f(color[1][i]), f(color[2][i]))
            }
            m => return Err(PsdError::Unsupported(format!("rgba8 for color mode {m:?}"))),
        };
        out[i * 4] = r;
        out[i * 4 + 1] = g;
        out[i * 4 + 2] = b;
        out[i * 4 + 3] = alpha.map_or(255, |a| a[i]);
    }
    Ok(out)
}

fn color_channel_count(mode: ColorMode) -> Result<usize> {
    match mode {
        ColorMode::Rgb => Ok(3),
        ColorMode::Cmyk => Ok(4),
        ColorMode::Grayscale | ColorMode::Indexed | ColorMode::Bitmap | ColorMode::Duotone => Ok(1),
        m => Err(PsdError::Unsupported(format!("rgba8 for color mode {m:?}"))),
    }
}

/// One 8-bit sample matted against white at alpha `a`, back to straight colour (rounded, at most
/// 255). At `a == 0` there is no colour to recover and the sample is kept.
fn unmatte_u8(c: u8, a: u8) -> u8 {
    let a = u32::from(a);
    ((u32::from(c) + a).saturating_sub(255) * 255 + a / 2).checked_div(a).map_or(c, |straight| straight.min(255) as u8)
}

/// [`unmatte_u8`] of every sample, one row per alpha: `table[a][c]`. Built per call (64 KiB, a
/// division per entry) rather than per pixel, and not as a static, which would add 64 KiB to
/// every binary that links this crate.
fn unmatte_u8_table() -> Vec<[u8; 256]> {
    (0..=255).map(|a| std::array::from_fn(|c| unmatte_u8(c as u8, a))).collect()
}

fn unmatte_to_u8(color: &[u8], alpha: &[u8], depth: u16) -> Result<Vec<u8>> {
    let convert = |c: f32, a: f32| {
        let straight = if a > 0.0 { (c + a - 1.0) / a } else { c };
        (straight.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    Ok(match depth {
        16 => color
            .as_chunks::<2>()
            .0
            .iter()
            .zip(alpha.as_chunks::<2>().0)
            .map(|(c, a)| convert(f32::from(u16::from_be_bytes(*c)) / 65535.0, f32::from(u16::from_be_bytes(*a)) / 65535.0))
            .collect(),
        32 => color.as_chunks::<4>().0.iter().zip(alpha.as_chunks::<4>().0).map(|(c, a)| convert(f32::from_be_bytes(*c), f32::from_be_bytes(*a))).collect(),
        d => return Err(PsdError::Unsupported(format!("RGB composite depth {d}"))),
    })
}

/// A borrowed view of a layer with the file context needed to decode it.
#[derive(Debug, Clone, Copy)]
pub struct Layer<'a> {
    /// Index in [`PsdFile::layers`].
    pub index: usize,
    /// The record.
    pub record: &'a LayerRecord,
    header: &'a Header,
}

impl<'a> Layer<'a> {
    /// Name, preferring `luni`.
    pub fn name(&self) -> String {
        self.record.name()
    }

    /// PSD/PSB version of the containing file.
    pub fn version(&self) -> Version {
        self.header.version
    }

    /// Decodes a channel (planar, big-endian samples).
    pub fn channel_bytes(&self, id: i16) -> Result<Vec<u8>> {
        self.record.decode_channel(id, self.header.depth, self.header.version)
    }

    /// Layer pixels as RGBA8 (transparency channel as alpha; the user mask is
    /// *not* applied, see [`Self::user_mask`]). Supports RGB, grayscale and
    /// CMYK at 8/16/32 bits.
    pub fn rgba8(&self) -> Result<RgbaImage> {
        let rect = self.record.rect;
        let (w, h) = rect.size()?;
        let n = w.checked_mul(h).ok_or(PsdError::LimitExceeded("layer size overflow"))?;
        let depth = self.header.depth;
        let mode = self.header.color_mode;
        let cc = color_channel_count(mode)?;
        // The planes (one per colour channel, filled in for channels the layer lacks, and alpha)
        // coexist with the RGBA output, so the cap covers all of them, not the output alone.
        if n.checked_mul(cc + 2 + 4).is_none_or(|len| len as u64 > crate::compression::MAX_DECODED_BYTES) {
            return Err(PsdError::LimitExceeded("RGBA output exceeds MAX_DECODED_BYTES"));
        }
        let mut color = Vec::with_capacity(cc);
        for id in 0..cc as i16 {
            let plane = if self.record.channel(id).is_some() {
                let bytes = self.channel_bytes(id)?;
                if depth == 8 { bytes } else { plane_to_u8(&bytes, depth, w, h)? }
            } else {
                vec![if mode == ColorMode::Cmyk { 255 } else { 0 }; n]
            };
            if plane.len() != n {
                return Err(PsdError::invalid("channel size mismatch"));
            }
            color.push(Cow::Owned(plane));
        }
        let alpha = match self.record.channel(CHANNEL_TRANSPARENCY) {
            Some(_) => {
                let bytes = self.channel_bytes(CHANNEL_TRANSPARENCY)?;
                Some(if depth == 8 { bytes } else { plane_to_u8(&bytes, depth, w, h)? })
            }
            None => None,
        };
        let data = interleave(mode, &color, alpha.as_deref(), &[], n)?;
        Ok(RgbaImage { left: rect.left, top: rect.top, width: w as u32, height: h as u32, data })
    }

    /// The user mask (channel -3 if present, else -2) as 8-bit gray, with its
    /// rectangle and default color. `None` if the layer has no mask channel.
    pub fn user_mask(&self) -> Option<Result<GrayImage>> {
        let id = if self.record.channel(CHANNEL_REAL_USER_MASK).is_some() {
            CHANNEL_REAL_USER_MASK
        } else if self.record.channel(CHANNEL_USER_MASK).is_some() {
            CHANNEL_USER_MASK
        } else {
            return None;
        };
        Some((|| {
            let rect: Rect = self.record.channel_rect(id);
            let (w, h) = rect.size()?;
            let data = plane_to_u8(&self.channel_bytes(id)?, self.header.depth, w, h)?;
            let default_value = match (self.record.layer_mask(), id) {
                (Some(m), CHANNEL_REAL_USER_MASK) => m.real.map_or(m.default_color, |r| r.background),
                (Some(m), _) => m.default_color,
                (None, _) => 0,
            };
            Ok(GrayImage { left: rect.left, top: rect.top, width: w as u32, height: h as u32, data, default_value })
        })())
    }
}

impl PsdFile {
    /// Borrowed view of layer `index`.
    pub fn layer(&self, index: usize) -> Option<Layer<'_>> {
        self.layers().get(index).map(|record| Layer { index, record, header: &self.header })
    }

    /// Iterates over layers in file order (bottom-most first).
    pub fn iter_layers(&self) -> impl Iterator<Item = Layer<'_>> {
        self.layers().iter().enumerate().map(|(index, record)| Layer { index, record, header: &self.header })
    }

    /// The merged composite as RGBA8. Supports RGB, grayscale, CMYK, indexed,
    /// duotone (as gray) and bitmap at all depths. Alpha comes from the first
    /// extra channel when [`PsdFile::merged_has_alpha`] is true. Removes the
    /// white matte from RGB composites before reducing precision.
    pub fn composite_rgba8(&self) -> Result<RgbaImage> {
        let h = &self.header;
        h.validate()?;
        let (w, hh) = (h.width as usize, h.height as usize);
        let n = w.checked_mul(hh).ok_or(PsdError::LimitExceeded("composite size overflow"))?;
        // Same budget as `Layer::rgba8`: refuse before decoding or converting any plane.
        if n.checked_mul(4).is_none_or(|len| len as u64 > crate::compression::MAX_DECODED_BYTES) {
            return Err(PsdError::LimitExceeded("RGBA output exceeds MAX_DECODED_BYTES"));
        }
        let cc = color_channel_count(h.color_mode)?;
        if usize::from(h.channels) < cc {
            return Err(PsdError::invalid("fewer channels than the color mode requires"));
        }
        let all = self.decode_merged()?;
        let plane = h.row_bytes().checked_mul(hh).ok_or(PsdError::LimitExceeded("plane size overflow"))?;
        let alpha_bytes = if self.merged_has_alpha() {
            Some(all.get(cc * plane..(cc + 1) * plane).ok_or_else(|| PsdError::invalid("alpha channel size mismatch"))?)
        } else {
            None
        };
        let get = |i: usize| -> Result<Cow<'_, [u8]>> {
            let bytes = all.get(i * plane..(i + 1) * plane).ok_or_else(|| PsdError::invalid("channel size mismatch"))?;
            if h.depth == 8 { Ok(Cow::Borrowed(bytes)) } else { plane_to_u8(bytes, h.depth, w, hh).map(Cow::Owned) }
        };
        let mut color = Vec::with_capacity(cc);
        for i in 0..cc {
            color.push(
                if h.color_mode == ColorMode::Rgb
                    && h.depth != 8
                    && let Some(alpha) = alpha_bytes
                {
                    let bytes = all.get(i * plane..(i + 1) * plane).ok_or_else(|| PsdError::invalid("channel size mismatch"))?;
                    Cow::Owned(unmatte_to_u8(bytes, alpha, h.depth)?)
                } else {
                    get(i)?
                },
            );
        }
        let alpha = if self.merged_has_alpha() { Some(get(cc)?) } else { None };
        let mut data = interleave(h.color_mode, &color, alpha.as_deref(), &self.color_mode_data, n)?;
        if h.color_mode == ColorMode::Rgb && h.depth == 8 && alpha.is_some() {
            let table = unmatte_u8_table();
            for pixel in data.as_chunks_mut::<4>().0 {
                let row = &table[usize::from(pixel[3])];
                for c in pixel.iter_mut().take(3) {
                    *c = row[usize::from(*c)];
                }
            }
        }
        Ok(RgbaImage { left: 0, top: 0, width: h.width, height: h.height, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_composite_removes_white_matte_without_changing_native_data() {
        let mut file = crate::testgen::merged_only(Version::Psd, ColorMode::Rgb, 32, crate::Compression::Raw, 1, 1);
        file.header.channels = 4;
        let native = f32_to_bytes(&[0.75, 0.875, 1.0, 0.25]);
        file.image_data = crate::ImageData::encode(crate::Compression::Raw, &native, &file.header).unwrap();
        assert_eq!(file.composite_rgba8().unwrap().data, [0, 128, 255, 64]);
        assert_eq!(file.decode_merged().unwrap(), native);
    }

    /// #5694: the 8-bit RGB composite unmattes through a table; every (sample, alpha) pair comes
    /// out exactly as the per-pixel division it replaced (kept here as the reference).
    #[test]
    fn rgba8_composite_unmattes_every_sample_and_alpha_as_the_division_did() {
        let mut file = crate::testgen::merged_only(Version::Psd, ColorMode::Rgb, 8, crate::Compression::Raw, 256, 256);
        file.header.channels = 4;
        // Pixel (x, y): red x, green 255 - x, blue x ^ y, alpha y.
        let (x, y) = (|i: usize| (i % 256) as u8, |i: usize| (i / 256) as u8);
        let plane = |f: &dyn Fn(usize) -> u8| (0..256 * 256).map(f).collect::<Vec<u8>>();
        let planes = [plane(&|i| x(i)), plane(&|i| 255 - x(i)), plane(&|i| x(i) ^ y(i)), plane(&|i| y(i))];
        file.image_data = crate::ImageData::encode(crate::Compression::Raw, &planes.concat(), &file.header).unwrap();
        let mut expected: Vec<u8> = (0..256 * 256).flat_map(|i| planes.each_ref().map(|p| p[i])).collect();
        for pixel in expected.as_chunks_mut::<4>().0 {
            let a = u32::from(pixel[3]);
            for c in pixel.iter_mut().take(3) {
                if let Some(straight) = ((u32::from(*c) + a).saturating_sub(255) * 255 + a / 2).checked_div(a) {
                    *c = straight.min(255) as u8;
                }
            }
        }
        assert_eq!(file.composite_rgba8().unwrap().data, expected);
    }

    #[test]
    fn oversized_layer_preview_rejects_missing_channels_before_allocating() {
        let mut file = crate::testgen::small(Version::Psb, crate::Compression::Raw);
        file.layers_mut().push(LayerRecord { rect: Rect::from_xywh(0, 0, 300000, 300000), ..Default::default() });
        assert!(matches!(file.iter_layers().last().unwrap().rgba8(), Err(PsdError::LimitExceeded(_))));
    }

    #[test]
    fn sample_conversions() {
        assert_eq!(samples_u16(&[0x12, 0x34, 0xff, 0xff]), vec![0x1234, 0xffff]);
        assert_eq!(u16_to_bytes(&[0x1234]), vec![0x12, 0x34]);
        let f = f32_to_bytes(&[0.5, -1.0]);
        assert_eq!(samples_f32(&f), vec![0.5, -1.0]);
    }

    #[test]
    fn unpack_bits_rows() {
        assert_eq!(unpack_bits(&[0b1010_0000, 0b1000_0000], 3, 2).unwrap(), vec![1, 0, 1, 1, 0, 0]);
    }

    #[test]
    fn plane_to_u8_depths() {
        assert_eq!(plane_to_u8(&[0xff, 0xff, 0, 0, 0x80, 0x00], 16, 3, 1).unwrap(), vec![255, 0, 128]);
        assert_eq!(plane_to_u8(&f32_to_bytes(&[0.0, 1.0, 2.0, -1.0, f32::NAN]), 32, 5, 1).unwrap(), vec![0, 255, 255, 0, 0]);
        assert_eq!(plane_to_u8(&[0b1000_0000], 1, 2, 1).unwrap(), vec![0, 255]);
        assert!(plane_to_u8(&[], 12, 0, 0).is_err());
    }

    /// A plane size that overflows or cannot be allocated is an error, not a panic or abort.
    #[test]
    fn plane_conversion_reports_absurd_sizes() {
        assert!(matches!(unpack_bits(&[0], usize::MAX / 2, 4), Err(PsdError::LimitExceeded(_))));
        assert!(matches!(plane_to_u8(&[0], 1, usize::MAX / 2, 4), Err(PsdError::LimitExceeded(_))));
        assert!(matches!(unpack_bits(&[0], 1 << 40, 1 << 20), Err(PsdError::LimitExceeded(_) | PsdError::OutOfMemory(_))));
    }

    /// A bitmap composite whose RGBA output is over the decode budget is refused up front.
    #[test]
    fn composite_rgba8_checks_the_output_budget_first() {
        let mut file = crate::testgen::small(Version::Psb, crate::Compression::Zip);
        file.header.color_mode = ColorMode::Bitmap;
        file.header.depth = 1;
        file.header.channels = 1;
        file.header.width = 60_000;
        file.header.height = 60_000;
        assert!(matches!(file.composite_rgba8(), Err(PsdError::LimitExceeded("RGBA output exceeds MAX_DECODED_BYTES"))));
    }

    #[test]
    fn rgb_interleave_checks_planes_and_preserves_alpha() {
        let color = [Cow::Borrowed(&[1, 2][..]), Cow::Borrowed(&[3, 4][..]), Cow::Borrowed(&[5, 6][..])];
        assert_eq!(interleave(ColorMode::Rgb, &color, Some(&[7, 8]), &[], 2).unwrap(), [1, 3, 5, 7, 2, 4, 6, 8]);
        assert_eq!(interleave(ColorMode::Rgb, &color, None, &[], 2).unwrap(), [1, 3, 5, 255, 2, 4, 6, 255]);
        assert!(interleave(ColorMode::Rgb, &color[..2], None, &[], 2).is_err());
        assert!(interleave(ColorMode::Rgb, &color, Some(&[7]), &[], 2).is_err());
    }
}
