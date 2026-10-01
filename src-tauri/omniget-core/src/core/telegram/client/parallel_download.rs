use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

static MAX_THREADS_CAP: AtomicUsize = AtomicUsize::new(8);

pub fn set_max_threads_cap(n: usize) {
    let clamped = n.clamp(1, 32);
    MAX_THREADS_CAP.store(clamped, Ordering::Relaxed);
}

pub fn get_max_threads_cap() -> usize {
    MAX_THREADS_CAP.load(Ordering::Relaxed)
}

use bytes::Bytes;
use futures::stream::Stream;
use grammers_client::grammers_tl_types as tl;
use grammers_client::Client;
use std::io::Write;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

pub struct StreamSpeedTracker {
    samples: Mutex<VecDeque<(Instant, u64)>>,
}

impl StreamSpeedTracker {
    pub fn new() -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
        }
    }

    pub async fn record(&self, bytes: u64) {
        let mut s = self.samples.lock().await;
        let now = Instant::now();
        s.push_back((now, bytes));
        let cutoff = now - std::time::Duration::from_secs(5);
        while let Some(&(t, _)) = s.front() {
            if t < cutoff {
                s.pop_front();
            } else {
                break;
            }
        }
    }

    pub async fn current_rate_bytes_per_sec(&self) -> u64 {
        let s = self.samples.lock().await;
        if s.is_empty() {
            return 0;
        }
        let total: u64 = s.iter().map(|(_, b)| *b).sum();
        let elapsed = match (s.front(), s.back()) {
            (Some((first, _)), Some((last, _))) => {
                last.duration_since(*first).as_secs_f64().max(1.0)
            }
            _ => 1.0,
        };
        (total as f64 / elapsed) as u64
    }
}

impl Default for StreamSpeedTracker {
    fn default() -> Self {
        Self::new()
    }
}

pub fn global_stream_tracker() -> &'static Arc<StreamSpeedTracker> {
    static INSTANCE: OnceLock<Arc<StreamSpeedTracker>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(StreamSpeedTracker::new()))
}

pub struct MediaLocation {
    pub location: tl::enums::InputFileLocation,
    pub size: u64,
    pub dc_id: i32,
}

pub fn media_to_location(media: &tl::enums::MessageMedia) -> Option<MediaLocation> {
    match media {
        tl::enums::MessageMedia::Document(doc_media) => {
            let doc = match doc_media.document.as_ref()? {
                tl::enums::Document::Document(d) => d,
                tl::enums::Document::Empty(_) => return None,
            };
            let location = tl::enums::InputFileLocation::InputDocumentFileLocation(
                tl::types::InputDocumentFileLocation {
                    id: doc.id,
                    access_hash: doc.access_hash,
                    file_reference: doc.file_reference.clone(),
                    thumb_size: String::new(),
                },
            );
            Some(MediaLocation {
                location,
                size: doc.size as u64,
                dc_id: doc.dc_id,
            })
        }
        tl::enums::MessageMedia::Photo(photo_media) => {
            let photo = match photo_media.photo.as_ref()? {
                tl::enums::Photo::Photo(p) => p,
                tl::enums::Photo::Empty(_) => return None,
            };
            let largest = photo
                .sizes
                .iter()
                .filter_map(|s| match s {
                    tl::enums::PhotoSize::Size(ps) => Some(ps),
                    _ => None,
                })
                .max_by_key(|ps| ps.size)?;

            let location = tl::enums::InputFileLocation::InputPhotoFileLocation(
                tl::types::InputPhotoFileLocation {
                    id: photo.id,
                    access_hash: photo.access_hash,
                    file_reference: photo.file_reference.clone(),
                    thumb_size: largest.r#type.clone(),
                },
            );
            Some(MediaLocation {
                location,
                size: largest.size as u64,
                dc_id: photo.dc_id,
            })
        }
        _ => None,
    }
}

fn extract_messages(result: tl::enums::messages::Messages) -> Vec<tl::enums::Message> {
    match result {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m) => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => vec![],
    }
}

