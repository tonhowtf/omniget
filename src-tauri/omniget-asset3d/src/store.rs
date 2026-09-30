//! Local asset store: `<root>/<hash>/{asset.glb, meta.json, thumb.png}`,
//! where `<root>` is `<app_data>/city3d/assets` in the app.

use crate::types::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AssetSource {
    /// `file`, `bytes`, `image`, `tripo`, …
    pub kind: String,
    /// Original file name (no path).
    pub name: String,
    pub format: String,
    /// SHA-256 of the original bytes.
    pub source_hash: String,
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AssetMeta {
    pub id: String,
    pub name: String,
    pub class: AssetClass,
    pub license: License,
    pub source: AssetSource,
    pub footprint: Footprint,
    pub collision: Collision,
    pub anchors: Vec<AssetAnchor>,
    pub front: String,
    #[serde(default)]
    pub vrm: Option<VrmInfo>,
    pub bytes: usize,
    pub triangles: usize,
    pub fit_style: bool,
    pub created_at: u64,
    pub normalizer: String,
    pub report: Report,
}

pub struct Store {
    pub root: PathBuf,
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    fn dir(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_id(id) {
            return Err("bad asset id".into());
        }
        Ok(self.root.join(id))
    }

    pub fn save(
        &self,
        asset: &NormalizedAsset,
        license: License,
        source: AssetSource,
        fit_style: bool,
    ) -> Result<AssetMeta, String> {
        let dir = self.dir(&asset.hash)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let meta = AssetMeta {
            id: asset.hash.clone(),
            name: asset.name.clone(),
            class: asset.class,
            license,
            source,
            footprint: asset.footprint.clone(),
            collision: asset.collision.clone(),
            anchors: asset.anchors.clone(),
            front: asset.front.clone(),
            vrm: asset.vrm.clone(),
            bytes: asset.glb.len(),
            triangles: asset.report.output.triangles,
            fit_style,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            normalizer: NORMALIZER_VERSION.into(),
            report: asset.report.clone(),
        };
        write_atomic(&dir.join("asset.glb"), &asset.glb)?;
        write_atomic(&dir.join("thumb.png"), &asset.thumbnail_png)?;
        let json = serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?;
        write_atomic(&dir.join("meta.json"), &json)?;
        Ok(meta)
    }

    pub fn list(&self) -> Vec<AssetMeta> {
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !valid_id(&name) {
                continue;
            }
            if let Ok(b) = std::fs::read(e.path().join("meta.json")) {
                if let Ok(m) = serde_json::from_slice::<AssetMeta>(&b) {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        out
    }

    pub fn meta(&self, id: &str) -> Result<AssetMeta, String> {
        let b = std::fs::read(self.dir(id)?.join("meta.json"))
            .map_err(|_| "asset not found".to_string())?;
        serde_json::from_slice(&b).map_err(|e| e.to_string())
    }

    pub fn glb(&self, id: &str) -> Result<Vec<u8>, String> {
        std::fs::read(self.dir(id)?.join("asset.glb")).map_err(|_| "asset not found".to_string())
    }

    pub fn thumbnail(&self, id: &str) -> Result<Vec<u8>, String> {
        std::fs::read(self.dir(id)?.join("thumb.png")).map_err(|_| "asset not found".to_string())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let d = self.dir(id)?;
        if d.is_dir() {
            std::fs::remove_dir_all(d).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}
