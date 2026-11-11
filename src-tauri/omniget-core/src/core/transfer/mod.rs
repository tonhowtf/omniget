pub mod ftp;
pub mod jobs;
pub mod known_hosts;
pub mod listing;
#[cfg(test)]
mod live_tests;
pub mod ops;
pub mod paths;
pub mod profiles;
pub mod service;
pub mod sftp;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const CONNECT_SECONDS: u64 = 15;
pub const STEP_SECONDS: u64 = 30;
pub const IDLE_SECONDS: u64 = 30;
const CHUNK_BYTES: usize = 64 * 1024;

pub type Progress<'a> = dyn Fn(u64) + Send + Sync + 'a;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TransferError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("no saved server is named \"{0}\"")]
    UnknownServer(String),
    #[error("{host} refused the user name, password or key")]
    AuthFailed { host: String, saved: bool },
    #[error("cannot reach {host}: {message}")]
    HostUnreachable { host: String, message: String },
    #[error("{host} now presents {got}, but {expected} was saved on first use")]
    HostKeyChanged {
        host: String,
        expected: String,
        got: String,
    },
    #[error("{0} does not exist on the server")]
    NotFound(String),
    #[error("the server denied access to {0}")]
    PermissionDenied(String),
    #[error("{what} got no answer after {seconds} s")]
    Timeout { what: String, seconds: u64 },
    #[error("{0} already exists and overwrite is off")]
    AlreadyExists(String),
    #[error("{0} is a directory that still has entries")]
    NotEmpty(String),
    #[error("{path} is a {kind}, not a file")]
    NotAFile { path: String, kind: &'static str },
    #[error("{path} ended with {got} bytes instead of {expected}")]
    Incomplete {
        path: String,
        expected: u64,
        got: u64,
    },
    #[error("the TLS handshake failed: {0}")]
    Tls(String),
    #[error("cannot use the key file {path}: {message}")]
    KeyFile { path: PathBuf, message: String },
    #[error("local file {path}: {message}")]
    Local { path: PathBuf, message: String },
    #[error("the server does not support {0}")]
    Unsupported(String),
    #[error("the server answered: {0}")]
    Server(String),
    #[error("{0} transfers are already running")]
    Busy(usize),
    #[error("no transfer has the id {0}")]
    UnknownTransfer(String),
    #[error("cannot save: {0}")]
    Storage(String),
}

