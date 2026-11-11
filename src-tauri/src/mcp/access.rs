//! What other programs may do through MCP. Everything is on until the person turns it off in
//! Settings › AI & agents. Web pages stay out regardless: local_bridge refuses any Origin.

use super::policy::{self, Principal, SCOPES};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{OnceLock, RwLock};

pub const LOCAL_ID: &str = "local";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Access {
    pub enabled: bool,
    /// Programs on this computer connect without a token.
    pub open: bool,
    /// Writes to accounts, system cleanups and new connections wait for a click in the window.
    pub ask_first: bool,
    pub disabled_scopes: Vec<String>,
    pub disabled_areas: Vec<String>,
}

impl Default for Access {
    fn default() -> Self {
        Self {
            enabled: true,
            open: true,
            ask_first: false,
            disabled_scopes: Vec::new(),
            disabled_areas: Vec::new(),
        }
    }
}

impl Access {
    pub fn scopes(&self) -> Vec<String> {
        SCOPES
            .iter()
            .filter(|s| !self.disabled_scopes.iter().any(|d| d == *s))
            .map(|s| s.to_string())
            .collect()
    }
}

fn file() -> Option<std::path::PathBuf> {
    omniget_core::core::tools::tools_dir().map(|d| d.join("mcp.json"))
}

fn read_file() -> Access {
    file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn current() -> &'static RwLock<Access> {
    static CURRENT: OnceLock<RwLock<Access>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new(read_file()))
}

pub fn get() -> Access {
    current().read().map(|a| a.clone()).unwrap_or_default()
}

pub fn set(access: Access) -> Result<(), String> {
    let path = file().ok_or("no data dir")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&access).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    if let Ok(mut slot) = current().write() {
        *slot = access.clone();
    }
    policy::sync_local(&access.scopes())
}

pub fn local() -> Principal {
    static SYNCED: OnceLock<()> = OnceLock::new();
    let scopes = get().scopes();
    SYNCED.get_or_init(|| {
        let _ = policy::sync_local(&scopes);
    });
    Principal {
        id: LOCAL_ID.into(),
        name: "Local".into(),
        scopes,
    }
}

/// Who is calling. A valid token keeps its identity, since agent grants hang on it; while the
/// server is open, it also gets every enabled scope, and no token at all is fine.
pub fn resolve(token: Option<&str>) -> Option<Principal> {
    let access = get();
    if let Some(token) = token {
        if let Ok(mut principal) = policy::authenticate(token) {
            if access.open {
                principal.scopes = access.scopes();
            }
            return Some(principal);
        }
        if let Some(session) = super::session_principal(token) {
            return Some(session);
        }
    }
    access.open.then(local)
}

/// While open, a connection sees every download, not only the ones it started.
pub fn sees_everything(p: &Principal) -> bool {
    p.id == LOCAL_ID
        || (get().open
            && !p.id.starts_with(super::SESSION_PREFIX)
            && !super::public::is_public(&p.id))
}

pub fn describe() -> Value {
    json!({
        "access": get(),
        "scopes": SCOPES,
        "areas": super::invoke::areas(),
    })
}

#[cfg(test)]
pub fn close_for_test() {
    if let Ok(mut slot) = current().write() {
        slot.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_on_until_turned_off() {
        let a = Access::default();
        assert!(a.enabled && a.open && !a.ask_first);
        assert_eq!(a.scopes().len(), SCOPES.len());
        let off = Access {
            disabled_scopes: vec!["social_write".into()],
            ..Access::default()
        };
        assert!(!off.scopes().iter().any(|s| s == "social_write"));
    }

    #[test]
    fn old_config_files_open_up() {
        let a: Access = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(a, Access::default());
    }
}
