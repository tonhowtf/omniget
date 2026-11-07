use serde_json::{json, Value};

pub const API_BASE: &str = "https://discord.com/api/v9";
pub const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=9&encoding=json&compress=zlib-stream";
pub const REMOTE_AUTH_URL: &str = "wss://remote-auth-gateway.discord.gg/?v=2";
pub const ORIGIN: &str = "https://discord.com";
pub const CAPABILITIES: u64 = 4605;

pub fn user_agent() -> String {
    format!(
        "OmniGet/{} (+https://getomniget.com; {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    )
}

pub fn properties_for(os: &str) -> Value {
    json!({
        "os": os,
        "browser": "OmniGet",
        "device": "OmniGet",
    })
}

pub fn properties() -> Value {
    properties_for(std::env::consts::OS)
}

pub fn client_state() -> Value {
    json!({
        "guild_versions": {},
        "highest_last_message_id": "0",
        "read_state_version": 0,
        "user_guild_settings_version": -1,
        "user_settings_version": -1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_as_omniget() {
        let ua = user_agent();
        assert!(ua.starts_with("OmniGet/"));
        assert!(!ua.contains("Mozilla"));
        let p = properties_for("macos");
        assert_eq!(p["browser"], "OmniGet");
        assert_eq!(p["os"], "macos");
        assert!(p.get("client_build_number").is_none());
    }
}
