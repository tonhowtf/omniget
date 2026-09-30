//! Optional `mihomo` sidecar for the protocols we do not speak natively
//! (VLESS/Reality, VMess, Hysteria/Hysteria2, TUIC, AnyTLS, SSR, Shadowsocks
//! with a plugin, Trojan over WebSocket/gRPC, and anything else a Clash
//! subscription lists).
//!
//! Nodes come from pasted links or subscriptions and are kept in
//! `<data>/telegram/mihomo/nodes.json` whether or not the sidecar is on, so
//! the screen can offer "N servers need the mihomo engine". When the person
//! turns it on, the official binary (GPL-3.0, MetaCubeX releases, pinned
//! version and sha256) is downloaded to the managed bin dir, a minimal
//! config is written with one loopback SOCKS5 listener per node (random
//! port, per-install password, `proxy:` pinned to that node), and each
//! listener becomes an ordinary SOCKS5 route the bridge and the probe
//! already know how to use. The process is a child of the app: killed on
//! `shutdown()`, by the Job Object on Windows, by `PDEATHSIG` on Linux, and
//! a leftover from a crashed session is stopped on the next start.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use base64::Engine;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::route::{Route, RouteKind, RouteSource};

/// Pinned release. Bump together with the hashes below (they are the
/// `digest` fields of the GitHub release assets).
pub const VERSION: &str = "v1.19.31";

const USER: &str = "omniget";

/// `(asset, sha256)` for this platform. amd64 uses the `compatible` build
/// (no AVX2 requirement): old office PCs are common among the people who
/// need this.
fn asset() -> Option<(&'static str, &'static str)> {
    let a = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        (
            "mihomo-darwin-arm64-v1.19.31.gz",
            "d131f44b3deb2a8356f7ac75048ad67a10d53243323951c4f3cda7b672922963",
        )
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        (
            "mihomo-darwin-amd64-compatible-v1.19.31.gz",
            "fb6fca0e105b4310a21eaacd3a8d3853d3d8b87fa4c69737bea52a30a435aac7",
        )
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (
            "mihomo-linux-amd64-compatible-v1.19.31.gz",
            "04cf9f09671704f839ddbee2e93069dc831a4123a75281e725d1d96ab9ac1afc",
        )
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        (
            "mihomo-linux-arm64-v1.19.31.gz",
            "9e0f11afbf38426b8bd88fdc594678f8161c57eccb4e1b77acb12b493904f1d4",
        )
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        (
            "mihomo-windows-amd64-compatible-v1.19.31.zip",
            "93d14e9a13b49b2f2d256202d02cc8d14a7c4695edf084cae0f941986bc9c218",
        )
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        (
            "mihomo-windows-arm64-v1.19.31.zip",
            "68659624a38ae1dbc4e1b92f45d4dc75da0eaedd4d732b0a6f01a308da945792",
        )
    } else {
        return None;
    };
    Some(a)
}

/// GitHub is poisoned or throttled in mainland China; these prefix the
/// release URL. They come and go, which is fine: the pinned hash makes any
/// of them as trustworthy as GitHub itself.
const MIRRORS: [&str; 3] = [
    "https://ghfast.top/",
    "https://gh-proxy.com/",
    "https://ghproxy.net/",
];

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

/// One server handed to mihomo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
    /// Stable id: hash of the proxy definition without its name.
    pub key: String,
    pub name: String,
    /// mihomo proxy `type` (`vless`, `hysteria2`, …).
    pub kind: String,
    /// The proxy map as mihomo reads it, without `name`.
    pub proxy: Value,
    pub source: RouteSource,
    /// Loopback listener port; kept across restarts so route ids hold.
    #[serde(default)]
    pub port: u16,
}

impl Node {
    fn new(name: String, proxy: Map<String, Value>, source: RouteSource) -> Option<Self> {
        let mut proxy = proxy;
        proxy.remove("name");
        // Chains to other proxies of the subscription cannot resolve here.
        proxy.remove("dialer-proxy");
        let kind = proxy.get("type")?.as_str()?.to_ascii_lowercase();
        proxy.get("server")?.as_str().filter(|s| !s.is_empty())?;
        let proxy = Value::Object(proxy);
        let key = node_key(&proxy);
        let name = if name.trim().is_empty() {
            format!(
                "{} {}",
                kind,
                proxy.get("server").and_then(|v| v.as_str()).unwrap_or("")
            )
        } else {
            name.trim().to_string()
        };
        Some(Node {
            key,
            name,
            kind,
            proxy,
            source,
            port: 0,
        })
    }

    /// Name used inside the generated config: ASCII, unique, never clashes
    /// with the built-ins (`DIRECT`, `REJECT`, …).
    fn internal_name(&self) -> String {
        format!("n-{}", self.key)
    }
}

fn node_key(proxy: &Value) -> String {
    use sha2::{Digest, Sha256};
    // serde_json maps are ordered (BTreeMap without preserve_order), so the
    // text is canonical.
    let text = serde_json::to_string(proxy).unwrap_or_default();
    let digest = Sha256::digest(text.as_bytes());
    hex::encode(&digest[..8])
}

/// What we persist.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Store {
    #[serde(default)]
    nodes: Vec<Node>,
    /// Password of every listener. Stable so route ids survive restarts.
    #[serde(default)]
    listener_pass: String,
    /// Recorded while running, to stop a leftover after a crash.
    #[serde(default)]
    pid: Option<(u32, u64)>,
}

fn work_dir() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("telegram")
        .join("mihomo")
}

fn store_path() -> PathBuf {
    work_dir().join("nodes.json")
}

