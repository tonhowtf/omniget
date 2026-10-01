use std::collections::BTreeMap;
use std::sync::OnceLock;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::compiler::{self, Call, CategoryData, Collection, Compiler, Node, ScriptData, Visit};
use crate::core::washmybelly::{Os, Risk};

pub const WINDOWS_YAML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/washmybelly/privacy-sexy/windows.yaml"
));
pub const MACOS_YAML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/washmybelly/privacy-sexy/macos.yaml"
));
pub const LINUX_YAML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/washmybelly/privacy-sexy/linux.yaml"
));

pub const EXTREME_TOPS: &[&str] = &["Privacy over security", "Remove bloatware"];

static LEFT_OUT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)defender|smartscreen|windows security|spectre|meltdown|virtualization-based|\bvbs\b|\bucpd\b|smart app control|bitlocker|sehop|control flow guard|exploit guard|\bamsi\b|antimalware")
        .unwrap()
});

const LEFT_OUT_FUNCTIONS: &[&str] = &["SetMpPreference"];

const CLEANUP_FUNCTIONS: &[&str] = &[
    "ClearDirectoryContents",
    "DeleteFiles",
    "SoftDeleteFiles",
    "DeleteDirectory",
    "DeleteGlob",
    "DeleteFilesFromFirefoxProfiles",
    "ClearRegistryValues",
    "DeleteFileFromThunderbirdProfiles",
    "DeleteFileFromUserAndRootHome",
    "ClearDirFromVsCodeUserDataDir",
    "DeleteFileFromGNOMEWebData",
    "CleanTableFromFirefoxProfileDatabase",
    "IterateGlob",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RegRevert {
    None,
    Delete,
    Set { kind: String, data: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case")]
pub enum Fact {
    RegSet {
        key: String,
        name: String,
        kind: String,
        data: String,
        revert: RegRevert,
    },
    RegDelete {
        key: String,
        name: String,
        revert: RegRevert,
    },
    RegKeyGone {
        key: String,
    },
    Service {
        name: String,
        default_mode: String,
    },
    ServiceReg {
        name: String,
        default_mode: String,
    },
    Task {
        path: String,
        name: String,
        enable_on_revert: bool,
    },
    HostsBlock {
        domain: String,
    },
    StoreApp {
        package: String,
    },
    DisallowRun {
        exe: String,
    },
    Defaults {
        domain: String,
        key: String,
        kind: Option<String>,
        value: String,
        sudo: bool,
    },
    DefaultsDelete {
        domain: String,
        key: String,
        sudo: bool,
    },
    TccReset {
        service: String,
    },
    Systemd {
        unit: String,
    },
    Package {
        manager: String,
        name: String,
    },
    FirefoxPref {
        name: String,
        value: String,
    },
    VsCodeSetting {
        name: String,
        json: String,
    },
    ConfigLine {
        path: String,
        on: String,
        off: String,
    },
    CommandExists {
        command: String,
    },
    Opaque {
        what: String,
    },
}

impl Fact {
    pub fn executable(&self, os: Os) -> bool {
        match self {
            Fact::RegSet { key, .. } | Fact::RegDelete { key, .. } => {
                os == Os::Windows && hive(key).is_some() && !key.contains('$')
            }
            Fact::Service { .. } | Fact::ServiceReg { .. } | Fact::Task { .. } => os == Os::Windows,
            Fact::Defaults { sudo, .. } | Fact::DefaultsDelete { sudo, .. } => {
                os == Os::Macos && !sudo
            }
            Fact::TccReset { service } => {
                os == Os::Macos
                    && service != "All"
                    && !service.is_empty()
                    && service.bytes().all(|b| b.is_ascii_alphanumeric())
            }
            Fact::Systemd { .. } | Fact::FirefoxPref { .. } => os == Os::Linux,
            Fact::CommandExists { .. } => true,
            _ => false,
        }
    }

    pub fn admin(&self) -> bool {
        match self {
            Fact::RegSet { key, .. } | Fact::RegDelete { key, .. } => hive(key) != Some(Hive::User),
            Fact::Service { .. }
            | Fact::ServiceReg { .. }
            | Fact::Task { .. }
            | Fact::Systemd { .. }
            | Fact::Package { .. } => true,
            Fact::Defaults { sudo, .. } | Fact::DefaultsDelete { sudo, .. } => *sudo,
            Fact::HostsBlock { .. } | Fact::StoreApp { .. } => true,
            _ => false,
        }
    }

    pub fn destructive(&self) -> bool {
        matches!(self, Fact::TccReset { .. } | Fact::Package { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    User,
    Machine,
}

pub fn hive(key: &str) -> Option<Hive> {
    let root = key.split('\\').next()?.to_ascii_uppercase();
    match root.as_str() {
        "HKCU" | "HKEY_CURRENT_USER" => Some(Hive::User),
        "HKLM" | "HKEY_LOCAL_MACHINE" | "HKU" | "HKEY_USERS" | "HKCR" | "HKEY_CLASSES_ROOT" => {
            Some(Hive::Machine)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Builds {
    pub min: Option<u32>,
    pub max: Option<u32>,
}

impl Builds {
    pub fn contains(&self, build: u32) -> bool {
        self.min.is_none_or(|m| build >= m) && self.max.is_none_or(|m| build <= m)
    }
}

pub fn windows_build(name: &str, maximum: bool) -> Option<u32> {
    Some(match (name.trim(), maximum) {
        ("Windows11-FirstRelease", _) | ("Windows11-21H2", _) => 22000,
        ("Windows11-22H2", false) => 22621,
        ("Windows10-22H2", _) => 19045,
        ("Windows10-MostRecent", true) => 19045,
        ("Windows10-21H2", false) => 19044,
        ("Windows10-20H2", false) => 19042,
        ("Windows10-1909", _) => 18363,
        ("Windows10-1903", true) => 18362,
        ("Windows10-1607", false) => 14393,
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    #[serde(flatten)]
    pub fact: Fact,
    #[serde(default)]
    pub builds: Builds,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub os: Os,
    pub name: String,
    pub path: Vec<String>,
    pub docs: String,
    pub recommend: Option<String>,
    pub parts: Vec<Part>,
    pub revert_parts: Option<Vec<Part>>,
    pub restart: Vec<String>,
    pub cleanup: bool,
    pub opaque_reason: Option<String>,
    pub has_revert: bool,
    pub script: ScriptData,
}

impl Item {
    pub fn top(&self) -> &str {
        self.path.first().map_or("", String::as_str)
    }

    pub fn category(&self) -> &str {
        self.path.last().map_or("", String::as_str)
    }

    pub fn extreme(&self) -> bool {
        EXTREME_TOPS.contains(&self.top())
    }

    pub fn executable(&self) -> bool {
        self.opaque_reason.is_none()
            && self
                .parts
                .iter()
                .any(|p| !matches!(p.fact, Fact::CommandExists { .. }))
            && self.parts.iter().all(|p| p.fact.executable(self.os))
    }

    pub fn code_only(&self) -> bool {
        !self.executable()
    }

    pub fn admin(&self) -> bool {
        self.parts.iter().any(|p| p.fact.admin())
    }

    pub fn risk(&self) -> Risk {
        if self.extreme() {
            Risk::Extreme
        } else if self.cleanup || self.parts.iter().any(|p| p.fact.destructive()) {
            Risk::Destructive
        } else {
            Risk::Reversible
        }
    }

    pub fn default_parts(&self) -> Option<Vec<Part>> {
        if let Some(r) = &self.revert_parts {
            return (!r.is_empty() && r.iter().all(|p| p.fact.executable(self.os)))
                .then(|| r.clone());
        }
        if !self.has_revert || !self.executable() {
            return None;
        }
        let reverts = self.parts.iter().any(|p| match &p.fact {
            Fact::RegSet { revert, .. } | Fact::RegDelete { revert, .. } => {
                *revert != RegRevert::None
            }
            Fact::Service { .. }
            | Fact::ServiceReg { .. }
            | Fact::Systemd { .. }
            | Fact::FirefoxPref { .. } => true,
            Fact::Task {
                enable_on_revert, ..
            } => *enable_on_revert,
            _ => false,
        });
        reverts.then(|| self.parts.clone())
    }

    pub fn compile(&self, c: &Collection) -> Result<compiler::Compiled, String> {
        Compiler::new(c).compile_script(&self.script)
    }
}

pub struct Catalog {
    pub os: Os,
    pub collection: Collection,
    pub items: Vec<Item>,
    pub left_out: usize,
}

impl Catalog {
    pub fn find(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
}

static CATALOGS: [OnceLock<Result<Catalog, String>>; 3] =
    [OnceLock::new(), OnceLock::new(), OnceLock::new()];

pub fn catalog(os: Os) -> Result<&'static Catalog, String> {
    let (slot, yaml) = match os {
        Os::Windows => (&CATALOGS[0], WINDOWS_YAML),
        Os::Macos => (&CATALOGS[1], MACOS_YAML),
        Os::Linux => (&CATALOGS[2], LINUX_YAML),
    };
    slot.get_or_init(|| build(os, yaml))
        .as_ref()
        .map_err(Clone::clone)
}

pub fn build(os: Os, yaml: &str) -> Result<Catalog, String> {
    let collection = compiler::parse_collection(yaml)?;
    let mut items = Vec::new();
    let mut left_out = 0;
    {
        let comp = Compiler::new(&collection);
        for top in &collection.actions {
            walk_category(os, &comp, top, &mut Vec::new(), &mut items, &mut left_out)?;
        }
    }
    Ok(Catalog {
        os,
        collection,
        items,
        left_out,
    })
}

fn walk_category(
    os: Os,
    comp: &Compiler,
    cat: &CategoryData,
    path: &mut Vec<String>,
    out: &mut Vec<Item>,
    left_out: &mut usize,
) -> Result<(), String> {
    path.push(cat.category.clone());
    for child in &cat.children {
        match child {
            Node::Category(c) => walk_category(os, comp, c, path, out, left_out)?,
            Node::Script(s) => match item(os, comp, s, path)? {
                Some(it) => out.push(it),
                None => *left_out += 1,
            },
        }
    }
    path.pop();
    Ok(())
}

pub fn item(
    os: Os,
    comp: &Compiler,
    s: &ScriptData,
    path: &[String],
) -> Result<Option<Item>, String> {
    if path.iter().any(|p| LEFT_OUT.is_match(p)) || LEFT_OUT.is_match(&s.name) {
        return Ok(None);
    }
    let mut x = Extract::new(os);
    let has_revert;
    let mut revert_parts = None;
    if let Some(calls) = &s.call {
        let calls = compiler::parse_calls(calls).map_err(|e| format!("{}: {e}", s.name))?;
        for c in &calls {
            x.root = c.function.clone();
            comp.walk(std::slice::from_ref(c), &mut |c, inline| x.visit(c, inline))
                .map_err(|e| format!("{}: {e}", s.name))?;
        }
        has_revert = !comp
            .compile_calls(&calls)
            .map_err(|e| format!("{}: {e}", s.name))?
            .revert
            .is_empty();
    } else {
        x.shell(s.code.as_deref().unwrap_or(""), Builds::default());
        has_revert = s
            .revert_code
            .as_deref()
            .is_some_and(|r| !r.trim().is_empty());
        if has_revert {
            let mut r = Extract::new(os);
            r.shell(s.revert_code.as_deref().unwrap_or(""), Builds::default());
            revert_parts = Some(r.parts);
        }
    }
    if x.left_out {
        return Ok(None);
    }
    if let Some(probe) = config_probes(os, &s.name) {
        x.parts = vec![Part {
            fact: probe,
            builds: Builds::default(),
        }];
        x.not_ours("edits a system file with sed");
    }
    let cleanup = x.cleanup || path.first().is_some_and(|t| t == "Privacy cleanup");
    Ok(Some(Item {
        id: format!("privacy:{}:{}", os.as_str(), s.name),
        os,
        name: s.name.clone(),
        path: path.to_vec(),
        docs: s.docs.text(),
        recommend: s.recommend.clone(),
        parts: x.parts,
        revert_parts,
        restart: x.restart,
        cleanup,
        opaque_reason: x.opaque_reason,
        has_revert,
        script: s.clone(),
    }))
}

struct Extract {
    os: Os,
    parts: Vec<Part>,
    restart: Vec<String>,
    cleanup: bool,
    left_out: bool,
    opaque_reason: Option<String>,
    root: String,
}

fn arg<'a>(c: &'a Call, name: &str) -> Option<&'a str> {
    c.args.get(name).map(String::as_str)
}

fn builds_of(c: &Call) -> Builds {
    Builds {
        min: arg(c, "minimumWindowsVersion").and_then(|v| windows_build(v, false)),
        max: arg(c, "maximumWindowsVersion").and_then(|v| windows_build(v, true)),
    }
}

fn truthy(v: Option<&str>) -> bool {
    v.is_some_and(|v| !v.trim().is_empty())
}

impl Extract {
    fn new(os: Os) -> Extract {
        Extract {
            os,
            parts: Vec::new(),
            restart: Vec::new(),
            cleanup: false,
            left_out: false,
            opaque_reason: None,
            root: String::new(),
        }
    }

    fn push(&mut self, fact: Fact, builds: Builds) {
        self.parts.push(Part { fact, builds });
    }

    fn opaque(&mut self, what: &str) {
        let what = if self.root.is_empty() || self.root == what {
            what.to_string()
        } else {
            format!("{} ({what})", self.root)
        };
        self.push(Fact::Opaque { what }, Builds::default());
    }

    fn not_ours(&mut self, why: &str) {
        self.opaque_reason.get_or_insert_with(|| why.to_string());
    }

    fn visit(&mut self, c: &Call, inline: bool) -> Visit {
        let f = c.function.as_str();
        let builds = builds_of(c);
        let a = |n: &str| arg(c, n).unwrap_or("").to_string();
        if LEFT_OUT_FUNCTIONS.contains(&f) {
            self.left_out = true;
            return Visit::Stop;
        }
        if CLEANUP_FUNCTIONS.contains(&f) {
            self.cleanup = true;
            self.opaque(f);
            return Visit::Stop;
        }
        match (self.os, f) {
            (_, "Comment" | "ShowMessage") => {}
            (_, "ShowExplorerRestartSuggestion") => self.restart.push("explorer".into()),
            (_, "ShowComputerRestartSuggestion") => self.restart.push("computer".into()),
            (_, "ShowEdgeRestartSuggestion") => self.restart.push("edge".into()),
            (_, "ShowChromeRestartSuggestion") => self.restart.push("chrome".into()),
            (Os::Windows, "SetRegistryValue") => {
                if truthy(arg(c, "evaluateDataAsPowerShell")) || truthy(arg(c, "setupCode")) {
                    self.not_ours("computes its value with PowerShell");
                }
                if truthy(arg(c, "elevateToTrustedInstaller")) {
                    self.not_ours("needs TrustedInstaller");
                }
                let revert = if truthy(arg(c, "deleteOnRevert")) {
                    RegRevert::Delete
                } else if let Some(d) = arg(c, "dataOnRevert") {
                    RegRevert::Set {
                        kind: a("dataType"),
                        data: d.to_string(),
                    }
                } else {
                    RegRevert::None
                };
                self.push(
                    Fact::RegSet {
                        key: a("keyPath"),
                        name: a("valueName"),
                        kind: a("dataType"),
                        data: a("data"),
                        revert,
                    },
                    builds,
                );
            }
            (Os::Windows, "DeleteRegistryValue") => {
                if truthy(arg(c, "evaluateDataAsPowerShell"))
                    || truthy(arg(c, "matchDataBeforeDelete"))
                    || truthy(arg(c, "grantPermissions"))
                {
                    self.not_ours("deletes the value under conditions only PowerShell checks");
                }
                if truthy(arg(c, "elevateToTrustedInstaller")) {
                    self.not_ours("needs TrustedInstaller");
                }
                let revert = match (arg(c, "dataOnRevert"), arg(c, "dataTypeOnRevert")) {
                    (Some(d), Some(k)) => RegRevert::Set {
                        kind: k.to_string(),
                        data: d.to_string(),
                    },
                    _ => RegRevert::None,
                };
                self.push(
                    Fact::RegDelete {
                        key: a("keyPath"),
                        name: a("valueName"),
                        revert,
                    },
                    builds,
                );
            }
            (Os::Windows, "DeleteRegistryKey" | "SoftDeleteRegistryKey") => {
                if truthy(arg(c, "replaceSid")) {
                    self.opaque(f);
                } else {
                    self.push(Fact::RegKeyGone { key: a("keyPath") }, builds);
                }
            }
            (Os::Windows, "DisableService") => self.push(
                Fact::Service {
                    name: a("serviceName"),
                    default_mode: a("defaultStartupMode").trim().to_string(),
                },
                builds,
            ),
            (Os::Windows, "DisableServiceInRegistry") => {
                if truthy(arg(c, "elevateToTrustedInstaller")) {
                    self.not_ours("needs TrustedInstaller");
                }
                self.push(
                    Fact::ServiceReg {
                        name: a("serviceName"),
                        default_mode: a("defaultStartupMode").trim().to_string(),
                    },
                    builds,
                );
            }
            (Os::Windows, "DisableScheduledTask") => {
                if truthy(arg(c, "grantPermissions")) {
                    self.not_ours("changes the permissions of the task file first");
                }
                self.push(
                    Fact::Task {
                        path: a("taskPathPattern"),
                        name: a("taskNamePattern"),
                        enable_on_revert: !truthy(arg(c, "disableOnRevert")),
                    },
                    builds,
                );
            }
            (Os::Windows, "BlockViaHostsFile") => self.push(
                Fact::HostsBlock {
                    domain: a("domain"),
                },
                builds,
            ),
            (
                Os::Windows,
                "UninstallStoreApp"
                | "UninstallNonRemovableStoreApp"
                | "UninstallNonRemovableStoreAppWithCleanup",
            ) => self.push(
                Fact::StoreApp {
                    package: a("packageName"),
                },
                builds,
            ),
            (Os::Windows, "TerminateRunningProcess") => {}
            (Os::Windows, "BlockExecutablesFromRunningViaShell") => self.push(
                Fact::DisallowRun {
                    exe: a("executableNameWithExtension"),
                },
                builds,
            ),
            (Os::Macos, "ResetServicePermissions") => self.push(
                Fact::TccReset {
                    service: a("serviceId"),
                },
                builds,
            ),
            (Os::Linux, "DisableSystemdService") => {
                self.push(
                    Fact::CommandExists {
                        command: "systemctl".into(),
                    },
                    builds,
                );
                self.push(
                    Fact::Systemd {
                        unit: a("serviceName"),
                    },
                    builds,
                );
            }
            (
                Os::Linux,
                "UninstallAptPackage" | "UninstallPacmanPackage" | "UninstallRPMPackage",
            ) => {
                let (cmd, manager) = match f {
                    "UninstallAptPackage" => ("apt-get", "apt"),
                    "UninstallPacmanPackage" => ("pacman", "pacman"),
                    _ => ("rpm", "rpm"),
                };
                self.push(
                    Fact::CommandExists {
                        command: cmd.into(),
                    },
                    builds,
                );
                self.push(
                    Fact::Package {
                        manager: manager.into(),
                        name: a("packageName"),
                    },
                    builds,
                );
            }
            (Os::Linux, "AddFirefoxPrefs") => self.push(
                Fact::FirefoxPref {
                    name: a("prefName"),
                    value: a("jsonValue"),
                },
                builds,
            ),
            (Os::Linux, "SetVsCodeSetting") => self.push(
                Fact::VsCodeSetting {
                    name: a("setting"),
                    json: a("jsonValue"),
                },
                builds,
            ),
            (_, "RunIfCommandExists") => {
                self.push(
                    Fact::CommandExists {
                        command: a("command"),
                    },
                    builds,
                );
                self.shell(&a("code"), builds);
            }
            (_, "RunInlineCode") => self.shell(&a("code"), builds),
            _ if inline || f.starts_with("Run") => self.opaque(f),
            _ => return Visit::Descend,
        }
        Visit::Stop
    }

    fn shell(&mut self, code: &str, builds: Builds) {
        let mut any = false;
        for line in code.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            any = true;
            match shell_fact(t) {
                Some(ShellLine::Fact(f)) => self.push(f, builds),
                Some(ShellLine::Restart(p)) => self.restart.push(p),
                None => {
                    if t.starts_with("rm ") || t.starts_with("sudo rm ") {
                        self.cleanup = true;
                    }
                    self.opaque("shell");
                }
            }
        }
        if !any {
            self.opaque("shell");
        }
    }
}

enum ShellLine {
    Fact(Fact),
    Restart(String),
}

pub fn shell_words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        x => cur.push(x),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => cur.push(chars.next()?),
                        '$' | '`' => return None,
                        x => cur.push(x),
                    }
                }
            }
            '\\' => {
                in_word = true;
                cur.push(chars.next()?);
            }
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            ';' | '|' | '&' | '>' | '<' | '*' | '?' | '(' | ')' | '`' | '{' | '}' => return None,
            x => {
                in_word = true;
                cur.push(x);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Some(words)
}

fn shell_fact(line: &str) -> Option<ShellLine> {
    let words = shell_words(line)?;
    let (sudo, rest) = match words.first().map(String::as_str) {
        Some("sudo") => (true, &words[1..]),
        _ => (false, &words[..]),
    };
    let w: Vec<&str> = rest.iter().map(String::as_str).collect();
    match w.as_slice() {
        ["killall", proc] | ["killall", "-HUP", proc] => {
            Some(ShellLine::Restart((*proc).to_string()))
        }
        ["defaults", "write", domain, key, flag, value]
            if flag.starts_with('-') && simple_defaults_type(flag) =>
        {
            Some(ShellLine::Fact(Fact::Defaults {
                domain: (*domain).to_string(),
                key: (*key).to_string(),
                kind: Some(flag.trim_start_matches('-').to_ascii_lowercase()),
                value: (*value).to_string(),
                sudo,
            }))
        }
        ["defaults", "write", domain, key, value]
            if !value.starts_with('-') && !value.starts_with('(') && !value.starts_with('{') =>
        {
            Some(ShellLine::Fact(Fact::Defaults {
                domain: (*domain).to_string(),
                key: (*key).to_string(),
                kind: None,
                value: (*value).to_string(),
                sudo,
            }))
        }
        ["defaults", "delete", domain, key] => Some(ShellLine::Fact(Fact::DefaultsDelete {
            domain: (*domain).to_string(),
            key: (*key).to_string(),
            sudo,
        })),
        _ => None,
    }
    .filter(|l| match l {
        ShellLine::Fact(Fact::Defaults { domain, key, .. })
        | ShellLine::Fact(Fact::DefaultsDelete { domain, key, .. }) => {
            !key.contains('$')
                && domain
                    .replace("$USER", "")
                    .replace("$HOME", "")
                    .chars()
                    .all(|c| c != '$')
        }
        _ => true,
    })
}

fn simple_defaults_type(flag: &str) -> bool {
    matches!(
        flag.to_ascii_lowercase().as_str(),
        "-bool" | "-boolean" | "-int" | "-integer" | "-float" | "-string"
    )
}

pub fn config_probes(os: Os, name: &str) -> Option<Fact> {
    if os != Os::Linux {
        return None;
    }
    let (path, on, off) = match name {
        "Disable participation in Popularity Contest" => (
            "/etc/popularity-contest.conf",
            "PARTICIPATE=no",
            "PARTICIPATE=yes",
        ),
        "Disable participation in Apport error messaging system" => {
            ("/etc/default/apport", "enabled=0", "enabled=1")
        }
        "Disable crash report submissions" => (
            "/etc/default/whoopsie",
            "report_crashes=false",
            "report_crashes=true",
        ),
        _ => return None,
    };
    Some(Fact::ConfigLine {
        path: path.into(),
        on: on.into(),
        off: off.into(),
    })
}

pub fn coverage(c: &Catalog) -> BTreeMap<&'static str, usize> {
    let mut m = BTreeMap::new();
    m.insert("items", c.items.len());
    m.insert("left_out", c.left_out);
    m.insert(
        "executable",
        c.items.iter().filter(|i| i.executable()).count(),
    );
    m.insert(
        "code_only",
        c.items.iter().filter(|i| i.code_only()).count(),
    );
    m.insert(
        "state_readable",
        c.items.iter().filter(|i| super::state::readable(i)).count(),
    );
    m.insert("extreme", c.items.iter().filter(|i| i.extreme()).count());
    m.insert("cleanup", c.items.iter().filter(|i| i.cleanup).count());
    m
}

pub fn blockers(c: &Catalog) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for it in c
        .items
        .iter()
        .filter(|i| !i.cleanup && !super::state::readable(i))
    {
        for p in &it.parts {
            let key = match &p.fact {
                Fact::Opaque { what } => what.clone(),
                Fact::TccReset { .. } => "TccReset".into(),
                _ => continue,
            };
            *m.entry(key).or_default() += 1;
        }
    }
    m
}
