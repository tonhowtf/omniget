use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use tokio::sync::mpsc;

use omniget_core::models::progress::ProgressUpdate;

use crate::commands::common;
use crate::output;
use crate::reporter;

const PROGRESS_STYLE: &str =
    "{spinner:.cyan} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos:>3}% {msg}";
const SPINNER_STYLE: &str = "{spinner:.cyan} {msg:.dim}";

pub async fn execute(
    url: String,
    quality: Option<u32>,
    output_dir: Option<String>,
    audio_only: bool,
    subs: Option<String>,
    format: Option<String>,
    proxy: Option<String>,
) -> Result<()> {
    common::init_cli_runtime(proxy.as_deref())?;

    let json_mode = output::is_json_mode();
    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(SPINNER_STYLE)
            .unwrap(),
    );
    pb.set_message("Resolving media info...");

    let output_path = match output_dir {
        Some(dir) => PathBuf::from(dir),
        None => reporter::default_output_dir(),
    };
    tokio::fs::create_dir_all(&output_path).await.ok();

    let registry = common::core_platform_registry();
    let (platform, info) = common::resolve_media_info(&registry, &url).await?;

    if !json_mode {
        pb.set_message(format!(
            "Starting {} download: {}",
            platform.name(),
            info.title
        ));
    }

    let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);

    let pb_clone = pb.clone();
    let progress_task = tokio::spawn(async move {
        while let Some(update) = rx.recv().await {
            if json_mode {
                println!(
                    r#"{{"type":"progress","percent":{},"downloaded_bytes":{:?},"total_bytes":{:?},"speed_bps":{:?},"eta_seconds":{:?}}}"#,
                    update.percent,
                    update.downloaded_bytes,
                    update.total_bytes,
                    update.speed_bps,
                    update.eta_seconds,
                );
            } else if update.percent >= 0.0 {
                pb_clone.set_position(update.percent.clamp(0.0, 100.0).round() as u64);
                pb_clone.set_style(
                    ProgressStyle::default_bar()
                        .template(PROGRESS_STYLE)
                        .unwrap(),
                );
                if let (Some(downloaded), Some(total)) =
                    (update.downloaded_bytes, update.total_bytes)
                {
                    pb_clone.set_message(format!("{} / {} bytes", downloaded, total));
                }
            } else if !json_mode {
                let phase = match update.percent as i32 {
                    -2 => "Connecting...",
                    -1 => "Starting...",
                    _ => "Preparing...",
                };
                pb_clone.set_message(phase);
            }
        }
    });

    let ytdlp_path = reporter::find_yt_dlp().await.ok();
    let opts = common::download_options(output_path, quality, audio_only, subs, format, ytdlp_path);
    let result = platform.download(&info, &opts, tx.clone()).await;

    drop(tx);
    let _ = progress_task.await;
    pb.finish();

    match result {
        Ok(dl_result) => {
            if output::is_json_mode() {
                let json = serde_json::json!({
                    "type": "complete",
                    "success": true,
                    "platform": platform.name(),
                    "file_path": dl_result.file_path,
                    "size": dl_result.file_size_bytes,
                    "duration": dl_result.duration_seconds,
                });
                println!("{}", json);
            } else {
                println!("✓ Downloaded to: {}", dl_result.file_path.display());
            }
            Ok(())
        }
        Err(e) => {
            if output::is_json_mode() {
                let json = serde_json::json!({
                    "type": "complete",
                    "success": false,
                    "platform": platform.name(),
                    "error": e.to_string(),
                });
                println!("{}", json);
            } else {
                eprintln!("✗ Failed: {}", e);
            }
            Err(e)
        }
    }
}
