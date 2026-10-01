use std::path::Path;

const MAX_CLIPBOARD_FILE_SIZE: u64 = 1_073_741_824;

pub async fn copy_file_to_clipboard(path: &Path) -> anyhow::Result<()> {
    let metadata = std::fs::metadata(path)?;
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
    let path = path.to_string();
    let output = tokio::task::spawn_blocking(move || {
        crate::core::process::std_command("osascript")
            .args([
                "-e",
                &format!("set the clipboard to POSIX file \"{}\"", path),
            ])
            .output()
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {}", e))??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("osascript failed: {}", stderr));
    }

    tracing::info!("[clipboard] copied file to clipboard (macOS)");
    Ok(())
}

#[cfg(target_os = "linux")]
async fn copy_file_linux(path: &str) -> anyhow::Result<()> {
    let uri = format!("file://{}", path);

    let uri_clone = uri.clone();
    let xclip_result = tokio::task::spawn_blocking(move || {
        let mut child = match crate::core::process::std_command("xclip")
            .args(["-selection", "clipboard", "-target", "text/uri-list"])
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return Err(anyhow::anyhow!("xclip not found")),
        };
        if let Some(ref mut stdin) = child.stdin {
            use std::io::Write;
            let _ = stdin.write_all(uri_clone.as_bytes());
        }
        let output = child.wait_with_output()?;
        if output.status.success() {
            Ok(true)
        } else {
            Ok(false)
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    if let Ok(true) = xclip_result {
        tracing::info!("[clipboard] copied file to clipboard (xclip): {}", path);
        return Ok(());
    }

    let uri_clone = uri.clone();
    let xsel_result = tokio::task::spawn_blocking(move || {
        let mut child = match crate::core::process::std_command("xsel")
            .args(["--clipboard", "--input"])
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return Err(anyhow::anyhow!("xsel not found")),
        };
        if let Some(ref mut stdin) = child.stdin {
            use std::io::Write;
            let _ = stdin.write_all(uri_clone.as_bytes());
        }
        let output = child.wait_with_output()?;
        if output.status.success() {
            Ok(true)
        } else {
            Ok(false)
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    if let Ok(true) = xsel_result {
        tracing::info!("[clipboard] copied file URI to clipboard (xsel): {}", path);
        return Ok(());
    }

    let uri_clone = uri.clone();
    let wl_result = tokio::task::spawn_blocking(move || {
        let mut child = match crate::core::process::std_command("wl-copy")
            .args(["--type", "text/uri-list"])
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return Err(anyhow::anyhow!("wl-copy not found")),
        };
        if let Some(ref mut stdin) = child.stdin {
            use std::io::Write;
            let _ = stdin.write_all(uri_clone.as_bytes());
        }
        let output = child.wait_with_output()?;
        if output.status.success() {
            Ok(true)
        } else {
            Ok(false)
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;

    if let Ok(true) = wl_result {
        tracing::info!("[clipboard] copied file to clipboard (wl-copy): {}", path);
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "No clipboard tool found (tried xclip, xsel, wl-copy)"
    ))
}

#[cfg(target_os = "windows")]
async fn copy_file_windows(path: &str) -> anyhow::Result<()> {
    let ps_script = format!("Set-Clipboard -LiteralPath '{}'", path.replace('\'', "''"));

    let output = tokio::task::spawn_blocking(move || {
        crate::core::process::std_command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
            .output()
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {}", e))??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "PowerShell Set-Clipboard failed: {}",
            stderr
        ));
    }

    tracing::info!("[clipboard] copied file to clipboard (Windows): {}", path);
    Ok(())
}
