// Adapted from Litecord (MIT): the Remote Auth v2 handshake order (init, nonce_proof,
// pending_remote_init, pending_ticket, pending_login) and the ticket exchange.
use std::time::Duration;

use base64::Engine;
use futures::{SinkExt, StreamExt};
use rsa::pkcs8::EncodePublicKey;
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use super::{props, ws};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

pub fn generate_key(bits: usize) -> Result<RsaPrivateKey, String> {
    let mut rng = rsa::rand_core::OsRng;
    RsaPrivateKey::new(&mut rng, bits).map_err(|e| format!("rsa: {e}"))
}

pub fn public_key_der(key: &RsaPrivateKey) -> Result<Vec<u8>, String> {
    RsaPublicKey::from(key)
        .to_public_key_der()
        .map(|d| d.as_bytes().to_vec())
        .map_err(|e| format!("rsa: {e}"))
}

pub fn encoded_public_key(key: &RsaPrivateKey) -> Result<String, String> {
    public_key_der(key).map(|der| B64.encode(der))
}

pub fn fingerprint(key: &RsaPrivateKey) -> Result<String, String> {
    public_key_der(key).map(|der| B64URL.encode(Sha256::digest(der)))
}

pub fn decrypt_b64(key: &RsaPrivateKey, data: &str) -> Result<Vec<u8>, String> {
    let raw = B64
        .decode(data.trim())
        .map_err(|_| "remote auth: bad base64".to_string())?;
    key.decrypt(Oaep::new::<Sha256>(), &raw)
        .map_err(|_| "remote auth: could not decrypt".to_string())
}

pub fn nonce_proof(key: &RsaPrivateKey, encrypted_nonce: &str) -> Result<String, String> {
    let nonce = decrypt_b64(key, encrypted_nonce)?;
    Ok(B64URL.encode(Sha256::digest(nonce)))
}

pub fn qr_url(fingerprint: &str) -> String {
    format!("https://discord.com/ra/{fingerprint}")
}

pub fn qr_svg(content: &str) -> Result<String, String> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let code = QrCode::new(content.as_bytes()).map_err(|e| format!("qr: {e}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PendingUser {
    pub id: String,
    pub discriminator: String,
    pub avatar: Option<String>,
    pub username: String,
}

pub fn parse_user_payload(text: &str) -> Option<PendingUser> {
    let mut parts = text.splitn(4, ':');
    let id = parts.next()?.trim();
    let discriminator = parts.next()?.trim();
    let avatar = parts.next()?.trim();
    let username = parts.next()?.trim();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) || username.is_empty() {
        return None;
    }
    Some(PendingUser {
        id: id.to_string(),
        discriminator: discriminator.to_string(),
        avatar: match avatar {
            "" | "0" | "null" => None,
            a => Some(a.to_string()),
        },
        username: username.to_string(),
    })
}

