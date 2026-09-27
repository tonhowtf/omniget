//! Reading the system: the state of each op ("applied" means Windows is in
//! that state now, never a flag we saved), the build and edition, and the
//! hardware checks some tweaks need.
//!
//! Every reader goes through [`Sys`] (argv only), so the tests drive them
//! with a fake registry; every text parser is a pure function tested with
//! fixtures on every OS.
//!
//! HAGS gating follows SophiApp (MIT, `Accessors.cs` `GPUScheduling`): a
//! dedicated GPU, not a virtual machine, and `WddmVersion_Min` ≥ 2700. The
//! GPU test is ours: SophiApp's `DetectDACType` returns true for almost any
//! adapter, so we look at the adapter names instead.

use serde::Serialize;
use serde_json::Value;

use super::super::parse::{reg_query, RegValue};
use super::super::startup::Sys;
use super::catalog::{Hive, TweakOp};

/// Canonical text of a value, for comparing what the system has with what
/// a tweak wants: numbers in decimal, binary as upper-case hex.
pub fn norm_data(kind: &str, data: &str) -> Option<String> {
    let d = data.trim();
    match kind {
        "REG_DWORD" => parse_num(d)
            .and_then(|n| u32::try_from(n).ok())
            .map(|n| n.to_string()),
        "REG_QWORD" => parse_num(d).map(|n| n.to_string()),
        "REG_BINARY" => {
            let hex: String = d.chars().filter(|c| !matches!(c, ',' | ' ')).collect();
            (hex.len().is_multiple_of(2) && hex.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| hex.to_ascii_uppercase())
        }
        "REG_SZ" | "REG_EXPAND_SZ" | "REG_MULTI_SZ" => Some(data.to_string()),
        _ => None,
    }
}

fn parse_num(d: &str) -> Option<u64> {
    match d.strip_prefix("0x").or_else(|| d.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => d.parse().ok(),
    }
}

/// `HKCU` keys are read as the signed-in user; the elevated helper gets them
/// as `HKU\<sid>` so an over-the-shoulder admin never writes its own hive.
pub fn full_key(hive: Hive, key: &str, sid: Option<&str>) -> String {
    match (hive, sid) {
        (Hive::Hkcu, Some(sid)) => format!("HKU\\{sid}\\{key}"),
        _ => format!("{}\\{key}", hive.as_str()),
    }
}

/// `reg.exe` refusing because of permissions, in the languages we ship.
pub fn denied(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    [
        "denied",
        "negado",
        "denegado",
        "refusé",
        "verweigert",
        "negato",
        "拒绝",
        "拒否",
        "отказано",
    ]
    .iter()
    .any(|w| s.contains(w))
}

/// What a value is right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ValState {
    Absent,
    Present {
        kind: String,
        data: String,
    },
    /// Could not read it (permissions): never guessed.
    Unknown,
}

pub fn reg_query_argv(full: &str, name: Option<&str>) -> Vec<String> {
    let mut v = vec!["reg".to_string(), "query".into(), full.to_string()];
    match name {
        Some("") => v.push("/ve".into()),
        Some(n) => {
            v.push("/v".into());
            v.push(n.to_string());
        }
        None => {}
    }
    v.push("/reg:64".into());
    v
}

/// The data `reg query` prints for a default value that was never set.
fn unset_default(v: &RegValue) -> bool {
    v.name.is_empty() && v.kind == "REG_SZ" && v.data.starts_with('(') && v.data.ends_with(')')
}

/// One key as `reg query <key>` (no `/s`) shows it.
#[derive(Debug, Clone)]
enum KeyRead {
    Missing,
    Denied,
    Values(Vec<RegValue>),
}

