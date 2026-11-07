use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::chat::now;
use super::model::{is_safe_id, AgentKind};
use super::store::CanvasStore;

pub const LIBRARY_DIR: &str = "roles-library";
const SEEDED_FILE: &str = ".seeded.json";
pub const MAX_PROMPT_BYTES: usize = 128 * 1024;
const MAX_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub builtin: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub updated_at: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Workspace,
    Global,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "workspace" | "current" | "ws" => Some(Self::Workspace),
            "global" | "library" => Some(Self::Global),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleInfo {
    #[serde(flatten)]
    pub role: Role,
    pub scope: Scope,
    pub terminals_in_use: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub agent: Option<AgentKind>,
}

struct Builtin {
    id: &'static str,
    name: &'static str,
    icon: &'static str,
    color: &'static str,
    agent: &'static str,
    summary: &'static str,
    prompt: &'static str,
}

const BUILTINS: &[Builtin] = &[
    Builtin {
        id: "lead",
        name: "Lead",
        icon: "crown",
        color: "#D97757",
        agent: "claude",
        summary: "Coordinator that plans the work and delegates it to the others.",
        prompt: "You are the Lead of a small team of agents on this canvas.\n\n\
- Start with `omniget canvas list` to see who you can reach and what each teammate's role is.\n\
- Turn the user's request into a short plan with clear, independent pieces. Write the plan into a connected note when one exists.\n\
- Delegate each piece to the teammate whose role fits, by name: `omniget canvas ask \"<Name>\" \"<task>\"`. Always say who you are and what \"done\" means for that task.\n\
- Do not implement what a teammate can do; you coordinate, review answers and decide the next step.\n\
- Never resend a question that is still running; use `omniget canvas check \"<Name>\"` to see progress.\n\
- When the work is done, give the user one concise summary: what changed, what was verified, what is left.",
    },
    Builtin {
        id: "coder",
        name: "Coder",
        icon: "hammer",
        color: "#10A37F",
        agent: "codex",
        summary: "Focuses purely on implementation.",
        prompt: "You are the Coder. You implement.\n\n\
- Take the task you receive, read the code around it first, then make the smallest change that fully solves it.\n\
- Follow the style, naming and comment language of the files you touch. No drive-by refactors.\n\
- Build or type-check what you changed before saying you are done.\n\
- When you finish a task someone asked you on the canvas, answer with what you changed (files and one line each) and anything you could not do.\n\
- If the task is ambiguous, ask the person who sent it one precise question instead of guessing.",
    },
    Builtin {
        id: "reviewer",
        name: "Reviewer",
        icon: "eye",
        color: "#8B5CF6",
        agent: "claude",
        summary: "Reviews and critiques code; does not rewrite it.",
        prompt: "You are the Reviewer. You review and critique; you do not rewrite other people's work.\n\n\
- Read the diff (`git diff`, `git diff --staged` or the files you are pointed at) and the code it touches.\n\
- Look for real problems first: wrong behaviour, missed edge cases, races, security issues, broken contracts between modules, missing error handling. Then style.\n\
- Report each finding with file:line, why it is a problem and the concrete fix. Say how sure you are.\n\
- Do not pad the review with praise or nitpicks that do not change anything.\n\
- End with a verdict: ship, ship after the listed fixes, or do not ship.",
    },
    Builtin {
        id: "tester",
        name: "Tester",
        icon: "flask",
        color: "#F59E0B",
        agent: "claude",
        summary: "Writes and runs tests.",
        prompt: "You are the Tester. You write and run tests.\n\n\
- Find how this project tests things (test folders, scripts in package.json / Cargo.toml / Makefile) and use the same tools.\n\
- For each change you are pointed at, write the tests that would have caught a bug in it: the happy path, the edges and the failure path.\n\
- Run the suite and report exact commands and results. A failing test is a finding, not something to delete.\n\
- Never weaken an assertion to make a test pass. If the code is wrong, say so and show the failing case.",
    },
    Builtin {
        id: "unslopifier",
        name: "Unslopifier",
        icon: "sparkles",
        color: "#EC4899",
        agent: "claude",
        summary: "Removes AI slop from code, docs and copy.",
        prompt: "You are the Unslopifier. You remove slop.\n\n\
Slop is anything that reads as generated filler: comments that restate the code, needless abstractions, defensive checks for impossible cases, dead code, verbose logging, buzzwords, marketing adjectives, em-dash chains, bullet lists that should be one sentence, and apologies or hedges in user-facing text.\n\n\
- Work file by file. Keep behaviour identical; this is a cleanup, not a rewrite.\n\
- Prefer deleting to rewording. Keep comments that explain why, drop the ones that explain what.\n\
- In prose and UI copy: short, concrete, plain words; say what the user gets.\n\
- Summarise what you removed per file when you finish.",
    },
    Builtin {
        id: "bug-whisperer",
        name: "Bug Whisperer",
        icon: "bug",
        color: "#EF4444",
        agent: "claude",
        summary: "Finds the root cause of a bug before touching anything.",
        prompt: "You are the Bug Whisperer. You find root causes.\n\n\
- Reproduce first. Write down the exact steps, input and observed vs expected output.\n\
- Form hypotheses and test them one at a time with evidence: logs, a failing test, a minimal script, bisecting commits. Do not change code to \"see if it helps\".\n\
- Keep going until you can explain the whole chain from cause to symptom.\n\
- Then propose (or make, if asked) the smallest fix at the cause, not at the symptom, plus a test that fails before and passes after.\n\
- Report: symptom, root cause, evidence, fix, how it was verified.",
    },
    Builtin {
        id: "vibe-coder",
        name: "Vibe Coder",
        icon: "music",
        color: "#06B6D4",
        agent: "claude",
        summary: "Fast prototyping: working first, polish later.",
        prompt: "You are the Vibe Coder. You get things working fast.\n\n\
- Bias to action: build the simplest thing that works end to end, run it, show it.\n\
- Use what the project already has before adding dependencies.\n\
- Leave short TODO notes where you cut corners, so someone can harden it later.\n\
- Keep the user in the loop with small, visible steps instead of one big reveal.\n\
- Still never break what already works: run it before you say it runs.",
    },
    Builtin {
        id: "ship-it",
        name: "Ship It",
        icon: "rocket",
        color: "#22C55E",
        agent: "claude",
        summary: "Gets a change ready to merge and release.",
        prompt: "You are Ship It. You get changes out the door safely.\n\n\
- Check that the branch builds, the linters and the test suite pass, exactly as CI runs them. Fix what is red.\n\
- Make sure the change is complete: docs, changelog or release notes, version bumps, migrations, feature flags, whatever this project expects.\n\
- Write the commit message and PR description in the project's convention, for the people who will read them.\n\
- Do not push, tag or publish unless the user explicitly asks; prepare everything and say what the final command is.",
    },
    Builtin {
        id: "refactor-gremlin",
        name: "Refactor Gremlin",
        icon: "shuffle",
        color: "#A855F7",
        agent: "codex",
        summary: "Restructures code without changing behaviour.",
        prompt: "You are the Refactor Gremlin. You restructure code without changing behaviour.\n\n\
- Before touching anything, make sure there are tests (or a quick way to check) covering what you will move. Add them if missing.\n\
- Refactor in small steps that each compile and pass: rename, extract, inline, move, delete duplication.\n\
- Aim for deeper modules with smaller interfaces, fewer concepts and less code, not more layers.\n\
- Never mix a refactor with a behaviour change. If you find a bug, report it separately.\n\
- Finish with a before/after summary of the structure.",
    },
    Builtin {
        id: "docs-goblin",
        name: "Docs Goblin",
        icon: "book",
        color: "#EAB308",
        agent: "claude",
        summary: "Writes and fixes documentation that matches the code.",
        prompt: "You are the Docs Goblin. You write documentation that matches the code as it is today.\n\n\
- Read the code before you write about it; every claim must be true now. Remove docs for things that no longer exist.\n\
- Write for the reader of that doc: a README for a newcomer, API docs for a caller, comments for the next maintainer.\n\
- Short sentences, concrete examples that run, no marketing words.\n\
- Keep the existing structure, language and tone of the project's docs.",
    },
    Builtin {
        id: "git-wizard",
        name: "Git Wizard",
        icon: "git-branch",
        color: "#F97316",
        agent: "claude",
        summary: "History, branches, rebases and conflicts.",
        prompt: "You are the Git Wizard. You handle history, branches, rebases and conflicts.\n\n\
- Always look before you act: `git status`, `git log --oneline --graph`, `git stash list`.\n\
- Resolve conflicts by understanding both sides; keep both intents when they are compatible, and explain any choice you had to make.\n\
- Prefer safe operations. Before anything that rewrites history or discards work (reset --hard, push --force, clean, branch -D), stop and ask the user, and say how to undo it.\n\
- Write commit messages in the project's convention.",
    },
    Builtin {
        id: "perf-nerd",
        name: "Perf Nerd",
        icon: "gauge",
        color: "#3B82F6",
        agent: "claude",
        summary: "Measures first, then makes it faster.",
        prompt: "You are the Perf Nerd. You make things faster, with numbers.\n\n\
- Measure before changing anything: a benchmark, a profile, timings with realistic input. Write down the baseline.\n\
- Find where the time or memory really goes; optimise the biggest cost first.\n\
- Change one thing at a time and measure again. Keep only changes that win and do not hurt readability for nothing.\n\
- Report before/after numbers, how they were measured, and the trade-offs.",
    },
];

