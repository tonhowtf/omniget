use std::net::Ipv4Addr;

use aes::cipher::{BlockDecryptMut, KeyIvInit};
use anyhow::{anyhow, bail, ensure};
use base64::Engine;
use num_bigint::BigUint;
use sha2::{Digest, Sha256};

use super::route::{Route, RouteKind, RouteSource};

const DOMAIN: &str = "apv3.stel.com";
const MODULUS_HEX: &str = "cabfb5f117b1da886d572f2cae818f0705c3dc33a82824a98c3a98a17802a81ee2a259f87830857ce05495f5d412f33f7e7282a45e3a56401fb656f856e3c3790492fd9c596041aa1dacb096ba159d71c88e0aa8c6201dd7ddb1446adeb9721a50a9a4c2533d2480fd592da352c4e9cd0f752fc3043cb27fe33ac1c19b9af16e5f102c020a1d9cf46e48cf3066fa8b4c4bf60ac26475a25cd10b2124c801235d6a8123d16dbf9786f26d15901cce1bae7958861dc95d077c32bf35672f1aa6b4c3f9eb88c0fa9838ca3abc9a9b0d0e3fd42e62030dd02f71316f72f1298fe21f763805a87536206358590b176fe9cb395275bdf8f5af4be5886e487e19a598c7";

const CONFIG_SIMPLE: u32 = 0x5a59_2a6c;
const IP_PORT: u32 = 0xd433_ad73;
const IP_PORT_SECRET: u32 = 0x3798_2646;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RescueEndpoint {
    pub dc_id: i32,
    pub ip: String,
    pub port: u16,
    pub secret: Option<String>,
}

pub async fn fetch() -> anyhow::Result<Vec<RescueEndpoint>> {
    let mut last = anyhow!("no DNS server answered");
    for (name, ip, path) in super::dns::DOH_SERVERS {
        match txt_via_doh(name, ip, path).await {
            Ok(parts) if !parts.is_empty() => match decode_parts(parts) {
                Ok(v) => return Ok(v),
                Err(e) => last = e,
            },
            Ok(_) => {}
            Err(e) => last = e,
        }
    }
    Err(last)
}

pub fn routes(endpoints: &[RescueEndpoint]) -> Vec<Route> {
    let mut out = Vec::new();
    for e in endpoints {
        let Some(secret) = &e.secret else { continue };
        let mut r = Route::new(
            RouteKind::MtProxy,
            e.ip.clone(),
            e.port,
            RouteSource::PublicList {
                list: "telegram-rescue".into(),
            },
        );
        r.secret = Some(secret.clone());
        r.dc_only = Some(e.dc_id as i16);
        r.name = format!("Telegram rescue DC{} · {}:{}", e.dc_id, e.ip, e.port);
        r.finish();
        if !out.iter().any(|x: &Route| x.id == r.id) {
            out.push(r);
        }
    }
    out
}

async fn txt_via_doh(name: &str, ip: &str, path: &str) -> anyhow::Result<Vec<String>> {
    let addr: std::net::SocketAddr = format!("{ip}:443").parse()?;
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .resolve(name, addr)
        .timeout(std::time::Duration::from_secs(6))
        .no_proxy()
        .build()?;
    let q = super::dns::build_query(DOMAIN, 16);
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(q);
    let body = client
        .get(format!("https://{name}{path}?dns={b64}"))
        .header("accept", "application/dns-message")
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    parse_txt(&body)
}

fn parse_txt(msg: &[u8]) -> anyhow::Result<Vec<String>> {
    ensure!(msg.len() >= 12, "short DNS reply");
    let qd = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let an = u16::from_be_bytes([msg[6], msg[7]]) as usize;
    let skip = |mut p: usize| -> anyhow::Result<usize> {
        loop {
            let l = *msg.get(p).ok_or_else(|| anyhow!("bad name"))?;
            if l & 0xc0 == 0xc0 {
                return Ok(p + 2);
            }
            if l == 0 {
                return Ok(p + 1);
            }
            p += 1 + l as usize;
        }
    };
    let mut pos = 12;
    for _ in 0..qd {
        pos = skip(pos)? + 4;
    }
    let mut out = Vec::new();
    for _ in 0..an {
        pos = skip(pos)?;
        ensure!(pos + 10 <= msg.len(), "truncated");
        let rtype = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
        let rdlen = u16::from_be_bytes([msg[pos + 8], msg[pos + 9]]) as usize;
        pos += 10;
        ensure!(pos + rdlen <= msg.len(), "truncated");
        if rtype == 16 {
            let mut s = String::new();
            let mut i = pos;
            while i < pos + rdlen {
                let l = msg[i] as usize;
                s.push_str(&String::from_utf8_lossy(
                    &msg[i + 1..(i + 1 + l).min(pos + rdlen)],
                ));
                i += 1 + l;
            }
            out.push(s);
        }
        pos += rdlen;
    }
    Ok(out)
}