fn load_store() -> Store {
    let mut s: Store = std::fs::read_to_string(store_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if s.listener_pass.is_empty() {
        s.listener_pass = token(12);
    }
    s
}

fn save_store(s: &Store) {
    let dir = work_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let path = store_path();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

fn token(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::rng().fill(&mut b[..]);
    hex::encode(b)
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(load_store()))
}

/// Every node we know, running or not.
pub fn nodes() -> Vec<Node> {
    store().lock().unwrap().nodes.clone()
}

/// Adds nodes (deduplicated by key). Returns how many were new.
pub fn add_nodes(new: Vec<Node>) -> usize {
    let mut s = store().lock().unwrap();
    let mut added = 0;
    for n in new {
        if !s.nodes.iter().any(|x| x.key == n.key) {
            s.nodes.push(n);
            added += 1;
        }
    }
    save_store(&s);
    added
}

/// Replaces every node from `source` (a subscription refresh).
pub fn replace_source(source: &RouteSource, new: Vec<Node>) {
    let mut s = store().lock().unwrap();
    let ports: HashMap<String, u16> = s.nodes.iter().map(|n| (n.key.clone(), n.port)).collect();
    s.nodes.retain(|n| &n.source != source);
    for mut n in new {
        if s.nodes.iter().any(|x| x.key == n.key) {
            continue;
        }
        n.port = ports.get(&n.key).copied().unwrap_or(0);
        s.nodes.push(n);
    }
    save_store(&s);
}

pub fn remove_source(source: &RouteSource) {
    let mut s = store().lock().unwrap();
    s.nodes.retain(|n| &n.source != source);
    save_store(&s);
}

/// Removes the node behind a route id. `true` when it was one of ours.
pub fn remove_by_route_id(route_id: &str) -> bool {
    let Some(key) = route_key(route_id) else {
        return false;
    };
    let mut s = store().lock().unwrap();
    s.nodes.retain(|n| n.key != key);
    save_store(&s);
    true
}

/// Removes one node by key.
pub fn remove_node(key: &str) {
    let mut s = store().lock().unwrap();
    s.nodes.retain(|n| n.key != key);
    save_store(&s);
}

/// Node key for a route id handed out by `routes()`.
pub fn route_key(route_id: &str) -> Option<String> {
    let s = store().lock().unwrap();
    s.nodes
        .iter()
        .find(|n| node_route(n, &s.listener_pass).id == route_id)
        .map(|n| n.key.clone())
}

pub fn is_mihomo_route(route_id: &str) -> bool {
    route_key(route_id).is_some()
}

fn node_route(n: &Node, pass: &str) -> Route {
    let mut r = Route::new(RouteKind::Socks5, "127.0.0.1", n.port, n.source.clone());
    r.username = Some(USER.into());
    r.password = Some(pass.to_string());
    r.name = format!("{} ({})", n.name, n.kind);
    r.finish();
    // The local listener's password is new on every run; the id must not
    // be, or a route pinned in fixed mode is lost at the next start.
    r.id = format!("mh{}", &n.key[..n.key.len().min(14)]);
    r
}

/// Routes for the nodes mihomo is serving right now. Empty when it is not
/// running, so the bridge never dials a dead listener.
pub fn routes() -> Vec<Route> {
    if !is_running() {
        return Vec::new();
    }
    let s = store().lock().unwrap();
    s.nodes
        .iter()
        .filter(|n| n.port != 0)
        .map(|n| node_route(n, &s.listener_pass))
        .collect()
}

// ---------------------------------------------------------------------------
// Parsing: pasted links and subscriptions
// ---------------------------------------------------------------------------

/// Nodes in pasted text that only mihomo can dial. Links the native layer
/// handles (`ss://` without plugin, `trojan://` over TCP, MTProto, SOCKS,
/// HTTP) are left to `links.rs`.
pub fn extract_nodes(text: &str, source: RouteSource) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    for tok in tokens(text) {
        if let Some(n) = parse_link(&tok, source.clone()) {
            if !out.iter().any(|x| x.key == n.key) {
                out.push(n);
            }
        }
    }
    out
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '<'
                    | '>'
                    | '，'
                    | '。'
                    | '；'
                    | '、'
                    | '「'
                    | '」'
                    | '（'
                    | '）'
                    | '【'
                    | '】'
            )
    })
    .map(|t| t.trim_matches(|c: char| matches!(c, ',' | ';' | ')' | '(' | '[' | ']')))
    .filter(|t| t.contains("://"))
    .map(str::to_string)
    .collect()
}

/// Scheme-generic pieces of `scheme://userinfo@host:port/?query#name`.
struct Parts {
    user: String,
    host: String,
    port: u16,
    /// `a-b` when the port field is a hop range (Hysteria2).
    port_range: Option<String>,
    q: HashMap<String, String>,
    name: String,
}

fn split_link(s: &str) -> Option<Parts> {
    let (_, rest) = s.split_once("://")?;
    let (rest, name) = match rest.split_once('#') {
        Some((r, n)) => (r, pct(n)),
        None => (rest, String::new()),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, q),
        None => (rest, ""),
    };
    let rest = rest.trim_end_matches('/');
    let (user, hostport) = match rest.rsplit_once('@') {
        Some((u, h)) => (pct(u), h),
        None => (String::new(), rest),
    };
    let (host, port_s) = if let Some(r) = hostport.strip_prefix('[') {
        let (h, p) = r.split_once("]:")?;
        (h.to_string(), p)
    } else {
        let (h, p) = hostport.rsplit_once(':')?;
        (h.to_string(), p)
    };
    let (port, port_range) = match port_s.split_once(['-', ',']) {
        Some((a, _)) => (a.parse().ok()?, Some(port_s.to_string())),
        None => (port_s.parse().ok()?, None),
    };
    if host.is_empty() {
        return None;
    }
    let q = url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.to_string(), v.into_owned()))
        .collect();
    Some(Parts {
        user,
        host,
        port,
        port_range,
        q,
        name,
    })
}

fn pct(s: &str) -> String {
    urlencoding::decode(s)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string())
}

fn b64_any(s: &str) -> Option<Vec<u8>> {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let t = t.trim_end_matches('=');
    [
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ]
    .iter()
    .find_map(|e| e.decode(t).ok())
}

fn truthy(v: Option<&String>) -> bool {
    matches!(
        v.map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

fn nonempty<'a>(q: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a String> {
    keys.iter()
        .find_map(|k| q.get(*k))
        .filter(|v| !v.is_empty())
}

fn csv(v: &str) -> Value {
    Value::Array(
        v.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.into()))
            .collect(),
    )
}

fn base(kind: &str, host: &str, port: u16) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("type".into(), kind.into());
    m.insert("server".into(), host.into());
    m.insert("port".into(), port.into());
    m
}

pub fn parse_link(raw: &str, source: RouteSource) -> Option<Node> {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    let scheme = lower.split_once("://")?.0.to_string();
    let (name, map) = match scheme.as_str() {
        "vless" => vless(s)?,
        "vmess" => vmess(s)?,
        "hysteria2" | "hy2" => hysteria2(s)?,
        "hysteria" => hysteria(s)?,
        "tuic" => tuic(s)?,
        "anytls" => anytls(s)?,
        "ssr" => ssr(s)?,
        "ss" => ss_plugin(s)?,
        "trojan" => trojan_transport(s)?,
        _ => return None,
    };
    Node::new(name, map, source)
}

/// `network` plus its `*-opts`, shared by VLESS, VMess and Trojan links.
fn transport(
    m: &mut Map<String, Value>,
    net: &str,
    host: Option<&String>,
    path: Option<&String>,
    service: Option<&String>,
    mode: Option<&String>,
) {
    let net = match net {
        "" | "tcp" | "raw" => return,
        "h2" => "h2",
        other => other,
    };
    m.insert("network".into(), net.into());
    match net {
        "ws" | "httpupgrade" => {
            let mut o = Map::new();
            if let Some(p) = path {
                o.insert("path".into(), p.as_str().into());
            }
            if let Some(h) = host {
                o.insert("headers".into(), json!({ "Host": h }));
            }
            if net == "httpupgrade" {
                m.insert("network".into(), "ws".into());
                o.insert("v2ray-http-upgrade".into(), true.into());
            }
            m.insert("ws-opts".into(), Value::Object(o));
        }
        "grpc" => {
            let name = service.or(path).cloned().unwrap_or_default();
            m.insert("grpc-opts".into(), json!({ "grpc-service-name": name }));
        }
        "h2" | "http" => {
            let mut o = Map::new();
            if let Some(p) = path {
                o.insert("path".into(), p.as_str().into());
            }
            if let Some(h) = host {
                o.insert("host".into(), csv(h));
            }
            m.insert("h2-opts".into(), Value::Object(o));
            m.insert("network".into(), "h2".into());
        }
        "xhttp" | "splithttp" => {
            let mut o = Map::new();
            if let Some(p) = path {
                o.insert("path".into(), p.as_str().into());
            }
            if let Some(h) = host {
                o.insert("host".into(), h.as_str().into());
            }
            if let Some(md) = mode {
                o.insert("mode".into(), md.as_str().into());
            }
            m.insert("network".into(), "xhttp".into());
            m.insert("xhttp-opts".into(), Value::Object(o));
        }
        _ => {}
    }
}

