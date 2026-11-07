//! Predictor functions for FlateDecode and LZWDecode (ISO 32000-2 §7.4.4.4).
//!
//! - Predictor 2: TIFF 6.0 horizontal differencing (Section 14), per component, for
//!   1, 2, 4, 8 and 16 bits per component.
//! - Predictors 10–15: PNG filters (RFC 2083 §6): each row carries its own filter-type
//!   byte (0 None, 1 Sub, 2 Up, 3 Average, 4 Paeth), whatever the predictor value is.
//!
//! A final partial row is decoded as far as its data goes.

use crate::{Failure, FilterError, Params, Step};

struct Layout {
    bpc: usize,
    colors: usize,
    /// Bytes per complete pixel, at least 1 (the PNG `bpp`).
    bpp: usize,
    /// Bytes per row (without the PNG tag byte).
    row: usize,
    /// Samples per row (colors × columns).
    samples: usize,
}

fn layout(p: &Params) -> Result<Layout, String> {
    let bpc = match p.bits_per_component {
        b @ (1 | 2 | 4 | 8 | 16) => b as usize,
        b => return Err(format!("invalid BitsPerComponent {b}")),
    };
    if !(1..=255).contains(&p.colors) {
        return Err(format!("invalid Colors {}", p.colors));
    }
    if p.columns < 1 {
        return Err(format!("invalid Columns {}", p.columns));
    }
    let colors = p.colors as usize;
    let samples = usize::try_from(p.columns).ok().and_then(|c| c.checked_mul(colors)).ok_or_else(|| format!("Columns {} too large", p.columns))?;
    let bits = samples.checked_mul(bpc).ok_or_else(|| format!("Columns {} too large", p.columns))?;
    Ok(Layout { bpc, colors, bpp: (colors * bpc).div_ceil(8), row: bits.div_ceil(8), samples })
}

/// Undoes the predictor named by `p.predictor` (1 = identity).
pub(crate) fn decode(p: &Params, data: Vec<u8>, name: &'static str) -> Step {
    match p.predictor {
        1 => Ok(data),
        2 | 10..=15 => {
            let l = layout(p).map_err(|e| Failure::corrupt(name, e, Vec::new()))?;
            if p.predictor == 2 { Ok(tiff_decode(&l, data)) } else { png_decode(&l, &data, name) }
        }
        other => Err(Failure::corrupt(name, format!("invalid Predictor {other}"), Vec::new())),
    }
}

/// Applies the predictor named by `p.predictor` (1 = identity; 15 = per-row optimum).
pub(crate) fn encode(p: &Params, data: &[u8]) -> Result<Vec<u8>, FilterError> {
    match p.predictor {
        1 => Ok(data.to_vec()),
        2 | 10..=15 => {
            let l = layout(p).map_err(FilterError::Unsupported)?;
            Ok(if p.predictor == 2 { tiff_encode(&l, data) } else { png_encode(&l, data, p.predictor) })
        }
        other => Err(FilterError::Unsupported(format!("Predictor {other}"))),
    }
}

// ---- TIFF predictor 2 ----

fn get(row: &[u8], i: usize, bpc: usize) -> u16 {
    match bpc {
        16 => u16::from_be_bytes([row[2 * i], row[2 * i + 1]]),
        8 => u16::from(row[i]),
        _ => {
            let bit = i * bpc;
            let shift = 8 - bpc - bit % 8;
            u16::from(row[bit / 8] >> shift) & ((1 << bpc) - 1)
        }
    }
}

fn set(row: &mut [u8], i: usize, bpc: usize, v: u16) {
    match bpc {
        16 => row[2 * i..2 * i + 2].copy_from_slice(&v.to_be_bytes()),
        8 => row[i] = v as u8,
        _ => {
            let bit = i * bpc;
            let shift = 8 - bpc - bit % 8;
            let mask = (((1u16 << bpc) - 1) << shift) as u8;
            row[bit / 8] = (row[bit / 8] & !mask) | (((v << shift) as u8) & mask);
        }
    }
}

