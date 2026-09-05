//! Download de mídia de um pin: original com cadeia de fallback (o CDN às
//! vezes dá 403 numa extensão e 200 em outra), GIF, carrossel, story pin,
//! vídeo (MP4 direto ou HLS → MP4 pelo ffmpeg) e WebP → PNG para quem não
//! consegue abrir WebP (iOS). Arquivo de já-baixados para sincronizar.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use super::api::{Pin, PinClient, Video};

#[derive(Debug, Clone, Deserialize)]
pub struct DownloadOptions {
    pub dest: String,
    /// baixar imagens
    #[serde(default = "t")]
    pub images: bool,
    /// baixar vídeos
    #[serde(default = "t")]
    pub videos: bool,
    /// converter WebP para PNG
    #[serde(default)]
    pub convert_webp: bool,
    /// "id" | "title" | "title-id"
    #[serde(default = "default_naming")]
    pub naming: String,
    /// gravar `<arquivo>.json` ao lado
    #[serde(default)]
    pub sidecar: bool,
    /// pular pins já registrados em `.omniget-pinterest.txt` na pasta
    #[serde(default = "t")]
    pub skip_downloaded: bool,
    /// subpasta por seção (backup de board)
    #[serde(default = "t")]
    pub section_folders: bool,
}

fn t() -> bool {
    true
}
fn default_naming() -> String {
    "title-id".into()
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PinFiles {
    pub id: String,
    pub files: Vec<String>,
    pub skipped: bool,
    pub error: Option<String>,
}

pub const ARCHIVE: &str = ".omniget-pinterest.txt";

pub fn load_archive(dest: &Path) -> HashSet<String> {
    std::fs::read_to_string(dest.join(ARCHIVE))
        .map(|s| {
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn append_archive(dest: &Path, id: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dest.join(ARCHIVE))
    {
        let _ = writeln!(f, "{}", id);
    }
}

/// Extensão pelo conteúdo (o CDN pode dizer .jpg e entregar WebP).
pub fn sniff_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 12 {
        return None;
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"\x00\x00\x00") && (&bytes[4..8] == b"ftyp") {
        Some("mp4")
    } else if &bytes[4..12] == b"ftypavif" {
        Some("avif")
    } else {
        None
    }
}

/// Candidatos em ordem: a URL dada, `/originals/` nas outras extensões, 1200x, 736x.
pub fn candidates(url: &str) -> Vec<String> {
    let re = regex::Regex::new(r"^(https?://i\.pinimg\.com)/(?:\d+x\d*|originals)(/[0-9a-f]{2}/[0-9a-f]{2}/[0-9a-f]{2}/[0-9a-f]{32})\.([a-zA-Z0-9]+)$").unwrap();
    let mut out = vec![url.to_string()];
    if let Some(c) = re.captures(url) {
        let base = &c[1];
        let path = &c[2];
        let ext = c[3].to_lowercase();
        for e in ["jpg", "png", "webp", "gif", "jpeg"] {
            let u = format!("{}/originals{}.{}", base, path, e);
            if !out.contains(&u) {
                out.push(u);
            }
        }
        for size in ["1200x", "736x", "564x"] {
            let u = format!("{}/{}{}.{}", base, size, path, ext);
            if !out.contains(&u) {
                out.push(u);
            }
        }
    }
    out
}

/// Baixa a melhor versão disponível de uma imagem do CDN.
pub async fn fetch_image(client: &PinClient, url: &str) -> anyhow::Result<(Vec<u8>, &'static str)> {
    let mut last = String::new();
    for cand in candidates(url) {
        let resp = match client
            .http()
            .get(&cand)
            .header("Referer", "https://www.pinterest.com/")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        if !resp.status().is_success() {
            last = format!("HTTP {} em {}", resp.status(), cand);
            continue;
        }
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = resp.bytes().await?.to_vec();
        if bytes.len() < 64 {
            last = format!("resposta vazia em {}", cand);
            continue;
        }
        let ext = sniff_ext(&bytes).unwrap_or(if ct.contains("png") {
            "png"
        } else if ct.contains("gif") {
            "gif"
        } else if ct.contains("webp") {
            "webp"
        } else {
            "jpg"
        });
        if ext == "mp4" {
            continue;
        }
        return Ok((bytes, ext));
    }
    Err(anyhow!("nao consegui baixar a imagem ({})", last))
}

fn webp_to_png(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(bytes)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

pub fn base_name(pin: &Pin, naming: &str) -> String {
    let title = pin.title.trim();
    let title = if title.is_empty() {
        pin.alt_text.trim()
    } else {
        title
    };
    let short: String = title.chars().take(70).collect();
    let clean = super::super::sanitize_name(&short);
    match naming {
        "id" => pin.id.clone(),
        "title" if clean != "arquivo" => clean,
        _ if clean != "arquivo" => format!("{}-{}", clean, pin.id),
        _ => pin.id.clone(),
    }
}

async fn write_bytes(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    tokio::fs::write(path, bytes).await?;
    Ok(())
}

/// Vídeo: MP4 direto quando existe, senão HLS remuxado pelo ffmpeg.
pub async fn download_video(client: &PinClient, video: &Video, out: &Path) -> anyhow::Result<()> {
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    if let Some(mp4) = &video.mp4 {
        let resp = client
            .http()
            .get(mp4)
            .header("Referer", "https://www.pinterest.com/")
            .send()
            .await?;
        if resp.status().is_success() {
            let bytes = resp.bytes().await?;
            if bytes.len() > 1024 {
                tokio::fs::write(out, &bytes).await?;
                return Ok(());
            }
        }
    }
    let hls = video
        .hls
        .as_ref()
        .ok_or_else(|| anyhow!("pin sem URL de video"))?;
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let o = crate::core::process::command(&ffmpeg)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-user_agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36",
            "-headers",
            "Referer: https://www.pinterest.com/\r\n",
            "-protocol_whitelist",
            "file,http,https,tcp,tls,crypto",
            "-i",
            hls,
            "-c",
            "copy",
            "-bsf:a",
            "aac_adtstoasc",
            "-movflags",
            "+faststart",
        ])
        .arg(out)
        .output()
        .await?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(anyhow!(
            "ffmpeg nao converteu o video: {}",
            err.lines().last().unwrap_or("").trim()
        ));
    }
    Ok(())
}