fn read_key(sys: &dyn Sys, full: &str) -> KeyRead {
    let Ok(o) = sys.run(&reg_query_argv(full, None)) else {
        return KeyRead::Denied;
    };
    if !o.ok {
        return if denied(&o.stderr) {
            KeyRead::Denied
        } else {
            KeyRead::Missing
        };
    }
    // The first key line is the key itself; later ones are its subkeys.
    let own = o
        .stdout
        .lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with(' '))
        .map(|l| l.trim().to_string())
        .unwrap_or_default();
    KeyRead::Values(
        reg_query(&o.stdout)
            .into_iter()
            .filter(|v| v.key == own)
            .collect(),
    )
}

/// Reads keys once and answers every value under them; [`Reader::prefetch`]
/// runs the `reg query` calls a few at a time.
pub struct Reader<'a> {
    pub sys: &'a dyn Sys,
    cache: std::sync::Mutex<std::collections::HashMap<String, KeyRead>>,
}

impl<'a> Reader<'a> {
    pub fn new(sys: &'a dyn Sys) -> Reader<'a> {
        Reader {
            sys,
            cache: Default::default(),
        }
    }

    fn key(&self, full: &str) -> KeyRead {
        let id = full.to_ascii_uppercase();
        if let Some(k) = self.cache.lock().ok().and_then(|c| c.get(&id).cloned()) {
            return k;
        }
        let k = read_key(self.sys, full);
        if let Ok(mut c) = self.cache.lock() {
            c.insert(id, k.clone());
        }
        k
    }

    /// Read these keys in parallel (8 at a time) before asking about them.
    pub fn prefetch(&self, keys: &[String]) {
        let chunks: Vec<&[String]> = keys.chunks(keys.len().div_ceil(8).max(1)).collect();
        std::thread::scope(|s| {
            for chunk in chunks {
                s.spawn(move || {
                    for k in chunk {
                        let _ = self.key(k);
                    }
                });
            }
        });
    }

    pub fn value(&self, full: &str, name: &str) -> ValState {
        match self.key(full) {
            KeyRead::Missing => ValState::Absent,
            KeyRead::Denied => ValState::Unknown,
            KeyRead::Values(vals) => {
                let found = vals.into_iter().find(|v| {
                    if name.is_empty() {
                        v.name.is_empty()
                    } else {
                        v.name.eq_ignore_ascii_case(name)
                    }
                });
                match found {
                    Some(v) if unset_default(&v) => ValState::Absent,
                    Some(v) => {
                        let data = norm_data(&v.kind, &v.data).unwrap_or(v.data);
                        ValState::Present { kind: v.kind, data }
                    }
                    None => ValState::Absent,
                }
            }
        }
    }

    /// `Some(true)` exists, `Some(false)` missing, `None` unreadable.
    pub fn exists(&self, full: &str) -> Option<bool> {
        match self.key(full) {
            KeyRead::Missing => Some(false),
            KeyRead::Denied => None,
            KeyRead::Values(_) => Some(true),
        }
    }
}

/// A value read fresh (no cache): what a snapshot records.
pub fn read_value(sys: &dyn Sys, full: &str, name: &str) -> ValState {
    Reader::new(sys).value(full, name)
}

pub fn key_exists(sys: &dyn Sys, full: &str) -> Option<bool> {
    Reader::new(sys).exists(full)
}

/// A key with everything under it, as `reg query /s` shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Tree {
    /// Every key, parents first.
    pub keys: Vec<String>,
    /// (key, name, kind, data)
    pub values: Vec<(String, String, String, String)>,
}

pub fn parse_tree(text: &str) -> Tree {
    let mut t = Tree::default();
    for line in text.lines() {
        if !line.trim().is_empty() && !line.starts_with(' ') {
            t.keys.push(line.trim().to_string());
        }
    }
    for v in reg_query(text) {
        if unset_default(&v) {
            continue;
        }
        let data = norm_data(&v.kind, &v.data).unwrap_or(v.data);
        t.values.push((v.key, v.name, v.kind, data));
    }
    t
}

