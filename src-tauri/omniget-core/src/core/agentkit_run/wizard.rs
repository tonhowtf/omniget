//! Project wizard (`/llm/catalog/wizard`): stack → the catalog's 14 project
//! templates → selectable pieces (commands, agents, settings, hooks, MCP
//! servers, template guidance) + a generated `AGENTS.md`, installed through the
//! agentkit plan into every tool the user picks. Unlike cct (estudo 75 01
//! §2.11) the selection is what gets installed: every piece is its own
//! component, and placeholder MCP servers (`node path/to/...`, `ruby -e`) are
//! shown but can not be picked.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::stack::{self, StackReport};
use crate::core::agentkit::model::{Component, ComponentKind, SourceRef};
use crate::core::agentkit::parse::{self, RawFiles};
use crate::core::catalog::{self as cat, model::CatalogItem};

pub const ERR_WIZARD: &str = "ERR_AGENTKIT_WIZARD";

/// Templates that apply to a stack, most general first.
pub fn templates_for(r: &StackReport) -> Vec<&'static str> {
    let mut t = vec!["cct:templates/common"];
    let js = r.has_language("javascript") || r.has_language("typescript");
    if js {
        t.push("cct:templates/javascript-typescript");
        if r.has_framework("react") || r.has_framework("next") {
            t.push("cct:templates/javascript-typescript/react-app");
        }
        if r.has_framework("vue") || r.has_framework("nuxt") {
            t.push("cct:templates/javascript-typescript/vue-app");
        }
        if r.has_framework("angular") {
            t.push("cct:templates/javascript-typescript/angular-app");
        }
        if ["express", "fastify", "koa", "nestjs"]
            .iter()
            .any(|f| r.has_framework(f))
        {
            t.push("cct:templates/javascript-typescript/node-api");
        }
    }
    if r.has_language("python") {
        t.push("cct:templates/python");
        for (fw, id) in [
            ("django", "cct:templates/python/django-app"),
            ("fastapi", "cct:templates/python/fastapi-app"),
            ("flask", "cct:templates/python/flask-app"),
        ] {
            if r.has_framework(fw) {
                t.push(id);
            }
        }
    }
    if r.has_language("ruby") {
        t.push("cct:templates/ruby");
        if r.has_framework("rails") {
            t.push("cct:templates/ruby/rails-app");
        }
    }
    if r.has_language("rust") {
        t.push("cct:templates/rust");
    }
    if r.has_language("go") {
        t.push("cct:templates/go");
    }
    t
}

/// Every template id of the catalog the wizard knows (the 14 of cct).
pub const ALL_TEMPLATES: &[&str] = &[
    "cct:templates/common",
    "cct:templates/javascript-typescript",
    "cct:templates/javascript-typescript/react-app",
    "cct:templates/javascript-typescript/vue-app",
    "cct:templates/javascript-typescript/angular-app",
    "cct:templates/javascript-typescript/node-api",
    "cct:templates/python",
    "cct:templates/python/django-app",
    "cct:templates/python/fastapi-app",
    "cct:templates/python/flask-app",
    "cct:templates/ruby",
    "cct:templates/ruby/rails-app",
    "cct:templates/rust",
    "cct:templates/go",
];

/// One selectable thing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    /// Stable key (`<template>/<kind>/<name>`).
    pub id: String,
    /// `rule | command | agent | setting | hook | mcp`.
    pub kind: String,
    pub name: String,
    pub description: String,
    pub template: String,
    /// Pre-checked in the UI.
    pub selected: bool,
    /// `false` = shown for honesty but can not be installed (placeholder MCP).
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Why it is (not) suggested / what it changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip)]
    pub component: Option<Component>,
}

/// What the wizard shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project: PathBuf,
    pub stack: StackReport,
    pub templates: Vec<String>,
    pub pieces: Vec<Piece>,
    /// Generated AGENTS.md (the `agents-md` piece; editable before install).
    pub agents_md: String,
    pub warnings: Vec<String>,
}

fn sessions() -> &'static Mutex<HashMap<String, Session>> {
    static S: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn session(id: &str) -> Option<Session> {
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
}

