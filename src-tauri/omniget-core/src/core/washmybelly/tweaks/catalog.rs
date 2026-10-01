//! The tweak catalog: "tweak as data", never free PowerShell.
//!
//! Four embedded sources, each with its license and `SOURCE.md` under
//! `data/washmybelly/`:
//!
//! - `win11debloat-features/` (MIT): converted from `Features.json`; the ops
//!   come from the upstream `.reg` files, parsed by [`super::regfile`], and
//!   the Undo `.reg` is "Back to the Windows default".
//! - `winutil-tweaks/` (MIT): the registry part of a curated list of
//!   `tweaks.json` entries.
//! - `sophia-tweaks/` (MIT): a handful extracted by hand from `Sophia.psm1`.
//! - `tweaks/ours.json`: our own items (HAGS gating from SophiApp, memory
//!   integrity, power plans, update deferral) and the old `win_tweaks.rs`.
//!
//! Every op is checked on load against [`super::guard`]: an item that
//! touches something washmybelly never changes (Defender, CPU mitigations,
//! VBS, UCPD, Smart App Control, BitLocker, placebo keys) makes the whole
//! catalog fail to load, and a test keeps it that way.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::super::Risk;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Hive {
    #[serde(rename = "HKCU")]
    Hkcu,
    #[serde(rename = "HKLM")]
    Hklm,
    #[serde(rename = "HKU")]
    Hku,
}

impl Hive {
    pub fn as_str(self) -> &'static str {
        match self {
            Hive::Hkcu => "HKCU",
            Hive::Hklm => "HKLM",
            Hive::Hku => "HKU",
        }
    }
}

/// The closed list of things a tweak can do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TweakOp {
    RegSet {
        hive: Hive,
        key: String,
        /// Empty = the key's default value.
        name: String,
        /// `REG_DWORD`, `REG_QWORD`, `REG_SZ`, `REG_EXPAND_SZ`, `REG_MULTI_SZ`, `REG_BINARY`.
        kind: String,
        /// DWORD/QWORD in decimal, binary as hex without separators,
        /// multi-string joined with `\0`.
        data: String,
        /// What an absent value means to Windows (winutil's `DefaultState`):
        /// the op counts as applied when the value is absent and this equals
        /// `data`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        absent_means: Option<String>,
    },
    /// A value (`name`) or, with no name, the whole key.
    RegDelete {
        hive: Hive,
        key: String,
        #[serde(default)]
        name: Option<String>,
    },
    ServiceStartup {
        name: String,
        /// `automatic`, `delayed`, `manual`, `disabled`.
        mode: String,
    },
    TaskState {
        path: String,
        enabled: bool,
    },
    /// The active power plan; undo goes back to the plan that was active.
    PowerSetting {
        /// `balanced`, `high_performance`, `ultimate`.
        scheme: String,
    },
    OptionalFeature {
        name: String,
        enabled: bool,
    },
}

impl TweakOp {
    /// Needs `omniget-elevate`: anything outside the user's own hive.
    pub fn needs_admin(&self) -> bool {
        match self {
            TweakOp::RegSet { hive, .. } | TweakOp::RegDelete { hive, .. } => *hive != Hive::Hkcu,
            TweakOp::PowerSetting { .. } => false,
            TweakOp::ServiceStartup { .. }
            | TweakOp::TaskState { .. }
            | TweakOp::OptionalFeature { .. } => true,
        }
    }

