//! Rules for every non-Claude dialect (estudo 06 §0C.1, plan §4.3).
//!
//! Root rules (always on) go as a marked block into `AGENTS.md`, the single
//! source most tools read; tools that do not read it get a bridge: Gemini
//! (`GEMINI.md` importing `@./AGENTS.md`, per rule so every block owns its
//! bridge) and Aider (`CONVENTIONS.md` plus a `read:` entry). Scoped rules
//! (`globs`) go to each tool's native scoped file (`.cursor/rules/*.mdc`,
//! `.github/instructions/*.instructions.md` `applyTo`, `.kiro/steering`
//! `inclusion`, `.devin/rules` `trigger`, `.augment/rules`, `.clinerules`
//! `paths`, `.continue/rules`, `.qoder/rules`, `.trae/rules`, `.qwen/rules`);
//! OpenCode/Kilo get a file listed in `instructions[]`; tools with no scoping
//! get a "When editing <globs>" section in AGENTS.md.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::agent_common::*;
use super::{format_for_path, kind_path, ConvertCtx, Converter, Dedupe, PatchOp, PlannedFile};
use crate::core::agentkit::edit::{keys, DocFormat};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

/// Rule format ids served here (`claude` stays with the Claude converter).
pub const FORMATS: &[&str] = &[
    "agents_md",
    "gemini_md",
    "qwen_md",
    "copilot_instructions",
    "goosehints",
    "aider_conventions",
    "cursor_mdc",
    "kiro_steering",
    "devin_rules",
    "augment_rules",
    "junie_rules",
    "cline_rules",
    "continue_rules",
    "qoder",
    "trae_rules",
];

pub struct RuleConverter;

/// The AGENTS.md this target reads for a scope: `paths.rules` when it is an
/// AGENTS.md, else an AGENTS.md among its alternatives, else the project root one.
pub(crate) fn agents_md(target: &TargetAdapter, scope: Scope, ctx: &ConvertCtx) -> Option<PathBuf> {
    let spec = target.paths.get("rules");
    let is_agents = |t: &str| t.ends_with("AGENTS.md");
    if let Some(t) = target
        .path_template("rules", scope, ctx.env.os)
        .filter(|t| is_agents(t))
    {
        return ctx.env.expand(t, ctx.project);
    }
    if let Some(alt) = spec.and_then(|s| s.alternatives(scope).iter().find(|t| is_agents(t))) {
        return ctx.env.expand(alt, ctx.project);
    }
    match scope {
        Scope::Project | Scope::Local => ctx.project.map(|p| p.join("AGENTS.md")),
        _ => None,
    }
}

fn globs_text(g: &[String]) -> String {
    g.join(", ")
}

/// Markdown section for tools that cannot scope a rule to files.
pub fn when_editing(r: &RuleSpec) -> String {
    format!(
        "## When editing {}\n\n{}",
        globs_text(&r.globs),
        r.markdown.trim()
    )
}

impl RuleConverter {
    /// Native scoped rule: (file name, text, losses, notes), or `None` when the
    /// format has no scoped file.
    pub fn scoped_file(
        fmt: &str,
        name: &str,
        r: &RuleSpec,
    ) -> Option<(String, String, Vec<String>, Vec<String>)> {
        let mut fm: Vec<(String, Value)> = Vec::new();
        let mut losses = Vec::new();
        let mut notes = Vec::new();
        let desc = r.description.trim();
        let mut body = r.markdown.clone();
        let file = match fmt {
            "cursor_mdc" => {
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                fm.push(("globs".into(), json!(r.globs.join(","))));
                fm.push(("alwaysApply".into(), json!(false)));
                format!("{name}.mdc")
            }
            "copilot_instructions" => {
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                fm.push(("applyTo".into(), json!(r.globs.join(","))));
                format!("{name}.instructions.md")
            }
            "kiro_steering" => {
                fm.push(("inclusion".into(), json!("fileMatch")));
                fm.push(("fileMatchPattern".into(), json!(r.globs)));
                notes.push(
                    "the Kiro CLI loads every steering file (inclusion modes are IDE-only)".into(),
                );
                format!("{name}.md")
            }
            "devin_rules" => {
                fm.push(("trigger".into(), json!("glob")));
                fm.push(("globs".into(), json!(r.globs.join(","))));
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                format!("{name}.md")
            }
            "augment_rules" => {
                fm.push(("type".into(), json!("agent_requested")));
                let d = if desc.is_empty() {
                    format!("Applies when editing {}", globs_text(&r.globs))
                } else {
                    format!("{desc} (applies when editing {})", globs_text(&r.globs))
                };
                fm.push(("description".into(), json!(d)));
                losses.push("glob scoping (Augment attaches the rule when the agent decides it is relevant)".into());
                format!("{name}.md")
            }
            "junie_rules" => {
                body = when_editing(r);
                losses.push(
                    "glob scoping (Junie loads every rule; the rule says where it applies)".into(),
                );
                format!("{name}.md")
            }
            "cline_rules" | "qwen_md" => {
                if fmt == "qwen_md" && !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                fm.push(("paths".into(), json!(r.globs)));
                format!("{name}.md")
            }
            "continue_rules" => {
                fm.push(("name".into(), json!(name)));
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                fm.push(("globs".into(), json!(r.globs)));
                fm.push(("alwaysApply".into(), json!(false)));
                format!("{name}.md")
            }
            "qoder" => {
                fm.push(("trigger".into(), json!("glob")));
                fm.push(("glob".into(), json!(r.globs)));
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                format!("{name}.md")
            }
            "trae_rules" => {
                fm.push(("alwaysApply".into(), json!(false)));
                fm.push(("globs".into(), json!(r.globs.join(","))));
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                format!("{name}.md")
            }
            _ => return None,
        };
        Some((file, parse::render_frontmatter(&fm, &body), losses, notes))
    }