fn template_short(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

fn source_of(item: &CatalogItem, path: &str) -> SourceRef {
    SourceRef {
        id: item.source.id.clone(),
        repo: item.source.repo.clone(),
        commit: item.source.commit.clone(),
        path: Some(format!("{}/{path}", item.source.dir)),
        url: None,
    }
}

fn finish(mut c: Component, id: String, item: &CatalogItem, path: &str) -> Component {
    c.id = id;
    c.category = Some(template_short(&item.id).to_string());
    c.source = Some(source_of(item, path));
    c.license = item.license.clone().or(Some("MIT".into()));
    c
}

fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Placeholder MCP entries of the templates (they do not run anywhere).
fn placeholder_reason(rec: &Value) -> Option<String> {
    let cmd = rec["command"].as_str().unwrap_or("");
    let args: Vec<&str> = rec["args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_str())
        .collect();
    let line = format!("{cmd} {}", args.join(" "));
    if line.contains("path/to") || line.contains("/path/") {
        return Some("placeholder path in the template (`path/to/...`)".into());
    }
    if cmd == "ruby" && args.first() == Some(&"-e") {
        return Some("stub that only prints JSON (`ruby -e`)".into());
    }
    if cmd.is_empty() && rec["url"].as_str().unwrap_or("").is_empty() {
        return Some("no command or URL".into());
    }
    None
}

fn needs_secret(rec: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(env) = rec["env"].as_object() {
        for (k, v) in env {
            let v = v.as_str().unwrap_or("");
            if v.is_empty()
                || v.contains("your")
                || v.contains('<')
                || v.contains("${")
                || k.contains("KEY")
                || k.contains("TOKEN")
            {
                out.push(k.clone());
            }
        }
    }
    out
}

/// Pieces of one template item from its fetched files.
pub fn pieces_of(item: &CatalogItem, files: &RawFiles) -> (Vec<Piece>, Vec<String>) {
    let mut pieces = Vec::new();
    let mut warnings = Vec::new();
    let tpl = template_short(&item.id).to_string();
    let base_id = |kind: &str, name: &str| format!("wizard:{tpl}/{kind}/{name}");
    for (path, bytes) in files {
        let fname = file_name(path);
        let one =
            |name: &str| -> RawFiles { [(name.to_string(), bytes.clone())].into_iter().collect() };
        if path.contains(".claude/commands/") && fname.ends_with(".md") {
            match parse::parse_raw(ComponentKind::Command, fname, &one(fname)) {
                Ok(c) => {
                    let name = c.name.clone();
                    let desc = c.description.clone();
                    pieces.push(Piece {
                        id: base_id("command", &name),
                        kind: "command".into(),
                        name: name.clone(),
                        description: desc,
                        template: item.id.clone(),
                        selected: true,
                        available: true,
                        reason: None,
                        notes: vec![],
                        component: Some(finish(c, base_id("command", &name), item, path)),
                    });
                }
                Err(e) => warnings.push(format!("{path}: {}", e.message)),
            }
        } else if (path.starts_with("agents/") || path.contains(".claude/agents/"))
            && fname.ends_with(".md")
        {
            match parse::parse_raw(ComponentKind::Agent, fname, &one(fname)) {
                Ok(c) => {
                    let name = c.name.clone();
                    pieces.push(Piece {
                        id: base_id("agent", &name),
                        kind: "agent".into(),
                        name: name.clone(),
                        description: c.description.clone(),
                        template: item.id.clone(),
                        selected: true,
                        available: true,
                        reason: None,
                        notes: vec![],
                        component: Some(finish(c, base_id("agent", &name), item, path)),
                    });
                }
                Err(e) => warnings.push(format!("{path}: {}", e.message)),
            }
        } else if path.ends_with(".claude/settings.json") {
            let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
                warnings.push(format!("{path}: not JSON"));
                continue;
            };
            let mut values: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
            let hooks = values.remove("hooks");
            let mut notes = Vec::new();
            // cct ships `defaultMode: "allowEdits"`, which Claude does not know.
            if let Some(Value::Object(perms)) = values.get_mut("permissions") {
                if perms.get("defaultMode").and_then(|m| m.as_str()) == Some("allowEdits") {
                    perms.insert("defaultMode".into(), json!("acceptEdits"));
                    notes.push(
                        "defaultMode `allowEdits` (unknown to Claude) written as `acceptEdits`"
                            .into(),
                    );
                }
                if let Some(Value::Array(allow)) = perms.get_mut("allow") {
                    let before = allow.len();
                    allow.retain(|a| a.as_str() != Some("Bash"));
                    if allow.len() != before {
                        notes.push("bare `Bash` (every shell command) left out of allow; the prefixed ones stay".into());
                    }
                }
            }
            if !values.is_empty() {
                let text = serde_json::to_string_pretty(&Value::Object(values.clone()))
                    .unwrap_or_default();
                let fname = format!("{tpl}-settings.json");
                let files: RawFiles = [(fname.clone(), text.into_bytes())].into_iter().collect();
                match parse::parse_raw(ComponentKind::Setting, &fname, &files) {
                    Ok(mut c) => {
                        c.name = format!("{tpl}-settings");
                        let allow: Vec<String> = values["permissions"]["allow"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|a| a.as_str().map(str::to_string))
                            .collect();
                        pieces.push(Piece {
                            id: base_id("setting", "settings"),
                            kind: "setting".into(),
                            name: format!("{tpl} permissions and environment"),
                            description: format!("allow: {}", allow.join(", ")),
                            template: item.id.clone(),
                            selected: false,
                            available: true,
                            reason: None,
                            notes,
                            component: Some(finish(c, base_id("setting", "settings"), item, path)),
                        });
                    }
                    Err(e) => warnings.push(format!("{path}: {}", e.message)),
                }
            }
            // Each hook group is its own piece (cct wrote them all, whatever was picked).
            if let Some(Value::Object(events)) = hooks {
                for (event, groups) in events {
                    for (gi, group) in groups
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .enumerate()
                    {
                        let matcher = group["matcher"].as_str().unwrap_or("").to_string();
                        let doc = json!({ "description": format!("{event} {matcher} hook from the {tpl} template"), "hooks": { event.clone(): [group.clone()] } });
                        let name = parse::sanitize_name(&format!(
                            "{tpl}-{event}-{}{}",
                            if matcher.is_empty() {
                                "all".to_string()
                            } else {
                                matcher.replace('|', "-")
                            },
                            if gi > 0 {
                                format!("-{gi}")
                            } else {
                                String::new()
                            }
                        ));
                        let fname = format!("{name}.json");
                        let files: RawFiles = [(
                            fname.clone(),
                            serde_json::to_vec_pretty(&doc).unwrap_or_default(),
                        )]
                        .into_iter()
                        .collect();
                        let cmd: String = group["hooks"][0]["command"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(160)
                            .collect();
                        let uses_jq = cmd.contains("jq ");
                        match parse::parse_raw(ComponentKind::Hook, &fname, &files) {
                            Ok(c) => pieces.push(Piece {
                                id: base_id("hook", &name),
                                kind: "hook".into(),
                                name: format!(
                                    "{event}{}",
                                    if matcher.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" · {matcher}")
                                    }
                                ),
                                description: cmd,
                                template: item.id.clone(),
                                selected: false,
                                available: true,
                                reason: None,
                                notes: if uses_jq {
                                    vec!["needs `jq` on the PATH".into()]
                                } else {
                                    vec![]
                                },
                                component: Some(finish(c, base_id("hook", &name), item, path)),
                            }),
                            Err(e) => warnings.push(format!("{path} {event}: {}", e.message)),
                        }
                    }
                }
            }
        } else if fname == ".mcp.json" {
            let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
                warnings.push(format!("{path}: not JSON"));
                continue;
            };
            for (name, rec) in v["mcpServers"].as_object().cloned().unwrap_or_default() {
                let placeholder = placeholder_reason(&rec);
                let secrets = needs_secret(&rec);
                let mut clean = rec.clone();
                if let Some(o) = clean.as_object_mut() {
                    o.remove("description");
                }
                let doc = json!({ "mcpServers": { name.clone(): clean } });
                let fname = format!("{}.json", parse::sanitize_name(&name));
                let files: RawFiles = [(
                    fname.clone(),
                    serde_json::to_vec_pretty(&doc).unwrap_or_default(),
                )]
                .into_iter()
                .collect();
                let comp = parse::parse_raw(ComponentKind::Mcp, &fname, &files)
                    .ok()
                    .map(|c| finish(c, base_id("mcp", &name), item, path));
                pieces.push(Piece {
                    id: base_id("mcp", &name),
                    kind: "mcp".into(),
                    name: name.clone(),
                    description: rec["description"].as_str().unwrap_or("").to_string(),
                    template: item.id.clone(),
                    selected: false,
                    available: placeholder.is_none() && comp.is_some(),
                    reason: placeholder,
                    notes: if secrets.is_empty() {
                        vec![]
                    } else {
                        vec![format!("needs {}", secrets.join(", "))]
                    },
                    component: comp,
                });
            }
        } else if fname == "CLAUDE.md" {
            let text = String::from_utf8_lossy(bytes).to_string();
            let lines = text.lines().count();
            let files: RawFiles = [(format!("{tpl}-guidance.md"), bytes.clone())]
                .into_iter()
                .collect();
            if let Ok(c) =
                parse::parse_raw(ComponentKind::Rule, &format!("{tpl}-guidance.md"), &files)
            {
                pieces.push(Piece {
                    id: base_id("rule", "guidance"),
                    kind: "rule".into(),
                    name: format!("{tpl} guidance"),
                    description: format!("The template's CLAUDE.md ({lines} lines), as a block of AGENTS.md / CLAUDE.md"),
                    template: item.id.clone(),
                    selected: false,
                    available: true,
                    reason: None,
                    notes: vec![],
                    component: Some(finish(c, base_id("rule", "guidance"), item, path)),
                });
            }
        }
    }
    (pieces, warnings)
}

