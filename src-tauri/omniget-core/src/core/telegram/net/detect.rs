//! Finds the proxy the person already has, without asking them anything.
//!
//! In mainland China almost everyone who uses Telegram runs a proxy client
//! (Clash Verge, mihomo party, v2rayN, NekoBox, Hiddify, Shadowrocket…) that
//! listens on a local port. Apps that ignore it look "broken" there, so we
//! collect every hint we can get and let the prober decide which one works:
//!
//! 1. the system proxy (macOS `scutil`, Windows registry, GNOME/KDE, env);
//! 2. the ports written in Clash/mihomo/verge configs on disk;
//! 3. a quick scan of the well-known local ports of the popular clients,
//!    telling SOCKS5 from HTTP by talking to them.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::route::{Route, RouteKind, RouteSource};

/// Default local ports of the proxy clients people run. The mixed ports
/// answer both SOCKS5 and HTTP; we still sniff which one.
pub const KNOWN_LOCAL_PORTS: &[(u16, &str)] = &[
    (7897, "Clash Verge Rev"),
    (7890, "Clash / mihomo / ClashX / FlClash / Stash"),
    (7891, "Clash (SOCKS)"),
    (7898, "Clash Verge Rev (SOCKS)"),
    (7899, "Clash Verge Rev (HTTP)"),
    (10808, "v2rayN"),
    (10809, "v2rayN (HTTP)"),
    (10810, "v2rayN"),
    (2080, "NekoBox / NekoRay"),
    (2081, "NekoBox / NekoRay (HTTP)"),
    (12334, "Hiddify"),
    (20170, "v2rayA"),
    (20171, "v2rayA (HTTP)"),
    (20172, "v2rayA (HTTP, rules)"),
    (1080, "Shadowsocks / SOCKS"),
    (1081, "SOCKS"),
    (1086, "ShadowsocksX-NG"),
    (1087, "ShadowsocksX-NG (HTTP)"),
    (1082, "Shadowrocket / sing-box"),
    (6152, "Surge (HTTP)"),
    (6153, "Surge (SOCKS)"),
    (1089, "Qv2ray"),
    (8889, "Qv2ray (HTTP)"),
    (7893, "mihomo party"),
    (8118, "Privoxy"),
    (9050, "Tor"),
    (9150, "Tor Browser"),
    (10801, "SOCKS"),
    (33210, "Quantumult X"),
];

#[derive(Debug, Clone, Serialize)]
pub struct Detected {
    pub route: Route,
    /// Human explanation: "System proxy", "Clash Verge Rev config", …
    pub hint: String,
}

/// Runs every detector and returns the unique candidates, system proxy
/// and configured ports first, blind scan hits last.
pub async fn detect_all() -> Vec<Detected> {
    let mut out: Vec<Detected> = Vec::new();
    let mut seen: HashSet<(RouteKind, String, u16)> = HashSet::new();
    let mut push = |d: Detected, out: &mut Vec<Detected>| {
        let key = (
            d.route.kind,
            d.route.host.to_ascii_lowercase(),
            d.route.port,
        );
        if seen.insert(key) {
            out.push(d);
        }
    };

    for d in env_proxies() {
        push(d, &mut out);
    }
    for d in tokio::task::spawn_blocking(system_proxies)
        .await
        .unwrap_or_default()
    {
        push(d, &mut out);
    }
    let config_ports = tokio::task::spawn_blocking(clash_config_ports)
        .await
        .unwrap_or_default();
    // Configured ports are sniffed too: `mixed-port` answers both and a stale
    // config may point to a port nobody listens on any more.
    let mut to_sniff: Vec<(u16, String)> = config_ports;
    for (port, app) in KNOWN_LOCAL_PORTS {
        to_sniff.push((*port, app.to_string()));
    }
    let mut dedup_ports = HashSet::new();
    to_sniff.retain(|(p, _)| dedup_ports.insert(*p));

    let sniffs = futures::future::join_all(
        to_sniff
            .into_iter()
            .map(|(port, app)| async move { (port, app, sniff_local(port).await) }),
    )
    .await;
    for (port, app, kinds) in sniffs {
        for kind in kinds {
            let mut r = Route::new(
                kind,
                "127.0.0.1",
                port,
                RouteSource::LocalApp { app: app.clone() },
            );
            r.name = format!("{app} · 127.0.0.1:{port}");
            r.finish();
            push(
                Detected {
                    route: r,
                    hint: app.clone(),
                },
                &mut out,
            );
        }
    }
    out
}

