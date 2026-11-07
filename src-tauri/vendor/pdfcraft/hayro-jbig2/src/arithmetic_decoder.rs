// Note: This is a copy of the arithmetic decoder from `hayro-jpeg2000`,
// keep the two in sync.

//! The arithmetic decoder, described in Annex C.
//!
//! The arithmetic decoder keeps track of some state and continuously receives
//! context labels as input, each time yielding a new bit from the original data
//! as output.

pub(crate) struct ArithmeticDecoder<'a> {
    /// The underlying encoded data.
    data: &'a [u8],
    /// The C-register (see Table C.1).
    c: u32,
    /// The A-register (see Table C.1).
    a: u32,
    /// The pointer to the current byte.
    base_pointer: u32,
    /// The bit shift counter.
    shift_count: u32,
}

impl<'a> ArithmeticDecoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut decoder = ArithmeticDecoder {
            data,
            c: 0,
            a: 0,
            base_pointer: 0,
            shift_count: 0,
        };

        decoder.initialize();

        decoder
    }

    /// Read the next bit using the given context label.
    #[inline(always)]
    pub(crate) fn read_bit(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        self.decode(context)
    }

    /// The INITDEC procedure from C.3.5.
    ///
    /// We use the version from Annex G in <https://www.itu.int/rec/T-REC-T.88-201808-I>.
    fn initialize(&mut self) {
        self.c = ((self.current_byte() as u32) ^ 0xff) << 16;
        self.read_byte();

        self.c <<= 7;
        self.shift_count -= 7;
        self.a = 0x8000;
    }

    /// The BYTEIN procedure from C.3.4.
    ///
    /// We use the version from Annex G from <https://www.itu.int/rec/T-REC-T.88-201808-I>.
    #[inline(always)]
    fn read_byte(&mut self) {
        if self.current_byte() == 0xff {
            let b1 = self.next_byte();

            if b1 > 0x8f {
                self.shift_count = 8;
            } else {
                self.base_pointer += 1;
                self.c = self
                    .c
                    .wrapping_add(0xfe00)
                    .wrapping_sub((self.current_byte() as u32) << 9);
                self.shift_count = 7;
            }
        } else {
            self.base_pointer += 1;
            self.c = self
                .c
                .wrapping_add(0xff00)
                .wrapping_sub((self.current_byte() as u32) << 8);
            self.shift_count = 8;
        }
    }

    /// The RENORMD procedure from C.3.3.
    #[inline(always)]
    fn renormalize(&mut self) {
        // Original code:
        // loop {
        //     if self.shift_count == 0 {
        //         self.read_byte();
        //     }
        //
        //     self.a <<= 1;
        //     self.c <<= 1;
        //     self.shift_count -= 1;
        //
        //     if self.a & 0x8000 != 0 {
        //         break;
        //     }
        // }

        // Optimization: Batch shifts.
        while self.a & 0x8000 == 0 {
            if self.shift_count == 0 {
                self.read_byte();
            }

            let shifts_needed = self.a.leading_zeros() - 16;
            let batch = shifts_needed.min(self.shift_count);
            self.a <<= batch;
            self.c <<= batch;
            self.shift_count -= batch;
        }
    }

    /// The `LPS_EXCHANGE` procedure from C.3.2.
    #[inline(always)]
    #[allow(unused, reason = "for reference")]
    fn exchange_lps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        // Original code:
        // let d;
        //
        // if self.a < qe_entry.qe {
        //     self.a = qe_entry.qe;
        //     d = context.mps;
        //     context.index = qe_entry.nmps;
        // } else {
        //     self.a = qe_entry.qe;
        //     d = 1 - context.mps;
        //
        //     if qe_entry.switch {
        //         context.mps = 1 - context.mps;
        //     }
        //
        //     context.index = qe_entry.nlps;
        // }

        // Branchless version, shows better performance.

        let cond = (self.a < qe_entry.qe) as u32;
        let inv_cond = 1 - cond;

        self.a = qe_entry.qe;
        // d = if cond { mps } else { 1 - mps }
        let d = context.mps() ^ inv_cond;
        // flip mps only when !cond && switch
        context.xor_mps(inv_cond & (qe_entry.switch as u32));
        // index = if cond { nmps } else { nlps }
        let cond_u8 = cond as u8;
        let inv_cond_u8 = inv_cond as u8;
        context.set_index(cond_u8 * qe_entry.nmps + inv_cond_u8 * qe_entry.nlps);

        d
    }

    /// The `MPS_EXCHANGE` procedure from C.3.2.
    #[inline(always)]
    #[allow(unused, reason = "for reference")]
    fn exchange_mps(&mut self, context: &mut ArithmeticDecoderContext, qe_entry: &QeData) -> u32 {
        // Original code:
        //  let d;
        //
        //  if self.a < qe_entry.qe {
        //      d = 1 - context.mps;
        //
        //      if qe_entry.switch {
        //          context.mps = 1 - context.mps;
        //      }
        //
        //      context.index = qe_entry.nlps;
        //  } else {
        //      d = context.mps;
        //      context.index = qe_entry.nmps;
        //  }

        // Branchless version, shows better performance.
        let cond = (self.a < qe_entry.qe) as u32;
        let inv_cond = 1 - cond;
        // d = if cond { 1 - mps } else { mps }
        let d = context.mps() ^ cond;
        // flip mps only when cond && switch
        context.xor_mps(cond & (qe_entry.switch as u32));
        // index = if cond { nlps } else { nmps }
        let cond_u8 = cond as u8;
        let inv_cond_u8 = inv_cond as u8;
        context.set_index(cond_u8 * qe_entry.nlps + inv_cond_u8 * qe_entry.nmps);
        d
    }

    /// The DECODE procedure from C.3.2.
    ///
    /// We use the version from Annex G from <https://www.itu.int/rec/T-REC-T.88-201808-I>.
    #[inline(always)]
    pub(crate) fn decode(&mut self, context: &mut ArithmeticDecoderContext) -> u32 {
        let qe_entry = &QE_TABLE[context.index() as usize];

        self.a -= qe_entry.qe;

        // Old code:
        // let d;
        //
        // if (self.c >> 16) < self.a {
        //     if self.a & 0x8000 == 0 {
        //         d = self.exchange_mps(context, qe_entry);
        //         self.renormalize();
        //     } else {
        //         d = context.mps();
        //     }
        // } else {
        //     self.c -= self.a << 16;
        //
        //     d = self.exchange_lps(context, qe_entry);
        //     self.renormalize();
        // }
        //
        // d

        // This is a faster version that reduces branching, which has shown
        // itself to be the main limiting factor for better performance.
        // We short-circuit the case where just the most probably symbol is
        // returned, and otherwise use a code path that works for both,
        // MPS_EXCHANGE and LPS_EXCHANGE.

        if (self.c >> 16) < self.a && self.a & 0x8000 != 0 {
            return context.mps();
        }

        // Unified branchless MPS_EXCHANGE / LPS_EXCHANGE.
        //
        // Compare with exchange_mps and exchange_lps above — the only
        // difference between them is that LPS flips the role of cond:
        //   exchange_mps: d = mps ^ cond,       flip when cond,      index = cond*nlps + inv*nmps
        //   exchange_lps: d = mps ^ inv_cond,   flip when inv_cond,  index = cond*nmps + inv*nlps
        //
        // This is equivalent to XOR-ing cond with is_lps, so we can handle
        // both paths with a single branchless computation.
        //
        // As can be seen above, renormalization is always performed.
        let is_lps = ((self.c >> 16) >= self.a) as u32;

        // LPS: C -= A << 16 (no-op when MPS).
        let lps_mask = is_lps.wrapping_neg(); // 0xFFFF_FFFF if LPS, 0 if MPS
        self.c -= (self.a << 16) & lps_mask;

        // Same condition as in exchange_mps / exchange_lps.
        let cond = (self.a < qe_entry.qe) as u32;

        // LPS: a = qe (no-op when MPS, a stays as a - qe).
        self.a = (self.a & !lps_mask) | (qe_entry.qe & lps_mask);

        // exchange_mps: d = mps ^ cond       →  cond ^ 0
        // exchange_lps: d = mps ^ inv_cond   →  cond ^ 1
        // unified:      d = mps ^ (cond ^ is_lps)
        let d = context.mps() ^ cond ^ is_lps;

        // exchange_mps: flip mps when cond & switch       →  (cond ^ 0) & switch
        // exchange_lps: flip mps when inv_cond & switch   →  (cond ^ 1) & switch
        // unified:      flip mps when (cond ^ is_lps) & switch
        context.xor_mps((cond ^ is_lps) & (qe_entry.switch as u32));

        // exchange_mps: index = cond * nlps + inv_cond * nmps
        // exchange_lps: index = cond * nmps + inv_cond * nlps  (swapped)
        // unified: the result is always exactly nmps or nlps —
        //          pick nlps when (cond ^ is_lps) == 1, nmps otherwise.
        let pick_nlps = ((cond ^ is_lps) as u8).wrapping_neg(); // 0xFF or 0x00
        context.set_index(qe_entry.nmps ^ ((qe_entry.nmps ^ qe_entry.nlps) & pick_nlps));

        self.renormalize();

        d
    }

    #[inline(always)]
    fn current_byte(&self) -> u8 {
        self.data
            .get(self.base_pointer as usize)
            .copied()
            // "The number of bytes corresponding to the coding passes is
            // specified in the packet header. Often at that point there are
            // more symbols to be decoded. Therefore, the decoder shall extend
            // the input bit stream to the arithmetic coder with 0xFF bytes,
            // as necessary, until all symbols have been decoded."
            .unwrap_or(0xFF)
    }

    #[inline(always)]
    fn next_byte(&self) -> u8 {
        self.data
            .get((self.base_pointer + 1) as usize)
            .copied()
            .unwrap_or(0xFF)
    }
}