/// `vless://uuid@host:port?security=reality&pbk=..&sid=..&sni=..&fp=..&flow=..&type=ws&path=..&host=..#name`
fn vless(s: &str) -> Option<(String, Map<String, Value>)> {
    let p = split_link(s)?;
    if p.user.is_empty() {
        return None;
    }
    let mut m = base("vless", &p.host, p.port);
    m.insert("uuid".into(), p.user.as_str().into());
    m.insert("udp".into(), true.into());
    if let Some(f) = nonempty(&p.q, &["flow"]) {
        m.insert("flow".into(), f.as_str().into());
    }
    if let Some(e) = nonempty(&p.q, &["encryption"]).filter(|e| e.as_str() != "none") {
        m.insert("encryption".into(), e.as_str().into());
    }
    tls_opts(&mut m, &p.q);
    transport(
        &mut m,
        p.q.get("type").map(String::as_str).unwrap_or("tcp"),
        nonempty(&p.q, &["host"]),
        nonempty(&p.q, &["path"]),
        nonempty(&p.q, &["serviceName", "servicename"]),
        nonempty(&p.q, &["mode"]),
    );
    Some((p.name, m))
}

/// `security`/`sni`/`fp`/`pbk`/`sid`/`alpn`/`allowInsecure` for VLESS-shaped links.
fn tls_opts(m: &mut Map<String, Value>, q: &HashMap<String, String>) {
    let security = q
        .get("security")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if security == "tls" || security == "reality" {
        m.insert("tls".into(), true.into());
    }
    if let Some(sni) = nonempty(q, &["sni", "peer"]) {
        m.insert("servername".into(), sni.as_str().into());
    }
    if let Some(fp) = nonempty(q, &["fp"]) {
        m.insert("client-fingerprint".into(), fp.as_str().into());
    }
    if let Some(alpn) = nonempty(q, &["alpn"]) {
        m.insert("alpn".into(), csv(alpn));
    }
    if truthy(q.get("allowInsecure")) || truthy(q.get("insecure")) {
        m.insert("skip-cert-verify".into(), true.into());
    }
    if security == "reality" {
        let mut r = Map::new();
        if let Some(pbk) = nonempty(q, &["pbk"]) {
            r.insert("public-key".into(), pbk.as_str().into());
        }
        if let Some(sid) = q.get("sid") {
            r.insert("short-id".into(), sid.as_str().into());
        }
        m.insert("reality-opts".into(), Value::Object(r));
        if !m.contains_key("client-fingerprint") {
            m.insert("client-fingerprint".into(), "chrome".into());
        }
    }
}

/// `vmess://BASE64(JSON)` (v2rayN) or the URL form `vmess://uuid@host:port?...`.
fn vmess(s: &str) -> Option<(String, Map<String, Value>)> {
    let body = &s[s.find("://")? + 3..];
    let body = body.split('#').next().unwrap_or(body);
    if let Some(v) = b64_any(body)
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        let get = |k: &str| -> Option<String> {
            match v.get(k)? {
                Value::String(s) if !s.is_empty() => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }
        };
        let host = get("add")?;
        let port: u16 = get("port")?.parse().ok()?;
        let mut m = base("vmess", &host, port);
        m.insert("uuid".into(), get("id")?.into());
        m.insert(
            "alterId".into(),
            get("aid")
                .and_then(|a| a.parse::<u64>().ok())
                .unwrap_or(0)
                .into(),
        );
        m.insert(
            "cipher".into(),
            get("scy").unwrap_or_else(|| "auto".into()).into(),
        );
        m.insert("udp".into(), true.into());
        if get("tls").as_deref() == Some("tls") {
            m.insert("tls".into(), true.into());
        }
        if let Some(sni) = get("sni") {
            m.insert("servername".into(), sni.into());
        }
        if let Some(fp) = get("fp") {
            m.insert("client-fingerprint".into(), fp.into());
        }
        if let Some(alpn) = get("alpn") {
            m.insert("alpn".into(), csv(&alpn));
        }
        let net = get("net").unwrap_or_else(|| "tcp".into());
        let host_h = get("host");
        let path = get("path");
        if net == "tcp" && get("type").as_deref() == Some("http") {
            m.insert("network".into(), "http".into());
            let mut o = Map::new();
            o.insert(
                "path".into(),
                json!([path.clone().unwrap_or_else(|| "/".into())]),
            );
            if let Some(h) = &host_h {
                o.insert("headers".into(), json!({ "Host": csv(h) }));
            }
            m.insert("http-opts".into(), Value::Object(o));
        } else {
            transport(
                &mut m,
                &net,
                host_h.as_ref(),
                path.as_ref(),
                path.as_ref(),
                None,
            );
        }
        return Some((get("ps").unwrap_or_default(), m));
    }
    // URL form, same query keys as VLESS.
    let p = split_link(s)?;
    if p.user.is_empty() {
        return None;
    }
    let mut m = base("vmess", &p.host, p.port);
    m.insert("uuid".into(), p.user.as_str().into());
    m.insert("alterId".into(), 0.into());
    m.insert(
        "cipher".into(),
        nonempty(&p.q, &["encryption"])
            .cloned()
            .unwrap_or_else(|| "auto".into())
            .into(),
    );
    m.insert("udp".into(), true.into());
    tls_opts(&mut m, &p.q);
    transport(
        &mut m,
        p.q.get("type").map(String::as_str).unwrap_or("tcp"),
        nonempty(&p.q, &["host"]),
        nonempty(&p.q, &["path"]),
        nonempty(&p.q, &["serviceName", "servicename"]),
        None,
    );
    Some((p.name, m))
}

/// `hysteria2://auth@host:port/?sni=..&insecure=1&obfs=salamander&obfs-password=..&mport=..#name`
fn hysteria2(s: &str) -> Option<(String, Map<String, Value>)> {
    let p = split_link(s)?;
    let mut m = base("hysteria2", &p.host, p.port);
    m.insert("password".into(), p.user.as_str().into());
    if let Some(sni) = nonempty(&p.q, &["sni", "peer"]) {
        m.insert("sni".into(), sni.as_str().into());
    }
    if truthy(p.q.get("insecure")) || truthy(p.q.get("allowInsecure")) {
        m.insert("skip-cert-verify".into(), true.into());
    }
    if let Some(o) = nonempty(&p.q, &["obfs"]).filter(|o| o.as_str() != "none") {
        m.insert("obfs".into(), o.as_str().into());
        if let Some(pw) = nonempty(&p.q, &["obfs-password"]) {
            m.insert("obfs-password".into(), pw.as_str().into());
        }
    }
    if let Some(ports) = nonempty(&p.q, &["mport"]).cloned().or(p.port_range.clone()) {
        m.insert("ports".into(), ports.into());
    }
    if let Some(pin) = nonempty(&p.q, &["pinSHA256"]) {
        m.insert("fingerprint".into(), pin.as_str().into());
    }
    if let Some(alpn) = nonempty(&p.q, &["alpn"]) {
        m.insert("alpn".into(), csv(alpn));
    }
    Some((p.name, m))
}

/// Hysteria v1: `hysteria://host:port?protocol=udp&auth=..&peer=..&insecure=1&upmbps=..&downmbps=..&alpn=..&obfs=..#name`
fn hysteria(s: &str) -> Option<(String, Map<String, Value>)> {
    let p = split_link(s)?;
    let mut m = base("hysteria", &p.host, p.port);
    if let Some(a) = nonempty(&p.q, &["auth"]) {
        m.insert("auth-str".into(), a.as_str().into());
    }
    if let Some(sni) = nonempty(&p.q, &["peer", "sni"]) {
        m.insert("sni".into(), sni.as_str().into());
    }
    if truthy(p.q.get("insecure")) {
        m.insert("skip-cert-verify".into(), true.into());
    }
    m.insert(
        "up".into(),
        nonempty(&p.q, &["upmbps", "up"])
            .cloned()
            .unwrap_or_else(|| "20".into())
            .into(),
    );
    m.insert(
        "down".into(),
        nonempty(&p.q, &["downmbps", "down"])
            .cloned()
            .unwrap_or_else(|| "100".into())
            .into(),
    );
    if let Some(alpn) = nonempty(&p.q, &["alpn"]) {
        m.insert("alpn".into(), csv(alpn));
    }
    if let Some(proto) = nonempty(&p.q, &["protocol"]) {
        m.insert("protocol".into(), proto.as_str().into());
    }
    if let Some(o) = nonempty(&p.q, &["obfsParam", "obfs"]) {
        m.insert("obfs".into(), o.as_str().into());
    }
    if let Some(ports) = nonempty(&p.q, &["mport"]).cloned().or(p.port_range.clone()) {
        m.insert("ports".into(), ports.into());
    }
    Some((p.name, m))
}