pub fn builtins() -> Vec<Role> {
    let now = now();
    BUILTINS
        .iter()
        .map(|b| Role {
            id: b.id.into(),
            name: b.name.into(),
            prompt: b.prompt.into(),
            summary: Some(b.summary.into()),
            icon: Some(b.icon.into()),
            color: Some(b.color.into()),
            agent: Some(AgentKind::parse(b.agent)),
            workspace_id: None,
            builtin: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .collect()
}

pub fn library_dir(store: &CanvasStore) -> PathBuf {
    store.root().join(LIBRARY_DIR)
}

fn scope_dir(store: &CanvasStore, ws: Option<&str>) -> PathBuf {
    match ws {
        Some(ws) => store.roles_dir(ws),
        None => library_dir(store),
    }
}

pub fn role_dir(store: &CanvasStore, ws: Option<&str>, id: &str) -> Result<PathBuf, String> {
    if !is_safe_id(id) {
        return Err("CANVAS_INVALID: role id".into());
    }
    Ok(scope_dir(store, ws).join(id))
}

pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "role".into()
    } else {
        out
    }
}

fn safe_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            Err(format!("CANVAS_IO: refusing symlink {}", path.display()))
        }
        Ok(m) if !m.is_file() => Err(format!("CANVAS_IO: not a regular file {}", path.display())),
        _ => Ok(()),
    }
}

