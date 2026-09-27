//! O gerador do índice portado para Rust: varre uma pasta (fonte local ou
//! cópia de um repo Git) e gera itens no formato do contrato, com a mesma
//! descoberta de tipo por caminho/arquivo e a mesma normalização de
//! `scripts/agentkit-catalog/build.mjs`.
//!
//! Layouts reconhecidos:
//! - claude-code-templates (`cli-tool/components/<tipo>/<categoria>/…` + `cli-tool/templates`);
//! - pastas de tipo na raiz (`agents/`, `commands/`, `skills/`, `hooks/`, `mcps/`,
//!   `settings/`, `statuslines/`, `loops/`, `mods/`, `rules/`, `workflows/`, `sandbox/`),
//!   com ou sem pasta de categoria (sem categoria → `general`);
//! - pastas de ferramenta (`.claude/{agents,commands,skills}`, `.agents/skills`,
//!   `.github/{agents,chatmodes,prompts}`);
//! - plugin/marketplace Claude (`.claude-plugin/plugin.json` / `marketplace.json`);
//! - `SKILL.md` solto em qualquer pasta (repo de skills).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use super::frontmatter::parse_document;
use super::model::{kind_plural, CatalogItem, ItemFile, ItemSource};
use super::normalize::{
    author_of, clean_description, description_from_body, detect_license_text, humanize,
    normalize_license, short_description, skill_exclusion, slim_json, to_list, tools_origin,
    valid_claude_model, ToolsOrigin,
};
use super::sha256_hex;

const MAX_DEPTH: usize = 8;
const MAX_FILES_PER_ITEM: usize = 2000;
const MAX_DISCOVERY_FILES: usize = 200_000;
const SKIP: &[&str] = &[
    ".git",
    ".DS_Store",
    "node_modules",
    "__pycache__",
    "Thumbs.db",
    "target",
];

