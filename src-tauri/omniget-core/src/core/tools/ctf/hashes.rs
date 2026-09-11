//! Hash de texto e de arquivo, e o palpite de que tipo é um hash dado.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct HashSet {
    pub crc32: String,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
    pub bytes: u64,
}

/// CRC-32 (IEEE), a mesma do zip e do PNG. Tabela montada na hora — são 256
/// entradas, não vale uma dependência.
pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, e) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *e = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for b in data {
        crc = table[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

pub fn hash_all(data: &[u8]) -> HashSet {
    // md-5, sha1 e sha2 compartilham o mesmo trait `Digest` do RustCrypto.
    use md5::Digest as _;
    HashSet {
        crc32: format!("{:08x}", crc32(data)),
        md5: hex::encode(md5::Md5::digest(data)),
        sha1: hex::encode(sha1::Sha1::digest(data)),
        sha256: hex::encode(sha2::Sha256::digest(data)),
        sha512: hex::encode(sha2::Sha512::digest(data)),
        bytes: data.len() as u64,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HmacOptions {
    pub data: String,
    pub key: String,
    /// "sha256" | "sha512" | "sha1" | "md5"
    #[serde(default = "default_alg")]
    pub algorithm: String,
}

fn default_alg() -> String {
    "sha256".into()
}

pub fn hmac(data: &[u8], key: &[u8], algorithm: &str) -> String {
    use hmac::Mac;
    match algorithm {
        "sha1" => {
            let mut m = <hmac::Hmac<sha1::Sha1>>::new_from_slice(key).unwrap();
            m.update(data);
            hex::encode(m.finalize().into_bytes())
        }
        "md5" => {
            let mut m = <hmac::Hmac<md5::Md5>>::new_from_slice(key).unwrap();
            m.update(data);
            hex::encode(m.finalize().into_bytes())
        }
        "sha512" => {
            let mut m = <hmac::Hmac<sha2::Sha512>>::new_from_slice(key).unwrap();
            m.update(data);
            hex::encode(m.finalize().into_bytes())
        }
        _ => {
            let mut m = <hmac::Hmac<sha2::Sha256>>::new_from_slice(key).unwrap();
            m.update(data);
            hex::encode(m.finalize().into_bytes())
        }
    }
}

// ── Que hash é esse? ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct HashGuess {
    pub name: String,
    /// "alta" | "média" | "baixa"
    pub confidence: String,
    pub note: String,
}

/// Palpites por prefixo, tamanho e alfabeto. Um hash cru de 32 hex é MD5 ou
/// NTLM e não tem como distinguir — por isso a lista sai ordenada, não única.
pub fn identify(input: &str) -> Vec<HashGuess> {
    let s = input.trim();
    let mut out = Vec::new();
    let guess = |name: &str, confidence: &str, note: &str| HashGuess {
        name: name.into(),
        confidence: confidence.into(),
        note: note.into(),
    };

    // Formatos com prefixo são inequívocos.
    for (prefix, name, note) in [
        ("$2a$", "bcrypt", "senha, com custo embutido"),
        ("$2b$", "bcrypt", "senha, com custo embutido"),
        ("$2y$", "bcrypt", "senha, com custo embutido"),
        ("$argon2i$", "Argon2i", "senha"),
        ("$argon2id$", "Argon2id", "senha"),
        ("$1$", "MD5-crypt", "/etc/shadow antigo"),
        ("$5$", "SHA-256 crypt", "/etc/shadow"),
        ("$6$", "SHA-512 crypt", "/etc/shadow"),
        ("$y$", "yescrypt", "/etc/shadow moderno"),
        ("$pbkdf2", "PBKDF2", "senha"),
        ("{SSHA}", "SSHA", "LDAP"),
        ("pbkdf2_sha256$", "PBKDF2-SHA256", "Django"),
    ] {
        if s.starts_with(prefix) {
            out.push(guess(name, "alta", note));
            return out;
        }
    }

    let hex = !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit());
    if hex {
        match s.len() {
            8 => out.push(guess("CRC-32", "média", "8 hex também pode ser Adler-32")),
            16 => out.push(guess("CRC-64 / MySQL antigo", "baixa", "16 hex")),
            32 => {
                out.push(guess("MD5", "média", "32 hex"));
                out.push(guess("NTLM", "média", "mesmo tamanho do MD5"));
                out.push(guess("MD4", "baixa", "mesmo tamanho do MD5"));
            }
            40 => {
                out.push(guess("SHA-1", "média", "40 hex"));
                out.push(guess("RIPEMD-160", "baixa", "mesmo tamanho do SHA-1"));
            }
            56 => out.push(guess("SHA-224", "média", "56 hex")),
            64 => {
                out.push(guess("SHA-256", "média", "64 hex"));
                out.push(guess("SHA3-256 / BLAKE2s", "baixa", "mesmo tamanho"));
            }
            96 => out.push(guess("SHA-384", "média", "96 hex")),
            128 => {
                out.push(guess("SHA-512", "média", "128 hex"));
                out.push(guess("BLAKE2b / Whirlpool", "baixa", "mesmo tamanho"));
            }
            _ => {}
        }
    }
    if out.is_empty() {
        let b64ish = s.len() >= 16
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=');
        if b64ish {
            out.push(guess(
                "base64",
                "baixa",
                "parece codificação, não hash — decodifique antes",
            ));
        } else {
            out.push(guess(
                "desconhecido",
                "baixa",
                "tamanho e alfabeto não batem com nenhum formato conhecido",
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors_match() {
        let h = hash_all(b"abc");
        assert_eq!(h.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            h.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(h.crc32, "352441c2");
        assert_eq!(h.bytes, 3);
    }

    #[test]
    fn crc32_matches_the_classic_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // RFC 4231, caso 2.
        let mac = hmac(b"what do ya want for nothing?", b"Jefe", "sha256");
        assert_eq!(
            mac,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn prefixed_formats_are_certain() {
        let g = identify("$2b$12$abcdefghijklmnopqrstuv");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "bcrypt");
        assert_eq!(g[0].confidence, "alta");
        assert_eq!(identify("$6$salt$hash")[0].name, "SHA-512 crypt");
    }

    #[test]
    fn ambiguous_lengths_return_every_candidate() {
        let g = identify("900150983cd24fb0d6963f7d28e17f72");
        let names: Vec<&str> = g.iter().map(|x| x.name.as_str()).collect();
        assert!(names.contains(&"MD5") && names.contains(&"NTLM"));
        let g = identify("a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(g[0].name, "SHA-1");
    }

    #[test]
    fn base64_is_called_out_as_not_a_hash() {
        let g = identify("SGVsbG8gbXVuZG8gZGUgdGVzdGU=");
        assert_eq!(g[0].name, "base64");
    }

    #[test]
    fn garbage_is_unknown() {
        assert_eq!(identify("nada disso aqui!!")[0].name, "desconhecido");
    }
}
