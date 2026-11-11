use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};

pub const EVENT: &str = "connect-tunnel-changed";
const VERSION: &str = "2026.10.7";

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    #[default]
    Off,
    Downloading,
    Starting,
    On,
    Error,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub state: State,
    pub url: Option<String>,
    pub error: Option<String>,
}

struct Inner {
    status: Status,
    child: Option<tokio::process::Child>,
}

fn inner() -> &'static Mutex<Inner> {
    static I: OnceLock<Mutex<Inner>> = OnceLock::new();
    I.get_or_init(|| {
        Mutex::new(Inner {
            status: Status::default(),
            child: None,
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, Inner> {
    inner().lock().unwrap_or_else(|e| e.into_inner())
}

pub fn status() -> Status {
    let mut g = lock();
    if let Some(child) = g.child.as_mut() {
        if let Ok(Some(code)) = child.try_wait() {
            g.child = None;
            g.status = Status {
                state: State::Error,
                url: None,
                error: Some(format!("tunnel_exited: {code}")),
            };
        }
    }
    g.status.clone()
}

fn set(app: &AppHandle, status: Status) {
    lock().status = status.clone();
    let _ = app.emit(EVENT, status);
}

fn asset() -> Option<(&'static str, &'static str, bool)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some((
            "cloudflared-darwin-arm64.tgz",
            "3586ccf397e98e5f437e60e7c307ca6ac463e110580503af8bb8e6f9ebf831bd",
            true,
        )),
        ("macos", "x86_64") => Some((
            "cloudflared-darwin-amd64.tgz",
            "02f3edfd87684dd5b705607688e4eb4b323f6e777b81e7b3a85af556aa356848",
            true,
        )),
        ("linux", "x86_64") => Some((
            "cloudflared-linux-amd64",
            "1f99fb518b83dec2ba8d870a7cc991d82a77e6196295a3d2754ba17ab9f91872",
            false,
        )),
        ("linux", "aarch64") => Some((
            "cloudflared-linux-arm64",
            "3df43aedc9ff291d1735b5cb576cb97e0b157488796db2e32b28aecabb445775",
            false,
        )),
        ("windows", "x86_64") => Some((
            "cloudflared-windows-amd64.exe",
            "2493dbc168923bb3e59e5e7716f71deced92992eea9acd6e9d74f84ac7751343",
            false,
        )),
        _ => None,
    }
}

fn binary_path() -> PathBuf {
    let name = if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    };
    omniget_core::core::paths::app_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("bin")
        .join(format!("cloudflared-{VERSION}"))
        .join(name)
}

fn on_path() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()])
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

async fn ensure_binary(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(p) = on_path() {
        return Ok(p);
    }
    let target = binary_path();
    if target.is_file() {
        return Ok(target);
    }
    let (name, sha, tgz) = asset().ok_or("TUNNEL_UNSUPPORTED_PLATFORM")?;
    set(
        app,
        Status {
            state: State::Downloading,
            url: None,
            error: None,
        },
    );
    let url =
        format!("https://github.com/cloudflare/cloudflared/releases/download/{VERSION}/{name}");
    let bytes = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("tunnel_download_failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("tunnel_download_failed: {e}"))?;
    if hex::encode(Sha256::digest(&bytes)) != sha {
        return Err("tunnel_download_corrupt".into());
    }
    let dir = target.parent().ok_or("TUNNEL_PATH")?.to_path_buf();
    let target_clone = target.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let tmp = target_clone.with_extension("part");
        if tgz {
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
            let mut found = false;
            for entry in archive.entries().map_err(|e| e.to_string())? {
                let mut entry = entry.map_err(|e| e.to_string())?;
                let is_bin = entry
                    .path()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n == "cloudflared"))
                    .unwrap_or(false);
                if is_bin {
                    let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
                    std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
                    found = true;
                    break;
                }
            }
            if !found {
                return Err("tunnel_download_corrupt".into());
            }
        } else {
            std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &target_clone).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(target)
}

pub fn parse_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '|' || c == '"')
        .unwrap_or(rest.len());
    let url = &rest[..end];
    let host = url.strip_prefix("https://")?;
    (host.ends_with(".trycloudflare.com")
        && host != "api.trycloudflare.com"
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'))
    .then(|| url.to_string())
}

pub async fn start(app: &AppHandle) -> Result<Status, String> {
    let current = status();
    if matches!(
        current.state,
        State::On | State::Starting | State::Downloading
    ) {
        return Ok(current);
    }
    let result = start_inner(app).await;
    if let Err(e) = &result {
        stop_child();
        set(
            app,
            Status {
                state: State::Error,
                url: None,
                error: Some(e.clone()),
            },
        );
    }
    result
}

async fn start_inner(app: &AppHandle) -> Result<Status, String> {
    let port = super::public::ensure_listener(app).await?;
    let bin = ensure_binary(app).await?;
    set(
        app,
        Status {
            state: State::Starting,
            url: None,
            error: None,
        },
    );
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args([
        "tunnel",
        "--no-autoupdate",
        "--url",
        &format!("http://127.0.0.1:{port}"),
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("tunnel_start_failed: {e}"))?;
    let stderr = child.stderr.take().ok_or("tunnel_start_failed")?;
    lock().child = Some(child);
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let mut tx = Some(tx);
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = parse_url(&line) {
                if let Some(tx) = tx.take() {
                    let _ = tx.send(url);
                }
            }
        }
    });
    let url = tokio::time::timeout(Duration::from_secs(45), rx)
        .await
        .map_err(|_| "tunnel_timeout".to_string())?
        .map_err(|_| "tunnel_exited".to_string())?;
    let status = Status {
        state: State::On,
        url: Some(url),
        error: None,
    };
    set(app, status.clone());
    Ok(status)
}

fn stop_child() {
    if let Some(mut child) = lock().child.take() {
        let _ = child.start_kill();
    }
}

pub fn stop(app: &AppHandle) -> Status {
    stop_child();
    let status = Status::default();
    set(app, status.clone());
    status
}

pub fn shutdown() {
    stop_child();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_quick_tunnel_url() {
        let line = "2026-11-10T10:00:00Z INF |  https://brave-lake-1234.trycloudflare.com                    |";
        assert_eq!(
            parse_url(line).as_deref(),
            Some("https://brave-lake-1234.trycloudflare.com")
        );
        assert!(
            parse_url("INF Requesting new quick Tunnel on https://api.trycloudflare.com...")
                .is_none()
        );
        assert!(parse_url("see https://evil.example.com/trycloudflare.com").is_none());
        assert!(parse_url("nothing here").is_none());
    }

    #[test]
    fn every_platform_has_a_pinned_digest() {
        if let Some((name, sha, _)) = asset() {
            assert!(name.starts_with("cloudflared-"));
            assert_eq!(sha.len(), 64);
        }
    }
}