/// `tuic://uuid:password@host:port?congestion_control=bbr&alpn=h3&sni=..#name` (v5),
/// or `tuic://token@host:port` (v4).
fn tuic(s: &str) -> Option<(String, Map<String, Value>)> {
    let p = split_link(s)?;
    let mut m = base("tuic", &p.host, p.port);
    match p.user.split_once(':') {
        Some((uuid, pw)) => {
            m.insert("uuid".into(), uuid.into());
            m.insert("password".into(), pw.into());
        }
        None if !p.user.is_empty() => {
            m.insert("token".into(), p.user.as_str().into());
        }
        None => return None,
    }
    if let Some(c) = nonempty(&p.q, &["congestion_control", "congestion-control"]) {
        m.insert("congestion-controller".into(), c.as_str().into());
    }
    m.insert(
        "alpn".into(),
        csv(nonempty(&p.q, &["alpn"])
            .map(String::as_str)
            .unwrap_or("h3")),
    );
    if let Some(sni) = nonempty(&p.q, &["sni"]) {
        m.insert("sni".into(), sni.as_str().into());
    }
    if truthy(p.q.get("allow_insecure"))
        || truthy(p.q.get("insecure"))
        || truthy(p.q.get("allowInsecure"))
    {
        m.insert("skip-cert-verify".into(), true.into());
    }
    if let Some(r) = nonempty(&p.q, &["udp_relay_mode", "udp-relay-mode"]) {
        m.insert("udp-relay-mode".into(), r.as_str().into());
    }
    if truthy(p.q.get("disable_sni")) {
        m.insert("disable-sni".into(), true.into());
    }
    m.insert("udp".into(), true.into());
    Some((p.name, m))
}

/// `anytls://password@host:port?sni=..&insecure=1&fp=..#name`
fn anytls(s: &str) -> Option<(String, Map<String, Value>)> {
    let p = split_link(s)?;
    let mut m = base("anytls", &p.host, p.port);
    m.insert("password".into(), p.user.as_str().into());
    if let Some(sni) = nonempty(&p.q, &["sni", "peer"]) {
        m.insert("sni".into(), sni.as_str().into());
    }
    if truthy(p.q.get("insecure")) || truthy(p.q.get("allowInsecure")) {
        m.insert("skip-cert-verify".into(), true.into());
    }
    if let Some(fp) = nonempty(&p.q, &["fp"]) {
        m.insert("client-fingerprint".into(), fp.as_str().into());
    }
    if let Some(alpn) = nonempty(&p.q, &["alpn"]) {
        m.insert("alpn".into(), csv(alpn));
    }
    m.insert("udp".into(), true.into());
    Some((p.name, m))
}

/// `ssr://BASE64(host:port:protocol:method:obfs:BASE64(pass)/?obfsparam=..&protoparam=..&remarks=..)`
fn ssr(s: &str) -> Option<(String, Map<String, Value>)> {
    let text = String::from_utf8(b64_any(&s[6..])?).ok()?;
    let (main, query) = match text.split_once("/?") {
        Some((a, b)) => (a, b),
        None => (text.trim_end_matches('/'), ""),
    };
    let parts: Vec<&str> = main.rsplitn(6, ':').collect();
    let [pass, obfs, method, protocol, port, host] = parts.as_slice() else {
        return None;
    };
    let dec = |v: &str| {
        b64_any(v)
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default()
    };
    let q: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.to_string(), v.into_owned()))
        .collect();
    let mut m = base("ssr", host, port.parse().ok()?);
    m.insert("cipher".into(), (*method).into());
    m.insert("password".into(), dec(pass).into());
    m.insert("obfs".into(), (*obfs).into());
    m.insert("protocol".into(), (*protocol).into());
    if let Some(v) = q.get("obfsparam") {
        m.insert("obfs-param".into(), dec(v).into());
    }
    if let Some(v) = q.get("protoparam") {
        m.insert("protocol-param".into(), dec(v).into());
    }
    m.insert("udp".into(), true.into());
    Some((q.get("remarks").map(|r| dec(r)).unwrap_or_default(), m))
}

/// `ss://` that the native client cannot take: a SIP003 plugin, or a
/// cipher outside the AEAD/2022 set.
fn ss_plugin(s: &str) -> Option<(String, Map<String, Value>)> {
    let body = &s[5..];
    let (body, name) = match body.split_once('#') {
        Some((b, n)) => (b, pct(n)),
        None => (body, String::new()),
    };
    let (body, query) = match body.split_once('?') {
        Some((b, q)) => (b.trim_end_matches('/'), q),
        None => (body.trim_end_matches('/'), ""),
    };
    let (userinfo, hostport) = match body.rsplit_once('@') {
        Some((u, h)) => {
            let u = pct(u);
            let u = if u.contains(':') {
                u
            } else {
                String::from_utf8(b64_any(&u)?).ok()?
            };
            (u, h.to_string())
        }
        None => {
            let decoded = String::from_utf8(b64_any(body)?).ok()?;
            let (u, h) = decoded.rsplit_once('@')?;
            (u.to_string(), h.to_string())
        }
    };
    let (method, password) = userinfo.split_once(':')?;
    let (host, port) = hostport.rsplit_once(':')?;
    let host = host.trim_matches(['[', ']']);
    let q: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.to_string(), v.into_owned()))
        .collect();
    let plugin = q.get("plugin").filter(|p| !p.is_empty());
    let method = method.to_ascii_lowercase();
    if plugin.is_none() && super::shadowsocks::supported(&method) {
        return None;
    }
    let mut m = base("ss", host, port.parse().ok()?);
    m.insert("cipher".into(), method.into());
    m.insert("password".into(), password.into());
    m.insert("udp".into(), true.into());
    if let Some(plugin) = plugin {
        let (pname, opts) = sip003_plugin(plugin)?;
        m.insert("plugin".into(), pname.into());
        m.insert("plugin-opts".into(), opts);
    }
    Some((name, m))
}

/// `obfs-local;obfs=http;obfs-host=x` / `v2ray-plugin;tls;mode=websocket;host=..;path=..`
/// / `shadow-tls;host=..;password=..;version=3` → mihomo `plugin` + `plugin-opts`.
fn sip003_plugin(spec: &str) -> Option<(&'static str, Value)> {
    let mut it = spec.split(';');
    let name = it.next()?.trim();
    let mut kv: HashMap<String, String> = HashMap::new();
    for part in it {
        let (k, v) = part.split_once('=').unwrap_or((part, "true"));
        kv.insert(k.trim().to_string(), v.trim().to_string());
    }
    match name {
        "obfs-local" | "simple-obfs" | "obfs" => Some((
            "obfs",
            json!({
                "mode": kv.get("obfs").cloned().unwrap_or_else(|| "http".into()),
                "host": kv.get("obfs-host").cloned().unwrap_or_else(|| "bing.com".into()),
            }),
        )),
        "v2ray-plugin" => {
            let mut o = Map::new();
            o.insert(
                "mode".into(),
                kv.get("mode")
                    .cloned()
                    .unwrap_or_else(|| "websocket".into())
                    .into(),
            );
            if let Some(h) = kv.get("host") {
                o.insert("host".into(), h.as_str().into());
            }
            if let Some(p) = kv.get("path") {
                o.insert("path".into(), p.as_str().into());
            }
            o.insert("tls".into(), kv.contains_key("tls").into());
            o.insert(
                "mux".into(),
                (kv.get("mux").map(String::as_str) == Some("true")).into(),
            );
            Some(("v2ray-plugin", Value::Object(o)))
        }
        "shadow-tls" => Some((
            "shadow-tls",
            json!({
                "host": kv.get("host").cloned().unwrap_or_default(),
                "password": kv.get("password").cloned().unwrap_or_default(),
                "version": kv.get("version").and_then(|v| v.parse::<u64>().ok()).unwrap_or(3),
            }),
        )),
        _ => None,
    }
}