impl TransferError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::UnknownServer(_) => "UNKNOWN_SERVER",
            Self::AuthFailed { .. } => "AUTH_FAILED",
            Self::HostUnreachable { .. } => "HOST_UNREACHABLE",
            Self::HostKeyChanged { .. } => "HOST_KEY_CHANGED",
            Self::NotFound(_) => "NOT_FOUND",
            Self::PermissionDenied(_) => "PERMISSION_DENIED",
            Self::Timeout { .. } => "TIMEOUT",
            Self::AlreadyExists(_) => "ALREADY_EXISTS",
            Self::NotEmpty(_) => "NOT_EMPTY",
            Self::NotAFile { .. } => "NOT_A_FILE",
            Self::Incomplete { .. } => "INCOMPLETE",
            Self::Tls(_) => "TLS_FAILED",
            Self::KeyFile { .. } => "KEY_FILE",
            Self::Local { .. } => "LOCAL_IO",
            Self::Unsupported(_) => "UNSUPPORTED",
            Self::Server(_) => "SERVER_ERROR",
            Self::Busy(_) => "BUSY",
            Self::UnknownTransfer(_) => "UNKNOWN_TRANSFER",
            Self::Storage(_) => "STORAGE_FAILED",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::HostUnreachable { .. }
                | Self::Timeout { .. }
                | Self::Busy(_)
                | Self::Incomplete { .. }
        )
    }

    pub fn next_step(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "fix the argument named in the message and call again",
            Self::UnknownServer(_) => {
                "call transfer_servers with op=list to see the saved names, or pass protocol and host instead of server"
            }
            Self::AuthFailed { saved: true, .. } => {
                "the saved credentials are wrong; ask the person to correct this server in the Transfer screen, then retry"
            }
            Self::AuthFailed { saved: false, .. } => {
                "tools never take a password; ask the person to save this server with its password in the Transfer screen, then pass server=<name>. For anonymous FTP leave user empty"
            }
            Self::HostUnreachable { .. } => {
                "check that the server is on, on the same network, and that host and port are right (FTP 21, SFTP 22, homebrew console servers often 5000), then retry"
            }
            Self::HostKeyChanged { .. } => {
                "nothing was sent. If the person confirms the server was reinstalled, call transfer_servers with op=forget_host for this host and port, then retry; otherwise stop, someone may be intercepting the connection"
            }
            Self::NotFound(_) => "call transfer_list on the parent folder to see the real names",
            Self::PermissionDenied(_) => {
                "this account cannot do that here; choose another folder or ask the person for an account with access"
            }
            Self::Timeout { .. } => "retry; if it repeats, the server or the network is too slow or went away",
            Self::AlreadyExists(_) => {
                "pass overwrite=true to replace it, or choose another name; nothing was written"
            }
            Self::NotEmpty(_) => {
                "delete only removes files and empty folders; list the folder and delete its entries first"
            }
            Self::NotAFile { .. } => "pass the path of a file; use transfer_list to browse folders",
            Self::Incomplete { .. } => {
                "the copy is not complete and must not be used; run the transfer again with overwrite=true"
            }
            Self::Tls(_) => {
                "the server may not support explicit TLS; try protocol=ftp only if the person accepts an unencrypted connection"
            }
            Self::KeyFile { .. } => {
                "pass keyPath to an OpenSSH private key this computer can read; a key with a passphrase must be saved by the person in the Transfer screen"
            }
            Self::Local { .. } => "check that the local path exists and can be read or written",
            Self::Unsupported(_) => "this server cannot do that operation",
            Self::Server(_) => "read the server message; retry once if it looks temporary",
            Self::Busy(_) => "wait for a running transfer to finish (call the tool with its transferId), then retry",
            Self::UnknownTransfer(_) => {
                "transfer ids are forgotten when the app restarts; start the transfer again"
            }
            Self::Storage(_) => "check that the app data folder can be written",
        }
    }

    pub fn wire(&self) -> String {
        let detail = json!({
            "message": self.to_string(),
            "retryable": self.retryable(),
            "next_step": self.next_step(),
        });
        format!("{}: {detail}", self.code())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ftp,
    Ftps,
    Sftp,
}

