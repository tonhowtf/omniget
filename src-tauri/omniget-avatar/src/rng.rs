//! Small deterministic PRNG (SplitMix64). Same seed, same sequence on every
//! platform, which is what genomes and residents need.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    /// Uniform integer in [0, n).
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        (((self.next_u64() >> 32) * n as u64) >> 32) as u32
    }

    /// Roughly normal (sum of three uniforms), clamped to [0, 1].
    pub fn bell(&mut self, mean: f32, spread: f32) -> f32 {
        let s = self.f32() + self.f32() + self.f32() - 1.5;
        (mean + s * spread * 2.0).clamp(0.0, 1.0)
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u32) as usize]
    }
}

/// Seed from text (city ids and the like), FNV-1a 64.
pub fn seed_of(text: &str) -> u64 {
    crate::catalog::fnv1a64(text.as_bytes())
}
