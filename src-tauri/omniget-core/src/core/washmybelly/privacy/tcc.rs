//! macOS privacy permissions (TCC): which app may use the camera, the
//! microphone, the screen, the whole disk... (F4).
//!
//! Read only from the person's `TCC.db` (and the system one when readable),
//! opened with `SQLITE_OPEN_READ_ONLY`. Both need Full Disk Access; without
//! it the list answers `{ available: false, reason: "full_disk_access" }`
//! and the screen sends the person to System Settings.
//!
//! Resetting is `tccutil reset <Service> <bundle-id>`, argv only, with the
//! service checked against [`SERVICES`] and the bundle id against a strict
//! pattern. It has no undo: the app simply asks again next time.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};

use crate::core::washmybelly::plan::{self, Op, Outcome, PlanItem};
use crate::core::washmybelly::Risk;

/// `(TCC service without the kTCCService prefix, screen key)`. The first
/// half is also the name `tccutil` takes. Closed list: nothing else is
/// ever passed to `tccutil`.
pub const SERVICES: &[(&str, &str)] = &[
    ("Camera", "camera"),
    ("Microphone", "microphone"),
    ("ScreenCapture", "screen"),
    ("Accessibility", "accessibility"),
    ("SystemPolicyAllFiles", "full_disk"),
    ("SystemPolicyDocumentsFolder", "documents"),
    ("SystemPolicyDownloadsFolder", "downloads"),
    ("SystemPolicyDesktopFolder", "desktop"),
    ("SystemPolicyNetworkVolumes", "network_volumes"),
    ("SystemPolicyRemovableVolumes", "removable_volumes"),
    ("SystemPolicyAppBundles", "app_management"),
    ("AppleEvents", "automation"),
    ("ListenEvent", "input_monitoring"),
    ("PostEvent", "input_control"),
    ("AddressBook", "contacts"),
    ("Calendar", "calendar"),
    ("Reminders", "reminders"),
    ("Photos", "photos"),
    ("PhotosAdd", "photos_add"),
    ("MediaLibrary", "media"),
    ("SpeechRecognition", "speech"),
    ("BluetoothAlways", "bluetooth"),
    ("DeveloperTool", "developer_tools"),
    ("UserTracking", "tracking"),
    ("FileProviderDomain", "file_provider"),
    ("Motion", "motion"),
    ("Willow", "home"),
    ("FocusStatus", "focus"),
];

fn service_key(service: &str) -> Option<&'static str> {
    SERVICES
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, k)| *k)
}

/// Bundle ids only (`com.example.App`): no path, no space, no quote.
pub fn valid_bundle(bundle: &str) -> bool {
    !bundle.is_empty()
        && bundle.len() <= 255
        && bundle.contains('.')
        && bundle.as_bytes()[0].is_ascii_alphanumeric()
        && bundle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

/// One row of `access`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Grant {
    /// Service without the `kTCCService` prefix (`Camera`).
    pub service: String,
    /// Bundle id or, for `client_type = 1`, an absolute path.
    pub client: String,
    pub client_type: i64,
    /// `allowed`, `denied`, `limited` or `unknown`.
    pub auth: &'static str,
    pub last_modified: Option<i64>,
    /// For Automation: the app being controlled.
    pub target: Option<String>,
    /// `user` or `system` database.
    pub db: &'static str,
}

impl Grant {
    pub fn can_reset(&self) -> bool {
        self.client_type == 0 && valid_bundle(&self.client) && service_key(&self.service).is_some()
    }

    pub fn item_id(&self) -> String {
        format!("privacy:tcc:{}:{}", self.service, self.client)
    }
}

fn auth_label(v: i64) -> &'static str {
    match v {
        0 => "denied",
        2 => "allowed",
        3 => "limited",
        _ => "unknown",
    }
}

/// Why the list could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    FullDiskAccess,
    NotFound,
    Unreadable,
}

impl Unavailable {
    pub fn as_str(self) -> &'static str {
        match self {
            Unavailable::FullDiskAccess => "full_disk_access",
            Unavailable::NotFound => "not_found",
            Unavailable::Unreadable => "unreadable",
        }
    }
}