impl Protocol {
    pub const NAMES: &'static [&'static str] = &["ftp", "ftps", "sftp"];

    pub fn default_port(self) -> u16 {
        match self {
            Self::Ftp | Self::Ftps => 21,
            Self::Sftp => 22,
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum Auth {
    Anonymous,
    Password(String),
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anonymous => f.write_str("Anonymous"),
            Self::Password(_) => f.write_str("Password(***)"),
            Self::Key { path, .. } => write!(f, "Key({})", path.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    pub saved: bool,
    pub known_hosts: PathBuf,
}

impl Connection {
    pub fn place(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub(crate) fn unreachable(&self, message: impl ToString) -> TransferError {
        TransferError::HostUnreachable {
            host: self.place(),
            message: message.to_string(),
        }
    }

    pub(crate) fn auth_failed(&self) -> TransferError {
        TransferError::AuthFailed {
            host: self.place(),
            saved: self.saved,
        }
    }

    pub(crate) async fn trust(
        &self,
        kind: &'static str,
        fingerprint: String,
    ) -> Result<(), TransferError> {
        let (file, host, port) = (self.known_hosts.clone(), self.host.clone(), self.port);
        let seen = fingerprint.clone();
        let verdict = tokio::task::spawn_blocking(move || {
            known_hosts::verify(&file, &host, port, kind, &seen)
        })
        .await
        .map_err(|e| TransferError::Storage(e.to_string()))??;
        match verdict {
            known_hosts::Verdict::Known | known_hosts::Verdict::New => Ok(()),
            known_hosts::Verdict::Changed { expected } => Err(TransferError::HostKeyChanged {
                host: self.place(),
                expected,
                got: fingerprint,
            }),
        }
    }
}

pub(crate) async fn within<T>(
    seconds: u64,
    what: &str,
    work: impl Future<Output = Result<T, TransferError>>,
) -> Result<T, TransferError> {
    match tokio::time::timeout(Duration::from_secs(seconds), work).await {
        Ok(done) => done,
        Err(_) => Err(TransferError::Timeout {
            what: what.to_string(),
            seconds,
        }),
    }
}

#[derive(Debug)]
pub(crate) enum PumpError {
    Read(std::io::Error),
    Write(std::io::Error),
    Idle,
}

pub(crate) async fn pump<R, W>(
    reader: &mut R,
    writer: &mut W,
    idle: Duration,
    progress: &Progress<'_>,
) -> Result<u64, PumpError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut done = 0u64;
    loop {
        let read = tokio::time::timeout(idle, reader.read(&mut buffer))
            .await
            .map_err(|_| PumpError::Idle)?
            .map_err(PumpError::Read)?;
        if read == 0 {
            break;
        }
        tokio::time::timeout(idle, writer.write_all(&buffer[..read]))
            .await
            .map_err(|_| PumpError::Idle)?
            .map_err(PumpError::Write)?;
        done += read as u64;
        progress(done);
    }
    tokio::time::timeout(idle, writer.flush())
        .await
        .map_err(|_| PumpError::Idle)?
        .map_err(PumpError::Write)?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pumping_copies_everything_and_reports_progress() {
        let data = vec![7u8; CHUNK_BYTES * 2 + 5];
        let mut out = Vec::new();
        let seen = std::sync::Mutex::new(Vec::new());
        let copied = pump(
            &mut data.as_slice(),
            &mut out,
            Duration::from_secs(5),
            &|done| seen.lock().unwrap().push(done),
        )
        .await
        .unwrap();
        assert_eq!(copied, data.len() as u64);
        assert_eq!(out, data);
        assert_eq!(seen.lock().unwrap().last(), Some(&(data.len() as u64)));
    }

    #[tokio::test]
    async fn a_silent_peer_is_an_idle_timeout_and_not_a_hang() {
        let (mut silent, _keep_open) = tokio::io::duplex(8);
        let mut out = Vec::new();
        let result = pump(&mut silent, &mut out, Duration::from_millis(30), &|_| {}).await;
        assert!(matches!(result, Err(PumpError::Idle)));
    }

    #[tokio::test]
    async fn a_step_that_never_answers_is_a_timeout() {
        let error = within(
            0,
            "listing /",
            std::future::pending::<Result<(), TransferError>>(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            TransferError::Timeout {
                what: "listing /".into(),
                seconds: 0
            }
        );
    }

    #[test]
    fn wire_text_starts_with_the_code_and_carries_the_recovery_fields() {
        let error = TransferError::HostKeyChanged {
            host: "nas.local:22".into(),
            expected: "SHA256:old".into(),
            got: "SHA256:new".into(),
        };
        let text = error.wire();
        let (code, detail) = text.split_once(": ").unwrap();
        assert_eq!(code, "HOST_KEY_CHANGED");
        let detail: serde_json::Value = serde_json::from_str(detail).unwrap();
        assert_eq!(detail["retryable"], false);
        assert!(detail["message"].as_str().unwrap().contains("SHA256:new"));
        assert!(detail["next_step"]
            .as_str()
            .unwrap()
            .contains("forget_host"));
    }

    #[test]
    fn only_network_trouble_is_retryable() {
        let unreachable = TransferError::HostUnreachable {
            host: "x".into(),
            message: "refused".into(),
        };
        let timeout = TransferError::Timeout {
            what: "connect".into(),
            seconds: 15,
        };
        let auth = TransferError::AuthFailed {
            host: "x".into(),
            saved: false,
        };
        assert!(unreachable.retryable());
        assert!(timeout.retryable());
        assert!(!auth.retryable());
        assert!(!TransferError::NotFound("/a".into()).retryable());
    }

    #[test]
    fn a_failed_login_says_where_the_password_goes() {
        let inline = TransferError::AuthFailed {
            host: "x".into(),
            saved: false,
        };
        let saved = TransferError::AuthFailed {
            host: "x".into(),
            saved: true,
        };
        assert!(inline.next_step().contains("never take a password"));
        assert!(saved.next_step().contains("saved credentials"));
    }

    #[test]
    fn a_password_never_reaches_debug_output() {
        let auth = Auth::Password("hunter2".into());
        assert_eq!(format!("{auth:?}"), "Password(***)");
        let key = Auth::Key {
            path: "/k".into(),
            passphrase: Some("hunter2".into()),
        };
        assert!(!format!("{key:?}").contains("hunter2"));
    }

    #[test]
    fn default_ports_follow_the_protocol() {
        assert_eq!(Protocol::Ftp.default_port(), 21);
        assert_eq!(Protocol::Ftps.default_port(), 21);
        assert_eq!(Protocol::Sftp.default_port(), 22);
        assert_eq!(serde_json::to_value(Protocol::Sftp).unwrap(), json!("sftp"));
    }
}
