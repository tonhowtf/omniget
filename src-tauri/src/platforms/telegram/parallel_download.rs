use std::path::Path;
use std::sync::Arc;

use grammers_client::Client;
use grammers_client::grammers_tl_types as tl;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const PART_SIZE: i32 = 1024 * 1024;
const PARALLEL_THRESHOLD: u64 = 5 * 1024 * 1024;

const MAX_RETRIES: u32 = 3;
const RETRY_BASE_DELAY_MS: u64 = 1000;

pub fn best_threads(file_size: u64, max: usize) -> usize {
    let threads = if file_size < 1024 * 1024 {
        1
    } else if file_size < 5 * 1024 * 1024 {
        2
    } else if file_size < 20 * 1024 * 1024 {
        4
    } else if file_size < 50 * 1024 * 1024 {
        8
    } else {
        max
    };
    threads.min(max)
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

pub fn media_to_input_location(
    media: &tl::enums::MessageMedia,
) -> Option<(tl::enums::InputFileLocation, u64)> {
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
            Some((location, doc.size as u64))
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
            Some((location, largest.size as u64))
        }
        _ => None,
    }
}

fn write_at_offset(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.write_all_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut written = 0usize;
        while written < buf.len() {
            let n = file.seek_write(&buf[written..], offset + written as u64)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write data",
                ));
            }
            written += n;
        }
        Ok(())
    }
}

pub async fn download_parallel(
    client: &Client,
    location: tl::enums::InputFileLocation,
    total_size: u64,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
    max_threads: usize,
) -> anyhow::Result<u64> {
    if total_size < PARALLEL_THRESHOLD {
        tracing::info!(
            "[tg-parallel] file size {} < threshold {}, using sequential",
            total_size, PARALLEL_THRESHOLD
        );
        return download_sequential(
            client,
            location,
            total_size,
            output_path,
            progress_tx,
            cancel_token,
        )
        .await;
    }

    let threads = best_threads(total_size, max_threads);
    let num_parts = total_size.div_ceil(PART_SIZE as u64);
    tracing::info!(
        "[tg-parallel] starting parallel download: size={}, parts={}, threads={}",
        total_size,
        num_parts,
        threads
    );

    let path_for_create = output_path.to_path_buf();
    let file = tokio::task::spawn_blocking(move || -> std::io::Result<std::fs::File> {
        let f = std::fs::File::create(path_for_create)?;
        f.set_len(total_size)?;
        Ok(f)
    })
    .await
    .map_err(|e| anyhow::anyhow!("File create task panicked: {}", e))??;
    let file = Arc::new(file);

    let semaphore = Arc::new(Semaphore::new(threads));
    let downloaded = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut join_set = JoinSet::new();

    for part_idx in 0..num_parts {
        let offset = part_idx * PART_SIZE as u64;
        let expected_len = std::cmp::min(PART_SIZE as u64, total_size - offset);

        let client = client.clone();
        let location = location.clone();
        let file = Arc::clone(&file);
        let semaphore = Arc::clone(&semaphore);
        let downloaded = Arc::clone(&downloaded);
        let progress_tx = progress_tx.clone();
        let cancel = cancel_token.clone();

        join_set.spawn(async move {
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|e| anyhow::anyhow!("Semaphore closed: {}", e))?;

            if cancel.is_cancelled() {
                return Err(anyhow::anyhow!("Download cancelled"));
            }

            let mut last_err = None;
            let mut bytes_result = None;

            for attempt in 0..=MAX_RETRIES {
                if attempt > 0 {
                    let delay = RETRY_BASE_DELAY_MS * (1u64 << (attempt - 1));
                    tracing::warn!(
                        "[tg-parallel] part {} attempt {}/{}, retrying in {}ms",
                        part_idx, attempt + 1, MAX_RETRIES + 1, delay
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;

                    if cancel.is_cancelled() {
                        return Err(anyhow::anyhow!("Download cancelled"));
                    }
                }

                let request = tl::functions::upload::GetFile {
                    precise: true,
                    cdn_supported: false,
                    location: location.clone(),
                    offset: offset as i64,
                    limit: PART_SIZE,
                };

                match client.invoke(&request).await {
                    Ok(response) => match response {
                        tl::enums::upload::File::File(f) => {
                            bytes_result = Some(f.bytes);
                            break;
                        }
                        tl::enums::upload::File::CdnRedirect(_) => {
                            return Err(anyhow::anyhow!("CDN redirect not supported"));
                        }
                    },
                    Err(e) => {
                        last_err = Some(e);
                        continue;
                    }
                }
            }

            let bytes = match bytes_result {
                Some(b) => b,
                None => {
                    return Err(anyhow::anyhow!(
                        "upload.GetFile at offset {} failed after {} retries: {}",
                        offset,
                        MAX_RETRIES,
                        last_err.map(|e| e.to_string()).unwrap_or_default()
                    ));
                }
            };

            if (bytes.len() as u64) != expected_len {
                tracing::warn!(
                    "[tg-parallel] part {} got {} bytes, expected {}",
                    part_idx,
                    bytes.len(),
                    expected_len
                );
            }

            let chunk_len = bytes.len() as u64;
            let file_ref = Arc::clone(&file);
            let write_offset = offset;
            tokio::task::spawn_blocking(move || write_at_offset(&file_ref, &bytes, write_offset))
                .await
                .map_err(|e| anyhow::anyhow!("Write task panicked: {}", e))?
                .map_err(|e| anyhow::anyhow!("Write at offset {} failed: {}", offset, e))?;

            let prev =
                downloaded.fetch_add(chunk_len, std::sync::atomic::Ordering::Relaxed);
            let new_total = prev + chunk_len;
            if total_size > 0 {
                let percent = (new_total as f64 / total_size as f64) * 100.0;
                let _ = progress_tx.send(percent.min(100.0)).await;
            }

            Ok::<u64, anyhow::Error>(chunk_len)
        });
    }

    let mut total_downloaded: u64 = 0;
    while let Some(result) = join_set.join_next().await {
        if cancel_token.is_cancelled() {
            join_set.abort_all();
            let _ = tokio::fs::remove_file(output_path).await;
            return Err(anyhow::anyhow!("Download cancelled"));
        }
        match result {
            Ok(Ok(bytes)) => total_downloaded += bytes,
            Ok(Err(e)) => {
                join_set.abort_all();
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(e);
            }
            Err(e) => {
                join_set.abort_all();
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(anyhow::anyhow!("Download task panicked: {}", e));
            }
        }
    }

    tracing::info!(
        "[tg-parallel] parallel download complete: {} bytes",
        total_downloaded
    );
    Ok(total_downloaded)
}

