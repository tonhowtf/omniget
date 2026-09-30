//! HTTPS on the remote listener with a pinned self-signed certificate
//! (Maestri Wire: "self-signed certificate, pinned by the SHA-256 of its
//! key"). The same port answers TLS and plain HTTP: the accept loop peeks
//! the first byte (0x16 = a TLS ClientHello).
//!
//! The identity is an Ed25519 key and a hand-built X.509 v3 certificate
//! (no extra crates: `ed25519-dalek` signs, DER is written here), kept in
//! `<app_data>/remote/tls.json` (0600). The pin is the SHA-256 of the
//! certificate's SubjectPublicKeyInfo, base64 (`sha256/…`) and hex; a client
//! trusts the listener when the presented key hashes to it, whatever the
//! name (a phone reaches the machine by LAN IP, `.local` or `.ts.net`).
//! Browsers do not take Ed25519 server certificates yet, so the page keeps
//! plain HTTP (or `tailscale serve` HTTPS) and native clients use the pin.

use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_rustls::rustls;

const OID_ED25519: &[u8] = &[0x06, 0x03, 0x2B, 0x65, 0x70];
const OID_CN: &[u8] = &[0x06, 0x03, 0x55, 0x04, 0x03];
const OID_SAN: &[u8] = &[0x06, 0x03, 0x55, 0x1D, 0x11];

#[derive(Serialize, Deserialize)]
struct Stored {
    seed: String,
    cert: String,
}

pub struct Identity {
    pub cert_der: Vec<u8>,
    pub key_pkcs8: Vec<u8>,
    /// base64 SHA-256 of the SubjectPublicKeyInfo.
    pub pin: String,
    pub pin_hex: String,
}

fn der_len(n: usize) -> Vec<u8> {
    if n < 0x80 {
        vec![n as u8]
    } else if n < 0x100 {
        vec![0x81, n as u8]
    } else {
        vec![0x82, (n >> 8) as u8, n as u8]
    }
}

fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    v.extend(der_len(body.len()));
    v.extend_from_slice(body);
    v
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

fn name(cn: &str) -> Vec<u8> {
    let atv = seq(&[OID_CN.to_vec(), tlv(0x0C, cn.as_bytes())]);
    seq(&[tlv(0x31, &atv)])
}

/// SubjectPublicKeyInfo of an Ed25519 public key.
pub fn spki(public: &[u8; 32]) -> Vec<u8> {
    let mut bits = vec![0u8];
    bits.extend_from_slice(public);
    seq(&[seq(&[OID_ED25519.to_vec()]), tlv(0x03, &bits)])
}

/// PKCS#8 v1 of an Ed25519 seed (what rustls/ring load).
fn pkcs8(seed: &[u8; 32]) -> Vec<u8> {
    seq(&[
        vec![0x02, 0x01, 0x00],
        seq(&[OID_ED25519.to_vec()]),
        tlv(0x04, &tlv(0x04, seed)),
    ])
}

fn build_cert(key: &ed25519_dalek::SigningKey) -> Vec<u8> {
    use ed25519_dalek::Signer;
    let public = key.verifying_key().to_bytes();
    let mut serial = [0u8; 16];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut serial);
    serial[0] &= 0x7F;
    serial[0] |= 0x01;
    let not_before = (chrono::Utc::now() - chrono::Duration::days(1))
        .format("%y%m%d%H%M%SZ")
        .to_string();
    let validity = seq(&[
        tlv(0x17, not_before.as_bytes()),
        tlv(0x17, b"491231235959Z"),
    ]);
    // subjectAltName: localhost, 127.0.0.1 (pinning clients ignore names).
    let san = seq(&[tlv(0x82, b"localhost"), tlv(0x87, &[127, 0, 0, 1])]);
    let ext = seq(&[OID_SAN.to_vec(), tlv(0x04, &san)]);
    let exts = tlv(0xA3, &seq(&[ext]));
    let alg = seq(&[OID_ED25519.to_vec()]);
    let tbs = seq(&[
        vec![0xA0, 0x03, 0x02, 0x01, 0x02],
        tlv(0x02, &serial),
        alg.clone(),
        name("OmniGet Remote"),
        validity,
        name("OmniGet Remote"),
        spki(&public),
        exts,
    ]);
    let sig = key.sign(&tbs).to_bytes();
    let mut bits = vec![0u8];
    bits.extend_from_slice(&sig);
    seq(&[tbs, alg, tlv(0x03, &bits)])
}