/// `file:` URI for SQLite, with the characters it cares about escaped.
fn uri_of(path: &Path) -> String {
    let mut out = String::from("file:");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'%' | b'?' | b'#' | b' ' => out.push_str(&format!("%{b:02X}")),
            _ => out.push(b as char),
        }
    }
    out
}

fn open_read_only(path: &Path) -> Result<Connection, Unavailable> {
    // A plain open tells "no Full Disk Access" (EPERM) from "not there".
    match std::fs::File::open(path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Unavailable::NotFound),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(Unavailable::FullDiskAccess)
        }
        Err(_) => return Err(Unavailable::Unreadable),
    }
    let ro = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let probe = |c: &Connection| {
        c.query_row("SELECT count(*) FROM access", [], |r| r.get::<_, i64>(0))
            .is_ok()
    };
    if let Ok(c) = Connection::open_with_flags(path, ro) {
        if probe(&c) {
            return Ok(c);
        }
    }
    // A WAL database we cannot write the `-shm` of: read it as immutable.
    let uri = format!("{}?mode=ro&immutable=1", uri_of(path));
    match Connection::open_with_flags(uri, ro | OpenFlags::SQLITE_OPEN_URI) {
        Ok(c) if probe(&c) => Ok(c),
        _ => Err(Unavailable::Unreadable),
    }
}

/// Parse the `access` table of one TCC database. Columns change between
/// macOS versions, so the query is built from what the table has
/// (`allowed` before Big Sur, `auth_value` after).
pub fn read_db(path: &Path, db: &'static str) -> Result<Vec<Grant>, Unavailable> {
    let conn = open_read_only(path)?;
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(access)")
        .and_then(|mut st| st.query_map([], |r| r.get::<_, String>(1))?.collect())
        .map_err(|_| Unavailable::Unreadable)?;
    let has = |c: &str| cols.iter().any(|x| x == c);
    if !has("service") || !has("client") {
        return Err(Unavailable::Unreadable);
    }
    let auth = if has("auth_value") {
        "auth_value"
    } else if has("allowed") {
        "CASE WHEN allowed = 1 THEN 2 ELSE 0 END"
    } else {
        "1"
    };
    let ctype = if has("client_type") {
        "client_type"
    } else {
        "0"
    };
    let modified = if has("last_modified") {
        "last_modified"
    } else {
        "NULL"
    };
    let target = if has("indirect_object_identifier") {
        "indirect_object_identifier"
    } else {
        "NULL"
    };
    let sql = format!("SELECT service, client, {ctype}, {auth}, {modified}, {target} FROM access ORDER BY service, client");
    let mut st = conn.prepare(&sql).map_err(|_| Unavailable::Unreadable)?;
    let rows = st
        .query_map([], |r| {
            let service: String = r.get(0)?;
            let target: Option<String> = r.get(5)?;
            Ok(Grant {
                service: service
                    .strip_prefix("kTCCService")
                    .unwrap_or(&service)
                    .to_string(),
                client: r.get(1)?,
                client_type: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                auth: auth_label(r.get::<_, Option<i64>>(3)?.unwrap_or(1)),
                last_modified: r.get(4)?,
                target: target.filter(|t| !t.is_empty() && t != "UNUSED"),
                db,
            })
        })
        .map_err(|_| Unavailable::Unreadable)?;
    Ok(rows.flatten().collect())
}

fn user_db() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/Application Support/com.apple.TCC/TCC.db"))
}

const SYSTEM_DB: &str = "/Library/Application Support/com.apple.TCC/TCC.db";

