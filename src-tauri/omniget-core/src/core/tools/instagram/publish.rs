//! Publicar no Instagram. Dois caminhos:
//! * **Sessão web** — o mesmo fluxo que o instagram.com faz no navegador:
//!   `rupload_igphoto`/`rupload_igvideo` + `media/configure*`. Funciona com
//!   os cookies da extensão, sem app da Meta. Beta: o Instagram muda esses
//!   endpoints sem aviso.
//! * **API oficial (Graph)** — precisa de conta profissional, app da Meta e
//!   um token; os arquivos têm que estar numa URL pública. Estável e
//!   documentado (referência: Postiz, AGPL — só a sequência de chamadas).
//!
//! Também guarda uma fila de agendamentos que o app executa enquanto
//! estiver aberto.

use std::path::Path;

use anyhow::anyhow;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{s, IgClient, IgError};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PublishRequest {
    /// "photo" | "video" | "reel" | "story" | "carousel"
    pub kind: String,
    /// Caminhos locais (sessão web) ou URLs públicas (Graph).
    pub files: Vec<String>,
    pub caption: String,
    /// Reels: mostrar também no feed.
    pub share_to_feed: bool,
    pub disable_comments: bool,
    pub hide_like_counts: bool,
    pub alt_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PublishResult {
    pub media_id: String,
    pub code: String,
    pub url: String,
}

fn upload_id() -> String {
    chrono::Utc::now().timestamp_millis().to_string()
}

/// Dimensões de um JPEG lendo os marcadores SOF (sem crate de imagem).
pub fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            let h = u32::from(bytes[i + 5]) << 8 | u32::from(bytes[i + 6]);
            let w = u32::from(bytes[i + 7]) << 8 | u32::from(bytes[i + 8]);
            return Some((w, h));
        }
        let len = usize::from(bytes[i + 2]) << 8 | usize::from(bytes[i + 3]);
        i += 2 + len;
    }
    None
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some((u32::from_be_bytes(bytes[16..20].try_into().ok()?), u32::from_be_bytes(bytes[20..24].try_into().ok()?)))
}

/// Garante JPEG (o upload web só aceita JPEG); converte PNG/WebP/HEIC pelo ffmpeg.
async fn as_jpeg(path: &Path) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let bytes = tokio::fs::read(path).await?;
    if let Some((w, h)) = jpeg_dimensions(&bytes) {
        return Ok((bytes, w, h));
    }
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let out = super::super::temp_dir().join(format!("ig-{}.jpg", upload_id()));
    let status = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-q:v", "2", "-pix_fmt", "yuvj420p"])
        .arg(&out)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;
    if !status.status.success() {
        return Err(anyhow!("ffmpeg: {}", String::from_utf8_lossy(&status.stderr).trim()));
    }
    let bytes = tokio::fs::read(&out).await?;
    let _ = tokio::fs::remove_file(&out).await;
    let (w, h) = jpeg_dimensions(&bytes).or_else(|| png_dimensions(&bytes)).unwrap_or((1080, 1080));
    Ok((bytes, w, h))
}

