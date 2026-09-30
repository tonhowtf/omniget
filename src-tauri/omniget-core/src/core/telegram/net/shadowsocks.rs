//! Shadowsocks client (AEAD and SIP022 "2022-blake3-*"), just enough to
//! carry MTProto: the most common protocol in the subscriptions people in
//! mainland China buy, and simple enough to own instead of shipping a core.
//!
//! The encrypted stream runs in two background tasks joined to the caller
//! by an in-memory duplex pipe, which keeps the cipher state machines out of
//! hand-written `poll_*` code.

use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes128Gcm, Aes256Gcm};
use anyhow::{anyhow, bail, Context};
use chacha20poly1305::ChaCha20Poly1305;
use rand::RngExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};

use super::route::Route;
use super::tunnel::{socks_addr, Target};

const TAG: usize = 16;
const PIPE: usize = 256 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Algo {
    Aes128,
    Aes256,
    Chacha,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flavor {
    Aead,
    Ss2022,
}

#[derive(Clone, Copy, Debug)]
struct Method {
    algo: Algo,
    flavor: Flavor,
    key_len: usize,
}

fn method(name: &str) -> anyhow::Result<Method> {
    let (algo, flavor, key_len) = match name {
        "aes-128-gcm" => (Algo::Aes128, Flavor::Aead, 16),
        "aes-256-gcm" => (Algo::Aes256, Flavor::Aead, 32),
        "chacha20-ietf-poly1305" | "chacha20-poly1305" => (Algo::Chacha, Flavor::Aead, 32),
        "2022-blake3-aes-128-gcm" => (Algo::Aes128, Flavor::Ss2022, 16),
        "2022-blake3-aes-256-gcm" => (Algo::Aes256, Flavor::Ss2022, 32),
        "2022-blake3-chacha20-poly1305" => (Algo::Chacha, Flavor::Ss2022, 32),
        other => bail!("Shadowsocks cipher {other} is not supported (use an AEAD or 2022 cipher)"),
    };
    Ok(Method {
        algo,
        flavor,
        key_len,
    })
}

pub fn supported(cipher: &str) -> bool {
    method(cipher).is_ok()
}

enum Aead {
    A128(Box<Aes128Gcm>),
    A256(Box<Aes256Gcm>),
    Chacha(Box<ChaCha20Poly1305>),
}

struct Cipher {
    aead: Aead,
    nonce: [u8; 12],
}

impl Cipher {
    fn new(algo: Algo, key: &[u8]) -> anyhow::Result<Self> {
        let aead = match algo {
            Algo::Aes128 => Aead::A128(Box::new(
                Aes128Gcm::new_from_slice(key).map_err(|_| anyhow!("key"))?,
            )),
            Algo::Aes256 => Aead::A256(Box::new(
                Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("key"))?,
            )),
            Algo::Chacha => Aead::Chacha(Box::new(
                ChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow!("key"))?,
            )),
        };
        Ok(Self {
            aead,
            nonce: [0; 12],
        })
    }

    fn bump(&mut self) {
        for b in self.nonce.iter_mut() {
            *b = b.wrapping_add(1);
            if *b != 0 {
                break;
            }
        }
    }

    fn seal(&mut self, data: &[u8], out: &mut Vec<u8>) -> anyhow::Result<()> {
        let mut buf = data.to_vec();
        let n = aes_gcm::Nonce::from_slice(&self.nonce);
        let tag = match &self.aead {
            Aead::A128(c) => c.encrypt_in_place_detached(n, &[], &mut buf),
            Aead::A256(c) => c.encrypt_in_place_detached(n, &[], &mut buf),
            Aead::Chacha(c) => c.encrypt_in_place_detached(
                chacha20poly1305::Nonce::from_slice(&self.nonce),
                &[],
                &mut buf,
            ),
        }
        .map_err(|_| anyhow!("encrypt"))?;
        self.bump();
        out.extend_from_slice(&buf);
        out.extend_from_slice(&tag);
        Ok(())
    }

    fn open(&mut self, sealed: &mut [u8]) -> anyhow::Result<usize> {
        anyhow::ensure!(sealed.len() >= TAG, "short chunk");
        let (data, tag) = sealed.split_at_mut(sealed.len() - TAG);
        let n = aes_gcm::Nonce::from_slice(&self.nonce);
        let tag = aes_gcm::Tag::from_slice(tag);
        match &self.aead {
            Aead::A128(c) => c.decrypt_in_place_detached(n, &[], data, tag),
            Aead::A256(c) => c.decrypt_in_place_detached(n, &[], data, tag),
            Aead::Chacha(c) => c.decrypt_in_place_detached(
                chacha20poly1305::Nonce::from_slice(&self.nonce),
                &[],
                data,
                tag,
            ),
        }
        .map_err(|_| anyhow!("Shadowsocks decryption failed: wrong password or cipher"))?;
        self.bump();
        Ok(data.len())
    }
}

