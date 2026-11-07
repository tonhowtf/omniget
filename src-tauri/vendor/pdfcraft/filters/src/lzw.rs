//! LZWDecode (ISO 32000-2 §7.4.4.2): variable-width codes of 9–12 bits, MSB first,
//! 256 = clear-table, 257 = EOD, first free code 258. With `EarlyChange` 1 (the
//! default) the code width grows one code early.
//!
//! Tolerated: data ending without EOD, a missing initial clear-table code, a table that
//! fills up without a clear-table code (entries are then no longer added).

use std::collections::HashMap;

use crate::{Failure, Step};

const NAME: &str = "LZWDecode";
const CLEAR: u16 = 256;
const EOD: u16 = 257;
const FIRST: usize = 258;
const TABLE: usize = 4096;
/// The encoder emits clear-table when its next free code reaches this value.
const ENCODER_RESET_AT: usize = TABLE;

/// The code width a decoder uses when its next free code is `next`.
fn width(next: usize, early: bool) -> u32 {
    match next + usize::from(early) {
        0..512 => 9,
        512..1024 => 10,
        1024..2048 => 11,
        _ => 12,
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u32,
    bits: u32,
}

impl BitReader<'_> {
    fn read(&mut self, n: u32) -> Option<u16> {
        while self.bits < n {
            let &b = self.data.get(self.pos)?;
            self.pos += 1;
            self.acc = (self.acc << 8) | u32::from(b);
            self.bits += 8;
        }
        self.bits -= n;
        let v = (self.acc >> self.bits) & ((1 << n) - 1);
        self.acc &= (1 << self.bits) - 1;
        Some(v as u16)
    }
}

pub(crate) fn decode(data: &[u8], early: bool, max: usize) -> Step {
    // Table entry = (prefix code, last byte, first byte, length).
    let mut prefix = vec![0u16; TABLE];
    let mut last = vec![0u8; TABLE];
    let mut first = vec![0u8; TABLE];
    let mut len = vec![0u16; TABLE];
    for i in 0..256 {
        last[i] = i as u8;
        first[i] = i as u8;
        len[i] = 1;
    }
    let mut next = FIRST;
    let mut w = 9;
    let mut prev: Option<usize> = None;
    let mut r = BitReader { data, pos: 0, acc: 0, bits: 0 };
    let mut out = Vec::with_capacity(data.len().saturating_mul(2).min(max));

    while let Some(code) = r.read(w) {
        match code {
            CLEAR => {
                next = FIRST;
                w = 9;
                prev = None;
                continue;
            }
            EOD => break,
            _ => {}
        }
        let code = usize::from(code);
        if let Some(p) = prev {
            let head = if code < next {
                first[code]
            } else if code == next && next < TABLE {
                first[p]
            } else {
                return Err(Failure::corrupt(NAME, format!("code {code} not in table (next {next})"), out));
            };
            if next < TABLE {
                prefix[next] = p as u16;
                last[next] = head;
                first[next] = first[p];
                len[next] = len[p] + 1;
                next += 1;
            }
        } else if code >= 256 {
            return Err(Failure::corrupt(NAME, format!("code {code} before any table entry"), out));
        }
        // Emit the string for `code`, walking the prefix chain backwards.
        let n = usize::from(len[code]);
        if out.len() + n > max {
            return Err(Failure::limit(max));
        }
        let start = out.len();
        out.resize(start + n, 0);
        let mut c = code;
        for slot in out[start..].iter_mut().rev() {
            *slot = last[c];
            c = usize::from(prefix[c]);
        }
        prev = Some(code);
        w = width(next, early);
    }
    Ok(out)
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    bits: u32,
}

impl BitWriter {
    fn write(&mut self, code: u16, n: u32) {
        self.acc = (self.acc << n) | u32::from(code);
        self.bits += n;
        while self.bits >= 8 {
            self.bits -= 8;
            self.out.push((self.acc >> self.bits) as u8);
        }
        self.acc &= (1 << self.bits) - 1;
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits > 0 {
            self.out.push((self.acc << (8 - self.bits)) as u8);
        }
        self.out
    }
}