/// `None` unreadable, `Some(None)` missing.
pub fn read_tree(sys: &dyn Sys, full: &str) -> Option<Option<Tree>> {
    let mut argv = reg_query_argv(full, None);
    argv.insert(3, "/s".into());
    let o = sys.run(&argv).ok()?;
    if o.ok {
        Some(Some(parse_tree(&o.stdout)))
    } else if denied(&o.stderr) {
        None
    } else {
        Some(None)
    }
}

/// Start type of a service: `None` unreadable, `Some(None)` no such service.
pub fn service_mode(sys: &dyn Sys, name: &str) -> Option<Option<String>> {
    let o = sys
        .run(&["sc.exe".into(), "qc".into(), name.to_string()])
        .ok()?;
    if !o.ok {
        // 1060: the service does not exist.
        return if o.stdout.contains("1060") || o.stderr.contains("1060") {
            Some(None)
        } else {
            None
        };
    }
    super::super::startup::windows_start_mode(&o.stdout).map(Some)
}

/// `<Enabled>` from `schtasks /query /xml` (not localized): missing = true.
pub fn parse_task_xml(xml: &str) -> Option<bool> {
    let xml = xml.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(xml).ok()?;
    let settings = doc.descendants().find(|n| n.has_tag_name("Settings"))?;
    match settings.children().find(|n| n.has_tag_name("Enabled")) {
        Some(n) => Some(n.text().unwrap_or("").trim().eq_ignore_ascii_case("true")),
        None => Some(true),
    }
}

/// `schtasks /query /tn X /fo csv /v`: the "Scheduled Task State" column
/// (by header, else position 11). Only English and Portuguese words are
/// known; anything else is `None`.
pub fn parse_task_csv(csv: &str) -> Option<bool> {
    let mut rows = csv.lines().filter(|l| !l.trim().is_empty()).map(split_csv);
    let head = rows.next()?;
    let col = head
        .iter()
        .position(|h| h == "Scheduled Task State" || h == "Estado da Tarefa Agendada")
        .unwrap_or(11);
    let row = rows.next()?;
    match row.get(col)?.to_ascii_lowercase().as_str() {
        "enabled" | "habilitado" | "habilitada" => Some(true),
        "disabled" | "desabilitado" | "desabilitada" => Some(false),
        _ => None,
    }
}

fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if q && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => q = !q,
            ',' if !q => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// `None` unreadable, `Some(None)` no such task.
pub fn task_enabled(sys: &dyn Sys, path: &str) -> Option<Option<bool>> {
    let o = sys
        .run(&[
            "schtasks".into(),
            "/query".into(),
            "/tn".into(),
            path.to_string(),
            "/xml".into(),
        ])
        .ok()?;
    if o.ok {
        return parse_task_xml(&o.stdout).map(Some);
    }
    if denied(&o.stderr) {
        return None;
    }
    Some(None)
}

// ── Power plans ─────────────────────────────────────────────────────

pub const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
pub const HIGH_PERFORMANCE: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
/// Hidden template `powercfg /duplicatescheme` copies into a real plan.
pub const ULTIMATE_TEMPLATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";

/// The first GUID in `powercfg` output (`/getactivescheme`, `/duplicatescheme`).
pub fn parse_guid(text: &str) -> Option<String> {
    static RE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(
            r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        )
        .unwrap()
    });
    RE.find(text).map(|m| m.as_str().to_ascii_lowercase())
}

pub fn active_scheme(sys: &dyn Sys) -> Option<String> {
    let o = sys
        .run(&["powercfg".into(), "/getactivescheme".into()])
        .ok()?;
    o.ok.then(|| parse_guid(&o.stdout)).flatten()
}

/// Whether the active plan is the one a scheme names. Ultimate is a copy
/// with a new GUID, so we compare with the copy we made (kept in kv).
pub fn scheme_matches(scheme: &str, active: &str, ultimate: Option<&str>) -> bool {
    match scheme {
        "balanced" => active == BALANCED,
        "high_performance" => active == HIGH_PERFORMANCE,
        "ultimate" => {
            active == ULTIMATE_TEMPLATE || ultimate.is_some_and(|u| u.eq_ignore_ascii_case(active))
        }
        _ => false,
    }
}