// Previously, we stored the context as 2 u32's, but doing it with a bit-packed
// u8 seems to be slightly better (though it doesn't make that huge of a
// difference).
/// Bits 0-6 = index (0-46).
/// Bit 7 = mps (0 or 1).
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct ArithmeticDecoderContext(u8);

impl ArithmeticDecoderContext {
    #[inline(always)]
    pub(crate) fn index(self) -> u32 {
        (self.0 & 0x7F) as u32
    }

    #[inline(always)]
    pub(crate) fn mps(self) -> u32 {
        (self.0 >> 7) as u32
    }

    #[inline(always)]
    fn set_index(&mut self, index: u8) {
        self.0 = (self.0 & 0x80) | index;
    }

    #[inline(always)]
    fn xor_mps(&mut self, val: u32) {
        self.0 ^= ((val & 1) << 7) as u8;
    }
}

#[derive(Debug, Clone, Copy)]
struct QeData {
    qe: u32,
    nmps: u8,
    nlps: u8,
    switch: bool,
}

macro_rules! qe {
    ($($qe:expr, $nmps:expr, $nlps:expr, $switch:expr),+ $(,)?) => {
        [
            $(
                QeData {
                    qe: $qe,
                    nmps: $nmps,
                    nlps: $nlps,
                    switch: $switch,
                }
            ),+
        ]
    };
}