/// De onde vêm os arquivos varridos.
#[derive(Debug, Clone, Default)]
pub struct ScanSource {
    /// Prefixo do id (`user-…`, `cct`).
    pub id: String,
    pub repo: Option<String>,
    pub commit: Option<String>,
    /// Pasta da raiz varrida dentro do repo (`""` = raiz do repo).
    pub repo_prefix: String,
    /// Raiz local para leitura direta (fonte local ou cópia extraída).
    pub local_root: Option<PathBuf>,
    pub updated: Option<String>,
    /// Licença do repo quando o item não declara nenhuma.
    pub default_license: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanReport {
    pub items: Vec<CatalogItem>,
    pub excluded: Vec<(String, String)>,
    pub skipped: Vec<(String, String)>,
}

fn posix(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn rel(base: &Path, p: &Path) -> String {
    posix(p.strip_prefix(base).unwrap_or(p))
}

fn skip_name(n: &str) -> bool {
    SKIP.contains(&n)
}

fn list_dirs(d: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            !skip_name(&n) && !n.starts_with('.')
        })
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn list_files(d: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| !skip_name(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

/// Todos os arquivos sob `dir` (ordenados), pulando pastas por predicado.
fn walk_files(dir: &Path, skip_dir: &dyn Fn(&Path) -> bool, skip_dot: bool) -> Vec<PathBuf> {
    walk_files_max(dir, skip_dir, skip_dot, MAX_FILES_PER_ITEM)
}

fn walk_files_max(
    dir: &Path,
    skip_dir: &dyn Fn(&Path) -> bool,
    skip_dot: bool,
    limit: usize,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut it = walkdir::WalkDir::new(dir)
        .max_depth(MAX_DEPTH)
        .sort_by_file_name()
        .into_iter();
    while let Some(Ok(e)) = it.next() {
        let name = e.file_name().to_string_lossy().into_owned();
        if e.depth() > 0 && skip_name(&name) {
            if e.file_type().is_dir() {
                it.skip_current_dir();
            }
            continue;
        }
        if e.depth() > 0 && skip_dot && name.starts_with('.') && name != ".claude-plugin" {
            if e.file_type().is_dir() {
                it.skip_current_dir();
            }
            continue;
        }
        if e.file_type().is_dir() {
            if e.depth() > 0 && skip_dir(e.path()) {
                it.skip_current_dir();
            }
            continue;
        }
        if e.file_type().is_file() {
            out.push(e.into_path());
            if out.len() > limit {
                break;
            }
        }
    }
    out
}

fn file_entry(base: &Path, full: &Path) -> Option<ItemFile> {
    let bytes = std::fs::read(full).ok()?;
    Some(ItemFile {
        path: rel(base, full),
        sha256: sha256_hex(&bytes),
        size: bytes.len() as u64,
    })
}

fn read_text(p: &Path) -> String {
    std::fs::read(p)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

fn read_json(p: &Path) -> Option<Value> {
    let bytes = std::fs::read(p).ok()?;
    serde_json::from_slice(&bytes).ok().or_else(|| {
        let t = String::from_utf8_lossy(&bytes);
        let re = regex::Regex::new(r",(\s*[}\]])").ok()?;
        serde_json::from_str(&re.replace_all(&t, "$1")).ok()
    })
}

fn stem(p: &Path) -> String {
    let n = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    for ext in [".md", ".mdc", ".json", ".toml", ".yaml", ".yml"] {
        if let Some(s) = n.strip_suffix(ext) {
            return s.to_string();
        }
    }
    n
}

fn license_from_files(dir: &Path, stop: &Path) -> Option<&'static str> {
    let mut d = dir.to_path_buf();
    while d.starts_with(stop) {
        for f in list_files(&d) {
            let n = f
                .file_name()
                .map(|s| s.to_string_lossy().to_ascii_uppercase())
                .unwrap_or_default();
            let base = n.split('.').next().unwrap_or("");
            if matches!(base, "LICENSE" | "LICENCE" | "COPYING") {
                if let Some(l) = detect_license_text(&read_text(&f)) {
                    return Some(l);
                }
            }
        }
        if d == stop || !d.pop() {
            break;
        }
    }
    None
}

struct Ctx<'a> {
    root: &'a Path,
    src: &'a ScanSource,
    report: ScanReport,
    seen_files: BTreeSet<PathBuf>,
    excluded_dirs: BTreeSet<PathBuf>,
}

struct NewItem<'a> {
    kind: &'a str,
    category: String,
    id_rest: String,
    name: String,
    entry: PathBuf,
    dir: PathBuf,
    files: Vec<ItemFile>,
    fm: Value,
    description: String,
    license: Option<String>,
    author: Option<String>,
    tags: Vec<String>,
    origin: &'a str,
    references: Vec<String>,
    norm: BTreeSet<String>,
    dir_is_item: bool,
}

impl Ctx<'_> {
    fn repo_path(&self, p: &Path) -> String {
        let r = rel(self.root, p);
        let pre = self.src.repo_prefix.trim_matches('/');
        match (pre.is_empty(), r.is_empty()) {
            (true, _) => r,
            (false, true) => pre.to_string(),
            (false, false) => format!("{pre}/{r}"),
        }
    }

    fn push(&mut self, n: NewItem<'_>) {
        let dir_repo = self.repo_path(&n.dir);
        let path_repo = if n.dir_is_item {
            dir_repo.clone()
        } else {
            self.repo_path(&n.entry)
        };
        let url = match (&self.src.repo, &self.src.commit) {
            (Some(r), Some(c)) => Some(format!(
                "https://github.com/{r}/{}/{c}/{path_repo}",
                if n.dir_is_item { "tree" } else { "blob" }
            )),
            _ => None,
        };
        let local = self
            .src
            .local_root
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let entry = rel(&n.dir, &n.entry);
        let mut n = n;
        n.files.sort_by(|a, b| a.path.cmp(&b.path));
        for f in &n.files {
            self.seen_files.insert(n.dir.join(&f.path));
        }
        let license = n
            .license
            .or_else(|| self.src.default_license.clone())
            .or_else(|| Some("unknown".into()))
            .filter(|l| l != "unknown");
        self.report.items.push(CatalogItem {
            id: format!("{}:{}/{}", self.src.id, kind_plural(n.kind), n.id_rest),
            kind: n.kind.to_string(),
            name: n.name,
            category: n.category,
            description: n.description,
            source: ItemSource {
                id: self.src.id.clone(),
                repo: self.src.repo.clone(),
                commit: self.src.commit.clone(),
                git_ref: None,
                path: path_repo,
                dir: dir_repo,
                url,
                local,
                ..Default::default()
            },
            license,
            author: n.author,
            tags: n.tags,
            origin_tool: n.origin.to_string(),
            files: n.files,
            entry,
            frontmatter: n.fm,
            references: n.references,
            stars: None,
            updated: self.src.updated.clone(),
            normalization: n.norm.into_iter().collect(),
            security: None,
            collides_with: Vec::new(),
            install_name: None,
        });
    }