// ── Optional features ───────────────────────────────────────────────

/// Fixed script; the feature name travels in `OMNIGET_WMB_FEATURE` and is
/// checked against `catalog::FEATURES` first. Prints `InstallState`
/// (1 enabled, 2 disabled, 3 absent) or `missing`.
pub const FEATURE_SCRIPT: &str = "$ErrorActionPreference='Stop'; $n=$env:OMNIGET_WMB_FEATURE; if ($n -notmatch '^[A-Za-z0-9.-]+$') { exit 2 }; $f=Get-CimInstance -ClassName Win32_OptionalFeature -Filter (\"Name='\" + $n + \"'\"); if ($f) { $f.InstallState } else { 'missing' }";

pub fn parse_feature_state(text: &str) -> Option<Option<bool>> {
    match text.trim() {
        "1" => Some(Some(true)),
        "2" | "3" => Some(Some(false)),
        "missing" => Some(None),
        _ => None,
    }
}

pub fn feature_enabled(sys: &dyn Sys, name: &str) -> Option<Option<bool>> {
    if !super::catalog::FEATURES.contains(&name) {
        return None;
    }
    let argv = [
        "powershell",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        FEATURE_SCRIPT,
    ]
    .map(String::from);
    let o = sys
        .run_env(&argv, &[("OMNIGET_WMB_FEATURE".into(), name.to_string())])
        .ok()?;
    o.ok.then(|| parse_feature_state(&o.stdout)).flatten()
}

// ── The machine ─────────────────────────────────────────────────────

/// What the list needs to know about this Windows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Facts {
    pub build: u32,
    pub edition: String,
    /// Why HAGS is not offered (`no_dedicated_gpu`, `virtual_machine`,
    /// `old_driver`, `unknown`), or `None` when it is.
    pub hags_blocked: Option<&'static str>,
    #[serde(skip)]
    pub ultimate_guid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
    pub dac: String,
}

/// Fixed script: video controllers and the computer model, as JSON.
pub const HW_SCRIPT: &str = "$ErrorActionPreference='SilentlyContinue'; $g=@(Get-CimInstance Win32_VideoController | Select-Object Name,AdapterCompatibility,AdapterDACType); $c=Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer,Model; [pscustomobject]@{gpus=$g;system=$c} | ConvertTo-Json -Compress -Depth 3";

/// `(gpus, "manufacturer model")` from [`HW_SCRIPT`]'s JSON.
pub fn parse_hw(json: &str) -> Option<(Vec<Gpu>, String)> {
    let v: Value = serde_json::from_str(json.trim().trim_start_matches('\u{feff}')).ok()?;
    let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let list = match v.get("gpus") {
        Some(Value::Array(a)) => a.clone(),
        Some(o @ Value::Object(_)) => vec![o.clone()],
        _ => vec![],
    };
    let gpus = list
        .iter()
        .map(|g| Gpu {
            name: s(g, "Name"),
            vendor: s(g, "AdapterCompatibility"),
            dac: s(g, "AdapterDACType"),
        })
        .collect();
    let sys = v.get("system").cloned().unwrap_or(Value::Null);
    Some((
        gpus,
        format!("{} {}", s(&sys, "Manufacturer"), s(&sys, "Model"))
            .trim()
            .to_string(),
    ))
}

/// A graphics card with its own memory (not the processor's graphics).
pub fn dedicated(g: &Gpu) -> bool {
    static INTEL_DGPU: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"\barc\b.*\b[ab]\d{3}m?\b").unwrap());
    let n = g.name.to_ascii_lowercase();
    let v = g.vendor.to_ascii_lowercase();
    if n.contains("nvidia") || v.contains("nvidia") {
        return true;
    }
    if n.contains("radeon")
        || n.contains("amd")
        || v.contains("advanced micro devices")
        || v.contains("amd")
    {
        // APUs are "Radeon(TM) Graphics", "Radeon Vega 8 Graphics", "Radeon 780M Graphics".
        if n.trim_end().ends_with("graphics") {
            return false;
        }
        return n.contains("rx")
            || n.contains("radeon pro")
            || n.contains("radeon vii")
            || n.contains("firepro")
            || n.contains(" r9 ")
            || n.contains(" r7 ");
    }
    if n.contains("intel") || v.contains("intel") {
        return INTEL_DGPU.is_match(&n);
    }
    false
}

