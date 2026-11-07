use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde::Serialize;

use crate::core::tools::ProgressFn;

pub const DEFAULT_MODEL: &str = "spleeter-2stems";

pub const DIR_ENV: &str = "OMNIGET_STEMS_MODEL_DIR";

const HOSTS: &[&str] = &["https://huggingface.co", "https://hf-mirror.com"];

#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub stem: &'static str,
    pub local: &'static str,
    pub remote: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub repo: &'static str,
    pub rev: &'static str,
    pub license: &'static str,
    pub source: &'static str,
    pub files: &'static [ModelFile],
}

pub const MODELS: &[ModelSpec] = &[ModelSpec {
    id: "spleeter-2stems",
    label: "Spleeter 2 stems (voice + instrumental)",
    repo: "csukuangfj/sherpa-onnx-spleeter-2stems",
    rev: "7001ba316a615cacddb3f9ef3ec416661a277e26",
    license: "MIT",
    source: "https://github.com/deezer/spleeter",
    files: &[
        ModelFile {
            stem: "vocals",
            local: "vocals.onnx",
            remote: "vocals.onnx",
            bytes: 39_318_336,
            sha256: "bdc16ab6bf6117ddd4842c19e80e40e2be188fc555295064d424616b0224ac97",
        },
        ModelFile {
            stem: "instrumental",
            local: "accompaniment.onnx",
            remote: "accompaniment.onnx",
            bytes: 39_318_343,
            sha256: "671ace17acd3720674a2bc14de32ac6292453dec20d9eb0ba4255d4ad8e3d8c0",
        },
    ],
}];

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id.trim())
}

pub fn pick(id: Option<&str>) -> anyhow::Result<&'static ModelSpec> {
    let id = id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MODEL);
    find(id).ok_or_else(|| anyhow!("unknown stem model `{id}`"))
}

pub fn root_dir() -> Option<PathBuf> {
    match std::env::var(DIR_ENV).ok().filter(|s| !s.trim().is_empty()) {
        Some(s) => Some(PathBuf::from(s.trim())),
        None => crate::core::tools::onnx::models_dir().map(|d| d.join("stems")),
    }
}

pub fn model_dir(m: &ModelSpec) -> Option<PathBuf> {
    root_dir().map(|d| d.join(m.id))
}

pub fn file_path(m: &ModelSpec, f: &ModelFile) -> Option<PathBuf> {
    model_dir(m).map(|d| d.join(f.local))
}

fn has_file(m: &ModelSpec, f: &ModelFile) -> bool {
    file_path(m, f)
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|md| md.is_file() && md.len() == f.bytes)
        .unwrap_or(false)
}

pub fn is_downloaded(m: &ModelSpec) -> bool {
    m.files.iter().all(|f| has_file(m, f))
}

pub fn total_bytes(m: &ModelSpec) -> u64 {
    m.files.iter().map(|f| f.bytes).sum()
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub license: String,
    pub source: String,
    pub stems: Vec<String>,
    pub downloaded: bool,
    pub default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub runtime_installed: bool,
    pub runtime_can_download: bool,
    pub models: Vec<ModelStatus>,
}

pub fn status() -> Status {
    let rt = crate::core::onnxrt::status();
    Status {
        runtime_installed: rt.installed,
        runtime_can_download: rt.can_download,
        models: MODELS
            .iter()
            .map(|m| ModelStatus {
                id: m.id.to_string(),
                label: m.label.to_string(),
                bytes: total_bytes(m),
                license: m.license.to_string(),
                source: m.source.to_string(),
                stems: m.files.iter().map(|f| f.stem.to_string()).collect(),
                downloaded: is_downloaded(m),
                default: m.id == DEFAULT_MODEL,
            })
            .collect(),
    }
}

fn sha256_of(path: &Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn urls(m: &ModelSpec, f: &ModelFile) -> Vec<String> {
    HOSTS
        .iter()
        .map(|h| format!("{}/{}/resolve/{}/{}", h, m.repo, m.rev, f.remote))
        .collect()
}

async fn fetch(
    client: &reqwest::Client,
    m: &ModelSpec,
    f: &ModelFile,
    progress: &ProgressFn,
) -> anyhow::Result<()> {
    if has_file(m, f) {
        return Ok(());
    }
    let dest = file_path(m, f).ok_or_else(|| anyhow!("no app data directory"))?;
    let dir = dest
        .parent()
        .ok_or_else(|| anyhow!("model path without a parent"))?
        .to_path_buf();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!(".{}.download", f.local));
    let id = format!("{}:{}", m.id, f.local);

    let mut last_err = None;
    for url in urls(m, f) {
        match crate::core::tools::download_to(client, &url, &tmp, progress, &id).await {
            Ok(_) => {
                let expected = f.sha256;
                let tmp2 = tmp.clone();
                let got = tokio::task::spawn_blocking(move || sha256_of(&tmp2))
                    .await
                    .map_err(|e| anyhow!("hash task died: {e}"))??;
                if got == expected {
                    if dest.exists() {
                        let _ = std::fs::remove_file(&dest);
                    }
                    std::fs::rename(&tmp, &dest)
                        .with_context(|| format!("moving to {}", dest.display()))?;
                    return Ok(());
                }
                let _ = std::fs::remove_file(&tmp);
                last_err = Some(anyhow!(
                    "{url}: checksum mismatch (expected {expected}, got {got})"
                ));
            }
            Err(e) => {
                tracing::warn!("[stems] download of {} failed: {}", url, e);
                last_err = Some(anyhow!("{url}: {e}"));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("no URL for {}", f.local)))
}

pub async fn install(m: &ModelSpec, progress: &ProgressFn) -> anyhow::Result<PathBuf> {
    if !crate::core::onnxrt::is_installed() {
        crate::core::onnxrt::install_runtime(None, progress).await?;
    }
    let client = crate::core::tools::client()?;
    for f in m.files {
        fetch(&client, m, f, progress).await?;
    }
    model_dir(m).ok_or_else(|| anyhow!("no app data directory"))
}

pub fn remove(m: &ModelSpec) -> anyhow::Result<()> {
    if let Some(dir) = model_dir(m).filter(|d| d.is_dir()) {
        std::fs::remove_dir_all(&dir).with_context(|| format!("deleting {}", dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_pins_a_revision_and_checksums() {
        for m in MODELS {
            assert_eq!(m.rev.len(), 40, "{} must pin a commit", m.id);
            assert!(!m.license.is_empty());
            for f in m.files {
                assert_eq!(f.sha256.len(), 64);
                assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit()));
                assert!(f.bytes > 0);
                for u in urls(m, f) {
                    assert!(u.contains(m.rev), "{u}");
                }
            }
        }
    }

    #[test]
    fn pick_falls_back_to_default() {
        assert_eq!(pick(None).unwrap().id, DEFAULT_MODEL);
        assert_eq!(pick(Some("  ")).unwrap().id, DEFAULT_MODEL);
        assert!(pick(Some("nope")).is_err());
    }
}