/// Complete samples available in a (possibly partial) row.
fn samples_in(l: &Layout, row_len: usize) -> usize {
    (row_len * 8 / l.bpc).min(l.samples)
}

fn tiff_decode(l: &Layout, mut data: Vec<u8>) -> Vec<u8> {
    let mask = if l.bpc == 16 { u16::MAX } else { (1u16 << l.bpc) - 1 };
    for row in data.chunks_mut(l.row) {
        for i in l.colors..samples_in(l, row.len()) {
            let v = get(row, i, l.bpc).wrapping_add(get(row, i - l.colors, l.bpc)) & mask;
            set(row, i, l.bpc, v);
        }
    }
    data
}

fn tiff_encode(l: &Layout, data: &[u8]) -> Vec<u8> {
    let mask = if l.bpc == 16 { u16::MAX } else { (1u16 << l.bpc) - 1 };
    let mut out = data.to_vec();
    for row in out.chunks_mut(l.row) {
        // Descending, so the left neighbour still holds its original value.
        for i in (l.colors..samples_in(l, row.len())).rev() {
            let v = get(row, i, l.bpc).wrapping_sub(get(row, i - l.colors, l.bpc)) & mask;
            set(row, i, l.bpc, v);
        }
    }
    out
}

// ---- PNG predictors ----

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// The PNG predictor value for byte `i` given the reconstructed current row `cur`
/// (bytes before `i`) and the prior row `up`.
fn predict(tag: u8, cur: &[u8], up: &[u8], i: usize, bpp: usize) -> u8 {
    let a = if i >= bpp { cur[i - bpp] } else { 0 };
    let b = up[i];
    let c = if i >= bpp { up[i - bpp] } else { 0 };
    match tag {
        1 => a,
        2 => b,
        3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
        4 => paeth(a, b, c),
        _ => 0,
    }
}

fn png_decode(l: &Layout, data: &[u8], name: &'static str) -> Step {
    let mut out = Vec::with_capacity(data.len());
    // The prior row: never longer than the input, even for absurd Columns values.
    let mut up = vec![0u8; l.row.min(data.len())];
    for chunk in data.chunks(l.row.saturating_add(1)) {
        let (tag, src) = (chunk[0], &chunk[1..]);
        if tag > 4 {
            return Err(Failure::corrupt(name, format!("invalid PNG filter type {tag}"), out));
        }
        let start = out.len();
        for (i, &x) in src.iter().enumerate() {
            let pred = predict(tag, &out[start..], &up, i, l.bpp);
            out.push(x.wrapping_add(pred));
        }
        up[..src.len()].copy_from_slice(&out[start..]);
    }
    Ok(out)
}

fn png_filter(tag: u8, cur: &[u8], up: &[u8], bpp: usize, dst: &mut Vec<u8>) {
    dst.clear();
    dst.push(tag);
    dst.extend(cur.iter().enumerate().map(|(i, &x)| x.wrapping_sub(predict(tag, cur, up, i, bpp))));
}

/// Sum of residuals as signed bytes: the usual heuristic for choosing a PNG filter.
fn cost(filtered: &[u8]) -> u64 {
    filtered[1..].iter().map(|&b| u64::from((b as i8).unsigned_abs())).sum()
}

