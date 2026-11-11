use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, StatusCode};
use tokio::io::AsyncWriteExt;

use super::listing::{self, Entry, Kind};
use super::{
    pump, within, Auth, Connection, Progress, PumpError, TransferError, CONNECT_SECONDS,
    IDLE_SECONDS, STEP_SECONDS,
};

struct Handler {
    connection: Connection,
    refusal: Arc<Mutex<Option<TransferError>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256),
            PublicKeyOrCertificate::Certificate(certificate) => {
                certificate.public_key().fingerprint(HashAlg::Sha256)
            }
        };
        match self.connection.trust("ssh", fingerprint.to_string()).await {
            Ok(()) => Ok(true),
            Err(refused) => {
                if let Ok(mut slot) = self.refusal.lock() {
                    *slot = Some(refused);
                }
                Ok(false)
            }
        }
    }
}

pub fn from_status(code: StatusCode, message: &str, what: &str) -> TransferError {
    match code {
        StatusCode::NoSuchFile => TransferError::NotFound(what.to_string()),
        StatusCode::PermissionDenied => TransferError::PermissionDenied(what.to_string()),
        StatusCode::OpUnsupported => TransferError::Unsupported("that operation".into()),
        other => TransferError::Server(
            format!("{other}: {message}")
                .trim_end_matches(": ")
                .to_string(),
        ),
    }
}

fn failed(error: SftpError, what: &str) -> TransferError {
    match error {
        SftpError::Status(status) => from_status(status.status_code, &status.error_message, what),
        SftpError::Timeout => TransferError::Timeout {
            what: what.to_string(),
            seconds: STEP_SECONDS,
        },
        other => TransferError::Server(other.to_string()),
    }
}

pub fn entry(name: String, attributes: &FileAttributes) -> Entry {
    let file_type = attributes.file_type();
    let kind = if file_type.is_dir() {
        Kind::Dir
    } else if file_type.is_symlink() {
        Kind::Link
    } else if file_type.is_file() {
        Kind::File
    } else {
        Kind::Other
    };
    Entry {
        name,
        kind,
        size: (kind == Kind::File).then_some(attributes.size).flatten(),
        modified: attributes.mtime.and_then(|seconds| {
            listing::timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(seconds)))
        }),
    }
}

pub struct Session {
    handle: client::Handle<Handler>,
    sftp: SftpSession,
    connection: Connection,
}

impl Session {
    pub async fn open(connection: &Connection) -> Result<Self, TransferError> {
        let what = format!("connecting to {}", connection.place());
        within(CONNECT_SECONDS, &what, Self::connect(connection)).await
    }

