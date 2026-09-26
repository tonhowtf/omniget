//! Validates an immutable authorized snapshot through a confined pipe-only engine.
use omniget_core::core::{
    engine_files,
    secure_files::{Identity, Root},
};
use serde_json::Value;
use std::{path::Path, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Largest output an external job may produce, validate and transfer. It is
/// the artifact grant limit: the probe itself stays bounded by streaming the
/// frozen copy through a pipe (ffprobe allocation cap, output cap, timeout),
/// never by refusing media the transfer path would accept (bench D2).
pub const MAX_AUTHORIZED_MEDIA_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_PROBE_OUTPUT: u64 = 256 * 1024;
/// Base probe time plus streaming time for large frozen copies.
fn probe_timeout(bytes: u64) -> Duration {
    Duration::from_secs(15 + (bytes / (1024 * 1024 * 1024)) * 15)
}

/// Legacy local UI validation; external jobs MUST call probe_authorized.
pub async fn probe(path: &Path) -> Result<Value, String> {
    if !path.is_absolute() || !path.is_file() {
        return Err("OUTPUT_MISSING: expected a local file".into());
    }
    let mut command = crate::core::process::command("ffprobe");
    command.kill_on_drop(true).args([
        "-v", "error", "-protocol_whitelist", "file,pipe", "-show_entries",
        "format=duration,size,format_name:stream=codec_type,codec_name,width,height,duration:stream_disposition=attached_pic",
        "-of", "json", "-i",
    ]).arg(path);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .map_err(|_| "VALIDATION_TIMEOUT")?
        .map_err(|_| "DEPENDENCY_MISSING")?;
    if !output.status.success() || output.stdout.len() > 262144 {
        return Err("INTEGRITY_FAILED".into());
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| "INTEGRITY_FAILED")?;
    if value["streams"].as_array().is_none_or(|s| s.is_empty()) {
        return Err("INTEGRITY_FAILED".into());
    }
    Ok(value)
}

pub async fn probe_authorized(
    root: &Path,
    identity: &Identity,
    path: &Path,
) -> Result<Value, String> {
    probe_authorized_sized(root, identity, path).await?.1
}

/// Outer error: the output cannot be frozen (too large: never retryable;
/// denied). Inner result: the probe of the frozen bytes, whose size is known.
pub async fn probe_authorized_sized(
    root: &Path,
    identity: &Identity,
    path: &Path,
) -> Result<(u64, Result<Value, String>), String> {
    let snapshot = snapshot_authorized(root, identity, path, MAX_AUTHORIZED_MEDIA_BYTES).await?;
    let bytes = snapshot.bytes;
    Ok((bytes, probe_snapshot(snapshot).await))
}

async fn snapshot_authorized(
    root: &Path,
    identity: &Identity,
    path: &Path,
    limit: u64,
) -> Result<omniget_core::core::secure_files::Snapshot, String> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .map_err(|_| "OUTPUT_ACCESS_DENIED")?
    } else {
        path
    };
    let (root, identity, relative) = (root.to_path_buf(), identity.clone(), relative.to_path_buf());
    tokio::task::spawn_blocking(move || {
        let opened =
            Root::open(&root, Some(&identity)).map_err(|_| "OUTPUT_SNAPSHOT_DENIED".to_owned())?;
        // Advisory size read (no link follow) only to name the refusal; the
        // snapshot enforces the limit on the bytes it actually copies.
        if let Ok(meta) = std::fs::symlink_metadata(root.join(&relative)) {
            if meta.is_file() && meta.len() > limit {
                return Err(too_large(meta.len(), limit));
            }
        }
        opened.snapshot(&relative, limit).map_err(|_| {
            match std::fs::symlink_metadata(root.join(&relative)) {
                Ok(meta) if meta.is_file() && meta.len() > limit => too_large(meta.len(), limit),
                _ => "OUTPUT_SNAPSHOT_DENIED".to_owned(),
            }
        })
    })
    .await
    .map_err(|_| "OUTPUT_SNAPSHOT_FAILED".to_owned())?
}

