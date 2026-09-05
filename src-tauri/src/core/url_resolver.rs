//! Escolha do downloader para uma URL, com a sondagem de arquivo direto (B4).
//!
//! `Platform::from_url` e `registry.find_platform` decidem só pela URL. Um
//! `https://host/100MB.bin` de extensão que ninguém conhece caía no generic e
//! ficava minutos em `connecting` esperando o extractor do yt-dlp. Aqui, se a
//! URL não é de plataforma conhecida e tem extensão desconhecida, um `HEAD`
//! curto decide: não é página e tem tamanho de arquivo ⇒ `direct_file`, que
//! usa o `HttpFetcher` segmentado.

use std::sync::Arc;

use omniget_core::core::registry::PlatformRegistry;
use omniget_core::platforms::{direct_file, Platform, PlatformDownloader};

pub struct Resolved {
    pub downloader: Arc<dyn PlatformDownloader>,
    pub platform_name: String,
}

pub async fn resolve_downloader(registry: &PlatformRegistry, url: &str) -> Option<Resolved> {
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
