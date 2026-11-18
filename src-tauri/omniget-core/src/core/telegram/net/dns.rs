use std::net::IpAddr;
use std::time::Duration;

use base64::Engine;
use rand::RngExt;

pub const DOH_SERVERS: &[(&str, &str, &str)] = &[
    ("dns.alidns.com", "223.5.5.5", "/dns-query"),
    ("dns.alidns.com", "223.6.6.6", "/dns-query"),
    ("doh.pub", "1.12.12.12", "/dns-query"),
    ("doh.pub", "120.53.53.53", "/dns-query"),
    ("cloudflare-dns.com", "1.1.1.1", "/dns-query"),
    ("dns.google", "8.8.8.8", "/dns-query"),
    ("dns.quad9.net", "9.9.9.9", "/dns-query"),
];

const SYSTEM_TIMEOUT: Duration = Duration::from_secs(3);
const DOH_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn resolve(host: &str, prefer_doh: bool) -> anyhow::Result<Vec<IpAddr>> {
    let host = host.trim_matches(['[', ']']);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    if !prefer_doh {
        let sys = tokio::time::timeout(SYSTEM_TIMEOUT, tokio::net::lookup_host((host, 0))).await;
        if let Ok(Ok(addrs)) = sys {
            let ips: Vec<IpAddr> = addrs.map(|a| a.ip()).filter(|ip| !is_bogus(ip)).collect();
            if !ips.is_empty() {
                return Ok(ips);
            }
        }
    }
    resolve_doh(host).await
}

fn is_bogus(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_unspecified() || v4.is_loopback() || v4.octets()[0] == 0,
        IpAddr::V6(v6) => v6.is_unspecified() || v6.is_loopback(),
    }
}

const OUTSIDE_SERVERS: &[&str] = &["cloudflare-dns.com", "dns.google", "dns.quad9.net"];

pub async fn resolve_doh(host: &str) -> anyhow::Result<Vec<IpAddr>> {
    resolve_doh_with(host, None).await
}

pub async fn resolve_doh_through(host: &str, proxy: &str) -> anyhow::Result<Vec<IpAddr>> {
    resolve_doh_with(host, Some(proxy)).await
}

async fn resolve_doh_with(host: &str, proxy: Option<&str>) -> anyhow::Result<Vec<IpAddr>> {
    let host = host.to_string();
    let mut tasks = Vec::new();
    for (name, ip, path) in DOH_SERVERS {
        if proxy.is_some() && !OUTSIDE_SERVERS.contains(name) {
            continue;
        }
        for qtype in [1u16, 28u16] {
            let host = host.clone();
            tasks.push(Box::pin(async move {
                doh_query(name, ip, path, &host, qtype, proxy).await
            }));
        }
    }
    let mut found: Vec<IpAddr> = Vec::new();
    let mut pending = tasks;
    let deadline = tokio::time::Instant::now() + DOH_TIMEOUT;
    while !pending.is_empty() {
        let next = futures::future::select_all(pending);
        let Ok((res, _, rest)) = tokio::time::timeout_at(deadline, next).await else {
            break;
        };
        pending = rest;
        if let Ok(ips) = res {
            for ip in ips {
                if !found.contains(&ip) && !is_bogus(&ip) {
                    found.push(ip);
                }
            }
            if found.iter().any(|ip| ip.is_ipv4()) {
                break;
            }
        }
    }
    if found.is_empty() {
        anyhow::bail!("DNS-over-HTTPS could not resolve {host}");
    }
    Ok(found)
}

async fn doh_query(
    name: &str,
    ip: &str,
    path: &str,
    host: &str,
    qtype: u16,
    proxy: Option<&str>,
) -> anyhow::Result<Vec<IpAddr>> {
    let addr: std::net::SocketAddr = format!("{ip}:443").parse()?;
    let builder = reqwest::Client::builder()
        .use_rustls_tls()
        .resolve(name, addr)
        .timeout(DOH_TIMEOUT);
    let client = match proxy {
        Some(proxy) => builder.proxy(reqwest::Proxy::all(proxy)?),
        None => builder.no_proxy(),
    }
    .build()?;
    let query = build_query(host, qtype);
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&query);
    let url = format!("https://{name}{path}?dns={b64}");
    let resp = client
        .get(url)
        .header("accept", "application/dns-message")
        .send()
        .await?
        .error_for_status()?;
    let body = resp.bytes().await?;
    parse_answer(&body, qtype)
}

pub fn build_query(host: &str, qtype: u16) -> Vec<u8> {
    let id: u16 = rand::rng().random();
    let mut q = Vec::with_capacity(32 + host.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in host.trim_end_matches('.').split('.') {
        let l = label.as_bytes();
        q.push(l.len().min(63) as u8);
        q.extend_from_slice(&l[..l.len().min(63)]);
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&[0, 1]);
    q
}

pub fn parse_answer(msg: &[u8], qtype: u16) -> anyhow::Result<Vec<IpAddr>> {
    anyhow::ensure!(msg.len() >= 12, "short DNS reply");
    let rcode = msg[3] & 0x0f;
    anyhow::ensure!(rcode == 0, "DNS rcode {rcode}");
    let qd = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let an = u16::from_be_bytes([msg[6], msg[7]]) as usize;
    let mut pos = 12;
    for _ in 0..qd {
        pos = skip_name(msg, pos)? + 4;
    }
    let mut out = Vec::new();
    for _ in 0..an {
        pos = skip_name(msg, pos)?;
        anyhow::ensure!(pos + 10 <= msg.len(), "truncated answer");
        let rtype = u16::from_be_bytes([msg[pos], msg[pos + 1]]);
        let rdlen = u16::from_be_bytes([msg[pos + 8], msg[pos + 9]]) as usize;
        pos += 10;
        anyhow::ensure!(pos + rdlen <= msg.len(), "truncated rdata");
        let rdata = &msg[pos..pos + rdlen];
        if rtype == qtype {
            match (rtype, rdlen) {
                (1, 4) => out.push(IpAddr::from(<[u8; 4]>::try_from(rdata)?)),
                (28, 16) => out.push(IpAddr::from(<[u8; 16]>::try_from(rdata)?)),
                _ => {}
            }
        }
        pos += rdlen;
    }
    Ok(out)
}

fn skip_name(msg: &[u8], mut pos: usize) -> anyhow::Result<usize> {
    loop {
        let len = *msg
            .get(pos)
            .ok_or_else(|| anyhow::anyhow!("bad DNS name"))?;
        if len & 0xc0 == 0xc0 {
            return Ok(pos + 2);
        }
        if len == 0 {
            return Ok(pos + 1);
        }
        pos += 1 + len as usize;
    }
}
