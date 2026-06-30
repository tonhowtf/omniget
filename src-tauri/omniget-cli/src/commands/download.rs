use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use omniget_core::core::ytdlp;
use omniget_core::models::progress::ProgressUpdate;

use crate::output;
use crate::reporter;

const PROGRESS_STYLE: &str = "{spinner:.cyan} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, ETA {eta})";
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
    let json_mode = output::is_json_mode();
    let pb = ProgressBar::new(100);
    pb.set_style(ProgressStyle::default_bar().template(SPINNER_STYLE).unwrap());
    pb.set_message("Searching for yt-dlp...");

    let ytdlp = reporter::find_yt_dlp().await?;
    pb.set_message("yt-dlp found");

    let output_path = match output_dir {
        Some(dir) => PathBuf::from(dir),
        None => reporter::default_output_dir(),
    };
    tokio::fs::create_dir_all(&output_path).await.ok();

    let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);

    let pb_clone = pb.clone();
    let progress_task = tokio::spawn(async move {
        while let Some(update) = rx.recv().await {
            if json_mode {
                println!(
                    r#"{{"type":"progress","percent":{},"downloaded_bytes":{:?},"total_bytes":{:?}}}"#,
                    update.percent, update.downloaded_bytes, update.total_bytes
                );
            } else if update.percent >= 0.0 {
                pb_clone.set_position((update.percent * 100.0) as u64);
                pb_clone.set_style(
                    ProgressStyle::default_bar()
                        .template(PROGRESS_STYLE)
                        .unwrap(),
                );
            }
        }
    });

    let download_mode = if audio_only { Some("audio") } else { None };
    let msg = format!("Starting: {}", url);
    pb.set_message(msg);

    // Build extra flags for proxy and subtitles
    let mut extra_flags: Vec<String> = Vec::new();
    if let Some(p) = &proxy {
        extra_flags.push("--proxy".to_string());
        extra_flags.push(p.clone());
    }

    let download_subtitles = subs.is_some();
    if let Some(lang) = &subs {
        extra_flags.push("--write-subs".to_string());
        extra_flags.push("--sub-langs".to_string());
        extra_flags.push(lang.clone());
    }

    let result = ytdlp::download_video(
        &ytdlp,
        &url,
        &output_path,
        quality,
        tx.clone(),
        download_mode,
        format.as_deref(),
        None,
        None,
        CancellationToken::new(),
        reporter::default_cookie_path().as_deref(),
        4,
        download_subtitles,
        &extra_flags,
        None,
    )
    .await;

    drop(tx);
    let _ = progress_task.await;
    pb.finish();

    match result {
        Ok(dl_result) => {
            if json_mode {
                println!(
                    r#"{{"type":"complete","success":true,"file_path":"{}","size":{},"duration":{}}}"#,
                    dl_result.file_path.display(),
                    dl_result.file_size_bytes,
                    dl_result.duration_seconds
                );
            } else {
                println!("✓ Downloaded to: {}", dl_result.file_path.display());
            }
            Ok(())
        }
        Err(e) => {
            if json_mode {
                println!(r#"{{"type":"complete","success":false,"error":"{}"}}"#, e);
            } else {
                eprintln!("✗ Failed: {}", e);
            }
            Err(e)
        }
    }
}
