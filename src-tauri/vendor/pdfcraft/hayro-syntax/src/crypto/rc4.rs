//! Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/crypto.js>.

use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone)]
pub(crate) struct Rc4 {
    a: u8,
    b: u8,
    s: [u8; 256],
}

impl Rc4 {
    pub(crate) fn new(key: &[u8]) -> Self {
        let mut s = [0_u8; 256];
        let key_length = key.len();

        for (i, s) in s.iter_mut().enumerate() {
            *s = i as u8;
        }

        let mut j = 0_u8;
        for i in 0..256 {
            let tmp = s[i];
            j = j.wrapping_add(tmp).wrapping_add(key[i % key_length]);
            s[i] = s[j as usize];
            s[j as usize] = tmp;
        }

        Self { a: 0, b: 0, s }
    }

    pub(crate) fn decrypt(&mut self, data: &[u8]) -> Vec<u8> {
        let n = data.len();
        let mut output = vec![0_u8; n];

        for i in 0..n {
            self.a = self.a.wrapping_add(1);
            let tmp = self.s[self.a as usize];
            self.b = self.b.wrapping_add(tmp);
            let tmp2 = self.s[self.b as usize];
            self.s[self.a as usize] = tmp2;
            self.s[self.b as usize] = tmp;
            output[i] = data[i] ^ self.s[tmp.wrapping_add(tmp2) as usize];
        }

        output
    }

    pub(crate) fn encrypt(&mut self, data: &[u8]) -> Vec<u8> {
        self.decrypt(data)
    }
}

#[cfg(test)]
mod rc4_tests {
    use crate::crypto::rc4::Rc4;

    fn rc4_decrypt(key: &[u8], input: &[u8]) -> Vec<u8> {
        let mut cipher = Rc4::new(key);
        cipher.decrypt(input)
    }

    #[test]
    fn correctness() {
        assert_eq!(rc4_decrypt(b"a", &[0x68]), b"x");
        assert_eq!(rc4_decrypt(b"key", &[0x7F, 0x09, 0x47, 0x99]), b"test");
        assert_eq!(
            rc4_decrypt(b"hello", &[0x78, 0x3E, 0xCD, 0x96, 0xCF]),
            b"world"
        );
        assert_eq!(rc4_decrypt(b"\x01\x02", &[0x0C, 0x74, 0xB9]), b"Hi!");
        assert_eq!(rc4_decrypt(b"secret", &[0x80, 0x45, 0xB5]), b"msg");

        assert_eq!(
            rc4_decrypt(
                b"encryption",
                &[
                    0x8A, 0x36, 0x3F, 0x85, 0xDB, 0x9A, 0x62, 0x7C, 0x6C, 0x56, 0x81, 0x89
                ]
            ),
            b"Hello World!"
        );
        assert_eq!(
            rc4_decrypt(
                b"my_secret_key",
                &[
                    0x1D, 0xE2, 0xCE, 0x64, 0x4C, 0x88, 0x1C, 0x42, 0x7D, 0x94, 0x7B, 0x1C, 0x49,
                    0xCD, 0x62, 0x3F, 0xCA, 0x90, 0x99
                ]
            ),
            b"The quick brown fox"
        );
        assert_eq!(
            rc4_decrypt(
                b"abcdefghijklmnop",
                &[
                    0xF8, 0xA8, 0x2A, 0xAE, 0x92, 0x2F, 0x8C, 0x3B, 0x13, 0xC7, 0x4B, 0x99, 0xDB,
                    0x41, 0x2C, 0x9F, 0x92, 0x59, 0xC2, 0xE5, 0x62, 0x2C
                ]
            ),
            b"This is a test message"
        );
        assert_eq!(
            rc4_decrypt(
                b"password123",
                &[
                    0xDB, 0xC3, 0xA3, 0x21, 0x7E, 0x55, 0x0B, 0x65, 0xA2, 0x3F, 0x0E, 0xB7, 0x58,
                    0x7C, 0x28, 0xEE, 0xD6, 0x92, 0x05, 0xBC, 0x6C
                ]
            ),
            b"Decryption successful"
        );
        assert_eq!(
            rc4_decrypt(
                b"very_long_key",
                &[
                    0xC6, 0xD2, 0x00, 0x3B, 0x8C, 0x18, 0x49, 0x2C, 0xD0, 0x5C, 0x8A, 0x34, 0xEC,
                    0xFE, 0xED, 0x62, 0xC0, 0x3C, 0x80, 0x81, 0x7E, 0x4E, 0xE1, 0x7D, 0xBD
                ]
            ),
            b"PDF uses RC4 for security"
        );
    }
}
