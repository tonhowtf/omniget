use std::sync::Arc;

use omniget_core::core::registry::PlatformRegistry;
use omniget_core::platforms::{direct_file, Platform, PlatformDownloader};

pub struct Resolved {
    pub downloader: Arc<dyn PlatformDownloader>,
    pub platform_name: String,
}

pub async fn resolve_downloader(registry: &PlatformRegistry, url: &str) -> Option<Resolved> {
    if omniget_core::core::platform_optout::is_opted_out(url) {
        return None;
    }
    let platform = Platform::from_url(url);
    if platform.is_none() && direct_file::looks_like_direct_file(url).await {
        if let Some(downloader) = registry.find_by_name("direct_file") {
            return Some(Resolved {
                downloader,
                platform_name: "direct_file".to_string(),
            });
        }
    }
    let downloader = registry.find_platform(url)?;
    let platform_name = platform_label(platform.map(|p| p.to_string()), downloader.name());
    Some(Resolved {
        downloader,
        platform_name,
    })
}

// Gallery hosts are not in the Platform enum. Labelling them "generic" sent
// them through the yt-dlp info fetch, which fails before gallery-dl runs.
fn platform_label(platform: Option<String>, downloader: &str) -> String {
    platform.unwrap_or_else(|| match downloader {
        "gallery" => "gallery".to_string(),
        _ => "generic".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::platform_label;

    #[test]
    fn gallery_hosts_keep_the_gallery_label() {
        assert_eq!(platform_label(None, "gallery"), "gallery");
        assert_eq!(platform_label(None, "generic"), "generic");
        assert_eq!(platform_label(Some("youtube".into()), "youtube"), "youtube");
    }
}
