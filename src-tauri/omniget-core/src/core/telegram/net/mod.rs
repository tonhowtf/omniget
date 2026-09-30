//! Connection layer. Finds and tests every way out (local proxy apps, the
//! system proxy, MTProto proxies, Shadowsocks/Trojan servers from a
//! subscription, plain direct) and hands the client a working stream.

pub mod bridge;
pub mod clock;
pub mod dc;
pub mod detect;
pub mod dns;
pub mod doctor;
pub mod links;
pub mod manager;
pub mod mihomo;
pub mod obfs;
pub mod packets;
pub mod probe;
pub mod rescue;
pub mod route;
pub mod shadowsocks;
pub mod subscription;
pub mod trojan;
pub mod tunnel;

use serde::{Deserialize, Serialize};

/// Knobs every dial reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetOpts {
    /// Dial DCs over IPv6. Off by default: IPv6 through a proxy on an
    /// IPv4-only network is the classic silent failure (tdl #754).
    #[serde(default)]
    pub prefer_ipv6: bool,
    /// Resolve proxy hostnames with DNS-over-HTTPS only.
    #[serde(default)]
    pub prefer_doh: bool,
    #[serde(default = "default_timeout")]
    pub connect_timeout_secs: u64,
}

fn default_timeout() -> u64 {
    10
}

impl Default for NetOpts {
    fn default() -> Self {
        Self {
            prefer_ipv6: false,
            prefer_doh: false,
            connect_timeout_secs: default_timeout(),
        }
    }
}
