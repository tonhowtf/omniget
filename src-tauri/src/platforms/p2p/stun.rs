use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

const STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

const MAGIC_COOKIE: u32 = 0x2112A442;
const ATTR_XOR_MAPPED: u16 = 0x0020;
const ATTR_MAPPED: u16 = 0x0001;

pub async fn discover_public_endpoint(socket: &UdpSocket) -> anyhow::Result<SocketAddr> {
    for server in STUN_SERVERS {
        match query(socket, server).await {
            Ok(ep) => {
                tracing::info!("[stun] public endpoint: {} (via {})", ep, server);
                return Ok(ep);
            }
            Err(e) => tracing::debug!("[stun] {} failed: {}", server, e),
        }
    }
    anyhow::bail!("All STUN servers failed")
}

async fn query(socket: &UdpSocket, server: &str) -> anyhow::Result<SocketAddr> {
    let addr: SocketAddr = tokio::net::lookup_host(server)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("DNS failed for {}", server))?;

    let mut req = [0u8; 20];
    req[0..2].copy_from_slice(&0x0001u16.to_be_bytes());
    req[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    req[8..20].copy_from_slice(&nanos.to_le_bytes()[..12]);
    let txn_id: [u8; 12] = req[8..20].try_into().unwrap();

    socket.send_to(&req, addr).await?;

    let mut buf = [0u8; 512];
    let (len, _) = tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buf))
        .await
        .map_err(|_| anyhow::anyhow!("STUN timeout"))??;

    parse_response(&buf[..len], &txn_id)
}

fn parse_response(data: &[u8], txn_id: &[u8; 12]) -> anyhow::Result<SocketAddr> {
    if data.len() < 20 {
        anyhow::bail!("response too short");
    }
    if u16::from_be_bytes([data[0], data[1]]) != 0x0101 {
        anyhow::bail!("not a binding response");
    }
    if &data[8..20] != txn_id {
        anyhow::bail!("transaction id mismatch");
    }

    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let end = (20 + msg_len).min(data.len());
    let mut pos = 20;

    while pos + 4 <= end {
        let atype = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let alen = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        let aend = (pos + 4 + alen).min(end);

        if aend - pos - 4 >= 8 {
            let body = &data[pos + 4..aend];
            if body[1] == 0x01 {
                if atype == ATTR_XOR_MAPPED {
                    let port =
                        u16::from_be_bytes([body[2], body[3]]) ^ (MAGIC_COOKIE >> 16) as u16;
                    let ip = u32::from_be_bytes([body[4], body[5], body[6], body[7]])
                        ^ MAGIC_COOKIE;
                    return Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port));
                }
                if atype == ATTR_MAPPED {
                    let port = u16::from_be_bytes([body[2], body[3]]);
                    return Ok(SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::new(body[4], body[5], body[6], body[7])),
                        port,
                    ));
                }
            }
        }

        pos += 4 + ((alen + 3) & !3);
    }

    anyhow::bail!("no mapped address in STUN response")
}
