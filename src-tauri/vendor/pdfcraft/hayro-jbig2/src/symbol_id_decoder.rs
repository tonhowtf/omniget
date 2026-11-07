//! Symbol ID (IAID) decoder (A.3).

use alloc::vec;
use alloc::vec::Vec;

use crate::arithmetic_decoder::{ArithmeticDecoder, ArithmeticDecoderContext};

pub(crate) struct SymbolIdDecoder {
    contexts: Vec<ArithmeticDecoderContext>,
    code_len: u32,
}

impl SymbolIdDecoder {
    pub(crate) fn new(code_len: u32) -> Self {
        // A.3: "The number of contexts required is 2^SBSYMCODELEN, which is less
        // than twice the maximum symbol ID."
        let num_contexts = 1_usize << code_len;

        Self {
            contexts: vec![ArithmeticDecoderContext::default(); num_contexts],
            code_len,
        }
    }

    #[inline(always)]
    pub(crate) fn decode(&mut self, decoder: &mut ArithmeticDecoder<'_>) -> u32 {
        let mut prev = 1_u32;

        for _ in 0..self.code_len {
            let ctx_mask = (1_u32 << (self.code_len + 1)) - 1;
            let ctx_idx = (prev & ctx_mask) as usize;
            let d = decoder.read_bit(&mut self.contexts[ctx_idx]);

            prev = (prev << 1) | d;
        }

        prev -= 1 << self.code_len;
        prev
    }
}