fn env_proxies() -> Vec<Detected> {
    let mut out = Vec::new();
    for var in [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        let Ok(v) = std::env::var(var) else { continue };
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        let with_scheme = if v.contains("://") {
            v.to_string()
        } else {
            format!("http://{v}")
        };
        if let Some(super::links::ParsedLink::Route { mut route }) = super::links::parse_one(
            &with_scheme,
            RouteSource::Env {
                var: var.to_string(),
            },
        ) {
            route.name = format!("{var} · {}:{}", route.host, route.port);
            route.finish();
            out.push(Detected {
                route,
                hint: format!("${var}"),
            });
        }
    }
    out
}

fn system_route(kind: RouteKind, host: &str, port: u16) -> Option<Detected> {
    if host.is_empty() || port == 0 {
        return None;
    }
    let mut r = Route::new(kind, host, port, RouteSource::System);
    r.name = format!("System {} · {host}:{port}", kind.label());
    r.finish();
    Some(Detected {
        route: r,
        hint: "System proxy".into(),
    })
}

#[cfg(target_os = "macos")]
fn system_proxies() -> Vec<Detected> {
    let Ok(out) = std::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
    else {
        return Vec::new();
    };
    parse_scutil(&String::from_utf8_lossy(&out.stdout))
}

/// `scutil --proxy` prints a dictionary like `  SOCKSEnable : 1`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_scutil(text: &str) -> Vec<Detected> {
    let get = |k: &str| -> Option<String> {
        text.lines().find_map(|l| {
            let (key, val) = l.split_once(" : ")?;
            (key.trim() == k).then(|| val.trim().to_string())
        })
    };
    let mut out = Vec::new();
    for (prefix, kind) in [
        ("SOCKS", RouteKind::Socks5),
        ("HTTPS", RouteKind::Http),
        ("HTTP", RouteKind::Http),
    ] {
        if get(&format!("{prefix}Enable")).as_deref() != Some("1") {
            continue;
        }
        let host = get(&format!("{prefix}Proxy")).unwrap_or_default();
        let port = get(&format!("{prefix}Port"))
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        out.extend(system_route(kind, &host, port));
    }
    out
}

#[cfg(target_os = "windows")]
fn system_proxies() -> Vec<Detected> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    let query = |name: &str| -> Option<String> {
        let out = std::process::Command::new("reg")
            .args(["query", key, "/v", name])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        text.lines().find_map(|l| {
            let l = l.trim();
            l.starts_with(name)
                .then(|| l.split_whitespace().skip(2).collect::<Vec<_>>().join(" "))
        })
    };
    let enabled = query("ProxyEnable")
        .map(|v| v.trim() == "0x1")
        .unwrap_or(false);
    if !enabled {
        return Vec::new();
    }
    parse_windows_proxy_server(&query("ProxyServer").unwrap_or_default())
}

/// `127.0.0.1:7890` or `http=127.0.0.1:7890;https=…;socks=127.0.0.1:7891`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_windows_proxy_server(value: &str) -> Vec<Detected> {
    let mut out = Vec::new();
    for part in value.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (scheme, addr) = part.split_once('=').unwrap_or(("http", part));
        let addr = addr.split("://").last().unwrap_or(addr);
        let Some((h, p)) = addr.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = p.parse() else { continue };
        let kind = if scheme.eq_ignore_ascii_case("socks") {
            RouteKind::Socks5
        } else {
            RouteKind::Http
        };
        out.extend(system_route(kind, h, port));
    }
    out
}