/// `trojan://` over WebSocket or gRPC (plain TCP is native).
fn trojan_transport(s: &str) -> Option<(String, Map<String, Value>)> {
    let p = split_link(s)?;
    let net =
        p.q.get("type")
            .map(|t| t.to_ascii_lowercase())
            .unwrap_or_default();
    if net.is_empty() || net == "tcp" {
        return None;
    }
    let mut m = base("trojan", &p.host, p.port);
    m.insert("password".into(), p.user.as_str().into());
    m.insert("udp".into(), true.into());
    if let Some(sni) = nonempty(&p.q, &["sni", "peer"]) {
        m.insert("sni".into(), sni.as_str().into());
    }
    if truthy(p.q.get("allowInsecure")) || truthy(p.q.get("insecure")) {
        m.insert("skip-cert-verify".into(), true.into());
    }
    if let Some(fp) = nonempty(&p.q, &["fp"]) {
        m.insert("client-fingerprint".into(), fp.as_str().into());
    }
    if let Some(alpn) = nonempty(&p.q, &["alpn"]) {
        m.insert("alpn".into(), csv(alpn));
    }
    transport(
        &mut m,
        &net,
        nonempty(&p.q, &["host"]),
        nonempty(&p.q, &["path"]),
        nonempty(&p.q, &["serviceName", "servicename"]),
        None,
    );
    Some((p.name, m))
}

/// Nodes of a subscription body that only mihomo can dial: Clash YAML
/// entries passed through as they are, or links from a base64/plain list.
pub fn nodes_from_subscription(body: &str, source: RouteSource) -> Vec<Node> {
    let text = body.trim();
    if text.starts_with('{') {
        return from_sip008(text, source);
    }
    if text.contains("proxies:") {
        return from_clash_yaml(text, source);
    }
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let decoded = if compact.contains("://") {
        text.to_string()
    } else {
        b64_any(&compact)
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| text.to_string())
    };
    extract_nodes(&decoded, source)
}

/// Clash YAML entries the native layer does not take (mirror of the rules
/// in `subscription::from_clash_yaml`).
fn from_clash_yaml(text: &str, source: RouteSource) -> Vec<Node> {
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return Vec::new();
    };
    let proxies = doc
        .get("proxies")
        .and_then(|p| p.as_sequence())
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<Node> = Vec::new();
    for p in proxies {
        let Ok(Value::Object(map)) = serde_json::to_value(&p) else {
            continue;
        };
        let get = |k: &str| {
            map.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let kind = get("type").to_ascii_lowercase();
        let native = match kind.as_str() {
            "ss" => !map.contains_key("plugin") && super::shadowsocks::supported(&get("cipher")),
            "trojan" => {
                let net = get("network");
                net.is_empty() || net == "tcp"
            }
            "socks5" | "http" => true,
            _ => false,
        };
        if native {
            continue;
        }
        let name = get("name");
        if let Some(n) = Node::new(name, map, source.clone()) {
            if !out.iter().any(|x| x.key == n.key) {
                out.push(n);
            }
        }
    }
    out
}

fn from_sip008(text: &str, source: RouteSource) -> Vec<Node> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let servers = v
        .get("servers")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    servers
        .iter()
        .filter_map(|s| {
            let plugin = s
                .get("plugin")
                .and_then(|p| p.as_str())
                .filter(|p| !p.is_empty())?;
            let opts = s.get("plugin_opts").and_then(|p| p.as_str()).unwrap_or("");
            let (pname, popts) = sip003_plugin(&format!("{plugin};{opts}"))?;
            let host = s.get("server")?.as_str()?;
            let port = s.get("server_port")?.as_u64()? as u16;
            let mut m = base("ss", host, port);
            m.insert("cipher".into(), s.get("method")?.clone());
            m.insert("password".into(), s.get("password")?.clone());
            m.insert("plugin".into(), pname.into());
            m.insert("plugin-opts".into(), popts);
            let name = s
                .get("remarks")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            Node::new(name, m, source.clone())
        })
        .collect()
}

/// Downloads a subscription again, this time for the nodes the native
/// importer skipped. The `clash.meta` UA makes panels include VLESS, TUIC
/// and Hysteria2.
pub async fn fetch_subscription_nodes(url: &str, via: Option<&Route>) -> anyhow::Result<Vec<Node>> {
    let body = match http_text(url, None).await {
        Ok(b) => b,
        Err(first) => match via {
            Some(r) => http_text(url, Some(proxy_url(r)))
                .await
                .map_err(|_| first)?,
            None => return Err(first),
        },
    };
    Ok(nodes_from_subscription(
        &body,
        RouteSource::Subscription {
            url: url.to_string(),
        },
    ))
}

async fn http_text(url: &str, proxy: Option<String>) -> anyhow::Result<String> {
    let mut b = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(20))
        .user_agent("clash.meta");
    b = match proxy {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    let resp = b.build()?.get(url).send().await?.error_for_status()?;
    Ok(resp.text().await?)
}

