//! MTProto transport obfuscation ("obfuscated2"), MTProto proxy secrets
//! (plain, `dd` padded, `ee` fake TLS) and the intermediate framing that
//! rides inside them.
//!
//! References: TDLib `td/mtproto/TcpTransport.cpp`, `TlsInit.cpp`;
//! mtcute `network/mtproxy/_fake-tls.ts`; the live prototype in
//! `estudos/98-telegram/03-anexos/tg_probe.py`.

use aes::cipher::{KeyIvInit, StreamCipher};
use anyhow::{bail, Context};
use hmac::{Hmac, Mac};
use rand::RngExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

type Ctr = ctr::Ctr128BE<aes::Aes256>;

pub const TAG_ABRIDGED: u32 = 0xefef_efef;
pub const TAG_INTERMEDIATE: u32 = 0xeeee_eeee;
pub const TAG_PADDED: u32 = 0xdddd_dddd;

/// Both directions of an obfuscated2 session.
pub struct Obfs {
    pub enc: Ctr,
    pub dec: Ctr,
}

/// Builds the 64-byte init packet. Returns (bytes to send, ciphers).
/// `secret` is the 16-byte proxy key, `None` for a direct DC connection.
pub fn init(tag: u32, dc: i16, secret: Option<&[u8; 16]>) -> ([u8; 64], Obfs) {
    let mut rng = rand::rng();
    let mut h = [0u8; 64];
    loop {
        rng.fill(&mut h[..]);
        if h[0] == 0xef {
            continue;
        }
        let first = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        if matches!(
            first,
            0x4441_4548
                | 0x5453_4f50
                | 0x2054_4547
                | 0x4954_504f
                | 0xdddd_dddd
                | 0xeeee_eeee
                | 0x0201_0316
        ) {
            continue;
        }
        if h[4..8] == [0, 0, 0, 0] {
            continue;
        }
        break;
    }
    h[56..60].copy_from_slice(&tag.to_le_bytes());
    h[60..62].copy_from_slice(&dc.to_le_bytes());

    let mut rev = h;
    rev.reverse();
    let key = |raw: &[u8]| -> [u8; 32] {
        match secret {
            Some(s) => {
                let mut d = Sha256::new();
                d.update(raw);
                d.update(s);
                d.finalize().into()
            }
            None => raw.try_into().expect("32 bytes"),
        }
    };
    let ek = key(&h[8..40]);
    let dk = key(&rev[8..40]);
    let mut enc = Ctr::new((&ek).into(), h[40..56].into());
    let dec = Ctr::new((&dk).into(), rev[40..56].into());

    let mut encrypted = h;
    enc.apply_keystream(&mut encrypted);
    let mut wire = h;
    wire[56..64].copy_from_slice(&encrypted[56..64]);
    (wire, Obfs { enc, dec })
}

/// What a proxy secret asks for.
#[derive(Clone, Debug)]
pub struct ProxySecret {
    pub key: [u8; 16],
    pub padded: bool,
    pub tls_domain: Option<String>,
}

impl ProxySecret {
    pub fn parse(hex_secret: &str) -> anyhow::Result<Self> {
        use super::links::{secret_mode, SecretMode};
        match secret_mode(hex_secret) {
            Some(SecretMode::Plain(key)) => Ok(Self {
                key,
                padded: false,
                tls_domain: None,
            }),
            Some(SecretMode::Padded(key)) => Ok(Self {
                key,
                padded: true,
                tls_domain: None,
            }),
            Some(SecretMode::FakeTls { key, domain }) => Ok(Self {
                key,
                padded: true,
                tls_domain: Some(domain),
            }),
            None => bail!("this MTProto proxy secret is not in a known format"),
        }
    }

    pub fn tag(&self) -> u32 {
        if self.padded {
            TAG_PADDED
        } else {
            TAG_INTERMEDIATE
        }
    }
}

// ---------------------------------------------------------------- fake TLS

