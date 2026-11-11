use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Mode};

use super::listing::{self, Entry, Format};
use super::{
    pump, within, Auth, Connection, Progress, Protocol, PumpError, TransferError, CONNECT_SECONDS,
    IDLE_SECONDS, STEP_SECONDS,
};

const ANONYMOUS_USER: &str = "anonymous";
const ANONYMOUS_PASSWORD: &str = "anonymous@";

pub fn fingerprint(der: &[u8]) -> String {
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(Sha256::digest(der))
    )
}

#[derive(Debug)]
struct Pinning {
    seen: Mutex<Option<String>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinning {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let presented = fingerprint(end_entity.as_ref());
        let mut seen = self
            .seen
            .lock()
            .map_err(|_| rustls::Error::General("certificate state is unavailable".into()))?;
        match seen.as_ref() {
            Some(first) if *first != presented => Err(rustls::Error::General(
                "the data connection presented another certificate than the control connection"
                    .into(),
            )),
            Some(_) => Ok(ServerCertVerified::assertion()),
            None => {
                *seen = Some(presented);
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
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
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn from_status(code: u32, body: &str, what: &str, connection: &Connection) -> TransferError {
    let text = body.trim();
    let lower = text.to_lowercase();
    match code {
        530 | 331 | 332 => connection.auth_failed(),
        421 => connection.unreachable(text),
        550 | 553 | 450 | 451 | 452 | 532 => {
            if lower.contains("permission")
                || lower.contains("denied")
                || lower.contains("not allowed")
            {
                TransferError::PermissionDenied(what.to_string())
            } else if lower.contains("not empty") {
                TransferError::NotEmpty(what.to_string())
            } else if lower.contains("exist")
                && !lower.contains("not exist")
                && !lower.contains("n't exist")
                && !lower.contains("no such")
            {
                TransferError::AlreadyExists(what.to_string())
            } else if code == 550 || lower.contains("no such") || lower.contains("not found") {
                TransferError::NotFound(what.to_string())
            } else {
                TransferError::Server(format!("{code} {text}"))
            }
        }
        500..=504 => TransferError::Unsupported(format!("that command ({code} {text})")),
        _ => TransferError::Server(format!("{code} {text}")),
    }
}

fn from_io(error: &std::io::Error, what: &str, connection: &Connection) -> TransferError {
    if error.kind() == std::io::ErrorKind::TimedOut {
        return TransferError::Timeout {
            what: what.to_string(),
            seconds: CONNECT_SECONDS,
        };
    }
    connection.unreachable(error)
}

fn failed(error: FtpError, what: &str, connection: &Connection) -> TransferError {
    match error {
        FtpError::ConnectionError(e) => from_io(&e, what, connection),
        FtpError::SecureError(message) => TransferError::Tls(message),
        FtpError::UnexpectedResponse(response) => from_status(
            response.status.code(),
            &String::from_utf8_lossy(&response.body),
            what,
            connection,
        ),
        other => TransferError::Server(other.to_string()),
    }
}

fn not_understood(error: &FtpError) -> bool {
    matches!(error, FtpError::UnexpectedResponse(response) if (500..=504).contains(&response.status.code()))
}

pub struct Session {
    stream: AsyncRustlsFtpStream,
    connection: Connection,
}

impl Session {
    pub async fn open(connection: &Connection) -> Result<Self, TransferError> {
        let what = format!("connecting to {}", connection.place());
        let stream = within(CONNECT_SECONDS, &what, Self::connect(connection)).await?;
        Ok(Self {
            stream,
            connection: connection.clone(),
        })
    }

    async fn connect(connection: &Connection) -> Result<AsyncRustlsFtpStream, TransferError> {
        let what = "connecting";
        let address = tokio::net::lookup_host((connection.host.as_str(), connection.port))
            .await
            .map_err(|e| connection.unreachable(e))?
            .next()
            .ok_or_else(|| connection.unreachable("the name has no address"))?;
        let mut stream =
            AsyncRustlsFtpStream::connect_timeout(address, Duration::from_secs(CONNECT_SECONDS))
                .await
                .map_err(|e| failed(e, what, connection))?;
        if connection.protocol == Protocol::Ftps {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let pinning = Arc::new(Pinning {
                seen: Mutex::new(None),
                provider: provider.clone(),
            });
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| TransferError::Tls(e.to_string()))?
                .dangerous()
                .with_custom_certificate_verifier(pinning.clone())
                .with_no_client_auth();
            let connector =
                AsyncRustlsConnector::from(tokio_rustls::TlsConnector::from(Arc::new(config)));
            stream = stream
                .into_secure(connector, &connection.host)
                .await
                .map_err(|e| match e {
                    FtpError::UnexpectedResponse(response) => TransferError::Tls(format!(
                        "the server refused AUTH TLS ({})",
                        response.status.code()
                    )),
                    other => failed(other, what, connection),
                })?;
            let presented = pinning
                .seen
                .lock()
                .ok()
                .and_then(|seen| seen.clone())
                .ok_or_else(|| TransferError::Tls("the server sent no certificate".into()))?;
            connection.trust("tls", presented).await?;
        }
        let (user, password) = match &connection.auth {
            Auth::Password(password) => (connection.user.as_str(), password.as_str()),
            _ if connection.user.is_empty() => (ANONYMOUS_USER, ANONYMOUS_PASSWORD),
            _ => (connection.user.as_str(), ""),
        };
        stream.login(user, password).await.map_err(|e| match e {
            FtpError::UnexpectedResponse(_) => connection.auth_failed(),
            other => failed(other, what, connection),
        })?;
        stream.set_passive_nat_workaround(true);
        if address.is_ipv6() {
            stream.set_mode(Mode::ExtendedPassive);
        }
        stream
            .transfer_type(FileType::Binary)
            .await
            .map_err(|e| failed(e, what, connection))?;
        Ok(stream)
    }

    pub async fn home(&mut self) -> Result<String, TransferError> {
        let connection = &self.connection;
        within(STEP_SECONDS, "reading the start folder", async {
            self.stream
                .pwd()
                .await
                .map_err(|e| failed(e, "the start folder", connection))
        })
        .await
    }

    pub async fn read_dir(&mut self, path: &str) -> Result<(Vec<Entry>, usize), TransferError> {
        let connection = &self.connection;
        let stream = &mut self.stream;
        within(STEP_SECONDS, &format!("listing {path}"), async {
            stream
                .cwd(path)
                .await
                .map_err(|e| failed(e, path, connection))?;
            let (lines, format) = match stream.mlsd(None).await {
                Ok(lines) => (lines, Format::Mlsd),
                Err(e) if not_understood(&e) => (
                    stream
                        .list(None)
                        .await
                        .map_err(|e| failed(e, path, connection))?,
                    Format::List,
                ),
                Err(e) => return Err(failed(e, path, connection)),
            };
            Ok(listing::parse_lines(&lines, format))
        })
        .await
    }

    pub async fn download(
        &mut self,
        path: &str,
        file: &mut tokio::fs::File,
        local: &Path,
        progress: &Progress<'_>,
    ) -> Result<u64, TransferError> {
        let connection = &self.connection;
        let mut remote = within(STEP_SECONDS, &format!("opening {path}"), async {
            self.stream
                .retr_as_stream(path)
                .await
                .map_err(|e| failed(e, path, connection))
        })
        .await?;
        let copied = pump(
            &mut remote,
            file,
            Duration::from_secs(IDLE_SECONDS),
            progress,
        )
        .await
        .map_err(|e| match e {
            PumpError::Read(e) => connection.unreachable(e),
            PumpError::Write(e) => TransferError::Local {
                path: local.to_path_buf(),
                message: e.to_string(),
            },
            PumpError::Idle => TransferError::Timeout {
                what: format!("downloading {path}"),
                seconds: IDLE_SECONDS,
            },
        })?;
        within(STEP_SECONDS, &format!("finishing {path}"), async {
            remote
                .finish()
                .await
                .map_err(|e| failed(e, path, connection))
        })
        .await?;
        Ok(copied)
    }

    pub async fn upload(
        &mut self,
        path: &str,
        file: &mut tokio::fs::File,
        local: &Path,
        progress: &Progress<'_>,
    ) -> Result<u64, TransferError> {
        let connection = &self.connection;
        let mut remote = within(STEP_SECONDS, &format!("opening {path}"), async {
            self.stream
                .put_with_stream(path)
                .await
                .map_err(|e| failed(e, path, connection))
        })
        .await?;
        let copied = pump(
            file,
            &mut remote,
            Duration::from_secs(IDLE_SECONDS),
            progress,
        )
        .await
        .map_err(|e| match e {
            PumpError::Write(e) => connection.unreachable(e),
            PumpError::Read(e) => TransferError::Local {
                path: local.to_path_buf(),
                message: e.to_string(),
            },
            PumpError::Idle => TransferError::Timeout {
                what: format!("uploading {path}"),
                seconds: IDLE_SECONDS,
            },
        })?;
        within(STEP_SECONDS, &format!("finishing {path}"), async {
            remote
                .finish()
                .await
                .map_err(|e| failed(e, path, connection))
        })
        .await?;
        Ok(copied)
    }

    pub async fn mkdir(&mut self, path: &str) -> Result<(), TransferError> {
        let connection = &self.connection;
        within(STEP_SECONDS, &format!("creating {path}"), async {
            self.stream
                .mkdir(path)
                .await
                .map_err(|e| failed(e, path, connection))
        })
        .await
    }

    pub async fn rename(&mut self, from: &str, to: &str) -> Result<(), TransferError> {
        let connection = &self.connection;
        within(STEP_SECONDS, &format!("renaming {from}"), async {
            self.stream
                .rename(from, to)
                .await
                .map_err(|e| failed(e, from, connection))
        })
        .await
    }

    pub async fn remove_file(&mut self, path: &str) -> Result<(), TransferError> {
        let connection = &self.connection;
        within(STEP_SECONDS, &format!("deleting {path}"), async {
            self.stream
                .rm(path)
                .await
                .map_err(|e| failed(e, path, connection))
        })
        .await
    }

    pub async fn remove_dir(&mut self, path: &str) -> Result<(), TransferError> {
        let connection = &self.connection;
        within(STEP_SECONDS, &format!("deleting {path}"), async {
            self.stream
                .rmdir(path)
                .await
                .map_err(|e| failed(e, path, connection))
        })
        .await
    }

    pub async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.stream.quit()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(saved: bool) -> Connection {
        Connection {
            protocol: Protocol::Ftp,
            host: "console.local".into(),
            port: 5000,
            user: String::new(),
            auth: Auth::Anonymous,
            saved,
            known_hosts: "/nonexistent/known_hosts".into(),
        }
    }

    #[test]
    fn server_replies_become_codes_an_agent_can_act_on() {
        let c = connection(false);
        let of = |code: u32, body: &str| from_status(code, body, "/a/b.txt", &c);
        assert_eq!(of(530, "Login incorrect."), c.auth_failed());
        assert_eq!(
            of(550, "Failed to open file."),
            TransferError::NotFound("/a/b.txt".into())
        );
        assert_eq!(
            of(550, "b.txt: No such file or directory"),
            TransferError::NotFound("/a/b.txt".into())
        );
        assert_eq!(
            of(550, "Permission denied."),
            TransferError::PermissionDenied("/a/b.txt".into())
        );
        assert_eq!(
            of(553, "Could not create file: Permission denied"),
            TransferError::PermissionDenied("/a/b.txt".into())
        );
        assert_eq!(
            of(
                550,
                "Remove directory operation failed: Directory not empty"
            ),
            TransferError::NotEmpty("/a/b.txt".into())
        );
        assert_eq!(
            of(550, "Create directory operation failed: File exists"),
            TransferError::AlreadyExists("/a/b.txt".into())
        );
        assert_eq!(
            of(550, "File does not exist"),
            TransferError::NotFound("/a/b.txt".into())
        );
        assert!(matches!(
            of(502, "Command not implemented."),
            TransferError::Unsupported(_)
        ));
        assert!(matches!(
            of(421, "Too many users"),
            TransferError::HostUnreachable { .. }
        ));
        assert_eq!(
            of(552, "Quota exceeded"),
            TransferError::Server("552 Quota exceeded".into())
        );
    }

    #[test]
    fn a_failed_login_remembers_whether_the_credentials_were_saved() {
        assert_eq!(
            from_status(530, "no", "login", &connection(true)),
            TransferError::AuthFailed {
                host: "console.local:5000".into(),
                saved: true
            }
        );
    }

    #[test]
    fn a_refused_connection_is_unreachable_and_a_slow_one_is_a_timeout() {
        let c = connection(false);
        let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        assert!(matches!(
            from_io(&refused, "connecting", &c),
            TransferError::HostUnreachable { .. }
        ));
        let slow = std::io::Error::from(std::io::ErrorKind::TimedOut);
        assert!(matches!(
            from_io(&slow, "connecting", &c),
            TransferError::Timeout { .. }
        ));
    }

    #[test]
    fn a_certificate_fingerprint_is_stable_and_differs_per_certificate() {
        assert_eq!(fingerprint(b"cert"), fingerprint(b"cert"));
        assert_ne!(fingerprint(b"cert"), fingerprint(b"other"));
        assert!(fingerprint(b"cert").starts_with("SHA256:"));
    }

    #[test]
    fn the_data_connection_must_present_the_certificate_of_the_control_connection() {
        let pinning = Pinning {
            seen: Mutex::new(None),
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        };
        let name = ServerName::try_from("console.local").unwrap();
        let check = |der: &'static [u8]| {
            pinning.verify_server_cert(&CertificateDer::from(der), &[], &name, &[], UnixTime::now())
        };
        assert!(check(b"first").is_ok());
        assert!(check(b"first").is_ok());
        assert!(check(b"second").is_err());
    }
}
