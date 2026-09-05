//! Itens de mídia do Instagram: um único formato normalizado para post,
//! reel, carrossel, story e highlight, venha ele do REST (`/api/v1`) ou do
//! GraphQL (os nós `xdt_*` têm o mesmo desenho do REST). Download com nome
//! previsível, legenda em .txt e extração de áudio pelo ffmpeg.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{b, s, u, IgClient, IgError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaFile {
    pub url: String,
    /// "image" | "video"
    pub kind: String,
    pub width: u64,
    pub height: u64,
    /// Capa (imagem) quando o arquivo é um vídeo.
    pub poster: Option<String>,
    pub pk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    pub pk: String,
    pub code: String,
    /// 1 foto · 2 vídeo · 8 carrossel
    pub media_type: u8,
    /// "feed" | "clips" | "igtv" | "carousel_container" | "story"
    pub product_type: String,
    pub taken_at: i64,
    pub expiring_at: Option<i64>,
    pub caption: String,
    pub like_count: u64,
    pub comment_count: u64,
    pub play_count: u64,
    pub owner_id: String,
    pub username: String,
    pub full_name: String,
    pub thumbnail: String,
    pub files: Vec<MediaFile>,
    pub duration: f64,
    pub location: Option<String>,
    pub url: String,
    pub width: u64,
    pub height: u64,
    pub hashtags: Vec<String>,
    pub mentions: Vec<String>,
    pub is_paid_partnership: bool,
    pub coauthors: Vec<String>,
    /// Título do highlight de onde o item veio, quando houver.
    pub title: Option<String>,
}

fn best_candidate(v: &Value, key: &str) -> Option<(String, u64, u64)> {
    let arr = v.get(key)?.get("candidates")?.as_array()?;
    arr.iter()
        .map(|c| (s(c, "url"), u(c, "width"), u(c, "height")))
        .filter(|(url, _, _)| !url.is_empty())
        .max_by_key(|(_, w, h)| w * h)
}

fn best_video(v: &Value) -> Option<(String, u64, u64)> {
    let arr = v.get("video_versions")?.as_array()?;
    arr.iter()
        .map(|c| (s(c, "url"), u(c, "width"), u(c, "height")))
        .filter(|(url, _, _)| !url.is_empty())
        .max_by_key(|(_, w, h)| w * h)
}

fn file_of(v: &Value) -> Option<MediaFile> {
    let image = best_candidate(v, "image_versions2");
    if let Some((url, w, h)) = best_video(v) {
        return Some(MediaFile { url, kind: "video".into(), width: w, height: h, poster: image.map(|i| i.0), pk: s(v, "pk") });
    }
    let (url, w, h) = image?;
    Some(MediaFile { url, kind: "image".into(), width: w, height: h, poster: None, pk: s(v, "pk") })
}

fn tags(text: &str, prefix: char) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == ',' || c == '.' && false) {
        let mut chars = word.chars();
        if chars.next() == Some(prefix) {
            let tag: String = chars.take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect();
            let tag = tag.trim_end_matches('.').to_lowercase();
            if !tag.is_empty() && !out.contains(&tag) {
                out.push(tag);
            }
        }
    }
    out
}