/// Largest TLS record payload the proxies accept from clients. Writes are
/// split into records of at most this size (gotd's bug was forgetting to).
pub const TLS_MAX_RECORD: usize = 2878;

fn grease_values() -> [u8; 7] {
    let mut g = [0u8; 7];
    rand::rng().fill(&mut g[..]);
    for b in g.iter_mut() {
        *b = (*b & 0xf0) + 0x0a;
    }
    for i in (1..7).step_by(2) {
        if g[i] == g[i - 1] {
            g[i] ^= 0x10;
        }
    }
    g
}

struct Hello {
    buf: Vec<u8>,
    scopes: Vec<usize>,
    grease: [u8; 7],
}

impl Hello {
    fn new(grease: [u8; 7]) -> Self {
        Self {
            buf: Vec::with_capacity(2048),
            scopes: Vec::new(),
            grease,
        }
    }
    fn s(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    fn random(&mut self, n: usize) {
        let mut v = vec![0u8; n];
        rand::rng().fill(&mut v[..]);
        self.buf.extend_from_slice(&v);
    }
    fn grease(&mut self, i: usize) {
        let g = self.grease[i];
        self.buf.extend_from_slice(&[g, g]);
    }
    fn begin(&mut self) {
        self.scopes.push(self.buf.len());
        self.buf.extend_from_slice(&[0, 0]);
    }
    fn end(&mut self) {
        let at = self.scopes.pop().expect("scope");
        let len = (self.buf.len() - at - 2) as u16;
        self.buf[at..at + 2].copy_from_slice(&len.to_be_bytes());
    }
    fn x25519_key(&mut self) {
        // A real X25519 public key: DPI boxes check the point is on the curve.
        let rng = ring::rand::SystemRandom::new();
        let key = ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::X25519, &rng)
            .and_then(|k| k.compute_public_key())
            .map(|p| p.as_ref().to_vec())
            .unwrap_or_else(|_| vec![0x42; 32]);
        self.s(&key);
    }
    fn mlkem768_key(&mut self) {
        let mut rng = rand::rng();
        for _ in 0..384 {
            let a: u32 = rng.random_range(0..3329);
            let b: u32 = rng.random_range(0..3329);
            self.buf.push((a & 0xff) as u8);
            self.buf.push(((a >> 8) + ((b & 0x0f) << 4)) as u8);
            self.buf.push((b >> 4) as u8);
        }
        self.random(32);
    }
}

type Part = fn(&mut Hello, &[u8]);