    async fn connect(connection: &Connection) -> Result<Self, TransferError> {
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(IDLE_SECONDS * 2)),
            keepalive_interval: Some(Duration::from_secs(IDLE_SECONDS / 2)),
            ..client::Config::default()
        });
        let refusal = Arc::new(Mutex::new(None));
        let handler = Handler {
            connection: connection.clone(),
            refusal: refusal.clone(),
        };
        let mut handle =
            client::connect(config, (connection.host.as_str(), connection.port), handler)
                .await
                .map_err(|e| {
                    refusal
                        .lock()
                        .ok()
                        .and_then(|mut slot| slot.take())
                        .unwrap_or_else(|| connection.unreachable(e))
                })?;
        let unreachable = |e: russh::Error| connection.unreachable(e);
        let accepted = match &connection.auth {
            Auth::Password(password) => handle
                .authenticate_password(connection.user.as_str(), password.as_str())
                .await
                .map_err(unreachable)?,
            Auth::Key { path, passphrase } => {
                let (file, secret) = (path.clone(), passphrase.clone());
                let key = tokio::task::spawn_blocking(move || {
                    russh::keys::load_secret_key(&file, secret.as_deref())
                })
                .await
                .map_err(|e| TransferError::Storage(e.to_string()))?
                .map_err(|e| TransferError::KeyFile {
                    path: path.clone(),
                    message: e.to_string(),
                })?;
                let hash = handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(unreachable)?
                    .flatten();
                handle
                    .authenticate_publickey(
                        connection.user.as_str(),
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await
                    .map_err(unreachable)?
            }
            Auth::Anonymous => handle
                .authenticate_none(connection.user.as_str())
                .await
                .map_err(unreachable)?,
        };
        if !accepted.success() {
            return Err(connection.auth_failed());
        }
        let channel = handle.channel_open_session().await.map_err(unreachable)?;
        channel.request_subsystem(true, "sftp").await.map_err(|_| {
            TransferError::Unsupported("SFTP (the SSH server has no sftp subsystem)".into())
        })?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| failed(e, "starting SFTP"))?;
        sftp.set_timeout(STEP_SECONDS);
        Ok(Self {
            handle,
            sftp,
            connection: connection.clone(),
        })
    }

    pub async fn home(&mut self) -> Result<String, TransferError> {
        self.sftp
            .canonicalize(".")
            .await
            .map_err(|e| failed(e, "the start folder"))
    }

    pub async fn read_dir(&mut self, path: &str) -> Result<(Vec<Entry>, usize), TransferError> {
        let found = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|e| failed(e, path))?;
        let entries = found
            .filter(|found| !matches!(found.file_name().as_str(), "." | ".."))
            .map(|found| entry(found.file_name(), &found.metadata()))
            .collect();
        Ok((entries, 0))
    }

    pub async fn stat(&mut self, path: &str) -> Result<Option<Entry>, TransferError> {
        match self.sftp.symlink_metadata(path).await {
            Ok(attributes) => Ok(Some(entry(
                super::paths::file_name(path).unwrap_or("/").to_string(),
                &attributes,
            ))),
            Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                Ok(None)
            }
            Err(e) => Err(failed(e, path)),
        }
    }

    pub async fn download(
        &mut self,
        path: &str,
        file: &mut tokio::fs::File,
        local: &Path,
        progress: &Progress<'_>,
    ) -> Result<u64, TransferError> {
        let mut remote = self.sftp.open(path).await.map_err(|e| failed(e, path))?;
        let connection = &self.connection;
        pump(
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
        })
    }

    pub async fn upload(
        &mut self,
        path: &str,
        file: &mut tokio::fs::File,
        local: &Path,
        progress: &Progress<'_>,
    ) -> Result<u64, TransferError> {
        let mut remote = self.sftp.create(path).await.map_err(|e| failed(e, path))?;
        let connection = &self.connection;
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
                .shutdown()
                .await
                .map_err(|e| connection.unreachable(e))
        })
        .await?;
        Ok(copied)
    }

    pub async fn mkdir(&mut self, path: &str) -> Result<(), TransferError> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| failed(e, path))
    }

    pub async fn rename(&mut self, from: &str, to: &str) -> Result<(), TransferError> {
        self.sftp
            .rename(from, to)
            .await
            .map_err(|e| failed(e, from))
    }

    pub async fn remove_file(&mut self, path: &str) -> Result<(), TransferError> {
        self.sftp
            .remove_file(path)
            .await
            .map_err(|e| failed(e, path))
    }

    pub async fn remove_dir(&mut self, path: &str) -> Result<(), TransferError> {
        self.sftp
            .remove_dir(path)
            .await
            .map_err(|e| failed(e, path))
    }

    pub async fn close(self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            let _ = self.sftp.close().await;
            let _ = self
                .handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_become_codes_an_agent_can_act_on() {
        assert_eq!(
            from_status(StatusCode::NoSuchFile, "No such file", "/a"),
            TransferError::NotFound("/a".into())
        );
        assert_eq!(
            from_status(StatusCode::PermissionDenied, "", "/a"),
            TransferError::PermissionDenied("/a".into())
        );
        assert!(matches!(
            from_status(StatusCode::OpUnsupported, "", "/a"),
            TransferError::Unsupported(_)
        ));
        assert_eq!(
            from_status(StatusCode::Failure, "Directory not empty", "/a"),
            TransferError::Server("Failure: Directory not empty".into())
        );
    }

    #[test]
    fn attributes_give_kind_size_and_utc_time() {
        let mut attributes = FileAttributes::empty();
        attributes.size = Some(42);
        attributes.mtime = Some(1_700_000_000);
        attributes.permissions = Some(0o100644);
        assert_eq!(
            entry("a.txt".into(), &attributes),
            Entry {
                name: "a.txt".into(),
                kind: Kind::File,
                size: Some(42),
                modified: Some("2023-11-14T22:13:20Z".into()),
            }
        );
        attributes.permissions = Some(0o040755);
        let folder = entry("dir".into(), &attributes);
        assert_eq!((folder.kind, folder.size), (Kind::Dir, None));
        attributes.permissions = Some(0o120777);
        assert_eq!(entry("link".into(), &attributes).kind, Kind::Link);
    }
}