/// A later template's piece with the same kind and name replaces an earlier
/// one (node-api's `api-endpoint` over the generic one; the MCP servers that
/// the common template shares with the language ones).
fn dedupe(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for p in pieces {
        if matches!(p.kind.as_str(), "command" | "agent" | "mcp") {
            if let Some(i) = out
                .iter()
                .position(|o| o.kind == p.kind && o.name == p.name)
            {
                out[i] = p;
                continue;
            }
        }
        out.push(p);
    }
    out
}

/// Generated AGENTS.md: facts of the project, no generic essay.
pub fn agents_md(r: &StackReport) -> String {
    let mut s = String::new();
    let title = r
        .name
        .clone()
        .or_else(|| r.root.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "Project".into());
    s.push_str(&format!("# {title}\n\n"));
    if let Some(d) = r.description.as_deref().filter(|d| !d.trim().is_empty()) {
        s.push_str(&format!("{}\n\n", d.trim()));
    }
    s.push_str("## Stack\n\n");
    if r.languages.is_empty() {
        s.push_str("- No language manifest found at the root.\n");
    } else {
        s.push_str(&format!(
            "- Languages: {}\n",
            r.languages
                .iter()
                .map(|l| l.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !r.frameworks.is_empty() {
        s.push_str(&format!(
            "- Frameworks: {}\n",
            r.frameworks
                .iter()
                .map(|l| l.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !r.package_managers.is_empty() {
        s.push_str(&format!(
            "- Package managers: {}\n",
            r.package_managers.join(", ")
        ));
    }
    let cmds: Vec<(&str, &Option<String>)> = vec![
        ("Build", &r.build_command),
        ("Test", &r.test_command),
        ("Lint / check", &r.lint_command),
        ("Dev server", &r.dev_command),
    ];
    if cmds.iter().any(|(_, c)| c.is_some()) {
        s.push_str("\n## Commands\n\n");
        for (label, c) in cmds {
            if let Some(c) = c {
                s.push_str(&format!("- {label}: `{c}`\n"));
            }
        }
    }
    if !r.top_dirs.is_empty() {
        s.push_str("\n## Layout\n\n");
        for d in &r.top_dirs {
            s.push_str(&format!("- `{d}/`\n"));
        }
    }
    s.push_str("\n## Working rules\n\n");
    s.push_str(
        "- Read the code around a change before editing; match the existing style and naming.\n",
    );
    s.push_str(
        "- Keep each change small and focused on the task; do not reformat unrelated code.\n",
    );
    if let Some(t) = &r.test_command {
        s.push_str(&format!(
            "- Run `{t}` before saying a task is done, and report what failed.\n"
        ));
    }
    if let Some(l) = &r.lint_command {
        s.push_str(&format!("- Keep `{l}` clean.\n"));
    }
    s.push_str("- Never commit secrets, `.env` files or credentials; never print them.\n");
    for fw in &r.frameworks {
        let line = match fw.id.as_str() {
            "react" | "next" => "- React: function components with hooks; keep state close to where it is used.",
            "vue" | "nuxt" => "- Vue: single-file components; composables for shared logic.",
            "svelte" | "sveltekit" => "- Svelte: follow the runes already used in the code base.",
            "angular" => "- Angular: services for data access, components stay thin.",
            "django" => "- Django: migrations for every model change (`python manage.py makemigrations`).",
            "fastapi" => "- FastAPI: typed request/response models with Pydantic.",
            "rails" => "- Rails: follow the conventions; migrations for schema changes.",
            "tauri" => "- Tauri: commands return `Result<T, String>`; the front talks to Rust only through them.",
            _ => continue,
        };
        s.push_str(line);
        s.push('\n');
    }
    s
}

fn agents_md_component(markdown: &str) -> Result<Component, String> {
    let files: RawFiles = [(
        "project-overview.md".to_string(),
        markdown.as_bytes().to_vec(),
    )]
    .into_iter()
    .collect();
    let mut c = parse::parse_raw(ComponentKind::Rule, "project-overview.md", &files)
        .map_err(String::from)?;
    c.id = "wizard:project/rule/agents-md".into();
    c.name = "project-overview".into();
    c.description = "AGENTS.md generated by the OmniGet wizard".into();
    c.category = Some("project".into());
    c.source = Some(SourceRef {
        id: "omniget".into(),
        repo: None,
        commit: None,
        path: None,
        url: None,
    });
    c.license = Some("MIT".into());
    Ok(c)
}

/// Detects the stack, fetches the templates it needs and returns the session.
/// `extra_templates` adds templates the user picked by hand.
pub async fn start(project: &Path, extra_templates: &[String]) -> Result<Session, String> {
    if !project.is_dir() {
        return Err(format!(
            "{ERR_WIZARD}: {} is not a folder",
            project.display()
        ));
    }
    let stack = stack::detect(project);
    let mut templates: Vec<String> = templates_for(&stack)
        .into_iter()
        .map(str::to_string)
        .collect();
    for t in extra_templates {
        if !templates.contains(t) {
            templates.push(t.clone());
        }
    }
    let mut pieces = Vec::new();
    let mut warnings = Vec::new();
    for id in &templates {
        let item = match cat::index::item(id) {
            Ok(i) => i,
            Err(e) => {
                warnings.push(format!("{id}: {e}"));
                continue;
            }
        };
        match cat::fetch::item_bytes(&item).await {
            Ok((files, _)) => {
                let (p, w) = pieces_of(&item, &files);
                pieces.extend(p);
                warnings.extend(w);
            }
            Err(e) => warnings.push(format!("{id}: {e}")),
        }
    }
    let md = agents_md(&stack);
    let mut all = vec![Piece {
        id: "wizard:project/rule/agents-md".into(),
        kind: "rule".into(),
        name: "AGENTS.md".into(),
        description:
            "Project overview, commands and working rules, generated from what was detected".into(),
        template: "omniget".into(),
        selected: true,
        available: true,
        reason: None,
        notes: vec![],
        component: agents_md_component(&md).ok(),
    }];
    all.extend(dedupe(pieces));
    let s = Session {
        id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        project: project.to_path_buf(),
        stack,
        templates,
        pieces: all,
        agents_md: md,
        warnings,
    };
    let mut map = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() > 16 {
        map.clear();
    }
    map.insert(s.id.clone(), s.clone());
    Ok(s)
}

/// Components of the chosen pieces (only those, in the order shown), with the
/// AGENTS.md text the user edited.
pub fn selected_components(
    session_id: &str,
    selected: &[String],
    agents_md_text: Option<&str>,
) -> Result<(PathBuf, Vec<Component>), String> {
    let s = session(session_id)
        .ok_or_else(|| format!("{ERR_WIZARD}: session expired, detect again"))?;
    let mut out = Vec::new();
    for p in &s.pieces {
        if !selected.contains(&p.id) {
            continue;
        }
        if !p.available {
            return Err(format!(
                "{ERR_WIZARD}: {} can not be installed: {}",
                p.name,
                p.reason.clone().unwrap_or_default()
            ));
        }
        if p.id == "wizard:project/rule/agents-md" {
            if let Some(t) = agents_md_text.filter(|t| !t.trim().is_empty()) {
                out.push(agents_md_component(t)?);
                continue;
            }
        }
        if let Some(c) = &p.component {
            out.push(c.clone());
        }
    }
    if out.is_empty() {
        return Err(format!("{ERR_WIZARD}: nothing selected"));
    }
    Ok((s.project, out))
}

/// The review prompt of "review the setup with an agent" (cct ran it with
/// `sh -c 'claude "<prompt>"'`; here it is a Job with the chosen runner).
pub fn review_prompt(r: &StackReport, installed: &BTreeMap<String, Vec<String>>) -> String {
    let mut s = String::from(
        "Review the coding-agent setup of this project. Check that AGENTS.md (and CLAUDE.md or the other tool files) match the real project: the languages, frameworks and commands must be the ones in the manifests, and every command it names must exist. Look at the installed commands, agents, hooks, settings and MCP servers and flag anything that does not fit this code base, needs a tool that is missing, or is too permissive. Do not change files; answer with a short list of concrete fixes, most important first.\n\n",
    );
    s.push_str(&stack::context_block(r));
    if !installed.is_empty() {
        s.push_str("\nInstalled by the wizard:\n");
        for (tool, items) in installed {
            s.push_str(&format!("- {tool}: {}\n", items.join(", ")));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> CatalogItem {
        serde_json::from_value(json!({
            "id": id, "kind": "template", "name": template_short(id), "category": "x",
            "description": "", "source": {"id": "cct", "repo": "davila7/claude-code-templates", "commit": "abc", "path": "p", "dir": "cli-tool/templates/x", "url": ""},
            "files": [], "entry": "CLAUDE.md"
        }))
        .unwrap()
    }

    #[test]
    fn every_piece_is_its_own_component_and_placeholders_are_blocked() {
        let settings = json!({
            "permissions": {"allow": ["Bash", "Edit", "Bash(npm:*)"], "deny": ["Bash(curl:*)"], "defaultMode": "allowEdits"},
            "env": {"NODE_ENV": "development"},
            "hooks": {"PostToolUse": [{"matcher": "Write|Edit", "hooks": [{"type": "command", "command": "npx prettier --write \"$FILE\""}]}],
                       "Stop": [{"hooks": [{"type": "command", "command": "npx eslint ."}]}]}
        });
        let mcp = json!({"mcpServers": {
            "typescript-sdk": {"command": "node", "args": ["path/to/typescript-sdk/dist/index.js"]},
            "github": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": {"GITHUB_TOKEN": "your-token"}}
        }});
        let files: RawFiles = [
            (
                ".claude/commands/test.md".to_string(),
                b"# Test\nRun the tests for $ARGUMENTS\n".to_vec(),
            ),
            (
                ".claude/settings.json".to_string(),
                serde_json::to_vec(&settings).unwrap(),
            ),
            (".mcp.json".to_string(), serde_json::to_vec(&mcp).unwrap()),
            ("CLAUDE.md".to_string(), b"# Guide\nUse TS.\n".to_vec()),
        ]
        .into_iter()
        .collect();
        let (p, w) = pieces_of(&item("cct:templates/javascript-typescript"), &files);
        assert!(w.is_empty(), "{w:?}");
        let kinds: Vec<(&str, &str)> = p
            .iter()
            .map(|x| (x.kind.as_str(), x.name.as_str()))
            .collect();
        assert!(kinds.contains(&("command", "test")));
        assert_eq!(p.iter().filter(|x| x.kind == "hook").count(), 2);
        let ts = p.iter().find(|x| x.name == "typescript-sdk").unwrap();
        assert!(!ts.available);
        let gh = p.iter().find(|x| x.name == "github").unwrap();
        assert!(gh.available && !gh.selected && gh.notes[0].contains("GITHUB_TOKEN"));
        let set = p.iter().find(|x| x.kind == "setting").unwrap();
        let c = set.component.as_ref().unwrap();
        let v = serde_json::to_value(&c.body).unwrap();
        let text = v.to_string();
        assert!(
            text.contains("acceptEdits") && !text.contains("\"Bash\",") && !text.contains("hooks")
        );
        assert!(p
            .iter()
            .all(|x| x.component.as_ref().map(|c| c.id == x.id).unwrap_or(true)));
    }

    #[test]
    fn agents_md_uses_real_commands() {
        let r = StackReport {
            name: Some("shop".into()),
            languages: vec![stack::Detected {
                id: "typescript".into(),
                evidence: String::new(),
            }],
            frameworks: vec![stack::Detected {
                id: "react".into(),
                evidence: String::new(),
            }],
            test_command: Some("pnpm test".into()),
            top_dirs: vec!["src".into()],
            ..Default::default()
        };
        let md = agents_md(&r);
        assert!(md.starts_with("# shop"));
        assert!(
            md.contains("- Test: `pnpm test`")
                && md.contains("Run `pnpm test`")
                && md.contains("React")
        );
        assert_eq!(
            templates_for(&r),
            vec![
                "cct:templates/common",
                "cct:templates/javascript-typescript",
                "cct:templates/javascript-typescript/react-app"
            ]
        );
    }

    /// Live (network: the catalog's template files come from GitHub): a Node
    /// project → wizard → only the picked pieces → plan + apply into Claude,
    /// Codex and Cursor under a fake home. Keeps the output folder for a look
    /// when `OMNIGET_WIZARD_KEEP` is set.
    #[tokio::test]
    #[ignore]
    async fn live_wizard_node_project_for_claude_codex_cursor() {
        use crate::core::agentkit::{plan, writer, Env, Os, Scope};
        let root = std::env::var("OMNIGET_WIZARD_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::temp_dir().join(format!("wiz-{}", uuid::Uuid::new_v4().simple()))
            });
        let home = root.join("home");
        let proj = root.join("shop");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            proj.join("package.json"),
            r#"{"name":"shop","description":"Tiny cart API","scripts":{"test":"node --test","dev":"node src/index.js"},"dependencies":{"express":"4"}}"#,
        )
        .unwrap();
        std::fs::write(proj.join("src/index.js"), "console.log('hi')\n").unwrap();
        let s = start(&proj, &[]).await.unwrap();
        eprintln!("templates: {:?}", s.templates);
        eprintln!("warnings: {:?}", s.warnings);
        for p in &s.pieces {
            eprintln!(
                "{:<8} {:<40} selected={} available={} {:?}",
                p.kind, p.name, p.selected, p.available, p.reason
            );
        }
        assert!(s
            .templates
            .contains(&"cct:templates/javascript-typescript/node-api".to_string()));
        // Pick: AGENTS.md, two commands, one agent-free hook none, no MCP.
        let mut picked: Vec<String> = vec!["wizard:project/rule/agents-md".into()];
        picked.extend(
            s.pieces
                .iter()
                .filter(|p| p.kind == "command" && (p.name == "test" || p.name == "api-endpoint"))
                .map(|p| p.id.clone()),
        );
        let (project, comps) = selected_components(&s.id, &picked, None).unwrap();
        assert_eq!(comps.len(), picked.len());
        let env = Env::sandbox(&home, Os::current());
        let pl = plan::plan(
            &env,
            plan::PlanRequest {
                components: comps,
                targets: vec!["claude".into(), "codex".into(), "cursor".into()],
                scope: Some(Scope::Project),
                project_dir: Some(project.clone()),
                policy: plan::ConflictPolicy::Rename,
                secret_values: Default::default(),
            },
        )
        .unwrap();
        for u in &pl.units {
            eprintln!(
                "unit {:<8} {:<30} {:?} {:?}",
                u.target, u.component.name, u.status, u.error
            );
        }
        let rep = writer::apply(&env, &pl).unwrap();
        for f in &rep.files_written {
            eprintln!("wrote {f:?}");
        }
        let agents_md = std::fs::read_to_string(proj.join("AGENTS.md")).unwrap();
        assert!(agents_md.contains("- Test: `npm test`"), "{agents_md}");
        assert!(proj.join(".claude/commands/test.md").exists());
        let all: Vec<String> = walkdir::WalkDir::new(&proj)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                e.path()
                    .strip_prefix(&proj)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        eprintln!("project files: {all:#?}");
        // Nothing but what was picked: no settings, hooks or .mcp.json.
        assert!(!proj.join(".mcp.json").exists());
        assert!(!proj.join(".claude/settings.json").exists());
        if std::env::var("OMNIGET_WIZARD_KEEP").is_err() {
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}
