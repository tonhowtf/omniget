//! The MQ arithmetic encoder of JBIG2 (ITU-T T.88 | ISO/IEC 14492, Annex E), written from the
//! procedures in its Figures E.2–E.12 and the probability estimates of Table E.1.

/// Table E.1: for each index, the LPS probability estimate Qe, the next index after an MPS and
/// after an LPS renormalisation, and whether an LPS switches the MPS sense.
const STATES: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true),
    (0x3401, 2, 6, false),
    (0x1801, 3, 9, false),
    (0x0AC1, 4, 12, false),
    (0x0521, 5, 29, false),
    (0x0221, 38, 33, false),
    (0x5601, 7, 6, true),
    (0x5401, 8, 14, false),
    (0x4801, 9, 14, false),
    (0x3801, 10, 14, false),
    (0x3001, 11, 17, false),
    (0x2401, 12, 18, false),
    (0x1C01, 13, 20, false),
    (0x1601, 29, 21, false),
    (0x5601, 15, 14, true),
    (0x5401, 16, 14, false),
    (0x5101, 17, 15, false),
    (0x4801, 18, 16, false),
    (0x3801, 19, 17, false),
    (0x3401, 20, 18, false),
    (0x3001, 21, 19, false),
    (0x2801, 22, 19, false),
    (0x2401, 23, 20, false),
    (0x2201, 24, 21, false),
    (0x1C01, 25, 22, false),
    (0x1801, 26, 23, false),
    (0x1601, 27, 24, false),
    (0x1401, 28, 25, false),
    (0x1201, 29, 26, false),
    (0x1101, 30, 27, false),
    (0x0AC1, 31, 28, false),
    (0x09C1, 32, 29, false),
    (0x08A1, 33, 30, false),
    (0x0521, 34, 31, false),
    (0x0441, 35, 32, false),
    (0x02A1, 36, 33, false),
    (0x0221, 37, 34, false),
    (0x0141, 38, 35, false),
    (0x0111, 39, 36, false),
    (0x0085, 40, 37, false),
    (0x0049, 41, 38, false),
    (0x0025, 42, 39, false),
    (0x0015, 43, 40, false),
    (0x0009, 44, 41, false),
    (0x0005, 45, 42, false),
    (0x0001, 45, 43, false),
    (0x5601, 46, 46, false),
];

#[cfg(test)]
thread_local! {
    /// Decisions coded on this thread.
    pub(crate) static DECISIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A context's adaptive state: the index into [`STATES`] (low 7 bits) and the MPS sense (bit 7).
/// 0, the initial value, is index 0 with MPS 0 (E.2.8).
pub(crate) type Context = u8;

/// The encoder's registers (E.2.1) and the bytes written so far.
pub(crate) struct MqEncoder {
    a: u32,
    c: u32,
    ct: u32,
    /// The output, after the byte before the first one (B at INITENC), which is 0x00 and is
    /// dropped by [`MqEncoder::finish`]: a carry can't reach it before the first byte is out.
    out: Vec<u8>,
}

impl MqEncoder {
    /// INITENC (Figure E.10). The byte before the first is 0x00, not 0xFF, so CT starts at 12.
    pub(crate) fn new() -> Self {
        MqEncoder { a: 0x8000, c: 0, ct: 12, out: vec![0] }
    }

    /// The byte B, the last one written (or the one before the first).
    fn b(&self) -> u8 {
        self.out.last().copied().unwrap_or(0)
    }

    /// ENCODE (Figures E.3–E.7): code decision `d` in context `cx`, updating its state.
    pub(crate) fn encode(&mut self, cx: &mut Context, d: bool) {
        #[cfg(test)]
        DECISIONS.with(|n| n.set(n.get() + 1));
        let (index, mps) = (usize::from(*cx & 0x7F), *cx >> 7 == 1);
        let Some(&(qe, nmps, nlps, switch)) = STATES.get(index) else { return };
        self.a -= qe;
        if d == mps {
            // CODEMPS.
            if self.a & 0x8000 != 0 {
                self.c += qe;
                return;
            }
            if self.a < qe {
                self.a = qe;
            } else {
                self.c += qe;
            }
            *cx = nmps | (*cx & 0x80);
        } else {
            // CODELPS.
            if self.a < qe {
                self.c += qe;
            } else {
                self.a = qe;
            }
            let mps = mps != switch;
            *cx = nlps | (u8::from(mps) << 7);
        }
        self.renorm();
    }

    /// RENORME (Figure E.8).
    fn renorm(&mut self) {
        loop {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                self.byte_out();
            }
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    /// BYTEOUT (Figure E.9), with bit stuffing after 0xFF and carry into the byte before.
    fn byte_out(&mut self) {
        if self.b() == 0xFF {
            self.stuffed();
        } else if self.c < 0x800_0000 {
            self.plain();
        } else {
            let b = self.b().wrapping_add(1);
            if let Some(last) = self.out.last_mut() {
                *last = b;
            }
            if b == 0xFF {
                self.c &= 0x7FF_FFFF;
                self.stuffed();
            } else {
                self.plain();
            }
        }
    }

    fn plain(&mut self) {
        self.out.push(((self.c >> 19) & 0xFF) as u8);
        self.c &= 0x7_FFFF;
        self.ct = 8;
    }

    fn stuffed(&mut self) {
        self.out.push(((self.c >> 20) & 0xFF) as u8);
        self.c &= 0xF_FFFF;
        self.ct = 7;
    }

    /// FLUSH (Figures E.11 and E.12): the final bytes, then the 0xFF 0xAC marker.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        let top = self.c + self.a;
        self.c |= 0xFFFF;
        if self.c >= top {
            self.c -= 0x8000;
        }
        self.c <<= self.ct;
        self.byte_out();
        self.c <<= self.ct;
        self.byte_out();
        if self.b() != 0xFF {
            self.out.push(0xFF);
        }
        self.out.push(0xAC);
        self.out.split_off(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Annex H.2's test sequence: 256 decisions in one context give these 30 bytes.
    #[test]
    fn the_annex_h_test_sequence_encodes_to_its_bytes() {
        let decisions = [
            0x00, 0x02, 0x00, 0x51, 0x00, 0x00, 0x00, 0xC0, 0x03, 0x52, 0x87, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x82, 0xC0, 0x20, 0x00, 0xFC, 0xD7, 0x9E,
            0xF6, 0xBF, 0x7F, 0xED, 0x90, 0x4F, 0x46, 0xA3, 0xBF,
        ];
        let mut mq = MqEncoder::new();
        let mut cx = 0;
        for byte in decisions {
            for bit in (0..8).rev() {
                mq.encode(&mut cx, byte >> bit & 1 == 1);
            }
        }
        let expected = [
            0x84, 0xC7, 0x3B, 0xFC, 0xE1, 0xA1, 0x43, 0x04, 0x02, 0x20, 0x00, 0x00, 0x41, 0x0D, 0xBB, 0x86, 0xF4, 0x31, 0x7F, 0xFF, 0x88, 0xFF, 0x37,
            0x47, 0x1A, 0xDB, 0x6A, 0xDF, 0xFF, 0xAC,
        ];
        assert_eq!(mq.finish(), expected);
    }
}
