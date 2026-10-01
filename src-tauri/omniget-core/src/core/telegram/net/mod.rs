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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetOpts {
    #[serde(default)]
    pub prefer_ipv6: bool,
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
