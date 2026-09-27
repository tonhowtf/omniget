//! Persistence inventory on macOS: launchd, background tasks, helpers, profiles, extensions (F7).
//!
//! This is not an antivirus. It lists everything that is set up to start by
//! itself or to sit inside the system (LaunchAgents and LaunchDaemons, the
//! Background Task Management database, privileged helpers, configuration
//! profiles, system/network/driver extensions, third-party kernel
//! extensions, the user's crontab) and, for each one, states facts the
//! system itself can check: who signed it, whether Apple notarized it,
//! whether it still carries the download quarantine, when it was installed,
//! and whether the file it points to still exists. The classification is
//! neutral ("signed and notarized by X", "signed without notarization",
//! "not signed", "points to a file that does not exist"); nothing here
//! calls anything malware. Turning items off or removing them belongs to the
//! startup and uninstall modules, linked by id.
//!
//! Optional: when a `yara` binary is installed, the rules of XProtect that
//! already sit on the Mac are run against the programs found. The rules are
//! Apple's and are only read from the disk, never copied or shipped.

use serde::Serialize;
use serde_json::Value;

#[cfg(any(target_os = "macos", test))]
pub mod collect;
#[cfg(any(target_os = "macos", test))]
pub mod parse;

/// What the checks say about one item, in neutral terms.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Signed by Apple.
    Apple,
    /// Distributed through the Mac App Store.
    AppStore,
    /// Developer ID signature and Apple's notarization.
    Notarized,
    /// A valid signature without notarization.
    Signed,
    /// Signed without a developer identity (ad hoc, local builds).
    Adhoc,
    /// No signature.
    Unsigned,
    /// A signature that does not check out (modified or revoked).
    Invalid,
    /// The registration points to a file that is not there.
    Missing,
    /// Could not be checked (no path, no permission).
    Unknown,
}

/// One persistent item.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Default)]
pub struct Item {
    /// `security:<kind>:<path or identifier>`.
    pub id: String,
    /// `launch_agent`, `launch_daemon`, `background_task`, `privileged_helper`,
    /// `profile`, `system_extension`, `network_extension`, `kernel_extension`, `cron`.
    pub kind: String,
    /// `user` or `system`.
    pub scope: String,
    pub name: String,
    /// Where it is registered (plist, helper file, kext bundle, profile id).
    pub path: Option<String>,
    /// The program it runs, when there is one.
    pub program: Option<String>,
    /// The script an interpreter runs, when the program is `sh`, `node`...
    pub script: Option<String>,
    /// The full command line (launchd, cron).
    pub command: Option<String>,
    /// The file the signature checks looked at.
    pub checked: Option<String>,
    /// The checked file exists (always true for items without a file).
    pub exists: bool,
    pub class: Option<Class>,
    /// The same verdict as one English sentence (agents read this).
    pub summary: String,
    pub signer: Option<String>,
    pub team_id: Option<String>,
    /// Some(true) notarized, Some(false) checked and not, None unknown.
    pub notarized: Option<bool>,
    pub hardened_runtime: bool,
    pub signed_at: Option<String>,
    /// Why a signature failed, as codesign said it.
    pub invalid_reason: Option<String>,
    pub quarantine: Option<QuarantineInfo>,
    /// When the registration (or the file) was created, unix seconds.
    pub installed: Option<i64>,
    /// `enabled`/`disabled` (BTM), extension state, `loaded`/`not loaded` (kexts).
    pub state: Option<String>,
    pub third_party: bool,
    /// Neutral facts worth a look: `runs_script`, `inline_command`,
    /// `in_temp_folder`, `hidden_folder`, `quarantined`, `removal_disallowed`.
    pub notes: Vec<String>,
    /// Rules of the local XProtect that matched the checked file.
    pub xprotect_matches: Vec<String>,
    /// The startup module's id for this item (turn off / on there).
    pub startup_id: Option<String>,
    /// Extra detail: bundle id, version, payloads...
    pub detail: Option<String>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct QuarantineInfo {
    pub agent: Option<String>,
    pub at: Option<i64>,
    /// Gatekeeper already let the person open it.
    pub approved: bool,
}

/// `wmb_persistence_list`: read only, never offers a plan.
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

/// Actions only the screen may run (`washmybelly_ui` with `security.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, _a: Value) -> Result<Value, String> {
    match action {
        // Full Disk Access cannot be asked for; the person grants it in
        // System Settings, which this opens on the right pane.
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
        // Login Items & Extensions, where background items and extensions
        // are switched off by the person.
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
