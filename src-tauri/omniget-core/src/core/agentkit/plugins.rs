//! Plugin and extension inventory of the coding tools (plan F10): what each tool
//! has installed, whether it is on, where it came from (marketplace) and what it
//! brings (skills, agents, commands, hooks, MCP/LSP servers …), plus the switch
//! that turns one on or off in the tool's own file.
//!
//! | tool     | installed                                              | on/off                                                     |
//! |----------|--------------------------------------------------------|------------------------------------------------------------|
//! | Claude   | `~/.claude/plugins/installed_plugins.json` (+ cache)   | `enabledPlugins` in settings.json (user/project/local)     |
//! | Copilot  | `~/.copilot/installed-plugins/<mkt>/<plugin>/`         | `enabledPlugins` in `~/.copilot/settings.json`, `.github/copilot/settings*.json` |
//! | Droid    | `~/.factory/plugins/**/.factory-plugin/plugin.json`    | `enabledPlugins` in `~/.factory/settings.json`, `.factory/settings*.json` |
//! | Cursor   | `~/.cursor/plugins/{cache/<mkt>/<plugin>/<sha>,local/<plugin>}` | kept in Cursor's app state (read only here)       |
//! | Gemini   | `~/.gemini/extensions/<name>/gemini-extension.json`    | `~/.gemini/extensions/extension-enablement.json` path rules |
//! | Qwen     | `~/.qwen/extensions/<name>/qwen-extension.json`        | same rules file (imported by Qwen's extension store)       |
//! | OpenCode | `plugin` in opencode.json(c) + `.opencode/plugins/*.ts` | npm entry removed/restored; file renamed `*.disabled`     |
//!
//! Only plugin and settings files are read (never credential files). Every
//! switch goes through a backup transaction, so `writer::restore` undoes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::edit::{self, DocFormat, Seg};
use super::writer::{self, Tx};
use super::{AgentkitError, Env, Result};

/// Tools this module knows.
pub const PLUGIN_TOOLS: [&str; 7] = [
    "claude", "copilot", "droid", "cursor", "gemini", "qwen", "opencode",
];

/// What a plugin brings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginComponents {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    /// Hook event names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lsp_servers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
    /// Anything else present (output styles, monitors, themes, policies, code …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other: Vec<String>,
}

impl PluginComponents {
    pub fn is_empty(&self) -> bool {
        *self == PluginComponents::default()
    }
}

/// One plugin/extension of one tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub tool: String,
    /// Key the tool uses (`name@marketplace`, extension name, `npm:<spec>`, `file:<name>`).
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<String>,
    /// Where the marketplace or the plugin comes from (`github:owner/repo`, URL, path, npm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Scope it is installed at: `user | project | local`.
    pub scope: String,
    /// Files are on disk (a plugin can be switched on in settings but not installed).
    pub installed: bool,
    /// Effective state for the project asked about (or the user); `None` = the
    /// tool keeps it where we cannot read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Explicit value per scope where one is set (`user`, `project`, `local`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub enabled_in: BTreeMap<String, bool>,
    /// Scopes [`set_enabled`] can write for this plugin.
    #[serde(default)]
    pub toggle_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub components: PluginComponents,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl PluginInfo {
    fn new(tool: &str, id: &str, name: &str, scope: &str) -> PluginInfo {
        PluginInfo {
            tool: tool.to_string(),
            id: id.to_string(),
            name: name.to_string(),
            version: None,
            description: String::new(),
            marketplace: None,
            source: None,
            scope: scope.to_string(),
            installed: true,
            enabled: None,
            enabled_in: BTreeMap::new(),
            toggle_scopes: vec![],
            path: None,
            components: PluginComponents::default(),
            notes: vec![],
        }
    }
}

/// Result of [`set_enabled`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToggleReport {
    pub tx: String,
    pub file: String,
    pub plugin: Option<PluginInfo>,
}

// ------------------------------------------------------------------ small readers