/// Converte um item do REST/GraphQL (post, story ou highlight) no formato
/// normalizado. Devolve `None` quando não há mídia utilizável.
pub fn parse_item(v: &Value) -> Option<MediaItem> {
    let user = v.get("user").cloned().unwrap_or(Value::Null);
    let username = s(&user, "username");
    let caption = v.get("caption").and_then(|c| c.get("text")).and_then(|t| t.as_str()).unwrap_or("").to_string();
    let expiring_at = v.get("expiring_at").and_then(|x| x.as_i64()).filter(|x| *x > 0);
    let mut files = Vec::new();
    if let Some(children) = v.get("carousel_media").and_then(|c| c.as_array()) {
        for child in children {
            if let Some(f) = file_of(child) {
                files.push(f);
            }
        }
    } else if let Some(f) = file_of(v) {
        files.push(f);
    }
    if files.is_empty() {
        return None;
    }
    let code = {
        let c = s(v, "code");
        if c.is_empty() { super::pk_to_shortcode(u(v, "pk")) } else { c }
    };
    let pk = s(v, "pk");
    let product_type = if expiring_at.is_some() || v.get("expiring_at").is_some() { "story".to_string() } else { s(v, "product_type") };
    let url = if product_type == "story" {
        format!("{}/stories/{}/{}/", super::BASE, username, pk)
    } else if product_type == "clips" {
        format!("{}/reel/{}/", super::BASE, code)
    } else {
        format!("{}/p/{}/", super::BASE, code)
    };
    let thumbnail = best_candidate(v, "image_versions2").map(|c| c.0).or_else(|| files.first().and_then(|f| f.poster.clone().or(Some(f.url.clone())))).unwrap_or_default();
    let coauthors = v
        .get("coauthor_producers")
        .and_then(|c| c.as_array())
        .map(|a| a.iter().map(|x| s(x, "username")).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    Some(MediaItem {
        media_type: u(v, "media_type") as u8,
        product_type,
        taken_at: v.get("taken_at").and_then(|x| x.as_i64()).unwrap_or(0),
        expiring_at,
        like_count: u(v, "like_count"),
        comment_count: u(v, "comment_count"),
        play_count: u(v, "play_count").max(u(v, "view_count")).max(u(v, "ig_play_count")),
        owner_id: s(&user, "pk"),
        full_name: s(&user, "full_name"),
        username,
        thumbnail,
        duration: v.get("video_duration").and_then(|x| x.as_f64()).unwrap_or(0.0),
        location: v.get("location").and_then(|l| l.get("name")).and_then(|n| n.as_str()).map(|x| x.to_string()),
        width: u(v, "original_width"),
        height: u(v, "original_height"),
        hashtags: tags(&caption, '#'),
        mentions: tags(&caption, '@'),
        is_paid_partnership: b(v, "is_paid_partnership"),
        coauthors,
        title: None,
        pk,
        code,
        url,
        caption,
        files,
    })
}

/// `GET /api/v1/media/{pk}/info/` — post, reel, carrossel ou IGTV.
pub async fn post_info(client: &IgClient, shortcode: &str) -> Result<MediaItem, IgError> {
    let pk = super::shortcode_to_pk(shortcode).ok_or_else(|| IgError::NotFound(format!("shortcode {}", shortcode)))?;
    let json = client.get_json(&format!("/api/v1/media/{}/info/", pk), &[]).await?;
    let item = json.get("items").and_then(|i| i.as_array()).and_then(|a| a.first()).ok_or_else(|| IgError::NotFound(format!("post {}", shortcode)))?;
    parse_item(item).ok_or_else(|| IgError::Other("o post nao tem midia que eu saiba baixar".into()))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DownloadOptions {
    /// Grava `<nome>.txt` com a legenda.
    pub caption_txt: bool,
    /// Grava `<nome>.json` com os metadados do item.
    pub metadata_json: bool,
    /// Só o áudio dos vídeos: "" (não), "m4a" ou "mp3".
    pub audio_only: String,
    /// Pula arquivos que já existem na pasta.
    pub skip_existing: bool,
    /// Subpasta por usuário (`<dest>/<username>/`).
    pub per_user_folder: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadResult {
    pub files: Vec<String>,
    pub skipped: usize,
    pub failed: Vec<String>,
    pub dest: String,
}

fn ext_of(url: &str, kind: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "heic" | "mp4" | "mov") {
        ext
    } else if kind == "video" {
        "mp4".into()
    } else {
        "jpg".into()
    }
}

pub fn base_name(item: &MediaItem) -> String {
    let date = chrono::DateTime::from_timestamp(item.taken_at, 0).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "sem-data".into());
    let user = if item.username.is_empty() { "instagram".to_string() } else { item.username.clone() };
    super::super::sanitize_name(&format!("{}_{}_{}", user, date, item.code))
}

async fn extract_audio(video: &Path, format: &str) -> anyhow::Result<PathBuf> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let out = video.with_extension(format);
    let mut cmd = crate::core::process::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-i"]).arg(video).arg("-vn");
    if format == "mp3" {
        cmd.args(["-c:a", "libmp3lame", "-q:a", "2"]);
    } else {
        cmd.args(["-c:a", "copy"]);
    }
    cmd.arg(&out);
    let status = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).output().await?;
    if !status.status.success() {
        return Err(anyhow!("ffmpeg: {}", String::from_utf8_lossy(&status.stderr).trim()));
    }
    Ok(out)
}

