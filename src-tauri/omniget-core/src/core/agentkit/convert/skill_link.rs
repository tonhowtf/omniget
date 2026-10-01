//! Skills installed once in the canonical store `.agents/skills/<name>/`
//! (project) or `~/.agents/skills/<name>/` (global), plus a copy in the skills
//! folder of every tool that does not read `.agents` (estudo 06 §0C.7: Claude,
//! Kiro, Continue, Letta, Rovo, Roo, Droid …), the `npx skills` model.
//!
//! The tool's folder is a symlink to the store ([`FileAction::Link`]); the
//! writer copies the store instead where links cannot be made (Windows without
//! developer mode). Store files and the link are recorded in the lockfile, and a
//! store shared by several tools is only removed with its last installer (the
//! writer's `created_elsewhere`).
//!
//! [`FileAction::Link`]: super::FileAction::Link
//!
//! This converter answers to the id `skill_dir`, so `Registry::resolve` (which
//! looks skills up by that id) picks it ahead of the F0 folder copier.

use std::path::{Path, PathBuf};

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::model::*;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

pub struct SkillLinkConverter;

/// Canonical store for a scope.
pub fn canonical_dir(scope: Scope, ctx: &ConvertCtx) -> Option<PathBuf> {
    match scope {
        Scope::Project | Scope::Local => ctx.project.map(|p| p.join(".agents").join("skills")),
        Scope::Global => Some(ctx.env.home.join(".agents").join("skills")),
        Scope::Managed => None,
    }
}

/// Does the tool read the canonical store for this scope? Data-driven: the
/// store must be the tool's skills path or one of its alternatives.
pub fn reads_canonical(target: &TargetAdapter, scope: Scope, ctx: &ConvertCtx) -> bool {
    let Some(canon) = canonical_dir(scope, ctx) else {
        return false;
    };
    let Some(spec) = target.paths.get("skills") else {
        return false;
    };
    let lookup = if scope == Scope::Local {
        Scope::Project
    } else {
        scope
    };
    let mut templates: Vec<&str> = spec.template(lookup, ctx.env.os).into_iter().collect();
    templates.extend(spec.alternatives(lookup).iter().map(String::as_str));
    templates
        .iter()
        .filter_map(|t| ctx.env.expand(t, ctx.project))
        .any(|p| p == canon)
}

/// Every file of the skill folder placed under `root`.
fn skill_files(target: &str, c: &Component, root: &Path, label: &str) -> Vec<PlannedFile> {
    let prefix = c
        .entry
        .rsplit_once('/')
        .map(|(d, _)| format!("{}/", d.trim_end_matches('/')))
        .unwrap_or_default();
    let mut out = Vec::new();
    for f in &c.files {
        let Some(rel) = f.path.strip_prefix(&prefix) else {
            continue;
        };
        if rel.split('/').any(|p| p == ".." || p.is_empty()) {
            continue;
        }
        let path = rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s));
        let mut pf = PlannedFile::write(target, c, path, f.bytes.clone(), label);
        pf.executable = f.executable;
        pf.unit_root = Some(root.to_path_buf());
        pf.primary = f.path == c.entry;
        out.push(pf);
    }
    out
}

impl Converter for SkillLinkConverter {
    fn id(&self) -> &'static str {
        "skill_dir"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Skill && target.format(kind) == Some("skill_dir")
    }

    fn native_for(&self, _c: &Component, _target: &TargetAdapter) -> bool {
        true
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let ComponentBody::Skill(s) = &c.body else {
            return Ok(vec![]);
        };
        let dir_name = ctx
            .name_override
            .clone()
            .unwrap_or_else(|| s.dir_name.clone());
        let own = kind_path(target, "skills", scope, ctx);
        let canon = canonical_dir(scope, ctx);
        let Some(canon) = canon else {
            // managed scope: the tool's own folder only
            let base = own.ok_or_else(|| no_path(target, "skills", scope))?;
            return Ok(skill_files(&target.id, c, &base.join(&dir_name), "skill"));
        };
        let mut out = skill_files(&target.id, c, &canon.join(&dir_name), "skill");
        if !reads_canonical(target, scope, ctx) {
            if let Some(base) = own.filter(|b| *b != canon) {
                let mut link = PlannedFile::link(
                    &target.id,
                    c,
                    base.join(&dir_name),
                    canon.join(&dir_name),
                    "skill (link)",
                );
                link.notes.push(format!(
                    "{} does not read .agents/skills: its skills folder gets a link to the store (a copy where links are not allowed)",
                    target.name
                ));
                // the store keeps the name; the tool's link is what it loads
                for f in out.iter_mut() {
                    f.primary = false;
                }
                out.push(link);
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
        if kind != ComponentKind::Skill {
            return Ok(vec![]);
        }
        let mut out: Vec<Component> =
            super::claude::ClaudeConverter::own_paths().import(kind, target, scope, ctx)?;
        if reads_canonical(target, scope, ctx) {
            if let Some(canon) = canonical_dir(scope, ctx) {
                if let Ok(rd) = std::fs::read_dir(&canon) {
                    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                    dirs.sort();
                    for d in dirs {
                        if !d.join("SKILL.md").is_file() {
                            continue;
                        }
                        if let Ok(mut comp) =
                            crate::core::agentkit::parse::parse_path(ComponentKind::Skill, &d)
                        {
                            super::claude::tag_installed(&mut comp, target, &d);
                            if !out.iter().any(|x| x.id == comp.id) {
                                out.push(comp);
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}