fn write_managed(path: &Path, body: &str) -> Result<(), String> {
    if body.len() > MAX_PROMPT_BYTES * 2 {
        return Err("CANVAS_INVALID: role file too large".into());
    }
    if let Some(dir) = path.parent() {
        if let Ok(m) = fs::symlink_metadata(dir) {
            if m.file_type().is_symlink() {
                return Err(format!(
                    "CANVAS_IO: refusing symlinked folder {}",
                    dir.display()
                ));
            }
        }
        fs::create_dir_all(dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    }
    safe_target(path)?;
    if fs::read_to_string(path).ok().as_deref() == Some(body) {
        return Ok(());
    }
    let tmp = path.with_extension("tmp~");
    fs::write(&tmp, body).map_err(|e| format!("CANVAS_IO: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("CANVAS_IO: {e}"))
}

fn read_role_file(dir: &Path) -> Option<Role> {
    let file = dir.join("role.json");
    safe_target(&file).ok()?;
    let v: Value = serde_json::from_slice(&fs::read(&file).ok()?).ok()?;
    let mut role: Role = serde_json::from_value(v).ok()?;
    let asked = dir.file_name()?.to_str()?;
    let folder = dir
        .parent()
        .and_then(|p| fs::read_dir(p).ok())
        .and_then(|rd| {
            let names: Vec<String> = rd
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            names
                .iter()
                .find(|n| n.as_str() == asked)
                .or_else(|| names.iter().find(|n| n.eq_ignore_ascii_case(asked)))
                .cloned()
        })
        .unwrap_or_else(|| asked.to_string());
    if role.id != folder {
        role.id = folder;
    }
    if role.prompt.is_empty() {
        for f in ["prompt.md", "AGENTS.md", "CLAUDE.md"] {
            if let Ok(t) = fs::read_to_string(dir.join(f)) {
                role.prompt = t;
                break;
            }
        }
    }
    Some(role)
}

fn validate(input: &RoleInput) -> Result<(), String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("CANVAS_INVALID: role name is required".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err("CANVAS_INVALID: role name too long".into());
    }
    if input.prompt.len() > MAX_PROMPT_BYTES {
        return Err("CANVAS_INVALID: role prompt too large".into());
    }
    if let Some(id) = &input.id {
        if !is_safe_id(id) {
            return Err("CANVAS_INVALID: role id".into());
        }
    }
    Ok(())
}

fn seeded_ids(dir: &Path) -> Option<Vec<String>> {
    serde_json::from_slice(&fs::read(dir.join(SEEDED_FILE)).ok()?).ok()
}

pub fn ensure_library(store: &CanvasStore) {
    let dir = library_dir(store);
    let mut seeded = seeded_ids(&dir).unwrap_or_default();
    let mut changed = false;
    for role in builtins() {
        if seeded.iter().any(|s| s == &role.id) {
            continue;
        }
        let rd = dir.join(&role.id);
        if !rd.join("role.json").exists() {
            if let Err(e) = write_role_files(&rd, &role, None) {
                tracing::warn!("[canvas.roles] seeding {} failed: {e}", role.id);
                continue;
            }
        }
        seeded.push(role.id.clone());
        changed = true;
    }
    if changed {
        let body = serde_json::to_string_pretty(&seeded).unwrap_or_default();
        let _ = write_managed(&dir.join(SEEDED_FILE), &body);
    }
}

fn list_dir(dir: &Path) -> Vec<Role> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Role> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| e.file_name().to_str().is_some_and(is_safe_id))
        .filter_map(|e| read_role_file(&e.path()))
        .collect();
    out.sort_by(|a, b| {
        b.builtin
            .cmp(&a.builtin)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

pub fn list_workspace(store: &CanvasStore, ws: &str) -> Vec<Role> {
    list_dir(&store.roles_dir(ws))
        .into_iter()
        .map(|mut r| {
            r.workspace_id = Some(ws.to_string());
            r
        })
        .collect()
}

pub fn list_library(store: &CanvasStore) -> Vec<Role> {
    ensure_library(store);
    list_dir(&library_dir(store))
        .into_iter()
        .map(|mut r| {
            r.workspace_id = None;
            r
        })
        .collect()
}

pub fn usage(store: &CanvasStore, ws: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(meta) = store.workspace(ws) else {
        return out;
    };
    for floor in meta.floors.iter().map(|f| f.id.clone()) {
        let _ = store.read(ws, &floor, |d| {
            for n in d.nodes.iter().filter(|n| n.is_terminal()) {
                if let Some(r) = n.data_str("roleId") {
                    out.entry(r.to_string()).or_default().push(n.id.clone());
                }
            }
        });
    }
    out
}

pub fn list(store: &CanvasStore, ws: &str) -> Vec<RoleInfo> {
    let used = usage(store, ws);
    let wsr = list_workspace(store, ws);
    let mut out: Vec<RoleInfo> = wsr
        .iter()
        .map(|r| RoleInfo {
            terminals_in_use: used.get(&r.id).cloned().unwrap_or_default(),
            role: r.clone(),
            scope: Scope::Workspace,
        })
        .collect();
    for r in list_library(store) {
        if wsr.iter().any(|w| w.id == r.id) {
            continue;
        }
        out.push(RoleInfo {
            terminals_in_use: used.get(&r.id).cloned().unwrap_or_default(),
            role: r,
            scope: Scope::Global,
        });
    }
    out
}

pub fn get(store: &CanvasStore, ws: &str, id: &str) -> Option<(Role, Scope)> {
    if !is_safe_id(id) {
        return None;
    }
    if let Some(mut r) = read_role_file(&store.roles_dir(ws).join(id)) {
        r.workspace_id = Some(ws.to_string());
        return Some((r, Scope::Workspace));
    }
    ensure_library(store);
    read_role_file(&library_dir(store).join(id)).map(|mut r| {
        r.workspace_id = None;
        (r, Scope::Global)
    })
}

pub fn find(store: &CanvasStore, ws: &str, name_or_id: &str) -> Option<(Role, Scope)> {
    let key = name_or_id.trim();
    if key.is_empty() {
        return None;
    }
    if let Some(hit) = get(store, ws, key) {
        return Some(hit);
    }
    let lower = key.to_lowercase();
    list(store, ws)
        .into_iter()
        .find(|i| i.role.name.to_lowercase() == lower)
        .map(|i| (i.role, i.scope))
}

fn name_taken(store: &CanvasStore, ws: Option<&str>, name: &str, except: Option<&str>) -> bool {
    let lower = name.trim().to_lowercase();
    let roles = match ws {
        Some(ws) => list_workspace(store, ws),
        None => list_library(store),
    };
    roles
        .iter()
        .any(|r| r.name.to_lowercase() == lower && Some(r.id.as_str()) != except)
}

fn free_id(store: &CanvasStore, ws: Option<&str>, name: &str) -> String {
    let base = slug(name);
    let mut dirs = vec![scope_dir(store, ws), library_dir(store)];
    if ws.is_none() {
        dirs.extend(
            store
                .workspaces()
                .iter()
                .map(|w| store.roles_dir(&w.workspace.id)),
        );
    }
    let builtin: Vec<String> = builtins().into_iter().map(|b| b.id).collect();
    let taken =
        |id: &str| builtin.iter().any(|b| b == id) || dirs.iter().any(|d| d.join(id).exists());
    if !taken(&base) {
        return base;
    }
    for n in 2..1000 {
        let id = format!("{base}-{n}");
        if !taken(&id) {
            return id;
        }
    }
    format!("{base}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

pub fn save(store: &CanvasStore, ws: Option<&str>, input: RoleInput) -> Result<Role, String> {
    validate(&input)?;
    if ws.is_none() {
        ensure_library(store);
    }
    let existing = input
        .id
        .as_deref()
        .and_then(|id| read_role_file(&scope_dir(store, ws).join(id)));
    let id = match (&existing, &input.id) {
        (Some(r), _) => r.id.clone(),
        (None, Some(id)) => id.clone(),
        (None, None) => free_id(store, ws, &input.name),
    };
    if name_taken(store, ws, &input.name, Some(&id)) {
        return Err(format!(
            "CANVAS_CONFLICT: a role named '{}' already exists",
            input.name.trim()
        ));
    }
    let now = now();
    let role = Role {
        id: id.clone(),
        name: input.name.trim().to_string(),
        prompt: input.prompt,
        summary: input
            .summary
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        icon: input.icon.filter(|s| !s.trim().is_empty()),
        color: input.color.filter(|s| !s.trim().is_empty()),
        agent: input.agent,
        workspace_id: ws.map(str::to_string),
        builtin: existing.as_ref().is_some_and(|r| r.builtin),
        created_at: existing
            .as_ref()
            .map(|r| r.created_at.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    };
    let dir = role_dir(store, ws, &id)?;
    let project = ws
        .and_then(|w| store.workspace(w).ok())
        .and_then(|w| w.working_directory);
    write_role_files(&dir, &role, project.as_deref())?;
    Ok(role)
}

pub fn delete(store: &CanvasStore, ws: Option<&str>, id: &str) -> Result<(), String> {
    let dir = role_dir(store, ws, id)?;
    if !dir.exists() {
        return Err(format!("CANVAS_NOT_FOUND: role {id}"));
    }
    if let Ok(m) = fs::symlink_metadata(&dir) {
        if m.file_type().is_symlink() {
            return Err("CANVAS_IO: refusing symlinked role folder".into());
        }
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))
}

pub fn adopt(store: &CanvasStore, ws: &str, id: &str) -> Result<Role, String> {
    if let Some(mut r) = read_role_file(&store.roles_dir(ws).join(id)) {
        r.workspace_id = Some(ws.to_string());
        return Ok(r);
    }
    let (lib, _) = get(store, ws, id).ok_or_else(|| format!("CANVAS_NOT_FOUND: role {id}"))?;
    let mut role = lib;
    role.workspace_id = Some(ws.to_string());
    if name_taken(store, Some(ws), &role.name, Some(&role.id)) {
        role.name = format!("{} (global)", role.name);
    }
    let dir = role_dir(store, Some(ws), &role.id)?;
    let project = store.workspace(ws).ok().and_then(|w| w.working_directory);
    write_role_files(&dir, &role, project.as_deref())?;
    Ok(role)
}

pub fn set_scope(store: &CanvasStore, ws: &str, id: &str, to: Scope) -> Result<Role, String> {
    let (role, from) = get(store, ws, id).ok_or_else(|| format!("CANVAS_NOT_FOUND: role {id}"))?;
    if from == to {
        return Ok(role);
    }
    let target_ws = match to {
        Scope::Workspace => Some(ws),
        Scope::Global => None,
    };
    let saved = save(
        store,
        target_ws,
        RoleInput {
            id: Some(role.id.clone()),
            name: role.name.clone(),
            prompt: role.prompt.clone(),
            summary: role.summary.clone(),
            icon: role.icon.clone(),
            color: role.color.clone(),
            agent: role.agent.clone(),
        },
    )?;
    let _ = delete(
        store,
        match from {
            Scope::Workspace => Some(ws),
            Scope::Global => None,
        },
        id,
    );
    Ok(saved)
}

pub fn discover(dir: &Path) -> Vec<Role> {
    let mut out = Vec::new();
    for sub in [".omniget/roles", ".maestri/roles"] {
        for mut r in list_dir(&dir.join(sub)) {
            if out.iter().any(|o: &Role| o.id == r.id) {
                continue;
            }
            r.workspace_id = None;
            r.builtin = false;
            out.push(r);
        }
    }
    out
}

pub fn import_discovered(
    store: &CanvasStore,
    ws: &str,
    dir: &Path,
    names: &[String],
) -> Result<Vec<(Role, bool)>, String> {
    let wanted: Vec<String> = names
        .iter()
        .map(|n| n.trim().to_lowercase())
        .filter(|n| !n.is_empty())
        .collect();
    let found: Vec<Role> = discover(dir)
        .into_iter()
        .filter(|r| {
            wanted.is_empty()
                || wanted.contains(&r.name.to_lowercase())
                || wanted.contains(&r.id.to_lowercase())
        })
        .collect();
    if found.is_empty() {
        return Err(if wanted.is_empty() {
            format!(
                "CANVAS_NOT_FOUND: no role.json under {0}/.omniget/roles or {0}/.maestri/roles",
                dir.display()
            )
        } else {
            format!(
                "CANVAS_NOT_FOUND: no discovered role named {}",
                names.join(", ")
            )
        });
    }
    let mine = list_workspace(store, ws);
    let mut out = Vec::new();
    for r in found {
        let same = mine
            .iter()
            .find(|m| m.name.to_lowercase() == r.name.to_lowercase());
        let input = RoleInput {
            id: same.map(|m| m.id.clone()),
            name: r.name.clone(),
            prompt: r.prompt.clone(),
            summary: r.summary.clone(),
            icon: r.icon.clone(),
            color: r.color.clone(),
            agent: r.agent.clone(),
        };
        let saved = save(store, Some(ws), input)?;
        out.push((saved, same.is_some()));
    }
    Ok(out)
}

pub fn instructions(role: &Role, project: Option<&str>, redirect: bool) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "<your_assigned_role name=\"{}\">\n",
        role.name.replace('"', "'")
    ));
    s.push_str(role.prompt.trim());
    s.push_str("\n</your_assigned_role>\n");
    if let Some(p) = project.filter(|p| !p.trim().is_empty()) {
        s.push_str("<working_directory>\n");
        if redirect {
            s.push_str(
                "IMPORTANT: You were started in this directory to receive the above role assignment. \
The actual project you should be working on is located at:\n",
            );
            s.push_str(p);
            s.push_str("\nWork there: cd into it or use absolute paths, and never create project files in this role folder.\n");
        } else {
            s.push_str("The project you are working on is located at:\n");
            s.push_str(p);
            s.push('\n');
        }
        s.push_str("</working_directory>\n");
    }
    s.push_str("<omniget_canvas>\n");
    s.push_str(&super::adapters::agents_md_snippet());
    s.push_str("\n</omniget_canvas>\n");
    s
}

fn write_role_files(dir: &Path, role: &Role, project: Option<&str>) -> Result<(), String> {
    let mut sidecar = role.clone();
    sidecar.workspace_id = None;
    let json = serde_json::to_string_pretty(&sidecar).map_err(|e| format!("CANVAS_IO: {e}"))?;
    write_managed(&dir.join("role.json"), &json)?;
    write_instruction_files(dir, role, project)
}

fn write_instruction_files(dir: &Path, role: &Role, project: Option<&str>) -> Result<(), String> {
    let text = instructions(role, project, project.is_some());
    write_managed(&dir.join("CLAUDE.md"), &text)?;
    write_managed(&dir.join("AGENTS.md"), &text)?;
    if let Some(p) = project {
        let settings = json!({
            "context": { "fileName": ["AGENTS.md"], "includeDirectories": [p] }
        });
        write_managed(
            &dir.join(".gemini").join("settings.json"),
            &serde_json::to_string_pretty(&settings).unwrap_or_default(),
        )?;
        let oc = json!({
            "$schema": "https://opencode.ai/config.json",
            "instructions": [dir.join("AGENTS.md").to_string_lossy()],
        });
        write_managed(
            &dir.join("opencode.json"),
            &serde_json::to_string_pretty(&oc).unwrap_or_default(),
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleLaunch {
    pub role_id: String,
    pub role_name: String,
    pub cwd: Option<String>,
    pub project_dir: Option<String>,
    pub role_dir: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub mode: &'static str,
}

fn launch_dir(role_dir: &Path, project: Option<&str>, ws_dir: Option<&str>) -> PathBuf {
    match project {
        Some(p) if Some(p) != ws_dir => {
            let h = Sha256::digest(p.as_bytes());
            let hex: String = h.iter().take(6).map(|b| format!("{b:02x}")).collect();
            role_dir.join("at").join(hex)
        }
        _ => role_dir.to_path_buf(),
    }
}

pub fn launch_plan(
    store: &CanvasStore,
    ws: &str,
    role_id: &str,
    agent: &AgentKind,
    project: Option<&str>,
) -> Result<RoleLaunch, String> {
    let role = adopt(store, ws, role_id)?;
    let ws_dir = store.workspace(ws).ok().and_then(|w| w.working_directory);
    let project = project
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| ws_dir.clone());
    let rdir = role_dir(store, Some(ws), &role.id)?;
    let dir = launch_dir(&rdir, project.as_deref(), ws_dir.as_deref());
    write_instruction_files(&dir, &role, project.as_deref())?;
    let dir_s = dir.to_string_lossy().into_owned();

    let mut env = BTreeMap::new();
    env.insert("OMNIGET_ROLE_ID".into(), role.id.clone());
    env.insert("OMNIGET_ROLE_NAME".into(), role.name.clone());
    env.insert("OMNIGET_ROLE_DIR".into(), dir_s.clone());
    if let Some(p) = &project {
        env.insert("OMNIGET_PROJECT_DIR".into(), p.clone());
    }

    let mut plan = RoleLaunch {
        role_id: role.id.clone(),
        role_name: role.name.clone(),
        cwd: project.clone(),
        project_dir: project.clone(),
        role_dir: dir_s.clone(),
        args: Vec::new(),
        env,
        initial_prompt: None,
        system_prompt: None,
        mode: "env",
    };
    match agent {
        AgentKind::Claude => {
            plan.args = vec![
                "--append-system-prompt".into(),
                instructions(&role, project.as_deref(), false),
            ];
            plan.mode = "append-system-prompt";
        }
        AgentKind::Codex => {
            let text = instructions(&role, project.as_deref(), false);
            plan.args = vec![
                "-c".into(),
                format!(
                    "developer_instructions={}",
                    serde_json::to_string(&text).unwrap_or_default()
                ),
            ];
            plan.mode = "developer-instructions";
        }
        AgentKind::Gemini => {
            plan.cwd = Some(dir_s.clone());
            plan.mode = "role-dir";
        }
        AgentKind::OpenCode => {
            if project.is_some() {
                plan.env.insert(
                    "OPENCODE_CONFIG".into(),
                    dir.join("opencode.json").to_string_lossy().into_owned(),
                );
                plan.mode = "config-instructions";
            } else {
                plan.cwd = Some(dir_s.clone());
                plan.mode = "role-dir";
            }
        }
        AgentKind::Ollama => {
            plan.initial_prompt = Some(instructions(&role, project.as_deref(), false));
            plan.mode = "first-prompt";
        }
        AgentKind::Native => {
            plan.system_prompt = Some(instructions(&role, project.as_deref(), false));
            plan.mode = "system-prompt";
        }
        AgentKind::Shell | AgentKind::Unknown(_) => {}
    }
    Ok(plan)
}