/// Baixa tudo de um pin para `dir`. Devolve os caminhos gravados.
pub async fn download_pin(
    client: &PinClient,
    pin: &Pin,
    dir: &Path,
    opts: &DownloadOptions,
) -> anyhow::Result<Vec<String>> {
    let base = base_name(pin, &opts.naming);
    let mut files = Vec::new();

    let save_image =
        |bytes: Vec<u8>, ext: &str, suffix: &str| -> anyhow::Result<(PathBuf, Vec<u8>)> {
            let (bytes, ext) = if ext == "webp" && opts.convert_webp {
                (webp_to_png(&bytes)?, "png")
            } else {
                (bytes, ext)
            };
            Ok((dir.join(format!("{}{}.{}", base, suffix, ext)), bytes))
        };

    let has_extras = !pin.extras.is_empty();
    if opts.images && !has_extras {
        if let Some(img) = pin.image.as_ref().or(pin.image_large.as_ref()) {
            if pin.kind != "video" || pin.video.is_none() {
                let (bytes, ext) = fetch_image(client, &img.url).await?;
                let (path, bytes) = save_image(bytes, ext, "")?;
                write_bytes(&path, &bytes).await?;
                files.push(path.to_string_lossy().to_string());
            }
        }
    }
    if opts.videos && !has_extras {
        if let Some(v) = &pin.video {
            let path = dir.join(format!("{}.mp4", base));
            download_video(client, v, &path).await?;
            files.push(path.to_string_lossy().to_string());
            if opts.images {
                if let Some(th) = &v.thumbnail {
                    if let Ok((bytes, ext)) = fetch_image(client, th).await {
                        let (path, bytes) = save_image(bytes, ext, "-capa")?;
                        write_bytes(&path, &bytes).await?;
                        files.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    for ex in &pin.extras {
        let suffix = format!("-{:02}", ex.index + 1);
        if ex.kind == "video" {
            if !opts.videos {
                continue;
            }
            if let Some(v) = &ex.video {
                let path = dir.join(format!("{}{}.mp4", base, suffix));
                download_video(client, v, &path).await?;
                files.push(path.to_string_lossy().to_string());
            }
        } else if opts.images {
            if let Some(m) = &ex.image {
                let (bytes, ext) = fetch_image(client, &m.url).await?;
                let (path, bytes) = save_image(bytes, ext, &suffix)?;
                write_bytes(&path, &bytes).await?;
                files.push(path.to_string_lossy().to_string());
            }
        }
    }
    if opts.sidecar && !files.is_empty() {
        let path = dir.join(format!("{}.json", base));
        write_bytes(&path, serde_json::to_string_pretty(pin)?.as_bytes()).await?;
    }
    if files.is_empty() {
        return Err(anyhow!("esse pin nao tem midia baixavel"));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_chain() {
        let c = candidates(
            "https://i.pinimg.com/originals/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.jpg",
        );
        assert_eq!(
            c[0],
            "https://i.pinimg.com/originals/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.jpg"
        );
        assert!(c.contains(
            &"https://i.pinimg.com/originals/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.png"
                .to_string()
        ));
        assert!(c.contains(
            &"https://i.pinimg.com/736x/3e/57/c1/3e57c1b723b8e9d39b8c09f6c5efbfb0.jpg".to_string()
        ));
        assert_eq!(
            candidates("https://x/y.jpg"),
            vec!["https://x/y.jpg".to_string()]
        );
    }

    #[test]
    fn sniff() {
        assert_eq!(
            sniff_ext(b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00\x01\x01"),
            Some("jpg")
        );
        assert_eq!(sniff_ext(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("webp"));
        assert_eq!(
            sniff_ext(b"GIF89a\x00\x00\x00\x00\x00\x00\x00"),
            Some("gif")
        );
    }

    #[test]
    fn names() {
        let mut p = Pin {
            id: "42".into(),
            title: "Mid Century: Living / Room".into(),
            ..Default::default()
        };
        assert_eq!(base_name(&p, "id"), "42");
        assert!(base_name(&p, "title-id").ends_with("-42"));
        p.title.clear();
        assert_eq!(base_name(&p, "title"), "42");
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::core::tools::pinterest::api::Feed;

    /// `cargo test -p omniget-core --lib pinterest::media::live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_download_image_and_video() {
        let c = PinClient::new(None).unwrap();
        let dir = std::env::temp_dir().join("omniget-pinterest-live");
        let _ = std::fs::remove_dir_all(&dir);
        let opts = DownloadOptions {
            dest: dir.to_string_lossy().to_string(),
            images: true,
            videos: true,
            convert_webp: true,
            naming: "title-id".into(),
            sidecar: true,
            skip_downloaded: true,
            section_folders: true,
        };
        let (pins, _, _) = c
            .feed_page(
                &Feed::Search {
                    query: "mid century living room".into(),
                    scope: "pins".into(),
                },
                None,
                3,
            )
            .await
            .unwrap();
        let files = download_pin(&c, &pins[0], &dir, &opts).await.unwrap();
        println!("image files: {:?}", files);
        assert!(!files.is_empty());
        let (vids, _, _) = c
            .feed_page(
                &Feed::Search {
                    query: "cats".into(),
                    scope: "videos".into(),
                },
                None,
                3,
            )
            .await
            .unwrap();
        let v = vids.iter().find(|p| p.video.is_some()).expect("video pin");
        let files = download_pin(&c, v, &dir, &opts).await.unwrap();
        println!("video files: {:?}", files);
        assert!(files.iter().any(|f| f.ends_with(".mp4")));
        let mp4 = files.iter().find(|f| f.ends_with(".mp4")).unwrap();
        let size = std::fs::metadata(mp4).unwrap().len();
        println!("mp4 {} bytes", size);
        assert!(size > 20_000);
        let mut m: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        m.insert(pins[0].id.clone(), files.clone());
        let html = super::super::export::html_gallery("t", "s", &pins, &m, &dir.to_string_lossy());
        std::fs::write(dir.join("index.html"), html).unwrap();
        std::fs::write(dir.join("pins.csv"), super::super::export::csv(&pins, &m)).unwrap();
    }
}
