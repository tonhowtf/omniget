use omniget_core::{
    core::{http_client, registry::PlatformRegistry, ytdlp},
    models::{
        media::{DownloadOptions, MediaInfo, MediaType},
        settings::ProxySettings,
    },
    platforms::*,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Inspect,
    Download,
    Collection,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum Mode {
    #[default]
    Video,
    Audio,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    operation: Operation,
    url: String,
    #[serde(default)]
    offset: u64,
    #[serde(default = "collection_limit")]
    limit: u64,
    proxy: String,
    runtime_dir: PathBuf,
    output_dir: Option<PathBuf>,
    platform: Option<String>,
    media_info: Option<MediaInfo>,
    quality: Option<u32>,
    #[serde(default)]
    download_mode: Mode,
    audio_format: Option<String>,
    format_id: Option<String>,
    referer: Option<String>,
    page_url: Option<String>,
    user_agent: Option<String>,
    extra_headers: Option<HashMap<String, String>>,
    #[serde(default)]
    subtitles: bool,
    #[serde(default)]
    include_auto_subtitles: bool,
    #[serde(default = "fragments")]
    concurrent_fragments: u32,
    ytdlp_path: Option<PathBuf>,
    ffmpeg_path: Option<PathBuf>,
    ffprobe_path: Option<PathBuf>,
    node_path: Option<PathBuf>,
    deno_path: Option<PathBuf>,
    #[serde(default)]
    info_json_dir: Option<PathBuf>,
    #[serde(default)]
    compatible_video: bool,
}
fn collection_limit() -> u64 {
    20
}
fn fragments() -> u32 {
    4
}
fn valid_url(raw: &str) -> bool {
    raw.len() <= 8192
        && reqwest::Url::parse(raw).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
        })
}
fn clean_path(p: &Path) -> bool {
    p.is_absolute()
        && !p
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}
fn validate(r: &Request) -> Result<u16, &'static str> {
    if r.offset > 10000 || !(1..=50).contains(&r.limit) {
        return Err("INVALID_PAGE");
    }
    if r.version != 1 || !valid_url(&r.url) || !clean_path(&r.runtime_dir) {
        return Err("INVALID_REQUEST");
    }
    let proxy = reqwest::Url::parse(&r.proxy).map_err(|_| "INVALID_PROXY")?;
    if proxy.scheme() != "http"
        || proxy.host_str() != Some("127.0.0.1")
        || proxy.username() != "omniget"
        || !proxy
            .password()
            .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        || proxy.path() != "/"
        || proxy.query().is_some()
        || proxy.fragment().is_some()
    {
        return Err("INVALID_PROXY");
    }
    let port = proxy.port().filter(|p| *p != 0).ok_or("INVALID_PROXY")?;
    if !(1..=8).contains(&r.concurrent_fragments)
        || r.quality.is_some_and(|q| !(1..=8640).contains(&q))
    {
        return Err("INVALID_OPTIONS");
    }
    if r.info_json_dir.as_ref().is_some_and(|p| !clean_path(p)) {
        return Err("INVALID_REQUEST");
    }
    if r.output_dir.as_ref().is_some_and(|p| !clean_path(p))
        || (matches!(r.operation, Operation::Download) && r.output_dir.is_none())
    {
        return Err("INVALID_OUTPUT");
    }
    for path in [
        &r.ytdlp_path,
        &r.ffmpeg_path,
        &r.ffprobe_path,
        &r.node_path,
        &r.deno_path,
    ]
    .into_iter()
    .flatten()
    {
        if !clean_path(path) || !path.is_file() {
            return Err("INVALID_BINARY");
        }
    }
    if r.audio_format.as_ref().is_some_and(|f| {
        !matches!(
            f.as_str(),
            "mp3" | "m4a" | "opus" | "wav" | "flac" | "aac" | "best"
        )
    }) {
        return Err("INVALID_OPTIONS");
    }
    if r.format_id
        .as_ref()
        .is_some_and(|f| !valid_format_selector(f))
    {
        return Err("INVALID_FORMAT");
    }
    for u in [&r.referer, &r.page_url].into_iter().flatten() {
        if !valid_url(u) {
            return Err("INVALID_OPTIONS");
        }
    }
    if r.user_agent
        .as_ref()
        .is_some_and(|v| v.len() > 512 || v.chars().any(char::is_control))
    {
        return Err("INVALID_OPTIONS");
    }
    if let Some(headers) = &r.extra_headers {
        if headers.len() > 16 {
            return Err("INVALID_HEADERS");
        }
        for (name, value) in headers {
            if name.is_empty()
                || name.len() > 64
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || value.len() > 4096
                || value.chars().any(char::is_control)
                || matches!(
                    name.to_ascii_lowercase().as_str(),
                    "host"
                        | "connection"
                        | "proxy-authorization"
                        | "proxy-connection"
                        | "transfer-encoding"
                        | "content-length"
                        | "upgrade"
                )
            {
                return Err("INVALID_HEADERS");
            }
        }
    }
    if let Some(info) = &r.media_info {
        validate_info(info)?;
        if r.platform.is_none() {
            return Err("PLATFORM_REQUIRED");
        }
    }
    Ok(port)
}
fn valid_format_selector(f: &str) -> bool {
    !f.is_empty()
        && f.len() <= 512
        && f.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_+-/*[].<>=!?,^".contains(&b))
}
fn validate_info(info: &MediaInfo) -> Result<(), &'static str> {
    if info.title.len() > 4096
        || info.author.len() > 2048
        || info.available_qualities.len() > 256
        || info
            .available_qualities
            .iter()
            .any(|q| !valid_url(&q.url) || q.label.len() > 128 || q.format.len() > 64)
        || info.thumbnail_url.as_ref().is_some_and(|u| !valid_url(u))
    {
        return Err("MEDIA_INFO_LIMIT");
    }
    Ok(())
}
fn emit(value: Value) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(&value).map_err(|_| "OUTPUT_ENCODING")?;
    if bytes.len() > 512 * 1024 {
        return Err("OUTPUT_LIMIT");
    }
    let mut out = std::io::stdout().lock();
    out.write_all(&bytes)
        .and_then(|_| out.write_all(b"\n"))
        .and_then(|_| out.flush())
        .map_err(|_| "OUTPUT_CLOSED")
}
fn registry() -> PlatformRegistry {
    let mut r = PlatformRegistry::new();
    r.register(Arc::new(InstagramDownloader::new()));
    r.register(Arc::new(ThreadsDownloader::new()));
    r.register(Arc::new(PinterestDownloader::new()));
    r.register(Arc::new(TikTokDownloader::new()));
    r.register(Arc::new(TwitchClipsDownloader::new()));
    r.register(Arc::new(TwitterDownloader::new()));
    r.register(Arc::new(BlueskyDownloader::new()));
    r.register(Arc::new(RedditDownloader::new()));
    r.register(Arc::new(YouTubeDownloader::new()));
    r.register(Arc::new(VimeoDownloader::new()));
    r.register(Arc::new(BilibiliDownloader::new()));
    r.register(Arc::new(DouyinDownloader::new()));
    r.register(Arc::new(DirectFileDownloader::new()));
    r.register(Arc::new(GenericYtdlpDownloader::new()));
    r
}
fn setup(r: &Request, port: u16) -> Result<(), &'static str> {
    let meta = std::fs::symlink_metadata(&r.runtime_dir).map_err(|_| "INVALID_RUNTIME")?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("INVALID_RUNTIME");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("RUNTIME_NOT_PRIVATE");
        }
    }
    if std::fs::read_dir(&r.runtime_dir)
        .map_err(|_| "INVALID_RUNTIME")?
        .next()
        .is_some()
    {
        return Err("RUNTIME_NOT_EMPTY");
    }
    for key in [
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
        "OMNIGET_POT_PROVIDER_URL",
        "PYTHONPATH",
        "PYTHONHOME",
        "OMNIGET_PYTHON",
    ] {
        std::env::remove_var(key)
    }
    std::env::set_var("OMNIGET_DATA_DIR", &r.runtime_dir);
    std::env::set_var("OMNIGET_SECRETS_DIR", r.runtime_dir.join("secrets"));
    std::env::set_var("XDG_CONFIG_HOME", &r.runtime_dir);
    match r.info_json_dir.as_ref().filter(|d| d.is_dir()) {
        Some(dir) => std::env::set_var("OMNIGET_INFO_JSON_DIR", dir),
        None => std::env::remove_var("OMNIGET_INFO_JSON_DIR"),
    }
    std::env::set_var("OMNIGET_WORKER_MODE", "1");
    for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
        std::env::set_var(key, &r.proxy)
    }
    if let Some(path) = &r.ytdlp_path {
        std::env::set_var("OMNIGET_WORKER_YTDLP", path)
    } else {
        std::env::remove_var("OMNIGET_WORKER_YTDLP")
    }
    if let Some(path) = &r.ffmpeg_path {
        std::env::set_var("OMNIGET_WORKER_FFMPEG", path)
    } else {
        std::env::remove_var("OMNIGET_WORKER_FFMPEG")
    }
    for (key, path) in [
        ("OMNIGET_WORKER_FFPROBE", &r.ffprobe_path),
        ("OMNIGET_WORKER_NODE", &r.node_path),
        ("OMNIGET_WORKER_DENO", &r.deno_path),
    ] {
        if let Some(path) = path {
            std::env::set_var(key, path);
        } else {
            std::env::remove_var(key);
        }
    }
    http_client::pin_proxy(ProxySettings {
        enabled: true,
        proxy_type: "http".into(),
        host: "127.0.0.1".into(),
        port,
        username: "omniget".into(),
        password: reqwest::Url::parse(&r.proxy)
            .map_err(|_| "INVALID_PROXY")?
            .password()
            .ok_or("INVALID_PROXY")?
            .to_owned(),
    });
    ytdlp::set_global_cookie_file_fn(|| None);
    ytdlp::set_per_domain_cookie_fn(|_| None);
    ytdlp::set_cookies_from_browser_fn(String::new);
    ytdlp::set_manual_cookie_header_fn(String::new);
    ytdlp::set_managed_cookies_only_fn(|| true);
    let include_auto = r.include_auto_subtitles;
    ytdlp::set_include_auto_subs_fn(move || include_auto);
    let user_agent = r.user_agent.clone();
    ytdlp::set_user_agent_fn(move || user_agent.clone());
    ytdlp::set_ext_cookie_path_fn(|| PathBuf::from("/dev/null"));
    let compatible_video = r.compatible_video;
    ytdlp::set_compatible_video_fn(move || compatible_video);
    ytdlp::set_command_sink(Some(Arc::new(|line: &str| {
        let _ = emit(command_event(line));
    })));
    confine_postprocessing();
    Ok(())
}
fn command_event(line: &str) -> Value {
    json!({"type":"diagnostic","detail":line.chars().take(2000).collect::<String>()})
}
fn diagnostic_event(code: &str, raw: &str) -> Value {
    json!({"type":"diagnostic","detail":format!("{code}: {raw}").chars().take(2000).collect::<String>()})
}
fn confine_postprocessing() {
    ytdlp::set_embed_metadata_fn(|| false);
    ytdlp::set_embed_thumbnail_fn(|| false);
    ytdlp::set_sponsorblock_fn(|| false);
    ytdlp::set_split_chapters_fn(|| false);
    ytdlp::set_live_from_start_fn(|| false);
    ytdlp::set_translate_metadata_fn(|| None);
}
fn engine_format(
    format_id: Option<String>,
    ceiling: Option<u32>,
    unsized_ytdlp: bool,
    compatible_video: bool,
    ffmpeg: bool,
) -> Option<String> {
    match (format_id, ceiling) {
        (Some(f), _) => Some(f),
        (None, Some(ceiling)) if unsized_ytdlp && !compatible_video => {
            Some(ceiling_selector(ceiling, ffmpeg))
        }
        _ => None,
    }
}
async fn run(r: Request) -> Result<(), &'static str> {
    if matches!(r.operation, Operation::Collection) {
        let path = r
            .ytdlp_path
            .as_ref()
            .ok_or("WORKER_DEPENDENCY_UNAVAILABLE")?;
        let flags = vec![
            "--playlist-start".into(),
            (r.offset + 1).to_string(),
            "--playlist-end".into(),
            (r.offset + r.limit).to_string(),
        ];
        let (title, entries) = match ytdlp::get_playlist_info(path, &r.url, &flags).await {
            Ok(value) => value,
            Err(e) => return engine_error(e),
        };
        if title.len() > 4096
            || entries.len() > r.limit as usize
            || entries
                .iter()
                .any(|e| e.title.len() > 4096 || !valid_url(&e.url))
        {
            return Err("COLLECTION_OUTPUT_LIMIT");
        }
        let more = entries.len() == r.limit as usize;
        return emit(
            json!({"type":"result","version":1,"success":true,"result":{"title":title,"items":entries.iter().map(|e|json!({"title":e.title,"url":e.url})).collect::<Vec<_>>(),"nextCursor":if more{Some(r.offset+r.limit)}else{None},"total":null}}),
        );
    }
    let registry = registry();
    let platform = if let Some(name) = &r.platform {
        registry.find_by_name(name).ok_or("UNSUPPORTED_PLATFORM")?
    } else {
        registry
            .find_platform(&r.url)
            .ok_or("UNSUPPORTED_PLATFORM")?
    };
    let explicit_policy = matches!(r.operation, Operation::Download)
        && (matches!(r.download_mode, Mode::Audio) || r.quality.is_some() || r.format_id.is_some());
    let mut info = match r.media_info {
        Some(i) if explicit_policy || !needs_in_process_extraction(&i) => i,
        _ => match platform.get_media_info(&r.url).await {
            Ok(i) => i,
            Err(e) => return engine_error(e),
        },
    };
    if explicit_policy {
        route_direct_to_engine(&mut info, &r.url);
    }
    validate_info(&info)?;
    if matches!(r.operation, Operation::Inspect) {
        let mut audio_languages = ytdlp::cached_audio_languages(&r.url);
        audio_languages.truncate(64);
        return emit(
            json!({"type":"media_info","version":1,"platform":platform.name(),"media_info":info,"audio_languages":audio_languages}),
        );
    }
    let video = matches!(r.download_mode, Mode::Video);
    if let Some(ceiling) = r.quality.filter(|_| video) {
        if exceeds_ceiling(&info, ceiling) {
            let _ = emit(
                json!({"type":"result","version":1,"success":false,"code":"FORMAT_UNAVAILABLE","category":"format_unavailable","message":"No available format fits the requested maxHeight.","raw_error_omitted":true}),
            );
            return Err("ENGINE_FAILED_REPORTED");
        }
        fit_ceiling(&mut info, ceiling);
    }
    let format_id = engine_format(
        r.format_id,
        r.quality.filter(|_| video),
        unsized_ytdlp(&info),
        r.compatible_video,
        r.ffmpeg_path.is_some(),
    );
    let opts = DownloadOptions {
        quality: r.quality.map(|q| q.to_string()),
        output_dir: r.output_dir.ok_or("INVALID_OUTPUT")?,
        filename_template: None,
        download_subtitles: r.subtitles,
        include_auto_subtitles: r.include_auto_subtitles,
        download_mode: Some(
            match r.download_mode {
                Mode::Video => "video",
                Mode::Audio => "audio",
            }
            .into(),
        ),
        audio_format: r.audio_format,
        format_id,
        referer: r.referer,
        extra_headers: r.extra_headers,
        page_url: r.page_url,
        user_agent: r.user_agent,
        cancel_token: CancellationToken::new(),
        concurrent_fragments: r.concurrent_fragments,
        ytdlp_path: r.ytdlp_path,
        torrent_listen_port: None,
        torrent_id_slot: None,
        custom_ytdlp_args: None,
        torrent_files: None,
        torrent_auto_trackers: false,
        torrent_upnp: false,
        torrent_stop_seeding: false,
        playlist_number_files: false,
    };
    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<omniget_core::models::progress::ProgressUpdate>(32);
    let progress = tokio::spawn(async move {
        while let Some(p) = rx.recv().await {
            let phase = match p.phase.as_deref() {
                Some("downloading") => "downloading",
                Some("merging") => "merging",
                Some("converting") => "converting",
                _ => "running",
            };
            emit(
                json!({"type":"progress","percent":p.percent_value(),"downloaded_bytes":p.downloaded_bytes,"total_bytes":p.total_bytes,"speed_bps":p.speed_bps,"eta_seconds":p.eta_seconds,"phase":phase}),
            )?;
        }
        Ok::<(), &'static str>(())
    });
    let result = platform.download(&info, &opts, tx).await;
    progress.await.map_err(|_| "PROGRESS_FAILED")??;
    match result {
        Ok(result) => emit(
            json!({"type":"result","version":1,"success":true,"platform":platform.name(),"result":result}),
        ),
        Err(e) => engine_error(e),
    }
}
fn route_direct_to_engine(info: &mut MediaInfo, page_url: &str) -> bool {
    match info.available_qualities.as_mut_slice() {
        [q] if q.format == "tiktok_direct" => {
            q.format = "ytdlp".into();
            q.url = page_url.to_owned();
            q.width = 0;
            q.height = 0;
            true
        }
        _ => false,
    }
}
fn needs_in_process_extraction(info: &MediaInfo) -> bool {
    info.available_qualities
        .iter()
        .any(|q| q.format == "tiktok_direct")
}
fn exceeds_ceiling(info: &MediaInfo, ceiling: u32) -> bool {
    !info.available_qualities.is_empty()
        && info
            .available_qualities
            .iter()
            .all(|q| q.height > 0 && q.height > ceiling)
}
fn fit_ceiling(info: &mut MediaInfo, ceiling: u32) {
    if info.media_type != MediaType::Video {
        return;
    }
    let q = &mut info.available_qualities;
    if q.iter().all(|v| v.format == "ytdlp") {
        return;
    }
    if q.iter()
        .any(|v| v.format != "ytdlp" && v.height > 0 && v.height <= ceiling)
    {
        q.retain(|v| v.format != "ytdlp" && v.height > 0 && v.height <= ceiling);
        q.sort_by_key(|v| std::cmp::Reverse(v.height));
        return;
    }
    q.retain(|v| v.height == 0);
    if q.iter().any(|v| v.format == "ytdlp") {
        q.retain(|v| v.format == "ytdlp");
    }
}
fn unsized_ytdlp(info: &MediaInfo) -> bool {
    !info.available_qualities.is_empty()
        && info
            .available_qualities
            .iter()
            .all(|q| q.format == "ytdlp" && q.height == 0)
}
fn ceiling_selector(ceiling: u32, ffmpeg: bool) -> String {
    if ffmpeg {
        format!("bv*[height<={ceiling}]+ba/b[height<={ceiling}]/bv*[height<=?{ceiling}]+ba/b[height<=?{ceiling}]/ba")
    } else {
        format!("b[height<={ceiling}]/b[height<=?{ceiling}]/ba")
    }
}
fn engine_error(error: anyhow::Error) -> Result<(), &'static str> {
    let rate_limit = error.chain().find_map(|cause| {
        cause.downcast_ref::<omniget_core::core::direct_downloader::RateLimitError>()
    });
    let retry_after_seconds = rate_limit.and_then(|e| e.retry_after_seconds);
    let raw = format!("{error:#}");
    let (code, category, message) = classify(&raw, rate_limit.is_some());
    let _ = emit(diagnostic_event(code, &raw));
    let _ = emit(
        json!({"type":"result","version":1,"success":false,"code":code,"category":category,"message":message,"retry_after_seconds":retry_after_seconds,"raw_error_omitted":true}),
    );
    Err("ENGINE_FAILED_REPORTED")
}
fn classify(raw: &str, rate_limit_error: bool) -> (&'static str, &'static str, &'static str) {
    let lower = raw.to_ascii_lowercase();
    if raw.contains("WORKER_DEPENDENCY_UNAVAILABLE") {
        return (
            "WORKER_DEPENDENCY_UNAVAILABLE",
            "dependency",
            "A required pinned executable is unavailable in the worker.",
        );
    }
    if lower.contains("requested format is not available") {
        return (
            "FORMAT_UNAVAILABLE",
            "format_unavailable",
            "No available format matches the requested options.",
        );
    }
    if lower.contains("ip address is blocked")
        || lower.contains("your ip is blocked")
        || lower.contains("blocked from accessing")
        || lower.contains("ip has been blocked")
    {
        return (
            "BLOCKED_BY_PLATFORM",
            "blocked_by_platform",
            "The platform blocked access to this content from this network.",
        );
    }
    if lower.contains("not a bot") || lower.contains("captcha") || lower.contains("bot challenge") {
        return (
            "BOT_CHALLENGE",
            "bot_challenge",
            "The platform requires a bot challenge.",
        );
    }
    let rate_limited = rate_limit_error
        || lower.contains("http error 429")
        || lower.contains("http 429")
        || lower.contains("429 too many")
        || lower.contains("too many requests")
        || lower.contains("rate limit");
    if rate_limited {
        return (
            "RATE_LIMITED",
            "rate_limited",
            "Too many requests. Try again in a few minutes.",
        );
    }
    match omniget_core::core::errors::classify_download_error(raw).0 {
        "auth_required" => (
            "AUTH_REQUIRED",
            "auth_required",
            "This content requires login.",
        ),
        "access_denied" => (
            "ACCESS_DENIED",
            "access_denied",
            "Access was denied (HTTP 403); the cause is not confirmed.",
        ),
        "not_found" => (
            "NOT_FOUND",
            "not_found",
            "Content not found or has been deleted.",
        ),
        "invalid_output" => (
            "OUTPUT_INVALID",
            "invalid_output",
            "The server returned a page instead of media.",
        ),
        "file_missing" => (
            "OUTPUT_MISSING",
            "file_missing",
            "Downloaded file could not be located in the output folder.",
        ),
        "broken_source" => (
            "BROKEN_SOURCE",
            "broken_source",
            "The source stopped serving part of the media (fragment failure).",
        ),
        "restricted" => (
            "SOURCE_RESTRICTED",
            "restricted",
            "This content is private or age-restricted.",
        ),
        "blocked_by_platform" => (
            "BLOCKED_BY_PLATFORM",
            "blocked_by_platform",
            "The platform blocked access to this content from this network.",
        ),
        "egress_failed" => (
            "EGRESS_FAILED",
            "egress_failed",
            "Network egress failed (proxy, TLS or DNS) before the platform answered.",
        ),
        "ffmpeg_needed" => (
            "FFMPEG_REQUIRED",
            "ffmpeg_needed",
            "FFmpeg is required for this download.",
        ),
        "ytdlp_needed" => ("YTDLP_REQUIRED", "ytdlp_needed", "yt-dlp is required."),
        "postprocess_failed" => (
            "POSTPROCESS_FAILED",
            "postprocess_failed",
            "Post-processing failed.",
        ),
        "format_unavailable" => (
            "FORMAT_UNAVAILABLE",
            "format_unavailable",
            "No format of this media fits the requested options.",
        ),
        "extractor_failure" => (
            "EXTRACTOR_FAILURE",
            "extractor_failure",
            "The extractor could not read this page.",
        ),
        "server_error" => (
            "SERVER_ERROR",
            "server_error",
            "The server had a temporary error (HTTP 5xx).",
        ),
        _ => (
            "ENGINE_FAILED",
            "unknown",
            "The engine failed without a recognized cause.",
        ),
    }
}
fn main() {
    std::panic::set_hook(Box::new(|_| {
        let _ = emit(json!({"type":"result","success":false,"code":"WORKER_PANIC"}));
    }));
    let outcome = (|| {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| "INPUT_FAILED")?;
        if bytes.len() > 65536 {
            return Err("INPUT_LIMIT");
        }
        let request: Request = serde_json::from_slice(&bytes).map_err(|_| "INVALID_REQUEST")?;
        let port = validate(&request)?;
        setup(&request, port)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| "RUNTIME_FAILED")?;
        runtime.block_on(run(request))
    })();
    if let Err(code) = outcome {
        if code != "ENGINE_FAILED_REPORTED" {
            let _ = emit(json!({"type":"result","version":1,"success":false,"code":code}));
        }
        std::process::exit(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_the_app_builds_pass_the_allow_list() {
        for selector in [
            "bv*[height<=720]+ba/b[height<=720]/bv*[height<=?720]+ba/b[height<=?720]/ba",
            "bv*+ba[language^=pt]/bv*+ba/b",
            "137+140",
            "ba[language^=en-US]/ba/b",
        ] {
            assert!(valid_format_selector(selector), "{selector}");
        }
        let longest =
            ytdlp::audio_language_format(None, Some(2160), "auto", true, true, "zh-Hans-desc")
                .unwrap();
        assert!(valid_format_selector(&longest), "{longest}");
    }

    #[test]
    fn a_selector_that_tries_to_break_out_is_rejected() {
        for selector in [
            "",
            "bv*+ba[language^=pt];id",
            "bv*+ba[language^=$(id)]",
            "bv*+ba[language^=`id`]",
            "bv*+ba --exec id",
            "bv*+ba[language^='pt']",
            "bv*+ba[language^=\"pt\"]",
            "bv*+ba\n--exec",
            "bv*+ba|sh",
            "bv*+ba&id",
            "bv*+ba[title~=(a)]",
            "bv*+ba{x}",
            "bv*+ba%(id)s",
            "bv*+ba\\",
            "bv*+ba:x",
            "bv*+ba#x",
        ] {
            assert!(!valid_format_selector(selector), "{selector:?}");
        }
        assert!(!valid_format_selector(&"b/".repeat(257)));
    }

    #[test]
    fn a_chosen_format_is_used_as_is_whatever_the_video_setting() {
        for compatible in [false, true] {
            assert_eq!(
                engine_format(Some("137+140".into()), Some(720), true, compatible, true).as_deref(),
                Some("137+140")
            );
        }
    }

    #[test]
    fn the_height_ceiling_becomes_a_selector_only_when_the_setting_is_best_quality() {
        assert_eq!(
            engine_format(None, Some(720), true, false, true),
            Some(ceiling_selector(720, true))
        );
        assert_eq!(engine_format(None, Some(720), true, true, true), None);
    }

    #[test]
    fn sized_formats_and_no_ceiling_never_get_a_selector() {
        assert_eq!(engine_format(None, Some(720), false, false, true), None);
        assert_eq!(engine_format(None, None, true, false, true), None);
    }

    #[test]
    fn a_request_without_the_setting_reads_as_best_quality() {
        let request: Request = serde_json::from_value(json!({
            "version": 1,
            "operation": "inspect",
            "url": "https://example.com/v",
            "proxy": "http://omniget:x@127.0.0.1:1/",
            "runtime_dir": "/tmp/runtime",
        }))
        .unwrap();
        assert!(!request.compatible_video);
    }
}