fn png_encode(l: &Layout, data: &[u8], predictor: i64) -> Vec<u8> {
    let rows = data.len().div_ceil(l.row.max(1));
    let mut out = Vec::with_capacity(data.len() + rows);
    let zeros = vec![0u8; l.row.min(data.len())];
    let (mut best, mut trial) = (Vec::new(), Vec::new());
    let mut up: &[u8] = &zeros;
    for cur in data.chunks(l.row) {
        let up_row = &up[..cur.len()];
        if predictor == 15 {
            png_filter(0, cur, up_row, l.bpp, &mut best);
            let mut best_cost = cost(&best);
            for tag in 1..=4 {
                png_filter(tag, cur, up_row, l.bpp, &mut trial);
                let c = cost(&trial);
                if c < best_cost {
                    best_cost = c;
                    std::mem::swap(&mut best, &mut trial);
                }
            }
        } else {
            png_filter((predictor - 10) as u8, cur, up_row, l.bpp, &mut best);
        }
        out.extend_from_slice(&best);
        up = cur;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(predictor: i64, colors: i64, bpc: i64, columns: i64) -> Params {
        Params { predictor, colors, bits_per_component: bpc, columns, early_change: 1 }
    }

    fn dec(params: &Params, data: &[u8]) -> Result<Vec<u8>, FilterError> {
        decode(params, data.to_vec(), "FlateDecode").map_err(|f| f.error)
    }

    #[test]
    fn png_hand_vectors() {
        // 3 columns, 1 color, 8 bpc. Rows: None, Sub, Up, Average, Paeth.
        let enc = [
            0, 10, 20, 30, //  None  -> 10 20 30
            1, 5, 1, 1, //     Sub   -> 5 6 7
            2, 1, 1, 1, //     Up    -> 6 7 8
            3, 10, 10, 10, //  Avg   -> 10+3=13, 10+(13+7)/2=20, 10+(20+8)/2=24
            4, 1, 1,
            1, //     Paeth: a=0,b=13,c=0 -> p=13 -> b: 14; a=14,b=20,c=13: p=21 -> pa=7,pb=1,pc=8 -> b: 21;
               //                        a=21,b=24,c=20: p=25 -> pa=4,pb=1,pc=5 -> b: 25
        ];
        let out = dec(&p(15, 1, 8, 3), &enc).unwrap();
        assert_eq!(out, [10, 20, 30, 5, 6, 7, 6, 7, 8, 13, 20, 24, 14, 21, 25]);
    }

    #[test]
    fn paeth_tie_breaks() {
        // RFC 2083 order: a, then b, then c.
        assert_eq!(paeth(1, 1, 1), 1);
        assert_eq!(paeth(10, 20, 15), 15); // p=15: pa=5 pb=5 pc=0 -> c
        assert_eq!(paeth(20, 10, 10), 20); // p=20: pa=0 -> a
        assert_eq!(paeth(5, 9, 7), 7); //    p=7: pa=2 pb=2 pc=0 -> c
    }

    #[test]
    fn png_multibyte_pixels() {
        // colors=3 bpc=16 -> bpp=6; 1 column; Sub on the first pixel uses a=0 throughout.
        let enc = [1, 1, 2, 3, 4, 5, 6, 2, 1, 1, 1, 1, 1, 1];
        let out = dec(&p(11, 3, 16, 1), &enc).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 2, 3, 4, 5, 6, 7]);
        // colors=3 bpc=8, 2 columns: Sub looks 3 bytes back.
        let enc = [1, 1, 2, 3, 1, 1, 1];
        assert_eq!(dec(&p(11, 3, 8, 2), &enc).unwrap(), [1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn png_partial_final_row() {
        let enc = [0, 1, 2, 3, 4, 2, 1, 1];
        assert_eq!(dec(&p(12, 1, 8, 4), &enc).unwrap(), [1, 2, 3, 4, 2, 3]);
        // A lone tag byte at the end yields nothing.
        assert_eq!(dec(&p(12, 1, 8, 4), &[0, 1, 2, 3, 4, 2]).unwrap(), [1, 2, 3, 4]);
    }

    #[test]
    fn png_invalid_tag() {
        let r = decode(&p(10, 1, 8, 2), vec![0, 1, 2, 5, 1, 1], "FlateDecode");
        let f = r.err().unwrap();
        assert!(matches!(f.error, FilterError::Corrupt { .. }));
        assert_eq!(f.partial, [1, 2]);
    }

    #[test]
    fn tiff_8bit() {
        // 2 colors, 3 columns: differences per component.
        let enc = [10, 20, 1, 2, 3, 4, /* row 2 */ 5, 5, 250, 10, 1, 1];
        assert_eq!(dec(&p(2, 2, 8, 3), &enc).unwrap(), [10, 20, 11, 22, 14, 26, 5, 5, 255, 15, 0, 16]);
    }

    #[test]
    fn tiff_16bit() {
        let enc = [0x01, 0x00, 0xff, 0xff, 0x00, 0x02];
        assert_eq!(dec(&p(2, 1, 16, 3), &enc).unwrap(), [0x01, 0x00, 0x00, 0xff, 0x01, 0x01]);
    }

    #[test]
    fn tiff_sub_byte() {
        // 1 bpc, 1 color, 10 columns: bits 1 0 0 1 1 0 0 0 | 1 1 (pad 000000)
        // Cumulative XOR: 1 1 1 0 1 1 1 1 | 0 1
        let enc = [0b1001_1000, 0b1100_0000];
        assert_eq!(dec(&p(2, 1, 1, 10), &enc).unwrap(), [0b1110_1111, 0b0100_0000]);
        // 4 bpc, 1 color, 3 columns: samples 5, 15, 3 -> 5, 4, 7 (mod 16).
        assert_eq!(dec(&p(2, 1, 4, 3), &[0x5f, 0x30]).unwrap(), [0x54, 0x70]);
        // 2 bpc, 2 colors, 2 columns: samples 1 2 3 3 -> 1 2 0 1.
        assert_eq!(dec(&p(2, 2, 2, 2), &[0b0110_1111]).unwrap(), [0b0110_0001]);
        // Padding bits are left alone.
        assert_eq!(dec(&p(2, 1, 4, 1), &[0x5f]).unwrap(), [0x5f]);
    }

    #[test]
    fn tiff_partial_rows() {
        // 16-bit with an odd trailing byte: the lone byte is left as is.
        assert_eq!(dec(&p(2, 1, 16, 4), &[0, 1, 0, 1, 9]).unwrap(), [0, 1, 0, 2, 9]);
        assert_eq!(dec(&p(2, 1, 8, 4), &[1, 1, 1, 1, 2, 2]).unwrap(), [1, 2, 3, 4, 2, 4]);
    }

    #[test]
    fn invalid_params() {
        for bad in [p(2, 1, 3, 1), p(12, 0, 8, 1), p(12, 1, 8, 0), p(3, 1, 8, 1), p(12, 1, 8, i64::MAX), p(2, 300, 8, 1)] {
            assert!(matches!(dec(&bad, &[0, 1, 2]), Err(FilterError::Corrupt { .. })), "{bad:?}");
            assert!(encode(&bad, &[1, 2]).is_err());
        }
        // Huge but representable Columns does not allocate a huge row.
        assert_eq!(dec(&p(12, 1, 8, 1 << 40), &[2, 7, 8]).unwrap(), [7, 8]);
    }

    #[test]
    fn encode_choices() {
        let data = [1u8, 2, 3, 4, 5, 6];
        let up = encode(&p(12, 1, 8, 3), &data).unwrap();
        assert_eq!(up, [2, 1, 2, 3, 2, 3, 3, 3]);
        let paeth = encode(&p(14, 1, 8, 3), &data).unwrap();
        assert_eq!(paeth[0], 4);
        assert_eq!(paeth[4], 4);
        // Per-row optimum picks Sub for a ramp.
        let ramp: Vec<u8> = (0..64).collect();
        let opt = encode(&p(15, 1, 8, 64), &ramp).unwrap();
        assert_eq!(opt[0], 1);
        for pred in [10, 11, 12, 13, 14, 15] {
            let e = encode(&p(pred, 1, 8, 3), &data).unwrap();
            assert_eq!(dec(&p(pred, 1, 8, 3), &e).unwrap(), data);
        }
    }
}
