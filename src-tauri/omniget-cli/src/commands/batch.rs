use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::output;

pub async fn execute(file: String, max_concurrent: usize, output_dir: Option<String>, proxy: Option<String>) -> Result<()> {
    let content = tokio::fs::read_to_string(&file)
        .await
        .with_context(|| format!("Failed to read {}", file))?;

    let urls: Vec<String> = content
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect();

    if urls.is_empty() {
        println!("No URLs found in {}", file);
        return Ok(());
    }

    let total = urls.len();
    if !output::is_json_mode() {
        println!("Batch downloading {} URLs (max concurrent: {})", total, max_concurrent);
    }

    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut handles = Vec::new();

    for url in &urls {
        let permit = semaphore.clone().acquire_owned().await?;
        let dir = output_dir.clone();
        let px = proxy.clone();
        let url = url.clone();

        handles.push(tokio::spawn(async move {
            let _permit = permit;
            crate::commands::download::execute(url, None, dir, false, None, None, px).await
        }));
    }

    let mut success = 0;
    let mut failed = 0;

    for handle in handles {
        match handle.await {
            Ok(Ok(())) => success += 1,
            Ok(Err(e)) => {
                eprintln!("Error: {}", e);
                failed += 1;
            }
            Err(e) => {
                eprintln!("Task panicked: {}", e);
                failed += 1;
            }
        }
    }

    if output::is_json_mode() {
        println!(r#"{{"total":{},"success":{},"failed":{}}}"#, total, success, failed);
    } else {
        println!("Done: {} ok, {} failed", success, failed);
    }

    Ok(())
}
