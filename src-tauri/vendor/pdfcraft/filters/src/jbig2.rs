//! JBIG2 encoding (ITU-T T.88 | ISO/IEC 14492), for `JBIG2Decode` (ISO 32000-2 §7.4.7), lossless:
//! a generic region, template 0 with its nominal adaptive pixels, typical prediction on, MQ
//! arithmetic coding ([`crate::mq`]); or the image's marks as symbols ([`symbols`]), which a group
//! of images may share through `/JBIG2Globals`. Written from the standard's 6.2.5 (coding a generic
//! region), 7.2 (segment headers), 7.4.1, 7.4.6 and 7.4.8 (the segments) and D.3 (the embedded
//! organisation PDF uses). Decoding is in [`decode`].

mod decode;
mod symbols;

pub use decode::{MAX_JBIG2_PIXELS, decode_jbig2_within};

use crate::FilterError;
use crate::mq::{Context, MqEncoder};

/// The widest row, and the most rows, [`encode_jbig2_generic`] encodes: what `hayro-jbig2`, the
/// decoder behind `JBIG2Decode` here, takes on a side. JBIG2 allows 32-bit sizes, but past this
/// nothing here could read the stream, and the optimizer, which doesn't read back what it stores,
/// replaced such an image with one that draws nothing.
pub const MAX_JBIG2_COLUMNS: u32 = 65_535;

/// Template 0's pixels (Figure 3), as (x, y) offsets from the pixel being coded, each a bit of the
/// context in this order: the four on the current row, the five fixed ones on the row above, the
/// three on the row above that, then the adaptive pixels A1–A4 at their nominal places (Table 5).
const TEMPLATE: [(i32, i32); 16] = [
    (-1, 0),
    (-2, 0),
    (-3, 0),
    (-4, 0),
    (-2, -1),
    (-1, -1),
    (0, -1),
    (1, -1),
    (2, -1),
    (-1, -2),
    (0, -2),
    (1, -2),
    (3, -1),
    (-3, -1),
    (2, -2),
    (-2, -2),
];

/// The context that codes SLTP when template 0 is used (Figure 8): those pixel values, in
/// [`TEMPLATE`]'s order. Figure 8 reads, by row from x = -4: `0 1 0 1` on the current row,
/// `0 1 1 0 0 1 0` (x = -3 to 3) above it, `1 0 0 1 1` (x = -2 to 2) above that.
const SLTP: [u8; 16] = [1, 0, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1];

/// Columns of padding either side of a row, so every template pixel of a row's pixels is in it.
const PAD: usize = 4;

/// `rows` rows of `columns` 1-bit PDF samples as lossless JBIG2, the smaller of two codings:
/// [`encode_jbig2_generic`]'s, or one that stores each distinct mark of the image once, as a
/// symbol, and places its copies (see [`symbols`]). Either way the stream is in the embedded
/// organisation and decodes to the same bits without `/JBIG2Globals`.
///
/// Errors: as [`encode_jbig2_generic`].
pub fn encode_jbig2(data: &[u8], columns: u32, rows: u32) -> Result<Vec<u8>, FilterError> {
    let generic = encode_jbig2_generic(data, columns, rows)?;
    let samples = rows_to_encode(data, columns, rows).ok();
    Ok(match samples.and_then(|s| symbols::encode(s, columns, rows)) {
        Some(coded) if coded.len() < generic.len() => coded,
        _ => generic,
    })
}