pub fn is_vm(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    [
        "virtual",
        "vmware",
        "virtualbox",
        "kvm",
        "qemu",
        "hvm domu",
        "parallels",
        "bochs",
        "xen",
    ]
    .iter()
    .any(|t| m.contains(t))
}

pub const WDDM_MIN: u64 = 2700;

pub fn hags_blocked(gpus: &[Gpu], model: &str, wddm: Option<u64>) -> Option<&'static str> {
    if is_vm(model) {
        return Some("virtual_machine");
    }
    if !gpus.iter().any(dedicated) {
        return Some("no_dedicated_gpu");
    }
    match wddm {
        Some(w) if w >= WDDM_MIN => None,
        Some(_) => Some("old_driver"),
        None => Some("unknown"),
    }
}

/// `whoami /user /fo csv /nh` → the SID column.
pub fn parse_whoami_user(csv: &str) -> Option<String> {
    let line = csv.lines().find(|l| !l.trim().is_empty())?;
    let cols = split_csv(line.trim());
    cols.into_iter()
        .rev()
        .find(|c| super::guard::user_sid(c.trim()))
        .map(|c| c.trim().to_string())
}

pub fn current_sid(sys: &dyn Sys) -> Option<String> {
    let o = sys
        .run(&[
            "whoami".into(),
            "/user".into(),
            "/fo".into(),
            "csv".into(),
            "/nh".into(),
        ])
        .ok()?;
    o.ok.then(|| parse_whoami_user(&o.stdout)).flatten()
}

pub fn read_facts(sys: &dyn Sys, want_hags: bool) -> Facts {
    let cv = sys
        .run(&reg_query_argv(
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            None,
        ))
        .map(|o| o.stdout)
        .unwrap_or_default();
    let vals = reg_query(&cv);
    let get = |n: &str| {
        vals.iter()
            .find(|v| v.name == n)
            .map(|v| v.data.clone())
            .unwrap_or_default()
    };
    let mut f = Facts {
        build: get("CurrentBuild").parse().unwrap_or(0),
        edition: get("EditionID"),
        ..Facts::default()
    };
    if want_hags {
        let wddm = match read_value(
            sys,
            r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers\FeatureSetUsage",
            "WddmVersion_Min",
        ) {
            ValState::Present { data, .. } => data.parse().ok(),
            _ => None,
        };
        let argv = [
            "powershell",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            HW_SCRIPT,
        ]
        .map(String::from);
        let hw = sys
            .run(&argv)
            .ok()
            .filter(|o| o.ok)
            .and_then(|o| parse_hw(&o.stdout));
        f.hags_blocked = match hw {
            Some((gpus, model)) => hags_blocked(&gpus, &model, wddm),
            None => Some("unknown"),
        };
    }
    f
}

// ── Detection ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState {
    Applied,
    NotApplied,
    Unknown,
    /// The thing is not on this machine (a task or service that is not
    /// installed): it does not count.
    NotApplicable,
}

