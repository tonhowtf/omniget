//! Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/calculate_sha_other.js>.

pub(crate) fn calculate(data: &[u8]) -> [u8; 48] {
    let mut h = [
        0xcbbb9d5dc1059ed8_u64,
        0x629a292a367cd507,
        0x9159015a3070dd17,
        0x152fecd8f70e5939,
        0x67332667ffc00b31,
        0x8eb44a8768581511,
        0xdb0c2e0d64f98fa7,
        0x47b5481dbefa4fa4,
    ];

    super::sha512::calculate_with_initial_values(data, &mut h);

    let mut result = [0_u8; 48];
    for (i, &hash_word) in h.iter().take(6).enumerate() {
        let bytes = hash_word.to_be_bytes();
        result[i * 8..(i + 1) * 8].copy_from_slice(&bytes);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha384};

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
            let expected = Sha384::digest(test_case);
            assert_eq!(
                our_result,
                expected.as_slice(),
                "Failed for input: {test_case:?}"
            );
        }
    }
}
