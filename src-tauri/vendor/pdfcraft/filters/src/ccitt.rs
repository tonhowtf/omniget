//! CCITT fax for `CCITTFaxDecode` (ISO 32000-2 §7.4.6): Group 4 (ITU-T T.6) encoding, with `/K -1`,
//! and decoding of Group 3 (T.4) and Group 4. The coding is the `fax` crate's and the decoding the
//! `hayro-ccitt` crate's; this module checks what they are given and bounds what they give.

use fax::encoder::Encoder;
use fax::{Color, VecWriter};

use crate::FilterError;

/// The widest row [`encode_g4`] encodes. The encoder counts the positions in a row in 16 bits
/// and adds two of them, so a row stays within half that range.
pub const MAX_G4_COLUMNS: u32 = 32_767;

/// `rows` rows of `columns` 1-bit samples, most significant bit first, each row starting on a new
/// byte (as a PDF image's samples are packed), encoded as CCITT Group 4 with an end-of-block mark.
///
/// A 0 bit is coded black and a 1 bit white, so the stream decodes to the same bits with
/// `/BlackIs1 false`, the default: an image keeps its colour space and decode array. Bytes after
/// the last row are ignored, as are the bits that pad a row to a whole byte.
///
/// Errors: no rows or columns, or rows wider than [`MAX_G4_COLUMNS`] ([`FilterError::Unsupported`]);
/// fewer bytes than the rows need ([`FilterError::Corrupt`]).
pub fn encode_g4(data: &[u8], columns: u32, rows: u32) -> Result<Vec<u8>, FilterError> {
    let width = match u16::try_from(columns) {
        Ok(w) if w > 0 && columns <= MAX_G4_COLUMNS => w,
        _ => return Err(FilterError::Unsupported(format!("CCITT Group 4 encoding of rows {columns} samples wide"))),
    };
    if rows == 0 {
        return Err(FilterError::Unsupported("CCITT Group 4 encoding of an image without rows".into()));
    }
    let stride = usize::from(width).div_ceil(8);
    let samples = usize::try_from(rows).ok().and_then(|r| r.checked_mul(stride)).and_then(|n| data.get(..n)).ok_or_else(|| FilterError::Corrupt {
        filter: "CCITTFaxDecode",
        detail: format!("{} bytes for {rows} rows of {columns} samples", data.len()),
    })?;
    let mut encoder = Encoder::new(VecWriter::new());
    for row in samples.chunks_exact(stride) {
        let pels = (0..usize::from(width)).map(|x| {
            let white = row.get(x / 8).is_some_and(|b| b & (0x80 >> (x % 8)) != 0);
            if white { Color::White } else { Color::Black }
        });
        // `VecWriter` writes to memory: its error type has no values.
        if let Err(never) = encoder.encode_line(pels, width) {
            match never {}
        }
    }
    match encoder.finish() {
        Ok(writer) => Ok(writer.finish()),
        Err(never) => match never {},
    }
}

/// The widest row [`decode_ccitt_within`] decodes: its row buffer stays within 128 KiB.
pub const MAX_CCITT_COLUMNS: u32 = 1 << 20;

/// A `CCITTFaxDecode` stream's `/DecodeParms` (ISO 32000-2 Table 11). [`Default`] gives the
/// table's defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CcittParams {
    /// `/K`: negative for Group 4, 0 for Group 3 one-dimensional, positive for Group 3 mixed.
    pub k: i64,
    pub columns: i64,
    /// `/Rows`: 0 when not given.
    pub rows: i64,
    pub end_of_line: bool,
    pub encoded_byte_align: bool,
    pub end_of_block: bool,
    pub black_is_1: bool,
}

impl Default for CcittParams {
    fn default() -> Self {
        CcittParams { k: 0, columns: 1728, rows: 0, end_of_line: false, encoded_byte_align: false, end_of_block: true, black_is_1: false }
    }
}

