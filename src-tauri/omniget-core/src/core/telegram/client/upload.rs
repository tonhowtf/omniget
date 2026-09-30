//! Sending files and folders to a chat the person picked (`tdl up`
//! parity). Never runs on its own: only `telegram_upload_paths` starts it.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use grammers_client::types::Attribute;
use grammers_client::{InputMedia, InputMessage};
use serde_json::json;
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;

use super::api;
use super::auth::TelegramSessionHandle;
use super::events;
use super::stream_server::mime_from_name;
use super::template::ext_of;

#[derive(Debug, Clone, Default)]
pub struct UploadOpts {
    pub caption: Option<String>,
    pub as_document: bool,
    pub delete_after: bool,
    pub group_albums: bool,
}

/// Files under `paths` (folders recursively, hidden files skipped), in a
/// stable order.
pub fn collect_files(paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if path.is_file() {
            out.push(path);
        } else if path.is_dir() {
            let mut found: Vec<PathBuf> = walkdir::WalkDir::new(&path)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.')
                })
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .map(|e| e.into_path())
                .collect();
            found.sort();
            out.extend(found);
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Photo,
    Video,
    Audio,
    Other,
}

/// Telegram refuses compressed photos over 10 MB.
const PHOTO_MAX: u64 = 10 * 1024 * 1024;

fn kind_of(path: &Path, size: u64) -> Kind {
    match ext_of(&path.to_string_lossy()).as_str() {
        "jpg" | "jpeg" | "png" | "webp" if size <= PHOTO_MAX => Kind::Photo,
        "mp4" | "m4v" | "mov" | "webm" | "mkv" => Kind::Video,
        "mp3" | "m4a" | "flac" | "ogg" | "oga" | "opus" | "wav" | "aac" => Kind::Audio,
        _ => Kind::Other,
    }
}

/// Counts bytes as grammers reads the file, for the progress bar.
struct ProgressReader<R> {
    inner: R,
    read: u64,
    total: u64,
    on_progress: Box<dyn FnMut(u64, u64) + Send>,
}

impl<R: AsyncRead + Unpin> AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let res = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &res {
            let n = (buf.filled().len() - before) as u64;
            if n > 0 {
                self.read += n;
                let (read, total) = (self.read, self.total);
                (self.on_progress)(read, total);
            }
        }
        res
    }
}

/// Reads the file into Telegram, emitting `telegram-upload-progress` for
/// the file (throttled to 4 per second).
async fn upload_one(
    client: &grammers_client::Client,
    path: &Path,
    file_id: u64,
    batch_id: u64,
    title: &str,
) -> anyhow::Result<grammers_client::types::media::Uploaded> {
    let file = tokio::fs::File::open(path).await?;
    let size = file.metadata().await?.len();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let title_owned = title.to_string();
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut reader = ProgressReader {
        inner: file,
        read: 0,
        total: size,
        on_progress: Box::new(move |read, total| {
            if last.elapsed() >= Duration::from_millis(250) || read >= total {
                last = Instant::now();
                let pct = if total > 0 {
                    read as f64 / total as f64 * 100.0
                } else {
                    100.0
                };
                emit_progress(file_id, &title_owned, "item", Some(batch_id), pct.min(99.0));
            }
        }),
    };
    let uploaded = client
        .upload_stream(&mut reader, size as usize, name)
        .await
        .map_err(|e| anyhow::anyhow!("upload: {}", e))?;
    Ok(uploaded)
}

async fn video_attr(path: &Path) -> Attribute {
    let (mut duration, mut w, mut h) = (0.0f64, 0i32, 0i32);
    if let Ok(info) = crate::core::ffmpeg::probe(path).await {
        duration = info.duration_seconds;
        if let Some(v) = info.streams.iter().find(|s| s.codec_type == "video") {
            w = v.width.unwrap_or(0) as i32;
            h = v.height.unwrap_or(0) as i32;
        }
    }
    Attribute::Video {
        round_message: false,
        supports_streaming: true,
        duration: Duration::from_secs_f64(duration.max(0.0)),
        w,
        h,
    }
}

