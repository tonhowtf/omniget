use anyhow::Result;

use crate::commands::common;
use crate::output;

pub async fn execute(url: String, proxy: Option<String>) -> Result<()> {
    common::init_cli_runtime(proxy.as_deref())?;

    let registry = common::core_platform_registry();
    let (platform, info) = common::resolve_media_info(&registry, &url).await?;

    if output::is_json_mode() {
        let json = serde_json::json!({
            "url": url,
            "platform": platform.name(),
            "title": info.title,
            "duration": info.duration_seconds.unwrap_or(0.0),
            "uploader": info.author,
            "media_type": info.media_type,
            "format_count": info.available_qualities.len(),
            "thumbnail_url": info.thumbnail_url,
            "file_size_bytes": info.file_size_bytes,
        });
        println!("{}", json);
    } else {
        println!("Platform: {}", platform.name());
        println!("Title: {}", info.title);
        println!("Duration: {:.0}s", info.duration_seconds.unwrap_or(0.0));
        println!("Uploader: {}", info.author);
        println!("Type: {:?}", info.media_type);
        println!("Formats: {}", info.available_qualities.len());

        for quality in info.available_qualities.iter().take(15) {
            let resolution = if quality.width > 0 && quality.height > 0 {
                format!("{}x{}", quality.width, quality.height)
            } else if quality.height > 0 {
                format!("{}p", quality.height)
            } else {
                "?".to_string()
            };
            println!(
                "  {:<14} {:<8} {:>8} {}",
                quality.label, quality.format, resolution, quality.url
            );
        }
    }

    Ok(())
}