#[cfg(all(unix, not(target_os = "macos")))]
fn system_proxies() -> Vec<Detected> {
    let mut out = Vec::new();
    let gs = |schema: &str, key: &str| -> Option<String> {
        let o = std::process::Command::new("gsettings")
            .args(["get", schema, key])
            .output()
            .ok()?;
        o.status.success().then(|| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .trim_matches('\'')
                .to_string()
        })
    };
    if gs("org.gnome.system.proxy", "mode").as_deref() == Some("manual") {
        for (sub, kind) in [
            ("socks", RouteKind::Socks5),
            ("https", RouteKind::Http),
            ("http", RouteKind::Http),
        ] {
            let schema = format!("org.gnome.system.proxy.{sub}");
            let host = gs(&schema, "host").unwrap_or_default();
            let port = gs(&schema, "port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0);
            out.extend(system_route(kind, &host, port));
        }
    }
    // KDE: ~/.config/kioslaverc, ProxyType=1 means manual.
    if let Some(cfg) = dirs::config_dir() {
        if let Ok(text) = std::fs::read_to_string(cfg.join("kioslaverc")) {
            if text.lines().any(|l| l.trim() == "ProxyType=1") {
                for (key, kind) in [
                    ("socksProxy", RouteKind::Socks5),
                    ("httpsProxy", RouteKind::Http),
                    ("httpProxy", RouteKind::Http),
                ] {
                    let Some(v) = text
                        .lines()
                        .find_map(|l| l.trim().strip_prefix(&format!("{key}=")))
                    else {
                        continue;
                    };
                    let v = v.split("://").last().unwrap_or(v).replace(' ', ":");
                    if let Some((h, p)) = v.rsplit_once(':') {
                        if let Ok(port) = p.parse() {
                            out.extend(system_route(kind, h, port));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Folders where Clash-family clients keep their YAML. We walk them
/// shallowly and read the listening ports, so a person who changed the
/// default port is still found.
fn clash_dirs() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".config"));
        roots.push(home.join("Library/Application Support"));
    }
    roots.extend(dirs::config_dir());
    roots.extend(dirs::data_dir());
    roots.extend(dirs::data_local_dir());
    let needles = [
        "clash",
        "mihomo",
        "verge",
        "flclash",
        "com.follow.clash",
        "stash",
    ];
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if needles.iter().any(|n| name.contains(n)) && seen.insert(e.path()) {
                out.push(e.path());
            }
        }
    }
    out
}

fn clash_config_ports() -> Vec<(u16, String)> {
    let mut out = Vec::new();
    for dir in clash_dirs() {
        let app = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let app = pretty_app(&app);
        for file in yaml_files(&dir, 3) {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            for (port, what) in ports_in_yaml(&text) {
                out.push((port, format!("{app} ({what})")));
            }
        }
    }
    out
}

fn pretty_app(dir_name: &str) -> String {
    let l = dir_name.to_ascii_lowercase();
    if l.contains("clash-verge-rev") {
        "Clash Verge Rev".into()
    } else if l.contains("verge") {
        "Clash Verge".into()
    } else if l.contains("mihomo") {
        "mihomo".into()
    } else if l.contains("follow.clash") || l.contains("flclash") {
        "FlClash".into()
    } else if l.contains("stash") {
        "Stash".into()
    } else {
        "Clash".into()
    }
}

fn yaml_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() && depth > 0 {
            out.extend(yaml_files(&p, depth - 1));
        } else if meta.is_file()
            && meta.len() < 4 * 1024 * 1024
            && matches!(p.extension().and_then(|x| x.to_str()), Some("yaml" | "yml"))
        {
            out.push(p);
        }
    }
    out
}

/// Reads only the top-level listener keys; full YAML parsing of a 2 MB
/// subscription would be wasted work.
fn ports_in_yaml(text: &str) -> Vec<(u16, &'static str)> {
    let keys: [(&str, &str); 6] = [
        ("mixed-port", "mixed"),
        ("socks-port", "socks"),
        ("port", "http"),
        ("verge_mixed_port", "mixed"),
        ("verge_socks_port", "socks"),
        ("verge_port", "http"),
    ];
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let Some((_, what)) = keys.iter().find(|(key, _)| *key == k.trim()) else {
            continue;
        };
        if let Ok(p) = v.trim().trim_matches('"').parse::<u16>() {
            if p > 0 {
                out.push((p, *what));
            }
        }
    }
    out
}

/// Connects to 127.0.0.1:`port` and asks what it speaks. Returns SOCKS5
/// and/or HTTP when the listener answers like one.
pub async fn sniff_local(port: u16) -> Vec<RouteKind> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut kinds = Vec::new();
    if sniff_socks5(addr).await {
        kinds.push(RouteKind::Socks5);
    } else if sniff_http(addr).await {
        kinds.push(RouteKind::Http);
    }
    kinds
}

const SNIFF_TIMEOUT: Duration = Duration::from_millis(600);

async fn sniff_socks5(addr: SocketAddr) -> bool {
    let fut = async {
        let mut s = TcpStream::connect(addr).await.ok()?;
        s.write_all(&[5, 1, 0]).await.ok()?;
        let mut buf = [0u8; 2];
        s.read_exact(&mut buf).await.ok()?;
        Some(buf == [5, 0])
    };
    matches!(
        tokio::time::timeout(SNIFF_TIMEOUT, fut).await,
        Ok(Some(true))
    )
}

async fn sniff_http(addr: SocketAddr) -> bool {
    let fut = async {
        let mut s = TcpStream::connect(addr).await.ok()?;
        // A CONNECT to a Telegram DC: a proxy answers "HTTP/1.x 200" or at
        // worst an HTTP error; anything else is not an HTTP proxy.
        s.write_all(b"CONNECT 149.154.167.51:443 HTTP/1.1\r\nHost: 149.154.167.51:443\r\n\r\n")
            .await
            .ok()?;
        let mut buf = [0u8; 12];
        s.read_exact(&mut buf).await.ok()?;
        Some(buf.starts_with(b"HTTP/1."))
    };
    matches!(
        tokio::time::timeout(Duration::from_millis(1500), fut).await,
        Ok(Some(true))
    )
}
