use serde::Serialize;
use serde_json::Value;

#[cfg(any(target_os = "macos", test))]
pub mod collect;
#[cfg(any(target_os = "macos", test))]
pub mod parse;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Apple,
    AppStore,
    Notarized,
    Signed,
    Adhoc,
    Unsigned,
    Invalid,
    Missing,
    Unknown,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Default)]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub scope: String,
    pub name: String,
    pub path: Option<String>,
    pub program: Option<String>,
    pub script: Option<String>,
    pub command: Option<String>,
    pub checked: Option<String>,
    pub exists: bool,
    pub class: Option<Class>,
    pub summary: String,
    pub signer: Option<String>,
    pub team_id: Option<String>,
    pub notarized: Option<bool>,
    pub hardened_runtime: bool,
    pub signed_at: Option<String>,
    pub invalid_reason: Option<String>,
    pub quarantine: Option<QuarantineInfo>,
    pub installed: Option<i64>,
    pub state: Option<String>,
    pub third_party: bool,
    pub notes: Vec<String>,
    pub xprotect_matches: Vec<String>,
    pub startup_id: Option<String>,
    pub detail: Option<String>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct QuarantineInfo {
    pub agent: Option<String>,
    pub at: Option<i64>,
    pub approved: bool,
}

pub async fn tool_persistence(_a: Value) -> Result<Value, String> {
    #[cfg(target_os = "macos")]
    {
        super::tools::blocking(|| {
            let out = collect::collect(&super::startup::RealSys, &collect::Roots::real());
            Ok(out.to_json())
        })
        .await
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("wmb_persistence_list is only available on macOS".into())
    }
}

pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
        "open_fda_settings" => {
            #[cfg(target_os = "macos")]
            {
                crate::core::process::std_command("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
                    .status()
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "opened": true }))
            }
            #[cfg(not(target_os = "macos"))]
            Err("Full Disk Access exists only on macOS".into())
        }
        "open_login_items_settings" => {
            #[cfg(target_os = "macos")]
            {
                crate::core::process::std_command("open")
                    .arg("x-apple.systempreferences:com.apple.LoginItems-Settings.extension")
                    .status()
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "opened": true }))
            }
            #[cfg(not(target_os = "macos"))]
            Err("Login Items settings exist only on macOS".into())
        }
        _ => Err(format!("security: unknown screen action {action}")),
    }
}
