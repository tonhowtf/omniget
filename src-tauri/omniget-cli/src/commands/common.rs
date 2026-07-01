use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use omniget_core::core::http_client;
use omniget_core::core::registry::PlatformRegistry;
use omniget_core::core::ytdlp;
use omniget_core::models::media::{DownloadOptions, MediaInfo};
use omniget_core::models::settings::ProxySettings;
use omniget_core::platforms::{
    BilibiliDownloader, BlueskyDownloader, DirectFileDownloader, DouyinDownloader,
    GenericYtdlpDownloader, InstagramDownloader, P2pDownloader, PinterestDownloader,
    PlatformDownloader, RedditDownloader,
    TikTokDownloader, TwitchClipsDownloader, TwitterDownloader, VimeoDownloader, YouTubeDownloader,
};
use tokio_util::sync::CancellationToken;

use crate::reporter;

pub fn init_cli_runtime(proxy: Option<&str>) -> Result<()> {
    init_cookie_provider();

    if let Some(proxy_url) = proxy {
        http_client::init_proxy(parse_proxy(proxy_url)?);
    } else {
        http_client::init_proxy(ProxySettings::default());
    }

    Ok(())
}

fn init_cookie_provider() {
    ytdlp::set_global_cookie_file_fn(|| {
        reporter::default_cookie_path().map(|path| path.to_string_lossy().to_string())
    });
}

fn parse_proxy(raw: &str) -> Result<ProxySettings> {
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| anyhow!("Proxy must include a scheme, e.g. http://127.0.0.1:7897"))?;

    let proxy_type = match scheme {
        "http" | "https" | "socks5" => scheme.to_string(),
        other => return Err(anyhow!("Unsupported proxy scheme: {}", other)),
    };

    let authority = rest.split('/').next().unwrap_or(rest);
    let (auth, host_port) = match authority.rsplit_once('@') {
        Some((auth, host_port)) => (Some(auth), host_port),
        None => (None, authority),
    };

    let (host, port) = host_port
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("Proxy must include host and port, e.g. http://127.0.0.1:7897"))?;

    if host.is_empty() {
        return Err(anyhow!("Proxy host cannot be empty"));
    }

    let port = port
        .parse::<u16>()
        .with_context(|| format!("Invalid proxy port: {}", port))?;

    let (username, password) = match auth.and_then(|a| a.split_once(':')) {
        Some((u, p)) => (u.to_string(), p.to_string()),
        None => (String::new(), String::new()),
    };

    Ok(ProxySettings {
        enabled: true,
        proxy_type,
        host: host.to_string(),
        port,
        username,
        password,
    })
}

pub fn core_platform_registry() -> PlatformRegistry {
    let mut registry = PlatformRegistry::new();
    registry.register(Arc::new(InstagramDownloader::new()));
    registry.register(Arc::new(PinterestDownloader::new()));
    registry.register(Arc::new(TikTokDownloader::new()));
    registry.register(Arc::new(TwitchClipsDownloader::new()));
    registry.register(Arc::new(TwitterDownloader::new()));
    registry.register(Arc::new(BlueskyDownloader::new()));
    registry.register(Arc::new(RedditDownloader::new()));
    registry.register(Arc::new(YouTubeDownloader::new()));
    registry.register(Arc::new(VimeoDownloader::new()));
    registry.register(Arc::new(BilibiliDownloader::new()));
    registry.register(Arc::new(DouyinDownloader::new()));
    registry.register(Arc::new(P2pDownloader::new()));
    registry.register(Arc::new(DirectFileDownloader::new()));
    registry.register(Arc::new(GenericYtdlpDownloader::new()));
    registry
}

pub async fn resolve_media_info(
    registry: &PlatformRegistry,
    url: &str,
) -> Result<(Arc<dyn PlatformDownloader>, MediaInfo)> {
    let platform = registry
        .find_platform(url)
        .unwrap_or_else(|| Arc::new(GenericYtdlpDownloader::new()));

    match platform.get_media_info(url).await {
        Ok(info) => Ok((platform, info)),
        Err(primary_error)
            if platform.name() != "generic" && GenericYtdlpDownloader::new().can_handle(url) =>
        {
            let generic: Arc<dyn PlatformDownloader> = Arc::new(GenericYtdlpDownloader::new());
            match generic.get_media_info(url).await {
                Ok(info) => Ok((generic, info)),
                Err(fallback_error) => Err(fallback_error).with_context(|| {
                    format!(
                        "{} extractor failed first: {}; generic fallback also failed",
                        platform.name(),
                        primary_error
                    )
                }),
            }
        }
        Err(error) => Err(error),
    }
}

pub fn download_options(
    output_dir: PathBuf,
    quality: Option<u32>,
    audio_only: bool,
    subs: Option<String>,
    format: Option<String>,
    ytdlp_path: Option<PathBuf>,
) -> DownloadOptions {
    let mut custom_ytdlp_args = Vec::new();
    let download_subtitles = subs.is_some();
    if let Some(lang) = subs {
        custom_ytdlp_args.push("--write-subs".to_string());
        custom_ytdlp_args.push("--sub-langs".to_string());
        custom_ytdlp_args.push(lang);
    }

    DownloadOptions {
        quality: quality.map(|q| q.to_string()),
        output_dir,
        filename_template: None,
        download_subtitles,
        include_auto_subtitles: false,
        download_mode: audio_only.then(|| "audio".to_string()),
        audio_format: audio_only.then(|| format.clone()).flatten(),
        format_id: (!audio_only).then(|| format.clone()).flatten(),
        referer: None,
        extra_headers: None,
        page_url: None,
        user_agent: None,
        cancel_token: CancellationToken::new(),
        concurrent_fragments: 4,
        ytdlp_path,
        torrent_listen_port: None,
        torrent_id_slot: None,
        custom_ytdlp_args: (!custom_ytdlp_args.is_empty()).then_some(custom_ytdlp_args),
        torrent_files: None,
        torrent_auto_trackers: true,
        torrent_upnp: false,
    }
}
