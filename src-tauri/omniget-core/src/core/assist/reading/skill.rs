//! The embedded film-curation skill for reading companions (spec 05: method
//! of three roles, recommendation format, access verification).
//!
//! On 2026-09-24 no curation skill was installed on the owner's machine
//! (`<app_data>/llm/skills` did not exist; `~/.claude/skills` had none about
//! films or reading), so the app ships this one and installs it through the
//! ordinary skill installer (staging + scan + sidecar), where it shows up in
//! the catalog like any other skill. The sources live next to this module as
//! `*.md.fixture` files and become `SKILL.md` / `references/*.md` only in a
//! temporary folder at install time.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::skills::{self, install, manifest::SKILL_FILE};

pub const SKILL_NAME: &str = "curadoria-filmes-leitura";
pub const ERR_READING_SKILL: &str = "ERR_READING_SKILL";

const SKILL_MD: &str = include_str!("../reading_skill/SKILL.md.fixture");
const REFERENCES: &[(&str, &str)] = &[
    (
        "references/verificacao-acesso.md",
        include_str!("../reading_skill/references/verificacao-acesso.md.fixture"),
    ),
    (
        "references/formato-indicacao.md",
        include_str!("../reading_skill/references/formato-indicacao.md.fixture"),
    ),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillStatus {
    pub name: String,
    pub installed: bool,
    /// Installed copy differs from the one this build ships (edited by the
    /// user, or an older version).
    pub differs: bool,
    pub embedded_hash: String,
    pub installed_hash: Option<String>,
}

pub fn embedded_hash() -> String {
    skills::catalog::sha256_hex(SKILL_MD.as_bytes())
}

pub fn status_in(root: &Path) -> SkillStatus {
    let installed_hash = install::skill_path(root, SKILL_NAME)
        .ok()
        .and_then(|d| std::fs::read(d.join(SKILL_FILE)).ok())
        .map(|b| skills::catalog::sha256_hex(&b));
    let embedded = embedded_hash();
    SkillStatus {
        name: SKILL_NAME.into(),
        installed: installed_hash.is_some(),
        differs: installed_hash
            .as_deref()
            .map(|h| h != embedded)
            .unwrap_or(false),
        embedded_hash: embedded,
        installed_hash,
    }
}

pub fn status() -> Result<SkillStatus, String> {
    let root = install::skills_dir().map_err(|e| format!("{ERR_READING_SKILL}: {e:?}"))?;
    Ok(status_in(&root))
}

/// Writes the skill folder into a fresh temporary directory.
fn materialise() -> Result<PathBuf, String> {
    let base =
        std::env::temp_dir().join(format!("omniget-reading-skill-{}", super::super::new_id()));
    let dir = base.join(SKILL_NAME);
    let io = |e: std::io::Error| format!("{ERR_READING_SKILL}: writing the skill: {e}");
    std::fs::create_dir_all(dir.join("references")).map_err(io)?;
    std::fs::write(dir.join(SKILL_FILE), SKILL_MD).map_err(io)?;
    for (rel, body) in REFERENCES {
        std::fs::write(dir.join(rel), body).map_err(io)?;
    }
    Ok(dir)
}

/// Installs the embedded skill into `root` with `scan`. An installed copy
/// that differs is left alone unless `replace` is true (it may be the user's
/// own edit).
pub fn install_in_with(
    root: &Path,
    replace: bool,
    scan: &install::ScanFn<'_>,
) -> Result<serde_json::Value, String> {
    let st = status_in(root);
    if st.installed && !st.differs {
        return Ok(serde_json::json!({ "status": st, "already_installed": true }));
    }
    if st.installed && st.differs && !replace {
        return Err(format!(
            "{ERR_READING_SKILL}: `{SKILL_NAME}` is installed with other contents (maybe your own edits); confirm to replace it"
        ));
    }
    let dir = materialise()?;
    let out = install::install_from_dir_in_with(root, &dir, scan);
    let _ = std::fs::remove_dir_all(dir.parent().unwrap_or(&dir));
    let out = out.map_err(|e| format!("{ERR_READING_SKILL}: {e:?}"))?;
    Ok(serde_json::json!({
        "status": status_in(root),
        "already_installed": false,
        "outcome": out,
    }))
}

/// Installs into the app's skills folder with the production scan.
pub fn install(replace: bool) -> Result<serde_json::Value, String> {
    let root = install::skills_dir().map_err(|e| format!("{ERR_READING_SKILL}: {e:?}"))?;
    install_in_with(&root, replace, &|d: &Path| skills::scan::scan_dir(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_skill_parses_installs_and_is_listed_with_its_references() {
        let root = std::env::temp_dir().join(format!(
            "omniget-reading-skill-root-{}",
            super::super::super::new_id()
        ));
        let not_scanned = |_: &Path| skills::ScanStatus::NotScanned;
        let v = install_in_with(&root, false, &not_scanned).unwrap();
        assert_eq!(v["already_installed"], false);
        let listed = install::list_in(&root);
        let m = listed
            .iter()
            .find(|m| m.name == SKILL_NAME)
            .expect("listed");
        assert!(m.description.contains("três papéis"));
        assert!(m.path.join("references/verificacao-acesso.md").is_file());
        let st = status_in(&root);
        assert!(st.installed && !st.differs);
        // Again: nothing to do.
        let v = install_in_with(&root, false, &not_scanned).unwrap();
        assert_eq!(v["already_installed"], true);
        // A user edit is not overwritten without consent.
        std::fs::write(
            m.path.join(SKILL_FILE),
            SKILL_MD.replace("Você acompanha", "Você (editado) acompanha"),
        )
        .unwrap();
        assert!(install_in_with(&root, false, &not_scanned).is_err());
        install_in_with(&root, true, &not_scanned).unwrap();
        assert!(!status_in(&root).differs);
    }
}
