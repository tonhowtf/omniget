use anyhow::Result;

use omniget_core::core::ytdlp;

use crate::output;
use crate::reporter::find_yt_dlp;

pub async fn execute(url: String, proxy: Option<String>) -> Result<()> {
    let ytdlp = find_yt_dlp().await?;
    
    let extra_flags: Vec<String> = match &proxy {
        Some(p) => vec!["--proxy".to_string(), p.clone()],
        None => vec![],
    };

    let info = match tokio::time::timeout(
        std::time::Duration::from_secs(120),
        ytdlp::get_video_info(&ytdlp, &url, &extra_flags)
    ).await {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => return Err(e),
        Err(_) => anyhow::bail!("Timed out after 120s"),
    };

    let formats = ytdlp::parse_formats(&info);

    if output::is_json_mode() {
        println!(
            r#"{{"url":"{}","title":"{}","duration":{},"uploader":"{}","format_count":{}}}"#,
            url.replace('"', "\\\""),
            info.get("title").and_then(|v| v.as_str()).unwrap_or("").replace('"', "\\\""),
            info.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0),
            info.get("uploader").and_then(|v| v.as_str()).unwrap_or("").replace('"', "\\\""),
            formats.len()
        );
    } else {
        println!(
            "Title: {}",
            info.get("title").and_then(|v| v.as_str()).unwrap_or("")
        );
        println!(
            "Duration: {:.0}s",
            info.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0)
        );
        println!("Uploader: {}", info.get("uploader").and_then(|v| v.as_str()).unwrap_or(""));
        println!("Formats: {}", formats.len());
        for f in formats.iter().take(15) {
            let res = f.resolution.as_deref().unwrap_or("?");
            println!(
                "  {:<10} {:<6} {:>5} {}",
                f.format_id,
                f.ext,
                res,
                f.format_note.as_deref().unwrap_or("")
            );
        }
    }

    Ok(())
}