/// What [`decode_ccitt_within`] decodes once it is paid for.
fn decode_ccitt(data: &[u8], p: &CcittParams, max_rows: u32) -> Result<Vec<u8>, FilterError> {
    let columns = u32::try_from(p.columns)
        .ok()
        .filter(|c| (1..=MAX_CCITT_COLUMNS).contains(c))
        .ok_or_else(|| FilterError::Unsupported(format!("CCITT fax rows {} samples wide", p.columns)))?;
    let rows = u32::try_from(p.rows).ok().filter(|r| *r > 0).map_or(max_rows, |r| r.min(max_rows));
    if rows == 0 {
        return Ok(Vec::new());
    }
    let settings = hayro_ccitt::DecodeSettings {
        columns,
        rows,
        end_of_block: p.end_of_block,
        end_of_line: p.end_of_line,
        // Group 3 rows on bytes may have end-of-line codes whatever `end_of_line` says (the filter
        // always accepts them), with the fill starting each code on a byte or ending it on one (T.4's
        // fill, as TIFF writes it). Read unaligned, the codes mark the rows either way; without
        // codes, the rows are read on bytes. A reading that comes up short isn't tried the other
        // way: data cut short would then read as other rows.
        rows_are_byte_aligned: p.encoded_byte_align && (p.k < 0 || !starts_with_end_of_line(data)),
        encoding: match p.k {
            ..0 => hayro_ccitt::EncodingMode::Group4,
            0 => hayro_ccitt::EncodingMode::Group3_1D,
            k => hayro_ccitt::EncodingMode::Group3_2D { k: u32::try_from(k).unwrap_or(u32::MAX) },
        },
        invert_black: false,
    };
    // u32 to usize: wasm32 and up have at least 32-bit usize.
    let mut out = Rows::new(columns as usize, rows as usize, p.black_is_1);
    // The rows decoded before damaged data are kept.
    let _ = hayro_ccitt::decode(data, &mut out, &mut hayro_ccitt::DecoderContext::new(settings));
    Ok(out.out)
}

/// The steps of reading a byte of fax code ([`decode_ccitt_within`]): noise takes 114 ns a byte of
/// rows and about two bytes of code to a byte of rows, so that and a step a byte of rows is about
/// what a step of JBIG2 decoding takes.
const CODE_WORK: u64 = 4;

/// CCITT fax `data` decoded as `CCITTFaxDecode` gives it: rows of `columns` 1-bit samples, most
/// significant bit first, each starting on a new byte, with black as 0 (as 1 with `black_is_1`).
///
/// At most `max_rows` rows are decoded (fewer when `rows` says so), so the output is bounded by
/// the caller. Damaged data gives the rows decoded before the damage, as viewers show them: a
/// caller that needs a number of rows checks it got them.
///
/// Group 3 rows on bytes (`k` from 0, `encoded_byte_align`) are read by their end-of-line codes,
/// whether the fill starts each code on a byte or ends it on one, when `data` starts with one (T.4
/// sends one before the first row when it sends them at all), and on bytes otherwise, whatever
/// `end_of_line` says. Group 4 is read as its parameters say. Data with codes but none before the
/// first row is read on bytes, as PdfCraft's renderer draws it; with `k` 0 and the fill ending each
/// code on a byte, that may be other rows than were coded.
///
/// Errors: `columns` that aren't from 1 to [`MAX_CCITT_COLUMNS`] ([`FilterError::Unsupported`]).
///
/// Decoding spends from `work`, what decoding may still cost across a run of images, and there is
/// no way in without it: a
/// byte of rows a step, whether or not the data holds them, and [`CODE_WORK`] a byte of `data`.
/// A blank row takes a bit of code, so the rows are what a few bytes can ask for. It is all paid
/// before anything is decoded, and what `work` can't pay for spends all of it.
pub fn decode_ccitt_within(data: &[u8], p: &CcittParams, max_rows: u32, work: &mut u64) -> Result<Vec<u8>, FilterError> {
    let rows = u32::try_from(p.rows).ok().filter(|r| *r > 0).map_or(max_rows, |r| r.min(max_rows));
    let stride = u64::try_from(p.columns).unwrap_or(0).div_ceil(8);
    crate::pay(work, stride.saturating_mul(u64::from(rows)).saturating_add((data.len() as u64).saturating_mul(CODE_WORK)))?;
    decode_ccitt(data, p, max_rows)
}