async fn audio_attr(path: &Path) -> Attribute {
    let duration = crate::core::ffmpeg::probe(path)
        .await
        .map(|i| i.duration_seconds)
        .unwrap_or(0.0);
    Attribute::Audio {
        duration: Duration::from_secs_f64(duration.max(0.0)),
        title: None,
        performer: None,
    }
}

async fn single_message(
    path: &Path,
    kind: Kind,
    as_document: bool,
    caption: &str,
    uploaded: grammers_client::types::media::Uploaded,
) -> InputMessage {
    let mut msg = InputMessage::new().text(caption.to_string());
    if let Some(m) = mime_from_name(&path.to_string_lossy()) {
        msg = msg.mime_type(m);
    }
    if as_document {
        return msg.file(uploaded);
    }
    match kind {
        Kind::Photo => msg.photo(uploaded),
        Kind::Video => msg.document(uploaded).attribute(video_attr(path).await),
        Kind::Audio => msg.document(uploaded).attribute(audio_attr(path).await),
        Kind::Other => msg.document(uploaded),
    }
}

async fn album_media(
    path: &Path,
    kind: Kind,
    caption: &str,
    uploaded: grammers_client::types::media::Uploaded,
) -> InputMedia {
    let mut m = InputMedia::new().caption(caption.to_string());
    if let Some(mime) = mime_from_name(&path.to_string_lossy()) {
        m = m.mime_type(mime);
    }
    match kind {
        Kind::Photo => m.photo(uploaded),
        _ => m.document(uploaded).attribute(video_attr(path).await),
    }
}

fn emit_progress(id: u64, title: &str, kind: &str, batch_id: Option<u64>, percent: f64) {
    events::emit(
        "telegram-upload-progress",
        json!({ "id": id, "title": title, "kind": kind, "batch_id": batch_id, "percent": percent }),
    );
}

fn emit_complete(
    id: u64,
    title: &str,
    batch_id: u64,
    success: bool,
    error: Option<String>,
    message_id: Option<i32>,
) {
    events::emit(
        "telegram-upload-complete",
        json!({
            "id": id, "title": title, "kind": "item", "batch_id": batch_id,
            "success": success, "error": error, "message_id": message_id,
        }),
    );
}

/// Waits out a FLOOD_WAIT (up to 5 minutes) and says whether to try again.
async fn flood_pause(err: &str, cancel: &CancellationToken) -> bool {
    match api::parse_flood_wait(err) {
        Some(secs) if secs <= 300 => {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(secs + 1)) => true,
                _ = cancel.cancelled() => false,
            }
        }
        _ => false,
    }
}

struct Entry {
    id: u64,
    path: PathBuf,
    title: String,
    kind: Kind,
}