/// `rows` rows of `columns` 1-bit PDF samples (most significant bit first, each row starting on
/// a new byte), as a JBIG2 stream in the embedded organisation: a page information segment and an
/// immediate lossless generic region segment, which `JBIG2Decode` decodes to the same bits
/// without `/JBIG2Globals`.
///
/// A 0 sample is coded as a JBIG2 1 (black), since `JBIG2Decode` gives black as 0, as a grey image's
/// samples have it. Bytes after the last row are ignored, as are the bits that pad a row to a whole
/// byte.
///
/// Errors: no rows or columns, more of either than [`MAX_JBIG2_COLUMNS`], or more samples than
/// [`MAX_JBIG2_PIXELS`], what the decoder takes ([`FilterError::Unsupported`]); fewer bytes than the
/// rows need ([`FilterError::Corrupt`]).
pub fn encode_jbig2_generic(data: &[u8], columns: u32, rows: u32) -> Result<Vec<u8>, FilterError> {
    let samples = rows_to_encode(data, columns, rows)?;
    let width = columns as usize;
    let stride = width.div_ceil(8);
    let mut contexts: Vec<Context> = vec![0; 1 << 16];
    let mut mq = MqEncoder::new();
    let height = samples.len() / stride;
    code_generic(&mut mq, &mut contexts, width, height, true, |y, row| {
        let packed = samples.get(y * stride..).unwrap_or_default();
        for (x, pixel) in row.iter_mut().enumerate() {
            let sample = packed.get(x / 8).is_some_and(|b| b & (0x80 >> (x % 8)) != 0);
            *pixel = u8::from(!sample);
        }
    });
    let region = mq.finish();
    let mut out = Vec::with_capacity(region.len() + 64);
    segment(&mut out, 0, 48, &[], false, &page_information(columns, rows));
    segment(&mut out, 1, 39, &[], false, &generic_region(columns, rows, 0, 0, &region));
    Ok(out)
}

/// The samples of `rows` rows of `columns` 1-bit samples in `data`, when the encoders here take
/// that size; the errors are [`encode_jbig2_generic`]'s.
fn rows_to_encode(data: &[u8], columns: u32, rows: u32) -> Result<&[u8], FilterError> {
    let too_large = columns > MAX_JBIG2_COLUMNS || rows > MAX_JBIG2_COLUMNS || u64::from(columns) * u64::from(rows) > MAX_JBIG2_PIXELS;
    if columns == 0 || rows == 0 || too_large {
        return Err(FilterError::Unsupported(format!("JBIG2 encoding of {columns} × {rows} samples")));
    }
    let stride = (columns as usize).div_ceil(8);
    usize::try_from(rows)
        .ok()
        .and_then(|r| r.checked_mul(stride))
        .and_then(|n| data.get(..n))
        .ok_or_else(|| FilterError::Corrupt { filter: "JBIG2Decode", detail: format!("{} bytes for {rows} rows of {columns} samples", data.len()) })
}

/// A group of images coded by [`encode_jbig2_shared`]: the data of the `/JBIG2Globals` stream
/// they share, and each image's own stream, in the order given, or `None` for one that shares
/// nothing with the others.
#[derive(Debug)]
pub struct SharedJbig2 {
    pub globals: Vec<u8>,
    pub images: Vec<Option<Vec<u8>>>,
}

/// Images, each `rows` rows of `columns` 1-bit PDF samples, as lossless JBIG2 sharing one symbol
/// dictionary: the marks found in more than one of them are stored once, in a symbol dictionary
/// segment not associated with a page, which is the `/JBIG2Globals` stream's data, and each image's
/// stream, in the embedded organisation, places them from there, with its other marks coded as
/// [`encode_jbig2`]'s symbol coding codes them (see [`symbols`]). With the globals, each decodes to
/// its own bits. An image that holds none of those marks, or that symbol coding doesn't take, has
/// no stream here; it is stored alone. `None` when the images share fewer than two marks.
///
/// What each image's stream saves is left for the caller to weigh against what the globals take,
/// which the images that use them share.
///
/// Errors: as [`encode_jbig2_generic`], for the first image it would refuse.
pub fn encode_jbig2_shared(images: &[(&[u8], u32, u32)]) -> Result<Option<SharedJbig2>, FilterError> {
    let images =
        images.iter().map(|(data, columns, rows)| Ok((rows_to_encode(data, *columns, *rows)?, *columns, *rows))).collect::<Result<Vec<_>, _>>()?;
    Ok(symbols::encode_shared(&images).map(|(globals, images)| SharedJbig2 { globals, images }))
}

/// A page information segment's data (7.4.8): the page's size, resolution unknown, eventually
/// lossless, default pixel 0 (white), OR combination, not striped.
fn page_information(columns: u32, rows: u32) -> Vec<u8> {
    let mut page = Vec::with_capacity(19);
    page.extend_from_slice(&columns.to_be_bytes());
    page.extend_from_slice(&rows.to_be_bytes());
    page.extend_from_slice(&[0; 8]);
    page.push(0x01);
    page.extend_from_slice(&[0, 0]);
    page
}