    /// Always-on rule file in a rules folder: (file name, text).
    pub fn always_file(fmt: &str, name: &str, body: &str, description: &str) -> (String, String) {
        let mut fm: Vec<(String, Value)> = Vec::new();
        let desc = description.trim();
        let mut file = format!("{name}.md");
        match fmt {
            "cursor_mdc" => {
                fm.push(("alwaysApply".into(), json!(true)));
                file = format!("{name}.mdc");
            }
            "copilot_instructions" => {
                fm.push(("applyTo".into(), json!("**")));
                file = format!("{name}.instructions.md");
            }
            "kiro_steering" => fm.push(("inclusion".into(), json!("always"))),
            "devin_rules" => fm.push(("trigger".into(), json!("always_on"))),
            "augment_rules" => fm.push(("type".into(), json!("always_apply"))),
            "continue_rules" | "trae_rules" => fm.push(("alwaysApply".into(), json!(true))),
            "qoder" => fm.push(("trigger".into(), json!("always_on"))),
            _ => {}
        }
        if !fm.is_empty() && !desc.is_empty() {
            fm.push(("description".into(), json!(desc)));
        }
        let body = format!("{}\n", body.trim_end());
        (file, parse::render_frontmatter(&fm, &body))
    }

    fn block(
        target: &TargetAdapter,
        c: &Component,
        file: PathBuf,
        id: String,
        content: String,
        label: &str,
    ) -> PlannedFile {
        PlannedFile::merge(
            &target.id,
            c,
            file,
            DocFormat::Markdown,
            vec![PatchOp::TextBlock { id, content }],
            label,
        )
    }
}

impl Converter for RuleConverter {
    fn id(&self) -> &'static str {
        "rule_files"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Rule
            && target
                .format(kind)
                .map(|f| FORMATS.contains(&f))
                .unwrap_or(false)
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let ComponentBody::Rule(r) = &c.body else {
            return Ok(vec![]);
        };
        let fmt = target.format(ComponentKind::Rule).unwrap_or("agents_md");
        let name = ctx.name_for(c);
        let block_id = format!("{}:{}", c.id, name);
        let scoped = r.scope == RuleScope::Scoped && !r.globs.is_empty();
        let mut out: Vec<PlannedFile> = Vec::new();

