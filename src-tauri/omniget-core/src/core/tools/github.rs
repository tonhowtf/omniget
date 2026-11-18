use std::path::Path;

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
            "could not check the releases of {}: HTTP {}",
            repo,
            response.status()
        ));
    }
    let json: serde_json::Value = response.json().await?;
    let tag = json["tag_name"].as_str().unwrap_or("").to_string();
    let assets = json["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("release of {} has no assets", repo))?;
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
        "release {} of {} has no asset for this system",
        tag,
        repo
    ))
}

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
                "[tools] {} came without a digest; accepted without verification",
                asset.name
            )
        }
        None => {
            return Err(anyhow!(
                "{} came without a digest from the GitHub API; download discarded",
                asset.name
            ))
        }
    }
    Ok(bytes)
}

#[cfg(unix)]
pub fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
pub fn make_executable(_path: &Path) {}

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