/// SHA-256 pin of a certificate we issued (its Ed25519 SPKI), or `None`.
pub fn pin_of_cert(der: &[u8]) -> Option<String> {
    let prefix = [
        0x30, 0x2A, 0x30, 0x05, 0x06, 0x03, 0x2B, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    let at = der.windows(prefix.len()).position(|w| w == prefix)?;
    let spki = der.get(at..at + 44)?;
    Some(base64::engine::general_purpose::STANDARD.encode(Sha256::digest(spki)))
}

fn path() -> Option<std::path::PathBuf> {
    Some(
        omniget_core::core::paths::app_data_dir()?
            .join("remote")
            .join("tls.json"),
    )
}

fn load_or_create() -> Result<Identity, String> {
    let p = path().ok_or("REMOTE_TLS: no app data dir")?;
    let b64 = base64::engine::general_purpose::STANDARD;
    let stored: Option<Stored> = std::fs::read(&p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let (seed, cert) = match stored.and_then(|s| {
        let seed: [u8; 32] = b64.decode(&s.seed).ok()?.try_into().ok()?;
        let cert = b64.decode(&s.cert).ok()?;
        Some((seed, cert))
    }) {
        Some(v) => v,
        None => {
            let mut seed = [0u8; 32];
            rand::Rng::fill_bytes(&mut rand::rng(), &mut seed);
            let key = ed25519_dalek::SigningKey::from_bytes(&seed);
            let cert = build_cert(&key);
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let body = serde_json::to_vec(&Stored {
                seed: b64.encode(seed),
                cert: b64.encode(&cert),
            })
            .map_err(|e| e.to_string())?;
            super::write_private(&p, &body).map_err(|e| format!("REMOTE_TLS: {e}"))?;
            (seed, cert)
        }
    };
    let key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let spki = spki(&key.verifying_key().to_bytes());
    let digest = Sha256::digest(&spki);
    Ok(Identity {
        cert_der: cert,
        key_pkcs8: pkcs8(&seed),
        pin: b64.encode(digest),
        pin_hex: hex::encode(digest),
    })
}

pub fn identity() -> Result<&'static Identity, String> {
    static ID: OnceLock<Result<Identity, String>> = OnceLock::new();
    ID.get_or_init(load_or_create)
        .as_ref()
        .map_err(|e| e.clone())
}

pub fn acceptor() -> Result<tokio_rustls::TlsAcceptor, String> {
    static ACC: OnceLock<Result<tokio_rustls::TlsAcceptor, String>> = OnceLock::new();
    ACC.get_or_init(|| {
        let id = identity()?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("REMOTE_TLS: {e}"))?
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(id.cert_der.clone())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(id.key_pkcs8.clone()),
                ),
            )
            .map_err(|e| format!("REMOTE_TLS: {e}"))?;
        Ok(tokio_rustls::TlsAcceptor::from(Arc::new(cfg)))
    })
    .clone()
}

/// `{pin: "sha256/…", pinHex, algorithm}` for `/api/info`, pairing and the
/// settings; `null` when the identity could not be made.
pub fn info() -> Value {
    match identity() {
        Ok(id) => json!({
            "pin": format!("sha256/{}", id.pin),
            "pinHex": id.pin_hex,
            "algorithm": "ed25519",
            "sameport": true,
        }),
        Err(_) => Value::Null,
    }
}

/// Client side (tests, the MCP `remote_call tls=true`): trusts exactly the
/// server whose certificate carries the pinned key.
#[derive(Debug)]
struct Pinned {
    pin: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        match pin_of_cert(end_entity.as_ref()) {
            Some(p) if p == self.pin => Ok(rustls::client::danger::ServerCertVerified::assertion()),
            _ => Err(rustls::Error::General("certificate pin mismatch".into())),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A rustls client config that accepts only `pin` (base64, `sha256/` optional).
pub fn pinned_client(pin: &str) -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let pin = pin.trim_start_matches("sha256/").to_string();
    rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("default versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { pin, provider }))
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_carries_the_pinned_key_and_loads_in_rustls() {
        let seed = [7u8; 32];
        let key = ed25519_dalek::SigningKey::from_bytes(&seed);
        let cert = build_cert(&key);
        let spki = spki(&key.verifying_key().to_bytes());
        let want = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&spki));
        assert_eq!(pin_of_cert(&cert).as_deref(), Some(want.as_str()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(cert)],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(pkcs8(&seed)),
                ),
            );
        assert!(cfg.is_ok(), "{:?}", cfg.err());
    }
}
