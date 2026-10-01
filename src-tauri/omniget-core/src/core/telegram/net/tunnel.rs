use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::route::{Route, RouteKind};

pub trait IoStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> IoStream for T {}
pub type BoxStream = Pin<Box<dyn IoStream>>;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub enum Target {
    Ip(SocketAddr),
    Host(String, u16),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Ip(a) => write!(f, "{a}"),
            Target::Host(h, p) => write!(f, "{h}:{p}"),
        }
    }
}

pub async fn open(route: &Route, target: &Target, prefer_doh: bool) -> anyhow::Result<BoxStream> {
    let fut = async {
        match route.kind {
            RouteKind::Direct => {
                let addr = match target {
                    Target::Ip(a) => *a,
                    Target::Host(h, p) => {
                        let ips = super::dns::resolve(h, prefer_doh).await?;
                        SocketAddr::new(ips[0], *p)
                    }
                };
                let s = TcpStream::connect(addr)
                    .await
                    .with_context(|| format!("connect {addr}"))?;
                let _ = s.set_nodelay(true);
                Ok(Box::pin(s) as BoxStream)
            }
            RouteKind::Socks5 => {
                let s = connect_proxy_host(route, prefer_doh).await?;
                Ok(Box::pin(socks5_connect(s, route, target).await?) as BoxStream)
            }
            RouteKind::Http => {
                let s = connect_proxy_host(route, prefer_doh).await?;
                Ok(Box::pin(http_connect(s, route, target).await?) as BoxStream)
            }
            RouteKind::Shadowsocks => {
                let s = connect_proxy_host(route, prefer_doh).await?;
                Ok(Box::pin(super::shadowsocks::connect(s, route, target).await?) as BoxStream)
            }
            RouteKind::Trojan => {
                let s = connect_proxy_host(route, prefer_doh).await?;
                Ok(super::trojan::connect(s, route, target).await?)
            }
            RouteKind::MtProxy => bail!("MTProto proxies are dialed by mtproxy::open"),
        }
    };
    tokio::time::timeout(CONNECT_TIMEOUT, fut)
        .await
        .map_err(|_| anyhow!("timed out reaching {} via {}", target, route.name))?
}

pub async fn connect_proxy_host(route: &Route, prefer_doh: bool) -> anyhow::Result<TcpStream> {
    let ips = super::dns::resolve(&route.host, prefer_doh).await?;
    let mut last = None;
    for ip in ips.iter().take(3) {
        match TcpStream::connect(SocketAddr::new(*ip, route.port)).await {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(anyhow!(
        "cannot reach {} {}:{} ({})",
        route.kind.label(),
        route.host,
        route.port,
        last.map(|e| e.to_string())
            .unwrap_or_else(|| "no address".into())
    ))
}

pub fn socks_addr(target: &Target) -> Vec<u8> {
    let mut out = Vec::with_capacity(22);
    match target {
        Target::Ip(SocketAddr::V4(a)) => {
            out.push(1);
            out.extend_from_slice(&a.ip().octets());
            out.extend_from_slice(&a.port().to_be_bytes());
        }
        Target::Ip(SocketAddr::V6(a)) => {
            out.push(4);
            out.extend_from_slice(&a.ip().octets());
            out.extend_from_slice(&a.port().to_be_bytes());
        }
        Target::Host(h, p) => {
            out.push(3);
            out.push(h.len().min(255) as u8);
            out.extend_from_slice(&h.as_bytes()[..h.len().min(255)]);
            out.extend_from_slice(&p.to_be_bytes());
        }
    }
    out
}

pub async fn socks5_connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    route: &Route,
    target: &Target,
) -> anyhow::Result<S> {
    let auth = route.username.is_some();
    if auth {
        s.write_all(&[5, 2, 0, 2]).await?;
    } else {
        s.write_all(&[5, 1, 0]).await?;
    }
    let mut hello = [0u8; 2];
    s.read_exact(&mut hello).await.context("SOCKS5 greeting")?;
    if hello[0] != 5 {
        bail!("{}:{} is not a SOCKS5 proxy", route.host, route.port);
    }
    match hello[1] {
        0 => {}
        2 => {
            let u = route.username.clone().unwrap_or_default();
            let p = route.password.clone().unwrap_or_default();
            let mut msg = vec![1, u.len() as u8];
            msg.extend_from_slice(u.as_bytes());
            msg.push(p.len() as u8);
            msg.extend_from_slice(p.as_bytes());
            s.write_all(&msg).await?;
            let mut r = [0u8; 2];
            s.read_exact(&mut r).await?;
            if r[1] != 0 {
                bail!("SOCKS5 proxy rejected the username or password");
            }
        }
        0xff => bail!("SOCKS5 proxy needs a username and password"),
        m => bail!("SOCKS5 proxy asked for unsupported auth method {m}"),
    }
    let mut req = vec![5, 1, 0];
    req.extend(socks_addr(target));
    s.write_all(&req).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head)
        .await
        .context("SOCKS5 connect reply")?;
    if head[1] != 0 {
        bail!(
            "SOCKS5 proxy could not reach {target}: {}",
            socks_reply_text(head[1])
        );
    }
    let skip = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await?;
            l[0] as usize + 2
        }
        _ => bail!("bad SOCKS5 reply"),
    };
    let mut rest = vec![0u8; skip];
    s.read_exact(&mut rest).await?;
    Ok(s)
}

fn socks_reply_text(code: u8) -> &'static str {
    match code {
        1 => "general failure",
        2 => "not allowed by the proxy rules",
        3 => "network unreachable",
        4 => "host unreachable",
        5 => "connection refused",
        6 => "TTL expired",
        7 => "command not supported",
        8 => "address type not supported",
        _ => "unknown error",
    }
}

pub async fn http_connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    route: &Route,
    target: &Target,
) -> anyhow::Result<S> {
    use base64::Engine;
    let hp = match target {
        Target::Ip(SocketAddr::V6(a)) => format!("[{}]:{}", a.ip(), a.port()),
        other => other.to_string(),
    };
    let mut req =
        format!("CONNECT {hp} HTTP/1.1\r\nHost: {hp}\r\nProxy-Connection: keep-alive\r\n");
    if let Some(u) = &route.username {
        let cred = format!("{u}:{}", route.password.as_deref().unwrap_or(""));
        req.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64::engine::general_purpose::STANDARD.encode(cred)
        ));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await?;
    let mut head = Vec::with_capacity(256);
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 {
            bail!("HTTP proxy sent an oversized reply");
        }
        s.read_exact(&mut b).await.context("HTTP proxy reply")?;
        head.push(b[0]);
    }
    let status = String::from_utf8_lossy(&head);
    let line = status.lines().next().unwrap_or("");
    let code = line.split_whitespace().nth(1).unwrap_or("");
    match code {
        "200" => Ok(s),
        "407" => bail!("HTTP proxy needs a username and password"),
        _ => bail!("HTTP proxy refused the tunnel to {target}: {line}"),
    }
}