/// `socks5h://`/`http://` URL for reqwest from a SOCKS5/HTTP route.
pub fn proxy_url(r: &Route) -> String {
    let scheme = if r.kind == RouteKind::Socks5 {
        "socks5h"
    } else {
        "http"
    };
    match (&r.username, &r.password) {
        (Some(u), Some(p)) => format!(
            "{scheme}://{}:{}@{}:{}",
            urlencoding::encode(u),
            urlencoding::encode(p),
            r.host,
            r.port
        ),
        _ => format!("{scheme}://{}:{}", r.host, r.port),
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// The whole mihomo config: no inbound except one authenticated loopback
/// SOCKS5 listener per node, no GeoIP (so nothing is downloaded at start),
/// controller on loopback with a random secret.
pub fn build_config(
    nodes: &[Node],
    listener_pass: &str,
    controller_port: u16,
    secret: &str,
) -> anyhow::Result<String> {
    let mut proxies = Vec::new();
    let mut listeners = Vec::new();
    for n in nodes.iter().filter(|n| n.port != 0) {
        let mut p = match &n.proxy {
            Value::Object(m) => m.clone(),
            _ => continue,
        };
        p.insert("name".into(), n.internal_name().into());
        proxies.push(Value::Object(p));
        listeners.push(json!({
            "name": format!("in-{}", n.key),
            "type": "socks",
            "listen": "127.0.0.1",
            "port": n.port,
            "udp": false,
            "users": [{ "username": USER, "password": listener_pass }],
            "proxy": n.internal_name(),
        }));
    }
    let cfg = json!({
        "allow-lan": false,
        "bind-address": "127.0.0.1",
        "mode": "rule",
        "log-level": "warning",
        "ipv6": true,
        "unified-delay": true,
        "find-process-mode": "off",
        "geo-auto-update": false,
        "geodata-mode": false,
        "external-controller": format!("127.0.0.1:{controller_port}"),
        "secret": secret,
        "profile": { "store-selected": false, "store-fake-ip": false },
        "proxies": proxies,
        "listeners": listeners,
        "rules": ["MATCH,DIRECT"],
    });
    Ok(serde_yaml::to_string(&cfg)?)
}

/// A port nothing listens on right now. `keep` is reused when still free.
fn free_port(keep: u16, taken: &[u16]) -> u16 {
    if keep != 0
        && !taken.contains(&keep)
        && std::net::TcpListener::bind(("127.0.0.1", keep)).is_ok()
    {
        return keep;
    }
    for _ in 0..32 {
        if let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", 0)) {
            if let Ok(a) = l.local_addr() {
                if !taken.contains(&a.port()) {
                    return a.port();
                }
            }
        }
    }
    0
}

/// Gives every node a listener port, keeping the old one when possible.
fn assign_ports() {
    let mut s = store().lock().unwrap();
    let mut taken: Vec<u16> = Vec::new();
    for n in s.nodes.iter_mut() {
        n.port = free_port(n.port, &taken);
        taken.push(n.port);
    }
    save_store(&s);
}

// ---------------------------------------------------------------------------
// Binary
// ---------------------------------------------------------------------------

fn bin_path() -> Option<PathBuf> {
    Some(
        crate::core::dependencies::managed_bin_dir()?
            .join(crate::core::dependencies::bin_name("mihomo")),
    )
}

fn version_marker() -> Option<PathBuf> {
    Some(crate::core::dependencies::managed_bin_dir()?.join("mihomo.version"))
}

/// The pinned binary is on disk.
pub fn installed() -> bool {
    let (Some(bin), Some(marker)) = (bin_path(), version_marker()) else {
        return false;
    };
    bin.is_file()
        && std::fs::read_to_string(marker)
            .map(|v| v.trim() == VERSION)
            .unwrap_or(false)
}

/// Where the download may come from, in order: GitHub directly, GitHub
/// through each proxy in `via`, then the mirrors directly.
fn download_plan(asset: &str, via: &[String]) -> Vec<(String, Option<String>)> {
    let gh = format!("https://github.com/MetaCubeX/mihomo/releases/download/{VERSION}/{asset}");
    let mut plan = vec![(gh.clone(), None)];
    for p in via {
        plan.push((gh.clone(), Some(p.clone())));
    }
    for m in MIRRORS {
        plan.push((format!("{m}{gh}"), None));
    }
    plan
}

/// Downloads, verifies (pinned sha256, fail-closed) and installs the
/// binary unless the pinned version is already there. `via` are proxy
/// URLs (`socks5h://…`, `http://…`) to try GitHub through.
pub async fn install(via: &[String]) -> anyhow::Result<PathBuf> {
    let bin = bin_path().ok_or_else(|| anyhow!("no data directory"))?;
    if installed() {
        return Ok(bin);
    }
    let (asset, sha) = asset().ok_or_else(|| anyhow!("mihomo has no build for this platform"))?;
    set_state(|st| {
        st.installing = true;
        st.error = None;
    });
    let res = install_inner(&bin, asset, sha, via).await;
    set_state(|st| {
        st.installing = false;
        st.error = res.as_ref().err().map(|e| format!("{e:#}"));
    });
    res.map(|_| bin)
}

async fn install_inner(bin: &Path, asset: &str, sha: &str, via: &[String]) -> anyhow::Result<()> {
    use crate::core::dependencies::integrity;
    let mut errors = Vec::new();
    let mut bytes = None;
    for (url, proxy) in download_plan(asset, via) {
        match download(&url, proxy.as_deref()).await {
            Ok(b) => match integrity::verify_sha256(&b, sha, asset) {
                Ok(()) => {
                    bytes = Some(b);
                    break;
                }
                Err(e) => errors.push(format!("{url}: {e:#}")),
            },
            Err(e) => errors.push(format!(
                "{url}{}: {e:#}",
                proxy
                    .map(|p| format!(" via {}", redact(&p)))
                    .unwrap_or_default()
            )),
        }
    }
    let Some(bytes) = bytes else {
        bail!("could not download mihomo: {}", errors.join("; "));
    };
    let exe = if asset.ends_with(".zip") {
        unzip_exe(&bytes)?
    } else {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&bytes[..])
            .read_to_end(&mut out)
            .context("unpack mihomo")?;
        out
    };
    let dir = bin.parent().ok_or_else(|| anyhow!("bad bin path"))?;
    std::fs::create_dir_all(dir)?;
    let staged = dir.join(format!(
        "{}.new",
        crate::core::dependencies::bin_name("mihomo")
    ));
    std::fs::write(&staged, exe)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    crate::core::dependencies::replace_managed_binary(&staged, bin)?;
    if let Some(marker) = version_marker() {
        std::fs::write(marker, VERSION)?;
    }
    tracing::info!("[tg-net] mihomo {VERSION} installed at {}", bin.display());
    Ok(())
}

fn redact(proxy: &str) -> String {
    match proxy.rsplit_once('@') {
        Some((scheme_user, host)) => {
            let scheme = scheme_user.split("://").next().unwrap_or("");
            format!("{scheme}://{host}")
        }
        None => proxy.to_string(),
    }
}

