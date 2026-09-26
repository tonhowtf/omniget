//! Selective import of knowledge packs (skills, agents, rules, contexts,
//! commands) from a local folder such as an ECC clone.
//!
//! Nothing is imported by default: the person lists what the pack offers
//! (metadata only, never the bodies into a prompt), picks items, and gets a
//! plan first: kind, destination, origin (repo + commit), content hash,
//! license and attribution, dependencies, the capabilities each item asks
//! for, and conflicts with what is installed. Applying the plan:
//! - installs skills through `skills::install` (staging, zip-slip and
//!   symlink rules, the security scan) and records their hash; scripts
//!   inside a skill are copied as text — running them is a separate grant
//!   that stays off (`bots_skill_bindings.allow_scripts = 0`);
//! - stores agents, rules, contexts and commands as versioned text in the
//!   assistant database (the same store as the rest of `/llm`);
//! - never runs a hook and never installs a dependency: hooks, installers and
//!   scripts outside a skill come back `unsupported` with the reason;
//! - never overwrites a local customisation silently (conflict until the
//!   person chooses), and defers a skill update while a running mission uses
//!   that skill;
//! - keeps the previous version of everything it replaces, so an update is a
//!   diff and a rollback restores it.

pub mod presets;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::db::{AssistDb, Migration};
use super::{new_id, now_ms};
use crate::core::skills;

pub const ERR_PACK: &str = "ERR_PACK";
pub const ERR_PACK_INPUT: &str = "ERR_PACK_INPUT";
pub const ERR_PACK_CONFLICT: &str = "ERR_PACK_CONFLICT";
/// Largest text item (agent, rule, context, command).
pub const TEXT_MAX: u64 = 256 * 1024;
/// Most items one plan may hold.
pub const PLAN_MAX: usize = 64;

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "packs",
    version: 1,
    sql: "
CREATE TABLE packs_items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    origin TEXT NOT NULL,
    origin_sha TEXT,
    source_path TEXT NOT NULL,
    hash TEXT NOT NULL,
    installed_hash TEXT,
    license TEXT,
    attribution TEXT,
    capabilities TEXT NOT NULL DEFAULT '[]',
    deps TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    content TEXT,
    installed_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL,
    UNIQUE(kind, name)
);
CREATE TABLE packs_history (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    action TEXT NOT NULL,
    hash TEXT NOT NULL,
    content TEXT,
    backup_path TEXT,
    created_ms INTEGER NOT NULL
);
CREATE INDEX packs_history_item ON packs_history(item_id, version DESC);
",
}];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Skill,
    Agent,
    Rule,
    Context,
    Command,
    /// Listed so the person sees it; never installed.
    Hook,
    Script,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::Skill => "skill",
            ItemKind::Agent => "agent",
            ItemKind::Rule => "rule",
            ItemKind::Context => "context",
            ItemKind::Command => "command",
            ItemKind::Hook => "hook",
            ItemKind::Script => "script",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "skill" => ItemKind::Skill,
            "agent" => ItemKind::Agent,
            "rule" => ItemKind::Rule,
            "context" => ItemKind::Context,
            "command" => ItemKind::Command,
            "hook" => ItemKind::Hook,
            "script" => ItemKind::Script,
            _ => return None,
        })
    }
}

