//! A route is one way of reaching Telegram's data centers: straight, through
//! a local proxy app (Clash, v2rayN…), through an MTProto proxy, or through a
//! Shadowsocks/Trojan server taken from a subscription. Everything the
//! connection layer knows how to dial is one of these.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteKind {
    Direct,
    Socks5,
    Http,
    MtProxy,
    Shadowsocks,
    Trojan,
}

/// Where a route came from. Drives the label in the UI and the order in
/// which automatic mode tries them (local apps first: they are what people
/// in mainland China already run and pay for).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RouteSource {
    Direct,
    Manual,
    Link,
    System,
    Env { var: String },
    LocalApp { app: String },
    Clash { config: String },
    Subscription { url: String },
    PublicList { list: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub id: String,
    pub kind: RouteKind,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// MTProto proxy secret, normalized to lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// Shadowsocks method (`aes-256-gcm`, `chacha20-ietf-poly1305`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cipher: Option<String>,
    /// TLS server name for Trojan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sni: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    /// Set when the endpoint reaches a single data center (Telegram's
    /// rescue addresses are per-DC): the bridge never uses it for others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dc_only: Option<i16>,
    pub source: RouteSource,
}

impl Route {
    pub fn direct() -> Self {
        let mut r = Route {
            id: String::new(),
            kind: RouteKind::Direct,
            name: "Direct".into(),
            host: String::new(),
            port: 0,
            username: None,
            password: None,
            secret: None,
            cipher: None,
            sni: None,
            insecure: false,
            dc_only: None,
            source: RouteSource::Direct,
        };
        r.id = r.compute_id();
        r
    }

    pub fn new(kind: RouteKind, host: impl Into<String>, port: u16, source: RouteSource) -> Self {
        let mut r = Route {
            id: String::new(),
            kind,
            name: String::new(),
            host: host.into(),
            port,
            username: None,
            password: None,
            secret: None,
            cipher: None,
            sni: None,
            insecure: false,
            dc_only: None,
            source,
        };
        r.finish();
        r
    }

    /// Recomputes the id and fills a default name. Call after mutating fields.
    pub fn finish(&mut self) {
        self.id = self.compute_id();
        if self.name.is_empty() {
            self.name = self.default_name();
        }
    }

    fn compute_id(&self) -> String {
        let mut h = Sha256::new();
        h.update(format!("{:?}", self.kind));
        h.update([0]);
        h.update(self.host.to_ascii_lowercase());
        h.update(self.port.to_be_bytes());
        for part in [
            &self.username,
            &self.password,
            &self.secret,
            &self.cipher,
            &self.sni,
        ] {
            h.update([0]);
            h.update(part.as_deref().unwrap_or(""));
        }
        let digest = h.finalize();
        hex::encode(&digest[..8])
    }

    fn default_name(&self) -> String {
        match self.kind {
            RouteKind::Direct => "Direct".into(),
            _ => format!("{} {}:{}", self.kind.label(), self.host, self.port),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(
            self.host.as_str(),
            "127.0.0.1" | "localhost" | "::1" | "[::1]" | "0.0.0.0"
        )
    }

    /// Link that another person can paste into OmniGet or Telegram.
    pub fn share_link(&self) -> Option<String> {
        use urlencoding::encode;
        match self.kind {
            RouteKind::Direct => None,
            RouteKind::MtProxy => Some(format!(
                "tg://proxy?server={}&port={}&secret={}",
                encode(&self.host),
                self.port,
                self.secret.as_deref().unwrap_or("")
            )),
            RouteKind::Socks5 => {
                let mut s = format!(
                    "tg://socks?server={}&port={}",
                    encode(&self.host),
                    self.port
                );
                if let Some(u) = &self.username {
                    s.push_str(&format!("&user={}", encode(u)));
                }
                if let Some(p) = &self.password {
                    s.push_str(&format!("&pass={}", encode(p)));
                }
                Some(s)
            }
            RouteKind::Http => Some(match (&self.username, &self.password) {
                (Some(u), Some(p)) => format!(
                    "http://{}:{}@{}:{}",
                    encode(u),
                    encode(p),
                    self.host,
                    self.port
                ),
                _ => format!("http://{}:{}", self.host, self.port),
            }),
            RouteKind::Shadowsocks => {
                use base64::Engine;
                let user = format!(
                    "{}:{}",
                    self.cipher.as_deref().unwrap_or(""),
                    self.password.as_deref().unwrap_or("")
                );
                let b = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(user);
                Some(format!(
                    "ss://{}@{}:{}#{}",
                    b,
                    self.host,
                    self.port,
                    encode(&self.name)
                ))
            }
            RouteKind::Trojan => {
                let mut s = format!(
                    "trojan://{}@{}:{}",
                    encode(self.password.as_deref().unwrap_or("")),
                    self.host,
                    self.port
                );
                let mut q = Vec::new();
                if let Some(sni) = &self.sni {
                    q.push(format!("sni={}", encode(sni)));
                }
                if self.insecure {
                    q.push("allowInsecure=1".into());
                }
                if !q.is_empty() {
                    s.push('?');
                    s.push_str(&q.join("&"));
                }
                s.push_str(&format!("#{}", encode(&self.name)));
                Some(s)
            }
        }
    }
}

impl RouteKind {
    pub fn label(self) -> &'static str {
        match self {
            RouteKind::Direct => "Direct",
            RouteKind::Socks5 => "SOCKS5",
            RouteKind::Http => "HTTP",
            RouteKind::MtProxy => "MTProto",
            RouteKind::Shadowsocks => "Shadowsocks",
            RouteKind::Trojan => "Trojan",
        }
    }
}