/// Baixa uma lista de itens para `dest`, reportando `ig:<job>` com
/// done/total e o nome do arquivo atual.
pub async fn download_items(
    client: &IgClient,
    items: &[MediaItem],
    dest: &str,
    opts: &DownloadOptions,
    progress: &super::super::ProgressFn,
    job: &str,
    flag: &AtomicBool,
) -> anyhow::Result<DownloadResult> {
    let id = format!("ig:{}", job);
    let total: u64 = items.iter().map(|i| i.files.len() as u64).sum();
    let mut done = 0u64;
    let mut files = Vec::new();
    let mut failed = Vec::new();
    let mut skipped = 0usize;
    std::fs::create_dir_all(dest)?;
    super::super::report(progress, &id, "started", 0, Some(total), None);
    'outer: for item in items {
        let folder = if opts.per_user_folder && !item.username.is_empty() {
            let f = Path::new(dest).join(super::super::sanitize_name(&item.username));
            std::fs::create_dir_all(&f)?;
            f
        } else {
            PathBuf::from(dest)
        };
        let base = base_name(item);
        let multi = item.files.len() > 1;
        for (idx, file) in item.files.iter().enumerate() {
            if super::cancelled(flag) {
                break 'outer;
            }
            let name = if multi { format!("{}_{:02}", base, idx + 1) } else { base.clone() };
            let ext = ext_of(&file.url, &file.kind);
            let target = folder.join(format!("{}.{}", name, ext));
            let audio = !opts.audio_only.is_empty() && file.kind == "video";
            let final_target = if audio { target.with_extension(&opts.audio_only) } else { target.clone() };
            super::super::report(progress, &id, "download", done, Some(total), Some(final_target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
            if opts.skip_existing && final_target.exists() {
                skipped += 1;
                done += 1;
                continue;
            }
            match client.download(&file.url, &target).await {
                Ok(_) => {
                    if audio {
                        match extract_audio(&target, &opts.audio_only).await {
                            Ok(a) => {
                                let _ = std::fs::remove_file(&target);
                                files.push(a.to_string_lossy().to_string());
                            }
                            Err(e) => {
                                failed.push(format!("{}: {}", name, e));
                                files.push(target.to_string_lossy().to_string());
                            }
                        }
                    } else {
                        files.push(target.to_string_lossy().to_string());
                    }
                }
                Err(e) => failed.push(format!("{}: {}", name, e)),
            }
            done += 1;
            super::super::report(progress, &id, "download", done, Some(total), None);
        }
        if opts.caption_txt && !item.caption.is_empty() {
            let _ = std::fs::write(folder.join(format!("{}.txt", base)), &item.caption);
        }
        if opts.metadata_json {
            let _ = std::fs::write(folder.join(format!("{}.json", base)), serde_json::to_string_pretty(item).unwrap_or_default());
        }
    }
    super::super::report(progress, &id, "done", done, Some(total), None);
    Ok(DownloadResult { files, skipped, failed, dest: dest.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rest_item() {
        let v: Value = serde_json::json!({
            "pk": "2251138696266127152", "code": "B89prebFBcw", "media_type": 1, "taken_at": 1583000000,
            "caption": {"text": "hello #Sun #sun @Friend"}, "like_count": 10, "comment_count": 2,
            "user": {"pk": "25025320", "username": "instagram", "full_name": "Instagram"},
            "image_versions2": {"candidates": [{"url": "https://x/a.jpg?x=1", "width": 640, "height": 640}, {"url": "https://x/b.jpg", "width": 1080, "height": 1080}]}
        });
        let item = parse_item(&v).unwrap();
        assert_eq!(item.files.len(), 1);
        assert_eq!(item.files[0].url, "https://x/b.jpg");
        assert_eq!(item.hashtags, vec!["sun"]);
        assert_eq!(item.mentions, vec!["friend"]);
        assert_eq!(item.url, "https://www.instagram.com/p/B89prebFBcw/");
        assert_eq!(base_name(&item), "instagram_2020-02-29_B89prebFBcw");
        assert_eq!(ext_of("https://x/v.mp4?efg=1", "video"), "mp4");
    }

    #[test]
    fn parses_carousel_and_story() {
        let v: Value = serde_json::json!({
            "pk": "1", "code": "A", "media_type": 8, "taken_at": 0, "product_type": "carousel_container", "user": {"username": "u"},
            "carousel_media": [
                {"pk": "11", "image_versions2": {"candidates": [{"url": "https://x/1.jpg", "width": 1, "height": 1}]}},
                {"pk": "12", "image_versions2": {"candidates": [{"url": "https://x/2.jpg", "width": 1, "height": 1}]}, "video_versions": [{"url": "https://x/2.mp4", "width": 720, "height": 1280}]}
            ]
        });
        let item = parse_item(&v).unwrap();
        assert_eq!(item.files.len(), 2);
        assert_eq!(item.files[1].kind, "video");
        assert_eq!(item.files[1].poster.as_deref(), Some("https://x/2.jpg"));
        let st: Value = serde_json::json!({"pk": "9", "media_type": 2, "taken_at": 5, "expiring_at": 99, "user": {"username": "u"}, "video_versions": [{"url": "https://x/s.mp4", "width": 1, "height": 1}]});
        let story = parse_item(&st).unwrap();
        assert_eq!(story.product_type, "story");
        assert!(story.url.contains("/stories/u/9/"));
    }
}