        // ---- scoped: the tool's own scoped file when it has one
        if scoped {
            let sfmt = target
                .formats
                .get("scoped_rule")
                .map(String::as_str)
                .unwrap_or(fmt);
            if let Some(dir) = kind_path(target, "scoped_rules", scope, ctx) {
                if let Some((file, text, losses, notes)) = Self::scoped_file(sfmt, &name, r) {
                    let mut pf = PlannedFile::write(&target.id, c, dir.join(file), text, "rule");
                    pf.primary = true;
                    pf.losses = losses;
                    pf.notes = notes;
                    return Ok(vec![pf]);
                }
                // OpenCode/Kilo: a file listed in `instructions` (always loaded)
                if let (Some(settings), true) = (
                    kind_path(target, "settings", scope, ctx),
                    matches!(target.format(ComponentKind::Setting), Some("opencode")),
                ) {
                    let file = dir.join(format!("{name}.md"));
                    let entry = match (scope, ctx.project) {
                        (Scope::Project | Scope::Local, Some(p)) => {
                            rel_slash(p, &file).unwrap_or_else(|| file.display().to_string())
                        }
                        _ => file.display().to_string(),
                    };
                    let mut pf =
                        PlannedFile::write(&target.id, c, file, when_editing(r) + "\n", "rule");
                    pf.primary = true;
                    pf.losses.push(format!(
                        "glob scoping ({} loads instruction files always; the rule says where it applies)",
                        target.name
                    ));
                    out.push(pf);
                    out.push(PlannedFile::merge(
                        &target.id,
                        c,
                        settings.clone(),
                        format_for_path(&settings),
                        vec![PatchOp::Append {
                            path: keys(["instructions"]),
                            value: json!(entry),
                            dedupe: Dedupe::Equal,
                        }],
                        "instructions entry",
                    ));
                    return Ok(out);
                }
                // a plain rules folder without frontmatter (Roo)
                if !matches!(
                    sfmt,
                    "agents_md" | "gemini_md" | "aider_conventions" | "goosehints"
                ) || target.id == "roo"
                {
                    let mut pf = PlannedFile::write(
                        &target.id,
                        c,
                        dir.join(format!("{name}.md")),
                        when_editing(r) + "\n",
                        "rule",
                    );
                    pf.primary = true;
                    pf.losses.push(format!(
                        "glob scoping ({} loads every rule file; the rule says where it applies)",
                        target.name
                    ));
                    return Ok(vec![pf]);
                }
            }
        }