/// OpenSSL `EVP_BytesToKey` with MD5, what every classic SS client uses to
/// turn the password into the master key.
fn evp_bytes_to_key(password: &[u8], key_len: usize) -> Vec<u8> {
    use md5::{Digest, Md5};
    let mut key = Vec::with_capacity(key_len + 16);
    let mut prev: Vec<u8> = Vec::new();
    while key.len() < key_len {
        let mut h = Md5::new();
        h.update(&prev);
        h.update(password);
        prev = h.finalize().to_vec();
        key.extend_from_slice(&prev);
    }
    key.truncate(key_len);
    key
}

fn session_key(m: Method, master: &[u8], salt: &[u8]) -> anyhow::Result<Vec<u8>> {
    match m.flavor {
        Flavor::Aead => {
            let hk = hkdf::Hkdf::<sha1::Sha1>::new(Some(salt), master);
            let mut okm = vec![0u8; m.key_len];
            hk.expand(b"ss-subkey", &mut okm)
                .map_err(|_| anyhow!("hkdf"))?;
            Ok(okm)
        }
        Flavor::Ss2022 => {
            let mut material = Vec::with_capacity(master.len() + salt.len());
            material.extend_from_slice(master);
            material.extend_from_slice(salt);
            let k = blake3::derive_key("shadowsocks 2022 session subkey", &material);
            Ok(k[..m.key_len].to_vec())
        }
    }
}

