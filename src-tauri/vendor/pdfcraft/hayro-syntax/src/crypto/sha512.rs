//! Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/calculate_sha_other.js>.

use alloc::vec;

pub(super) const K: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

#[inline]
const fn rotr(x: u64, n: u32) -> u64 {
    x.rotate_right(n)
}

#[inline]
pub(super) const fn ch(x: u64, y: u64, z: u64) -> u64 {
    (x & y) ^ (!x & z)
}

#[inline]
pub(super) const fn maj(x: u64, y: u64, z: u64) -> u64 {
    (x & y) ^ (x & z) ^ (y & z)
}

#[inline]
pub(super) const fn sigma_0(x: u64) -> u64 {
    rotr(x, 28) ^ rotr(x, 34) ^ rotr(x, 39)
}

#[inline]
pub(super) const fn sigma_1(x: u64) -> u64 {
    rotr(x, 14) ^ rotr(x, 18) ^ rotr(x, 41)
}

#[inline]
pub(super) const fn little_sigma_0(x: u64) -> u64 {
    rotr(x, 1) ^ rotr(x, 8) ^ (x >> 7)
}

#[inline]
pub(super) const fn little_sigma_1(x: u64) -> u64 {
    rotr(x, 19) ^ rotr(x, 61) ^ (x >> 6)
}

pub(super) fn calculate_with_initial_values(data: &[u8], h: &mut [u64; 8]) {
    let bit_len = data.len() as u128 * 8;
    let padded_len = (data.len() + 17).div_ceil(128) * 128; // Round up to nearest multiple of 128
    let mut padded = vec![0_u8; padded_len];

    padded[..data.len()].copy_from_slice(data);

    padded[data.len()] = 0x80;

    let len_bytes = bit_len.to_be_bytes();
    padded[padded_len - 16..].copy_from_slice(&len_bytes);

    for chunk in padded.chunks_exact(128) {
        let mut w = [0_u64; 80];

        for (i, word_bytes) in chunk.chunks_exact(8).enumerate() {
            w[i] = u64::from_be_bytes([
                word_bytes[0],
                word_bytes[1],
                word_bytes[2],
                word_bytes[3],
                word_bytes[4],
                word_bytes[5],
                word_bytes[6],
                word_bytes[7],
            ]);
        }

        for i in 16..80 {
            w[i] = little_sigma_1(w[i - 2])
                .wrapping_add(w[i - 7])
                .wrapping_add(little_sigma_0(w[i - 15]))
                .wrapping_add(w[i - 16]);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h_var] = *h;

        for i in 0..80 {
            let temp1 = h_var
                .wrapping_add(sigma_1(e))
                .wrapping_add(ch(e, f, g))
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let temp2 = sigma_0(a).wrapping_add(maj(a, b, c));

            h_var = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(h_var);
    }
}

pub(crate) fn calculate(data: &[u8]) -> [u8; 64] {
    let mut h = [
        0x6a09e667f3bcc908_u64,
        0xbb67ae8584caa73b,
        0x3c6ef372fe94f82b,
        0xa54ff53a5f1d36f1,
        0x510e527fade682d1,
        0x9b05688c2b3e6c1f,
        0x1f83d9abfb41bd6b,
        0x5be0cd19137e2179,
    ];

    calculate_with_initial_values(data, &mut h);

    let mut result = [0_u8; 64];
    for (i, &hash_word) in h.iter().enumerate() {
        let bytes = hash_word.to_be_bytes();
        result[i * 8..(i + 1) * 8].copy_from_slice(&bytes);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha512};

    #[test]
    fn correctness() {
        let test_cases = [
            b"" as &[u8],
            b"a",
            b"abc",
            b"Hello, World!",
            &[0xde, 0xad, 0xbe, 0xef],
        ];

        for test_case in test_cases {
            let our_result = calculate(test_case);
            let expected = Sha512::digest(test_case);
            assert_eq!(
                our_result,
                expected.as_slice(),
                "Failed for input: {test_case:?}"
            );
        }
    }
}