async fn download(url: &str, proxy: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let mut b = reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .user_agent("omniget");
    b = match proxy {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    let resp = b.build()?.get(url).send().await?.error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

fn unzip_exe(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if f.name().to_ascii_lowercase().ends_with(".exe") {
            let mut out = Vec::new();
            f.read_to_end(&mut out)?;
            return Ok(out);
        }
    }
    bail!("the mihomo archive has no executable")
}

// ---------------------------------------------------------------------------
// Process
// ---------------------------------------------------------------------------

struct Running {
    child: tokio::process::Child,
    guard: Option<crate::core::process::ProcessTreeGuard>,
    controller: u16,
    secret: String,
    config: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct State {
    installing: bool,
    error: Option<String>,
    version: Option<String>,
}

fn running() -> &'static Mutex<Option<Running>> {
    static R: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
}

fn set_state(f: impl FnOnce(&mut State)) {
    f(&mut state().lock().unwrap());
}

/// Serializes start/stop/reload.
fn op_lock() -> &'static tokio::sync::Mutex<()> {
    static L: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    L.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub fn is_running() -> bool {
    let mut r = running().lock().unwrap();
    match r.as_mut() {
        Some(run) => match run.child.try_wait() {
            Ok(None) => true,
            _ => {
                *r = None;
                false
            }
        },
        None => false,
    }
}

/// Stops a mihomo left behind by a session that crashed.
fn stop_leftover() {
    let pid = store().lock().unwrap().pid.take();
    if let Some((pid, start)) = pid {
        let r = crate::core::process::stop_stale(pid, start);
        tracing::debug!("[tg-net] leftover mihomo {pid}: {r:?}");
        save_store(&store().lock().unwrap());
    }
}

/// Starts mihomo with every stored node, or reloads it when already
/// running. Requires `install()` first. Returns the routes it serves.
pub async fn start() -> anyhow::Result<Vec<Route>> {
    let _op = op_lock().lock().await;
    if nodes().is_empty() {
        stop_now();
        return Ok(Vec::new());
    }
    assign_ports();
    if is_running() {
        match reload().await {
            Ok(()) => return Ok(routes()),
            Err(e) => {
                tracing::warn!("[tg-net] mihomo reload failed, restarting: {e:#}");
                stop_now();
            }
        }
    }
    let bin = bin_path()
        .filter(|_| installed())
        .ok_or_else(|| anyhow!("mihomo is not installed"))?;
    stop_leftover();
    let dir = work_dir();
    std::fs::create_dir_all(&dir)?;
    let controller = free_port(0, &nodes().iter().map(|n| n.port).collect::<Vec<_>>());
    let secret = token(16);
    let config = dir.join("config.yaml");
    write_config(&config, controller, &secret)?;

    let log = std::fs::File::create(dir.join("mihomo.log"))?;
    let mut cmd = crate::core::process::command(&bin);
    cmd.arg("-d")
        .arg(&dir)
        .arg("-f")
        .arg(&config)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(target_os = "linux")]
    unsafe {
        // Dies with the app even when the app is killed.
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    crate::core::process::suspend_on_spawn(&mut cmd);
    let mut child = cmd.spawn().context("start mihomo")?;
    let guard = match crate::core::process::ProcessTreeGuard::new(&child) {
        Ok(g) => Some(g),
        Err(e) => {
            let _ = child.start_kill();
            bail!("could not supervise mihomo: {e}");
        }
    };
    if let Some(pid) = child.id() {
        if let Some(start) = crate::core::process::start_token(pid) {
            let mut s = store().lock().unwrap();
            s.pid = Some((pid, start));
            save_store(&s);
        }
    }
    *running().lock().unwrap() = Some(Running {
        child,
        guard,
        controller,
        secret: secret.clone(),
        config,
    });

    // Ready when the controller answers.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if !is_running() {
            let tail = log_tail();
            set_state(|st| st.error = Some(format!("mihomo exited: {tail}")));
            bail!("mihomo exited at start: {tail}");
        }
        if let Ok(v) = api(controller, &secret, reqwest::Method::GET, "/version", None).await {
            let version = v
                .get("version")
                .and_then(|x| x.as_str())
                .map(str::to_string);
            set_state(|st| {
                st.version = version;
                st.error = None;
            });
            break;
        }
        if std::time::Instant::now() > deadline {
            stop_now();
            bail!("mihomo did not come up: {}", log_tail());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tracing::info!("[tg-net] mihomo running, {} node(s)", nodes().len());
    Ok(routes())
}

fn write_config(path: &Path, controller: u16, secret: &str) -> anyhow::Result<()> {
    let (nodes, pass) = {
        let s = store().lock().unwrap();
        (s.nodes.clone(), s.listener_pass.clone())
    };
    let text = build_config(&nodes, &pass, controller, secret)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Rewrites the config and asks the running mihomo to load it.
async fn reload() -> anyhow::Result<()> {
    let (controller, secret, config) = {
        let r = running().lock().unwrap();
        let run = r.as_ref().ok_or_else(|| anyhow!("not running"))?;
        (run.controller, run.secret.clone(), run.config.clone())
    };
    write_config(&config, controller, &secret)?;
    api(
        controller,
        &secret,
        reqwest::Method::PUT,
        "/configs?force=true",
        Some(json!({ "path": config.to_string_lossy() })),
    )
    .await?;
    Ok(())
}

fn log_tail() -> String {
    let text = std::fs::read_to_string(work_dir().join("mihomo.log")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().rev().take(5).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

async fn api(
    port: u16,
    secret: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<Value> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(12))
        .build()?;
    let mut req = client
        .request(method, format!("http://127.0.0.1:{port}{path}"))
        .bearer_auth(secret);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await?.error_for_status()?;
    let text = resp.text().await?;
    Ok(if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    })
}

/// mihomo's own delay test for one node (HTTP 204 through the node), in
/// ms. A cheap second opinion next to the MTProto probe.
pub async fn node_delay(key: &str) -> anyhow::Result<u64> {
    let (controller, secret) = {
        let r = running().lock().unwrap();
        let run = r.as_ref().ok_or_else(|| anyhow!("mihomo is not running"))?;
        (run.controller, run.secret.clone())
    };
    let path = format!(
        "/proxies/n-{key}/delay?timeout=6000&url={}",
        urlencoding::encode("https://www.gstatic.com/generate_204")
    );
    let v = api(controller, &secret, reqwest::Method::GET, &path, None).await?;
    v.get("delay")
        .and_then(|d| d.as_u64())
        .ok_or_else(|| anyhow!("no answer through this node"))
}

fn stop_now() {
    if let Some(mut run) = running().lock().unwrap().take() {
        if let Some(g) = run.guard.as_mut() {
            g.terminate();
        }
        let _ = run.child.start_kill();
    }
    let mut s = store().lock().unwrap();
    if s.pid.take().is_some() {
        save_store(&s);
    }
}

pub async fn stop() {
    let _op = op_lock().lock().await;
    stop_now();
}

/// Kills the sidecar. Synchronous so the app can call it from its exit
/// handler.
pub fn shutdown() {
    stop_now();
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct NodeView {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub server: String,
    pub port: u16,
    pub source: RouteSource,
    /// Route id while running.
    pub route_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MihomoStatus {
    /// `None` = never asked (the screen should offer it when `nodes` is
    /// not empty).
    pub enabled: Option<bool>,
    pub supported_platform: bool,
    pub installed: bool,
    pub installing: bool,
    pub running: bool,
    pub pinned_version: &'static str,
    pub version: Option<String>,
    pub error: Option<String>,
    pub nodes: Vec<NodeView>,
    /// Node count per protocol.
    pub kinds: BTreeMap<String, usize>,
}

pub fn status(enabled: Option<bool>) -> MihomoStatus {
    let running = is_running();
    let st = state().lock().unwrap().clone();
    let s = store().lock().unwrap();
    let nodes: Vec<NodeView> = s
        .nodes
        .iter()
        .map(|n| NodeView {
            key: n.key.clone(),
            name: n.name.clone(),
            kind: n.kind.clone(),
            server: n
                .proxy
                .get("server")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            port: n
                .proxy
                .get("port")
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0) as u16,
            source: n.source.clone(),
            route_id: (running && n.port != 0).then(|| node_route(n, &s.listener_pass).id),
        })
        .collect();
    let mut kinds = BTreeMap::new();
    for n in &nodes {
        *kinds.entry(n.kind.clone()).or_insert(0) += 1;
    }
    MihomoStatus {
        enabled,
        supported_platform: asset().is_some(),
        installed: installed(),
        installing: st.installing,
        running,
        pinned_version: VERSION,
        version: st.version,
        error: st.error,
        nodes,
        kinds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(link: &str) -> Node {
        parse_link(link, RouteSource::Link).unwrap_or_else(|| panic!("no node for {link}"))
    }

    #[test]
    fn vless_reality() {
        let n = node("vless://1b2c3d4e-0000-4000-8000-000000000001@r.example.com:443?encryption=none&security=reality&sni=www.microsoft.com&fp=chrome&pbk=PUBKEY&sid=ab12&flow=xtls-rprx-vision&type=tcp#JP%20Reality");
        assert_eq!(n.name, "JP Reality");
        assert_eq!(n.kind, "vless");
        let p = &n.proxy;
        assert_eq!(p["server"], "r.example.com");
        assert_eq!(p["port"], 443);
        assert_eq!(p["uuid"], "1b2c3d4e-0000-4000-8000-000000000001");
        assert_eq!(p["tls"], true);
        assert_eq!(p["servername"], "www.microsoft.com");
        assert_eq!(p["flow"], "xtls-rprx-vision");
        assert_eq!(p["reality-opts"]["public-key"], "PUBKEY");
        assert_eq!(p["reality-opts"]["short-id"], "ab12");
        assert_eq!(p["client-fingerprint"], "chrome");
        assert!(p.get("network").is_none());
    }

    #[test]
    fn vless_ws_tls() {
        let n = node("vless://u@cdn.example.com:443?security=tls&type=ws&path=%2Fray&host=h.example.com&sni=h.example.com#WS");
        assert_eq!(n.proxy["network"], "ws");
        assert_eq!(n.proxy["ws-opts"]["path"], "/ray");
        assert_eq!(n.proxy["ws-opts"]["headers"]["Host"], "h.example.com");
    }

    #[test]
    fn vmess_base64_json() {
        let j = r#"{"v":"2","ps":"US 01","add":"v.example.com","port":"443","id":"uuid-1","aid":"0","scy":"auto","net":"ws","type":"none","host":"v.example.com","path":"/ws","tls":"tls","sni":"v.example.com"}"#;
        let link = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(j)
        );
        let n = node(&link);
        assert_eq!(n.name, "US 01");
        assert_eq!(n.proxy["uuid"], "uuid-1");
        assert_eq!(n.proxy["alterId"], 0);
        assert_eq!(n.proxy["tls"], true);
        assert_eq!(n.proxy["network"], "ws");
        assert_eq!(n.proxy["ws-opts"]["path"], "/ws");
    }

    #[test]
    fn hysteria2_and_tuic_and_anytls() {
        let n = node("hy2://pa%3Ass@h.example.com:8443/?sni=h.example.com&insecure=1&obfs=salamander&obfs-password=ob&mport=20000-30000#HY");
        assert_eq!(n.kind, "hysteria2");
        assert_eq!(n.proxy["password"], "pa:ss");
        assert_eq!(n.proxy["skip-cert-verify"], true);
        assert_eq!(n.proxy["obfs"], "salamander");
        assert_eq!(n.proxy["ports"], "20000-30000");

        let n = node("tuic://uuid-2:pw@t.example.com:443?congestion_control=bbr&alpn=h3&sni=t.example.com#TU");
        assert_eq!(n.proxy["uuid"], "uuid-2");
        assert_eq!(n.proxy["password"], "pw");
        assert_eq!(n.proxy["congestion-controller"], "bbr");
        assert_eq!(n.proxy["alpn"], json!(["h3"]));

        let n = node("anytls://secret@a.example.com:443?sni=a.example.com#AT");
        assert_eq!(n.kind, "anytls");
        assert_eq!(n.proxy["password"], "secret");
    }

    #[test]
    fn ss_plugin_only_when_needed() {
        // Plain AEAD stays native.
        assert!(parse_link(
            "ss://YWVzLTI1Ni1nY206cGFzcw@1.2.3.4:8388#A",
            RouteSource::Link
        )
        .is_none());
        let n = node("ss://YWVzLTI1Ni1nY206cGFzcw@1.2.3.4:8388/?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dbing.com#B");
        assert_eq!(n.proxy["plugin"], "obfs");
        assert_eq!(n.proxy["plugin-opts"]["mode"], "http");
        assert_eq!(n.proxy["plugin-opts"]["host"], "bing.com");
        // Trojan over TCP stays native, over ws goes to mihomo.
        assert!(parse_link("trojan://pw@t.example.com:443#T", RouteSource::Link).is_none());
        let n = node("trojan://pw@t.example.com:443?type=ws&path=%2Ftj&sni=t.example.com#TW");
        assert_eq!(n.proxy["network"], "ws");
    }

    #[test]
    fn ssr_link() {
        let inner = format!(
            "s.example.com:8388:auth_aes128_md5:aes-256-cfb:tls1.2_ticket_auth:{}/?remarks={}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("pw"),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("HK SSR"),
        );
        let link = format!(
            "ssr://{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(inner)
        );
        let n = node(&link);
        assert_eq!(n.name, "HK SSR");
        assert_eq!(n.proxy["password"], "pw");
        assert_eq!(n.proxy["protocol"], "auth_aes128_md5");
    }

    #[test]
    fn subscription_yaml_passthrough() {
        let y = r#"
proxies:
  - {name: "香港 01", type: ss, server: hk.example.com, port: 443, cipher: aes-128-gcm, password: pw}
  - {name: JP, type: trojan, server: jp.example.com, port: 443, password: pw}
  - {name: US, type: vless, server: us.example.com, port: 443, uuid: x, tls: true, dialer-proxy: other}
  - {name: SG, type: ss, server: sg.example.com, port: 443, cipher: aes-128-gcm, password: pw, plugin: obfs}
  - {name: TW, type: trojan, server: tw.example.com, port: 443, password: pw, network: grpc}
"#;
        let src = RouteSource::Subscription {
            url: "https://x".into(),
        };
        let nodes = nodes_from_subscription(y, src.clone());
        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["US", "SG", "TW"]);
        assert!(nodes[0].proxy.get("dialer-proxy").is_none());
        assert!(nodes[0].proxy.get("name").is_none());
        assert_eq!(nodes[0].source, src);
    }

    #[test]
    fn subscription_base64_list() {
        let list = "ss://YWVzLTI1Ni1nY206cGFzcw@1.2.3.4:8388#A\nvless://u@v.example.com:443?security=tls#V\nhysteria2://p@h.example.com:443#H\n";
        let b = base64::engine::general_purpose::STANDARD.encode(list);
        let nodes = nodes_from_subscription(&b, RouteSource::Manual);
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn pasted_text_dedupes() {
        let text = "节点：vless://u@v.example.com:443?security=tls#V，再来 vless://u@v.example.com:443?security=tls#Other name";
        assert_eq!(extract_nodes(text, RouteSource::Link).len(), 1);
    }

    #[test]
    fn config_has_one_listener_per_node() {
        let mut a = node("vless://u@v.example.com:443?security=tls#V");
        a.port = 40001;
        let mut b = node("hy2://p@h.example.com:443#H");
        b.port = 40002;
        let mut c = node("tuic://x:y@t.example.com:443#no port yet");
        c.port = 0;
        let text = build_config(&[a.clone(), b.clone(), c], "pw", 40100, "sec").unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert_eq!(doc["external-controller"].as_str(), Some("127.0.0.1:40100"));
        assert_eq!(doc["secret"].as_str(), Some("sec"));
        assert_eq!(doc["allow-lan"].as_bool(), Some(false));
        let proxies = doc["proxies"].as_sequence().unwrap();
        let listeners = doc["listeners"].as_sequence().unwrap();
        assert_eq!(proxies.len(), 2);
        assert_eq!(listeners.len(), 2);
        assert_eq!(
            proxies[0]["name"].as_str(),
            Some(a.internal_name().as_str())
        );
        assert_eq!(
            listeners[0]["proxy"].as_str(),
            Some(a.internal_name().as_str())
        );
        assert_eq!(listeners[0]["port"].as_u64(), Some(40001));
        assert_eq!(listeners[0]["listen"].as_str(), Some("127.0.0.1"));
        assert_eq!(listeners[0]["type"].as_str(), Some("socks"));
        assert_eq!(listeners[1]["users"][0]["password"].as_str(), Some("pw"));
        assert_eq!(doc["rules"][0].as_str(), Some("MATCH,DIRECT"));
    }

    #[test]
    fn route_is_stable_for_same_port() {
        let mut a = node("vless://u@v.example.com:443?security=tls#V");
        a.port = 40001;
        let r1 = node_route(&a, "pw");
        let r2 = node_route(&a, "pw");
        assert_eq!(r1.id, r2.id);
        assert_eq!(r1.kind, RouteKind::Socks5);
        assert_eq!(r1.username.as_deref(), Some(USER));
    }

    #[test]
    fn mirrors_after_github_and_proxies() {
        let plan = download_plan("a.gz", &["socks5h://127.0.0.1:7890".into()]);
        assert!(plan[0]
            .0
            .starts_with("https://github.com/MetaCubeX/mihomo/releases/download/"));
        assert_eq!(plan[1].1.as_deref(), Some("socks5h://127.0.0.1:7890"));
        assert!(plan[2]
            .0
            .starts_with("https://ghfast.top/https://github.com/"));
    }
}