/// One entry of a pack's catalog (metadata only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogItem {
    pub kind: ItemKind,
    pub name: String,
    /// `kind:name`, what a selection lists.
    pub key: String,
    pub rel_path: String,
    pub description: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceMeta {
    /// Repository URL or folder label.
    pub origin: String,
    pub sha: Option<String>,
    pub license: Option<String>,
    pub attribution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    pub key: String,
    pub kind: ItemKind,
    pub name: String,
    pub rel_path: String,
    /// Where it lands: `skills/<name>` or `assist.db:packs_items`.
    pub dest: String,
    pub hash: String,
    pub bytes: u64,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub deps: Vec<String>,
    /// Tools/abilities the item asks for (`tools: Read, Bash` → `Read`, `Bash`).
    pub capabilities: Vec<String>,
    /// `new|update|unchanged|conflict_local_changes|conflict_name|unsupported|refused`
    pub action: String,
    pub reason: Option<String>,
    /// Line diff for an update of a text item (or SKILL.md).
    pub diff: Option<String>,
    /// Scripts found inside a skill: copied as text, never run by default.
    pub scripts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportPlan {
    pub source_dir: String,
    pub meta: SourceMeta,
    pub items: Vec<PlanItem>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackItem {
    pub id: String,
    pub kind: ItemKind,
    pub name: String,
    pub origin: String,
    pub origin_sha: Option<String>,
    pub source_path: String,
    pub hash: String,
    pub installed_hash: Option<String>,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub capabilities: Vec<String>,
    pub deps: Vec<String>,
    pub status: String,
    pub version: i64,
    pub content: Option<String>,
    pub installed_ms: i64,
    pub updated_ms: i64,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{ERR_PACK}: {e}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.contains("..")
        && !s.starts_with('/')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// A path inside `root` that is not a symlink escaping it.
fn inside(root: &Path, p: &Path) -> Result<(), String> {
    let md = std::fs::symlink_metadata(p).map_err(err)?;
    if md.file_type().is_symlink() {
        let target = std::fs::canonicalize(p)
            .map_err(|_| format!("{} is a dangling symlink", p.display()))?;
        let real_root = std::fs::canonicalize(root).map_err(err)?;
        if !target.starts_with(&real_root) {
            return Err(format!(
                "{} is a symlink that leaves the pack",
                p.strip_prefix(root).unwrap_or(p).display()
            ));
        }
    }
    Ok(())
}

/// Audit F-P1: the real (canonical) path of `p` must lie inside the real pack
/// root. Catches a symlinked category folder (`agents -> ../private-notes`)
/// or any symlinked ancestor, which `inside` (the item alone) does not.
fn contained(root: &Path, p: &Path) -> Result<(), String> {
    let real_root = std::fs::canonicalize(root).map_err(err)?;
    let real = std::fs::canonicalize(p).map_err(|_| {
        format!(
            "{} cannot be resolved",
            p.strip_prefix(root).unwrap_or(p).display()
        )
    })?;
    if !real.starts_with(&real_root) {
        return Err(format!(
            "{} resolves outside the pack (symlinked folder)",
            p.strip_prefix(root).unwrap_or(p).display()
        ));
    }
    Ok(())
}

/// Reads at most `TEXT_MAX` bytes of UTF-8 text (catalog descriptions).
fn read_bounded(p: &Path) -> String {
    use std::io::Read;
    let mut out = Vec::new();
    if let Ok(f) = std::fs::File::open(p) {
        let _ = f.take(TEXT_MAX).read_to_end(&mut out);
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Installed name of a skill folder: frontmatter `name` or the folder name,
/// validated as a skill name (audit F-P2) before any path is built from it.
fn skill_name(src: &Path, folder: &str) -> Result<(String, String), String> {
    let text = read_bounded(&src.join("SKILL.md"));
    let raw = frontmatter(&text)
        .get("name")
        .cloned()
        .unwrap_or_else(|| folder.to_string());
    let name = skills::manifest::validate_name(&raw).map_err(|e| e.to_string())?;
    Ok((name, text))
}

fn frontmatter(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(rest) = text.strip_prefix("---") else {
        return out;
    };
    let Some(end) = rest.find("\n---") else {
        return out;
    };
    for line in rest[..end].lines() {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            if !k.is_empty() && !k.starts_with(' ') && !k.starts_with('#') {
                out.insert(k.to_string(), v.trim().trim_matches('"').to_string());
            }
        }
    }
    out
}

fn first_line_description(text: &str) -> String {
    let fm = frontmatter(text);
    if let Some(d) = fm.get("description") {
        return super::missions::clip(d, 200);
    }
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("---") && !l.starts_with('#'))
        .map(|l| super::missions::clip(l, 200))
        .unwrap_or_default()
}

/// License of the pack root (`LICENSE*` first line).
pub fn detect_license(dir: &Path) -> Option<String> {
    for name in ["LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE"] {
        if let Ok(t) = std::fs::read_to_string(dir.join(name)) {
            return t
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_string);
        }
    }
    None
}

/// The pack's catalog, metadata only (ECC layout: `skills/<n>/SKILL.md`,
/// `agents/*.md`, `rules/**.md`, `contexts/*.md`, `commands/*.md`, plus
/// `hooks/` and `scripts/` listed as unsupported).
pub fn catalog(dir: &Path) -> Result<Vec<CatalogItem>, String> {
    if !dir.is_dir() {
        return Err(format!(
            "{ERR_PACK_INPUT}: {} is not a folder",
            dir.display()
        ));
    }
    let mut out = Vec::new();
    let skills_dir = dir.join("skills");
    let listed = |p: &Path| contained(dir, p).is_ok();
    if let Ok(rd) = std::fs::read_dir(&skills_dir).map_err(drop).and_then(|rd| {
        if listed(&skills_dir) {
            Ok(rd)
        } else {
            Err(())
        }
    }) {
        for e in rd.flatten() {
            let p = e.path();
            let f = p.join("SKILL.md");
            if f.is_file() && listed(&f) {
                let name = e.file_name().to_string_lossy().to_string();
                let text = read_bounded(&f);
                out.push(CatalogItem {
                    kind: ItemKind::Skill,
                    key: format!("skill:{name}"),
                    rel_path: format!("skills/{name}"),
                    description: first_line_description(&text),
                    bytes: text.len() as u64,
                    name,
                });
            }
        }
    }
    for (sub, kind) in [
        ("agents", ItemKind::Agent),
        ("contexts", ItemKind::Context),
        ("commands", ItemKind::Command),
    ] {
        if !listed(&dir.join(sub)) {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(dir.join(sub)) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "md").unwrap_or(false) && p.is_file() && listed(&p) {
                    let name = p
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let text = read_bounded(&p);
                    out.push(CatalogItem {
                        kind,
                        key: format!("{}:{name}", kind.as_str()),
                        rel_path: format!("{sub}/{name}.md"),
                        description: first_line_description(&text),
                        bytes: text.len() as u64,
                        name,
                    });
                }
            }
        }
    }
    // rules/<lang>/<file>.md
    let rules = dir.join("rules");
    if let Ok(rd) = std::fs::read_dir(&rules).map_err(drop).and_then(|rd| {
        if listed(&rules) {
            Ok(rd)
        } else {
            Err(())
        }
    }) {
        for lang in rd
            .flatten()
            .filter(|e| e.path().is_dir() && listed(&e.path()))
        {
            if let Ok(files) = std::fs::read_dir(lang.path()) {
                for f in files.flatten() {
                    let p = f.path();
                    if p.extension().map(|x| x == "md").unwrap_or(false)
                        && p.is_file()
                        && listed(&p)
                    {
                        let name = format!(
                            "{}/{}",
                            lang.file_name().to_string_lossy(),
                            p.file_stem().unwrap_or_default().to_string_lossy()
                        );
                        let text = read_bounded(&p);
                        out.push(CatalogItem {
                            kind: ItemKind::Rule,
                            key: format!("rule:{name}"),
                            rel_path: format!("rules/{name}.md"),
                            description: first_line_description(&text),
                            bytes: text.len() as u64,
                            name,
                        });
                    }
                }
            }
        }
    }
    for (sub, kind) in [("hooks", ItemKind::Hook), ("scripts", ItemKind::Script)] {
        if dir.join(sub).is_dir() {
            out.push(CatalogItem {
                kind,
                key: format!("{}:{sub}", kind.as_str()),
                rel_path: sub.to_string(),
                description: format!("{sub}: executable; listed, never installed by an import"),
                bytes: 0,
                name: sub.to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

fn scripts_in(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p
                    .extension()
                    .map(|x| {
                        matches!(
                            x.to_str(),
                            Some("sh" | "py" | "js" | "ts" | "ps1" | "rb" | "bash")
                        )
                    })
                    .unwrap_or(false)
                {
                    out.push(
                        p.strip_prefix(dir)
                            .unwrap_or(&p)
                            .to_string_lossy()
                            .to_string(),
                    );
                }
            }
        }
    }
    out.sort();
    out
}

fn tools_of(fm: &BTreeMap<String, String>) -> Vec<String> {
    let raw = fm
        .get("tools")
        .or_else(|| fm.get("allowed-tools"))
        .cloned()
        .unwrap_or_default();
    raw.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|t| t.trim().trim_matches('"').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn line_diff(old: &str, new: &str) -> String {
    super::learning::line_diff(old, new)
}

fn item(db: &AssistDb, kind: ItemKind, name: &str) -> Result<Option<PackItem>, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {ITEM_COLS} FROM packs_items WHERE kind = ?1 AND name = ?2"),
            params![kind.as_str(), name],
            row_item,
        )
        .optional()
    })
}

