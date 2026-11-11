use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use libunftp::ServerBuilder;
use unftp_core::auth::{AuthenticationError, Authenticator, Credentials, Principal};
use unftp_sbe_fs::Filesystem;

use super::listing::Kind;
use super::{known_hosts, ops, Auth, Connection, Protocol, TransferError};

#[derive(Debug)]
struct OnePassword;

#[async_trait]
impl Authenticator for OnePassword {
    async fn authenticate(
        &self,
        username: &str,
        creds: &Credentials,
    ) -> Result<Principal, AuthenticationError> {
        if username == "me" && creds.password.as_deref() == Some("right") {
            Ok(Principal {
                username: username.to_string(),
            })
        } else {
            Err(AuthenticationError::BadPassword)
        }
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn until_listening(port: u16) {
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("nothing is listening on {port}");
}

async fn ftp_server(root: &Path, tls: Option<(&Path, &Path)>, password: bool) -> u16 {
    let port = free_port();
    let home = root.to_path_buf();
    let mut builder = ServerBuilder::new(Box::new(move || Filesystem::new(home.clone()).unwrap()));
    if password {
        builder = builder.authenticator(Arc::new(OnePassword));
    }
    if let Some((cert, key)) = tls {
        builder = builder.ftps(cert, key);
    }
    let server = builder.build().unwrap();
    tokio::spawn(server.listen(format!("127.0.0.1:{port}")));
    until_listening(port).await;
    port
}

fn connection(protocol: Protocol, port: u16, state: &Path) -> Connection {
    Connection {
        protocol,
        host: "127.0.0.1".into(),
        port,
        user: String::new(),
        auth: Auth::Anonymous,
        saved: false,
        known_hosts: state.join("known_hosts"),
    }
}

fn quiet(_: Option<u64>) {}

async fn exercise(c: &Connection, base: &str, local: &Path) {
    let at = |name: &str| format!("{}/{name}", base.trim_end_matches('/'));

    let empty = ops::list(c, Some(base), None).await.unwrap();
    assert_eq!(empty.total, 0);
    assert_eq!(empty.reason, Some("empty_directory"));
    assert!(empty.untrusted);

    let made = ops::mkdir(c, &at("saves")).await.unwrap();
    assert!(made.done);
    assert_eq!(
        ops::mkdir(c, &at("saves")).await,
        Err(TransferError::AlreadyExists(at("saves")))
    );

    let payload: Vec<u8> = (0..300_000u32).map(|n| (n % 251) as u8).collect();
    let source = local.join("zelda.bin");
    std::fs::write(&source, &payload).unwrap();
    let seen = std::sync::Mutex::new(0u64);
    let sent = ops::put(
        c,
        &source,
        &format!("{}/", at("saves")),
        false,
        &quiet,
        &|bytes| *seen.lock().unwrap() = bytes,
    )
    .await
    .unwrap();
    assert_eq!(sent.remote, at("saves/zelda.bin"));
    assert_eq!(sent.bytes, payload.len() as u64);
    assert_eq!(*seen.lock().unwrap(), payload.len() as u64);

    assert_eq!(
        ops::put(c, &source, &at("saves/zelda.bin"), false, &quiet, &|_| {}).await,
        Err(TransferError::AlreadyExists(at("saves/zelda.bin")))
    );
    std::fs::write(&source, b"short").unwrap();
    let replaced = ops::put(c, &source, &at("saves/zelda.bin"), true, &quiet, &|_| {})
        .await
        .unwrap();
    assert_eq!(replaced.bytes, 5);
    std::fs::write(&source, &payload).unwrap();
    ops::put(c, &source, &at("saves/zelda.bin"), true, &quiet, &|_| {})
        .await
        .unwrap();

    let listed = ops::list(c, Some(&at("saves")), None).await.unwrap();
    assert_eq!(listed.path, at("saves"));
    assert_eq!(listed.total, 1);
    assert_eq!(listed.entries[0].name, "zelda.bin");
    assert_eq!(listed.entries[0].kind, Kind::File);
    assert_eq!(listed.entries[0].size, Some(payload.len() as u64));
    assert!(listed.entries[0].modified.is_some());
    let parent = ops::list(c, Some(base), Some(1)).await.unwrap();
    assert_eq!(parent.entries[0].kind, Kind::Dir);

    let downloads = local.join("downloads");
    std::fs::create_dir(&downloads).unwrap();
    let total = std::sync::Mutex::new(None);
    let fetched = ops::get(
        c,
        &at("saves/zelda.bin"),
        &downloads,
        false,
        &|size| *total.lock().unwrap() = size,
        &|_| {},
    )
    .await
    .unwrap();
    assert_eq!(fetched.local, downloads.join("zelda.bin"));
    assert_eq!(std::fs::read(&fetched.local).unwrap(), payload);
    assert_eq!(*total.lock().unwrap(), Some(payload.len() as u64));
    assert!(!downloads.join("zelda.bin.part").exists());
    assert_eq!(
        ops::get(
            c,
            &at("saves/zelda.bin"),
            &downloads,
            false,
            &quiet,
            &|_| {}
        )
        .await,
        Err(TransferError::AlreadyExists(
            downloads.join("zelda.bin").display().to_string()
        ))
    );
    assert_eq!(
        ops::get(
            c,
            &at("saves/ghost.bin"),
            &downloads,
            false,
            &quiet,
            &|_| {}
        )
        .await,
        Err(TransferError::NotFound(at("saves/ghost.bin")))
    );
    assert!(!downloads.join("ghost.bin").exists());
    assert!(matches!(
        ops::get(
            c,
            &at("saves"),
            &local.join("folder.bin"),
            false,
            &quiet,
            &|_| {}
        )
        .await,
        Err(TransferError::NotAFile { .. })
    ));

    let renamed = ops::rename(c, &at("saves/zelda.bin"), "link.bin")
        .await
        .unwrap();
    assert_eq!(renamed.to.as_deref(), Some(at("saves/link.bin").as_str()));
    assert_eq!(
        ops::rename(c, &at("saves/zelda.bin"), "other.bin").await,
        Err(TransferError::NotFound(at("saves/zelda.bin")))
    );

    assert_eq!(
        ops::delete(c, &at("saves"), true).await,
        Err(TransferError::NotEmpty(at("saves")))
    );
    let rehearsal = ops::delete(c, &at("saves/link.bin"), false).await.unwrap();
    assert!(rehearsal.dry_run);
    assert!(!rehearsal.done);
    assert_eq!(
        rehearsal.would.as_deref(),
        Some(
            format!(
                "delete the file {} ({} bytes)",
                at("saves/link.bin"),
                payload.len()
            )
            .as_str()
        )
    );
    assert_eq!(
        ops::list(c, Some(&at("saves")), None).await.unwrap().total,
        1
    );
    let gone = ops::delete(c, &at("saves/link.bin"), true).await.unwrap();
    assert!(gone.done);
    assert!(!gone.dry_run);
    assert!(ops::delete(c, &at("saves"), true).await.unwrap().done);
    assert_eq!(
        ops::delete(c, &at("saves"), true).await,
        Err(TransferError::NotFound(at("saves")))
    );
    assert_eq!(
        ops::list(c, Some(&at("saves")), None).await,
        Err(TransferError::NotFound(at("saves")))
    );
    assert_eq!(ops::list(c, Some(base), None).await.unwrap().total, 0);
}

#[tokio::test]
async fn anonymous_ftp_does_the_whole_round_against_a_real_server() {
    let remote = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let port = ftp_server(remote.path(), None, false).await;
    let c = connection(Protocol::Ftp, port, local.path());
    exercise(&c, "/", local.path()).await;
    let home = ops::list(&c, None, None).await.unwrap();
    assert_eq!(home.path, "/");
    assert!(!c.known_hosts.exists());
}

#[tokio::test]
async fn ftp_with_a_password_logs_in_and_a_wrong_one_is_auth_failed() {
    let remote = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    std::fs::write(remote.path().join("hello.txt"), b"hi").unwrap();
    let port = ftp_server(remote.path(), None, true).await;
    let with = |password: &str, saved: bool| Connection {
        user: "me".into(),
        auth: Auth::Password(password.into()),
        saved,
        ..connection(Protocol::Ftp, port, local.path())
    };
    let listed = ops::list(&with("right", true), Some("/"), None)
        .await
        .unwrap();
    assert_eq!(listed.entries[0].name, "hello.txt");
    assert_eq!(
        ops::list(&with("wrong", true), Some("/"), None).await,
        Err(TransferError::AuthFailed {
            host: format!("127.0.0.1:{port}"),
            saved: true
        })
    );
    assert!(matches!(
        ops::list(
            &connection(Protocol::Ftp, port, local.path()),
            Some("/"),
            None
        )
        .await,
        Err(TransferError::AuthFailed { saved: false, .. })
    ));
}

#[tokio::test]
async fn a_closed_port_is_host_unreachable() {
    let local = tempfile::tempdir().unwrap();
    for protocol in [Protocol::Ftp, Protocol::Sftp] {
        let c = Connection {
            user: "me".into(),
            ..connection(protocol, free_port(), local.path())
        };
        assert!(matches!(
            ops::list(&c, Some("/"), None).await,
            Err(TransferError::HostUnreachable { .. })
        ));
    }
}

fn certificate(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let cert = dir.join(format!("{name}.crt"));
    let key = dir.join(format!("{name}.key"));
    std::fs::write(&cert, made.cert.pem()).unwrap();
    std::fs::write(&key, made.signing_key.serialize_pem()).unwrap();
    (cert, key)
}

#[tokio::test]
async fn ftps_encrypts_both_channels_and_pins_the_certificate_on_first_use() {
    let remote = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let (cert, key) = certificate(local.path(), "first");
    let port = ftp_server(remote.path(), Some((&cert, &key)), false).await;
    let c = connection(Protocol::Ftps, port, local.path());
    exercise(&c, "/", local.path()).await;
    let saved = known_hosts::parse(&std::fs::read_to_string(&c.known_hosts).unwrap());
    assert_eq!(saved.len(), 1);
    assert_eq!((saved[0].kind.as_str(), saved[0].port), ("tls", port));

    let other_remote = tempfile::tempdir().unwrap();
    std::fs::write(other_remote.path().join("bait.txt"), b"x").unwrap();
    let (cert, key) = certificate(local.path(), "second");
    let impostor = ftp_server(other_remote.path(), Some((&cert, &key)), false).await;
    let text = std::fs::read_to_string(&c.known_hosts)
        .unwrap()
        .replace(&format!(" {port} "), &format!(" {impostor} "));
    std::fs::write(&c.known_hosts, &text).unwrap();
    let hijacked = connection(Protocol::Ftps, impostor, local.path());
    match ops::list(&hijacked, Some("/"), None).await {
        Err(TransferError::HostKeyChanged { expected, got, .. }) => {
            assert_eq!(expected, saved[0].fingerprint);
            assert_ne!(got, expected);
        }
        other => panic!("expected HOST_KEY_CHANGED, got {other:?}"),
    }
    assert_eq!(std::fs::read_to_string(&c.known_hosts).unwrap(), text);
}

#[tokio::test]
async fn asking_a_plain_server_for_tls_is_a_tls_error() {
    let remote = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let port = ftp_server(remote.path(), None, false).await;
    assert!(matches!(
        ops::list(
            &connection(Protocol::Ftps, port, local.path()),
            Some("/"),
            None
        )
        .await,
        Err(TransferError::Tls(_))
    ));
}

struct Sshd(std::process::Child);

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn keygen(file: &Path, passphrase: &str) {
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", passphrase, "-f"])
        .arg(file)
        .status()
        .unwrap();
    assert!(status.success());
}

async fn sshd(dir: &Path, host_key: &str) -> (Sshd, u16) {
    let port = free_port();
    let config = dir.join(format!("sshd_config_{host_key}"));
    std::fs::write(
        &config,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {host}\nPidFile {pid}\nAuthorizedKeysFile {keys}\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\nSubsystem sftp internal-sftp\n",
            host = dir.join(host_key).display(),
            pid = dir.join(format!("{host_key}.pid")).display(),
            keys = dir.join("authorized_keys").display(),
        ),
    )
    .unwrap();
    let child = std::process::Command::new("/usr/sbin/sshd")
        .args(["-D", "-e", "-f"])
        .arg(&config)
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    until_listening(port).await;
    (Sshd(child), port)
}

#[tokio::test]
#[ignore = "needs /usr/sbin/sshd and ssh-keygen; starts a private sshd on a free port as the current user"]
async fn sftp_with_a_key_does_the_whole_round_against_a_real_sshd() {
    let state = tempfile::tempdir().unwrap();
    let dir = state.path().canonicalize().unwrap();
    keygen(&dir.join("host_one"), "");
    keygen(&dir.join("host_two"), "");
    keygen(&dir.join("id_plain"), "");
    keygen(&dir.join("id_locked"), "open sesame");
    keygen(&dir.join("id_stranger"), "");
    let allowed = [
        std::fs::read_to_string(dir.join("id_plain.pub")).unwrap(),
        std::fs::read_to_string(dir.join("id_locked.pub")).unwrap(),
    ]
    .concat();
    std::fs::write(dir.join("authorized_keys"), allowed).unwrap();
    let user = std::env::var("USER").unwrap();
    let (_server, port) = sshd(&dir, "host_one").await;
    let with = |port: u16, key: &str, passphrase: Option<&str>| Connection {
        user: user.clone(),
        auth: Auth::Key {
            path: dir.join(key),
            passphrase: passphrase.map(str::to_string),
        },
        ..connection(Protocol::Sftp, port, &dir)
    };

    let remote = dir.join("remote");
    std::fs::create_dir(&remote).unwrap();
    let local = dir.join("local");
    std::fs::create_dir(&local).unwrap();
    let c = with(port, "id_plain", None);
    exercise(&c, &remote.to_string_lossy(), &local).await;

    let home = ops::list(&c, None, Some(1)).await.unwrap();
    assert!(home.path.starts_with('/'));
    let saved = known_hosts::parse(&std::fs::read_to_string(&c.known_hosts).unwrap());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].kind, "ssh");

    let locked = with(port, "id_locked", Some("open sesame"));
    assert_eq!(
        ops::list(&locked, Some(&remote.to_string_lossy()), None)
            .await
            .unwrap()
            .total,
        0
    );
    assert!(matches!(
        ops::list(&with(port, "id_locked", Some("wrong")), None, None).await,
        Err(TransferError::KeyFile { .. })
    ));
    assert!(matches!(
        ops::list(&with(port, "missing_key", None), None, None).await,
        Err(TransferError::KeyFile { .. })
    ));
    assert!(matches!(
        ops::list(&with(port, "id_stranger", None), None, None).await,
        Err(TransferError::AuthFailed { .. })
    ));
    assert!(matches!(
        ops::list(
            &Connection {
                auth: Auth::Password("nope".into()),
                ..with(port, "id_plain", None)
            },
            None,
            None
        )
        .await,
        Err(TransferError::AuthFailed { .. })
    ));

    let (_impostor, other) = sshd(&dir, "host_two").await;
    let text = std::fs::read_to_string(&c.known_hosts)
        .unwrap()
        .replace(&format!(" {port} "), &format!(" {other} "));
    std::fs::write(&c.known_hosts, &text).unwrap();
    match ops::list(&with(other, "id_plain", None), None, None).await {
        Err(TransferError::HostKeyChanged { expected, got, .. }) => {
            assert_eq!(expected, saved[0].fingerprint);
            assert_ne!(got, expected);
        }
        other => panic!("expected HOST_KEY_CHANGED, got {other:?}"),
    }
    assert_eq!(std::fs::read_to_string(&c.known_hosts).unwrap(), text);
    assert_eq!(
        known_hosts::forget(&c.known_hosts, "127.0.0.1", other).unwrap(),
        1
    );
    assert!(ops::list(&with(other, "id_plain", None), None, Some(1))
        .await
        .is_ok());
}
