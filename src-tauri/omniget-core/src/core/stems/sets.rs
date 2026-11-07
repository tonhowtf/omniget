use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StemSet {
    pub source: String,
    pub vocals: String,
    pub instrumental: String,
    pub model: String,
    pub created_at: i64,
}

impl StemSet {
    pub fn files_exist(&self) -> bool {
        Path::new(&self.vocals).is_file() && Path::new(&self.instrumental).is_file()
    }
}

pub fn load(file: &Path) -> Vec<StemSet> {
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(file: &Path, sets: &[StemSet]) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(sets)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, file).with_context(|| format!("moving to {}", file.display()))?;
    Ok(())
}

pub fn upsert(sets: &mut Vec<StemSet>, set: StemSet) {
    sets.retain(|s| s.instrumental != set.instrumental && s.vocals != set.vocals);
    sets.push(set);
}

pub fn prune(sets: &mut Vec<StemSet>) -> bool {
    let before = sets.len();
    sets.retain(StemSet::files_exist);
    sets.len() != before
}

pub fn index_file(music_dir: &Path) -> PathBuf {
    music_dir.join("stems.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(dir: &Path, name: &str) -> StemSet {
        let v = dir.join(format!("{name} (Vocals).flac"));
        let i = dir.join(format!("{name} (Instrumental).flac"));
        std::fs::write(&v, b"v").unwrap();
        std::fs::write(&i, b"i").unwrap();
        StemSet {
            source: dir.join(format!("{name}.mp3")).to_string_lossy().into(),
            vocals: v.to_string_lossy().into(),
            instrumental: i.to_string_lossy().into(),
            model: "spleeter-2stems".into(),
            created_at: 1,
        }
    }

    #[test]
    fn round_trips_and_replaces_by_output() {
        let dir = tempfile::tempdir().unwrap();
        let file = index_file(dir.path());
        assert!(load(&file).is_empty());
        let mut sets = Vec::new();
        upsert(&mut sets, set(dir.path(), "a"));
        let mut again = set(dir.path(), "a");
        again.created_at = 2;
        upsert(&mut sets, again);
        upsert(&mut sets, set(dir.path(), "b"));
        save(&file, &sets).unwrap();
        let back = load(&file);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].created_at, 2);
    }

    #[test]
    fn prune_drops_sets_whose_files_are_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut sets = vec![set(dir.path(), "a"), set(dir.path(), "b")];
        std::fs::remove_file(&sets[0].vocals).unwrap();
        assert!(prune(&mut sets));
        assert_eq!(sets.len(), 1);
        assert!(!prune(&mut sets));
    }

    #[test]
    fn a_broken_index_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = index_file(dir.path());
        std::fs::write(&file, b"{oops").unwrap();
        assert!(load(&file).is_empty());
    }
}
