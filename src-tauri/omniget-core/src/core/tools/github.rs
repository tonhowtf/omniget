//! Releases do GitHub: escolher o asset certo para este sistema, baixar,
//! conferir o SHA-256 que a API informa e desempacotar. É o mesmo desenho do
//! Spicetify (`core/spicetify.rs`), generalizado para whisper.cpp,
//! Real-ESRGAN e afins.

use std::path::{Path, PathBuf};

use anyhow::anyhow;

use crate::core::dependencies::integrity;

pub fn client() -> anyhow::Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("OmniGet"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    Ok(
        crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(600))
            .build()?,
    )
}

#[derive(Debug, Clone)]
pub struct ReleaseAsset {
    pub tag: String,
    pub name: String,
    pub url: String,
    pub size: u64,
    pub digest: Option<String>,
}

/// Primeiro asset da release `tag` (ou da última) que `pick` aceitar.
pub async fn asset(
    client: &reqwest::Client,
    repo: &str,
    tag: Option<&str>,
    pick: impl Fn(&str) -> bool,
) -> anyhow::Result<ReleaseAsset> {
    let url = match tag {
        Some(t) => format!("https://api.github.com/repos/{}/releases/tags/{}", repo, t),
        None => format!("https://api.github.com/repos/{}/releases/latest", repo),
    };
    let response = client.get(&url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "nao foi possivel consultar releases de {}: HTTP {}",
            repo,
            response.status()
        ));
    }
    let json: serde_json::Value = response.json().await?;
    let tag = json["tag_name"].as_str().unwrap_or("").to_string();
    let assets = json["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("release de {} sem assets", repo))?;
    for a in assets {
        let name = a["name"].as_str().unwrap_or("");
        if pick(name) {
            return Ok(ReleaseAsset {
                tag: tag.clone(),
                name: name.to_string(),
                url: a["browser_download_url"].as_str().unwrap_or("").to_string(),
                size: a["size"].as_u64().unwrap_or(0),
                digest: a["digest"]
                    .as_str()
                    .and_then(integrity::parse_github_digest),
            });
        }
    }
    Err(anyhow!(
        "release {} de {} nao tem um asset para este sistema",
        tag,
        repo
    ))
}

/// Baixa e confere o hash. Sem digest na API o download é aceito só se
/// `allow_unverified` (repos antigos não publicam digest).
pub async fn download(
    client: &reqwest::Client,
    asset: &ReleaseAsset,
    allow_unverified: bool,
    progress: &super::ProgressFn,
    id: &str,
) -> anyhow::Result<Vec<u8>> {
    let tmp = super::temp_dir().join(format!("{}.download", asset.name));
    super::download_to(client, &asset.url, &tmp, progress, id).await?;
    let bytes = tokio::fs::read(&tmp).await?;
    let _ = tokio::fs::remove_file(&tmp).await;
    match asset.digest.as_deref() {
        Some(expected) => integrity::verify_sha256(&bytes, expected, &asset.name)?,
        None if allow_unverified => {
            tracing::warn!(
                "[tools] {} veio sem digest; aceito sem verificacao",
                asset.name
            )
        }
        None => {
            return Err(anyhow!(
                "{} veio sem digest da API do GitHub; download descartado",
                asset.name
            ))
        }
    }
    Ok(bytes)
}

pub fn unpack(data: &[u8], name: &str, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    if name.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))
            .map_err(|e| anyhow!("zip invalido: {}", e))?;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let Some(rel) = file.enclosed_name() else {
                continue;
            };
            let out = dest.join(rel);
            if file.is_dir() {
                std::fs::create_dir_all(&out)?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut w = std::fs::File::create(&out)?;
            std::io::copy(&mut file, &mut w)?;
        }
        Ok(())
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(data));
        let mut archive = tar::Archive::new(decoder);
        archive.set_preserve_permissions(true);
        archive
            .unpack(dest)
            .map_err(|e| anyhow!("tar.gz invalido: {}", e))
    } else if name.ends_with(".tar.xz") {
        let decoder = xz2::read::XzDecoder::new(std::io::Cursor::new(data));
        let mut archive = tar::Archive::new(decoder);
        archive.set_preserve_permissions(true);
        archive
            .unpack(dest)
            .map_err(|e| anyhow!("tar.xz invalido: {}", e))
    } else {
        Err(anyhow!("formato de pacote desconhecido: {}", name))
    }
}

/// Procura um arquivo pelo nome dentro de uma árvore (os zips do whisper.cpp
/// e do Real-ESRGAN têm subpastas diferentes por plataforma).
pub fn find_file(root: &Path, file_name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().map(|n| n == file_name).unwrap_or(false) {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(unix)]
pub fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
pub fn make_executable(_path: &Path) {}

/// Troca `dir` pela pasta nova só depois que ela está inteira no disco.
pub fn swap_dir(staging: &Path, dir: &Path) -> anyhow::Result<()> {
    let old = dir.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    if dir.exists() {
        std::fs::rename(dir, &old)?;
    }
    if let Err(e) = std::fs::rename(staging, dir) {
        if old.exists() {
            let _ = std::fs::rename(&old, dir);
        }
        return Err(anyhow!(
            "nao foi possivel mover a instalacao para o lugar: {}",
            e
        ));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

#[cfg(target_os = "macos")]
pub async fn strip_quarantine(dir: &Path) {
    let target = dir.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        crate::core::process::std_command("xattr")
            .args(["-dr", "com.apple.quarantine"])
            .arg(&target)
            .output()
    })
    .await;
}

#[cfg(not(target_os = "macos"))]
pub async fn strip_quarantine(_dir: &Path) {}
