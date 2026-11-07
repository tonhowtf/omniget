//! Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/calculate_sha256.js>.

use alloc::vec;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[inline]
const fn rotr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}

#[inline]
const fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

#[inline]
const fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

#[inline]
const fn sigma_0(x: u32) -> u32 {
    rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22)
}

#[inline]
const fn sigma_1(x: u32) -> u32 {
    rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25)
}

#[inline]
const fn little_sigma_0(x: u32) -> u32 {
    rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3)
}

#[inline]
const fn little_sigma_1(x: u32) -> u32 {
    rotr(x, 17) ^ rotr(x, 19) ^ (x >> 10)
}

pub(crate) fn calculate(data: &[u8]) -> [u8; 32] {
    let mut h = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = data.len() as u64 * 8;
    let padded_len = (data.len() + 9).div_ceil(64) * 64;
    let mut padded = vec![0_u8; padded_len];

    padded[..data.len()].copy_from_slice(data);
    padded[data.len()] = 0x80;

    let len_bytes = bit_len.to_be_bytes();
    padded[padded_len - 8..].copy_from_slice(&len_bytes);

    for chunk in padded.chunks_exact(64) {
        let mut w = [0_u32; 64];

        for (i, word_bytes) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word_bytes[0], word_bytes[1], word_bytes[2], word_bytes[3]]);
        }

        for i in 16..64 {
            w[i] = little_sigma_1(w[i - 2])
                .wrapping_add(w[i - 7])
                .wrapping_add(little_sigma_0(w[i - 15]))
                .wrapping_add(w[i - 16]);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h_var] = h;

        for i in 0..64 {
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

    let mut result = [0_u8; 32];
    for (i, &hash_word) in h.iter().enumerate() {
        let bytes = hash_word.to_be_bytes();
        result[i * 4..(i + 1) * 4].copy_from_slice(&bytes);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

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
            let expected = Sha256::digest(test_case);
            assert_eq!(
                our_result,
                expected.as_slice(),
                "Failed for input: {test_case:?}"
            );
        }
    }
}