fn read_json(path: &Path) -> Option<Value> {
    // the JSONC parser takes plain JSON too and tolerates comments
    let text = std::fs::read_to_string(path).ok()?;
    edit::parse_value(DocFormat::Jsonc, &text).ok()
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn push_unique(v: &mut Vec<String>, s: impl Into<String>) {
    let s = s.into();
    if !s.is_empty() && !v.contains(&s) {
        v.push(s);
    }
}

fn sorted_dir(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn file_stem(p: &Path) -> String {
    let n = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    for ext in [
        ".agent.md",
        ".chatmode.md",
        ".prompt.md",
        ".md",
        ".toml",
        ".mdc",
        ".yaml",
        ".yml",
        ".json",
    ] {
        if let Some(s) = n.strip_suffix(ext) {
            return s.to_string();
        }
    }
    n
}

/// `enabledPlugins` of one settings file.
fn enabled_map(path: &Path) -> BTreeMap<String, bool> {
    read_json(path)
        .and_then(|v| v.get("enabledPlugins").cloned())
        .and_then(|v| v.as_object().cloned())
        .map(|m| {
            m.into_iter()
                .filter_map(|(k, v)| v.as_bool().map(|b| (k, b)))
                .collect()
        })
        .unwrap_or_default()
}

/// Marketplace source as one short string.
fn source_text(src: &Value) -> Option<String> {
    if let Some(s) = src.as_str() {
        return Some(s.to_string());
    }
    let kind = str_of(src, "source").unwrap_or_default();
    match kind.as_str() {
        "github" => str_of(src, "repo").map(|r| format!("github:{r}")),
        "npm" => str_of(src, "package").map(|p| format!("npm:{p}")),
        "directory" | "local" => str_of(src, "path"),
        _ => str_of(src, "url")
            .or_else(|| str_of(src, "repo"))
            .or_else(|| str_of(src, "path")),
    }
}

// ------------------------------------------------------------------ components

const MANIFESTS: [&str; 9] = [
    ".claude-plugin/plugin.json",
    ".cursor-plugin/plugin.json",
    ".factory-plugin/plugin.json",
    ".devin-plugin/plugin.json",
    ".github/plugin/plugin.json",
    ".plugin/plugin.json",
    "plugin.json",
    "gemini-extension.json",
    "qwen-extension.json",
];

/// The plugin's manifest (first one found) as a value.
pub fn manifest(root: &Path) -> Value {
    for m in MANIFESTS {
        let p = m.split('/').fold(root.to_path_buf(), |p, s| p.join(s));
        if let Some(v) = read_json(&p) {
            return v;
        }
    }
    Value::Object(Map::new())
}

/// Folders a manifest key points at (string or list), else the defaults.
fn dirs_for(root: &Path, man: &Value, key: &str, defaults: &[&str]) -> Vec<PathBuf> {
    let from = |s: &str| -> PathBuf {
        s.trim_start_matches("./")
            .split('/')
            .filter(|x| !x.is_empty())
            .fold(root.to_path_buf(), |p, x| p.join(x))
    };
    match man.get(key) {
        Some(Value::String(s)) => vec![from(s)],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).map(from).collect(),
        _ => defaults.iter().map(|d| from(d)).collect(),
    }
}

/// Keys of a server map, whether the file wraps it in `mcpServers`/`servers`
/// or not.
fn server_names(v: &Value) -> Vec<String> {
    let map = v
        .get("mcpServers")
        .or_else(|| v.get("servers"))
        .or_else(|| v.get("lspServers"))
        .unwrap_or(v);
    map.as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

fn hook_events(v: &Value) -> Vec<String> {
    let map = v.get("hooks").unwrap_or(v);
    map.as_object()
        .map(|m| {
            m.keys()
                .filter(|k| *k != "version" && *k != "description")
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Scans a plugin folder for what it brings.
pub fn scan_components(root: &Path, man: &Value) -> PluginComponents {
    let mut c = PluginComponents::default();
    for d in dirs_for(root, man, "skills", &["skills"]) {
        for s in sorted_dir(&d) {
            if s.join("SKILL.md").is_file() {
                push_unique(
                    &mut c.skills,
                    s.file_name().unwrap_or_default().to_string_lossy(),
                );
            }
        }
    }
    for d in dirs_for(root, man, "agents", &["agents", "droids"]) {
        for f in sorted_dir(&d) {
            if f.is_file() && f.extension().is_some_and(|e| e == "md") {
                push_unique(&mut c.agents, file_stem(&f));
            }
        }
    }
    for d in dirs_for(root, man, "commands", &["commands"]) {
        for e in walkdir::WalkDir::new(&d).max_depth(3).into_iter().flatten() {
            let p = e.path();
            if !e.file_type().is_file() || !p.extension().is_some_and(|x| x == "md" || x == "toml")
            {
                continue;
            }
            let rel = p.strip_prefix(&d).unwrap_or(p);
            let mut parts: Vec<String> = rel
                .parent()
                .map(|x| {
                    x.components()
                        .map(|c| c.as_os_str().to_string_lossy().to_string())
                        .collect()
                })
                .unwrap_or_default();
            parts.push(file_stem(p));
            push_unique(&mut c.commands, parts.join(":"));
        }
    }
    // hooks: manifest (inline or path) or hooks/hooks.json
    let hooks_val = match man.get("hooks") {
        Some(Value::Object(_)) => man.get("hooks").cloned(),
        Some(Value::String(s)) => read_json(&dirs_for(root, &Value::Null, "", &[s.as_str()])[0]),
        _ => [
            "hooks/hooks.json",
            "hooks.json",
            "com.github.copilot/hooks/hooks.json",
        ]
        .iter()
        .find_map(|r| read_json(&r.split('/').fold(root.to_path_buf(), |p, s| p.join(s)))),
    };
    if let Some(h) = hooks_val {
        for e in hook_events(&h) {
            push_unique(&mut c.hooks, e);
        }
    }
    // MCP: manifest map or path, else .mcp.json / mcp.json
    let mcp_val = match man.get("mcpServers") {
        Some(Value::Object(m)) => Some(Value::Object(m.clone())),
        Some(Value::String(s)) => read_json(&dirs_for(root, &Value::Null, "", &[s.as_str()])[0]),
        _ => [".mcp.json", "mcp.json"]
            .iter()
            .find_map(|r| read_json(&root.join(r))),
    };
    if let Some(m) = mcp_val {
        for n in server_names(&m) {
            push_unique(&mut c.mcp_servers, n);
        }
    }
    let lsp_val = match man.get("lspServers") {
        Some(Value::Object(m)) => Some(Value::Object(m.clone())),
        Some(Value::String(s)) => read_json(&dirs_for(root, &Value::Null, "", &[s.as_str()])[0]),
        _ => read_json(&root.join(".lsp.json")),
    };
    if let Some(l) = lsp_val {
        for n in server_names(&l) {
            push_unique(&mut c.lsp_servers, n);
        }
    }
    for d in dirs_for(root, man, "rules", &["rules"]) {
        for f in sorted_dir(&d) {
            if f.is_file() {
                push_unique(&mut c.rules, file_stem(&f));
            }
        }
    }
    if let Some(ctx) = str_of(man, "contextFileName") {
        push_unique(&mut c.rules, ctx);
    } else if root.join("GEMINI.md").is_file() && root.join("gemini-extension.json").is_file() {
        push_unique(&mut c.rules, "GEMINI.md");
    }
    for (dir, label) in [
        ("output-styles", "output styles"),
        ("monitors", "monitors"),
        ("themes", "themes"),
        ("policies", "policies"),
        ("bin", "executables"),
        ("automations", "automations"),
        ("workflows", "workflows"),
    ] {
        if root.join(dir).is_dir() {
            push_unique(&mut c.other, label);
        }
    }
    if man.get("themes").is_some() {
        push_unique(&mut c.other, "themes");
    }
    c
}

fn fill_from_manifest(p: &mut PluginInfo, root: &Path) {
    let man = manifest(root);
    if let Some(n) = str_of(&man, "displayName").or_else(|| str_of(&man, "name")) {
        if p.name.is_empty() {
            p.name = n;
        }
    }
    if p.version.is_none() {
        p.version = str_of(&man, "version");
    }
    if p.description.is_empty() {
        p.description = str_of(&man, "description").unwrap_or_default();
    }
    p.components = scan_components(root, &man);
    p.path = Some(root.display().to_string());
}

// ------------------------------------------------------------------ settings-driven tools

/// Settings files with `enabledPlugins` per scope, for Claude, Copilot and Droid.
fn settings_files(env: &Env, tool: &str, project: Option<&Path>) -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    let (user, proj, local): (PathBuf, Option<PathBuf>, Option<PathBuf>) = match tool {
        "claude" => (
            env.home.join(".claude").join("settings.json"),
            project.map(|p| p.join(".claude").join("settings.json")),
            project.map(|p| p.join(".claude").join("settings.local.json")),
        ),
        "copilot" => (
            env.home.join(".copilot").join("settings.json"),
            project.map(|p| p.join(".github").join("copilot").join("settings.json")),
            project.map(|p| {
                p.join(".github")
                    .join("copilot")
                    .join("settings.local.json")
            }),
        ),
        "droid" => (
            env.home.join(".factory").join("settings.json"),
            project.map(|p| p.join(".factory").join("settings.json")),
            project.map(|p| p.join(".factory").join("settings.local.json")),
        ),
        _ => return v,
    };
    v.push(("user".to_string(), user));
    if let Some(p) = proj {
        v.push(("project".to_string(), p));
    }
    if let Some(p) = local {
        v.push(("local".to_string(), p));
    }
    v
}

/// Applies `enabledPlugins` of every scope (later scopes win) to the list, adding
/// plugins that are switched on but not installed.
fn apply_enabled(
    env: &Env,
    tool: &str,
    project: Option<&Path>,
    out: &mut Vec<PluginInfo>,
    default_on: bool,
) {
    let files = settings_files(env, tool, project);
    let scopes: Vec<String> = files.iter().map(|(s, _)| s.clone()).collect();
    for (scope, f) in &files {
        for (id, on) in enabled_map(f) {
            if !out.iter().any(|p| p.id == id) {
                let (name, mkt) = id.split_once('@').unwrap_or((id.as_str(), ""));
                let mut p = PluginInfo::new(tool, &id, name, scope);
                p.installed = false;
                p.marketplace = (!mkt.is_empty()).then(|| mkt.to_string());
                p.notes
                    .push("switched on in settings but not installed".into());
                out.push(p);
            }
            if let Some(p) = out.iter_mut().find(|p| p.id == id) {
                p.enabled_in.insert(scope.clone(), on);
            }
        }
    }
    for p in out.iter_mut() {
        let mut on = default_on && p.installed;
        for s in ["user", "project", "local"] {
            if let Some(b) = p.enabled_in.get(s) {
                on = *b;
            }
        }
        p.enabled = Some(on);
        p.toggle_scopes = scopes.clone();
    }
}

fn claude(env: &Env, project: Option<&Path>) -> Vec<PluginInfo> {
    let base = env.home.join(".claude").join("plugins");
    let known = read_json(&base.join("known_marketplaces.json")).unwrap_or(Value::Null);
    let mut out: Vec<PluginInfo> = Vec::new();
    let installed = read_json(&base.join("installed_plugins.json")).unwrap_or(Value::Null);
    if let Some(map) = installed.get("plugins").and_then(|v| v.as_object()) {
        for (id, entries) in map {
            // v2: a list of installs; v1: one object
            let list: Vec<Value> = match entries {
                Value::Array(a) => a.clone(),
                v @ Value::Object(_) => vec![v.clone()],
                _ => vec![],
            };
            for e in list {
                let scope = str_of(&e, "scope").unwrap_or_else(|| "user".into());
                if let (Some(pp), Some(proj)) = (str_of(&e, "projectPath"), project) {
                    if scope != "user" && Path::new(&pp) != proj {
                        continue;
                    }
                }
                let (name, mkt) = id.split_once('@').unwrap_or((id.as_str(), ""));
                let mut p = PluginInfo::new("claude", id, name, &scope);
                p.version = str_of(&e, "version");
                if !mkt.is_empty() {
                    p.marketplace = Some(mkt.to_string());
                    p.source = known
                        .get(mkt)
                        .and_then(|m| m.get("source"))
                        .and_then(source_text);
                }
                match str_of(&e, "installPath").map(PathBuf::from) {
                    Some(root) if root.is_dir() => fill_from_manifest(&mut p, &root),
                    Some(root) => {
                        p.installed = false;
                        p.path = Some(root.display().to_string());
                        p.notes.push("install folder is missing".into());
                    }
                    None => p.installed = false,
                }
                if !out.iter().any(|x| x.id == p.id) {
                    out.push(p);
                }
            }
        }
    }
    apply_enabled(env, "claude", project, &mut out, false);
    let mut catalogs: BTreeMap<String, Value> = BTreeMap::new();
    for p in out.iter_mut() {
        let Some(mkt) = p.marketplace.clone() else {
            continue;
        };
        if p.source.is_none() {
            p.source = known
                .get(&mkt)
                .and_then(|m| m.get("source"))
                .and_then(source_text);
        }
        // a non-strict marketplace entry carries parts of the plugin itself
        let cat = catalogs.entry(mkt.clone()).or_insert_with(|| {
            let dir = known
                .get(&mkt)
                .and_then(|m| str_of(m, "installLocation"))
                .map(PathBuf::from)
                .unwrap_or_else(|| base.join("marketplaces").join(&mkt));
            read_json(&dir.join(".claude-plugin").join("marketplace.json")).unwrap_or(Value::Null)
        });
        let entry = cat.get("plugins").and_then(|v| v.as_array()).and_then(|a| {
            a.iter()
                .find(|e| str_of(e, "name").as_deref() == Some(p.name.as_str()))
        });
        if let Some(e) = entry {
            merge_market_entry(p, e);
        }
    }
    out
}

/// Description, version and inline servers/hooks of a marketplace entry.
fn merge_market_entry(p: &mut PluginInfo, e: &Value) {
    if p.description.is_empty() {
        p.description = str_of(e, "description").unwrap_or_default();
    }
    if p.version.is_none() {
        p.version = str_of(e, "version");
    }
    if let Some(Value::Object(m)) = e.get("mcpServers") {
        for k in m.keys() {
            push_unique(&mut p.components.mcp_servers, k.clone());
        }
    }
    if let Some(Value::Object(m)) = e.get("lspServers") {
        for k in m.keys() {
            push_unique(&mut p.components.lsp_servers, k.clone());
        }
    }
    if let Some(h @ Value::Object(_)) = e.get("hooks") {
        for ev in hook_events(h) {
            push_unique(&mut p.components.hooks, ev);
        }
    }
}

/// Plugin folders under `root` that carry a manifest, skipping marketplace clones.
fn plugin_dirs(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut it = walkdir::WalkDir::new(root).max_depth(max_depth).into_iter();
    while let Some(Ok(e)) = it.next() {
        if !e.file_type().is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name == "marketplaces" || name == "node_modules" || name == ".git" {
            it.skip_current_dir();
            continue;
        }
        let p = e.path();
        if p != root
            && MANIFESTS.iter().any(|m| {
                m.split('/')
                    .fold(p.to_path_buf(), |x, s| x.join(s))
                    .is_file()
            })
        {
            out.push(p.to_path_buf());
            it.skip_current_dir();
        }
    }
    out
}

fn copilot(env: &Env, project: Option<&Path>) -> Vec<PluginInfo> {
    let root = env.home.join(".copilot").join("installed-plugins");
    let mut out: Vec<PluginInfo> = Vec::new();
    for mkt_dir in sorted_dir(&root) {
        if !mkt_dir.is_dir() {
            continue;
        }
        let mkt = mkt_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        for pdir in sorted_dir(&mkt_dir) {
            if !pdir.is_dir() {
                continue;
            }
            let dir_name = pdir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let mut p = PluginInfo::new("copilot", "", "", "user");
            fill_from_manifest(&mut p, &pdir);
            let name = str_of(&manifest(&pdir), "name").unwrap_or(dir_name);
            if p.name.is_empty() {
                p.name = name.clone();
            }
            if mkt == "_direct" {
                p.id = name;
                p.source = Some("direct install".into());
            } else {
                p.id = format!("{name}@{mkt}");
                p.marketplace = Some(mkt.clone());
            }
            out.push(p);
        }
    }
    apply_enabled(env, "copilot", project, &mut out, true);
    out
}

fn droid(env: &Env, project: Option<&Path>) -> Vec<PluginInfo> {
    let root = env.home.join(".factory").join("plugins");
    let mut out: Vec<PluginInfo> = Vec::new();
    for pdir in plugin_dirs(&root, 5) {
        let rel: Vec<String> = pdir
            .strip_prefix(&root)
            .unwrap_or(&pdir)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        // cache/<marketplace>/<plugin>/<version> or <marketplace>/<plugin>
        let (mkt, dir_name) = match rel.as_slice() {
            [c, m, p, ..] if c == "cache" => (Some(m.clone()), p.clone()),
            [m, p, ..] => (Some(m.clone()), p.clone()),
            [p] => (None, p.clone()),
            _ => continue,
        };
        let name = str_of(&manifest(&pdir), "name").unwrap_or(dir_name);
        let id = match &mkt {
            Some(m) => format!("{name}@{m}"),
            None => name.clone(),
        };
        if out.iter().any(|x| x.id == id) {
            continue;
        }
        let mut p = PluginInfo::new("droid", &id, &name, "user");
        p.marketplace = mkt;
        fill_from_manifest(&mut p, &pdir);
        out.push(p);
    }
    apply_enabled(env, "droid", project, &mut out, false);
    out
}

fn newest(dirs: Vec<PathBuf>) -> Option<PathBuf> {
    dirs.into_iter()
        .filter(|d| d.is_dir())
        .max_by_key(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok())
}

fn cursor(env: &Env) -> Vec<PluginInfo> {
    let root = env.home.join(".cursor").join("plugins");
    let mut out = Vec::new();
    let note = "Cursor keeps plugins on/off in its own app state: switch it in Cursor".to_string();
    for mkt_dir in sorted_dir(&root.join("cache")) {
        let mkt = mkt_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        for pdir in sorted_dir(&mkt_dir) {
            let Some(ver) = newest(sorted_dir(&pdir)) else {
                continue;
            };
            let name = pdir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let mut p = PluginInfo::new("cursor", &format!("{name}@{mkt}"), "", "user");
            p.marketplace = Some(mkt.clone());
            fill_from_manifest(&mut p, &ver);
            if p.name.is_empty() {
                p.name = name;
            }
            p.notes.push(note.clone());
            out.push(p);
        }
    }
    for pdir in sorted_dir(&root.join("local")) {
        if !pdir.is_dir() {
            continue;
        }
        let name = pdir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut p = PluginInfo::new("cursor", &format!("{name}@local"), "", "user");
        p.source = Some("local".into());
        fill_from_manifest(&mut p, &pdir);
        if p.name.is_empty() {
            p.name = name;
        }
        p.notes.push(note.clone());
        out.push(p);
    }
    out
}

// ------------------------------------------------------------------ Gemini / Qwen extensions

/// `/a/b/` form Gemini uses for paths in its rules.
fn slash_path(p: &Path) -> String {
    let mut s = p.display().to_string().replace('\\', "/");
    if !s.starts_with('/') {
        s.insert(0, '/');
    }
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

/// One rule of `extension-enablement.json` (`[!]<path>[*]`).
#[derive(Debug, Clone, PartialEq)]
struct Rule {
    base: String,
    disable: bool,
    subdirs: bool,
}

impl Rule {
    fn parse(s: &str) -> Rule {
        let disable = s.starts_with('!');
        let mut base = if disable { &s[1..] } else { s }.to_string();
        let subdirs = base.ends_with('*');
        if subdirs {
            base.pop();
        }
        Rule {
            base,
            disable,
            subdirs,
        }
    }

    fn output(&self) -> String {
        format!(
            "{}{}{}",
            if self.disable { "!" } else { "" },
            self.base,
            if self.subdirs { "*" } else { "" }
        )
    }

    /// Gemini's glob: `<base>*` matches the folder and everything below it.
    fn matches(&self, path: &str) -> bool {
        if self.subdirs {
            let b = self.base.trim_end_matches('/');
            path == self.base || path == b || path.starts_with(&format!("{b}/"))
        } else {
            path == self.base
        }
    }

    fn is_child_of(&self, parent: &Rule) -> bool {
        parent.subdirs && parent.matches(&self.base)
    }
}

/// Gemini's `isEnabled`: the last matching rule wins, default on.
pub fn extension_enabled(rules: &[String], path: &Path) -> bool {
    let p = slash_path(path);
    let mut on = true;
    for r in rules {
        let r = Rule::parse(r);
        if r.matches(&p) {
            on = !r.disable;
        }
    }
    on
}

/// Gemini's `enable`/`disable` (with subfolders) for `scope_path`.
pub fn set_extension_rule(rules: &[String], scope_path: &Path, enabled: bool) -> Vec<String> {
    let new = Rule {
        base: slash_path(scope_path),
        disable: !enabled,
        subdirs: true,
    };
    let mut out: Vec<String> = rules
        .iter()
        .filter(|r| {
            let r = Rule::parse(r);
            r.base != new.base && !r.is_child_of(&new)
        })
        .cloned()
        .collect();
    out.push(new.output());
    out
}

fn extensions(env: &Env, tool: &str, project: Option<&Path>) -> Vec<PluginInfo> {
    let (dir, manifest_name, meta_name) = match tool {
        "gemini" => (
            env.home.join(".gemini").join("extensions"),
            "gemini-extension.json",
            ".gemini-extension-install.json",
        ),
        _ => (
            env.home.join(".qwen").join("extensions"),
            "qwen-extension.json",
            ".qwen-extension-install.json",
        ),
    };
    let rules_file = read_json(&dir.join("extension-enablement.json")).unwrap_or(Value::Null);
    let at = project
        .map(Path::to_path_buf)
        .unwrap_or_else(|| env.home.clone());
    let mut out = Vec::new();
    for edir in sorted_dir(&dir) {
        if !edir.join(manifest_name).is_file() {
            continue;
        }
        let man = read_json(&edir.join(manifest_name)).unwrap_or(Value::Null);
        let name = str_of(&man, "name").unwrap_or_else(|| {
            edir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
        let mut p = PluginInfo::new(tool, &name, &name, "user");
        fill_from_manifest(&mut p, &edir);
        if let Some(meta) = read_json(&edir.join(meta_name)) {
            p.source = str_of(&meta, "source");
            if let Some(t) = str_of(&meta, "type") {
                p.notes.push(format!("installed from {t}"));
            }
        }
        let rules: Vec<String> = rules_file
            .get(&name)
            .and_then(|v| v.get("overrides"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        p.enabled = Some(extension_enabled(&rules, &at));
        p.enabled_in
            .insert("user".into(), extension_enabled(&rules, &env.home));
        if let Some(pr) = project {
            p.enabled_in
                .insert("project".into(), extension_enabled(&rules, pr));
        }
        p.toggle_scopes = if project.is_some() {
            vec!["user".into(), "project".into()]
        } else {
            vec!["user".into()]
        };
        out.push(p);
    }
    out
}

// ------------------------------------------------------------------ OpenCode

fn opencode_configs(env: &Env, project: Option<&Path>) -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    let g = env.xdg_config.join("opencode");
    let gfile = ["opencode.jsonc", "opencode.json"]
        .iter()
        .map(|n| g.join(n))
        .find(|p| p.is_file())
        .unwrap_or_else(|| g.join("opencode.json"));
    v.push(("user".to_string(), gfile));
    if let Some(p) = project {
        let pfile = ["opencode.jsonc", "opencode.json"]
            .iter()
            .map(|n| p.join(n))
            .find(|x| x.is_file())
            .unwrap_or_else(|| p.join("opencode.json"));
        v.push(("project".to_string(), pfile));
    }
    v
}

fn opencode_dirs(env: &Env, project: Option<&Path>) -> Vec<(String, PathBuf)> {
    let g = env.xdg_config.join("opencode");
    let mut v = vec![
        ("user".to_string(), g.join("plugins")),
        ("user".to_string(), g.join("plugin")),
    ];
    if let Some(p) = project {
        v.push(("project".to_string(), p.join(".opencode").join("plugins")));
        v.push(("project".to_string(), p.join(".opencode").join("plugin")));
    }
    v
}

const CODE_EXT: [&str; 5] = [".ts", ".js", ".mjs", ".mts", ".cjs"];

/// OmniGet's memory of npm plugin entries it took out of a config (to put back).
fn disabled_store(env: &Env) -> PathBuf {
    env.agentkit_dir().join("plugins-disabled.json")
}

fn npm_spec(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.first().and_then(|x| x.as_str()).map(str::to_string),
        _ => None,
    }
}

fn opencode(env: &Env, project: Option<&Path>) -> Vec<PluginInfo> {
    let mut out = Vec::new();
    let store = read_json(&disabled_store(env)).unwrap_or(Value::Null);
    for (scope, cfg) in opencode_configs(env, project) {
        let key = cfg.display().to_string();
        let active: Vec<Value> = read_json(&cfg)
            .and_then(|v| v.get("plugin").cloned())
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let parked: Vec<Value> = store
            .get("opencode")
            .and_then(|m| m.get(&key))
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        for (entry, on) in active
            .iter()
            .map(|e| (e, true))
            .chain(parked.iter().map(|e| (e, false)))
        {
            let Some(spec) = npm_spec(entry) else {
                continue;
            };
            let local = spec.starts_with('.') || spec.starts_with('/');
            let name = if local {
                spec.clone()
            } else {
                // `@scope/pkg@1.2` → `@scope/pkg`
                let at = spec[1..].find('@').map(|i| i + 1);
                at.map(|i| spec[..i].to_string())
                    .unwrap_or_else(|| spec.clone())
            };
            let id = format!("npm:{spec}");
            if out
                .iter()
                .any(|p: &PluginInfo| p.id == id && p.scope == scope)
            {
                continue;
            }
            let mut p = PluginInfo::new("opencode", &id, &name, &scope);
            p.source = Some(if local {
                spec.clone()
            } else {
                format!("npm:{spec}")
            });
            p.version = (!local && spec != name).then(|| spec[name.len() + 1..].to_string());
            p.path = Some(key.clone());
            p.enabled = Some(on);
            p.enabled_in.insert(scope.clone(), on);
            p.toggle_scopes = vec![scope.clone()];
            p.components
                .other
                .push("code plugin (events, tools)".into());
            out.push(p);
        }
    }
    for (scope, dir) in opencode_dirs(env, project) {
        for f in sorted_dir(&dir) {
            if !f.is_file() {
                continue;
            }
            let fname = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let (base, on) = match fname.strip_suffix(".disabled") {
                Some(b) => (b.to_string(), false),
                None => (fname.clone(), true),
            };
            if !CODE_EXT.iter().any(|e| base.ends_with(e)) {
                continue;
            }
            let mut p = PluginInfo::new("opencode", &format!("file:{base}"), &base, &scope);
            p.source = Some("local file".into());
            p.path = Some(dir.join(&base).display().to_string());
            p.enabled = Some(on);
            p.enabled_in.insert(scope.clone(), on);
            p.toggle_scopes = vec![scope.clone()];
            p.components
                .other
                .push("code plugin (events, tools)".into());
            if base.starts_with("omniget-") {
                p.notes.push("written by OmniGet's hook converter".into());
            }
            out.push(p);
        }
    }
    out
}

// ------------------------------------------------------------------ public API

/// Every plugin/extension of every tool this module knows (tools with nothing
/// installed contribute nothing).
pub fn list(env: &Env, project: Option<&Path>) -> Vec<PluginInfo> {
    let mut out = Vec::new();
    for t in PLUGIN_TOOLS {
        out.extend(list_tool(env, t, project));
    }
    out
}

/// Plugins of one tool.
pub fn list_tool(env: &Env, tool: &str, project: Option<&Path>) -> Vec<PluginInfo> {
    match tool {
        "claude" => claude(env, project),
        "copilot" => copilot(env, project),
        "droid" => droid(env, project),
        "cursor" => cursor(env),
        "gemini" | "qwen" => extensions(env, tool, project),
        "opencode" => opencode(env, project),
        _ => vec![],
    }
}

fn err(code: &'static str, msg: impl Into<String>) -> AgentkitError {
    AgentkitError::new(code, msg)
}

/// Writes `text` over `path` inside the transaction.
fn write_in(tx: &mut Tx, path: &Path, text: &str) -> Result<()> {
    tx.backup(path)?;
    writer::ensure_parent(path)?;
    writer::atomic_write(path, text.as_bytes(), None)
}

/// Sets one key of a JSON(C) file, keeping its formatting.
fn set_json_key(tx: &mut Tx, path: &Path, keys: &[Seg], value: &Value) -> Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let fmt = super::convert::format_for_path(path);
    let fmt = if fmt == DocFormat::Markdown {
        DocFormat::Json
    } else {
        fmt
    };
    let base = if text.trim().is_empty() {
        fmt.empty_doc().to_string()
    } else {
        text.clone()
    };
    let mut doc = edit::open(fmt, &base)?;
    if doc.get(keys).as_ref() == Some(value) && !text.trim().is_empty() {
        return Ok(());
    }
    doc.set(keys, value)?;
    let mut new = doc.text().to_string();
    if text.trim().is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    write_in(tx, path, &new)
}

/// Turns a plugin on or off in the tool's own file. `scope`: `user | project |
/// local` (what the plugin's `toggle_scopes` lists).
pub fn set_enabled(
    env: &Env,
    tool: &str,
    plugin: &str,
    enabled: bool,
    scope: &str,
    project: Option<&Path>,
) -> Result<ToggleReport> {
    let scope = match scope.trim() {
        "" | "user" | "global" => "user",
        "project" => "project",
        "local" => "local",
        other => return Err(err("AGENTKIT_SCOPE", format!("unknown scope `{other}`"))),
    };
    if scope != "user" && project.is_none() {
        return Err(err(
            "AGENTKIT_SCOPE",
            "project scope needs a project folder",
        ));
    }
    let current = list_tool(env, tool, project);
    let found = current
        .iter()
        .find(|p| p.id == plugin || p.name == plugin)
        .cloned();
    let mut tx = Tx::begin(env, "plugin")?;
    let file: PathBuf = match tool {
        "claude" | "copilot" | "droid" => {
            let id = found
                .as_ref()
                .map(|p| p.id.clone())
                .unwrap_or_else(|| plugin.to_string());
            if found.is_none() && !id.contains('@') {
                return Err(err(
                    "AGENTKIT_PLUGIN",
                    format!("{tool} has no plugin `{plugin}`"),
                ));
            }
            let f = settings_files(env, tool, project)
                .into_iter()
                .find(|(s, _)| s == scope)
                .map(|(_, f)| f)
                .ok_or_else(|| err("AGENTKIT_SCOPE", format!("{tool} has no {scope} settings")))?;
            set_json_key(
                &mut tx,
                &f,
                &[Seg::key("enabledPlugins"), Seg::key(id)],
                &Value::Bool(enabled),
            )?;
            f
        }
        "gemini" | "qwen" => {
            let p = found.ok_or_else(|| {
                err(
                    "AGENTKIT_PLUGIN",
                    format!("{tool} has no extension `{plugin}`"),
                )
            })?;
            if scope == "local" {
                return Err(err(
                    "AGENTKIT_SCOPE",
                    "extensions have user and project scopes",
                ));
            }
            let dir = env
                .home
                .join(if tool == "gemini" { ".gemini" } else { ".qwen" })
                .join("extensions");
            let f = dir.join("extension-enablement.json");
            let mut all = read_json(&f)
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            let rules: Vec<String> = all
                .get(&p.name)
                .and_then(|v| v.get("overrides"))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let at = if scope == "user" {
                env.home.clone()
            } else {
                project.unwrap().to_path_buf()
            };
            let new = set_extension_rule(&rules, &at, enabled);
            all.insert(p.name.clone(), serde_json::json!({ "overrides": new }));
            // Gemini writes this file with JSON.stringify(_, null, 2)
            let text = serde_json::to_string_pretty(&Value::Object(all)).unwrap_or_default();
            write_in(&mut tx, &f, &text)?;
            f
        }
        "opencode" => {
            let p = found.ok_or_else(|| {
                err(
                    "AGENTKIT_PLUGIN",
                    format!("OpenCode has no plugin `{plugin}`"),
                )
            })?;
            if let Some(base) = p.id.strip_prefix("file:") {
                let live = PathBuf::from(p.path.clone().unwrap_or_default());
                let parked = live.with_file_name(format!("{base}.disabled"));
                let (from, to) = if enabled {
                    (parked, live)
                } else {
                    (live, parked)
                };
                if from.is_file() && !to.exists() {
                    let bytes = std::fs::read(&from)
                        .map_err(|e| AgentkitError::io("reading", &from, &e))?;
                    tx.backup(&from)?;
                    tx.backup(&to)?;
                    writer::atomic_write(&to, &bytes, None)?;
                    std::fs::remove_file(&from)
                        .map_err(|e| AgentkitError::io("removing", &from, &e))?;
                }
                to
            } else {
                let cfg = PathBuf::from(p.path.clone().unwrap_or_default());
                let key = cfg.display().to_string();
                let spec = p.id.trim_start_matches("npm:").to_string();
                let store_path = disabled_store(env);
                let mut store = read_json(&store_path)
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                let text = std::fs::read_to_string(&cfg).unwrap_or_default();
                let fmt = super::convert::format_for_path(&cfg);
                let base = if text.trim().is_empty() {
                    fmt.empty_doc().to_string()
                } else {
                    text
                };
                let mut doc = edit::open(fmt, &base)?;
                let path = [Seg::key("plugin")];
                let arr = doc
                    .get(&path)
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let mut parked: Vec<Value> = store
                    .get("opencode")
                    .and_then(|m| m.get(&key))
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                if enabled {
                    if let Some(i) = parked
                        .iter()
                        .position(|e| npm_spec(e).as_deref() == Some(&spec))
                    {
                        let entry = parked.remove(i);
                        if !arr.iter().any(|e| npm_spec(e).as_deref() == Some(&spec)) {
                            doc.push(&path, &entry)?;
                        }
                    }
                } else if let Some(entry) = arr
                    .iter()
                    .find(|e| npm_spec(e).as_deref() == Some(&spec))
                    .cloned()
                {
                    doc.remove_item(&path, &entry)?;
                    if !parked.contains(&entry) {
                        parked.push(entry);
                    }
                }
                let new_text = doc.text().to_string();
                write_in(&mut tx, &cfg, &new_text)?;
                let oc = store
                    .entry("opencode")
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Some(m) = oc.as_object_mut() {
                    if parked.is_empty() {
                        m.remove(&key);
                    } else {
                        m.insert(key.clone(), Value::Array(parked));
                    }
                }
                let text = serde_json::to_string_pretty(&Value::Object(store)).unwrap_or_default();
                write_in(&mut tx, &store_path, &text)?;
                cfg
            }
        }
        "cursor" => {
            return Err(err(
                "AGENTKIT_PLUGIN_READONLY",
                "Cursor keeps plugins on/off in its own app state; switch it in Cursor",
            ))
        }
        other => {
            return Err(err(
                "AGENTKIT_PLUGIN",
                format!("no plugin support for `{other}`"),
            ))
        }
    };
    let plugin_now = list_tool(env, tool, project)
        .into_iter()
        .find(|p| p.id == plugin || p.name == plugin);
    Ok(ToggleReport {
        tx: tx.id.clone(),
        file: file.display().to_string(),
        plugin: plugin_now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::Os;

    fn w(p: &Path, t: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }

    #[test]
    fn gemini_rules_follow_gemini_semantics() {
        let home = Path::new("/Users/me");
        let proj = Path::new("/Users/me/work/app");
        let r = set_extension_rule(&[], home, false);
        assert_eq!(r, vec!["!/Users/me/*"]);
        assert!(!extension_enabled(&r, proj));
        let r2 = set_extension_rule(&r, proj, true);
        assert_eq!(r2, vec!["!/Users/me/*", "/Users/me/work/app/*"]);
        assert!(extension_enabled(&r2, proj));
        assert!(!extension_enabled(&r2, Path::new("/Users/me/other")));
        // enabling at home again drops the child rule
        let r3 = set_extension_rule(&r2, home, true);
        assert_eq!(r3, vec!["/Users/me/*"]);
    }

    #[test]
    fn inventory_and_switches_per_tool() {
        let root = std::env::temp_dir().join(format!(
            "agentkit-plugins-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        let home = root.join("home");
        let proj = root.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let env = Env::sandbox(&home, Os::Linux);
        // Claude: one installed plugin with skills, hooks, MCP; settings with comments kept
        let cache = home.join(".claude/plugins/cache/mkt/guard/1.0.0");
        w(
            &cache.join(".claude-plugin/plugin.json"),
            r#"{"name":"guard","version":"1.0.0","description":"Guards"}"#,
        );
        w(
            &cache.join("skills/check/SKILL.md"),
            "---\nname: check\n---\n",
        );
        w(&cache.join("commands/git/ship.md"), "ship\n");
        w(
            &cache.join("hooks/hooks.json"),
            r#"{"hooks":{"PreToolUse":[]}}"#,
        );
        w(
            &cache.join(".mcp.json"),
            r#"{"mcpServers":{"db":{"command":"x"}}}"#,
        );
        w(
            &home.join(".claude/plugins/installed_plugins.json"),
            &format!(
                r#"{{"version":2,"plugins":{{"guard@mkt":[{{"scope":"user","installPath":"{}","version":"1.0.0"}}]}}}}"#,
                cache.display()
            ),
        );
        w(
            &home.join(".claude/plugins/known_marketplaces.json"),
            r#"{"mkt":{"source":{"source":"github","repo":"acme/plugins"}}}"#,
        );
        let settings =
            "{\n  \"model\": \"opus\",\n  \"enabledPlugins\": {\n    \"guard@mkt\": true\n  }\n}\n";
        w(&home.join(".claude/settings.json"), settings);
        // Gemini extension
        let ext = home.join(".gemini/extensions/lint");
        w(
            &ext.join("gemini-extension.json"),
            r#"{"name":"lint","version":"0.2.0","mcpServers":{"lint":{"command":"node"}}}"#,
        );
        w(&ext.join("commands/fix.toml"), "prompt = \"x\"\n");
        w(
            &ext.join(".gemini-extension-install.json"),
            r#"{"source":"https://github.com/acme/lint","type":"git"}"#,
        );
        // OpenCode npm + file plugin
        w(
            &proj.join("opencode.json"),
            "{\n  // mine\n  \"plugin\": [\"opencode-wakatime\", \"@acme/p@1.2.0\"]\n}\n",
        );
        w(
            &proj.join(".opencode/plugins/notify.ts"),
            "export const N = async () => ({})\n",
        );

        let all = list(&env, Some(&proj));
        let g = all.iter().find(|p| p.tool == "claude").unwrap();
        assert_eq!(g.id, "guard@mkt");
        assert_eq!(g.enabled, Some(true));
        assert_eq!(g.source.as_deref(), Some("github:acme/plugins"));
        assert_eq!(g.components.skills, vec!["check"]);
        assert_eq!(g.components.commands, vec!["git:ship"]);
        assert_eq!(g.components.hooks, vec!["PreToolUse"]);
        assert_eq!(g.components.mcp_servers, vec!["db"]);
        let l = all.iter().find(|p| p.tool == "gemini").unwrap();
        assert_eq!(l.enabled, Some(true));
        assert_eq!(l.components.commands, vec!["fix"]);
        assert_eq!(l.source.as_deref(), Some("https://github.com/acme/lint"));
        let oc: Vec<&PluginInfo> = all.iter().filter(|p| p.tool == "opencode").collect();
        assert_eq!(oc.len(), 3, "{oc:?}");
        assert_eq!(
            oc.iter()
                .find(|p| p.name == "@acme/p")
                .unwrap()
                .version
                .as_deref(),
            Some("1.2.0")
        );

        // Claude off in the project scope: user keeps true, project false wins
        let r = set_enabled(&env, "claude", "guard@mkt", false, "project", Some(&proj)).unwrap();
        let p = r.plugin.unwrap();
        assert_eq!(p.enabled, Some(false));
        assert_eq!(p.enabled_in.get("user"), Some(&true));
        assert_eq!(
            std::fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
            settings
        );
        // user scope off keeps the rest of the file byte for byte
        set_enabled(&env, "claude", "guard@mkt", false, "user", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
            settings.replace("\"guard@mkt\": true", "\"guard@mkt\": false")
        );
        // Gemini off for the user, on for the project
        set_enabled(&env, "gemini", "lint", false, "user", None).unwrap();
        let r = set_enabled(&env, "gemini", "lint", true, "project", Some(&proj)).unwrap();
        assert_eq!(r.plugin.unwrap().enabled, Some(true));
        assert_eq!(list_tool(&env, "gemini", None)[0].enabled, Some(false));
        // OpenCode npm entry parked and restored, file renamed and back
        let off = set_enabled(
            &env,
            "opencode",
            "npm:opencode-wakatime",
            false,
            "project",
            Some(&proj),
        )
        .unwrap();
        assert_eq!(off.plugin.unwrap().enabled, Some(false));
        let cfg = std::fs::read_to_string(proj.join("opencode.json")).unwrap();
        assert!(
            !cfg.contains("wakatime") && cfg.contains("// mine"),
            "{cfg}"
        );
        set_enabled(
            &env,
            "opencode",
            "npm:opencode-wakatime",
            true,
            "project",
            Some(&proj),
        )
        .unwrap();
        let cfg = std::fs::read_to_string(proj.join("opencode.json")).unwrap();
        assert!(cfg.contains("opencode-wakatime"), "{cfg}");
        set_enabled(
            &env,
            "opencode",
            "file:notify.ts",
            false,
            "project",
            Some(&proj),
        )
        .unwrap();
        assert!(proj.join(".opencode/plugins/notify.ts.disabled").is_file());
        let back = set_enabled(
            &env,
            "opencode",
            "file:notify.ts",
            true,
            "project",
            Some(&proj),
        )
        .unwrap();
        assert!(proj.join(".opencode/plugins/notify.ts").is_file());
        // the last switch can be rolled back
        writer::restore(&env, &back.tx).unwrap();
        assert!(proj.join(".opencode/plugins/notify.ts.disabled").is_file());
        assert!(!proj.join(".opencode/plugins/notify.ts").exists());
        assert!(set_enabled(&env, "cursor", "x", true, "user", None).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `cargo test -p omniget-core --lib agentkit::plugins::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_inventory_on_this_machine() {
        let env = Env::system().unwrap();
        let all = list(&env, None);
        println!("{}", serde_json::to_string_pretty(&all).unwrap());
    }
}