/// Chrome-shaped ClientHello with the proxy's domain as SNI, the HMAC of
/// the secret in the random field and the timestamp folded into it. Same
/// layout as TDLib's current `TlsInit` (post-quantum key share, GREASE ECH,
/// shuffled extensions, padding to 513).
pub fn client_hello(domain: &str, key: &[u8; 16]) -> Vec<u8> {
    let grease = grease_values();
    let mut h = Hello::new(grease);
    h.s(&[0x16, 0x03, 0x01]);
    h.begin();
    h.s(&[0x01, 0x00]);
    h.begin();
    h.s(&[0x03, 0x03]);
    h.s(&[0u8; 32]);
    h.s(&[0x20]);
    h.random(32);
    h.s(&[0x00, 0x20]);
    h.grease(0);
    h.s(&[
        0x13, 0x01, 0x13, 0x02, 0x13, 0x03, 0xc0, 0x2b, 0xc0, 0x2f, 0xc0, 0x2c, 0xc0, 0x30, 0xcc,
        0xa9, 0xcc, 0xa8, 0xc0, 0x13, 0xc0, 0x14, 0x00, 0x9c, 0x00, 0x9d, 0x00, 0x2f, 0x00, 0x35,
        0x01, 0x00,
    ]);
    h.begin();
    h.grease(2);
    h.s(&[0x00, 0x00]);

    let parts: [Part; 16] = [
        |w, d| {
            w.s(&[0x00, 0x00]);
            w.begin();
            w.begin();
            w.s(&[0x00]);
            w.begin();
            w.s(d);
            w.end();
            w.end();
            w.end();
        },
        |w, _| w.s(&[0x00, 0x05, 0x00, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00]),
        |w, _| {
            w.s(&[0x00, 0x0a, 0x00, 0x0c, 0x00, 0x0a]);
            w.grease(4);
            w.s(&[0x11, 0xec, 0x00, 0x1d, 0x00, 0x17, 0x00, 0x18]);
        },
        |w, _| w.s(&[0x00, 0x0b, 0x00, 0x02, 0x01, 0x00]),
        |w, _| {
            w.s(&[
                0x00, 0x0d, 0x00, 0x12, 0x00, 0x10, 0x04, 0x03, 0x08, 0x04, 0x04, 0x01, 0x05, 0x03,
                0x08, 0x05, 0x05, 0x01, 0x08, 0x06, 0x06, 0x01,
            ])
        },
        |w, _| {
            w.s(&[
                0x00, 0x10, 0x00, 0x0e, 0x00, 0x0c, 0x02, 0x68, 0x32, 0x08, 0x68, 0x74, 0x74, 0x70,
                0x2f, 0x31, 0x2e, 0x31,
            ])
        },
        |w, _| w.s(&[0x00, 0x12, 0x00, 0x00]),
        |w, _| w.s(&[0x00, 0x17, 0x00, 0x00]),
        |w, _| w.s(&[0x00, 0x1b, 0x00, 0x03, 0x02, 0x00, 0x02]),
        |w, _| w.s(&[0x00, 0x23, 0x00, 0x00]),
        |w, _| {
            w.s(&[0x00, 0x2b, 0x00, 0x07, 0x06]);
            w.grease(6);
            w.s(&[0x03, 0x04, 0x03, 0x03]);
        },
        |w, _| w.s(&[0x00, 0x2d, 0x00, 0x02, 0x01, 0x01]),
        |w, _| {
            w.s(&[0x00, 0x33, 0x04, 0xef, 0x04, 0xed]);
            w.grease(4);
            w.s(&[0x00, 0x01, 0x00, 0x11, 0xec, 0x04, 0xc0]);
            w.mlkem768_key();
            w.x25519_key();
            w.s(&[0x00, 0x1d, 0x00, 0x20]);
            w.x25519_key();
        },
        |w, _| w.s(&[0x44, 0xcd, 0x00, 0x05, 0x00, 0x03, 0x02, 0x68, 0x32]),
        |w, _| {
            w.s(&[0xfe, 0x0d]);
            w.begin();
            w.s(&[0x00, 0x00, 0x01, 0x00, 0x01]);
            w.random(1);
            w.s(&[0x00, 0x20]);
            w.random(32);
            w.begin();
            let n = rand::rng().random_range(0..4usize) * 32 + 144;
            w.random(n);
            w.end();
            w.end();
        },
        |w, _| w.s(&[0xff, 0x01, 0x00, 0x01, 0x00]),
    ];
    let mut rendered: Vec<Vec<u8>> = parts
        .iter()
        .map(|f| {
            let mut sub = Hello::new(grease);
            f(&mut sub, domain.as_bytes());
            sub.buf
        })
        .collect();
    let mut rng = rand::rng();
    for i in (1..rendered.len()).rev() {
        let j = rng.random_range(0..=i);
        rendered.swap(i, j);
    }
    for part in rendered {
        h.s(&part);
    }
    h.grease(3);
    h.s(&[0x00, 0x01, 0x00]);
    if h.buf.len() < 513 {
        let size = 513 - h.buf.len();
        h.s(&[0x00, 0x15]);
        h.begin();
        h.s(&vec![0u8; size]);
        h.end();
    }
    h.end();
    h.end();
    h.end();

    let mut data = h.buf;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    mac.update(&data);
    let mut digest: [u8; 32] = mac.finalize().into_bytes().into();
    let now = super::clock::now_unix() as i32;
    let old = i32::from_le_bytes([digest[28], digest[29], digest[30], digest[31]]);
    digest[28..32].copy_from_slice(&(old ^ now).to_le_bytes());
    data[11..43].copy_from_slice(&digest);
    data
}