async fn rupload_photo(client: &IgClient, bytes: Vec<u8>, w: u32, h: u32, uid: &str, params_extra: &[(&str, Value)]) -> Result<(), IgError> {
    let name = format!("fb_uploader_{}", uid);
    let mut params = serde_json::json!({"media_type": 1, "upload_id": uid, "upload_media_height": h, "upload_media_width": w});
    for (k, v) in params_extra {
        params[k] = v.clone();
    }
    let mut h = HeaderMap::new();
    h.insert("X-Instagram-Rupload-Params", HeaderValue::from_str(&params.to_string()).map_err(|e| IgError::Other(e.to_string()))?);
    h.insert("X-Entity-Name", HeaderValue::from_str(&name).unwrap());
    h.insert("X-Entity-Length", HeaderValue::from_str(&bytes.len().to_string()).unwrap());
    h.insert("X-Entity-Type", HeaderValue::from_static("image/jpeg"));
    h.insert("Offset", HeaderValue::from_static("0"));
    h.insert("Content-Type", HeaderValue::from_static("image/jpeg"));
    h.insert("X-Instagram-AJAX", HeaderValue::from_static("1"));
    let json = client.post_raw(&format!("{}/rupload_igphoto/{}", super::BASE, name), h, bytes).await?;
    if s(&json, "status") != "ok" {
        return Err(IgError::Other(format!("upload da foto: {}", json)));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn rupload_video(client: &IgClient, bytes: Vec<u8>, w: u32, h: u32, duration_ms: u64, uid: &str, reel: bool, story: bool) -> Result<(), IgError> {
    let name = format!("fb_uploader_{}", uid);
    let mut params = serde_json::json!({
        "media_type": 2, "upload_id": uid, "upload_media_height": h, "upload_media_width": w,
        "upload_media_duration_ms": duration_ms, "video_format": "video/mp4",
        "retry_context": "{\"num_step_auto_retry\":0,\"num_reupload\":0,\"num_step_manual_retry\":0}",
        "xsharing_user_ids": "[]",
    });
    if reel {
        params["is_clips_video"] = Value::String("1".into());
    }
    if story {
        params["for_album"] = Value::String("0".into());
    }
    let mut hd = HeaderMap::new();
    hd.insert("X-Instagram-Rupload-Params", HeaderValue::from_str(&params.to_string()).map_err(|e| IgError::Other(e.to_string()))?);
    hd.insert("X-Entity-Name", HeaderValue::from_str(&name).unwrap());
    hd.insert("X-Entity-Length", HeaderValue::from_str(&bytes.len().to_string()).unwrap());
    hd.insert("X-Entity-Type", HeaderValue::from_static("video/mp4"));
    hd.insert("Offset", HeaderValue::from_static("0"));
    hd.insert("Content-Type", HeaderValue::from_static("video/mp4"));
    hd.insert("X-Instagram-AJAX", HeaderValue::from_static("1"));
    let json = client.post_raw(&format!("{}/rupload_igvideo/{}", super::BASE, name), hd, bytes).await?;
    if s(&json, "status") != "ok" {
        return Err(IgError::Other(format!("upload do video: {}", json)));
    }
    Ok(())
}

async fn probe_video(path: &Path) -> anyhow::Result<(u32, u32, u64)> {
    let info = crate::core::ffmpeg::probe(path).await?;
    let stream = info.streams.iter().find(|s| s.codec_type == "video").ok_or_else(|| anyhow!("o arquivo nao tem faixa de video"))?;
    let duration = if info.duration_seconds > 0.0 { info.duration_seconds } else { stream.duration_seconds.unwrap_or(0.0) };
    Ok((stream.width.unwrap_or(1080), stream.height.unwrap_or(1920), (duration * 1000.0) as u64))
}

/// Extrai um frame como capa do vídeo.
async fn cover_frame(path: &Path) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let ffmpeg = crate::core::dependencies::ensure_ffmpeg().await?;
    let out = super::super::temp_dir().join(format!("ig-cover-{}.jpg", upload_id()));
    let status = crate::core::process::command(&ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-ss", "0.5", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-q:v", "2", "-pix_fmt", "yuvj420p"])
        .arg(&out)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;
    if !status.status.success() {
        return Err(anyhow!("ffmpeg: {}", String::from_utf8_lossy(&status.stderr).trim()));
    }
    let bytes = tokio::fs::read(&out).await?;
    let _ = tokio::fs::remove_file(&out).await;
    let (w, h) = jpeg_dimensions(&bytes).unwrap_or((1080, 1920));
    Ok((bytes, w, h))
}

fn result_of(json: &Value) -> PublishResult {
    let media = json.get("media").cloned().unwrap_or(Value::Null);
    let code = s(&media, "code");
    PublishResult { media_id: s(&media, "pk"), url: if code.is_empty() { String::new() } else { format!("{}/p/{}/", super::BASE, code) }, code }
}

fn common_form(req: &PublishRequest, uid: &str) -> Vec<(&'static str, String)> {
    vec![
        ("upload_id", uid.to_string()),
        ("caption", req.caption.clone()),
        ("source_type", "library".into()),
        ("disable_comments", if req.disable_comments { "1" } else { "0" }.into()),
        ("like_and_view_counts_disabled", if req.hide_like_counts { "1" } else { "0" }.into()),
        ("custom_accessibility_caption", req.alt_text.clone()),
        ("usertags", "".into()),
        ("archive_only", "false".into()),
        ("is_meta_only_post", "0".into()),
    ]
}