pub async fn fetch_raw_media(
    client: &Client,
    chat_id: i64,
    access_hash: i64,
    is_channel: bool,
    message_id: i32,
) -> anyhow::Result<(tl::enums::MessageMedia, i32)> {
    let msg_input = vec![tl::enums::InputMessage::Id(tl::types::InputMessageId {
        id: message_id,
    })];

    let raw_messages = if is_channel {
        let request = tl::functions::channels::GetMessages {
            channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
                channel_id: chat_id,
                access_hash,
            }),
            id: msg_input,
        };
        let result = client
            .invoke(&request)
            .await
            .map_err(|e| anyhow::anyhow!("channels.getMessages: {}", e))?;
        extract_messages(result)
    } else {
        let request = tl::functions::messages::GetMessages { id: msg_input };
        let result = client
            .invoke(&request)
            .await
            .map_err(|e| anyhow::anyhow!("messages.getMessages: {}", e))?;
        extract_messages(result)
    };

    let raw_msg = raw_messages
        .into_iter()
        .find_map(|m| match m {
            tl::enums::Message::Message(msg) => Some(msg),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("Message {} not found", message_id))?;

    let raw_media = raw_msg
        .media
        .ok_or_else(|| anyhow::anyhow!("Message {} has no media", message_id))?;

    Ok((raw_media, raw_msg.date))
}

const MAX_CHUNK_SIZE: i32 = 512 * 1024;
const FILE_MIGRATE_ERROR: i32 = 303;

pub fn compute_thread_count(file_size: u64) -> usize {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    let raw = if file_size < MB {
        1
    } else if file_size < 5 * MB {
        2
    } else if file_size < 20 * MB {
        4
    } else if file_size >= 50 * MB {
        8
    } else {
        4
    };
    raw.min(get_max_threads_cap()).max(1)
}

fn auth_copied_dcs() -> &'static Arc<Mutex<Vec<i32>>> {
    static INSTANCE: OnceLock<Arc<Mutex<Vec<i32>>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