async fn download_sequential(
    client: &Client,
    location: tl::enums::InputFileLocation,
    total_size: u64,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::File::create(output_path).await?;
    let mut offset: i64 = 0;
    let mut downloaded: u64 = 0;

    loop {
        if cancel_token.is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(output_path).await;
            return Err(anyhow::anyhow!("Download cancelled"));
        }

        let request = tl::functions::upload::GetFile {
            precise: true,
            cdn_supported: false,
            location: location.clone(),
            offset,
            limit: PART_SIZE,
        };

        let response = client
            .invoke(&request)
            .await
            .map_err(|e| anyhow::anyhow!("upload.GetFile at offset {}: {}", offset, e))?;

        let bytes = match response {
            tl::enums::upload::File::File(f) => f.bytes,
            tl::enums::upload::File::CdnRedirect(_) => {
                return Err(anyhow::anyhow!("CDN redirect not supported"));
            }
        };

        if bytes.is_empty() {
            break;
        }

        file.write_all(&bytes).await?;
        downloaded += bytes.len() as u64;
        offset += bytes.len() as i64;

        if total_size > 0 {
            let percent = (downloaded as f64 / total_size as f64) * 100.0;
            let _ = progress_tx.send(percent.min(100.0)).await;
        }

        if (bytes.len() as i32) < PART_SIZE {
            break;
        }
    }

    file.flush().await?;
    tracing::info!(
        "[tg-parallel] sequential download complete: {} bytes",
        downloaded
    );
    Ok(downloaded)
}