/// A region segment information field (7.4.1): a `width` × `height` region at (`x`, `y`) on the
/// page, combined with it by OR.
fn region_information(out: &mut Vec<u8>, width: u32, height: u32, x: u32, y: u32) {
    for v in [width, height, x, y] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.push(0);
}

/// An immediate lossless generic region segment's data (7.4.6): its region information, flags
/// (MMR 0, template 0, TPGDON 1), the AT pixels at their nominal places, then `coded`.
fn generic_region(width: u32, height: u32, x: u32, y: u32, coded: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(26 + coded.len());
    region_information(&mut data, width, height, x, y);
    data.push(0x08);
    at_pixels(&mut data);
    data.extend_from_slice(coded);
    data
}

/// Template 0's four AT pixels at their nominal places, as AT flags (7.4.6.3, 7.4.2.1.2).
fn at_pixels(out: &mut Vec<u8>) {
    for (x, y) in &TEMPLATE[12..] {
        out.push(*x as i8 as u8);
        out.push(*y as i8 as u8);
    }
}

/// A segment (7.2) of page 1: its header (number, type, the segments it refers to, the page, data
/// length), then its data. `retained`: a later segment refers to this one. `refers_to` holds each
/// referred-to segment's number and whether a segment after this one refers to it too (its
/// retain bit, 7.2.4). Segment numbers here stay below 256, so each referred-to one takes a byte
/// (7.2.5); a segment refers to at most four, so their count and retain bits take one byte too.
fn segment(out: &mut Vec<u8>, number: u32, kind: u8, refers_to: &[(u8, bool)], retained: bool, data: &[u8]) {
    segment_of(out, 1, number, kind, refers_to, retained, data);
}

/// [`segment`], of page `page`: 0 for a segment associated with no page (7.2.6), such as the
/// global ones `/JBIG2Globals` holds, which a page's segments may refer to.
fn segment_of(out: &mut Vec<u8>, page: u8, number: u32, kind: u8, refers_to: &[(u8, bool)], retained: bool, data: &[u8]) {
    out.extend_from_slice(&number.to_be_bytes());
    out.push(kind);
    // At most four, so the count fits its three bits.
    let refers_to = refers_to.get(..refers_to.len().min(4)).unwrap_or_default();
    let kept = refers_to.iter().enumerate().fold(0u8, |bits, (k, (_, kept))| bits | u8::from(*kept) << (k + 1));
    out.push((refers_to.len() as u8) << 5 | kept | u8::from(retained));
    out.extend(refers_to.iter().map(|(n, _)| *n));
    out.push(page);
    // A segment's data never comes near 4 GiB here: an image is at most 2^28 samples.
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(data);
}