/// QE values and associated data from Table C.2.
#[rustfmt::skip]
static QE_TABLE: [QeData; 47] = qe!(
    0x5601, 1, 1, true,
    0x3401, 2, 6, false,
    0x1801, 3, 9, false,
    0x0AC1, 4, 12, false,
    0x0521, 5, 29, false,
    0x0221, 38, 33, false,
    0x5601, 7, 6, true,
    0x5401, 8, 14, false,
    0x4801, 9, 14, false,
    0x3801, 10, 14, false,
    0x3001, 11, 17, false,
    0x2401, 12, 18, false,
    0x1C01, 13, 20, false,
    0x1601, 29, 21, false,
    0x5601, 15, 14, true,
    0x5401, 16, 14, false,
    0x5101, 17, 15, false,
    0x4801, 18, 16, false,
    0x3801, 19, 17, false,
    0x3401, 20, 18, false,
    0x3001, 21, 19, false,
    0x2801, 22, 19, false,
    0x2401, 23, 20, false,
    0x2201, 24, 21, false,
    0x1C01, 25, 22, false,
    0x1801, 26, 23, false,
    0x1601, 27, 24, false,
    0x1401, 28, 25, false,
    0x1201, 29, 26, false,
    0x1101, 30, 27, false,
    0x0AC1, 31, 28, false,
    0x09C1, 32, 29, false,
    0x08A1, 33, 30, false,
    0x0521, 34, 31, false,
    0x0441, 35, 32, false,
    0x02A1, 36, 33, false,
    0x0221, 37, 34, false,
    0x0141, 38, 35, false,
    0x0111, 39, 36, false,
    0x0085, 40, 37, false,
    0x0049, 41, 38, false,
    0x0025, 42, 39, false,
    0x0015, 43, 40, false,
    0x0009, 44, 41, false,
    0x0005, 45, 42, false,
    0x0001, 45, 43, false,
    0x5601, 46, 46, false,
);
