//! SlideShare → PDF (estudo 50): as páginas públicas trazem cada slide em
//! `<img data-testid="vertical-slide-image" srcset="… 2048w">`.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SlidesResult {
    pub title: String,
    pub pages: usize,
    pub pdf_path: String,
}

pub fn parse_slide_urls(html: &str) -> Vec<String> {
    let img_re = regex::Regex::new(r#"<img[^>]*data-testid="vertical-slide-image"[^>]*>"#).unwrap();
    let srcset_re = regex::Regex::new(r#"srcset="([^"]+)""#).unwrap();
    let src_re = regex::Regex::new(r#"\ssrc="([^"]+)""#).unwrap();
    let mut out = Vec::new();
    for m in img_re.find_iter(html) {
        let tag = m.as_str();
        let url = if let Some(c) = srcset_re.captures(tag) {
            // "url1 320w, url2 638w, url3 2048w" → maior largura
            let mut best: Option<(u32, &str)> = None;
            for part in c[1].split(',') {
                let mut it = part.split_whitespace();
                let (Some(u), w) = (it.next(), it.next()) else {
                    continue;
                };
                let width = w
                    .and_then(|w| w.trim_end_matches('w').parse::<u32>().ok())
                    .unwrap_or(0);
                if best.map(|(bw, _)| width > bw).unwrap_or(true) {
                    best = Some((width, u));
                }
            }
            best.map(|(_, u)| u.to_string())
        } else {
            src_re.captures(tag).map(|c| c[1].to_string())
        };
        if let Some(u) = url {
            if !out.contains(&u) {
                out.push(u.replace("&amp;", "&"));
            }
        }
    }
    out
}

pub fn og_title(html: &str) -> Option<String> {
    let re = regex::Regex::new(r#"<meta[^>]*property="og:title"[^>]*content="([^"]*)""#).ok()?;
    re.captures(html).map(|c| {
        c[1].replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
    })
}

/// Converte qualquer imagem em JPEG pelo ffmpeg (webp/png escapam do DCTDecode).
pub(crate) async fn ensure_jpeg(data: Vec<u8>, work: &Path, idx: usize) -> anyhow::Result<Vec<u8>> {
    if super::jpeg_pdf::is_jpeg(&data) {
        return Ok(data);
    }
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let input = work.join(format!("{}.img", idx));
    let output = work.join(format!("{}.jpg", idx));
    tokio::fs::write(&input, &data).await?;
    let o = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(&input)
        .args(["-q:v", "2"])
        .arg(&output)
        .output()
        .await?;
    if !o.status.success() {
        return Err(anyhow!("ffmpeg nao converteu a imagem {}", idx));
    }
    Ok(tokio::fs::read(&output).await?)
}

pub async fn download(
    url: &str,
    dest_dir: &str,
    progress: super::ProgressFn,
) -> anyhow::Result<SlidesResult> {
    if !url.contains("slideshare.net") {
        return Err(anyhow!("cole um link do slideshare.net"));
    }
    let client = super::client()?;
    let html = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let urls = parse_slide_urls(&html);
    if urls.is_empty() {
        return Err(anyhow!("nao encontrei slides nessa pagina (o SlideShare pode ter mudado o HTML ou o deck e privado)"));
    }
    let title = og_title(&html).unwrap_or_else(|| "slideshare".to_string());
    let work = super::temp_dir().join(format!("slides-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work)?;
    let mut images = Vec::with_capacity(urls.len());
    let id = format!("slides:{}", url);
    for (i, u) in urls.iter().enumerate() {
        super::report(
            &progress,
            &id,
            "download",
            i as u64,
            Some(urls.len() as u64),
            None,
        );
        let data = client
            .get(u)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec();
        images.push(ensure_jpeg(data, &work, i).await?);
    }
    let pdf = super::jpeg_pdf::build_pdf(&images)?;
    let dir = PathBuf::from(dest_dir);
    std::fs::create_dir_all(&dir)?;
    let pdf_path = dir.join(format!("{}.pdf", super::sanitize_name(&title)));
    tokio::fs::write(&pdf_path, &pdf).await?;
    let _ = std::fs::remove_dir_all(&work);
    super::report(
        &progress,
        &id,
        "done",
        urls.len() as u64,
        Some(urls.len() as u64),
        None,
    );
    Ok(SlidesResult {
        title,
        pages: images.len(),
        pdf_path: pdf_path.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_largest_srcset() {
        let html = r#"<img class="x" data-testid="vertical-slide-image" src="https://a/1-320.jpg" srcset="https://a/1-320.jpg 320w, https://a/1-638.jpg 638w, https://a/75/1-2048.jpg 2048w"/><img data-testid="vertical-slide-image" src="https://a/2-320.jpg"/>"#;
        let urls = parse_slide_urls(html);
        assert_eq!(urls, vec!["https://a/75/1-2048.jpg", "https://a/2-320.jpg"]);
    }
}