        // ---- root rules (and scoped rules folded into a section)
        let content = if scoped {
            when_editing(r)
        } else {
            r.markdown.trim().to_string()
        };
        let fold_loss = scoped.then(|| {
            format!(
                "glob scoping ({} has no scoped rules; written as a \"When editing\" section)",
                target.name
            )
        });
        match fmt {
            "trae_rules" if !scoped => {
                // AGENTS.md needs a toggle in Trae; an always-apply rule does not
                let dir = dir_for(target, "scoped_rules", scope, ctx)?;
                let mut fm: Vec<(String, Value)> = vec![("alwaysApply".into(), json!(true))];
                if !r.description.trim().is_empty() {
                    fm.push(("description".into(), json!(r.description.trim())));
                }
                let mut pf = PlannedFile::write(
                    &target.id,
                    c,
                    dir.join(format!("{name}.md")),
                    parse::render_frontmatter(&fm, &r.markdown),
                    "rule",
                );
                pf.primary = true;
                out.push(pf);
            }
            "gemini_md" => {
                let native = dir_for(target, "rules", scope, ctx)?;
                match agents_md(target, scope, ctx).filter(|_| scope == Scope::Project) {
                    Some(src) => {
                        out.push(Self::block(
                            target,
                            c,
                            src,
                            block_id.clone(),
                            content,
                            "rule",
                        ));
                        let mut bridge = Self::block(
                            target,
                            c,
                            native,
                            format!("{block_id}:bridge"),
                            "@./AGENTS.md".into(),
                            "rules bridge",
                        );
                        bridge.notes.push("GEMINI.md imports AGENTS.md (Gemini does not read AGENTS.md by default)".into());
                        out.push(bridge);
                    }
                    None => out.push(Self::block(target, c, native, block_id, content, "rule")),
                }
            }
            "aider_conventions" => {
                let conv = dir_for(target, "rules", scope, ctx)?;
                let settings = dir_for(target, "settings", scope, ctx)?;
                let entry = match (scope, ctx.project) {
                    (Scope::Project | Scope::Local, Some(p)) => {
                        rel_slash(p, &conv).unwrap_or_else(|| conv.display().to_string())
                    }
                    _ => conv.display().to_string(),
                };
                out.push(Self::block(target, c, conv, block_id, content, "rule"));
                out.push(PlannedFile::merge(
                    &target.id,
                    c,
                    settings,
                    DocFormat::Yaml,
                    vec![PatchOp::Append {
                        path: keys(["read"]),
                        value: json!(entry),
                        dedupe: Dedupe::Equal,
                    }],
                    "read entry",
                ));
            }
            "junie_rules" => {
                let file = match (scope, ctx.project) {
                    (Scope::Project | Scope::Local, Some(p)) => {
                        // `.junie/AGENTS.md` is used exclusively when present
                        let own = p.join(".junie").join("AGENTS.md");
                        if own.is_file() {
                            own
                        } else {
                            p.join("AGENTS.md")
                        }
                    }
                    _ => dir_for(target, "rules", scope, ctx)?,
                };
                out.push(Self::block(target, c, file, block_id, content, "rule"));
            }
            _ => {
                // AGENTS.md readers; Copilot/Qwen/Cursor/Kiro … read it natively
                match agents_md(target, scope, ctx)
                    .or_else(|| kind_path(target, "rules", scope, ctx))
                {
                    Some(file) => out.push(Self::block(target, c, file, block_id, content, "rule")),
                    None => {
                        // no rules file for this scope (Roo global …): an always-on
                        // file in the rules folder
                        let dir = kind_path(target, "scoped_rules", scope, ctx)
                            .ok_or_else(|| no_path(target, "rules file", scope))?;
                        let sfmt = target
                            .formats
                            .get("scoped_rule")
                            .map(String::as_str)
                            .unwrap_or(fmt);
                        let (file, text) = Self::always_file(sfmt, &name, &content, &r.description);
                        let mut pf =
                            PlannedFile::write(&target.id, c, dir.join(file), text, "rule");
                        pf.primary = true;
                        out.push(pf);
                    }
                }
            }
        }
        if let Some(l) = fold_loss {
            if let Some(first) = out.first_mut() {
                first.losses.push(l);
            }
        }
        Ok(out)
    }

    fn import(
        &self,
        kind: ComponentKind,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<Component>> {
        if kind != ComponentKind::Rule {
            return Ok(vec![]);
        }
        let Some(dir) = kind_path(target, "scoped_rules", scope, ctx) else {
            return Ok(vec![]);
        };
        let suffixes: &[&str] = &[".instructions.md", ".mdc", ".md"];
        let mut out = Vec::new();
        for p in files_with(&dir, suffixes) {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            // Qoder writes `glob:`; the parser reads globs/paths/applyTo/fileMatchPattern
            let (fm, body) = parse::frontmatter(&text);
            let text = match fm.get("glob") {
                Some(g) if !fm.contains_key("globs") => {
                    let mut fm2: Vec<(String, Value)> = fm
                        .iter()
                        .filter(|(k, _)| k.as_str() != "glob")
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    fm2.push(("globs".into(), g.clone()));
                    parse::render_frontmatter(&fm2, body)
                }
                _ => text,
            };
            let stem = stem_of(&p, suffixes);
            let entry = format!("{}.md", parse::sanitize_name(&stem));
            let files: parse::RawFiles = [(entry.clone(), text.into_bytes())].into_iter().collect();
            if let Ok(mut comp) = parse::parse_raw(ComponentKind::Rule, &entry, &files) {
                super::claude::tag_installed(&mut comp, target, &p);
                out.push(comp);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::convert::Registry;
    use crate::core::agentkit::targets;
    use crate::core::agentkit::{Env, Os};
    use std::collections::BTreeMap;
    use std::path::Path;

    fn rule(globs: &[&str]) -> Component {
        let fm = if globs.is_empty() {
            "---\ndescription: API rules\n---\n".to_string()
        } else {
            format!(
                "---\ndescription: API rules\npaths:\n{}---\n",
                globs
                    .iter()
                    .map(|g| format!("  - \"{g}\"\n"))
                    .collect::<String>()
            )
        };
        let text = format!("{fm}# API\n- validate input\n");
        let files: parse::RawFiles = [("api.md".to_string(), text.into_bytes())]
            .into_iter()
            .collect();
        let mut c = parse::parse_raw(ComponentKind::Rule, "api.md", &files).unwrap();
        c.id = "cct:rules/api".into();
        c
    }

    fn run(c: &Component, tid: &str) -> Vec<PlannedFile> {
        let env = Env::sandbox(Path::new("/h"), Os::Linux);
        let project = PathBuf::from("/p");
        let claude = targets::target("claude").unwrap();
        let empty = BTreeMap::new();
        let ctx = ConvertCtx {
            env: &env,
            project: Some(&project),
            scope: Scope::Project,
            claude,
            name_override: None,
            secret_values: &empty,
        };
        RuleConverter
            .convert(c, targets::target(tid).unwrap(), Scope::Project, &ctx)
            .unwrap()
    }

    #[test]
    fn root_rule_goes_to_agents_md_with_gemini_bridge() {
        let c = rule(&[]);
        let f = run(&c, "codex");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].path, PathBuf::from("/p/AGENTS.md"));
        let g = run(&c, "gemini");
        assert_eq!(g[0].path, PathBuf::from("/p/AGENTS.md"));
        assert_eq!(g[1].path, PathBuf::from("/p/GEMINI.md"));
        assert!(
            matches!(&g[1].ops[0], PatchOp::TextBlock { content, .. } if content == "@./AGENTS.md")
        );
        let q = run(&c, "qwen");
        assert_eq!(q[0].path, PathBuf::from("/p/AGENTS.md"));
        let a = run(&c, "aider");
        assert_eq!(a[0].path, PathBuf::from("/p/CONVENTIONS.md"));
        assert_eq!(a[1].path, PathBuf::from("/p/.aider.conf.yml"));
    }

    #[test]
    fn scoped_rules_per_dialect() {
        let c = rule(&["src/api/**/*.ts", "src/**/*.tsx"]);
        let cases = [
            (
                "cursor",
                "/p/.cursor/rules/api.mdc",
                "globs: src/api/**/*.ts,src/**/*.tsx",
            ),
            ("kiro", "/p/.kiro/steering/api.md", "inclusion: fileMatch"),
            ("devin", "/p/.devin/rules/api.md", "trigger: glob"),
            ("cline", "/p/.clinerules/api.md", "paths:"),
            (
                "continue",
                "/p/.continue/rules/api.md",
                "alwaysApply: false",
            ),
            ("qwen", "/p/.qwen/rules/api.md", "paths:"),
            ("trae", "/p/.trae/rules/api.md", "alwaysApply: false"),
            ("qoder", "/p/.qoder/rules/api.md", "trigger: glob"),
            ("antigravity", "/p/.agents/rules/api.md", "trigger: glob"),
        ];
        for (tid, path, needle) in cases {
            let f = run(&c, tid);
            assert_eq!(f[0].path, PathBuf::from(path), "{tid}");
            let text = String::from_utf8(f[0].content.clone().unwrap()).unwrap();
            assert!(text.contains(needle), "{tid}: {text}");
            assert!(f[0].losses.is_empty(), "{tid}: {:?}", f[0].losses);
        }
        // Copilot's scoped files (the registry sends Copilot rules to .claude/rules)
        let (file, text, _, _) = RuleConverter::scoped_file(
            "copilot_instructions",
            "api",
            match &c.body {
                ComponentBody::Rule(r) => r,
                _ => unreachable!(),
            },
        )
        .unwrap();
        assert_eq!(file, "api.instructions.md");
        let (fm, _) = parse::frontmatter(&text);
        assert_eq!(fm["applyTo"], "src/api/**/*.ts,src/**/*.tsx", "{text}");
        // OpenCode: file + instructions entry
        let o = run(&c, "opencode");
        assert_eq!(o[0].path, PathBuf::from("/p/.opencode/rules/api.md"));
        assert_eq!(o[1].path, PathBuf::from("/p/opencode.json"));
        assert!(
            matches!(&o[1].ops[0], PatchOp::Append { value, .. } if value == ".opencode/rules/api.md")
        );
        // no scoping at all: a section in AGENTS.md
        let z = run(&c, "crush");
        assert_eq!(z[0].path, PathBuf::from("/p/AGENTS.md"));
        assert!(
            matches!(&z[0].ops[0], PatchOp::TextBlock { content, .. } if content.starts_with("## When editing src/api/**/*.ts, src/**/*.tsx"))
        );
        assert!(!z[0].losses.is_empty());
    }

    #[test]
    fn registry_routes_rules_here() {
        let c = rule(&[]);
        let reg = Registry::global();
        for tid in ["codex", "gemini", "kiro", "aider", "goose", "qwen"] {
            match reg.resolve(ComponentKind::Rule, targets::target(tid).unwrap()) {
                crate::core::agentkit::convert::Resolution::Use {
                    converter,
                    via_claude,
                } => {
                    assert_eq!(converter.id(), "rule_files", "{tid}");
                    assert!(!via_claude);
                }
                crate::core::agentkit::convert::Resolution::Unsupported(r) => panic!("{tid}: {r}"),
            }
        }
        let _ = c;
    }
}
