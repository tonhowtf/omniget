use serde::Serialize;

use crate::core::ytdlp;

#[derive(Clone, Serialize)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub author: String,
    pub duration: Option<f64>,
    pub thumbnail_url: Option<String>,
    pub url: String,
    pub platform: String,
}

#[tauri::command]
pub async fn search_videos(
    query: String,
    platform: String,
    max_results: u32,
) -> Result<Vec<SearchResult>, String> {
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp indisponível: {}", e))?;

    let n = max_results.clamp(1, 20);

    let search_query = match platform.as_str() {
        "youtube" => format!("ytsearch{}:{}", n, query),
        "soundcloud" => format!("scsearch{}:{}", n, query),
        _ => format!("ytsearch{}:{}", n, query),
    };

    let output = crate::core::process::command(&ytdlp_path)
        .args([
            "--flat-playlist",
            "--dump-json",
            "--no-warnings",
            "--no-check-certificates",
            "--socket-timeout",
            "15",
            &search_query,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Falha ao executar yt-dlp: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Busca falhou: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = json
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if id.is_empty() {
            continue;
        }

        let title = json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string();

        let author = json
            .get("uploader")
            .or_else(|| json.get("channel"))
            .or_else(|| json.get("uploader_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let duration = json.get("duration").and_then(|v| v.as_f64());

        let thumbnail_url = json
            .get("thumbnails")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .and_then(|t| t.get("url"))
            .or_else(|| json.get("thumbnail"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let url = json
            .get("url")
            .or_else(|| json.get("webpage_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| match platform.as_str() {
                "youtube" => format!("https://www.youtube.com/watch?v={}", id),
                _ => format!("https://www.youtube.com/watch?v={}", id),
            });

        results.push(SearchResult {
            id,
            title,
            author,
            duration,
            thumbnail_url,
            url,
            platform: platform.clone(),
        });
    }

    Ok(results)
}