    pub fn reg_target(&self) -> Option<(Hive, &str, Option<&str>)> {
        match self {
            TweakOp::RegSet {
                hive, key, name, ..
            } => Some((*hive, key, Some(name))),
            TweakOp::RegDelete { hive, key, name } => Some((*hive, key, name.as_deref())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applies {
    #[serde(default)]
    pub min_build: Option<u32>,
    #[serde(default)]
    pub max_build: Option<u32>,
    /// `EditionID` values (`Professional`, `Enterprise`...); none = every edition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editions: Option<Vec<String>>,
}

impl Applies {
    pub fn build_ok(&self, build: u32) -> bool {
        build == 0
            || (self.min_build.is_none_or(|m| build >= m)
                && self.max_build.is_none_or(|m| build <= m))
    }

    pub fn edition_ok(&self, edition: &str) -> bool {
        match &self.editions {
            None => true,
            Some(list) => {
                edition.is_empty() || list.iter().any(|e| e.eq_ignore_ascii_case(edition))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Tweak {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_key: Option<String>,
    pub description: String,
    pub category: String,
    pub applies: Applies,
    pub ops: Vec<TweakOp>,
    /// "Back to the Windows default" (the source's undo), when it has one.
    pub default_ops: Option<Vec<TweakOp>>,
    pub group: Option<String>,
    pub risk: Risk,
    pub needs_restart: bool,
    pub warning: Option<String>,
    /// Hardware checks: `hags` (dedicated GPU, not a VM, WDDM 2.7+).
    pub requires: Vec<String>,
    pub source: String,
    pub license: String,
}

impl Tweak {
    pub fn admin(&self) -> bool {
        self.ops.iter().any(TweakOp::needs_admin)
    }

    pub fn default_admin(&self) -> bool {
        self.default_ops
            .as_ref()
            .is_some_and(|d| d.iter().any(TweakOp::needs_admin))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub members: Vec<String>,
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Deserialize)]
struct RawTweak {
    id: String,
    label: String,
    #[serde(default)]
    label_key: Option<String>,
    #[serde(default)]
    description: String,
    category: String,
    #[serde(default)]
    applies: Applies,
    #[serde(default)]
    ops: Option<Vec<TweakOp>>,
    #[serde(default)]
    ops_reg: Option<String>,
    #[serde(default)]
    default_ops: Option<Vec<TweakOp>>,
    #[serde(default)]
    default_reg: Option<String>,
    #[serde(default)]
    group: Option<String>,
    risk: Risk,
    #[serde(default)]
    needs_restart: bool,
    #[serde(default)]
    warning: Option<String>,
    #[serde(default)]
    requires: Vec<String>,
    source: String,
    license: String,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    groups: Vec<Group>,
    items: Vec<RawTweak>,
}

pub struct Catalog {
    pub tweaks: Vec<Tweak>,
    pub groups: Vec<Group>,
    by_id: HashMap<String, usize>,
}

impl Catalog {
    pub fn get(&self, id: &str) -> Option<&Tweak> {
        self.by_id.get(id).map(|&i| &self.tweaks[i])
    }

    /// Every key a catalog op writes or deletes, as `HIVE\key`, upper case.
    /// The helper only deletes whole keys that are one of these or their
    /// ancestors (see `guard::key_delete_ok`).
    pub fn reg_keys(&self) -> HashSet<String> {
        self.tweaks
            .iter()
            .flat_map(|t| t.ops.iter().chain(t.default_ops.iter().flatten()))
            .filter_map(|op| {
                op.reg_target()
                    .map(|(h, k, _)| format!("{}\\{}", h.as_str(), k).to_ascii_uppercase())
            })
            .collect()
    }

    /// Keys a catalog op deletes whole (`[-KEY]` in a `.reg`).
    pub fn whole_key_deletes(&self) -> HashSet<String> {
        self.tweaks
            .iter()
            .flat_map(|t| t.ops.iter().chain(t.default_ops.iter().flatten()))
            .filter_map(|op| match op {
                TweakOp::RegDelete {
                    hive,
                    key,
                    name: None,
                } => Some(format!("{}\\{}", hive.as_str(), key).to_ascii_uppercase()),
                _ => None,
            })
            .collect()
    }
}

pub const CATEGORIES: &[&str] = &[
    "performance",
    "gaming",
    "taskbar_start",
    "appearance",
    "explorer",
    "bloat",
    "ai",
    "updates",
    "network",
    "system",
    "features",
    "hardening",
];

const REG_KINDS: &[&str] = &[
    "REG_DWORD",
    "REG_QWORD",
    "REG_SZ",
    "REG_EXPAND_SZ",
    "REG_MULTI_SZ",
    "REG_BINARY",
];
pub const SCHEMES: &[&str] = &["balanced", "high_performance", "ultimate"];
/// Optional features a tweak may switch (Win11Debloat's two).
pub const FEATURES: &[&str] = &[
    "Containers-DisposableClientVM",
    "Microsoft-Windows-Subsystem-Linux",
    "VirtualMachinePlatform",
];
const REQUIRES: &[&str] = &["hags"];

const SOURCES: &[(&str, &str)] = &[
    (
        "win11debloat-features",
        include_str!("../../../../data/washmybelly/win11debloat-features/features.json"),
    ),
    (
        "winutil-tweaks",
        include_str!("../../../../data/washmybelly/winutil-tweaks/tweaks.json"),
    ),
    (
        "sophia-tweaks",
        include_str!("../../../../data/washmybelly/sophia-tweaks/tweaks.json"),
    ),
    (
        "tweaks",
        include_str!("../../../../data/washmybelly/tweaks/ours.json"),
    ),
];

fn reg_file(name: &str) -> Result<Vec<TweakOp>, String> {
    let bytes = super::regfiles::REG_FILES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, b)| *b)
        .ok_or_else(|| format!("missing .reg {name}"))?;
    super::regfile::parse(&super::regfile::decode(bytes)?).map_err(|e| format!("{name}: {e}"))
}

/// A well-formed op: plain keys, known kinds, numbers that parse, names the
/// helper will accept, and nothing [`super::guard`] forbids.
pub fn validate_op(op: &TweakOp) -> Result<(), String> {
    if let Some((hive, key, name)) = op.reg_target() {
        super::guard::plain_key(key)?;
        if let Some(n) = name {
            if n.contains(['\0', '\n', '\r']) {
                return Err(format!("bad value name {n:?}"));
            }
        }
        let full = format!("{}\\{}", hive.as_str(), key);
        if let Some(why) = super::guard::forbidden(&full, name) {
            return Err(format!("FORBIDDEN: {full} {name:?}: {why}"));
        }
    }
    match op {
        TweakOp::RegSet {
            kind,
            data,
            absent_means,
            ..
        } => {
            if !REG_KINDS.contains(&kind.as_str()) {
                return Err(format!("unknown kind {kind}"));
            }
            super::state::norm_data(kind, data)
                .ok_or_else(|| format!("{kind} data {data:?} does not parse"))?;
            if let Some(a) = absent_means {
                super::state::norm_data(kind, a)
                    .ok_or_else(|| format!("absent_means {a:?} does not parse"))?;
            }
            Ok(())
        }
        TweakOp::RegDelete { .. } => Ok(()),
        TweakOp::ServiceStartup { name, mode } => {
            if !super::super::startup::valid_service_name(name)
                || !["automatic", "delayed", "manual", "disabled"].contains(&mode.as_str())
            {
                return Err(format!("bad service op {name} {mode}"));
            }
            if let Some(why) = super::guard::forbidden_service(name) {
                return Err(format!("FORBIDDEN: service {name}: {why}"));
            }
            Ok(())
        }
        TweakOp::TaskState { path, .. } => {
            if super::super::startup::valid_task_path(path) {
                Ok(())
            } else {
                Err(format!("bad task path {path}"))
            }
        }
        TweakOp::PowerSetting { scheme } => {
            if SCHEMES.contains(&scheme.as_str()) {
                Ok(())
            } else {
                Err(format!("unknown power scheme {scheme}"))
            }
        }
        TweakOp::OptionalFeature { name, .. } => {
            if FEATURES.contains(&name.as_str()) {
                Ok(())
            } else {
                Err(format!("optional feature {name} is not in the list"))
            }
        }
    }
}

/// Load and check every source. Pure over the embedded files.
pub fn load() -> Result<Catalog, String> {
    let mut tweaks = Vec::new();
    let mut groups = Vec::new();
    for (src, text) in SOURCES {
        let file: RawFile = serde_json::from_str(text).map_err(|e| format!("{src}: {e}"))?;
        groups.extend(file.groups);
        for r in file.items {
            let ops = match (r.ops, &r.ops_reg) {
                (Some(o), None) => o,
                (None, Some(f)) => reg_file(f)?,
                _ => return Err(format!("{}: needs exactly one of ops / ops_reg", r.id)),
            };
            let default_ops = match (r.default_ops, &r.default_reg) {
                (Some(o), None) => Some(o),
                (None, Some(f)) => Some(reg_file(f)?),
                (None, None) => None,
                _ => return Err(format!("{}: both default_ops and default_reg", r.id)),
            };
            if ops.is_empty() {
                return Err(format!("{}: no ops", r.id));
            }
            for op in ops.iter().chain(default_ops.iter().flatten()) {
                validate_op(op).map_err(|e| format!("{}: {e}", r.id))?;
            }
            if !CATEGORIES.contains(&r.category.as_str()) {
                return Err(format!("{}: unknown category {}", r.id, r.category));
            }
            if let Some(req) = r.requires.iter().find(|q| !REQUIRES.contains(&q.as_str())) {
                return Err(format!("{}: unknown requirement {req}", r.id));
            }
            if let Some(why) = super::guard::forbidden_words(&r.id, &r.label) {
                return Err(format!("{}: FORBIDDEN: {why}", r.id));
            }
            tweaks.push(Tweak {
                id: r.id,
                label: r.label,
                label_key: r.label_key,
                description: r.description,
                category: r.category,
                applies: r.applies,
                ops,
                default_ops,
                group: r.group,
                risk: r.risk,
                needs_restart: r.needs_restart,
                warning: r.warning,
                requires: r.requires,
                source: r.source,
                license: r.license,
            });
        }
    }
    let mut by_id = HashMap::new();
    for (i, t) in tweaks.iter().enumerate() {
        if by_id.insert(t.id.clone(), i).is_some() {
            return Err(format!("duplicate tweak id {}", t.id));
        }
        if !t
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(format!("tweak id {} has odd characters", t.id));
        }
    }
    for g in &groups {
        for m in &g.members {
            let t = by_id
                .get(m)
                .map(|&i| &tweaks[i])
                .ok_or_else(|| format!("group {} names unknown {m}", g.id))?;
            if t.group.as_deref() != Some(g.id.as_str()) {
                return Err(format!("{m} is in group {} but says {:?}", g.id, t.group));
            }
        }
    }
    for t in &tweaks {
        if let Some(g) = &t.group {
            if !groups.iter().any(|x| &x.id == g) {
                return Err(format!("{}: unknown group {g}", t.id));
            }
        }
    }
    Ok(Catalog {
        tweaks,
        groups,
        by_id,
    })
}

static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();

pub fn catalog() -> Result<&'static Catalog, String> {
    CATALOG
        .get_or_init(load)
        .as_ref()
        .map_err(|e| format!("tweak catalog: {e}"))
}