const ITEM_COLS: &str = "id, kind, name, origin, origin_sha, source_path, hash, installed_hash, license, attribution, capabilities, deps, status, version, content, installed_ms, updated_ms";

fn row_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<PackItem> {
    Ok(PackItem {
        id: r.get(0)?,
        kind: ItemKind::parse(&r.get::<_, String>(1)?).unwrap_or(ItemKind::Script),
        name: r.get(2)?,
        origin: r.get(3)?,
        origin_sha: r.get(4)?,
        source_path: r.get(5)?,
        hash: r.get(6)?,
        installed_hash: r.get(7)?,
        license: r.get(8)?,
        attribution: r.get(9)?,
        capabilities: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or_default(),
        deps: serde_json::from_str(&r.get::<_, String>(11)?).unwrap_or_default(),
        status: r.get(12)?,
        version: r.get(13)?,
        content: r.get(14)?,
        installed_ms: r.get(15)?,
        updated_ms: r.get(16)?,
    })
}

pub fn items(db: &AssistDb) -> Result<Vec<PackItem>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {ITEM_COLS} FROM packs_items ORDER BY kind, name"
        ))?;
        let rows = st.query_map([], row_item)?;
        rows.collect()
    })
}

pub fn item_by_id(db: &AssistDb, id: &str) -> Result<PackItem, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {ITEM_COLS} FROM packs_items WHERE id = ?1"),
            params![id],
            row_item,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_PACK}: no item {id}"))
}