fn master_key(m: Method, password: &str) -> anyhow::Result<Vec<u8>> {
    match m.flavor {
        Flavor::Aead => Ok(evp_bytes_to_key(password.as_bytes(), m.key_len)),
        Flavor::Ss2022 => {
            use base64::Engine;
            // Multi-user servers hand out "iPSK:uPSK"; single-user is the
            // norm for subscriptions and is what we speak.
            let psk = password.rsplit(':').next().unwrap_or(password);
            let key = base64::engine::general_purpose::STANDARD
                .decode(psk.trim())
                .context("SS2022 password must be a base64 key")?;
            anyhow::ensure!(
                key.len() == m.key_len,
                "SS2022 key has the wrong length for this cipher"
            );
            Ok(key)
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Opens a Shadowsocks session over `server` towards `target` and returns
/// the plaintext end.
pub async fn connect<S>(server: S, route: &Route, target: &Target) -> anyhow::Result<DuplexStream>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let m = method(route.cipher.as_deref().unwrap_or(""))?;
    let master = master_key(m, route.password.as_deref().unwrap_or(""))?;
    let mut salt = vec![0u8; m.key_len];
    rand::rng().fill(&mut salt[..]);
    let mut enc = Cipher::new(m.algo, &session_key(m, &master, &salt)?)?;

    // Request header goes out immediately; MTProto always speaks first, but
    // SS2022 forbids a header without payload or padding, so we pad.
    let addr = socks_addr(target);
    let mut first = salt.clone();
    match m.flavor {
        Flavor::Aead => {
            let len = (addr.len() as u16).to_be_bytes();
            enc.seal(&len, &mut first)?;
            enc.seal(&addr, &mut first)?;
        }
        Flavor::Ss2022 => {
            let pad_len: u16 = rand::rng().random_range(16..=256);
            let mut var = addr.clone();
            var.extend_from_slice(&pad_len.to_be_bytes());
            var.extend(std::iter::repeat_n(0u8, pad_len as usize));
            let mut fixed = vec![0u8];
            fixed.extend_from_slice(&now_secs().to_be_bytes());
            fixed.extend_from_slice(&(var.len() as u16).to_be_bytes());
            enc.seal(&fixed, &mut first)?;
            enc.seal(&var, &mut first)?;
        }
    }

    let (mut srv_r, mut srv_w) = tokio::io::split(server);
    srv_w
        .write_all(&first)
        .await
        .context("Shadowsocks handshake")?;
    let (ours, theirs) = tokio::io::duplex(PIPE);
    let (mut app_r, mut app_w) = tokio::io::split(theirs);

    let max_chunk = if m.flavor == Flavor::Ss2022 {
        0xffff
    } else {
        0x3fff
    };
    tokio::spawn(async move {
        let mut buf = vec![0u8; max_chunk];
        loop {
            let n = match app_r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut out = Vec::with_capacity(n + 2 * TAG + 2);
            if enc.seal(&(n as u16).to_be_bytes(), &mut out).is_err()
                || enc.seal(&buf[..n], &mut out).is_err()
            {
                break;
            }
            if srv_w.write_all(&out).await.is_err() {
                break;
            }
        }
        let _ = srv_w.shutdown().await;
    });

    let request_salt = salt;
    tokio::spawn(async move {
        let res: anyhow::Result<()> = async {
            let mut srv_salt = vec![0u8; m.key_len];
            srv_r.read_exact(&mut srv_salt).await?;
            let mut dec = Cipher::new(m.algo, &session_key(m, &master, &srv_salt)?)?;
            if m.flavor == Flavor::Ss2022 {
                let mut fixed = vec![0u8; 1 + 8 + m.key_len + 2 + TAG];
                srv_r.read_exact(&mut fixed).await?;
                let n = dec.open(&mut fixed)?;
                let fixed = &fixed[..n];
                anyhow::ensure!(fixed[0] == 1, "SS2022 bad response type");
                anyhow::ensure!(
                    &fixed[9..9 + m.key_len] == request_salt.as_slice(),
                    "SS2022 salt mismatch"
                );
                let len =
                    u16::from_be_bytes([fixed[9 + m.key_len], fixed[10 + m.key_len]]) as usize;
                let mut body = vec![0u8; len + TAG];
                srv_r.read_exact(&mut body).await?;
                let n = dec.open(&mut body)?;
                app_w.write_all(&body[..n]).await?;
            }
            loop {
                let mut lenb = [0u8; 2 + TAG];
                if srv_r.read_exact(&mut lenb).await.is_err() {
                    break;
                }
                dec.open(&mut lenb)?;
                let len = u16::from_be_bytes([lenb[0], lenb[1]]) as usize;
                let mut body = vec![0u8; len + TAG];
                srv_r.read_exact(&mut body).await?;
                let n = dec.open(&mut body)?;
                app_w.write_all(&body[..n]).await?;
            }
            Ok(())
        }
        .await;
        if let Err(e) = res {
            tracing::debug!("[tg-net] shadowsocks read ended: {e}");
        }
        let _ = app_w.shutdown().await;
    });

    Ok(ours)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evp_matches_reference() {
        // shadowsocks-rust test vector: "foobar", 32 bytes.
        let k = evp_bytes_to_key(b"foobar", 32);
        assert_eq!(hex::encode(&k[..16]), "3858f62230ac3c915f300c664312c63f");
    }

    #[test]
    fn nonce_increments_little_endian() {
        let mut c = Cipher::new(Algo::Aes128, &[0u8; 16]).unwrap();
        for _ in 0..256 {
            c.bump();
        }
        assert_eq!(&c.nonce[..2], &[0, 1]);
    }

    #[tokio::test]
    async fn aead_round_trip_against_ourselves() {
        // Encrypt a request like the client does and decrypt it like a
        // server would: proves chunk framing and key schedule agree.
        let m = method("chacha20-ietf-poly1305").unwrap();
        let master = master_key(m, "pw").unwrap();
        let salt = [7u8; 32];
        let key = session_key(m, &master, &salt).unwrap();
        let mut enc = Cipher::new(m.algo, &key).unwrap();
        let mut out = Vec::new();
        enc.seal(&5u16.to_be_bytes(), &mut out).unwrap();
        enc.seal(b"hello", &mut out).unwrap();
        let mut dec = Cipher::new(m.algo, &key).unwrap();
        let (a, b) = out.split_at_mut(2 + TAG);
        dec.open(a).unwrap();
        let n = dec.open(b).unwrap();
        assert_eq!(&b[..n], b"hello");
    }
}
