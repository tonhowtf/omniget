use std::path::{Path, PathBuf};

use crate::core::cas::{ingest, materialize, same_file, LinkOutcome};

fn quantos_links(object: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        crate::core::cas::link_count(object)
    }
    #[cfg(not(unix))]
    {
        let _ = object;
        None
    }
}

fn store_root() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("content-store"))
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DedupeReport {
    pub examined: usize,
    pub deduplicated: usize,
    pub bytes_saved: u64,
    pub savings_measurable: bool,
    pub errors: Vec<String>,
}

#[tauri::command]
pub async fn deduplicate_files(paths: Vec<String>) -> Result<DedupeReport, String> {
    let root = store_root().ok_or_else(|| "no data folder".to_string())?;

    tokio::task::spawn_blocking(move || {
        let mut rep = DedupeReport {
            savings_measurable: cfg!(unix),
            ..Default::default()
        };
        for raw in paths {
            let path = PathBuf::from(&raw);
            if !path.is_file() {
                continue;
            }
            rep.examined += 1;

            let tamanho = match std::fs::metadata(&path) {
                Ok(m) => m.len(),
                Err(e) => {
                    rep.errors.push(format!("{}: {}", path.display(), e));
                    continue;
                }
            };

            let hash = match sha256_of(&path) {
                Ok(h) => h,
                Err(e) => {
                    rep.errors.push(format!("{}: {}", path.display(), e));
                    continue;
                }
            };

            let objeto = match ingest(&root, &path, &hash) {
                Ok(o) => o,
                Err(e) => {
                    rep.errors.push(format!("{}: {}", path.display(), e));
                    continue;
                }
            };

            if same_file(&objeto, &path) {
                continue;
            }

            match materialize(&objeto, &path) {
                Ok(LinkOutcome::Linked) => {
                    rep.deduplicated += 1;
                    if rep.savings_measurable {
                        rep.bytes_saved += tamanho;
                    }
                }
                Ok(LinkOutcome::AlreadyPresent) => {}
                Ok(LinkOutcome::Copied) => {}
                Err(e) => rep.errors.push(format!("{}: {}", path.display(), e)),
            }
        }
        Ok(rep)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StoreStats {
    pub objects: usize,
    pub bytes_on_disk: u64,
    pub bytes_without_dedupe: u64,
    pub savings_measurable: bool,
}

#[tauri::command]
pub async fn content_store_stats() -> Result<StoreStats, String> {
    let Some(root) = store_root() else {
        return Ok(StoreStats::default());
    };
    tokio::task::spawn_blocking(move || {
        let mut s = StoreStats {
            savings_measurable: cfg!(unix),
            ..Default::default()
        };
        let Ok(niveis) = std::fs::read_dir(&root) else {
            return s;
        };
        for nivel in niveis.filter_map(|e| e.ok()) {
            let Ok(objetos) = std::fs::read_dir(nivel.path()) else {
                continue;
            };
            for obj in objetos.filter_map(|e| e.ok()) {
                let Ok(meta) = obj.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                s.objects += 1;
                s.bytes_on_disk += meta.len();
                let links = quantos_links(&obj.path()).unwrap_or(1).max(1);
                s.bytes_without_dedupe += meta.len() * links;
            }
        }
        s
    })
    .await
    .map_err(|e| e.to_string())
}
