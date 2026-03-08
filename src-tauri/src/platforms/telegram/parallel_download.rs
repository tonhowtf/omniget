use std::path::Path;

use grammers_client::Client;
use grammers_client::grammers_tl_types as tl;
use grammers_client::types::Downloadable;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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

/// Wraps an InputFileLocation + size so it can be used with `client.iter_download()`.
/// This lets grammers handle FILE_MIGRATE and AUTH_KEY_UNREGISTERED internally.
pub struct DownloadableLocation {
    location: tl::enums::InputFileLocation,
    file_size: u64,
}

impl DownloadableLocation {
    pub fn new(location: tl::enums::InputFileLocation, file_size: u64) -> Self {
        Self { location, file_size }
    }
}

impl Downloadable for DownloadableLocation {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(self.location.clone())
    }

    fn size(&self) -> Option<usize> {
        if self.file_size > 0 {
            Some(self.file_size as usize)
        } else {
            None
        }
    }
}

/// Download using grammers' built-in iter_download which handles FILE_MIGRATE
/// and AUTH_KEY_UNREGISTERED internally via copy_auth_to_dc().
pub async fn download_with_iter(
    client: &Client,
    downloadable: &impl Downloadable,
    total_size: u64,
    output_path: &Path,
    progress_tx: mpsc::Sender<f64>,
    cancel_token: &CancellationToken,
) -> anyhow::Result<u64> {
    tracing::info!(
        "[tg-download] starting download: size={}, path={}",
        total_size,
        output_path.display()
    );

    let mut download_iter = client.iter_download(downloadable);
    let mut file = tokio::fs::File::create(output_path).await?;
    let mut downloaded: u64 = 0;

    loop {
        if cancel_token.is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(output_path).await;
            return Err(anyhow::anyhow!("Download cancelled"));
        }

        match download_iter.next().await {
            Ok(Some(chunk)) => {
                file.write_all(&chunk).await?;
                downloaded += chunk.len() as u64;

                if total_size > 0 {
                    let percent = (downloaded as f64 / total_size as f64) * 100.0;
                    let _ = progress_tx.send(percent.min(100.0)).await;
                }
            }
            Ok(None) => break,
            Err(e) => {
                drop(file);
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(anyhow::anyhow!("Download failed: {}", e));
            }
        }
    }

    file.flush().await?;
    tracing::info!(
        "[tg-download] download complete: {} bytes",
        downloaded
    );
    Ok(downloaded)
}