pub async fn ensure_auth_on_dc(client: &Client, target_dc_id: i32) -> anyhow::Result<()> {
    if target_dc_id <= 0 {
        return Ok(());
    }

    if let Some(home_dc) = super::auth::current_home_dc_id() {
        if home_dc == target_dc_id {
            return Ok(());
        }
    }

    {
        let copied = auth_copied_dcs().lock().await;
        if copied.contains(&target_dc_id) {
            return Ok(());
        }
    }

    tracing::info!("[tg-dl] copying auth to DC {}", target_dc_id);

    let exported_res = client
        .invoke(&tl::functions::auth::ExportAuthorization {
            dc_id: target_dc_id,
        })
        .await;

    let exported = match exported_res {
        Ok(tl::enums::auth::ExportedAuthorization::Authorization(e)) => e,
        Err(grammers_mtsender::InvocationError::Rpc(ref err)) if err.name == "DC_ID_INVALID" => {
            tracing::warn!(
                "[tg-dl] DC_ID_INVALID for ExportAuthorization to DC {} — assuming home DC, skipping",
                target_dc_id
            );
            let mut copied = auth_copied_dcs().lock().await;
            if !copied.contains(&target_dc_id) {
                copied.push(target_dc_id);
            }
            return Ok(());
        }
        Err(e) => {
            return Err(anyhow::anyhow!(
                "ExportAuthorization to DC {}: {}",
                target_dc_id,
                e
            ));
        }
    };

    let _: tl::enums::auth::Authorization = client
        .invoke_in_dc(
            target_dc_id,
            &tl::functions::auth::ImportAuthorization {
                id: exported.id,
                bytes: exported.bytes,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("ImportAuthorization on DC {}: {}", target_dc_id, e))?;

    {
        let mut copied = auth_copied_dcs().lock().await;
        if !copied.contains(&target_dc_id) {
            copied.push(target_dc_id);
        }
    }

    tracing::info!("[tg-dl] auth copied to DC {} successfully", target_dc_id);
    Ok(())
}

pub async fn download_file(
    client: &Client,
    media: &MediaLocation,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    let threads = compute_thread_count(media.size);
    tracing::info!(
        "[tg-dl] starting download: size={}, dc={}, threads={}, path={}",
        media.size,
        media.dc_id,
        threads,
        output_path.display()
    );
    if threads <= 1 || media.size == 0 {
        return download_file_sequential(client, media, output_path, progress_tx, cancel_token)
            .await;
    }
    download_file_parallel(
        client,
        media,
        output_path,
        threads,
        progress_tx,
        cancel_token,
    )
    .await
}

async fn download_file_sequential(
    client: &Client,
    media: &MediaLocation,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    let mut dc = media.dc_id;
    ensure_auth_on_dc(client, dc).await?;

    let mut file = std::fs::File::create(output_path)?;
    let mut downloaded: u64 = 0;
    let mut offset: i64 = 0;

    loop {
        if cancel_token.is_cancelled() {
            drop(file);
            let _ = std::fs::remove_file(output_path);
            return Err(anyhow::anyhow!("Download cancelled"));
        }

        let request = tl::functions::upload::GetFile {
            precise: true,
            cdn_supported: false,
            location: media.location.clone(),
            offset,
            limit: MAX_CHUNK_SIZE,
        };

        match client.invoke_in_dc(dc, &request).await {
            Ok(tl::enums::upload::File::File(f)) => {
                if f.bytes.is_empty() {
                    break;
                }

                file.write_all(&f.bytes)?;
                downloaded += f.bytes.len() as u64;
                offset += MAX_CHUNK_SIZE as i64;

                if media.size > 0 {
                    let percent = (downloaded as f64 / media.size as f64) * 100.0;
                    let _ = progress_tx.send(percent.min(100.0)).await;
                }

                if f.bytes.len() < MAX_CHUNK_SIZE as usize {
                    break;
                }
            }
            Ok(tl::enums::upload::File::CdnRedirect(_)) => {
                return Err(anyhow::anyhow!("CDN redirect not supported"));
            }
            Err(grammers_mtsender::InvocationError::Rpc(ref err))
                if err.name == "AUTH_KEY_UNREGISTERED" =>
            {
                tracing::warn!(
                    "[tg-dl] AUTH_KEY_UNREGISTERED on DC {}, re-copying auth",
                    dc
                );
                {
                    let mut copied = auth_copied_dcs().lock().await;
                    copied.retain(|&d| d != dc);
                }
                ensure_auth_on_dc(client, dc).await?;
                continue;
            }
            Err(grammers_mtsender::InvocationError::Rpc(ref err))
                if err.code == FILE_MIGRATE_ERROR =>
            {
                let new_dc = err.value.map(|v| v as i32).unwrap_or(dc);
                tracing::info!("[tg-dl] FILE_MIGRATE to DC {}", new_dc);
                dc = new_dc;
                ensure_auth_on_dc(client, dc).await?;
                continue;
            }
            Err(e) => {
                drop(file);
                let _ = std::fs::remove_file(output_path);
                return Err(anyhow::anyhow!("upload.getFile failed: {}", e));
            }
        }
    }

    file.flush()?;
    tracing::info!(
        "[tg-dl] download complete: {} bytes, dc={}",
        downloaded,
        media.dc_id
    );
    Ok(downloaded)
}

async fn download_file_parallel(
    client: &Client,
    media: &MediaLocation,
    output_path: &Path,
    threads: usize,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    use std::io::{Seek, SeekFrom};
    use std::sync::atomic::{AtomicU64, Ordering};

    let dc = media.dc_id;
    ensure_auth_on_dc(client, dc).await?;

    let total = media.size;
    let chunk: i32 = MAX_CHUNK_SIZE;
    let chunk_u = chunk as u64;
    let aligned = total.div_ceil(chunk_u) * chunk_u;
    let per_worker = aligned.div_ceil(threads as u64);
    let per_worker = ((per_worker / chunk_u).max(1)) * chunk_u;

    let prealloc = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(output_path)?;
    prealloc.set_len(total)?;
    drop(prealloc);

    let downloaded = Arc::new(AtomicU64::new(0));
    let file_mu = Arc::new(Mutex::new(
        std::fs::OpenOptions::new().write(true).open(output_path)?,
    ));

    let mut handles: Vec<tokio::task::JoinHandle<anyhow::Result<()>>> = Vec::new();
    for w in 0..threads {
        let start = (w as u64) * per_worker;
        if start >= total {
            break;
        }
        let end = (start + per_worker).min(total);
        let client = client.clone();
        let location = media.location.clone();
        let cancel = cancel_token.clone();
        let downloaded = downloaded.clone();
        let progress_tx = progress_tx.clone();
        let file_mu = file_mu.clone();

        handles.push(tokio::spawn(async move {
            let mut offset = start as i64;
            while (offset as u64) < end {
                if cancel.is_cancelled() {
                    return Err(anyhow::anyhow!("Download cancelled"));
                }
                let request = tl::functions::upload::GetFile {
                    precise: true,
                    cdn_supported: false,
                    location: location.clone(),
                    offset,
                    limit: chunk,
                };
                match client.invoke_in_dc(dc, &request).await {
                    Ok(tl::enums::upload::File::File(f)) => {
                        if f.bytes.is_empty() {
                            return Ok(());
                        }
                        let bytes_len = f.bytes.len() as u64;
                        {
                            let mut g = file_mu.lock().await;
                            g.seek(SeekFrom::Start(offset as u64))?;
                            g.write_all(&f.bytes)?;
                        }
                        let new_total =
                            downloaded.fetch_add(bytes_len, Ordering::Relaxed) + bytes_len;
                        if total > 0 {
                            let percent = (new_total as f64 / total as f64) * 100.0;
                            let _ = progress_tx.send(percent.min(100.0)).await;
                        }
                        offset += chunk as i64;
                        if (f.bytes.len() as i32) < chunk {
                            return Ok(());
                        }
                    }
                    Ok(tl::enums::upload::File::CdnRedirect(_)) => {
                        return Err(anyhow::anyhow!("CDN redirect not supported"));
                    }
                    Err(e) => {
                        return Err(anyhow::anyhow!(
                            "upload.getFile failed (worker {} offset {}): {}",
                            w,
                            offset,
                            e
                        ));
                    }
                }
            }
            Ok(())
        }));
    }

    let mut first_err: Option<anyhow::Error> = None;
    for h in handles {
        match h.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
                cancel_token.cancel();
            }
            Err(je) => {
                if first_err.is_none() {
                    first_err = Some(anyhow::anyhow!("worker join error: {}", je));
                }
                cancel_token.cancel();
            }
        }
    }

    if let Some(e) = first_err {
        let _ = std::fs::remove_file(output_path);
        return Err(e);
    }

    let final_total = downloaded.load(std::sync::atomic::Ordering::Relaxed);
    tracing::info!(
        "[tg-dl] parallel download complete: {} bytes, dc={}, threads={}",
        final_total,
        dc,
        threads
    );
    Ok(final_total)
}

