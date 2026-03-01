use std::path::Path;

const MAX_CLIPBOARD_FILE_SIZE: u64 = 1_073_741_824; // 1 GB

pub async fn copy_file_to_clipboard(path: &Path) -> anyhow::Result<()> {
    let metadata = tokio::fs::metadata(path).await?;
    if metadata.len() > MAX_CLIPBOARD_FILE_SIZE {
        tracing::info!(
            "[clipboard] skipping copy: file too large ({} bytes)",
            metadata.len()
        );
        return Ok(());
    }

    let path_str = path.to_string_lossy().to_string();

    #[cfg(target_os = "macos")]
    {
        copy_file_macos(&path_str).await
    }

    #[cfg(target_os = "linux")]
    {
        copy_file_linux(&path_str).await
    }

    #[cfg(target_os = "windows")]
    {
        copy_file_windows(&path_str).await
    }
}

#[cfg(target_os = "macos")]
async fn copy_file_macos(path: &str) -> anyhow::Result<()> {
    let output = tokio::process::Command::new("osascript")
        .args([
            "-e",
            &format!("set the clipboard to POSIX file \"{}\"", path),
        ])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("osascript failed: {}", stderr));
    }

    tracing::info!("[clipboard] copied file to clipboard (macOS): {}", path);
    Ok(())
}

#[cfg(target_os = "linux")]
async fn copy_file_linux(path: &str) -> anyhow::Result<()> {
    let uri = format!("file://{}", path);

    let xclip = tokio::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-target", "text/uri-list"])
        .stdin(std::process::Stdio::piped())
        .spawn();

    match xclip {
        Ok(mut child) => {
            if let Some(ref mut stdin) = child.stdin {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(uri.as_bytes()).await?;
            }
            let output = child.wait_with_output().await?;
            if output.status.success() {
                tracing::info!("[clipboard] copied file to clipboard (xclip): {}", path);
                return Ok(());
            }
        }
        Err(_) => {
            tracing::debug!("[clipboard] xclip not found, trying xsel");
        }
    }

    let xsel = tokio::process::Command::new("xsel")
        .args(["--clipboard", "--input"])
        .stdin(std::process::Stdio::piped())
        .spawn();

    match xsel {
        Ok(mut child) => {
            if let Some(ref mut stdin) = child.stdin {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(uri.as_bytes()).await?;
            }
            let output = child.wait_with_output().await?;
            if output.status.success() {
                tracing::info!("[clipboard] copied file URI to clipboard (xsel): {}", path);
                return Ok(());
            }
        }
        Err(_) => {
            tracing::debug!("[clipboard] xsel not found, trying wl-copy");
        }
    }

    let wl_copy = tokio::process::Command::new("wl-copy")
        .args(["--type", "text/uri-list"])
        .stdin(std::process::Stdio::piped())
        .spawn();

    match wl_copy {
        Ok(mut child) => {
            if let Some(ref mut stdin) = child.stdin {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(uri.as_bytes()).await?;
            }
            let output = child.wait_with_output().await?;
            if output.status.success() {
                tracing::info!(
                    "[clipboard] copied file to clipboard (wl-copy): {}",
                    path
                );
                return Ok(());
            }
        }
        Err(_) => {}
    }

    Err(anyhow::anyhow!(
        "No clipboard tool found (tried xclip, xsel, wl-copy)"
    ))
}

#[cfg(target_os = "windows")]
async fn copy_file_windows(path: &str) -> anyhow::Result<()> {
    let ps_script = format!(
        "Set-Clipboard -Path '{}'",
        path.replace('\'', "''")
    );

    let output = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("PowerShell Set-Clipboard failed: {}", stderr));
    }

    tracing::info!("[clipboard] copied file to clipboard (Windows): {}", path);
    Ok(())
}
