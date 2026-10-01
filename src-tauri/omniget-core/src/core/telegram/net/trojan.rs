use std::sync::Arc;

use anyhow::{anyhow, Context};
use sha2::{Digest, Sha224};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::rustls;

use super::route::Route;
use super::tunnel::{socks_addr, BoxStream, Target};

pub async fn connect(tcp: TcpStream, route: &Route, target: &Target) -> anyhow::Result<BoxStream> {
    let sni = route.sni.clone().unwrap_or_else(|| route.host.clone());
    let mut tls = tls_connect(tcp, &sni, route.insecure).await?;
    let mut head = Vec::with_capacity(96);
    let hash = Sha224::digest(route.password.as_deref().unwrap_or("").as_bytes());
    head.extend_from_slice(hex::encode(hash).as_bytes());
    head.extend_from_slice(b"\r\n\x01");
    head.extend(socks_addr(target));
    head.extend_from_slice(b"\r\n");
    tls.write_all(&head).await.context("Trojan handshake")?;
    Ok(Box::pin(tls))
}

pub async fn tls_connect(
    tcp: TcpStream,
    sni: &str,
    insecure: bool,
) -> anyhow::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let config = client_config(insecure);
    let name = rustls::pki_types::ServerName::try_from(sni.to_string())
        .map_err(|_| anyhow!("invalid TLS server name {sni}"))?;
    tokio_rustls::TlsConnector::from(config)
        .connect(name, tcp)
        .await
        .with_context(|| format!("TLS handshake with {sni}"))
}

fn client_config(insecure: bool) -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("rustls protocol versions");
    let mut cfg = if insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(cfg)
}

#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
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
            &self.0.signature_verification_algorithms,
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
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