/// Whether CCITT fax `data` starts with an end-of-line code: a 1 after 11 zeros or more, fill
/// included. T.4 sends one before the first row when it sends them at all, and no other code starts
/// with more than eight zeros.
fn starts_with_end_of_line(data: &[u8]) -> bool {
    let Some((i, b)) = data.iter().enumerate().find(|(_, b)| **b != 0) else { return false };
    i.saturating_mul(8).saturating_add(b.leading_zeros() as usize) >= 11
}

/// Rows of 1-bit samples filled from a decoder's pixels: `columns` a row, each row on a new byte, at
/// most `rows` rows. Pixels past a row's end and rows past the last are dropped. The JBIG2 decoder
/// ([`crate::decode_jbig2_within`]) fills them too.
pub(crate) struct Rows {
    pub(crate) out: Vec<u8>,
    row: Vec<u8>,
    x: usize,
    columns: usize,
    rows: usize,
    done: usize,
    black_is_1: bool,
}

impl Rows {
    pub(crate) fn new(columns: usize, rows: usize, black_is_1: bool) -> Self {
        Rows { out: Vec::new(), row: vec![0; columns.div_ceil(8)], x: 0, columns, rows, done: 0, black_is_1 }
    }

    /// `count` pixels, white or black.
    pub(crate) fn push(&mut self, white: bool, count: usize) {
        let end = self.x.saturating_add(count);
        if white != self.black_is_1 {
            for x in self.x..end.min(self.columns) {
                if let Some(b) = self.row.get_mut(x / 8) {
                    *b |= 0x80 >> (x % 8);
                }
            }
        }
        self.x = end;
    }

    /// The row filled so far kept, if fewer than `rows` are, and the next one begun.
    pub(crate) fn end_row(&mut self) {
        if self.done < self.rows {
            self.out.extend_from_slice(&self.row);
            self.done += 1;
        }
        self.row.fill(0);
        self.x = 0;
    }
}