/// Builds the plan for the selected keys (`kind:name`). An empty selection
/// is refused: there is no "import everything" by default; `all_of` must
/// name the kinds explicitly.
pub fn plan(
    db: &AssistDb,
    dir: &Path,
    skills_root: &Path,
    selection: &[String],
    all_of: &[ItemKind],
    meta: SourceMeta,
) -> Result<ImportPlan, String> {
    if selection.is_empty() && all_of.is_empty() {
        return Err(format!(
            "{ERR_PACK_INPUT}: pick what to import; nothing is imported by default"
        ));
    }
    let cat = catalog(dir)?;
    let license = meta.license.clone().or_else(|| detect_license(dir));
    let mut chosen: Vec<CatalogItem> = Vec::new();
    for key in selection {
        let (k, n) = key
            .split_once(':')
            .ok_or_else(|| format!("{ERR_PACK_INPUT}: `{key}` is not kind:name"))?;
        let kind =
            ItemKind::parse(k).ok_or_else(|| format!("{ERR_PACK_INPUT}: unknown kind `{k}`"))?;
        match cat.iter().find(|c| c.key == *key) {
            Some(c) => chosen.push(c.clone()),
            None => chosen.push(CatalogItem {
                kind,
                name: n.to_string(),
                key: key.clone(),
                rel_path: n.to_string(),
                description: String::new(),
                bytes: 0,
            }),
        }
    }
    for c in cat.iter().filter(|c| all_of.contains(&c.kind)) {
        if !chosen.iter().any(|x| x.key == c.key) {
            chosen.push(c.clone());
        }
    }
    if chosen.len() > PLAN_MAX {
        return Err(format!(
            "{ERR_PACK_INPUT}: {} items; one plan holds at most {PLAN_MAX}",
            chosen.len()
        ));
    }
    let running_skills = skills_in_running_missions(db)?;
    let mut items = Vec::new();
    for c in chosen {
        let mut pi = PlanItem {
            key: c.key.clone(),
            kind: c.kind,
            name: c.name.clone(),
            rel_path: c.rel_path.clone(),
            dest: String::new(),
            hash: String::new(),
            bytes: 0,
            license: license.clone(),
            attribution: meta.attribution.clone().or_else(|| {
                Some(format!(
                    "{}{}",
                    meta.origin,
                    meta.sha
                        .as_deref()
                        .map(|s| format!(" @ {}", &s[..s.len().min(12)]))
                        .unwrap_or_default()
                ))
            }),
            deps: Vec::new(),
            capabilities: Vec::new(),
            action: "new".into(),
            reason: None,
            diff: None,
            scripts: Vec::new(),
        };
        if matches!(c.kind, ItemKind::Hook | ItemKind::Script) {
            pi.action = "unsupported".into();
            pi.reason = Some("hooks and scripts run code; an import installs text only. Running them needs a separate capability that is not granted by importing".into());
            items.push(pi);
            continue;
        }
        if !valid_name(&c.name) {
            pi.action = "refused".into();
            pi.reason =
                Some("the name leaves the pack or has characters that are not allowed".into());
            items.push(pi);
            continue;
        }
        let src = dir.join(&c.rel_path);
        if !src.exists() {
            pi.action = "refused".into();
            pi.reason = Some(format!("{} is not in the pack", c.rel_path));
            items.push(pi);
            continue;
        }
        if let Err(e) = inside(dir, &src).and_then(|_| contained(dir, &src)) {
            pi.action = "refused".into();
            pi.reason = Some(e);
            items.push(pi);
            continue;
        }
        match c.kind {
            ItemKind::Skill => {
                // Every file inside must stay inside the pack.
                let mut bad = None;
                let mut stack = vec![src.clone()];
                while let Some(d) = stack.pop() {
                    for e in std::fs::read_dir(&d).map_err(err)?.flatten() {
                        let p = e.path();
                        if let Err(x) = inside(dir, &p) {
                            bad = Some(x);
                        } else if p.is_dir()
                            && !std::fs::symlink_metadata(&p)
                                .map(|m| m.file_type().is_symlink())
                                .unwrap_or(false)
                        {
                            stack.push(p);
                        }
                    }
                }
                if let Some(x) = bad {
                    pi.action = "refused".into();
                    pi.reason = Some(x);
                    items.push(pi);
                    continue;
                }
                let (installed_name, text) = match skill_name(&src, &c.name) {
                    Ok(v) => v,
                    Err(e) => {
                        pi.action = "refused".into();
                        pi.reason = Some(format!("the skill's name is not allowed: {e}"));
                        items.push(pi);
                        continue;
                    }
                };
                let fm = frontmatter(&text);
                pi.capabilities = tools_of(&fm);
                if let Some(l) = fm.get("license") {
                    pi.license = Some(l.clone());
                }
                pi.scripts = scripts_in(&src);
                if !pi.scripts.is_empty() {
                    pi.capabilities.push(
                        "scripts (copied as text; not runnable unless you allow it per bot)".into(),
                    );
                }
                pi.hash = skills::hash::dir_hash_uncached(&src).map_err(|e| e.to_string())?;
                pi.bytes = text.len() as u64;
                pi.dest = format!("skills/{installed_name}");
                let installed_dir = skills_root.join(&installed_name);
                let mine = item(db, ItemKind::Skill, &installed_name)?;
                if installed_dir.join("SKILL.md").is_file() {
                    let now_hash = skills::hash::dir_hash_uncached(&installed_dir)
                        .map_err(|e| e.to_string())?;
                    match mine {
                        None => {
                            pi.action = "conflict_name".into();
                            pi.reason = Some("a skill with this name is already installed from somewhere else; importing would replace it".into());
                        }
                        Some(m) if m.installed_hash.as_deref() != Some(now_hash.as_str()) => {
                            pi.action = "conflict_local_changes".into();
                            pi.reason = Some("the installed copy was changed locally since it was imported; it is not overwritten unless you choose to".into());
                        }
                        Some(m) if m.hash == pi.hash => pi.action = "unchanged".into(),
                        Some(_) => {
                            pi.action = "update".into();
                            let old = std::fs::read_to_string(installed_dir.join("SKILL.md"))
                                .unwrap_or_default();
                            pi.diff = Some(super::missions::clip(&line_diff(&old, &text), 8000));
                        }
                    }
                    if running_skills.contains(&installed_name)
                        && matches!(
                            pi.action.as_str(),
                            "update" | "conflict_local_changes" | "conflict_name"
                        )
                    {
                        pi.reason = Some(format!("{} — a running mission uses this skill; the update waits until it ends", pi.reason.clone().unwrap_or_default()));
                        pi.action = "deferred".into();
                    }
                }
            }
            _ => {
                let md = std::fs::metadata(&src).map_err(err)?;
                if md.len() > TEXT_MAX {
                    pi.action = "refused".into();
                    pi.reason = Some(format!(
                        "{} bytes; a text item holds at most {TEXT_MAX}",
                        md.len()
                    ));
                    items.push(pi);
                    continue;
                }
                let text = std::fs::read_to_string(&src).map_err(err)?;
                let fm = frontmatter(&text);
                pi.capabilities = tools_of(&fm);
                pi.hash = sha256_hex(text.as_bytes());
                pi.bytes = md.len();
                pi.dest = "assist.db:packs_items".into();
                if let Some(m) = item(db, c.kind, &c.name)? {
                    let local = m.content.as_deref().map(|t| sha256_hex(t.as_bytes()));
                    if local.as_deref() != m.installed_hash.as_deref() {
                        pi.action = "conflict_local_changes".into();
                        pi.reason = Some(
                            "edited locally since the import; not overwritten unless you choose to"
                                .into(),
                        );
                    } else if m.hash == pi.hash {
                        pi.action = "unchanged".into();
                    } else {
                        pi.action = "update".into();
                        pi.diff = Some(super::missions::clip(
                            &line_diff(m.content.as_deref().unwrap_or(""), &text),
                            8000,
                        ));
                    }
                }
            }
        }
        items.push(pi);
    }
    Ok(ImportPlan {
        source_dir: dir.to_string_lossy().to_string(),
        meta: SourceMeta { license, ..meta },
        items,
        created_ms: now_ms(),
    })
}