/// Greedy LZW with a leading clear-table code and a trailing EOD.
pub(crate) fn encode(data: &[u8], early: bool) -> Vec<u8> {
    let mut wr = BitWriter { out: Vec::with_capacity(data.len() / 2 + 8), acc: 0, bits: 0 };
    let mut dict: HashMap<(u16, u8), u16> = HashMap::new();
    // `next` is the encoder's next free code; `dnext` mirrors the decoder's, which lags
    // one entry behind (the decoder adds an entry only once it sees the following code).
    let mut next = FIRST;
    let mut dnext = FIRST;
    let mut codes_since_clear = 0usize;
    wr.write(CLEAR, 9);
    let emit = |wr: &mut BitWriter, code: u16, dnext: &mut usize, since: &mut usize| {
        wr.write(code, width(*dnext, early));
        if *since > 0 && *dnext < TABLE {
            *dnext += 1;
        }
        *since += 1;
    };
    let Some((&b0, rest)) = data.split_first() else {
        wr.write(EOD, 9);
        return wr.finish();
    };
    let mut cur = u16::from(b0);
    for &b in rest {
        if let Some(&c) = dict.get(&(cur, b)) {
            cur = c;
            continue;
        }
        emit(&mut wr, cur, &mut dnext, &mut codes_since_clear);
        dict.insert((cur, b), next as u16);
        next += 1;
        if next == ENCODER_RESET_AT {
            wr.write(CLEAR, width(dnext, early));
            dict.clear();
            next = FIRST;
            dnext = FIRST;
            codes_since_clear = 0;
        }
        cur = u16::from(b);
    }
    emit(&mut wr, cur, &mut dnext, &mut codes_since_clear);
    wr.write(EOD, width(dnext, early));
    wr.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FilterError;

    fn dec(d: &[u8], early: bool) -> Result<Vec<u8>, FilterError> {
        decode(d, early, 1 << 26).map_err(|f| f.error)
    }

    /// ISO 32000-2 §7.4.4.2 example: codes 256 45 258 258 65 259 66 257.
    const SPEC_IN: [u8; 10] = [45, 45, 45, 45, 45, 65, 45, 45, 45, 66];
    const SPEC_OUT: [u8; 9] = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];

    #[test]
    fn spec_example() {
        assert_eq!(dec(&SPEC_OUT, true).unwrap(), SPEC_IN);
        assert_eq!(encode(&SPEC_IN, true), SPEC_OUT);
        // Below 511 codes the early-change setting makes no difference.
        assert_eq!(dec(&SPEC_OUT, false).unwrap(), SPEC_IN);
    }

    #[test]
    fn missing_eod_and_clear() {
        // Drop the EOD (last 9 bits span the final 2 bytes; keep bits for 66 only).
        let mut w = BitWriter { out: vec![], acc: 0, bits: 0 };
        for c in [45u16, 258, 258, 65, 259, 66] {
            w.write(c, 9);
        }
        assert_eq!(dec(&w.finish(), true).unwrap(), SPEC_IN);
    }

    #[test]
    fn invalid_codes() {
        let mut w = BitWriter { out: vec![], acc: 0, bits: 0 };
        for c in [256u16, 65, 300] {
            w.write(c, 9);
        }
        let f = decode(&w.finish(), true, 1000).err().unwrap();
        assert!(matches!(f.error, FilterError::Corrupt { .. }));
        assert_eq!(f.partial, b"A");
        let mut w = BitWriter { out: vec![], acc: 0, bits: 0 };
        w.write(258, 9);
        assert!(matches!(dec(&w.finish(), true), Err(FilterError::Corrupt { .. })));
    }

    #[test]
    fn widths() {
        assert_eq!((width(510, true), width(511, true), width(511, false), width(512, false)), (9, 10, 9, 10));
        assert_eq!((width(1022, true), width(1023, true), width(2047, true), width(4095, false)), (10, 11, 12, 12));
    }

    fn noisy(n: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((s >> 16) % 7) as u8 * 31
            })
            .collect()
    }

    #[test]
    fn all_widths_and_clears_round_trip() {
        for early in [false, true] {
            for data in [noisy(200_000, 1), (0..=255u8).cycle().take(70_000).collect(), vec![9u8; 100_000]] {
                let e = encode(&data, early);
                assert_eq!(dec(&e, early).unwrap(), data);
            }
        }
    }

    #[test]
    fn cross_check_with_weezl() {
        use weezl::BitOrder::Msb;
        for data in [noisy(150_000, 7), SPEC_IN.to_vec(), vec![]] {
            // early change 1 is the TIFF size switch; 0 is the plain variant.
            let tiff = weezl::encode::Encoder::with_tiff_size_switch(Msb, 8).encode(&data).unwrap();
            assert_eq!(dec(&tiff, true).unwrap(), data);
            let plain = weezl::encode::Encoder::new(Msb, 8).encode(&data).unwrap();
            assert_eq!(dec(&plain, false).unwrap(), data);
            let ours = encode(&data, true);
            assert_eq!(weezl::decode::Decoder::with_tiff_size_switch(Msb, 8).decode(&ours).unwrap(), data);
            let ours = encode(&data, false);
            assert_eq!(weezl::decode::Decoder::new(Msb, 8).decode(&ours).unwrap(), data);
        }
    }

    #[test]
    fn full_table_without_clear() {
        // Hand-roll a stream that fills the table and keeps going at 12 bits.
        let data = noisy(40_000, 3);
        let mut w = BitWriter { out: vec![], acc: 0, bits: 0 };
        let mut dict: HashMap<Vec<u8>, u16> = (0..=255u8).map(|b| (vec![b], u16::from(b))).collect();
        let (mut next, mut dnext, mut since) = (FIRST, FIRST, 0usize);
        let mut cur: Vec<u8> = vec![];
        let mut put = |w: &mut BitWriter, code: u16, dnext: &mut usize| {
            w.write(code, width(*dnext, true));
            if since > 0 && *dnext < TABLE {
                *dnext += 1;
            }
            since += 1;
        };
        w.write(CLEAR, 9);
        for &b in &data {
            let mut ext = cur.clone();
            ext.push(b);
            if dict.contains_key(&ext) {
                cur = ext;
                continue;
            }
            put(&mut w, dict[&cur], &mut dnext);
            if next < TABLE {
                dict.insert(ext, next as u16);
                next += 1;
            }
            cur = vec![b];
        }
        put(&mut w, dict[&cur], &mut dnext);
        w.write(EOD, width(dnext, true));
        assert_eq!(dec(&w.finish(), true).unwrap(), data);
    }

    #[test]
    fn limit() {
        let e = encode(&vec![0u8; 100_000], true);
        assert!(matches!(decode(&e, true, 50_000).map_err(|f| f.error), Err(FilterError::LimitExceeded(50_000))));
        assert_eq!(decode(&e, true, 100_000).ok().unwrap().len(), 100_000);
    }
}
