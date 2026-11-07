//! Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/calculate_md5.js>.

use alloc::vec::Vec;

const SHIFT_AMOUNTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const CONSTANTS: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

pub(crate) fn calculate(data: &[u8]) -> [u8; 16] {
    let mut state = [
        0x67452301_u32,
        0xefcdab89_u32,
        0x98badcfe_u32,
        0x10325476_u32,
    ];

    let original_len = data.len();
    let mut message = data.to_vec();
    message.push(0x80);

    while (message.len() % 64) != 56 {
        message.push(0);
    }

    message.extend_from_slice(&(original_len as u64 * 8).to_le_bytes());

    for chunk in message.chunks_exact(64) {
        let words: Vec<u32> = chunk
            .chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();

        let mut working_vars = state;

        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => (
                    (working_vars[1] & working_vars[2]) | (!working_vars[1] & working_vars[3]),
                    i,
                ),
                16..=31 => (
                    (working_vars[3] & working_vars[1]) | (!working_vars[3] & working_vars[2]),
                    (5 * i + 1) % 16,
                ),
                32..=47 => (
                    working_vars[1] ^ working_vars[2] ^ working_vars[3],
                    (3 * i + 5) % 16,
                ),
                48..=63 => (
                    working_vars[2] ^ (working_vars[1] | !working_vars[3]),
                    (7 * i) % 16,
                ),
                _ => unreachable!(),
            };

            let temp = working_vars[3];
            working_vars[3] = working_vars[2];
            working_vars[2] = working_vars[1];
            working_vars[1] = working_vars[1].wrapping_add(
                (working_vars[0]
                    .wrapping_add(f)
                    .wrapping_add(CONSTANTS[i])
                    .wrapping_add(words[g]))
                .rotate_left(SHIFT_AMOUNTS[i]),
            );
            working_vars[0] = temp;
        }

        for (state_val, working_val) in state.iter_mut().zip(working_vars.iter()) {
            *state_val = state_val.wrapping_add(*working_val);
        }
    }

    let mut result = [0_u8; 16];
    for (i, &word) in state.iter().enumerate() {
        result[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md5_test(data: &[u8]) {
        let our_result = calculate(data);

        let external_result = md5::compute(data);

        assert_eq!(
            our_result,
            external_result.as_slice(),
            "MD5 calculation should match external md5 crate for input: {data:?}"
        );
    }

    #[test]
    fn correctness() {
        md5_test(b"Hello, World!");
        md5_test(b"The quick brown fox jumps over the lazy dog");
        md5_test(b"abcdefghijklmnopqrstuvwxyz");
    }
}
