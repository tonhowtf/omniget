//! Saved script typologies: `<dir>/<hash>.json` with the name, class and
//! script. A building of the city program points at one as
//! `codegeo:<hash>` in `BuildingSpec.typology`.

use crate::{run_script, Anchor, Footprint, ScriptError};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Typology {
    pub name: String,
    pub class: String,
    pub script: String,
    pub hash: String,
    /// `codegeo:<hash>`.
    pub typology: String,
    pub triangles: usize,
    pub footprint: Footprint,
    pub anchors: Vec<Anchor>,
    pub created_ms: u64,
}

fn id_hash(id: &str) -> Option<&str> {
    let h = id.strip_prefix("codegeo:").unwrap_or(id);
    (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then_some(h)
}

/// Runs and validates the script, then stores it. Saving the same recipe
/// again only renames it.
pub fn save(
    dir: &Path,
    name: &str,
    script: &str,
    class: &str,
) -> Result<Typology, Vec<ScriptError>> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(vec![ScriptError::plain(
            "validation",
            "the name must have 1 to 80 characters".into(),
        )]);
    }
    let out = run_script(script, class)?;
    let t = Typology {
        name: name.to_string(),
        class: out.class.clone(),
        script: script.to_string(),
        hash: out.hash.clone(),
        typology: out.typology.clone(),
        triangles: out.triangles,
        footprint: out.footprint.clone(),
        anchors: out.anchors.clone(),
        created_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    };
    let io = |e: std::io::Error| vec![ScriptError::plain("cache", format!("could not save: {e}"))];
    std::fs::create_dir_all(dir).map_err(io)?;
    let path = dir.join(format!("{}.json", t.hash));
    let tmp = dir.join(format!("{}.json.tmp", t.hash));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&t).unwrap_or_default()).map_err(io)?;
    std::fs::rename(&tmp, &path).map_err(io)?;
    Ok(t)
}

pub fn list(dir: &Path) -> Vec<Typology> {
    let mut out: Vec<Typology> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a: &Typology, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.hash.cmp(&b.hash))
    });
    out
}

/// By `codegeo:<hash>` or the bare hash.
pub fn load(dir: &Path, id: &str) -> Option<Typology> {
    let h = id_hash(id)?;
    serde_json::from_slice(&std::fs::read(dir.join(format!("{h}.json"))).ok()?).ok()
}

pub fn delete(dir: &Path, id: &str) -> bool {
    match id_hash(id) {
        Some(h) => std::fs::remove_file(dir.join(format!("{h}.json"))).is_ok(),
        None => false,
    }
}