fn too_large(bytes: u64, limit: u64) -> String {
    format!("OUTPUT_TOO_LARGE: output has {bytes} bytes; the artifact limit is {limit} bytes")
}

async fn probe_snapshot(
    snapshot: omniget_core::core::secure_files::Snapshot,
) -> Result<Value, String> {
    let timeout = probe_timeout(snapshot.bytes);
    let binary = crate::core::dependencies::find_tool("ffprobe")
        .await
        .ok_or("DEPENDENCY_MISSING")?;
    let reads = engine_files::dependency_files(&[binary.clone()]).await?;
    let mut command = engine_files::isolated_command(&binary, &reads)?;
    command.args([
        "-v", "error", "-max_alloc", "67108864", "-protocol_whitelist", "pipe",
        "-show_entries", "format=duration,size,format_name:stream=codec_type,codec_name,width,height,duration:stream_disposition=attached_pic",
        "-of", "json", "-i", "pipe:0",
    ]);
    let child = command.spawn().map_err(|_| "VALIDATION_START_FAILED")?;
    let mut process = ProbeProcess::new(child)?;
    let mut input = process.child.stdin.take().ok_or("VALIDATION_PIPE_FAILED")?;
    let output = process
        .child
        .stdout
        .take()
        .ok_or("VALIDATION_PIPE_FAILED")?;
    let errors = process
        .child
        .stderr
        .take()
        .ok_or("VALIDATION_PIPE_FAILED")?;
    let mut frozen = tokio::fs::File::from_std(snapshot.file);
    let result = tokio::time::timeout(timeout, async {
        let writer = async {
            let copied = tokio::io::copy(&mut frozen, &mut input).await;
            // A valid probe may finish after reading only the header.
            if let Err(e) = copied {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err("VALIDATION_INPUT_FAILED".to_owned());
                }
            }
            let _ = input.shutdown().await;
            drop(input);
            Ok::<_, String>(())
        };
        let reader = async {
            let mut bytes = Vec::new();
            output
                .take(MAX_PROBE_OUTPUT + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "VALIDATION_OUTPUT_FAILED")?;
            if bytes.len() as u64 > MAX_PROBE_OUTPUT {
                return Err("VALIDATION_OUTPUT_LIMIT".to_owned());
            }
            Ok::<_, String>(bytes)
        };
        let stderr = async {
            let mut bytes = Vec::new();
            errors
                .take(MAX_PROBE_OUTPUT + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "VALIDATION_OUTPUT_FAILED")?;
            if bytes.len() as u64 > MAX_PROBE_OUTPUT {
                return Err("VALIDATION_OUTPUT_LIMIT".to_owned());
            }
            Ok::<_, String>(())
        };
        let (_, bytes, _) = tokio::try_join!(writer, reader, stderr)?;
        let status = process
            .child
            .wait()
            .await
            .map_err(|_| "VALIDATION_WAIT_FAILED")?;
        if !status.success() {
            return Err("INTEGRITY_FAILED".to_owned());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "INTEGRITY_FAILED")?;
        if value["streams"].as_array().is_none_or(|s| s.is_empty()) {
            return Err("INTEGRITY_FAILED".to_owned());
        }
        Ok(value)
    })
    .await
    .map_err(|_| "VALIDATION_TIMEOUT".to_owned())
    .and_then(|v| v);
    process.terminate();
    if !matches!(
        tokio::time::timeout(Duration::from_secs(5), process.child.wait()).await,
        Ok(Ok(_))
    ) {
        return Err("VALIDATION_TERMINATION_UNCONFIRMED".to_owned());
    }
    result
}
struct ProbeProcess {
    child: tokio::process::Child,
    pid: i32,
}
impl ProbeProcess {
    fn new(child: tokio::process::Child) -> Result<Self, String> {
        let pid = child.id().ok_or("VALIDATION_PID_FAILED")? as i32;
        Ok(Self { child, pid })
    }
    fn terminate(&mut self) {
        #[cfg(unix)]
        {
            unsafe extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            if self.pid > 0 {
                unsafe {
                    kill(-self.pid, 9);
                }
            }
        }
        let _ = self.child.start_kill();
        self.pid = 0;
    }
}
impl Drop for ProbeProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub fn check_height(probe: &Value, ceiling: u64) -> Result<(), String> {
    let heights: Vec<_> = probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .filter_map(|s| s["height"].as_u64())
        .collect();
    // An audio-only source has no video stream to exceed the ceiling: audio
    // alone is within it. A file with neither is still refused.
    let has_video = probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1);
    let has_audio = probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| s["codec_type"] == "audio");
    if !has_video && has_audio {
        return Ok(());
    }
    if heights.is_empty() || heights.iter().any(|h| *h == 0 || *h > ceiling) {
        return Err(format!("FORMAT_UNAVAILABLE: requested format is not available within maxHeight={ceiling}; output does not meet the requested ceiling"));
    }
    Ok(())
}

