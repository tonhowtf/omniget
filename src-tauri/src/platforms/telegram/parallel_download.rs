use std::path::Path;
use std::sync::{Arc, OnceLock};

use grammers_client::Client;
use grammers_client::grammers_tl_types as tl;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

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

fn auth_copied_dcs() -> &'static Arc<Mutex<Vec<i32>>> {
    static INSTANCE: OnceLock<Arc<Mutex<Vec<i32>>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

async fn ensure_auth_on_dc(client: &Client, target_dc_id: i32) -> anyhow::Result<()> {
    {
        let copied = auth_copied_dcs().lock().await;
        if copied.contains(&target_dc_id) {
            return Ok(());
        }
    }

    tracing::info!("[tg-dl] copying auth to DC {}", target_dc_id);

    let tl::enums::auth::ExportedAuthorization::Authorization(exported) = client
        .invoke(&tl::functions::auth::ExportAuthorization {
            dc_id: target_dc_id,
        })
        .await
        .map_err(|e| anyhow::anyhow!("ExportAuthorization to DC {}: {}", target_dc_id, e))?;

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
    tracing::info!(
        "[tg-dl] starting download: size={}, dc={}, path={}",
        media.size, media.dc_id, output_path.display()
    );

    let mut dc = media.dc_id;

    ensure_auth_on_dc(client, dc).await?;

    let mut file = tokio::fs::File::create(output_path).await?;
    let mut downloaded: u64 = 0;
    let mut offset: i64 = 0;

    loop {
        if cancel_token.is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(output_path).await;
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

                file.write_all(&f.bytes).await?;
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
                tracing::warn!("[tg-dl] AUTH_KEY_UNREGISTERED on DC {}, re-copying auth", dc);
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
                let _ = tokio::fs::remove_file(output_path).await;
                return Err(anyhow::anyhow!("upload.getFile failed: {}", e));
            }
        }
    }

    file.flush().await?;
    tracing::info!("[tg-dl] download complete: {} bytes, dc={}", downloaded, dc);
    Ok(downloaded)
}