pub fn looks_like_token(token: &str) -> bool {
    let t = token.trim();
    t.len() >= 50
        && t.len() <= 200
        && t.split('.').count() == 3
        && t.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

pub fn clean_token(raw: &str) -> String {
    let t = raw.trim().trim_matches('"').trim();
    let t = t
        .strip_prefix("Bearer ")
        .or_else(|| t.strip_prefix("Bot "))
        .unwrap_or(t);
    t.trim().to_string()
}

pub enum QrEvent {
    Code { svg: String, url: String },
    Scanned(PendingUser),
    Token(String),
    Cancelled,
    Expired,
    Failed(String),
}

pub async fn run_qr(cancel: CancellationToken, emit: impl Fn(QrEvent)) {
    let result = tokio::select! {
        _ = cancel.cancelled() => Ok(Some(QrEvent::Cancelled)),
        r = qr_session(&emit) => r,
    };
    match result {
        Ok(Some(ev)) => emit(ev),
        Ok(None) => emit(QrEvent::Expired),
        Err(e) => emit(QrEvent::Failed(e)),
    }
}

async fn send_json(sock: &mut ws::Ws, v: Value) -> Result<(), String> {
    sock.send(Message::Text(v.to_string().into()))
        .await
        .map_err(|e| format!("network: {e}"))
}

async fn qr_session(emit: &impl Fn(QrEvent)) -> Result<Option<QrEvent>, String> {
    let key = tokio::task::spawn_blocking(|| generate_key(2048))
        .await
        .map_err(|e| e.to_string())??;
    let own_fingerprint = fingerprint(&key)?;
    let mut sock = ws::connect(props::REMOTE_AUTH_URL).await?;
    let mut beat: Option<tokio::time::Interval> = None;
    loop {
        let next = async {
            match beat.as_mut() {
                Some(i) => {
                    i.tick().await;
                }
                None => std::future::pending::<()>().await,
            }
        };
        let msg = tokio::select! {
            _ = next => {
                send_json(&mut sock, json!({"op": "heartbeat"})).await?;
                continue;
            }
            m = sock.next() => m,
        };
        let text = match msg {
            None | Some(Ok(Message::Close(_))) => return Ok(None),
            Some(Err(e)) => return Err(format!("network: {e}")),
            Some(Ok(Message::Text(t))) => t.to_string(),
            Some(Ok(_)) => continue,
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        match v.get("op").and_then(|o| o.as_str()).unwrap_or("") {
            "hello" => {
                let ms = v
                    .get("heartbeat_interval")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(41_250)
                    .max(1_000);
                let mut i = tokio::time::interval(Duration::from_millis(ms));
                i.tick().await;
                beat = Some(i);
                let public = encoded_public_key(&key)?;
                send_json(
                    &mut sock,
                    json!({"op": "init", "encoded_public_key": public}),
                )
                .await?;
            }
            "nonce_proof" => {
                let enc = v
                    .get("encrypted_nonce")
                    .and_then(|x| x.as_str())
                    .ok_or("remote auth: missing nonce")?;
                let proof = nonce_proof(&key, enc)?;
                send_json(&mut sock, json!({"op": "nonce_proof", "proof": proof})).await?;
            }
            "pending_remote_init" => {
                let fp = v
                    .get("fingerprint")
                    .and_then(|x| x.as_str())
                    .ok_or("remote auth: missing fingerprint")?;
                if fp != own_fingerprint {
                    return Err("remote auth: fingerprint mismatch".into());
                }
                let url = qr_url(fp);
                emit(QrEvent::Code {
                    svg: qr_svg(&url)?,
                    url,
                });
            }
            "pending_ticket" => {
                if let Some(enc) = v.get("encrypted_user_payload").and_then(|x| x.as_str()) {
                    let raw = decrypt_b64(&key, enc)?;
                    if let Some(user) = parse_user_payload(&String::from_utf8_lossy(&raw)) {
                        emit(QrEvent::Scanned(user));
                    }
                }
            }
            "pending_login" => {
                let ticket = v
                    .get("ticket")
                    .and_then(|x| x.as_str())
                    .ok_or("remote auth: missing ticket")?
                    .to_string();
                let token = exchange_ticket(&ticket, &key).await?;
                let _ = sock.close(None).await;
                return Ok(Some(QrEvent::Token(token)));
            }
            "cancel" => {
                let _ = sock.close(None).await;
                return Ok(Some(QrEvent::Cancelled));
            }
            _ => {}
        }
    }
}

async fn exchange_ticket(ticket: &str, key: &RsaPrivateKey) -> Result<String, String> {
    let http = super::rest::http_client()?;
    let resp = http
        .post(format!("{}/users/@me/remote-auth/login", props::API_BASE))
        .headers(super::rest::web_headers(None))
        .json(&json!({ "ticket": ticket }))
        .send()
        .await
        .map_err(|e| format!("network: {}", e.without_url()))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if body.get("captcha_key").is_some() || body.get("captcha_sitekey").is_some() {
        return Err("captcha_required".into());
    }
    if !status.is_success() {
        return Err(format!("http_{}", status.as_u16()));
    }
    let enc = body
        .get("encrypted_token")
        .and_then(|x| x.as_str())
        .ok_or("remote auth: no token in response")?;
    let raw = decrypt_b64(key, enc)?;
    String::from_utf8(raw).map_err(|_| "remote auth: bad token".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn key() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        KEY.get_or_init(|| generate_key(1024).unwrap())
    }

    fn encrypt_for(key: &RsaPrivateKey, data: &[u8]) -> String {
        let public = RsaPublicKey::from(key);
        let mut rng = rsa::rand_core::OsRng;
        B64.encode(
            public
                .encrypt(&mut rng, Oaep::new::<Sha256>(), data)
                .unwrap(),
        )
    }

    #[test]
    fn public_key_is_base64_spki_der() {
        let enc = encoded_public_key(key()).unwrap();
        let der = B64.decode(&enc).unwrap();
        assert_eq!(der[0], 0x30);
        use rsa::pkcs8::DecodePublicKey;
        let back = RsaPublicKey::from_public_key_der(&der).unwrap();
        assert_eq!(back, RsaPublicKey::from(key()));
    }

    #[test]
    fn fingerprint_is_url_safe_sha256_of_the_der() {
        let fp = fingerprint(key()).unwrap();
        let der = public_key_der(key()).unwrap();
        assert_eq!(B64URL.decode(&fp).unwrap(), Sha256::digest(&der).to_vec());
        assert!(!fp.contains('=') && !fp.contains('+') && !fp.contains('/'));
        assert_eq!(fp.len(), 43);
        assert_eq!(qr_url(&fp), format!("https://discord.com/ra/{fp}"));
    }

    #[test]
    fn nonce_proof_hashes_the_decrypted_nonce() {
        let nonce = b"a-random-server-nonce-0123456789";
        let proof = nonce_proof(key(), &encrypt_for(key(), nonce)).unwrap();
        assert_eq!(proof, B64URL.encode(Sha256::digest(nonce)));
        assert!(nonce_proof(key(), "not base64!").is_err());
        assert!(nonce_proof(key(), &B64.encode([0u8; 128])).is_err());
    }

    #[test]
    fn user_payload_decrypts_and_parses() {
        let enc = encrypt_for(key(), b"123456789012345678:0:a1b2c3:some_user");
        let raw = decrypt_b64(key(), &enc).unwrap();
        let user = parse_user_payload(&String::from_utf8(raw).unwrap()).unwrap();
        assert_eq!(user.id, "123456789012345678");
        assert_eq!(user.discriminator, "0");
        assert_eq!(user.avatar.as_deref(), Some("a1b2c3"));
        assert_eq!(user.username, "some_user");
        assert_eq!(
            parse_user_payload("1:0:0:name:with:colons")
                .unwrap()
                .username,
            "name:with:colons"
        );
        assert!(parse_user_payload("1:0:0:name").unwrap().avatar.is_none());
        assert!(parse_user_payload("abc:0:0:name").is_none());
        assert!(parse_user_payload("1:0").is_none());
    }

    #[test]
    fn qr_svg_renders() {
        let svg = qr_svg("https://discord.com/ra/abc").unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn token_cleanup_and_shape() {
        let fake = format!("{}.{}.{}", "M".repeat(26), "G".repeat(6), "x".repeat(38));
        assert_eq!(clean_token(&format!("  \"{fake}\" ")), fake);
        assert_eq!(clean_token(&format!("Bearer {fake}")), fake);
        assert!(looks_like_token(&fake));
        assert!(!looks_like_token("hello"));
        assert!(!looks_like_token(&format!("{fake} spaces")));
    }
}