/// Group grants by service in [`SERVICES`] order; unknown services last.
pub fn group(grants: &[Grant]) -> Vec<Value> {
    let mut order: Vec<String> = SERVICES.iter().map(|(s, _)| s.to_string()).collect();
    for g in grants {
        if !order.contains(&g.service) {
            order.push(g.service.clone());
        }
    }
    order
        .into_iter()
        .filter_map(|svc| {
            let apps: Vec<Value> = grants
                .iter()
                .filter(|g| g.service == svc)
                .map(|g| {
                    json!({
                        "client": g.client, "client_type": g.client_type, "auth": g.auth,
                        "last_modified": g.last_modified, "target": g.target, "db": g.db,
                        "can_reset": g.can_reset(),
                        "item_id": g.can_reset().then(|| g.item_id()),
                    })
                })
                .collect();
            (!apps.is_empty()).then(|| {
                json!({
                    "service": svc,
                    "key": service_key(&svc).unwrap_or("other"),
                    "allowed": apps.iter().filter(|a| a["auth"] == "allowed").count(),
                    "apps": apps,
                })
            })
        })
        .collect()
}

/// Plan candidates (kind `privacy`) for every row that can be reset. One
/// item per (service, app); automation targets of the same app share it.
pub fn plan_items(grants: &[Grant]) -> Vec<PlanItem> {
    let mut seen = std::collections::HashSet::new();
    grants
        .iter()
        .filter(|g| g.can_reset() && seen.insert(g.item_id()))
        .map(|g| PlanItem {
            id: g.item_id(),
            label: format!(
                "Reset {} permission of {}",
                service_key(&g.service).unwrap_or("?"),
                g.client
            ),
            detail: Some(format!("tccutil reset {} {}", g.service, g.client)),
            group: "tcc".into(),
            bytes: 0,
            risk: Risk::Destructive,
            admin: false,
            op: Op::Module {
                module: "privacy".into(),
                action: "tcc_reset".into(),
                args: json!({ "service": g.service, "bundle": g.client }),
            },
        })
        .collect()
}

fn list_now() -> Value {
    if super::super::current_os() != super::super::Os::Macos {
        return json!({ "available": false, "reason": "not_macos", "services": [] });
    }
    let Some(user) = user_db() else {
        return json!({ "available": false, "reason": "not_found", "services": [] });
    };
    let mut grants = match read_db(&user, "user") {
        Ok(g) => g,
        Err(why) => return json!({ "available": false, "reason": why.as_str(), "services": [] }),
    };
    let system = read_db(Path::new(SYSTEM_DB), "system");
    let system_read = system.is_ok();
    grants.extend(system.unwrap_or_default());
    plan::register("privacy", &plan_items(&grants));
    json!({
        "available": true,
        "system_db": system_read,
        "count": grants.len(),
        "services": group(&grants),
    })
}

/// Screen action `privacy.tcc_list` (routed by `privacy::ui_action`).
pub async fn tcc_list(_a: Value) -> Result<Value, String> {
    crate::core::washmybelly::tools::blocking(|| Ok(list_now())).await
}

/// `tccutil reset <Service> <bundle>`. Validates both against the closed
/// list and the bundle pattern before anything runs.
pub fn tcc_reset(service: &str, bundle: &str) -> Result<String, String> {
    let key = service_key(service).ok_or_else(|| format!("unknown privacy service {service}"))?;
    if !valid_bundle(bundle) {
        return Err(format!("not an app bundle id: {bundle}"));
    }
    #[cfg(target_os = "macos")]
    {
        let out = crate::core::process::std_command("/usr/bin/tccutil")
            .args(["reset", service, bundle])
            .output()
            .map_err(|e| format!("tccutil: {e}"))?;
        match out.status.code() {
            Some(0) => Ok(format!("{key} permission reset; {bundle} will ask again")),
            Some(70) => Err(format!("this macOS cannot reset the {key} permission")),
            _ => {
                let err = String::from_utf8_lossy(&out.stderr);
                Err(format!("tccutil: {}", err.trim()))
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = key;
        Err("privacy permissions can only be reset on macOS".into())
    }
}

/// What `privacy::plan_execute` calls for action `tcc_reset`.
pub fn plan_tcc_reset(args: &Value) -> Result<Outcome, String> {
    let service = args
        .get("service")
        .and_then(Value::as_str)
        .ok_or("service is missing")?;
    let bundle = args
        .get("bundle")
        .and_then(Value::as_str)
        .ok_or("bundle is missing")?;
    Ok(Outcome {
        detail: tcc_reset(service, bundle)?,
        bytes: 0,
        undo: None,
    })
}