/// Publica pela sessão web.
pub async fn publish_web(client: &IgClient, req: &PublishRequest, progress: &super::super::ProgressFn, job: &str) -> anyhow::Result<PublishResult> {
    let id = format!("ig:{}", job);
    if req.files.is_empty() {
        return Err(anyhow!("escolha pelo menos um arquivo"));
    }
    let report = |stage: &str, done: u64, total: u64| super::super::report(progress, &id, stage, done, Some(total), None);
    let m = |e: IgError| anyhow!(e.to_string());
    match req.kind.as_str() {
        "photo" => {
            report("upload", 0, 2);
            let uid = upload_id();
            let (bytes, w, h) = as_jpeg(Path::new(&req.files[0])).await?;
            rupload_photo(client, bytes, w, h, &uid, &[]).await.map_err(m)?;
            report("configure", 1, 2);
            let json = client.post_form("/api/v1/media/configure/", &common_form(req, &uid)).await.map_err(m)?;
            report("done", 2, 2);
            Ok(result_of(&json))
        }
        "story" => {
            let path = Path::new(&req.files[0]);
            let uid = upload_id();
            let is_video = matches!(path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(), Some("mp4" | "mov" | "m4v" | "webm"));
            report("upload", 0, 2);
            if is_video {
                let (w, h, dur) = probe_video(path).await?;
                let bytes = tokio::fs::read(path).await?;
                rupload_video(client, bytes, w, h, dur, &uid, false, true).await.map_err(m)?;
                let (cover, cw, ch) = cover_frame(path).await?;
                rupload_photo(client, cover, cw, ch, &uid, &[("media_type", Value::from(2))]).await.map_err(m)?;
                report("configure", 1, 2);
                let json = client.post_form("/api/v1/media/configure_to_story/?video=1", &[("upload_id", uid.clone()), ("source_type", "library".into()), ("configure_mode", "1".into()), ("video_result", "".into()), ("length", (dur as f64 / 1000.0).to_string())]).await.map_err(m)?;
                report("done", 2, 2);
                Ok(result_of(&json))
            } else {
                let (bytes, w, h) = as_jpeg(path).await?;
                rupload_photo(client, bytes, w, h, &uid, &[]).await.map_err(m)?;
                report("configure", 1, 2);
                let json = client.post_form("/api/v1/media/configure_to_story/", &[("upload_id", uid.clone()), ("source_type", "library".into()), ("configure_mode", "1".into())]).await.map_err(m)?;
                report("done", 2, 2);
                Ok(result_of(&json))
            }
        }
        "video" | "reel" => {
            let path = Path::new(&req.files[0]);
            let uid = upload_id();
            report("upload", 0, 3);
            let (w, h, dur) = probe_video(path).await?;
            let bytes = tokio::fs::read(path).await?;
            rupload_video(client, bytes, w, h, dur, &uid, true, false).await.map_err(m)?;
            report("cover", 1, 3);
            let (cover, cw, ch) = cover_frame(path).await?;
            rupload_photo(client, cover, cw, ch, &uid, &[("media_type", Value::from(2))]).await.map_err(m)?;
            report("configure", 2, 3);
            let mut form = common_form(req, &uid);
            form.push(("video_format", "video/mp4".into()));
            form.push(("length", (dur as f64 / 1000.0).to_string()));
            form.push(("clips_share_to_feed", if req.share_to_feed || req.kind == "video" { "1" } else { "0" }.into()));
            form.push(("share_to_feed", if req.share_to_feed || req.kind == "video" { "1" } else { "0" }.into()));
            // O Instagram pode ainda estar transcodificando: tenta algumas vezes.
            let mut last = None;
            for attempt in 0..6 {
                match client.post_form("/api/v1/media/configure_to_clips/?video=1", &form).await {
                    Ok(json) if s(&json, "status") == "ok" => {
                        report("done", 3, 3);
                        return Ok(result_of(&json));
                    }
                    Ok(json) => last = Some(anyhow!("configure: {}", json)),
                    Err(IgError::Other(e)) if e.contains("Transcode") || e.contains("not ready") || e.contains("202") => {
                        last = Some(anyhow!(e));
                    }
                    Err(e) => return Err(m(e)),
                }
                tokio::time::sleep(std::time::Duration::from_secs(4 + attempt * 3)).await;
            }
            Err(last.unwrap_or_else(|| anyhow!("o Instagram nao confirmou o video")))
        }
        "carousel" => {
            if req.files.len() < 2 || req.files.len() > 20 {
                return Err(anyhow!("um carrossel tem de 2 a 20 arquivos"));
            }
            let total = req.files.len() as u64 + 1;
            let sidecar_id = upload_id();
            let mut children: Vec<Value> = Vec::new();
            for (i, f) in req.files.iter().enumerate() {
                report("upload", i as u64, total);
                let path = Path::new(f);
                let uid = format!("{}{}", upload_id(), i);
                let is_video = matches!(path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(), Some("mp4" | "mov" | "m4v" | "webm"));
                if is_video {
                    let (w, h, dur) = probe_video(path).await?;
                    let bytes = tokio::fs::read(path).await?;
                    rupload_video(client, bytes, w, h, dur, &uid, false, false).await.map_err(m)?;
                    let (cover, cw, ch) = cover_frame(path).await?;
                    rupload_photo(client, cover, cw, ch, &uid, &[("media_type", Value::from(2)), ("is_sidecar", Value::from("1"))]).await.map_err(m)?;
                    children.push(serde_json::json!({"upload_id": uid, "source_type": "library", "video_result": "", "length": dur as f64 / 1000.0}));
                } else {
                    let (bytes, w, h) = as_jpeg(path).await?;
                    rupload_photo(client, bytes, w, h, &uid, &[("is_sidecar", Value::from("1"))]).await.map_err(m)?;
                    children.push(serde_json::json!({"upload_id": uid, "source_type": "library"}));
                }
                super::sleep_jitter(500, 1500).await;
            }
            report("configure", total - 1, total);
            let mut form = common_form(req, &sidecar_id);
            form.push(("client_sidecar_id", sidecar_id.clone()));
            form.push(("children_metadata", serde_json::to_string(&children).unwrap_or_default()));
            let json = client.post_form("/api/v1/media/configure_sidecar/", &form).await.map_err(m)?;
            report("done", total, total);
            Ok(result_of(&json))
        }
        other => Err(anyhow!("tipo desconhecido: {}", other)),
    }
}

