//! AES-CBC (128 or 256-bit key, chosen by key length) with optional PKCS#5 padding.
//!
//! Decryption is tolerant, as viewers are: a trailing partial block is ignored, and invalid
//! padding leaves the data unpadded rather than failing.

use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use aes::{Aes128, Aes256};

enum Cipher {
    A128(Box<Aes128>),
    A256(Box<Aes256>),
}

impl Cipher {
    fn new(key: &[u8]) -> Self {
        if let Some(k) = key.first_chunk::<32>() {
            Cipher::A256(Box::new(Aes256::new(&Array::from(*k))))
        } else {
            let mut k = [0u8; 16];
            let n = key.len().min(16);
            k[..n].copy_from_slice(&key[..n]);
            Cipher::A128(Box::new(Aes128::new(&Array::from(k))))
        }
    }

    fn encrypt(&self, block: &mut [u8; 16]) {
        let mut b = Array::from(*block);
        match self {
            Cipher::A128(c) => c.encrypt_block(&mut b),
            Cipher::A256(c) => c.encrypt_block(&mut b),
        }
        *block = b.into();
    }

    fn decrypt(&self, block: &mut [u8; 16]) {
        let mut b = Array::from(*block);
        match self {
            Cipher::A128(c) => c.decrypt_block(&mut b),
            Cipher::A256(c) => c.decrypt_block(&mut b),
        }
        *block = b.into();
    }
}

/// Encrypt; with `pad`, PKCS#5 padding is added (otherwise `data` must be whole blocks, and a
/// trailing partial block is zero-padded).
pub fn aes_cbc_encrypt(key: &[u8], iv: &[u8; 16], data: &[u8], pad: bool) -> Vec<u8> {
    let c = Cipher::new(key);
    let mut buf = data.to_vec();
    if pad {
        let n = 16 - buf.len() % 16;
        buf.extend(std::iter::repeat_n(n as u8, n));
    } else if !buf.len().is_multiple_of(16) {
        buf.resize(buf.len().div_ceil(16) * 16, 0);
    }
    let mut prev = *iv;
    for b in buf.as_chunks_mut::<16>().0 {
        for (x, p) in b.iter_mut().zip(prev) {
            *x ^= p;
        }
        c.encrypt(b);
        prev = *b;
    }
    buf
}

/// Decrypt; with `pad`, PKCS#5 padding is removed when valid.
pub fn aes_cbc_decrypt(key: &[u8], iv: &[u8; 16], data: &[u8], pad: bool) -> Vec<u8> {
    let c = Cipher::new(key);
    let blocks = data.as_chunks::<16>().0;
    let mut out = Vec::with_capacity(blocks.len() * 16);
    let mut prev = *iv;
    for &cipher in blocks {
        let mut b = cipher;
        c.decrypt(&mut b);
        for (x, p) in b.iter_mut().zip(prev) {
            *x ^= p;
        }
        out.extend_from_slice(&b);
        prev = cipher;
    }
    if pad && let Some(&n) = out.last() {
        let n = n as usize;
        if (1..=16).contains(&n) && n <= out.len() && out[out.len() - n..].iter().all(|b| *b as usize == n) {
            out.truncate(out.len() - n);
        }
    }
    out
}
