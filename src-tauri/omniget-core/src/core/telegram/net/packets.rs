//! A packet connection to one Telegram DC over any route. Callers send and
//! receive whole MTProto payloads; underneath, every connection is
//! obfuscated2 (direct connections too, since plain MTProto is trivial for
//! DPI to spot), padded when the proxy asks for it, and wrapped in fake-TLS
//! records for `ee` secrets.

use std::time::Duration;

use aes::cipher::StreamCipher;
use rand::RngExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::obfs::{self, ProxySecret, TAG_INTERMEDIATE};
use super::route::{Route, RouteKind};
use super::tunnel::{self, BoxStream, Target};
use super::{dc, NetOpts};

pub struct PacketConn {
    pub tx: mpsc::Sender<Vec<u8>>,
    pub rx: mpsc::Receiver<Vec<u8>>,
}

pub async fn open(route: &Route, dc_id: i16, opts: &NetOpts) -> anyhow::Result<PacketConn> {
    let fut = async {
        match route.kind {
            RouteKind::MtProxy => {
                let secret = ProxySecret::parse(route.secret.as_deref().unwrap_or(""))?;
                let mut tcp = tunnel::connect_proxy_host(route, opts.prefer_doh).await?;
                if let Some(domain) = &secret.tls_domain {
                    obfs::tls_handshake(&mut tcp, domain, &secret.key).await?;
                }
                let (wire, ciphers) = obfs::init(secret.tag(), dc_id, Some(&secret.key));
                Ok(spawn(
                    Box::pin(tcp),
                    wire,
                    ciphers,
                    secret.padded,
                    secret.tls_domain.is_some(),
                ))
            }
            _ => {
                let target = Target::Ip(dc::addr(dc_id, opts.prefer_ipv6));
                let s = tunnel::open(route, &target, opts.prefer_doh).await?;
                let (wire, ciphers) = obfs::init(TAG_INTERMEDIATE, dc_id, None);
                Ok(spawn(s, wire, ciphers, false, false))
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(opts.connect_timeout_secs.max(3)), fut)
        .await
        .map_err(|_| anyhow::anyhow!("timed out connecting through {}", route.name))?
}

fn spawn(
    stream: BoxStream,
    init: [u8; 64],
    ciphers: obfs::Obfs,
    padded: bool,
    tls: bool,
) -> PacketConn {
    let (app_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (in_tx, app_rx) = mpsc::channel::<Vec<u8>>(64);
    let (mut r, mut w) = tokio::io::split(stream);
    let obfs::Obfs { mut enc, mut dec } = ciphers;

    tokio::spawn(async move {
        let mut first = Some(init);
        while let Some(payload) = out_rx.recv().await {
            let pad = if padded {
                rand::rng().random_range(0..16usize)
            } else {
                0
            };
            let mut frame = Vec::with_capacity(payload.len() + 4 + pad);
            frame.extend_from_slice(&((payload.len() + pad) as u32).to_le_bytes());
            frame.extend_from_slice(&payload);
            if pad > 0 {
                let mut p = vec![0u8; pad];
                rand::rng().fill(&mut p[..]);
                frame.extend_from_slice(&p);
            }
            enc.apply_keystream(&mut frame);
            let mut plain = first.take().map(|i| i.to_vec()).unwrap_or_default();
            plain.extend_from_slice(&frame);
            let out = if tls {
                let mut o = Vec::with_capacity(plain.len() + 16);
                obfs::tls_records(&plain, &mut o);
                o
            } else {
                plain
            };
            if w.write_all(&out).await.is_err() {
                break;
            }
        }
        let _ = w.shutdown().await;
    });

    tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        let mut chunk = vec![0u8; 64 * 1024];
        'outer: loop {
            let n = if tls {
                let mut head = [0u8; 5];
                if r.read_exact(&mut head).await.is_err() {
                    break;
                }
                if head[..3] != [0x17, 0x03, 0x03] {
                    tracing::debug!("[tg-net] unexpected TLS record {:02x?}", &head[..3]);
                    break;
                }
                let len = u16::from_be_bytes([head[3], head[4]]) as usize;
                if chunk.len() < len {
                    chunk.resize(len, 0);
                }
                if r.read_exact(&mut chunk[..len]).await.is_err() {
                    break;
                }
                len
            } else {
                match r.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                }
            };
            dec.apply_keystream(&mut chunk[..n]);
            buf.extend_from_slice(&chunk[..n]);
            loop {
                if buf.len() < 4 {
                    break;
                }
                let len =
                    (u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) & 0x7fff_ffff) as usize;
                if len > 16 * 1024 * 1024 {
                    tracing::debug!("[tg-net] absurd frame length {len}, dropping connection");
                    break 'outer;
                }
                if buf.len() < 4 + len {
                    break;
                }
                let payload = &buf[4..4 + len];
                let payload = if padded { unpad(payload) } else { payload };
                if payload.len() < 8 {
                    let code = payload
                        .get(..4)
                        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .unwrap_or(0);
                    tracing::warn!(
                        "[tg-net] transport error {code} (frame {len} bytes, padded={padded})"
                    );
                }
                if in_tx.send(payload.to_vec()).await.is_err() {
                    break 'outer;
                }
                buf.drain(..4 + len);
            }
        }
    });

    PacketConn {
        tx: app_tx,
        rx: app_rx,
    }
}

/// Drops the random tail a padded-intermediate peer appends. MTProto
/// payloads carry their own length: errors are 4 bytes, plaintext messages
/// state it at offset 16, encrypted ones are 24 + a multiple of 16.
pub fn unpad(p: &[u8]) -> &[u8] {
    if p.len() >= 20 && p[..8] == [0u8; 8] {
        let l = u32::from_le_bytes([p[16], p[17], p[18], p[19]]) as usize;
        return &p[..(20 + l).min(p.len())];
    }
    if p.len() < 24 {
        return &p[..p.len().min(4)];
    }
    &p[..24 + (p.len() - 24) / 16 * 16]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpad_rules() {
        assert_eq!(unpad(&[0x6c, 0xfe, 0xff, 0xff, 1, 2, 3]).len(), 4);
        let mut plain = vec![0u8; 20];
        plain[16] = 8;
        plain.extend([1u8; 8 + 5]);
        assert_eq!(unpad(&plain).len(), 28);
        let mut enc = vec![1u8; 24 + 32];
        enc.extend([9u8; 7]);
        assert_eq!(unpad(&enc).len(), 56);
    }
}