// ── API oficial (Graph) ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphAuth {
    pub access_token: String,
    pub ig_user_id: String,
}

const GRAPH: &str = "https://graph.facebook.com/v21.0";

async fn graph_post(http: &reqwest::Client, url: &str, form: &[(&str, String)]) -> anyhow::Result<Value> {
    let resp = http.post(url).form(form).send().await?;
    let json: Value = resp.json().await?;
    if let Some(err) = json.get("error") {
        return Err(anyhow!("Graph API: {}", err.get("message").and_then(|m| m.as_str()).unwrap_or("erro")));
    }
    Ok(json)
}

async fn graph_get(http: &reqwest::Client, url: &str) -> anyhow::Result<Value> {
    let json: Value = http.get(url).send().await?.json().await?;
    if let Some(err) = json.get("error") {
        return Err(anyhow!("Graph API: {}", err.get("message").and_then(|m| m.as_str()).unwrap_or("erro")));
    }
    Ok(json)
}

async fn wait_container(http: &reqwest::Client, id: &str, token: &str) -> anyhow::Result<()> {
    for _ in 0..40 {
        let st = graph_get(http, &format!("{}/{}?fields=status_code,status&access_token={}", GRAPH, id, token)).await?;
        match s(&st, "status_code").as_str() {
            "FINISHED" => return Ok(()),
            "ERROR" | "EXPIRED" => return Err(anyhow!("o container falhou: {}", s(&st, "status"))),
            _ => tokio::time::sleep(std::time::Duration::from_secs(5)).await,
        }
    }
    Err(anyhow!("o Instagram demorou demais para processar a midia"))
}

