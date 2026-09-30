//! End-to-end route test: send `req_pq_multi` to a DC and wait for the
//! matching `resPQ`. A TCP connect proves nothing behind the GFW (the
//! handshake often succeeds and the stream is killed later); an answer
//! from Telegram with our nonce echoed back proves the whole path works.
//! The same round trip is the "ping" shown next to each route, as in
//! Telegram Desktop and TDLib's `pingProxy`.

use std::time::{Duration, Instant};

use rand::RngExt;
use serde::Serialize;

use super::route::Route;
use super::{clock, packets, NetOpts};

const REQ_PQ_MULTI: u32 = 0xbe7e_8ef1;
const RES_PQ: u32 = 0x0516_2463;

#[derive(Debug, Clone, Serialize)]
pub struct ProbeOk {
    /// Dial + obfuscation/TLS handshake + first round trip.
    pub connect_ms: u64,
    /// The `req_pq` round trip alone.
    pub rtt_ms: u64,
    pub dc: i16,
    /// Server clock minus ours, in seconds.
    pub clock_offset_secs: i64,
}

pub fn req_pq_message(nonce: &[u8; 16]) -> Vec<u8> {
    let now = clock::now_unix();
    let low: u32 = rand::rng().random::<u32>() & !3;
    let msg_id: u64 = ((now as u64) << 32) | low as u64;
    let mut m = Vec::with_capacity(40);
    m.extend_from_slice(&[0u8; 8]);
    m.extend_from_slice(&msg_id.to_le_bytes());
    m.extend_from_slice(&20u32.to_le_bytes());
    m.extend_from_slice(&REQ_PQ_MULTI.to_le_bytes());
    m.extend_from_slice(nonce);
    m
}

pub async fn probe(route: &Route, dc: i16, opts: &NetOpts) -> anyhow::Result<ProbeOk> {
    let start = Instant::now();
    let mut conn = packets::open(route, dc, opts).await?;
    let mut nonce = [0u8; 16];
    rand::rng().fill(&mut nonce[..]);
    let sent = Instant::now();
    conn.tx
        .send(req_pq_message(&nonce))
        .await
        .map_err(|_| anyhow::anyhow!("the connection closed before the test was sent"))?;
    let wait = Duration::from_secs(opts.connect_timeout_secs.max(3));
    let resp = tokio::time::timeout(wait, conn.rx.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Telegram did not answer through {} (blocked or too slow)", route.name))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} closed the connection without an answer (wrong secret/password, or the server is blocked)",
                route.name
            )
        })?;
    let rtt = sent.elapsed();
    if resp.len() == 4 {
        let code = i32::from_le_bytes([resp[0], resp[1], resp[2], resp[3]]);
        anyhow::bail!("Telegram answered with transport error {code}");
    }
    anyhow::ensure!(
        resp.len() >= 40 && resp[..8] == [0u8; 8],
        "unexpected answer from Telegram"
    );
    let ctor = u32::from_le_bytes([resp[20], resp[21], resp[22], resp[23]]);
    anyhow::ensure!(ctor == RES_PQ, "unexpected answer from Telegram");
    anyhow::ensure!(
        resp[24..40] == nonce,
        "the answer did not match our request"
    );
    let msg_id = u64::from_le_bytes(resp[8..16].try_into().expect("8 bytes"));
    let server_time = (msg_id >> 32) as i64;
    clock::observe_server_time(server_time);
    Ok(ProbeOk {
        connect_ms: start.elapsed().as_millis() as u64,
        rtt_ms: rtt.as_millis() as u64,
        dc,
        clock_offset_secs: server_time - clock::system_unix(),
    })
}