    fn skip(&mut self, p: &Path, why: &str) {
        self.report
            .skipped
            .push((self.repo_path(p), why.to_string()));
    }

    /// Categoria + resto do id de um arquivo sob a pasta do tipo.
    fn cat_and_rest(type_dir: &Path, file: &Path) -> (String, String, bool) {
        let r = rel(type_dir, file);
        let no_ext = {
            let s = stem(file);
            match r.rsplit_once('/') {
                Some((d, _)) => format!("{d}/{s}"),
                None => s,
            }
        };
        match no_ext.split_once('/') {
            Some((cat, rest)) => (cat.to_string(), format!("{cat}/{rest}"), rest.contains('/')),
            None => ("general".into(), format!("general/{no_ext}"), false),
        }
    }

    fn markdown_kind(
        &mut self,
        type_dir: &Path,
        kind: &'static str,
        forced_origin: Option<&'static str>,
    ) {
        let files = walk_files_max(type_dir, &|_| false, true, MAX_DISCOVERY_FILES);
        for f in files {
            let bn = f
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let lower = bn.to_ascii_lowercase();
            if lower == "readme.md" || lower == "license.md" || lower == "contributing.md" {
                self.skip(&f, "readme");
                continue;
            }
            let is_md = lower.ends_with(".md") || lower.ends_with(".mdc");
            let depth_in_cat = rel(type_dir, &f).matches('/').count();
            if kind == "agent" && depth_in_cat > 1 {
                self.skip(
                    &f,
                    if is_md {
                        "nested_agent_path"
                    } else {
                        "helper_script"
                    },
                );
                continue;
            }
            if !(is_md || (kind == "agent" && !bn.contains('.'))) {
                continue;
            }
            let mut norm = BTreeSet::new();
            let text = read_text(&f);
            let (mut fm, body, had) = parse_document(&text);
            if !fm.is_object() {
                fm = json!({});
            }
            if !had {
                norm.insert("added_frontmatter".to_string());
            }
            if !bn.contains('.') {
                norm.insert("missing_extension".into());
            }
            let name = stem(&f)
                .trim_end_matches(".agent")
                .trim_end_matches(".chatmode")
                .trim_end_matches(".prompt")
                .to_string();
            if kind == "agent" && fm.get("name").is_none() {
                fm["name"] = json!(name);
                norm.insert("added_frontmatter".into());
            }
            let desc = match fm.get("description").and_then(Value::as_str) {
                Some(d) if !d.trim().is_empty() => d.to_string(),
                _ => {
                    let d = description_from_body(body);
                    let d = if d.is_empty() { humanize(&name) } else { d };
                    fm["description"] = json!(d);
                    norm.insert("added_frontmatter".into());
                    d
                }
            };
            let mut origin = forced_origin.unwrap_or("claude");
            if kind == "agent" && forced_origin.is_none() {
                match tools_origin(
                    fm.get("tools").unwrap_or(&Value::Null),
                    fm.get("model").and_then(Value::as_str),
                ) {
                    ToolsOrigin::Copilot => {
                        origin = "copilot";
                        norm.insert("copilot_tools".into());
                    }
                    ToolsOrigin::Mixed => {
                        norm.insert("foreign_tool_ids".into());
                    }
                    ToolsOrigin::Claude => {}
                }
            }
            if origin == "claude" {
                if let Some(m) = fm.get("model").cloned() {
                    if !m.as_str().is_some_and(valid_claude_model) {
                        fm["model_removed"] = m;
                        if let Some(o) = fm.as_object_mut() {
                            o.remove("model");
                        }
                        norm.insert("removed_invalid_model".into());
                    }
                }
            }
            let (short, marks) = short_description(&desc);
            norm.extend(marks.into_iter().map(str::to_string));
            fm["description"] = json!(clean_description(&desc));
            let (category, id_rest, nested) = Self::cat_and_rest(type_dir, &f);
            if nested {
                norm.insert("nested_path_indexed".into());
            }
            let dir = f.parent().unwrap_or(type_dir).to_path_buf();
            let Some(fe) = file_entry(&dir, &f) else {
                continue;
            };
            let license = normalize_license(fm.get("license").unwrap_or(&Value::Null));
            let tags = to_list(fm.get("tags"));
            let author = author_of(&fm);
            self.push(NewItem {
                kind,
                category,
                id_rest,
                name,
                entry: f.clone(),
                dir,
                files: vec![fe],
                fm,
                description: short,
                license,
                author,
                tags,
                origin,
                references: Vec::new(),
                norm,
                dir_is_item: false,
            });
        }
    }

