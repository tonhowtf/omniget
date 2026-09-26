//! Retrieves an owned provider locator. Never expose this as a model URL-fetch tool.
use crate::core::egress::{Broker, Policy};
use reqwest::{header, Url};
use sha2::{Digest, Sha256};
use std::{sync::LazyLock, time::Duration};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const MAX_BYTES: usize = 64 * 1024 * 1024;
static IN_FLIGHT: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));
pub(crate) struct Collected {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub digest: String,
}
fn fail(code: &str) -> String {
    code.to_owned()
}
fn locator(raw: &str) -> Result<Url, String> {
    if raw.len() > 16 * 1024 {
        return Err(fail("MEDIA_LOCATOR_INVALID"));
    }
    let url = Url::parse(raw).map_err(|_| fail("MEDIA_LOCATOR_INVALID"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.fragment().is_some()
    {
        return Err(fail("MEDIA_LOCATOR_INVALID"));
    }
    Ok(url)
}
fn redirect(current: &Url, location: &str) -> Result<Url, String> {
    let joined = current
        .join(location)
        .map_err(|_| fail("MEDIA_REDIRECT_INVALID"))?;
    locator(joined.as_str()).map_err(|_| fail("MEDIA_REDIRECT_DENIED"))
}
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some("audio/wav")
    } else if bytes.starts_with(b"OggS\x00") {
        Some("audio/ogg")
    } else if bytes.starts_with(b"ID3") && bytes.get(3).is_some_and(|v| (2..=4).contains(v))
        || bytes.len() >= 4
            && bytes[0] == 0xff
            && bytes[1] & 0xe0 == 0xe0
            && bytes[1] & 0x18 != 0x08
            && bytes[1] & 0x06 == 0x02
            && bytes[2] & 0xf0 != 0
            && bytes[2] & 0xf0 != 0xf0
            && bytes[2] & 0x0c != 0x0c
    {
        Some("audio/mpeg")
    } else if bytes.len() >= 16
        && &bytes[4..8] == b"ftyp"
        && u32::from_be_bytes(bytes[..4].try_into().ok()?) >= 16
        && matches!(
            &bytes[8..12],
            b"isom" | b"iso2" | b"mp41" | b"mp42" | b"avc1" | b"dash" | b"M4V "
        )
    {
        Some("video/mp4")
    } else if bytes.starts_with(b"\x1a\x45\xdf\xa3")
        && bytes[..bytes.len().min(4096)]
            .windows(7)
            .any(|w| w == b"\x42\x82\x84webm")
    {
        Some("video/webm")
    } else {
        None
    }
}
fn validate_media(bytes: Vec<u8>, declared: &str) -> Result<Collected, String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(fail("MEDIA_SIZE_INVALID"));
    }
    let mime = sniff(&bytes).ok_or_else(|| fail("MEDIA_MAGIC_UNSUPPORTED"))?;
    let declared = declared
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let declared = match declared.as_str() {
        "audio/x-wav" | "audio/wave" => "audio/wav",
        "audio/mp3" => "audio/mpeg",
        "application/ogg" => "audio/ogg",
        value => value,
    };
    if declared != mime && declared != "application/octet-stream" {
        return Err(fail("MEDIA_MIME_MISMATCH"));
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Ok(Collected {
        bytes,
        mime: mime.to_owned(),
        digest,
    })
}
async fn receive(client: &reqwest::Client, mut url: Url) -> Result<Collected, String> {
    for hop in 0..=5 {
        // A new execute re-applies proxy authentication at every hop.
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| fail("MEDIA_TRANSFER_FAILED"))?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if hop == 5 {
                return Err(fail("MEDIA_REDIRECT_LIMIT"));
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| fail("MEDIA_REDIRECT_INVALID"))?;
            url = redirect(&url, location)?;
            continue;
        }
        if response.status().as_u16() == 429 {
            return Err(fail("MEDIA_RATE_LIMITED"));
        }
        if response.status() != reqwest::StatusCode::OK {
            return Err(fail("MEDIA_HTTP_REJECTED"));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BYTES as u64)
        {
            return Err(fail("MEDIA_SIZE_LIMIT"));
        }
        let declared = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| fail("MEDIA_MIME_MISSING"))?
            .to_owned();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| fail("MEDIA_TRANSFER_FAILED"))?
        {
            if chunk.len() > MAX_BYTES - bytes.len() {
                return Err(fail("MEDIA_SIZE_LIMIT"));
            }
            bytes.extend_from_slice(&chunk);
        }
        return validate_media(bytes, &declared);
    }
    Err(fail("MEDIA_REDIRECT_LIMIT"))
}
pub(crate) async fn collect_bytes(raw: &str) -> Result<Collected, String> {
    let url = locator(raw)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let _permit = tokio::time::timeout_at(deadline, IN_FLIGHT.acquire())
        .await
        .map_err(|_| fail("MEDIA_TIMEOUT"))?
        .map_err(|_| fail("MEDIA_UNAVAILABLE"))?;
    let policy = Policy {
        public_ports: [443].into(),
        max_connections: 6,
        max_concurrent: 1,
        max_bytes: (MAX_BYTES as u64) + 2 * 1024 * 1024,
        lifetime: Duration::from_secs(60),
        connect_timeout: Duration::from_secs(15),
        ..Policy::default()
    };
    let broker = Broker::start(policy, CancellationToken::new())
        .await
        .map_err(|_| fail("MEDIA_BROKER_UNAVAILABLE"))?;
    // Broker Drop revokes on caller cancellation, including while reading a body.
    let result = tokio::time::timeout_at(deadline, async {
        let proxy =
            reqwest::Proxy::all(broker.proxy_url()).map_err(|_| fail("MEDIA_PROXY_INVALID"))?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(proxy)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| fail("MEDIA_CLIENT_UNAVAILABLE"))?;
        receive(&client, url).await
    })
    .await
    .map_err(|_| fail("MEDIA_TIMEOUT"))
    .and_then(|v| v);
    broker.shutdown().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_untrusted_locators_and_downgrade() {
        for raw in [
            "http://example.com/a",
            "https://u:p@example.com/a",
            "https://example.com:444/a",
            "file:///tmp/a",
            "https://example.com/a#x",
        ] {
            assert!(locator(raw).is_err());
        }
        let base = locator("https://example.com/a").unwrap();
        assert!(redirect(&base, "/b").is_ok());
        assert!(redirect(&base, "https://cdn.example.com/b").is_ok());
        assert!(redirect(&base, "http://example.com/b").is_err());
    }
    #[test]
    fn checks_magic_mime_and_digest() {
        let png = b"\x89PNG\r\n\x1a\nfixture".to_vec();
        let collected = validate_media(png.clone(), "image/png; charset=binary").unwrap();
        assert_eq!(collected.mime, "image/png");
        assert_eq!(collected.bytes, png);
        assert_eq!(collected.digest, format!("{:x}", Sha256::digest(&png)));
        assert!(validate_media(png.clone(), "application/octet-stream").is_ok());
        assert!(validate_media(png, "image/jpeg").is_err());
        assert!(validate_media(b"<html>failure</html>".to_vec(), "image/png").is_err());
        assert!(validate_media(Vec::new(), "image/png").is_err());
    }
    #[tokio::test]
    async fn controlled_http_transport_is_bounded_and_strips_proxy_secret() {
        // HTTP exists only in this internal fixture. collect_bytes always rejects it.
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut policy = Policy::default();
        policy.local_allowances.insert(address);
        let broker = Broker::start(policy, CancellationToken::new())
            .await
            .unwrap();
        let server = tokio::spawn(async move {
            for index in 0..3 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                }
                assert!(!String::from_utf8(request)
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("proxy-authorization"));
                let response = match index {
                    0 => b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 8\r\nConnection: close\r\n\r\n\x89PNG\r\n\x1a\n".to_vec(),
                    1 => b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                    _ => b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 67108865\r\nConnection: close\r\n\r\n".to_vec(),
                };
                stream.write_all(&response).await.unwrap();
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(broker.proxy_url()).unwrap())
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        let url = Url::parse(&format!("http://{address}/fixture")).unwrap();
        assert_eq!(
            receive(&client, url.clone()).await.unwrap().mime,
            "image/png"
        );
        assert!(matches!(receive(&client,url.clone()).await,Err(e) if e=="MEDIA_RATE_LIMITED"));
        assert!(matches!(receive(&client,url).await,Err(e) if e=="MEDIA_SIZE_LIMIT"));
        server.await.unwrap();
        broker.shutdown().await;
    }
    #[test]
    fn allowlist_signatures() {
        for (bytes, mime) in [
            (&b"\xff\xd8\xffx"[..], "image/jpeg"),
            (&b"GIF89ax"[..], "image/gif"),
            (&b"RIFFxxxxWEBP"[..], "image/webp"),
            (&b"RIFFxxxxWAVE"[..], "audio/wav"),
            (&b"OggS\0xxxx"[..], "audio/ogg"),
            (&b"ID3\x04xxxx"[..], "audio/mpeg"),
            (&b"\0\0\0\x18ftypisomxxxx"[..], "video/mp4"),
            (&b"\x1a\x45\xdf\xa3x\x42\x82\x84webm"[..], "video/webm"),
        ] {
            assert_eq!(sniff(bytes), Some(mime));
        }
        assert_eq!(sniff(b"\0\0\0\x18ftypavifxxxx"), None);
    }
}