/// Does the fake TLS handshake on `s`. On success the proxy expects TLS
/// application-data records.
pub async fn tls_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    domain: &str,
    key: &[u8; 16],
) -> anyhow::Result<()> {
    let hello = client_hello(domain, key);
    let client_random: [u8; 32] = hello[11..43].try_into().expect("random");
    s.write_all(&hello).await.context("fake TLS hello")?;

    let mut resp = Vec::with_capacity(4096);
    for first in [
        &[0x16u8, 0x03, 0x03][..],
        &[0x14, 0x03, 0x03, 0x00, 0x01, 0x01, 0x17, 0x03, 0x03][..],
    ] {
        let mut head = vec![0u8; first.len() + 2];
        s.read_exact(&mut head)
            .await
            .context("the proxy closed the connection during the TLS handshake (wrong secret, or it is blocked)")?;
        if &head[..first.len()] != first {
            bail!("the proxy did not answer like a fake-TLS MTProto proxy");
        }
        let len = u16::from_be_bytes([head[first.len()], head[first.len() + 1]]) as usize;
        resp.extend_from_slice(&head);
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await?;
        resp.extend_from_slice(&body);
    }
    anyhow::ensure!(resp.len() >= 43, "short server hello");
    let server_random: [u8; 32] = resp[11..43].try_into().expect("random");
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    mac.update(&client_random);
    mac.update(&resp[..11]);
    mac.update(&[0u8; 32]);
    mac.update(&resp[43..]);
    let expected: [u8; 32] = mac.finalize().into_bytes().into();
    if expected != server_random {
        bail!("the proxy's TLS answer is not signed with this secret (wrong secret, or the clock is off)");
    }
    s.write_all(&[0x14, 0x03, 0x03, 0x00, 0x01, 0x01]).await?;
    Ok(())
}

/// Wraps bytes into TLS application-data records.
pub fn tls_records(data: &[u8], out: &mut Vec<u8>) {
    for chunk in data.chunks(TLS_MAX_RECORD) {
        out.extend_from_slice(&[0x17, 0x03, 0x03]);
        out.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        out.extend_from_slice(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_decrypts_to_tag_and_dc() {
        let secret = [9u8; 16];
        let (wire, _) = init(TAG_PADDED, -4, Some(&secret));
        // A server rebuilds the encryptor from the plain part and decrypts
        // the tail: it must read back our tag and DC.
        let mut d = Sha256::new();
        d.update(&wire[8..40]);
        d.update(secret);
        let k: [u8; 32] = d.finalize().into();
        let mut c = Ctr::new((&k).into(), wire[40..56].into());
        let mut buf = wire;
        c.apply_keystream(&mut buf);
        assert_eq!(
            u32::from_le_bytes(buf[56..60].try_into().unwrap()),
            TAG_PADDED
        );
        assert_eq!(i16::from_le_bytes(buf[60..62].try_into().unwrap()), -4);
    }

    #[test]
    fn hello_shape() {
        let h = client_hello("www.google.com", &[1u8; 16]);
        assert_eq!(&h[..3], &[0x16, 0x03, 0x01]);
        let rec = u16::from_be_bytes([h[3], h[4]]) as usize;
        assert_eq!(rec + 5, h.len());
        assert!(h.len() >= 517);
        assert!(h.windows(14).any(|w| w == b"www.google.com"));
    }

    #[test]
    fn records_split() {
        let mut out = Vec::new();
        tls_records(&vec![0u8; TLS_MAX_RECORD * 2 + 1], &mut out);
        assert_eq!(out.len(), TLS_MAX_RECORD * 2 + 1 + 15);
    }
}
