//! Mídia em lote (estudo 67): todas as fotos e videos de um perfil (ou de
//! uma lista de posts) em qualidade original, como o
//! Twitter-X-Media-Batch-Downloader e o "Export Media" do
//! twitter-web-exporter. 1 arquivo por vez com pausa curta.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::{ProgressFn, XPost};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaResult {
    pub files: Vec<String>,
    pub skipped: usize,
    pub failed: usize,
    pub posts: usize,
    pub dest: String,
    pub cancelled: bool,
}

fn ext_of(url: &str, kind: &str) -> String {
    if let Some(fmt) = url
        .split("format=")
        .nth(1)
        .and_then(|s| s.split('&').next())
    {
        return fmt.to_string();
    }
    let path = url.split('?').next().unwrap_or(url);
    path.rsplit('.')
        .next()
        .filter(|e| e.len() <= 4)
        .map(|e| e.to_string())
        .unwrap_or_else(|| {
            if kind == "photo" {
                "jpg".into()
            } else {
                "mp4".into()
            }
        })
}

pub fn file_name(post: &XPost, index: usize, url: &str, kind: &str) -> String {
    let date = post
        .created_at
        .get(..10)
        .unwrap_or("0000-00-00")
        .replace('-', "");
    format!(
        "{}_{}_{}_{}.{}",
        date,
        post.author.handle,
        post.id,
        index + 1,
        ext_of(url, kind)
    )
}

/// Baixa a midia de `posts` para `dest`. `job` e o id de progresso/cancelamento.
pub async fn download_posts(
    posts: &[XPost],
    dest: &std::path::Path,
    photos: bool,
    videos: bool,
    job: &str,
    progress: &ProgressFn,
) -> anyhow::Result<MediaResult> {
    std::fs::create_dir_all(dest)?;
    let client = super::super::client()?;
    let mut result = MediaResult {
        dest: dest.to_string_lossy().to_string(),
        posts: posts.len(),
        ..Default::default()
    };
    let total: usize = posts
        .iter()
        .map(|p| {
            p.media
                .iter()
                .filter(|m| (photos && m.kind == "photo") || (videos && m.kind != "photo"))
                .count()
        })
        .sum();
    let mut done = 0usize;
    for post in posts {
        for (i, m) in post.media.iter().enumerate() {
            if super::cancelled(job) {
                result.cancelled = true;
                super::report(progress, job, "done", done as u64, Some(total as u64), None);
                return Ok(result);
            }
            let wanted = (photos && m.kind == "photo") || (videos && m.kind != "photo");
            if !wanted {
                continue;
            }
            let name = file_name(post, i, &m.url, &m.kind);
            let path = dest.join(&name);
            if path.exists() {
                result.skipped += 1;
                done += 1;
                continue;
            }
            super::report(
                progress,
                job,
                "download",
                done as u64,
                Some(total as u64),
                Some(name.clone()),
            );
            match fetch_to(&client, &m.url, &path).await {
                Ok(()) => result.files.push(path.to_string_lossy().to_string()),
                Err(e) => {
                    tracing::warn!("[x] midia {} falhou: {}", m.url, e);
                    result.failed += 1;
                }
            }
            done += 1;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    super::report(progress, job, "done", done as u64, Some(total as u64), None);
    Ok(result)
}

async fn fetch_to(
    client: &reqwest::Client,
    url: &str,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    let resp = client
        .get(url)
        .header("Referer", "https://x.com/")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await?;
    let part = path.with_extension("part");
    tokio::fs::write(&part, &bytes).await?;
    tokio::fs::rename(&part, path).await?;
    Ok(())
}

/// Todas as midias publicas de um perfil (aba Midia), ate `limit` posts.
pub async fn download_profile(
    input: &str,
    dest: &str,
    limit: usize,
    photos: bool,
    videos: bool,
    progress: ProgressFn,
) -> anyhow::Result<MediaResult> {
    let handle = super::handle_from(input)
        .ok_or_else(|| anyhow!("nao reconheci um perfil do X em: {}", input))?;
    let job = format!("x-media:{}", handle.to_ascii_lowercase());
    super::clear_cancel(&job);
    let mut posts: Vec<XPost> = Vec::new();
    let mut cursor: Option<String> = None;
    let limit = if limit == 0 { 5000 } else { limit };
    for _ in 0..200 {
        if super::cancelled(&job) {
            break;
        }
        super::report(&progress, &job, "listing", posts.len() as u64, None, None);
        let page = super::fx::profile_media(&handle, cursor.as_deref()).await?;
        if page.items.is_empty() {
            break;
        }
        posts.extend(page.items.into_iter().filter(|p| !p.media.is_empty()));
        if posts.len() >= limit || page.cursor.is_none() {
            break;
        }
        cursor = page.cursor;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    posts.truncate(limit);
    let posts = super::dedup_posts(posts);
    let dest = std::path::Path::new(dest).join(&handle);
    download_posts(&posts, &dest, photos, videos, &job, &progress).await
}
