//! Seeded integer hashing for `rand()` and jitter. No floats go in, so the
//! same seed gives the same numbers on every platform.

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn hash3(seed: u64, k: (i64, i64, i64)) -> u64 {
    let a = mix(seed ^ k.0 as u64);
    let b = mix(a ^ (k.1 as u64).rotate_left(21));
    mix(b ^ (k.2 as u64).rotate_left(42))
}

/// `[0, 1)` from the top 53 bits.
pub fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

pub fn seed_of(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    mix(h)
}