/// Publica pela API oficial. `req.files` são URLs públicas.
pub async fn publish_graph(auth: &GraphAuth, req: &PublishRequest, progress: &super::super::ProgressFn, job: &str) -> anyhow::Result<PublishResult> {
    let id = format!("ig:{}", job);
    let http = super::super::client()?;
    let token = auth.access_token.trim().to_string();
    let user = auth.ig_user_id.trim().to_string();
    if token.is_empty() || user.is_empty() {
        return Err(anyhow!("informe o token e o ID da conta do Instagram"));
    }
    if req.files.is_empty() || !req.files.iter().all(|f| f.starts_with("http")) {
        return Err(anyhow!("a API oficial so aceita URLs publicas (https://…) para os arquivos"));
    }
    let media_url = format!("{}/{}/media", GRAPH, user);
    let is_video = |f: &str| f.split('?').next().unwrap_or(f).to_lowercase().ends_with(".mp4") || f.to_lowercase().ends_with(".mov");
    let creation_id = match req.kind.as_str() {
        "carousel" => {
            let total = req.files.len() as u64 + 2;
            let mut children = Vec::new();
            for (i, f) in req.files.iter().enumerate() {
                super::super::report(progress, &id, "container", i as u64, Some(total), None);
                let mut form = vec![("is_carousel_item", "true".to_string()), ("access_token", token.clone())];
                if is_video(f) {
                    form.push(("media_type", "VIDEO".into()));
                    form.push(("video_url", f.clone()));
                } else {
                    form.push(("image_url", f.clone()));
                }
                let c = graph_post(&http, &media_url, &form).await?;
                let cid = s(&c, "id");
                wait_container(&http, &cid, &token).await?;
                children.push(cid);
            }
            let c = graph_post(&http, &media_url, &[("media_type", "CAROUSEL".into()), ("children", children.join(",")), ("caption", req.caption.clone()), ("access_token", token.clone())]).await?;
            s(&c, "id")
        }
        kind => {
            super::super::report(progress, &id, "container", 0, Some(3), None);
            let f = &req.files[0];
            let mut form = vec![("caption", req.caption.clone()), ("access_token", token.clone())];
            match kind {
                "story" => {
                    form.push(("media_type", "STORIES".into()));
                    form.push((if is_video(f) { "video_url" } else { "image_url" }, f.clone()));
                }
                "reel" | "video" => {
                    form.push(("media_type", "REELS".into()));
                    form.push(("video_url", f.clone()));
                    form.push(("share_to_feed", if req.share_to_feed || kind == "video" { "true" } else { "false" }.into()));
                }
                _ => form.push(("image_url", f.clone())),
            }
            let c = graph_post(&http, &media_url, &form).await?;
            s(&c, "id")
        }
    };
    super::super::report(progress, &id, "processing", 1, Some(3), None);
    wait_container(&http, &creation_id, &token).await?;
    super::super::report(progress, &id, "publish", 2, Some(3), None);
    let p = graph_post(&http, &format!("{}/{}/media_publish", GRAPH, user), &[("creation_id", creation_id), ("access_token", token.clone())]).await?;
    let media_id = s(&p, "id");
    let link = graph_get(&http, &format!("{}/{}?fields=permalink,shortcode&access_token={}", GRAPH, media_id, token)).await.unwrap_or(Value::Null);
    super::super::report(progress, &id, "done", 3, Some(3), None);
    Ok(PublishResult { media_id, code: s(&link, "shortcode"), url: s(&link, "permalink") })
}

// ── Agendamento ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduledPost {
    pub id: String,
    /// Unix (segundos).
    pub run_at: i64,
    pub request: PublishRequest,
    /// "web" | "graph"
    pub mode: String,
    pub account_slug: Option<String>,
    pub graph: Option<GraphAuth>,
    /// "pending" | "running" | "done" | "failed"
    pub status: String,
    pub result: Option<PublishResult>,
    pub error: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduleStore {
    pub posts: Vec<ScheduledPost>,
}

fn schedule_path() -> std::path::PathBuf {
    super::data_dir().join("schedule.json")
}

pub fn schedule_list() -> ScheduleStore {
    super::read_json(&schedule_path())
}

pub fn schedule_save(store: &ScheduleStore) -> anyhow::Result<()> {
    super::write_json(&schedule_path(), store)
}

pub fn schedule_add(mut post: ScheduledPost) -> anyhow::Result<ScheduleStore> {
    let mut store = schedule_list();
    if post.id.is_empty() {
        post.id = uuid::Uuid::new_v4().to_string();
    }
    post.status = "pending".into();
    post.created_at = chrono::Utc::now().timestamp();
    store.posts.push(post);
    store.posts.sort_by_key(|p| p.run_at);
    schedule_save(&store)?;
    Ok(store)
}

pub fn schedule_remove(id: &str) -> anyhow::Result<ScheduleStore> {
    let mut store = schedule_list();
    store.posts.retain(|p| p.id != id);
    schedule_save(&store)?;
    Ok(store)
}

pub fn schedule_update(post: &ScheduledPost) -> anyhow::Result<()> {
    let mut store = schedule_list();
    if let Some(p) = store.posts.iter_mut().find(|p| p.id == post.id) {
        *p = post.clone();
    }
    schedule_save(&store)
}

/// Próximo agendamento vencido e ainda pendente.
pub fn schedule_due(now: i64) -> Option<ScheduledPost> {
    schedule_list().posts.into_iter().find(|p| p.status == "pending" && p.run_at <= now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_dims() {
        // JPEG mínimo: SOI + SOF0 (altura 2, largura 3).
        let bytes = [0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x02, 0x00, 0x03, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xD9];
        assert_eq!(jpeg_dimensions(&bytes), Some((3, 2)));
        assert_eq!(jpeg_dimensions(b"nope"), None);
    }
}