pub fn decode_parts(mut parts: Vec<String>) -> anyhow::Result<Vec<RescueEndpoint>> {
    parts.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let joined: String = parts.concat();
    decode(&joined)
}

pub fn decode(text: &str) -> anyhow::Result<Vec<RescueEndpoint>> {
    let b64: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    let data = base64::engine::general_purpose::STANDARD.decode(b64)?;
    ensure!(data.len() == 256, "rescue record has the wrong size");
    let n = BigUint::parse_bytes(MODULUS_HEX.as_bytes(), 16).expect("modulus");
    let y = BigUint::from_bytes_be(&data).modpow(&BigUint::from(65537u32), &n);
    let mut y_bytes = y.to_bytes_be();
    while y_bytes.len() < 256 {
        y_bytes.insert(0, 0);
    }
    let key: [u8; 32] = y_bytes[..32].try_into()?;
    let iv: [u8; 16] = y_bytes[16..32].try_into()?;
    let mut cbc = y_bytes[32..].to_vec();
    ensure!(cbc.len() == 224, "bad payload");
    let dec = cbc::Decryptor::<aes::Aes256>::new((&key).into(), (&iv).into());
    dec.decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut cbc)
        .map_err(|_| anyhow!("rescue decrypt"))?;
    let hash = Sha256::digest(&cbc[..208]);
    ensure!(hash[..16] == cbc[208..], "rescue record signature mismatch");
    parse_config_simple(&cbc[..208])
}

fn parse_config_simple(buf: &[u8]) -> anyhow::Result<Vec<RescueEndpoint>> {
    let mut r = Reader { b: buf, p: 0 };
    let len = r.i32()? as usize;
    ensure!(r.u32()? == CONFIG_SIMPLE, "not a help.configSimple");
    let _ = len;
    let _date = r.i32()?;
    let _expires = r.i32()?;
    let rules = r.i32()?;
    let mut out = Vec::new();
    for _ in 0..rules.clamp(0, 64) {
        let _ctor = r.u32()?;
        let _phone_rules = r.bytes()?;
        let dc_id = r.i32()?;
        let n = r.i32()?;
        for _ in 0..n.clamp(0, 64) {
            let ctor = r.u32()?;
            let ip = Ipv4Addr::from(r.u32()?);
            let port = r.i32()? as u16;
            let secret = match ctor {
                IP_PORT_SECRET => Some(hex::encode(r.bytes()?)),
                IP_PORT => None,
                _ => bail!("unknown endpoint type"),
            };
            out.push(RescueEndpoint {
                dc_id,
                ip: ip.to_string(),
                port,
                secret,
            });
        }
    }
    Ok(out)
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> anyhow::Result<u32> {
        let v = self
            .b
            .get(self.p..self.p + 4)
            .ok_or_else(|| anyhow!("truncated"))?;
        self.p += 4;
        Ok(u32::from_le_bytes(v.try_into()?))
    }
    fn i32(&mut self) -> anyhow::Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn bytes(&mut self) -> anyhow::Result<Vec<u8>> {
        let first = *self.b.get(self.p).ok_or_else(|| anyhow!("truncated"))? as usize;
        let (len, head) = if first < 254 {
            (first, 1)
        } else {
            let v = self
                .b
                .get(self.p + 1..self.p + 4)
                .ok_or_else(|| anyhow!("truncated"))?;
            (
                v[0] as usize | (v[1] as usize) << 8 | (v[2] as usize) << 16,
                4,
            )
        };
        let start = self.p + head;
        let v = self
            .b
            .get(start..start + len)
            .ok_or_else(|| anyhow!("truncated"))?
            .to_vec();
        self.p = (start + len + 3) & !3;
        Ok(v)
    }
}