pub fn op_state(r: &Reader, op: &TweakOp, facts: &Facts) -> OpState {
    use OpState::*;
    let sys = r.sys;
    match op {
        TweakOp::RegSet {
            hive,
            key,
            name,
            kind,
            data,
            absent_means,
        } => match r.value(&full_key(*hive, key, None), name) {
            ValState::Present { kind: k, data: d } => {
                if &k == kind && norm_data(kind, &d) == norm_data(kind, data) {
                    Applied
                } else {
                    NotApplied
                }
            }
            ValState::Absent => {
                if absent_means
                    .as_deref()
                    .is_some_and(|a| norm_data(kind, a) == norm_data(kind, data))
                {
                    Applied
                } else {
                    NotApplied
                }
            }
            ValState::Unknown => Unknown,
        },
        TweakOp::RegDelete {
            hive,
            key,
            name: Some(n),
        } => match r.value(&full_key(*hive, key, None), n) {
            ValState::Absent => Applied,
            ValState::Present { .. } => NotApplied,
            ValState::Unknown => Unknown,
        },
        TweakOp::RegDelete {
            hive,
            key,
            name: None,
        } => match r.exists(&full_key(*hive, key, None)) {
            Some(false) => Applied,
            Some(true) => NotApplied,
            None => Unknown,
        },
        TweakOp::ServiceStartup { name, mode } => match service_mode(sys, name) {
            Some(Some(m)) if &m == mode => Applied,
            Some(Some(_)) => NotApplied,
            Some(None) => NotApplicable,
            None => Unknown,
        },
        TweakOp::TaskState { path, enabled } => match task_enabled(sys, path) {
            Some(Some(e)) if e == *enabled => Applied,
            Some(Some(_)) => NotApplied,
            Some(None) => NotApplicable,
            None => Unknown,
        },
        TweakOp::PowerSetting { scheme } => match active_scheme(sys) {
            Some(a) if scheme_matches(scheme, &a, facts.ultimate_guid.as_deref()) => Applied,
            Some(_) => NotApplied,
            None => Unknown,
        },
        TweakOp::OptionalFeature { name, enabled } => match feature_enabled(sys, name) {
            Some(Some(e)) if e == *enabled => Applied,
            Some(Some(_)) => NotApplied,
            Some(None) => NotApplicable,
            None => Unknown,
        },
    }
}

