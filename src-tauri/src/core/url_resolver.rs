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
    let platform_name = platform
        .map(|p| p.to_string())
        .unwrap_or_else(|| "generic".to_string());
    Some(Resolved {
        downloader,
        platform_name,
    })
}