    fn json_kind(&mut self, type_dir: &Path, kind: &'static str) {
        for f in walk_files_max(type_dir, &|_| false, true, MAX_DISCOVERY_FILES) {
            if f.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if rel(type_dir, &f).matches('/').count() > 1 {
                continue;
            }
            let name = stem(&f);
            if kind == "hook" && name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                self.skip(&f, "hook_pattern_library");
                continue;
            }
            let Some(j) = read_json(&f) else {
                self.skip(&f, "invalid_json");
                continue;
            };
            let cat_dir = f.parent().unwrap_or(type_dir).to_path_buf();
            let (category, id_rest, _) = Self::cat_and_rest(type_dir, &f);
            let mut norm = BTreeSet::new();
            let mut files: BTreeMap<PathBuf, ItemFile> = BTreeMap::new();
            if let Some(fe) = file_entry(&cat_dir, &f) {
                files.insert(f.clone(), fe);
            }
            let in_statuslines = type_dir.file_name().is_some_and(|n| n == "statuslines");
            let real_kind = if kind == "setting"
                && (in_statuslines || category == "statusline" || category == "statuslines")
            {
                "statusline"
            } else {
                kind
            };
            match kind {
                "hook" => {
                    for ext in ["py", "sh", "js"] {
                        let sib = cat_dir.join(format!("{name}.{ext}"));
                        if let Some(fe) =
                            sib.is_file().then(|| file_entry(&cat_dir, &sib)).flatten()
                        {
                            files.insert(sib, fe);
                        }
                    }
                    for sf in j["supportingFiles"].as_array().into_iter().flatten() {
                        let Some(s) = sf["source"].as_str() else {
                            continue;
                        };
                        let p = cat_dir.join(s);
                        let inside = p
                            .canonicalize()
                            .ok()
                            .zip(cat_dir.canonicalize().ok())
                            .is_some_and(|(a, b)| a.starts_with(b));
                        match (inside && p.is_file())
                            .then(|| file_entry(&cat_dir, &p))
                            .flatten()
                        {
                            Some(fe) => {
                                files.insert(p, fe);
                            }
                            None => {
                                norm.insert("missing_supporting_file".into());
                            }
                        }
                    }
                    let cmd = j["hooks"].to_string();
                    let re = hook_script_re();
                    for c in re.captures_iter(&cmd) {
                        let p = cat_dir.join(&c[1]);
                        if let Some(fe) = p.is_file().then(|| file_entry(&cat_dir, &p)).flatten() {
                            files.insert(p, fe);
                        }
                    }
                }
                "setting" => {
                    let cmd = j["statusLine"]["command"].as_str().unwrap_or_default();
                    let re = statusline_script_re();
                    for c in re.captures_iter(cmd) {
                        let p = cat_dir.join(&c[1]);
                        match p.is_file().then(|| file_entry(&cat_dir, &p)).flatten() {
                            Some(fe) => {
                                files.insert(p, fe);
                            }
                            None => {
                                norm.insert("missing_supporting_file".into());
                            }
                        }
                    }
                    if j.get("files").is_some_and(Value::is_object) {
                        norm.insert("inline_files".into());
                    }
                }
                _ => {}
            }
            let mut desc = j["description"].as_str().map(str::to_string);
            if desc.is_none() && kind == "mcp" {
                let servers = j.get("mcpServers").or_else(|| j.get("servers"));
                for s in servers
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|o| o.values())
                {
                    if let Some(d) = s["description"].as_str() {
                        desc = Some(d.to_string());
                        break;
                    }
                    if let Some(d) = s["descrption"].as_str() {
                        desc = Some(d.to_string());
                        norm.insert("fixed_description_typo".into());
                        break;
                    }
                }
            }
            let desc = desc.unwrap_or_else(|| {
                norm.insert("added_description".into());
                humanize(&name)
            });
            let (short, marks) = short_description(&desc);
            norm.extend(marks.into_iter().map(str::to_string));
            let mut tags = to_list(j.get("tags"));
            tags.extend(to_list(j.get("keywords")));
            let (plural_kind_id, id_rest) = (real_kind, id_rest);
            self.push(NewItem {
                kind: plural_kind_id,
                category,
                id_rest,
                name,
                entry: f.clone(),
                dir: cat_dir,
                files: files.into_values().collect(),
                fm: slim_json(&j),
                description: short,
                license: normalize_license(j.get("license").unwrap_or(&Value::Null)),
                author: author_of(&j),
                tags,
                origin: "claude",
                references: Vec::new(),
                norm,
                dir_is_item: false,
            });
        }
    }

    fn skills(&mut self, base: &Path, id_base: &Path) {
        let mut dirs: Vec<PathBuf> = walk_files_max(base, &|_| false, false, MAX_DISCOVERY_FILES)
            .into_iter()
            .filter(|f| f.file_name().and_then(|n| n.to_str()) == Some("SKILL.md"))
            .filter_map(|f| f.parent().map(Path::to_path_buf))
            .collect();
        dirs.sort();
        dirs.dedup();
        let set: BTreeSet<PathBuf> = dirs.iter().cloned().collect();
        for dir in &dirs {
            if self.seen_files.contains(&dir.join("SKILL.md")) || self.excluded_dirs.contains(dir) {
                continue;
            }
            let r = rel(id_base, dir);
            let r = if r.is_empty() {
                dir.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "skill".into())
            } else {
                r
            };
            let parts: Vec<&str> = if base == id_base && base == self.root {
                // passe na raiz: ignora pastas ocultas e `skills` no caminho
                let f: Vec<&str> = r
                    .split('/')
                    .filter(|p| !p.starts_with('.') && *p != "skills")
                    .collect();
                if f.is_empty() {
                    vec![r.as_str()]
                } else {
                    f
                }
            } else {
                r.split('/').collect()
            };
            let r = parts.join("/");
            let name = parts.last().copied().unwrap_or("skill").to_string();
            let category = if parts.len() >= 2 {
                parts[0].to_string()
            } else {
                "general".to_string()
            };
            let id_rest = if parts.len() >= 2 {
                r.clone()
            } else {
                format!("general/{r}")
            };
            let entry = dir.join("SKILL.md");
            let text = read_text(&entry);
            let (mut fm, body, had) = parse_document(&text);
            if !fm.is_object() {
                fm = json!({});
            }
            let mut norm = BTreeSet::new();
            if !had {
                norm.insert("added_frontmatter".to_string());
            }
            if fm.get("name").is_none() {
                fm["name"] = json!(name);
                norm.insert("added_frontmatter".into());
            }
            if fm.get("description").and_then(Value::as_str).is_none() {
                let d = description_from_body(body);
                fm["description"] = json!(if d.is_empty() { humanize(&name) } else { d });
                norm.insert("added_frontmatter".into());
            }
            let raw_lic = fm["license"].as_str().unwrap_or_default().to_string();
            let mut license = normalize_license(
                fm.get("license")
                    .or_else(|| fm.get("metadata").and_then(|m| m.get("license")))
                    .unwrap_or(&Value::Null),
            );
            if license.is_none() || raw_lic.to_ascii_lowercase().contains("complete terms") {
                if let Some(l) = license_from_files(dir, id_base) {
                    license = Some(l.to_string());
                    norm.insert("license_from_file".into());
                }
            }
            if raw_lic.to_ascii_lowercase().contains("proprietary") {
                license = Some("Proprietary".into());
            }
            if let Some(why) = skill_exclusion(&name, license.as_deref()) {
                let p = self.repo_path(dir);
                self.report.excluded.push((p, why.to_string()));
                self.excluded_dirs.insert(dir.clone());
                continue;
            }
            if license.is_none() && self.src.default_license.is_some() {
                norm.insert("license_from_repo".into());
            }
            let nested: Vec<String> = dirs
                .iter()
                .filter(|d| *d != dir && d.starts_with(dir))
                .map(|d| {
                    let rr = rel(id_base, d);
                    format!("{}:skills/{}", self.src.id, rr)
                })
                .collect();
            let set_ref = &set;
            let files: Vec<ItemFile> = walk_files(dir, &|d| set_ref.contains(d), false)
                .iter()
                .filter_map(|f| file_entry(dir, f))
                .collect();
            if parts.len() > 2 {
                norm.insert("nested_skill_indexed".into());
            }
            if fm["name"].as_str().is_some_and(|n| n != name) {
                norm.insert("name_differs_from_dir".into());
            }
            let desc = fm["description"].as_str().unwrap_or_default().to_string();
            let (short, marks) = short_description(&desc);
            norm.extend(marks.into_iter().map(str::to_string));
            let mut tags = to_list(fm.get("tags"));
            tags.extend(to_list(fm.get("metadata").and_then(|m| m.get("tags"))));
            if dir.join("agents").join("openai.yaml").is_file() {
                tags.push("codex-metadata".into());
            }
            tags.sort();
            tags.dedup();
            let author = author_of(&fm);
            self.push(NewItem {
                kind: "skill",
                category,
                id_rest,
                name,
                entry,
                dir: dir.clone(),
                files,
                fm,
                description: short,
                license,
                author,
                tags,
                origin: "claude",
                references: nested,
                norm,
                dir_is_item: true,
            });
        }
    }

    fn loops(&mut self, type_dir: &Path) {
        let before = self.report.items.len();
        self.markdown_kind(type_dir, "loop", None);
        for it in &mut self.report.items[before..] {
            let comps = to_list(it.frontmatter.get("components"));
            let mut refs = Vec::new();
            for c in comps {
                if let Some((k, rest)) = c.split_once(':') {
                    refs.push(format!("{}:{}/{rest}", self.src.id, kind_plural(k)));
                }
            }
            it.references = refs;
        }
    }

    fn plugin_dir(&mut self, dir: &Path, kind: &'static str, category: String, id_rest: String) {
        let manifest = dir.join(".claude-plugin").join("plugin.json");
        let Some(j) = read_json(&manifest) else {
            self.skip(dir, "invalid_plugin_manifest");
            return;
        };
        let mut norm = BTreeSet::new();
        if kind == "mod" && !dir.join("hooks").join("hooks.json").is_file() {
            norm.insert("missing_hooks_json".to_string());
        }
        let mut license = normalize_license(j.get("license").unwrap_or(&Value::Null));
        if license.is_none() {
            if let Some(l) = license_from_files(dir, dir) {
                license = Some(l.into());
                norm.insert("license_from_file".into());
            }
        }
        let files: Vec<ItemFile> = walk_files(dir, &|_| false, true)
            .iter()
            .filter_map(|f| file_entry(dir, f))
            .collect();
        let name = j["name"].as_str().map(str::to_string).unwrap_or_else(|| {
            dir.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let (short, marks) =
            short_description(j["description"].as_str().unwrap_or(&humanize(&name)));
        norm.extend(marks.into_iter().map(str::to_string));
        self.push(NewItem {
            kind,
            category,
            id_rest,
            name,
            entry: manifest,
            dir: dir.to_path_buf(),
            files,
            fm: slim_json(&j),
            description: short,
            license,
            author: author_of(&j),
            tags: to_list(j.get("keywords")),
            origin: "claude",
            references: Vec::new(),
            norm,
            dir_is_item: true,
        });
    }

    fn mods(&mut self, type_dir: &Path) {
        for cat in list_dirs(type_dir) {
            let cname = cat
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            for d in list_dirs(&cat) {
                let n = d
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if d.join(".claude-plugin").join("plugin.json").is_file() {
                    self.plugin_dir(&d, "mod", cname.clone(), format!("{cname}/{n}"));
                } else {
                    self.skip(&d, "mod_without_manifest");
                }
            }
        }
    }

    fn dir_items(&mut self, type_dir: &Path, kind: &'static str) {
        for d in list_dirs(type_dir) {
            let n = d
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let entry = ["claude-code-sandbox.md", "CLAUDE.md", "README.md"]
                .iter()
                .map(|f| d.join(f))
                .find(|p| p.is_file());
            let Some(entry) = entry else { continue };
            self.dir_item(
                &d,
                kind,
                kind_plural(kind).to_string(),
                n.clone(),
                n,
                &entry,
                &|_| false,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dir_item(
        &mut self,
        dir: &Path,
        kind: &'static str,
        category: String,
        id_rest: String,
        name: String,
        entry: &Path,
        skip_dir: &dyn Fn(&Path) -> bool,
    ) {
        let text = read_text(entry);
        let (mut fm, body, had) = parse_document(&text);
        if !fm.is_object() {
            fm = json!({});
        }
        let mut norm = BTreeSet::new();
        if fm.get("description").and_then(Value::as_str).is_none() {
            let d = description_from_body(body);
            fm["description"] = json!(if d.is_empty() { humanize(&name) } else { d });
            norm.insert("added_frontmatter".to_string());
        }
        if !had {
            norm.insert("added_frontmatter".to_string());
        }
        fm.as_object_mut()
            .map(|o| o.entry("name").or_insert(json!(name)));
        let (short, marks) = short_description(fm["description"].as_str().unwrap_or_default());
        norm.extend(marks.into_iter().map(str::to_string));
        let files: Vec<ItemFile> = walk_files(dir, skip_dir, false)
            .iter()
            .filter_map(|f| file_entry(dir, f))
            .collect();
        self.push(NewItem {
            kind,
            category: category.clone(),
            id_rest,
            name,
            entry: entry.to_path_buf(),
            dir: dir.to_path_buf(),
            files,
            fm,
            description: short,
            license: None,
            author: None,
            tags: vec![category],
            origin: "claude",
            references: Vec::new(),
            norm,
            dir_is_item: true,
        });
    }

    fn templates(&mut self, base: &Path) {
        for lang in list_dirs(base) {
            let l = lang
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ex = lang.join("examples");
            let exc = ex.clone();
            let skip_ex = move |d: &Path| d == exc;
            if let Some(e) = template_entry(&lang, &skip_ex) {
                self.dir_item(
                    &lang,
                    "template",
                    l.clone(),
                    l.clone(),
                    l.clone(),
                    &e,
                    &skip_ex,
                );
            }
            for fw in list_dirs(&ex) {
                let f = fw
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(e) = template_entry(&fw, &|_| false) {
                    self.dir_item(
                        &fw,
                        "template",
                        l.clone(),
                        format!("{l}/{f}"),
                        f,
                        &e,
                        &|_| false,
                    );
                }
            }
        }
    }

    fn marketplace(&mut self, root: &Path) {
        let Some(m) = read_json(&root.join(".claude-plugin").join("marketplace.json")) else {
            return;
        };
        let plugin_root = m["metadata"]["pluginRoot"].as_str();
        for p in m["plugins"].as_array().into_iter().flatten() {
            let Some(name) = p["name"].as_str() else {
                continue;
            };
            let src = super::marketplace::plugin_source(
                &p["source"],
                self.src.repo.as_deref().unwrap_or(""),
                self.src.commit.as_deref(),
                plugin_root,
            );
            if src.url.is_some()
                || src.repo.as_deref() != self.src.repo.as_deref() && self.src.repo.is_some()
            {
                continue; // plugin externo: fica para `catalog_marketplace`
            }
            let dir = if src.path.is_empty() {
                root.to_path_buf()
            } else {
                root.join(&src.path)
            };
            if !dir.join(".claude-plugin").join("plugin.json").is_file() {
                // marketplace com plugin sem manifesto próprio: manifesto vem da entrada
                if dir.is_dir() && dir != root {
                    let files: Vec<ItemFile> = walk_files(&dir, &|_| false, true)
                        .iter()
                        .filter_map(|f| file_entry(&dir, f))
                        .collect();
                    let (short, marks) =
                        short_description(p["description"].as_str().unwrap_or(name));
                    let mut norm: BTreeSet<String> =
                        marks.into_iter().map(str::to_string).collect();
                    norm.insert("manifest_from_marketplace".into());
                    let cat = p["category"].as_str().unwrap_or("general").to_lowercase();
                    self.push(NewItem {
                        kind: "plugin",
                        category: cat.clone(),
                        id_rest: format!("{cat}/{name}"),
                        name: name.to_string(),
                        entry: dir.join(".claude-plugin").join("plugin.json"),
                        dir: dir.clone(),
                        files,
                        fm: slim_json(p),
                        description: short,
                        license: normalize_license(p.get("license").unwrap_or(&Value::Null)),
                        author: author_of(p),
                        tags: to_list(p.get("keywords")),
                        origin: "claude",
                        references: Vec::new(),
                        norm,
                        dir_is_item: true,
                    });
                }
                continue;
            }
            let cat = p["category"].as_str().unwrap_or("general").to_lowercase();
            self.plugin_dir(&dir, "plugin", cat.clone(), format!("{cat}/{name}"));
        }
    }
}

fn hook_script_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\.claude/hooks/([\w.-]+\.(?:py|sh|js))").expect("re"))
}

fn statusline_script_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\.claude/scripts/([\w.-]+\.(?:py|sh|js))").expect("re"))
}

/// Arquivo principal de um template: CLAUDE.md, README.md ou o primeiro arquivo.
fn template_entry(dir: &Path, skip: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    ["CLAUDE.md", "README.md"]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.is_file())
        .or_else(|| walk_files(dir, skip, false).into_iter().next())
}

/// Tipo de uma pasta conhecida (nome relativo à raiz dos componentes).
const TYPE_DIRS: &[(&str, &str, Option<&str>)] = &[
    ("agents", "agent", None),
    ("commands", "command", None),
    ("rules", "rule", None),
    ("workflows", "workflow", None),
    ("loops", "loop", None),
    (".claude/agents", "agent", None),
    (".claude/commands", "command", None),
    (".github/agents", "agent", Some("copilot")),
    (".github/chatmodes", "agent", Some("copilot")),
    (".github/prompts", "command", Some("copilot")),
    (".cursor/rules", "rule", Some("cursor")),
];

/// Varre `root` e gera itens. Nunca falha: problemas viram `skipped`.
pub fn scan(root: &Path, src: &ScanSource) -> ScanReport {
    let mut ctx = Ctx {
        root,
        src,
        report: ScanReport::default(),
        seen_files: BTreeSet::new(),
        excluded_dirs: BTreeSet::new(),
    };
    let cct = root.join("cli-tool").join("components");
    let comp = if cct.is_dir() {
        cct.clone()
    } else {
        root.to_path_buf()
    };

    // plugin único na raiz: o repo inteiro é um plugin
    let root_plugin = root.join(".claude-plugin").join("plugin.json").is_file()
        && !root
            .join(".claude-plugin")
            .join("marketplace.json")
            .is_file();
    if root_plugin {
        let name = read_json(&root.join(".claude-plugin").join("plugin.json"))
            .and_then(|j| j["name"].as_str().map(str::to_string))
            .unwrap_or_else(|| "plugin".into());
        ctx.plugin_dir(root, "plugin", "general".into(), format!("general/{name}"));
    }
    ctx.marketplace(root);

    for (d, kind, origin) in TYPE_DIRS {
        let p = comp.join(d);
        if !p.is_dir() {
            continue;
        }
        if *kind == "loop" {
            ctx.loops(&p);
        } else {
            ctx.markdown_kind(&p, kind, *origin);
        }
    }
    for (d, kind) in [
        ("hooks", "hook"),
        ("mcps", "mcp"),
        ("settings", "setting"),
        ("statuslines", "statusline"),
    ] {
        let p = comp.join(d);
        if p.is_dir() {
            ctx.json_kind(
                &p,
                if kind == "statusline" {
                    "setting"
                } else {
                    kind
                },
            );
        }
    }
    let mods = comp.join("mods");
    if mods.is_dir() {
        ctx.mods(&mods);
    }
    let sandbox = comp.join("sandbox");
    if sandbox.is_dir() {
        ctx.dir_items(&sandbox, "sandbox");
    }
    let templates = root.join("cli-tool").join("templates");
    if templates.is_dir() {
        ctx.templates(&templates);
    }
    for d in ["skills", ".claude/skills", ".agents/skills"] {
        let p = comp.join(d);
        if p.is_dir() {
            ctx.skills(&p, &p);
        }
    }
    // SKILL.md soltos (repo de skills sem pasta `skills/`)
    ctx.skills(root, root);

    // colisões de nome dentro do mesmo tipo
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, it) in ctx.report.items.iter().enumerate() {
        groups
            .entry((it.kind.clone(), it.name.to_lowercase()))
            .or_default()
            .push(i);
    }
    for idx in groups.values().filter(|g| g.len() > 1) {
        let ids: Vec<String> = idx
            .iter()
            .map(|&i| ctx.report.items[i].id.clone())
            .collect();
        for &i in idx {
            let it = &mut ctx.report.items[i];
            it.collides_with = ids.iter().filter(|x| **x != it.id).cloned().collect();
            it.install_name = Some(format!("{}-{}", it.category, it.name));
            if !it.normalization.iter().any(|n| n == "renamed_collision") {
                it.normalization.push("renamed_collision".into());
                it.normalization.sort();
            }
        }
    }
    // ids duplicados (mesmo arquivo alcançado por dois caminhos): fica o primeiro
    let mut seen = BTreeSet::new();
    ctx.report.items.retain(|i| seen.insert(i.id.clone()));
    ctx.report.items.sort_by(|a, b| a.id.cmp(&b.id));
    ctx.report
}