fn flood_wait_secs(name: &str) -> Option<u64> {
    for pattern in ["FLOOD_WAIT_", "FLOOD_PREMIUM_WAIT_"] {
        if let Some(rest) = name.strip_prefix(pattern) {
            return rest.parse().ok();
        }
    }
    None
}

pub fn chunks_sidecar(part_path: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.chunks", part_path.display()))
}

fn read_done_chunks(sidecar: &Path, total: u64, chunk: u64) -> std::collections::HashSet<u32> {
    let mut done = std::collections::HashSet::new();
    let Ok(raw) = std::fs::read(sidecar) else {
        return done;
    };
    if raw.len() < 16 {
        return done;
    }
    let saved_total = u64::from_le_bytes(raw[0..8].try_into().unwrap_or([0; 8]));
    let saved_chunk = u64::from_le_bytes(raw[8..16].try_into().unwrap_or([0; 8]));
    if saved_total != total || saved_chunk != chunk {
        return done;
    }
    for c in raw[16..].chunks_exact(4) {
        done.insert(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
    }
    done
}

pub async fn download_file_resumable(
    client: &Client,
    media: &MediaLocation,
    part_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    use std::io::{Seek, SeekFrom};
    use std::sync::atomic::AtomicU64;

    let total = media.size;
    if total == 0 {
        return download_file_sequential(client, media, part_path, progress_tx, cancel_token).await;
    }
    let chunk_u = MAX_CHUNK_SIZE as u64;
    let n_chunks = total.div_ceil(chunk_u) as u32;
    let sidecar = chunks_sidecar(part_path);

    let mut done = if part_path.exists() {
        read_done_chunks(&sidecar, total, chunk_u)
    } else {
        std::collections::HashSet::new()
    };
    if done.is_empty() {
        let mut header = Vec::with_capacity(16);
        header.extend_from_slice(&total.to_le_bytes());
        header.extend_from_slice(&chunk_u.to_le_bytes());
        std::fs::write(&sidecar, header)?;
    }
    done.retain(|i| *i < n_chunks);

    {
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(part_path)?;
        if f.metadata()?.len() != total {
            f.set_len(total)?;
        }
    }

    let already: u64 = done
        .iter()
        .map(|i| {
            if *i == n_chunks - 1 {
                total - (*i as u64) * chunk_u
            } else {
                chunk_u
            }
        })
        .sum();
    if !done.is_empty() {
        tracing::info!(
            "[tg-dl] resuming {}: {}/{} chunks already on disk",
            part_path.display(),
            done.len(),
            n_chunks
        );
    }
    let pending: VecDeque<u32> = (0..n_chunks).filter(|i| !done.contains(i)).collect();
    if pending.is_empty() {
        let _ = std::fs::remove_file(&sidecar);
        return Ok(total);
    }

    let dc0 = media.dc_id;
    ensure_auth_on_dc(client, dc0).await?;

    let threads = compute_thread_count(total).min(pending.len()).max(1);
    let queue = Arc::new(Mutex::new(pending));
    let downloaded = Arc::new(AtomicU64::new(already));
    let file_mu = Arc::new(Mutex::new(
        std::fs::OpenOptions::new().write(true).open(part_path)?,
    ));
    let side_mu = Arc::new(Mutex::new(
        std::fs::OpenOptions::new().append(true).open(&sidecar)?,
    ));
    let dc_shared = Arc::new(std::sync::atomic::AtomicI32::new(dc0));
    let stop = cancel_token.child_token();

    let mut handles: Vec<tokio::task::JoinHandle<anyhow::Result<()>>> = Vec::new();
    for _ in 0..threads {
        let client = client.clone();
        let location = media.location.clone();
        let queue = queue.clone();
        let downloaded = downloaded.clone();
        let progress_tx = progress_tx.clone();
        let file_mu = file_mu.clone();
        let side_mu = side_mu.clone();
        let dc_shared = dc_shared.clone();
        let stop = stop.clone();
        handles.push(tokio::spawn(async move {
            loop {
                if stop.is_cancelled() {
                    return Err(anyhow::anyhow!("Download cancelled"));
                }
                let Some(idx) = queue.lock().await.pop_front() else { return Ok(()) };
                let offset = idx as i64 * MAX_CHUNK_SIZE as i64;
                let mut attempts = 0u32;
                loop {
                    if stop.is_cancelled() {
                        return Err(anyhow::anyhow!("Download cancelled"));
                    }
                    let dc = dc_shared.load(Ordering::Relaxed);
                    let request = tl::functions::upload::GetFile {
                        precise: true,
                        cdn_supported: false,
                        location: location.clone(),
                        offset,
                        limit: MAX_CHUNK_SIZE,
                    };
                    let res = tokio::time::timeout(
                        std::time::Duration::from_secs(60),
                        client.invoke_in_dc(dc, &request),
                    )
                    .await;
                    let res = match res {
                        Ok(r) => r,
                        Err(_) => {
                            attempts += 1;
                            if attempts >= 5 {
                                return Err(anyhow::anyhow!("upload.getFile timed out at offset {}", offset));
                            }
                            continue;
                        }
                    };
                    match res {
                        Ok(tl::enums::upload::File::File(f)) => {
                            {
                                let mut g = file_mu.lock().await;
                                g.seek(SeekFrom::Start(offset as u64))?;
                                g.write_all(&f.bytes)?;
                            }
                            {
                                let mut sg = side_mu.lock().await;
                                sg.write_all(&idx.to_le_bytes())?;
                            }
                            let len = f.bytes.len() as u64;
                            let new_total = downloaded.fetch_add(len, Ordering::Relaxed) + len;
                            let _ = progress_tx
                                .send(((new_total as f64 / total as f64) * 100.0).min(100.0))
                                .await;
                            break;
                        }
                        Ok(tl::enums::upload::File::CdnRedirect(_)) => {
                            return Err(anyhow::anyhow!("CDN redirect not supported"));
                        }
                        Err(grammers_mtsender::InvocationError::Rpc(ref err)) if err.name == "AUTH_KEY_UNREGISTERED" => {
                            {
                                let mut copied = auth_copied_dcs().lock().await;
                                copied.retain(|&d| d != dc);
                            }
                            ensure_auth_on_dc(&client, dc).await?;
                        }
                        Err(grammers_mtsender::InvocationError::Rpc(ref err)) if err.code == FILE_MIGRATE_ERROR => {
                            let new_dc = err.value.map(|v| v as i32).unwrap_or(dc);
                            ensure_auth_on_dc(&client, new_dc).await?;
                            dc_shared.store(new_dc, Ordering::Relaxed);
                        }
                        Err(grammers_mtsender::InvocationError::Rpc(ref err)) if flood_wait_secs(&err.name).is_some() || err.code == 420 => {
                            let secs = flood_wait_secs(&err.name).or(err.value.map(|v| v as u64)).unwrap_or(5);
                            tracing::warn!("[tg-dl] FLOOD_WAIT {}s during download", secs);
                            tokio::select! {
                                _ = tokio::time::sleep(std::time::Duration::from_secs(secs + 1)) => {}
                                _ = stop.cancelled() => return Err(anyhow::anyhow!("Download cancelled")),
                            }
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            attempts += 1;
                            if msg.to_uppercase().contains("FILE_REFERENCE") || attempts >= 5 {
                                return Err(anyhow::anyhow!("upload.getFile failed at offset {}: {}", offset, msg));
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(500 * attempts as u64)).await;
                        }
                    }
                }
            }
        }));
    }

    let mut first_err: Option<anyhow::Error> = None;
    for h in handles {
        let r = match h.await {
            Ok(r) => r,
            Err(je) => Err(anyhow::anyhow!("worker join error: {}", je)),
        };
        if let Err(e) = r {
            if first_err.is_none() {
                first_err = Some(e);
            }
            stop.cancel();
        }
    }
    {
        let g = file_mu.lock().await;
        let _ = g.sync_all();
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    let _ = std::fs::remove_file(&sidecar);
    Ok(total)
}

const STREAM_PREFETCH: usize = 6;

async fn fetch_stream_part(
    client: Client,
    location: tl::enums::InputFileLocation,
    dc: Arc<std::sync::atomic::AtomicI32>,
    offset: i64,
) -> Result<Vec<u8>, std::io::Error> {
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let cur_dc = dc.load(Ordering::Relaxed);
        let request = tl::functions::upload::GetFile {
            precise: true,
            cdn_supported: false,
            location: location.clone(),
            offset,
            limit: MAX_CHUNK_SIZE,
        };
        let err = match client.invoke_in_dc(cur_dc, &request).await {
            Ok(tl::enums::upload::File::File(f)) => return Ok(f.bytes),
            Ok(tl::enums::upload::File::CdnRedirect(_)) => {
                return Err(std::io::Error::other("CDN redirect not supported"))
            }
            Err(e) => e,
        };
        if attempts > 4 {
            return Err(std::io::Error::other(format!("upload.getFile: {}", err)));
        }
        match err {
            grammers_mtsender::InvocationError::Rpc(ref rpc)
                if rpc.name == "AUTH_KEY_UNREGISTERED" =>
            {
                auth_copied_dcs().lock().await.retain(|&d| d != cur_dc);
                ensure_auth_on_dc(&client, cur_dc)
                    .await
                    .map_err(|e| std::io::Error::other(format!("auth re-copy: {}", e)))?;
            }
            grammers_mtsender::InvocationError::Rpc(ref rpc) if rpc.code == FILE_MIGRATE_ERROR => {
                let new_dc = rpc.value.map(|v| v as i32).unwrap_or(cur_dc);
                dc.store(new_dc, Ordering::Relaxed);
                ensure_auth_on_dc(&client, new_dc)
                    .await
                    .map_err(|e| std::io::Error::other(format!("auth migrate: {}", e)))?;
            }
            ref other => {
                let text = other.to_string();
                let wait = match other {
                    grammers_mtsender::InvocationError::Rpc(rpc)
                        if rpc.name.starts_with("FLOOD") =>
                    {
                        rpc.value
                            .map(|v| v as u64)
                            .or_else(|| flood_wait_in_text(&text))
                    }
                    _ => flood_wait_in_text(&text),
                };
                match wait {
                    Some(secs) if secs <= 30 => {
                        tokio::time::sleep(std::time::Duration::from_secs(secs + 1)).await;
                    }
                    _ => return Err(std::io::Error::other(format!("upload.getFile: {}", text))),
                }
            }
        }
    }
}

async fn stream_part_at(
    client: Client,
    location: tl::enums::InputFileLocation,
    dc: Arc<std::sync::atomic::AtomicI32>,
    offset: i64,
) -> (i64, Result<Vec<u8>, std::io::Error>) {
    (
        offset,
        fetch_stream_part(client, location, dc, offset).await,
    )
}

fn flood_wait_in_text(text: &str) -> Option<u64> {
    let i = text.find("FLOOD_WAIT_").map(|i| i + "FLOOD_WAIT_".len())?;
    let digits: String = text[i..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

pub fn stream_file_range(
    client: Client,
    media: MediaLocation,
    range_start: u64,
    range_end: u64,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
    async_stream::stream! {
        use futures::StreamExt;

        if range_end < range_start {
            return;
        }
        let chunk = MAX_CHUNK_SIZE as u64;
        let first_part = range_start / chunk;
        let last_part = range_end / chunk;
        let dc = Arc::new(std::sync::atomic::AtomicI32::new(media.dc_id));

        if let Err(e) = ensure_auth_on_dc(&client, media.dc_id).await {
            yield Err(std::io::Error::other(format!("auth: {}", e)));
            return;
        }

        let mut next_part = first_part;
        let mut inflight = futures::stream::FuturesOrdered::new();
        while next_part <= last_part && inflight.len() < STREAM_PREFETCH {
            let offset = (next_part * chunk) as i64;
            inflight.push_back(stream_part_at(client.clone(), media.location.clone(), dc.clone(), offset));
            next_part += 1;
        }

        while let Some((offset, result)) = inflight.next().await {
            let bytes = match result {
                Ok(b) => b,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };
            let part_start = offset as u64;
            let part_end = part_start + bytes.len() as u64;
            let from = range_start.max(part_start);
            let to = (range_end + 1).min(part_end);
            if to > from {
                let slice = &bytes[(from - part_start) as usize..(to - part_start) as usize];
                global_stream_tracker().record(slice.len() as u64).await;
                yield Ok(Bytes::copy_from_slice(slice));
            }
            if (bytes.len() as u64) < chunk || to > range_end {
                break;
            }
            if next_part <= last_part {
                let offset = (next_part * chunk) as i64;
                inflight.push_back(stream_part_at(client.clone(), media.location.clone(), dc.clone(), offset));
                next_part += 1;
            }
        }
    }
}

pub fn mime_type_from_media(media: &tl::enums::MessageMedia) -> String {
    match media {
        tl::enums::MessageMedia::Document(d) => match d.document.as_ref() {
            Some(tl::enums::Document::Document(doc)) => doc.mime_type.clone(),
            _ => "application/octet-stream".to_string(),
        },
        tl::enums::MessageMedia::Photo(_) => "image/jpeg".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

#[cfg(test)]
mod compute_thread_count_tests {
    use super::compute_thread_count;
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;

    #[test]
    fn small_file_under_1mb_uses_1_thread() {
        assert_eq!(compute_thread_count(0), 1);
        assert_eq!(compute_thread_count(500 * KB), 1);
        assert_eq!(compute_thread_count(MB - 1), 1);
    }

    #[test]
    fn between_1mb_and_5mb_uses_2_threads() {
        assert_eq!(compute_thread_count(MB), 2);
        assert_eq!(compute_thread_count(2 * MB), 2);
        assert_eq!(compute_thread_count(5 * MB - 1), 2);
    }

    #[test]
    fn between_5mb_and_20mb_uses_4_threads() {
        assert_eq!(compute_thread_count(5 * MB), 4);
        assert_eq!(compute_thread_count(10 * MB), 4);
        assert_eq!(compute_thread_count(20 * MB - 1), 4);
    }

    #[test]
    fn between_20mb_and_50mb_uses_4_fallback() {
        assert_eq!(compute_thread_count(20 * MB), 4);
        assert_eq!(compute_thread_count(30 * MB), 4);
        assert_eq!(compute_thread_count(50 * MB - 1), 4);
    }

    #[test]
    fn at_50mb_or_above_uses_8_threads() {
        assert_eq!(compute_thread_count(50 * MB), 8);
        assert_eq!(compute_thread_count(100 * MB), 8);
        assert_eq!(compute_thread_count(1000 * MB), 8);
    }
}