impl hayro_ccitt::Decoder for Rows {
    fn push_pixel(&mut self, white: bool) {
        self.push(white, 1);
    }
    fn push_pixel_chunk(&mut self, white: bool, chunks: u32) {
        self.push(white, (chunks as usize).saturating_mul(8));
    }
    fn next_line(&mut self) {
        self.end_row();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run pays for the rows an image asks for and the code it reads, before any of it is
    /// decoded: rows the data doesn't hold are paid for too, as a few bytes can ask for any number.
    #[test]
    fn a_run_of_images_pays_for_the_rows_it_asks_for() {
        let (w, h) = (4096u32, 64u32);
        let stride = (w as usize).div_ceil(8);
        let data = encode_g4(&vec![0xFF; stride * h as usize], w, h).unwrap();
        let p = CcittParams { k: -1, columns: i64::from(w), ..CcittParams::default() };
        let whole = decode_ccitt(&data, &p, h).unwrap();
        let cost = (whole.len() + 4 * data.len()) as u64;
        let mut work = cost;
        assert_eq!(decode_ccitt_within(&data, &p, h, &mut work).unwrap(), whole, "exactly enough");
        assert_eq!(work, 0);
        let mut work = cost + 7;
        decode_ccitt_within(&data, &p, h, &mut work).unwrap();
        assert_eq!(work, 7, "only what it took");
        let mut work = cost - 1;
        let refused = decode_ccitt_within(&data, &p, h, &mut work).unwrap_err();
        assert!(matches!(refused, FilterError::Spent), "a step short: {refused}");
        assert_eq!(work, 0, "spends all that was left");
        // Nothing left: not read at all, so bad columns aren't reported as bad columns.
        let bad = CcittParams { columns: 0, ..p };
        assert!(decode_ccitt_within(&data, &bad, h, &mut 0).is_err_and(|e| matches!(e, FilterError::Spent)));
        // No code at all, for the same rows.
        let mut work = u64::MAX;
        decode_ccitt_within(&[], &p, h, &mut work).unwrap();
        assert_eq!(u64::MAX - work, whole.len() as u64);
        // `/Rows` below what is asked for, and no rows.
        let mut work = u64::MAX;
        decode_ccitt_within(&data, &CcittParams { rows: 3, ..p }, h, &mut work).unwrap();
        assert_eq!(u64::MAX - work, (3 * stride + 4 * data.len()) as u64);
        let mut work = u64::MAX;
        decode_ccitt_within(&data, &p, 0, &mut work).unwrap();
        assert_eq!(u64::MAX - work, 4 * data.len() as u64);
    }

    /// The rows decoded by an independent decoder (`hayro-ccitt`), as 1 for white and 0 for black.
    pub(crate) fn decode(data: &[u8], columns: u32, rows: u32) -> Vec<Vec<bool>> {
        struct Rows(Vec<Vec<bool>>, Vec<bool>, usize);
        impl hayro_ccitt::Decoder for Rows {
            fn push_pixel(&mut self, white: bool) {
                self.1.push(white);
            }
            fn push_pixel_chunk(&mut self, white: bool, chunks: u32) {
                self.1.extend(std::iter::repeat_n(white, chunks as usize * 8));
            }
            fn next_line(&mut self) {
                let mut row = std::mem::take(&mut self.1);
                row.truncate(self.2);
                self.0.push(row);
            }
        }
        let settings = hayro_ccitt::DecodeSettings {
            columns,
            rows,
            end_of_block: true,
            end_of_line: false,
            rows_are_byte_aligned: false,
            encoding: hayro_ccitt::EncodingMode::Group4,
            invert_black: false,
        };
        let mut out = Rows(Vec::new(), Vec::new(), columns as usize);
        hayro_ccitt::decode(data, &mut out, &mut hayro_ccitt::DecoderContext::new(settings)).unwrap();
        out.0
    }

    /// The rows of packed samples `data`, as 1 for a 1 bit.
    pub(crate) fn unpack(data: &[u8], columns: u32, rows: u32) -> Vec<Vec<bool>> {
        let stride = (columns as usize).div_ceil(8);
        (0..rows as usize).map(|y| (0..columns as usize).map(|x| data[y * stride + x / 8] & (0x80 >> (x % 8)) != 0).collect()).collect()
    }

    #[test]
    fn a_page_of_text_like_marks_round_trips_and_shrinks() {
        let (w, h) = (1700u32, 220u32);
        let stride = (w as usize).div_ceil(8);
        let mut data = vec![0xFF; stride * h as usize];
        // Short black strokes in lines, like glyphs on a white page.
        for y in (20..h as usize).step_by(40) {
            for dy in 0..12 {
                for x in (100..1600).step_by(9) {
                    if (x / 9 + dy) % 5 != 0 {
                        data[(y + dy) * stride + x / 8] &= !(0x80 >> (x % 8));
                    }
                }
            }
        }
        let g4 = encode_g4(&data, w, h).unwrap();
        assert_eq!(decode(&g4, w, h), unpack(&data, w, h));
        assert!(g4.len() * 4 < data.len(), "{} bytes from {}", g4.len(), data.len());
    }

    #[test]
    fn padding_bits_and_trailing_bytes_are_ignored() {
        // 3 columns: the last 5 bits of each byte pad the row; set them to anything.
        let data = [0b1010_1111, 0b0101_0000, 0b1110_1010, 0xAB, 0xCD];
        let g4 = encode_g4(&data, 3, 3).unwrap();
        assert_eq!(decode(&g4, 3, 3), [[true, false, true], [false, true, false], [true, true, true]]);
    }

    #[test]
    fn what_it_cannot_encode_is_an_error() {
        assert!(matches!(encode_g4(&[0; 8], 0, 1), Err(FilterError::Unsupported(_))));
        assert!(matches!(encode_g4(&[0; 8], 8, 0), Err(FilterError::Unsupported(_))));
        assert!(matches!(encode_g4(&vec![0; 4097], MAX_G4_COLUMNS + 1, 1), Err(FilterError::Unsupported(_))));
        assert!(matches!(encode_g4(&[0; 3], 16, 2), Err(FilterError::Corrupt { .. })), "two rows of 16 need 4 bytes");
        assert!(matches!(encode_g4(&[], 1, u32::MAX), Err(FilterError::Corrupt { .. })));
        let widest = vec![0x55; (MAX_G4_COLUMNS as usize).div_ceil(8) * 2];
        let g4 = encode_g4(&widest, MAX_G4_COLUMNS, 2).unwrap();
        assert_eq!(decode(&g4, MAX_G4_COLUMNS, 2), unpack(&widest, MAX_G4_COLUMNS, 2));
    }

    /// Rows of `columns` samples built from `runs`: alternating white and black runs one longer
    /// than those values (so never empty, even when shrinking makes them all 0), repeated until the
    /// rows are full; or, with `noise`, the bytes of `runs` as they are.
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

    fn g4(columns: u32) -> CcittParams {
        CcittParams { k: -1, columns: i64::from(columns), ..CcittParams::default() }
    }

    #[test]
    fn decoding_gives_the_rows_back() {
        // 3 columns, padded with anything: the decoded padding is 0.
        let data = [0b1010_1111, 0b0101_0000, 0b1110_1010];
        let coded = encode_g4(&data, 3, 3).unwrap();
        assert_eq!(decode_ccitt(&coded, &g4(3), 3).unwrap(), [0b1010_0000, 0b0100_0000, 0b1110_0000]);
        // /BlackIs1 gives black as 1.
        let p = CcittParams { black_is_1: true, ..g4(3) };
        assert_eq!(decode_ccitt(&coded, &p, 3).unwrap(), [0b0100_0000, 0b1010_0000, 0b0000_0000]);
        // At most /Rows rows, and at most max_rows.
        assert_eq!(decode_ccitt(&coded, &CcittParams { rows: 2, ..g4(3) }, 3).unwrap().len(), 2);
        assert_eq!(decode_ccitt(&coded, &g4(3), 1).unwrap().len(), 1);
        assert_eq!(decode_ccitt(&coded, &CcittParams { rows: 3, ..g4(3) }, 1).unwrap().len(), 1, "max_rows bounds /Rows");
        assert_eq!(decode_ccitt(&coded, &g4(3), 0).unwrap(), [0u8; 0]);
        // Damaged data gives the rows before the damage; none here.
        assert!(decode_ccitt(&coded[..1], &g4(3), 3).unwrap().len() < 3);
    }

    #[test]
    fn what_it_cannot_decode_is_an_error() {
        for columns in [0, -1, i64::from(MAX_CCITT_COLUMNS) + 1, i64::MAX] {
            let p = CcittParams { columns, ..CcittParams::default() };
            assert!(matches!(decode_ccitt(&[0; 8], &p, 4), Err(FilterError::Unsupported(_))), "{columns}");
        }
        assert_eq!(CcittParams::default().columns, 1728, "Table 11");
    }

    /// Group 3 data: rows of run lengths below 64, white first, coded one-dimensionally with T.4's
    /// terminating codes (ITU-T T.4 Tables 2 and 3). Each row starts with an end-of-line code with
    /// `eol`, then with `tag` a 1 bit (the next row one-dimensional, as `k` above 0 has it). Fill
    /// bits (0s) go where `fill` says.
    fn group_3(rows: &[Vec<u32>], eol: bool, tag: bool, fill: Fill) -> Vec<u8> {
        // The codes for runs 0 to 63, in order.
        const WHITE: &str = concat!(
            "00110101 000111 0111 1000 1011 1100 1110 1111 10011 10100 00111 01000 001000 000011 110100 110101 101010 101011 ",
            "0100111 0001100 0001000 0010111 0000011 0000100 0101000 0101011 0010011 0100100 0011000 00000010 00000011 00011010 ",
            "00011011 00010010 00010011 00010100 00010101 00010110 00010111 00101000 00101001 00101010 00101011 00101100 00101101 ",
            "00000100 00000101 00001010 00001011 01010010 01010011 01010100 01010101 00100100 00100101 01011000 01011001 01011010 ",
            "01011011 01001010 01001011 00110010 00110011 00110100",
        );
        const BLACK: &str = concat!(
            "0000110111 010 11 10 011 0011 0010 00011 000101 000100 0000100 0000101 0000111 00000100 00000111 000011000 0000010111 ",
            "0000011000 0000001000 00001100111 00001101000 00001101100 00000110111 00000101000 00000010111 00000011000 ",
            "000011001010 000011001011 000011001100 000011001101 000001101000 000001101001 000001101010 000001101011 000011010010 ",
            "000011010011 000011010100 000011010101 000011010110 000011010111 000001101100 000001101101 000011011010 000011011011 ",
            "000001010100 000001010101 000001010110 000001010111 000001100100 000001100101 000001010010 000001010011 000000100100 ",
            "000000110111 000000111000 000000100111 000000101000 000001011000 000001011001 000000101011 000000101100 000001011010 ",
            "000001100110 000001100111",
        );
        let code = |white: bool, run: u32| (if white { WHITE } else { BLACK }).split(' ').nth(run as usize).expect("a run below 64");
        let mut bits = String::new();
        for row in rows {
            // The end-of-line code is 12 bits.
            let ends_code = if fill == Fill::CodeEnd && eol { 12 } else { 0 };
            while fill != Fill::None && !(bits.len() + ends_code).is_multiple_of(8) {
                bits.push('0');
            }
            if eol {
                bits.push_str("000000000001");
            }
            if tag {
                bits.push('1');
            }
            row.iter().enumerate().for_each(|(i, run)| bits.push_str(code(i % 2 == 0, *run)));
        }
        bits.as_bytes().chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (i, d)| b | (u8::from(*d == b'1') << (7 - i)))).collect()
    }

    /// Where [`group_3`] puts fill bits: none; so each row, its end-of-line code first, starts on a
    /// byte; or so each end-of-line code ends on one (T.4's fill, as TIFF writes it).
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Fill {
        None,
        RowStart,
        CodeEnd,
    }

    /// `rows` text-like rows of `columns` samples as runs, white first, from `seed`: long white runs
    /// and short black ones; a row may start black.
    fn text_like(seed: u64, columns: u32, rows: u32) -> Vec<Vec<u32>> {
        let mut seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut random = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n) as u32
        };
        (0..rows)
            .map(|_| {
                let (mut runs, mut left) = (Vec::new(), columns);
                while left > 0 {
                    let run = match runs.len() % 2 {
                        0 if runs.is_empty() => random(31),
                        0 => 1 + random(30),
                        _ => 1 + random(5),
                    };
                    runs.push(run.min(left));
                    left -= run.min(left);
                }
                runs
            })
            .collect()
    }

    /// Rows of runs, white first, packed as `decode_ccitt` gives them (white 1).
    fn pack(runs: &[Vec<u32>], columns: u32) -> Vec<u8> {
        let stride = (columns as usize).div_ceil(8);
        let mut out = vec![0u8; stride * runs.len()];
        for (y, row) in runs.iter().enumerate() {
            let mut x = 0;
            for (i, run) in row.iter().enumerate() {
                if i % 2 == 0 {
                    (x..x + *run as usize).for_each(|x| out[y * stride + x / 8] |= 0x80 >> (x % 8));
                }
                x += *run as usize;
            }
        }
        out
    }

    /// Group 3 rows on bytes in each layout, over many pictures, as how a misreading comes out
    /// depends on the picture: no codes (as TIFF's Modified Huffman has it), and `end_of_line`
    /// without them; codes, each starting on a byte or ending on one, `end_of_line` or not. Each
    /// decodes to its picture, and a byte short to fewer rows, not to other rows read another way.
    /// Group 4 rows on bytes keep their aligned read, `end_of_line` or not.
    #[test]
    fn group_3_rows_on_bytes_read_by_the_codes_in_the_data() {
        for seed in 0..32u64 {
            let (columns, rows) = (8 + (seed as u32 * 37) % 200, 2 + seed as u32 % 20);
            let runs = text_like(seed, columns, rows);
            let picture = pack(&runs, columns);
            for k in [0, 1] {
                for (codes, fill, end_of_line) in [
                    (false, Fill::RowStart, false),
                    (false, Fill::RowStart, true),
                    (true, Fill::RowStart, false),
                    (true, Fill::RowStart, true),
                    (true, Fill::CodeEnd, false),
                    (true, Fill::CodeEnd, true),
                ] {
                    let p = CcittParams { k, columns: i64::from(columns), end_of_line, encoded_byte_align: true, ..CcittParams::default() };
                    let mut data = group_3(&runs, codes, k > 0, fill);
                    let case = format!("seed {seed}, {columns} × {rows}, k {k}, codes {codes}, fill {fill:?}, end_of_line {end_of_line}");
                    assert_eq!(decode_ccitt(&data, &p, rows).unwrap(), picture, "{case}");
                    data.pop();
                    assert!(decode_ccitt(&data, &p, rows).unwrap().len() < picture.len(), "{case}, a byte short");
                }
            }
        }
        // White rows, each the one code 1 (vertical, no change) and its fill.
        for end_of_line in [false, true] {
            let p = CcittParams { k: -1, columns: 64, end_of_line, encoded_byte_align: true, ..CcittParams::default() };
            assert_eq!(decode_ccitt(&[0x80; 16], &p, 16).unwrap(), [0xFF; 8 * 16], "Group 4, end_of_line {end_of_line}");
        }
    }

    proptest::proptest! {
        /// Any bitmap, of noise or of runs, decodes to its own bits, through an independent decoder
        /// and through `decode_ccitt`.
        #[test]
        fn bitmaps_round_trip(
            columns in 1u32..400,
            rows in 1u32..24,
            runs in proptest::collection::vec(0u8..=255, 1..48),
            noise in proptest::bool::ANY,
        ) {
            let data = bitmap(columns, rows, &runs, noise);
            let coded = encode_g4(&data, columns, rows).unwrap();
            proptest::prop_assert_eq!(decode(&coded, columns, rows), unpack(&data, columns, rows));
            let decoded = decode_ccitt(&coded, &g4(columns), rows).unwrap();
            proptest::prop_assert_eq!(unpack(&decoded, columns, rows), unpack(&data, columns, rows));
        }

        /// Whatever it is given, it returns.
        #[test]
        fn any_input_returns(
            data in proptest::collection::vec(0u8..=255, 0..600),
            columns in 0u32..40_000,
            rows in 0u32..80,
            k in -2i64..3,
            end_of_line in proptest::bool::ANY,
            encoded_byte_align in proptest::bool::ANY,
        ) {
            let _ = encode_g4(&data, columns, rows);
            let p = CcittParams { k, columns: i64::from(columns), end_of_line, encoded_byte_align, ..CcittParams::default() };
            if let Ok(out) = decode_ccitt(&data, &p, rows) {
                proptest::prop_assert!(out.len() <= (columns as usize).div_ceil(8) * rows as usize);
            }
        }
    }
}