/// Sends every file, one message each, or as albums of up to 10 photos and
/// videos when `group_albums` is on. Returns right away; progress comes as
/// `telegram-upload-progress` / `telegram-upload-complete` (`kind` =
/// `item` per file, `batch` for the whole job, whose `id` is the batch id).
pub async fn start(
    handle: TelegramSessionHandle,
    active: super::ActiveDownloads,
    chat_id: i64,
    chat_type: String,
    files: Vec<PathBuf>,
    opts: UploadOpts,
) -> Result<u64, String> {
    let (client, peer, _) = super::history::peer_for(&handle, chat_id, &chat_type)
        .await
        .map_err(|e| e.to_string())?;
    let batch_id = events::next_id();
    let cancel = CancellationToken::new();
    active.lock().await.insert(
        batch_id,
        (format!("tg-upload:{}", batch_id), cancel.clone()),
    );
    let total = files.len();
    let batch_title = if total == 1 {
        files[0]
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        format!("{} files", total)
    };

    let entries: Vec<Entry> = files
        .into_iter()
        .map(|path| {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Entry {
                id: events::next_id(),
                title: path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                kind: kind_of(&path, size),
                path,
            }
        })
        .collect();

    // Groups: consecutive photos/videos share an album (max 10).
    let mut groups: Vec<Vec<Entry>> = Vec::new();
    for e in entries {
        let albumable =
            opts.group_albums && !opts.as_document && matches!(e.kind, Kind::Photo | Kind::Video);
        match groups.last_mut() {
            Some(g)
                if albumable
                    && g.len() < 10
                    && g.iter()
                        .all(|x| matches!(x.kind, Kind::Photo | Kind::Video)) =>
            {
                g.push(e)
            }
            _ => groups.push(vec![e]),
        }
    }
    for g in &groups {
        for e in g {
            emit_progress(e.id, &e.title, "item", Some(batch_id), 0.0);
        }
    }
    emit_progress(batch_id, &batch_title, "batch", None, 0.0);

    let caption = Arc::new(opts.caption.clone().unwrap_or_default());
    tokio::spawn(async move {
        let mut done = 0usize;
        let mut ok = 0usize;
        let mut failed = 0usize;
        let mut first_caption_used = false;
        for group in groups {
            if cancel.is_cancelled() {
                for e in &group {
                    emit_complete(
                        e.id,
                        &e.title,
                        batch_id,
                        false,
                        Some("cancelled".into()),
                        None,
                    );
                }
                continue;
            }
            // The caption goes on every single message, and once per album.
            let cap = if group.len() > 1 && first_caption_used {
                String::new()
            } else {
                caption.as_str().to_string()
            };
            let result: anyhow::Result<Vec<Option<i32>>> = async {
                if group.len() == 1 {
                    let e = &group[0];
                    let mut attempt = 0;
                    loop {
                        attempt += 1;
                        let uploaded =
                            upload_one(&client, &e.path, e.id, batch_id, &e.title).await?;
                        let msg =
                            single_message(&e.path, e.kind, opts.as_document, &cap, uploaded).await;
                        match client.send_message(peer.clone(), msg).await {
                            Ok(m) => return Ok(vec![Some(m.id())]),
                            Err(err) => {
                                let s = err.to_string();
                                if attempt < 3 && flood_pause(&s, &cancel).await {
                                    continue;
                                }
                                anyhow::bail!("send: {}", s);
                            }
                        }
                    }
                } else {
                    let mut medias = Vec::new();
                    for (i, e) in group.iter().enumerate() {
                        let uploaded =
                            upload_one(&client, &e.path, e.id, batch_id, &e.title).await?;
                        let c = if i == 0 { cap.as_str() } else { "" };
                        medias.push(album_media(&e.path, e.kind, c, uploaded).await);
                    }
                    let sent = client
                        .send_album(peer.clone(), medias)
                        .await
                        .map_err(|e| anyhow::anyhow!("send album: {}", e))?;
                    Ok(sent.into_iter().map(|m| m.map(|m| m.id())).collect())
                }
            }
            .await;
            if group.len() > 1 {
                first_caption_used = true;
            }
            match result {
                Ok(ids) => {
                    for (i, e) in group.iter().enumerate() {
                        ok += 1;
                        emit_progress(e.id, &e.title, "item", Some(batch_id), 100.0);
                        emit_complete(
                            e.id,
                            &e.title,
                            batch_id,
                            true,
                            None,
                            ids.get(i).copied().flatten(),
                        );
                        if opts.delete_after {
                            if let Err(err) = std::fs::remove_file(&e.path) {
                                tracing::warn!(
                                    "[tg-upload] could not delete {}: {}",
                                    e.path.display(),
                                    err
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    let msg = err.to_string();
                    for e in &group {
                        failed += 1;
                        emit_complete(e.id, &e.title, batch_id, false, Some(msg.clone()), None);
                    }
                }
            }
            done += group.len();
            emit_progress(
                batch_id,
                &batch_title,
                "batch",
                None,
                done as f64 / total.max(1) as f64 * 100.0,
            );
        }
        active.lock().await.remove(&batch_id);
        let cancelled = cancel.is_cancelled();
        let error = if cancelled {
            Some("Upload cancelled".to_string())
        } else if failed > 0 {
            Some(format!("{} files failed", failed))
        } else {
            None
        };
        events::emit(
            "telegram-upload-complete",
            json!({
                "id": batch_id, "title": batch_title, "kind": "batch", "batch_id": batch_id,
                "success": failed == 0 && !cancelled, "error": error, "message_id": null,
                "count": total, "uploaded": ok, "failed": failed,
            }),
        );
    });
    Ok(batch_id)
}