fn skills_in_running_missions(db: &AssistDb) -> Result<std::collections::HashSet<String>, String> {
    db.with(|c| {
        let mut st = c.prepare(
            "SELECT DISTINCT b.skill FROM missions_missions m JOIN bots_skill_bindings b ON b.bot_id = m.bot_id WHERE m.state IN ('queued','running','verifying')",
        )?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApplyReport {
    pub installed: Vec<String>,
    pub updated: Vec<String>,
    pub skipped: Vec<(String, String)>,
    /// Skills parked by the security scan: confirm or discard in Skills.
    pub pending_scan: Vec<(String, String)>,
    /// Items that failed while applying. Each item is all-or-nothing: a
    /// failed item left the installed state as it was before (or says where
    /// its previous copy is when restoring it failed too).
    pub failed: Vec<(String, String)>,
}

/// Applies a plan. `overwrite` names conflicting keys the person chose to
/// replace (their current copy is kept for rollback). Items the plan marked
/// unsupported, refused, unchanged or deferred are skipped with the reason.
///
/// The plan comes back from the UI, so nothing in it is trusted: every item
/// path must still resolve inside the pack, a skill's name is re-read and
/// re-validated, and a skill is re-hashed and must equal the planned hash
/// (audit F-P3). Items apply one by one, each atomically; an error on one is
/// reported in `failed` and the others still apply.
pub fn apply(
    db: &AssistDb,
    plan: &ImportPlan,
    skills_root: &Path,
    overwrite: &[String],
    scan: &skills::install::ScanFn<'_>,
) -> Result<ApplyReport, String> {
    let dir = PathBuf::from(&plan.source_dir);
    let mut rep = ApplyReport::default();
    for pi in &plan.items {
        let go = match pi.action.as_str() {
            "new" | "update" => true,
            "conflict_local_changes" | "conflict_name" if overwrite.contains(&pi.key) => true,
            other => {
                rep.skipped.push((
                    pi.key.clone(),
                    pi.reason.clone().unwrap_or_else(|| other.to_string()),
                ));
                false
            }
        };
        if !go {
            continue;
        }
        match apply_item(db, plan, &dir, pi, skills_root, overwrite, scan) {
            Ok(Applied::Done) => {
                if pi.action == "new" {
                    rep.installed.push(pi.key.clone());
                } else {
                    rep.updated.push(pi.key.clone());
                }
            }
            Ok(Applied::Skipped(why)) => rep.skipped.push((pi.key.clone(), why)),
            Ok(Applied::PendingScan(tok)) => rep.pending_scan.push((pi.key.clone(), tok)),
            Err(e) => rep.failed.push((pi.key.clone(), e)),
        }
    }
    Ok(rep)
}

enum Applied {
    Done,
    Skipped(String),
    PendingScan(String),
}

fn apply_item(
    db: &AssistDb,
    plan: &ImportPlan,
    dir: &Path,
    pi: &PlanItem,
    skills_root: &Path,
    overwrite: &[String],
    scan: &skills::install::ScanFn<'_>,
) -> Result<Applied, String> {
    if !valid_name(&pi.rel_path) {
        return Ok(Applied::Skipped(
            "the item path is not allowed; plan again".into(),
        ));
    }
    let src = dir.join(&pi.rel_path);
    if std::fs::symlink_metadata(&src).is_err() {
        return Ok(Applied::Skipped(format!(
            "{} is no longer in the pack; plan again",
            pi.rel_path
        )));
    }
    if let Err(e) = inside(dir, &src).and_then(|_| contained(dir, &src)) {
        return Ok(Applied::Skipped(format!("{e}; plan again")));
    }
    let now = now_ms();
    match pi.kind {
        ItemKind::Skill => {
            let folder = src
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let (name, _) = skill_name(&src, &folder)?;
            if pi.dest != format!("skills/{name}") {
                return Ok(Applied::Skipped(
                    "the skill's name changed after the plan; plan again".into(),
                ));
            }
            let hash = skills::hash::dir_hash_uncached(&src).map_err(|e| e.to_string())?;
            if hash != pi.hash {
                return Ok(Applied::Skipped(
                    "the skill changed after the plan; plan again".into(),
                ));
            }
            let installed_dir = skills_root.join(&name);
            let exists = installed_dir.join("SKILL.md").is_file();
            // A copy that appeared after a `new` plan is a conflict, never a
            // silent overwrite.
            if exists && pi.action == "new" && !overwrite.contains(&pi.key) {
                return Ok(Applied::Skipped(
                    "a skill with this name appeared after the plan; plan again".into(),
                ));
            }
            // Keep the current copy for rollback, inside the skills root only.
            let backup = if exists {
                let b = skills_root
                    .join(".packs-backup")
                    .join(format!("{name}-{now}"));
                if !b.starts_with(skills_root) {
                    return Err(format!("{ERR_PACK}: backup path leaves the skills folder"));
                }
                if let Err(e) = copy_dir(&installed_dir, &b) {
                    let _ = std::fs::remove_dir_all(&b);
                    return Err(e);
                }
                Some(b)
            } else {
                None
            };
            let undo = |why: String| -> String {
                // Restore exactly what was installed before this item.
                let restored = match &backup {
                    Some(b) => {
                        (!installed_dir.exists() || std::fs::remove_dir_all(&installed_dir).is_ok())
                            && copy_dir(b, &installed_dir).is_ok()
                    }
                    None => {
                        !installed_dir.exists() || std::fs::remove_dir_all(&installed_dir).is_ok()
                    }
                };
                if restored {
                    why
                } else {
                    format!(
                        "{why}; restoring the previous copy failed, it is kept at {}",
                        backup
                            .as_ref()
                            .map(|b| b.display().to_string())
                            .unwrap_or_else(|| "(none)".into())
                    )
                }
            };
            let out = match skills::install::install_from_dir_in_with(skills_root, &src, scan) {
                Ok(o) => o,
                Err(e) => return Err(undo(e.to_string())),
            };
            if let Some(tok) = out.needs_confirm.clone() {
                return Ok(Applied::PendingScan(tok));
            }
            let installed_hash = match skills::hash::dir_hash_uncached(&installed_dir) {
                Ok(h) => h,
                Err(e) => return Err(undo(e.to_string())),
            };
            if let Err(e) = upsert(
                db,
                pi,
                &plan.meta,
                Some(&installed_hash),
                None,
                backup.as_deref(),
            ) {
                return Err(undo(e));
            }
            let _ = super::bots::skills::record_install(db, &out.manifest, &installed_hash);
        }
        _ => {
            let md = std::fs::metadata(&src).map_err(err)?;
            if md.len() > TEXT_MAX {
                return Ok(Applied::Skipped(format!(
                    "{} bytes; a text item holds at most {TEXT_MAX}",
                    md.len()
                )));
            }
            let text = std::fs::read_to_string(&src).map_err(err)?;
            if sha256_hex(text.as_bytes()) != pi.hash {
                return Ok(Applied::Skipped(
                    "the file changed after the plan; plan again".into(),
                ));
            }
            // One transaction: the text item is stored whole or not at all.
            upsert(db, pi, &plan.meta, Some(&pi.hash), Some(&text), None)?;
        }
    }
    Ok(Applied::Done)
}

fn upsert(
    db: &AssistDb,
    pi: &PlanItem,
    meta: &SourceMeta,
    installed_hash: Option<&str>,
    content: Option<&str>,
    backup: Option<&Path>,
) -> Result<(), String> {
    let now = now_ms();
    let name = if pi.kind == ItemKind::Skill {
        pi.dest.trim_start_matches("skills/").to_string()
    } else {
        pi.name.clone()
    };
    let prev = item(db, pi.kind, &name)?;
    db.tx(|tx| {
        match &prev {
            Some(p) => {
                tx.execute(
                    "INSERT INTO packs_history (id, item_id, version, action, hash, content, backup_path, created_ms) VALUES (?1,?2,?3,'replaced',?4,?5,?6,?7)",
                    params![new_id(), p.id, p.version, p.hash, p.content, backup.map(|b| b.to_string_lossy().to_string()), now],
                )
                .map_err(err)?;
                tx.execute(
                    "UPDATE packs_items SET origin=?2, origin_sha=?3, source_path=?4, hash=?5, installed_hash=?6, license=?7, attribution=?8, capabilities=?9, deps=?10, status='installed', version=version+1, content=?11, updated_ms=?12 WHERE id=?1",
                    params![p.id, meta.origin, meta.sha, pi.rel_path, pi.hash, installed_hash, pi.license, pi.attribution, serde_json::to_string(&pi.capabilities).unwrap_or_default(), serde_json::to_string(&pi.deps).unwrap_or_default(), content, now],
                )
                .map_err(err)?;
            }
            None => {
                tx.execute(
                    &format!("INSERT INTO packs_items ({ITEM_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'installed',1,?13,?14,?14)"),
                    params![new_id(), pi.kind.as_str(), name, meta.origin, meta.sha, pi.rel_path, pi.hash, installed_hash, pi.license, pi.attribution, serde_json::to_string(&pi.capabilities).unwrap_or_default(), serde_json::to_string(&pi.deps).unwrap_or_default(), content, now],
                )
                .map_err(err)?;
            }
        }
        Ok(())
    })
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(err)?;
    for e in std::fs::read_dir(from).map_err(err)?.flatten() {
        let p = e.path();
        let ft = e.file_type().map_err(err)?;
        let dest = to.join(e.file_name());
        if ft.is_symlink() {
            continue;
        } else if ft.is_dir() {
            copy_dir(&p, &dest)?;
        } else {
            std::fs::copy(&p, &dest).map_err(err)?;
        }
    }
    Ok(())
}

/// Restores the previous version of an item (text from history; a skill
/// from its backup copy). The replaced version goes to history too.
pub fn rollback(db: &AssistDb, item_id: &str, skills_root: &Path) -> Result<PackItem, String> {
    let it = item_by_id(db, item_id)?;
    let prev: Option<(i64, String, Option<String>, Option<String>)> = db.with(|c| {
        c.query_row(
            "SELECT version, hash, content, backup_path FROM packs_history WHERE item_id = ?1 AND action = 'replaced' ORDER BY version DESC, created_ms DESC LIMIT 1",
            params![item_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
    })?;
    let (pv, phash, pcontent, pbackup) =
        prev.ok_or_else(|| format!("{ERR_PACK}: no previous version to restore"))?;
    let now = now_ms();
    let mut installed_hash = Some(phash.clone());
    if it.kind == ItemKind::Skill {
        let b = PathBuf::from(
            pbackup.ok_or_else(|| format!("{ERR_PACK}: the skill's previous copy was not kept"))?,
        );
        let dest = skills_root.join(&it.name);
        let cur_backup = skills_root
            .join(".packs-backup")
            .join(format!("{}-{now}-rolledback", it.name));
        if dest.is_dir() {
            copy_dir(&dest, &cur_backup)?;
            std::fs::remove_dir_all(&dest).map_err(err)?;
        }
        copy_dir(&b, &dest)?;
        let h = skills::hash::dir_hash_uncached(&dest).map_err(|e| e.to_string())?;
        if let Ok(m) = skills::manifest::parse(&dest) {
            let _ = super::bots::skills::record_install(db, &m, &h);
        }
        installed_hash = Some(h);
    }
    db.tx(|tx| {
        tx.execute(
            "INSERT INTO packs_history (id, item_id, version, action, hash, content, backup_path, created_ms) VALUES (?1,?2,?3,'rolled_back_from',?4,?5,NULL,?6)",
            params![new_id(), item_id, it.version, it.hash, it.content, now],
        )
        .map_err(err)?;
        tx.execute(
            "UPDATE packs_items SET hash = ?2, installed_hash = ?3, content = COALESCE(?4, content), version = version + 1, status = 'installed', updated_ms = ?5 WHERE id = ?1",
            params![item_id, phash, installed_hash, pcontent, now],
        )
        .map_err(err)?;
        Ok(())
    })?;
    let _ = pv;
    item_by_id(db, item_id)
}

/// Stored text of an imported rule/context/command/agent (loaded on
/// demand, never dumped into a prompt wholesale).
pub fn text_of(db: &AssistDb, kind: ItemKind, name: &str) -> Result<Option<String>, String> {
    Ok(item(db, kind, name)?.and_then(|i| i.content))
}

pub fn history(db: &AssistDb, item_id: &str) -> Result<Vec<Value>, String> {
    db.with(|c| {
        let mut st = c.prepare("SELECT version, action, hash, created_ms, backup_path IS NOT NULL FROM packs_history WHERE item_id = ?1 ORDER BY created_ms DESC")?;
        let rows = st.query_map(params![item_id], |r| {
            Ok(json!({ "version": r.get::<_, i64>(0)?, "action": r.get::<_, String>(1)?, "hash": r.get::<_, String>(2)?, "at_ms": r.get::<_, i64>(3)?, "has_backup": r.get::<_, bool>(4)? }))
        })?;
        rows.collect()
    })
}

/// Commit of a git checkout, read from `.git` files (no `git` process).
pub fn detect_sha(dir: &Path) -> Option<String> {
    let git = dir.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref: ") else {
        return (head.len() == 40).then(|| head.to_string());
    };
    if let Ok(s) = std::fs::read_to_string(git.join(r)) {
        return Some(s.trim().to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed
        .lines()
        .find(|l| l.ends_with(r))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string)
}

/// `origin` URL of a git checkout, from `.git/config`.
pub fn detect_origin(dir: &Path) -> Option<String> {
    let cfg = std::fs::read_to_string(dir.join(".git").join("config")).ok()?;
    let mut in_origin = false;
    for l in cfg.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_origin = t == "[remote \"origin\"]";
        } else if in_origin {
            if let Some(u) = t.strip_prefix("url = ") {
                return Some(u.trim().to_string());
            }
        }
    }
    None
}
