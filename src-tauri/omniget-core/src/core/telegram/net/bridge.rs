use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use anyhow::{bail, Context};
use rand::RngExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::route::Route;
use super::{dc, packets, NetOpts};

pub trait RouteProvider: Send + Sync + 'static {
    fn routes(&self) -> Vec<Route>;
    fn opts(&self) -> NetOpts;
    fn report(&self, route_id: &str, ok: bool, error: Option<String>);
}

#[derive(Clone, Debug)]
pub struct Bridge {
    pub port: u16,
    pub user: String,
    pub pass: String,
}

impl Bridge {
    pub fn url(&self) -> String {
        format!(
            "socks5://{}:{}@127.0.0.1:{}",
            self.user, self.pass, self.port
        )
    }
}

pub async fn start(provider: Arc<dyn RouteProvider>) -> anyhow::Result<Bridge> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("bind bridge")?;
    let port = listener.local_addr()?.port();
    let token = |n: usize| -> String {
        let mut b = vec![0u8; n];
        rand::rng().fill(&mut b[..]);
        hex::encode(b)
    };
    let bridge = Bridge {
        port,
        user: token(6),
        pass: token(12),
    };
    let auth = (bridge.user.clone(), bridge.pass.clone());
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                continue;
            };
            let provider = provider.clone();
            let auth = auth.clone();
            tokio::spawn(async move {
                if let Err(e) = serve(sock, provider, auth).await {
                    tracing::debug!("[tg-net] bridge connection ended: {e:#}");
                }
            });
        }
    });
    tracing::info!("[tg-net] bridge listening on 127.0.0.1:{port}");
    Ok(bridge)
}

async fn serve(
    mut sock: TcpStream,
    provider: Arc<dyn RouteProvider>,
    auth: (String, String),
) -> anyhow::Result<()> {
    let _ = sock.set_nodelay(true);
    let target = socks_handshake(&mut sock, &auth).await?;
    let dc_id = dc::dc_for(target.ip());
    let opts = provider.opts();

    let mut last_err = None;
    let mut conn = None;
    for route in provider.routes() {
        if route.dc_only.is_some_and(|only| only != dc_id) {
            continue;
        }
        match packets::open(&route, dc_id, &opts).await {
            Ok(c) => {
                provider.report(&route.id, true, None);
                tracing::info!("[tg-net] DC{dc_id} ({target}) via {}", route.name);
                conn = Some(c);
                break;
            }
            Err(e) => {
                provider.report(&route.id, false, Some(format!("{e:#}")));
                last_err = Some(e);
            }
        }
    }
    let Some(conn) = conn else {
        let _ = sock.write_all(&[5, 4, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        bail!(
            "no route reached DC{dc_id}: {}",
            last_err
                .map(|e| format!("{e:#}"))
                .unwrap_or_else(|| "no routes".into())
        );
    };
    sock.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
    pump_full(sock, conn).await
}

async fn socks_handshake(
    sock: &mut TcpStream,
    auth: &(String, String),
) -> anyhow::Result<SocketAddr> {
    let mut head = [0u8; 2];
    sock.read_exact(&mut head).await?;
    anyhow::ensure!(head[0] == 5, "not SOCKS5");
    let mut methods = vec![0u8; head[1] as usize];
    sock.read_exact(&mut methods).await?;
    if !methods.contains(&2) {
        sock.write_all(&[5, 0xff]).await?;
        bail!("client did not offer username/password");
    }
    sock.write_all(&[5, 2]).await?;
    let mut ver = [0u8; 2];
    sock.read_exact(&mut ver).await?;
    let mut user = vec![0u8; ver[1] as usize];
    sock.read_exact(&mut user).await?;
    let mut plen = [0u8; 1];
    sock.read_exact(&mut plen).await?;
    let mut pass = vec![0u8; plen[0] as usize];
    sock.read_exact(&mut pass).await?;
    if user != auth.0.as_bytes() || pass != auth.1.as_bytes() {
        sock.write_all(&[1, 1]).await?;
        bail!("bad bridge credentials");
    }
    sock.write_all(&[1, 0]).await?;

    let mut req = [0u8; 4];
    sock.read_exact(&mut req).await?;
    anyhow::ensure!(req[1] == 1, "only CONNECT is supported");
    let ip: IpAddr = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            sock.read_exact(&mut a).await?;
            a.into()
        }
        4 => {
            let mut a = [0u8; 16];
            sock.read_exact(&mut a).await?;
            a.into()
        }
        3 => {
            let mut l = [0u8; 1];
            sock.read_exact(&mut l).await?;
            let mut name = vec![0u8; l[0] as usize];
            sock.read_exact(&mut name).await?;
            let name = String::from_utf8_lossy(&name).to_string();
            name.parse().unwrap_or(IpAddr::from([149, 154, 167, 51]))
        }
        _ => bail!("bad address type"),
    };
    let mut port = [0u8; 2];
    sock.read_exact(&mut port).await?;
    Ok(SocketAddr::new(ip, u16::from_be_bytes(port)))
}

async fn pump_full(sock: TcpStream, conn: packets::PacketConn) -> anyhow::Result<()> {
    let packets::PacketConn { tx, mut rx } = conn;
    let (mut r, mut w) = sock.into_split();

    let upstream = tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let n = match r.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            buf.extend_from_slice(&chunk[..n]);
            while buf.len() >= 4 {
                let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                if !(12..=16 * 1024 * 1024).contains(&len) {
                    return;
                }
                if buf.len() < len {
                    break;
                }
                let payload = buf[8..len - 4].to_vec();
                buf.drain(..len);
                if tx.send(payload).await.is_err() {
                    return;
                }
            }
        }
    });

    let mut seq: u32 = 0;
    while let Some(payload) = rx.recv().await {
        let total = payload.len() + 12;
        let mut frame = Vec::with_capacity(total);
        frame.extend_from_slice(&(total as u32).to_le_bytes());
        frame.extend_from_slice(&seq.to_le_bytes());
        frame.extend_from_slice(&payload);
        let crc = crc32fast::hash(&frame);
        frame.extend_from_slice(&crc.to_le_bytes());
        seq = seq.wrapping_add(1);
        if w.write_all(&frame).await.is_err() {
            break;
        }
    }
    upstream.abort();
    Ok(())
}
