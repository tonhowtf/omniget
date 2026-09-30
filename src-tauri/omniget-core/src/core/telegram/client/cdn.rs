use std::time::Duration;

use aes::Aes256;
use cipher::{KeyIvInit, StreamCipher};
use grammers_client::grammers_tl_types as tl;
use grammers_client::Client;

type Aes256Ctr = ctr::Ctr64BE<Aes256>;

const PART_SIZE: i32 = 512 * 1024;
const CHUNK_TIMEOUT_SECS: u64 = 15;
const REUPLOAD_RETRY_LIMIT: u32 = 3;

pub async fn download_via_cdn(
    client: &Client,
    redirect: &tl::types::upload::FileCdnRedirect,
    expected_total: u64,
) -> anyhow::Result<Vec<u8>> {
    if redirect.encryption_key.len() != 32 {
        return Err(anyhow::anyhow!(
            "CDN encryption_key has wrong size: {}",
            redirect.encryption_key.len()
        ));
    }
    if redirect.encryption_iv.len() != 16 {
        return Err(anyhow::anyhow!(
            "CDN encryption_iv has wrong size: {}",
            redirect.encryption_iv.len()
        ));
    }

    super::parallel_download::ensure_auth_on_dc(client, redirect.dc_id).await?;

    let mut data: Vec<u8> = if expected_total > 0 {
        Vec::with_capacity(expected_total as usize)
    } else {
        Vec::new()
    };
    let mut offset: i64 = 0;

    let max_chunks = if expected_total > 0 {
        expected_total.div_ceil(PART_SIZE as u64).max(1) as usize + 4
    } else {
        4096
    };

    for chunk_idx in 0..max_chunks {
        if expected_total > 0 && (offset as u64) >= expected_total {
            break;
        }

        let req = tl::functions::upload::GetCdnFile {
            file_token: redirect.file_token.clone(),
            offset,
            limit: PART_SIZE,
        };

        let mut reupload_attempts = 0u32;
        let bytes = loop {
            let invoke_fut = client.invoke_in_dc(redirect.dc_id, &req);
            let resp =
                match tokio::time::timeout(Duration::from_secs(CHUNK_TIMEOUT_SECS), invoke_fut)
                    .await
                {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => return Err(anyhow::anyhow!("upload.GetCdnFile: {}", e)),
                    Err(_) => return Err(anyhow::anyhow!("upload.GetCdnFile timed out")),
                };
            match resp {
                tl::enums::upload::CdnFile::File(f) => {
                    break f.bytes;
                }
                tl::enums::upload::CdnFile::ReuploadNeeded(reupload) => {
                    if reupload_attempts >= REUPLOAD_RETRY_LIMIT {
                        return Err(anyhow::anyhow!(
                            "upload.GetCdnFile reupload retry limit exceeded"
                        ));
                    }
                    reupload_attempts += 1;
                    tracing::warn!(
                        "[tg-cdn] reupload needed (attempt {}/{})",
                        reupload_attempts,
                        REUPLOAD_RETRY_LIMIT
                    );
                    let reupload_req = tl::functions::upload::ReuploadCdnFile {
                        file_token: redirect.file_token.clone(),
                        request_token: reupload.request_token,
                    };
                    let _ = client
                        .invoke(&reupload_req)
                        .await
                        .map_err(|e| anyhow::anyhow!("upload.ReuploadCdnFile: {}", e))?;
                }
            }
        };

        if bytes.is_empty() {
            break;
        }

        let plaintext = decrypt_cdn_chunk(
            &bytes,
            &redirect.encryption_key,
            &redirect.encryption_iv,
            offset as u64,
        )?;
        let _ = chunk_idx;

        let len = plaintext.len();
        data.extend_from_slice(&plaintext);
        offset += len as i64;

        if (len as i32) < PART_SIZE {
            break;
        }
    }

    tracing::info!(
        "[tg-cdn] downloaded {} bytes via CDN dc={}",
        data.len(),
        redirect.dc_id
    );
    Ok(data)
}

fn decrypt_cdn_chunk(
    ciphertext: &[u8],
    key: &[u8],
    base_iv: &[u8],
    offset: u64,
) -> anyhow::Result<Vec<u8>> {
    if !offset.is_multiple_of(16) {
        return Err(anyhow::anyhow!(
            "CDN chunk offset must be 16-byte aligned, got {}",
            offset
        ));
    }
    let mut iv = [0u8; 16];
    iv.copy_from_slice(base_iv);
    let increment = offset / 16;
    let inc_bytes = increment.to_be_bytes();
    let mut carry: u16 = 0;
    for (i, b) in iv.iter_mut().enumerate().rev() {
        let inc_idx = i.wrapping_sub(8);
        let inc_byte = if i >= 8 { inc_bytes[inc_idx] } else { 0 };
        let total = *b as u16 + inc_byte as u16 + carry;
        *b = total as u8;
        carry = total >> 8;
    }
    let mut cipher = Aes256Ctr::new(key.into(), (&iv).into());
    let mut buf = ciphertext.to_vec();
    cipher.apply_keystream(&mut buf);
    Ok(buf)
}
