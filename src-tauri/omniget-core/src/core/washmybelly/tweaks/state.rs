use serde::Serialize;
use serde_json::Value;

use super::super::parse::{reg_query, RegValue};
use super::super::startup::Sys;
use super::catalog::{Hive, TweakOp};

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

pub fn full_key(hive: Hive, key: &str, sid: Option<&str>) -> String {
    match (hive, sid) {
        (Hive::Hkcu, Some(sid)) => format!("HKU\\{sid}\\{key}"),
        _ => format!("{}\\{key}", hive.as_str()),
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ValState {
    Absent,
    Present { kind: String, data: String },
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

fn unset_default(v: &RegValue) -> bool {
    v.name.is_empty() && v.kind == "REG_SZ" && v.data.starts_with('(') && v.data.ends_with(')')
}

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

    pub fn exists(&self, full: &str) -> Option<bool> {
        match self.key(full) {
            KeyRead::Missing => Some(false),
            KeyRead::Denied => None,
            KeyRead::Values(_) => Some(true),
        }
    }
}

pub fn read_value(sys: &dyn Sys, full: &str, name: &str) -> ValState {
    Reader::new(sys).value(full, name)
}

pub fn key_exists(sys: &dyn Sys, full: &str) -> Option<bool> {
    Reader::new(sys).exists(full)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Tree {
    pub keys: Vec<String>,
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

pub fn service_mode(sys: &dyn Sys, name: &str) -> Option<Option<String>> {
    let o = sys
        .run(&["sc.exe".into(), "qc".into(), name.to_string()])
        .ok()?;
    if !o.ok {
        return if o.stdout.contains("1060") || o.stderr.contains("1060") {
            Some(None)
        } else {
            None
        };
    }
    super::super::startup::windows_start_mode(&o.stdout).map(Some)
}

pub fn parse_task_xml(xml: &str) -> Option<bool> {
    let xml = xml.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(xml).ok()?;
    let settings = doc.descendants().find(|n| n.has_tag_name("Settings"))?;
    match settings.children().find(|n| n.has_tag_name("Enabled")) {
        Some(n) => Some(n.text().unwrap_or("").trim().eq_ignore_ascii_case("true")),
        None => Some(true),
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

pub const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
pub const HIGH_PERFORMANCE: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
pub const ULTIMATE_TEMPLATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";

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

#[derive(Debug, Clone, Default, Serialize)]
pub struct Facts {
    pub build: u32,
    pub edition: String,
    pub hags_blocked: Option<&'static str>,
    #[serde(skip)]
    pub ultimate_guid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
}

pub const HW_SCRIPT: &str = "$ErrorActionPreference='SilentlyContinue'; $g=@(Get-CimInstance Win32_VideoController | Select-Object Name,AdapterCompatibility,AdapterDACType); $c=Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer,Model; [pscustomobject]@{gpus=$g;system=$c} | ConvertTo-Json -Compress -Depth 3";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState {
    Applied,
    NotApplied,
    Unknown,
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