/// `applied` when every op that counts is in place, `not_applied` when none
/// is, `partial` in between, `unknown` when one could not be read.
pub fn aggregate(states: &[OpState]) -> &'static str {
    let counted: Vec<OpState> = states
        .iter()
        .copied()
        .filter(|s| *s != OpState::NotApplicable)
        .collect();
    if counted.is_empty() || counted.contains(&OpState::Unknown) {
        return "unknown";
    }
    let on = counted.iter().filter(|s| **s == OpState::Applied).count();
    if on == counted.len() {
        "applied"
    } else if on == 0 {
        "not_applied"
    } else {
        "partial"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/washmybelly_fixtures/tweaks/"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIX}{name}")).unwrap()
    }

    #[test]
    fn numbers_and_binary_compare_by_value() {
        assert_eq!(norm_data("REG_DWORD", "0x1"), Some("1".into()));
        assert_eq!(
            norm_data("REG_DWORD", "255"),
            norm_data("REG_DWORD", "0xff")
        );
        assert_eq!(norm_data("REG_DWORD", "0x100000000"), None);
        assert_eq!(norm_data("REG_QWORD", "0x1"), Some("1".into()));
        assert_eq!(
            norm_data("REG_BINARY", "90,12,07,80"),
            Some("90120780".into())
        );
        assert_eq!(norm_data("REG_BINARY", "9"), None);
    }

    #[test]
    fn a_reg_query_tree_lists_keys_and_values() {
        let t = parse_tree(&fixture("reg_query_tree.txt"));
        assert_eq!(t.keys.len(), 3);
        assert!(t.keys[2].ends_with("\\Empty"));
        assert_eq!(t.values.len(), 2, "the unset default is not a value");
        assert!(t.values.iter().any(|(_, n, k, d)| n.is_empty()
            && k == "REG_SZ"
            && d == "{f81e9010-6ea4-11ce-a7ff-00aa003ca9f6}"));
    }

    #[test]
    fn task_state_reads_from_xml_and_csv() {
        assert_eq!(
            parse_task_xml(&fixture("schtasks_disabled.xml")),
            Some(false)
        );
        assert_eq!(
            parse_task_xml("<Task><Settings><Hidden>true</Hidden></Settings></Task>"),
            Some(true)
        );
        assert_eq!(
            parse_task_csv(&fixture("schtasks_verbose.csv")),
            Some(false)
        );
    }

    #[test]
    fn powercfg_output_gives_the_guid() {
        assert_eq!(
            parse_guid(&fixture("powercfg_active.txt")).as_deref(),
            Some(HIGH_PERFORMANCE)
        );
        assert!(scheme_matches(
            "ultimate",
            "11111111-2222-3333-4444-555555555555",
            Some("11111111-2222-3333-4444-555555555555")
        ));
        assert!(!scheme_matches("balanced", HIGH_PERFORMANCE, None));
    }

    #[test]
    fn hags_needs_a_real_dedicated_gpu() {
        let (gpus, model) = parse_hw(&fixture("hw_desktop_nvidia.json")).unwrap();
        assert_eq!(gpus.len(), 2);
        assert!(!dedicated(&gpus[0]) && dedicated(&gpus[1]));
        assert_eq!(hags_blocked(&gpus, &model, Some(3100)), None);
        assert_eq!(hags_blocked(&gpus, &model, Some(2600)), Some("old_driver"));
        let (gpus, model) = parse_hw(&fixture("hw_laptop_apu.json")).unwrap();
        assert_eq!(
            hags_blocked(&gpus, &model, Some(3100)),
            Some("no_dedicated_gpu")
        );
        let (gpus, model) = parse_hw(&fixture("hw_vm.json")).unwrap();
        assert_eq!(
            hags_blocked(&gpus, &model, Some(3100)),
            Some("virtual_machine")
        );
        let arc = |n: &str| {
            dedicated(&Gpu {
                name: n.into(),
                vendor: "Intel Corporation".into(),
                dac: "Internal".into(),
            })
        };
        assert!(arc("Intel(R) Arc(TM) A770 Graphics") && arc("Intel(R) Arc(TM) B580 Graphics"));
        assert!(!arc("Intel(R) Arc(TM) Graphics") && !arc("Intel(R) UHD Graphics 770"));
        let amd = |n: &str| {
            dedicated(&Gpu {
                name: n.into(),
                vendor: "Advanced Micro Devices, Inc.".into(),
                dac: String::new(),
            })
        };
        assert!(amd("AMD Radeon RX 7800 XT") && amd("AMD Radeon RX 6500M"));
        assert!(
            !amd("AMD Radeon(TM) Graphics")
                && !amd("AMD Radeon 780M Graphics")
                && !amd("Radeon RX Vega 11 Graphics")
        );
    }

    #[test]
    fn whoami_gives_the_user_sid() {
        assert_eq!(
            parse_whoami_user(
                "\"desktop-1\\ana\",\"S-1-5-21-1111111111-2222222222-3333333333-1001\"\r\n"
            )
            .as_deref(),
            Some("S-1-5-21-1111111111-2222222222-3333333333-1001")
        );
        assert_eq!(
            parse_whoami_user("\"nt authority\\system\",\"S-1-5-18\""),
            None
        );
    }

    #[test]
    fn features_and_aggregate() {
        assert_eq!(parse_feature_state("1\r\n"), Some(Some(true)));
        assert_eq!(parse_feature_state("2"), Some(Some(false)));
        assert_eq!(parse_feature_state("missing"), Some(None));
        assert_eq!(parse_feature_state("oops"), None);
        use OpState::*;
        assert_eq!(aggregate(&[Applied, Applied, NotApplicable]), "applied");
        assert_eq!(aggregate(&[Applied, NotApplied]), "partial");
        assert_eq!(aggregate(&[NotApplied]), "not_applied");
        assert_eq!(aggregate(&[Applied, Unknown]), "unknown");
        assert_eq!(aggregate(&[NotApplicable]), "unknown");
    }
}