/// Codes a `width` × `height` bitmap as a generic region (6.2.5.7) with template 0 and the nominal
/// AT pixels, its decisions in `contexts` (2^16 of them), with typical prediction when `typical`
/// (TPGDON). `fill(y, row)` sets row `y`'s `width` pixels, 1 for black. Pixels outside the bitmap
/// count as 0 (white), as the decoder takes them.
fn code_generic(mq: &mut MqEncoder, contexts: &mut [Context], width: usize, height: usize, typical: bool, mut fill: impl FnMut(usize, &mut [u8])) {
    let sltp = SLTP.iter().enumerate().fold(0usize, |cx, (k, v)| cx | usize::from(*v) << k);
    // The two rows above the current one and the current one, as JBIG2 pixels (1 black), padded.
    let blank = vec![0u8; width + 2 * PAD];
    let (mut above2, mut above1) = (blank.clone(), blank.clone());
    let mut row = blank;
    let mut ltp = false;
    for y in 0..height {
        if let Some(pixels) = row.get_mut(PAD..PAD + width) {
            fill(y, pixels);
        }
        if typical {
            // Typical prediction (6.2.5.5): a row the same as the one above is coded as one decision.
            let same = row == above1;
            if let Some(cx) = contexts.get_mut(sltp) {
                mq.encode(cx, same != ltp);
            }
            ltp = same;
        }
        if !ltp {
            for x in 0..width {
                let at = |dx: i32, dy: i32| {
                    let line = match dy {
                        0 => &row,
                        -1 => &above1,
                        _ => &above2,
                    };
                    let i = (PAD + x).checked_add_signed(dx as isize);
                    i.and_then(|i| line.get(i)).copied().unwrap_or(0)
                };
                let cx = TEMPLATE.iter().enumerate().fold(0usize, |cx, (k, (dx, dy))| cx | usize::from(at(*dx, *dy)) << k);
                if let Some(state) = contexts.get_mut(cx) {
                    mq.encode(state, row.get(PAD + x) == Some(&1));
                }
            }
        }
        std::mem::swap(&mut above2, &mut above1);
        std::mem::swap(&mut above1, &mut row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows an independent decoder (`hayro-jbig2`) reads from `data`, as PDF samples: true
    /// for white (a JBIG2 0).
    pub(super) fn decode(data: &[u8]) -> Vec<Vec<bool>> {
        decode_with(data, None)
    }

    /// [`decode`], with `/JBIG2Globals`.
    pub(super) fn decode_with(data: &[u8], globals: Option<&[u8]>) -> Vec<Vec<bool>> {
        struct Rows(Vec<Vec<bool>>, Vec<bool>, usize);
        impl hayro_jbig2::Decoder for Rows {
            fn push_pixel(&mut self, black: bool) {
                self.1.push(!black);
            }
            fn push_pixel_chunk(&mut self, black: bool, chunks: u32) {
                self.1.extend(std::iter::repeat_n(!black, chunks as usize * 8));
            }
            fn next_line(&mut self) {
                let mut row = std::mem::take(&mut self.1);
                row.truncate(self.2);
                self.0.push(row);
            }
        }
        let image = hayro_jbig2::Image::new_embedded(data, globals).unwrap();
        let mut rows = Rows(Vec::new(), Vec::new(), image.width() as usize);
        image.decode(&mut rows).unwrap();
        rows.0
    }

    /// The rows of packed samples `data`, true for a 1 bit.
    pub(super) fn unpack(data: &[u8], columns: u32, rows: u32) -> Vec<Vec<bool>> {
        let stride = (columns as usize).div_ceil(8);
        (0..rows as usize).map(|y| (0..columns as usize).map(|x| data[y * stride + x / 8] & (0x80 >> (x % 8)) != 0).collect()).collect()
    }

    /// Rows of `columns` samples from `runs`: alternating white and black runs one longer than
    /// those values (never empty, even when shrinking makes them all 0), until the rows are full;
    /// or, with `noise`, the bytes of `runs` as they are.
    fn bitmap(columns: u32, rows: u32, runs: &[u8], noise: bool) -> Vec<u8> {
        let stride = (columns as usize).div_ceil(8);
        let mut data = vec![0u8; stride * rows as usize];
        if runs.is_empty() {
            return data;
        }
        if noise {
            data.iter_mut().zip(runs.iter().cycle()).for_each(|(d, r)| *d = *r);
            return data;
        }
        let (mut white, mut left, mut next) = (true, 1 + usize::from(runs[0]), 1);
        for y in 0..rows as usize {
            for x in 0..columns as usize {
                if left == 0 {
                    white = !white;
                    left = 1 + usize::from(runs[next % runs.len()]);
                    next += 1;
                }
                if white {
                    data[y * stride + x / 8] |= 0x80 >> (x % 8);
                }
                left -= 1;
            }
        }
        data
    }

    #[test]
    fn a_page_of_text_like_marks_round_trips_and_shrinks() {
        let (w, h) = (1700u32, 220u32);
        let stride = (w as usize).div_ceil(8);
        let mut data = vec![0xFF; stride * h as usize];
        for y in (20..h as usize).step_by(40) {
            for dy in 0..12 {
                for x in (100..1600).step_by(9) {
                    if (x / 9 + dy) % 5 != 0 {
                        data[(y + dy) * stride + x / 8] &= !(0x80 >> (x % 8));
                    }
                }
            }
        }
        let jbig2 = encode_jbig2_generic(&data, w, h).unwrap();
        assert_eq!(decode(&jbig2), unpack(&data, w, h));
        let g4 = crate::encode_g4(&data, w, h).unwrap();
        assert!(jbig2.len() < g4.len(), "JBIG2 {} bytes, Group 4 {}", jbig2.len(), g4.len());
    }

    #[test]
    fn edges_padding_and_typical_rows_round_trip() {
        // 3 columns: the last 5 bits of each byte pad the row; set them to anything.
        let data = [0b1010_1111, 0b0101_0000, 0b1110_1010, 0xAB, 0xCD];
        assert_eq!(decode(&encode_jbig2_generic(&data, 3, 3).unwrap()), [[true, false, true], [false, true, false], [true, true, true]]);
        for (w, h, fill) in [(1, 1, 0x00), (1, 1, 0xFF), (13, 9, 0x00), (13, 9, 0xFF), (64, 40, 0x5A)] {
            let data = vec![fill; (w as usize).div_ceil(8) * h as usize];
            assert_eq!(decode(&encode_jbig2_generic(&data, w, h).unwrap()), unpack(&data, w, h), "{w} × {h} of {fill:#04x}");
        }
    }

    /// Packed samples from JBIG2 pixels (1 black): a 0 sample for each 1.
    pub(super) fn samples(pixels: &[Vec<u8>]) -> Vec<u8> {
        let width = pixels.first().map_or(0, Vec::len);
        let stride = width.div_ceil(8);
        let mut data = vec![0xFF; stride * pixels.len()];
        for (y, row) in pixels.iter().enumerate() {
            for (x, p) in row.iter().enumerate() {
                if *p == 1 {
                    data[y * stride + x / 8] &= !(0x80 >> (x % 8));
                }
            }
        }
        data
    }

    /// A row the same as the one above is typical (6.2.5.5): one decision, not a row of pixels.
    #[test]
    fn a_repeated_row_is_one_decision() {
        let row: Vec<u8> = (0..800u32).map(|x| u8::from((x * 7919 + x / 3) % 5 < 2)).collect();
        let pixels = vec![row; 1000];
        let data = samples(&pixels);
        crate::mq::DECISIONS.with(|n| n.set(0));
        let jbig2 = encode_jbig2_generic(&data, 800, 1000).unwrap();
        // The first row's 800 pixels, and one typical-row decision for each of the 1000 rows.
        assert_eq!(crate::mq::DECISIONS.with(|n| n.get()), 800 + 1000);
        assert_eq!(decode(&jbig2), unpack(&data, 800, 1000));
    }

    /// SLTP is coded in the context Figure 8 shows, which ordinary pixels can share: an image
    /// tiled with that very neighbourhood decodes only if both coders use the same context.
    #[test]
    fn the_typical_row_flag_shares_figure_8s_context() {
        // Figure 8, around a pixel at x = 4 of an 8-column tile: two rows up x = 2..6, one row up
        // x = 1..7, the pixel's own row x = 0..3.
        let tile = [[0, 0, 1, 0, 0, 1, 1, 0], [0, 0, 1, 1, 0, 0, 1, 0], [0, 1, 0, 1, 1, 0, 0, 0]];
        let pixels: Vec<Vec<u8>> = (0..90).map(|y| (0..400).map(|x| tile[y % 3][x % 8]).collect()).collect();
        let data = samples(&pixels);
        assert_eq!(decode(&encode_jbig2_generic(&data, 400, 90).unwrap()), unpack(&data, 400, 90));
    }

    #[test]
    fn what_it_cannot_encode_is_an_error() {
        assert!(matches!(encode_jbig2_generic(&[0; 8], 0, 1), Err(FilterError::Unsupported(_))));
        assert!(matches!(encode_jbig2_generic(&[0; 8], 8, 0), Err(FilterError::Unsupported(_))));
        assert!(matches!(encode_jbig2_generic(&[0; 3], 16, 2), Err(FilterError::Corrupt { .. })), "two rows of 16 need 4 bytes");
        assert!(matches!(encode_jbig2_generic(&[], 1, 2), Err(FilterError::Corrupt { .. })));
    }

    /// The encoder stops where the decoder does, on each side and in total: past that it would
    /// write a stream nothing here can read. 16,384 squared is exactly the bound on samples.
    /// `encode_jbig2_shared` refuses what `encode_jbig2_generic` refuses: an image taller than
    /// the decoder takes is `Unsupported`, one at the bound isn't refused. (Its one tall mark is
    /// too large to be a symbol, so nothing is shared: `Ok(None)`.)
    #[test]
    fn shared_coding_takes_only_what_the_decoder_takes() {
        let past = vec![0x7F; MAX_JBIG2_COLUMNS as usize + 1];
        let tall = (&past[..], 1, MAX_JBIG2_COLUMNS + 1);
        assert!(matches!(encode_jbig2_shared(&[tall, tall]), Err(FilterError::Unsupported(_))), "too tall");
        let at = (&past[..MAX_JBIG2_COLUMNS as usize], 1, MAX_JBIG2_COLUMNS);
        assert!(matches!(encode_jbig2_shared(&[at, at]), Ok(None)), "at the bound");
    }

    #[test]
    fn it_encodes_only_what_the_decoder_takes() {
        let unsupported = |columns, rows| matches!(encode_jbig2_generic(&[], columns, rows), Err(FilterError::Unsupported(_)));
        assert!(unsupported(MAX_JBIG2_COLUMNS + 1, 1), "too wide");
        assert!(unsupported(1, MAX_JBIG2_COLUMNS + 1), "too tall");
        assert!(unsupported(16_385, 16_384), "too many samples, each side within the bound");
        assert!(unsupported(u32::MAX, u32::MAX), "the product doesn't overflow");
        // At the bounds what it writes reads back, through the decoder that has to read it.
        let wide = vec![0x0F; (MAX_JBIG2_COLUMNS as usize).div_ceil(8) * 4];
        assert_eq!(decode(&encode_jbig2_generic(&wide, MAX_JBIG2_COLUMNS, 4).unwrap()), unpack(&wide, MAX_JBIG2_COLUMNS, 4));
        let tall = vec![0x7F; MAX_JBIG2_COLUMNS as usize];
        assert_eq!(decode(&encode_jbig2_generic(&tall, 1, MAX_JBIG2_COLUMNS).unwrap()), unpack(&tall, 1, MAX_JBIG2_COLUMNS));
        struct Rows(u32, u64);
        impl hayro_jbig2::Decoder for Rows {
            fn push_pixel(&mut self, black: bool) {
                self.1 += u64::from(black);
            }
            fn push_pixel_chunk(&mut self, black: bool, chunks: u32) {
                self.1 += u64::from(black) * u64::from(chunks) * 8;
            }
            fn next_line(&mut self) {
                self.0 += 1;
            }
        }
        // One black pixel, so a page that came back blank wouldn't pass.
        let mut largest = vec![0xFF; 2_048 * 16_384];
        largest[0] = 0x7F;
        let largest = encode_jbig2_generic(&largest, 16_384, 16_384).unwrap();
        let mut rows = Rows(0, 0);
        hayro_jbig2::Image::new_embedded(&largest, None).unwrap().decode(&mut rows).unwrap();
        assert_eq!(rows.0, 16_384);
        assert_eq!(rows.1, 1);
    }

    proptest::proptest! {
        /// Any bitmap, of noise or of runs, decodes to its own bits through an independent decoder.
        #[test]
        fn bitmaps_round_trip(
            columns in 1u32..300,
            rows in 1u32..24,
            runs in proptest::collection::vec(0u8..=255, 1..48),
            noise in proptest::bool::ANY,
        ) {
            let data = bitmap(columns, rows, &runs, noise);
            let jbig2 = encode_jbig2_generic(&data, columns, rows).unwrap();
            proptest::prop_assert_eq!(decode(&jbig2), unpack(&data, columns, rows));
        }

        /// Whatever it is given, it returns.
        #[test]
        fn any_input_returns(data in proptest::collection::vec(0u8..=255, 0..600), columns in 0u32..5000, rows in 0u32..80) {
            let _ = encode_jbig2_generic(&data, columns, rows);
        }
    }
}