pub fn check_audio(probe: &Value) -> Result<(), String> {
    let streams = probe["streams"]
        .as_array()
        .ok_or("OUTPUT_INVALID: no streams")?;
    if !streams.iter().any(|s| s["codec_type"] == "audio")
        || streams
            .iter()
            .any(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
    {
        return Err(
            "OUTPUT_INVALID: requested audio-only output contains video or no audio".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn vertical_video_is_measured_by_height_and_unknown_is_not_success() {
        assert!(check_height(
            &json!({"streams":[{"codec_type":"video","width":720,"height":1280}]}),
            720
        )
        .is_err());
        assert!(check_height(
            &json!({"streams":[{"codec_type":"video","width":1280,"height":720}]}),
            720
        )
        .is_ok());
        // Audio-only sources (SoundCloud) are within any video ceiling.
        assert!(check_height(&json!({"streams":[{"codec_type":"audio"}]}), 720).is_ok());
        assert!(check_height(&json!({"streams":[]}), 720).is_err());
        assert!(check_height(
            &json!({"streams":[{"codec_type":"video","height":1080},{"codec_type":"audio"}]}),
            720
        )
        .is_err());
    }
    #[test]
    fn transfer_limit_is_the_artifact_limit_and_probe_time_scales() {
        assert_eq!(MAX_AUTHORIZED_MEDIA_BYTES, 2 * 1024 * 1024 * 1024);
        assert!(MAX_AUTHORIZED_MEDIA_BYTES > 81_411_033); // bench D2 mkv
        assert_eq!(probe_timeout(81_411_033), Duration::from_secs(15));
        assert_eq!(
            probe_timeout(MAX_AUTHORIZED_MEDIA_BYTES),
            Duration::from_secs(45)
        );
    }
    #[tokio::test]
    async fn over_limit_output_is_named_too_large_and_never_retryable() {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let identity = Root::open(&dir, None).unwrap().identity().clone();
        std::fs::write(dir.join("big.mkv"), vec![0u8; 2048]).unwrap();
        let err = snapshot_authorized(&dir, &identity, &dir.join("big.mkv"), 1024)
            .await
            .err()
            .unwrap();
        assert!(err.starts_with("OUTPUT_TOO_LARGE"), "{err}");
        assert!(!crate::core::queue::external_retryable(&err));
        assert!(!crate::core::queue::is_retryable_error_message(&err));
        let snap = snapshot_authorized(&dir, &identity, &dir.join("big.mkv"), 4096)
            .await
            .ok()
            .unwrap();
        assert_eq!(snap.bytes, 2048);
        let err = snapshot_authorized(&dir, &identity, &dir.join("missing.mkv"), 4096)
            .await
            .err()
            .unwrap();
        assert_eq!(err, "OUTPUT_SNAPSHOT_DENIED");
        assert!(!crate::core::queue::external_retryable(&err));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn audio_requires_audio_and_allows_only_attached_cover_images() {
        assert!(check_audio(&json!({"streams":[{"codec_type":"audio"},{"codec_type":"video","disposition":{"attached_pic":1}}]})).is_ok());
        assert!(
            check_audio(&json!({"streams":[{"codec_type":"audio"},{"codec_type":"video"}]}))
                .is_err()
        );
        assert!(check_audio(&json!({"streams":[]})).is_err());
    }
}
