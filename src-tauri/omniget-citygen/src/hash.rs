//! Domain sub-seeds and canonical hashing. Never a global RNG in sequence:
//! every random choice is `sha256(seed | domain | id | n)`, so changing the
//! flowers never moves the streets.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// 32 bytes of `sha256(seed|domain|id|n)`.
pub fn sub_seed(seed: &str, domain: &str, id: &str, n: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    h.update(b"|");
    h.update(domain.as_bytes());
    h.update(b"|");
    h.update(id.as_bytes());
    h.update(b"|");
    h.update(n.to_le_bytes());
    h.finalize().into()
}

/// A u64 from a sub-seed.
pub fn pick_u64(seed: &str, domain: &str, id: &str, n: u64) -> u64 {
    let b = sub_seed(seed, domain, id, n);
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Uniform integer in `0..m` (m > 0).
pub fn pick(seed: &str, domain: &str, id: &str, n: u64, m: u64) -> u64 {
    if m == 0 {
        return 0;
    }
    pick_u64(seed, domain, id, n) % m
}

/// Hash of two integer coordinates plus a salt: the positional hash used by
/// terrain noise and jittered grids (Veloren's `RandomField`, Minecraft's
/// `random_spread`).
pub fn hash2(seed: u64, x: i32, y: i32, salt: u32) -> u32 {
    let mut v = seed
        ^ (x as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (salt as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    v ^= v >> 33;
    v = v.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    v ^= v >> 33;
    v = v.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    v ^= v >> 33;
    v as u32
}

/// A u64 seed from the charter's text seed.
pub fn seed_u64(seed: &str) -> u64 {
    pick_u64(seed, "root", "", 0)
}

/// `sha256:<hex>` of the canonical JSON of `value` (serde_json with struct
/// field order and `BTreeMap`s: stable across machines).
pub fn canonical_hash<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("serializable");
    let digest = Sha256::digest(&bytes);
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for b in digest {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
