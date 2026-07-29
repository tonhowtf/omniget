use crate::models::progress::ProgressUpdate;
use anyhow::anyhow;
use async_trait::async_trait;
use regex::Regex;
use tokio::sync::mpsc;

use crate::core::direct_downloader::download_direct_with_headers;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
/// Threads login-walls some anonymous requests; crawler UA gets the full page
const GOOGLEBOT_UA: &str =
    "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";

pub struct ThreadsDownloader {
    client: reqwest::Client,
}

pub(crate) enum ThreadsMedia {
    Single {
        url: String,
        is_video: bool,
        width: u32,
        height: u32,
    },
    Carousel {
        items: Vec<CarouselItem>,
    },
}

pub(crate) struct CarouselItem {
    url: String,
    is_video: bool,
    width: u32,
    height: u32,
}

impl Default for ThreadsDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadsDownloader {
    pub fn new() -> Self {
        let mut builder = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15));

        if let Some(jar) =
            crate::core::cookie_parser::load_extension_cookies_for_domain("threads.com")
        {
            builder = builder.cookie_provider(jar);
        }

        let client = builder.build().unwrap_or_default();
        Self { client }
    }

    /// Estrae il post ID da vari formati URL di Threads
    /// Supporta:
    /// - https://www.threads.net/@user/post/POST_ID
    /// - https://www.threads.com/@user/post/POST_ID
    /// - https://www.threads.net/t/POST_ID
    /// - https://www.threads.com/t/POST_ID
    pub fn extract_post_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        // Formato: /@user/post/POST_ID
        if segments.len() >= 3 && segments[0].starts_with('@') && segments[1] == "post" {
            return Some(segments[2].to_string());
        }

        // Formato: /t/POST_ID
        if segments.len() >= 2 && segments[0] == "t" {
            return Some(segments[1].to_string());
        }

        None
    }

    /// Fetch della pagina e estrazione del post dal JSON bootstrap
    async fn fetch_post(&self, url: &str, post_id: &str) -> anyhow::Result<serde_json::Value> {
        let html = self.fetch_page(url, USER_AGENT).await?;
        if let Some(post) = Self::find_post_in_html(&html, post_id) {
            return Ok(post);
        }

        // Alcune richieste anonime vengono murate dal login; col UA crawler
        // Threads serve la pagina completa
        tracing::debug!("[threads] post not found with browser UA, retrying with crawler UA");
        let html = self.fetch_page(url, GOOGLEBOT_UA).await?;
        if let Some(post) = Self::find_post_in_html(&html, post_id) {
            return Ok(post);
        }

        Err(anyhow!(
            "Could not extract post data from Threads page. The post may be private or deleted."
        ))
    }

    async fn fetch_page(&self, url: &str, user_agent: &str) -> anyhow::Result<String> {
        let response = self
            .client
            .get(url)
            .header("User-Agent", user_agent)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-GB,en;q=0.9")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .header("Sec-Fetch-User", "?1")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("Threads returned HTTP {}", response.status()));
        }

        Ok(response.text().await?)
    }

    /// Cerca il post nei <script data-sjs> della pagina. Threads cambia spesso
    /// il nesting del bootstrap (require/__bbox/result/data/media), quindi invece
    /// di un path fisso parsiamo ogni script e cerchiamo ricorsivamente
    /// l'oggetto con il `code` del post e chiavi media.
    fn find_post_in_html(html: &str, post_id: &str) -> Option<serde_json::Value> {
        let re = Regex::new(r#"(?s)<script[^>]*data-sjs[^>]*>(.*?)</script>"#).ok()?;

        for cap in re.captures_iter(html) {
            let script_content = cap.get(1).map(|m| m.as_str()).unwrap_or("");

            if !script_content.contains(post_id) {
                continue;
            }

            let Ok(json) = serde_json::from_str::<serde_json::Value>(script_content) else {
                continue;
            };

            if let Some(post) = Self::find_post_recursive(&json, post_id) {
                return Some(post);
            }
        }

        None
    }

    /// Cerca ricorsivamente l'oggetto post (code == post_id con chiavi media)
    pub(crate) fn find_post_recursive(
        value: &serde_json::Value,
        post_id: &str,
    ) -> Option<serde_json::Value> {
        match value {
            serde_json::Value::Object(map) => {
                let is_post = map.get("code").and_then(|v| v.as_str()) == Some(post_id)
                    && (map.contains_key("video_versions")
                        || map.contains_key("image_versions2")
                        || map.contains_key("carousel_media"));
                if is_post {
                    return Some(value.clone());
                }
                map.values()
                    .find_map(|v| Self::find_post_recursive(v, post_id))
            }
            serde_json::Value::Array(items) => items
                .iter()
                .find_map(|v| Self::find_post_recursive(v, post_id)),
            _ => None,
        }
    }

    /// Verifica se l'oggetto contiene media effettivamente scaricabili
    fn has_usable_media(media: &serde_json::Value) -> bool {
        let video = media
            .get("video_versions")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        let carousel = media
            .get("carousel_media")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        let image = media
            .pointer("/image_versions2/candidates")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        video || carousel || image
    }

    /// Per post di tipo share (media_type=19) i campi media diretti sono vuoti
    /// e il media vive in text_post_app_info.linked_inline_media (o nel post
    /// quotato/repostato). Ritorna l'oggetto che porta il media effettivo.
    pub(crate) fn resolve_media_container(post: &serde_json::Value) -> &serde_json::Value {
        if Self::has_usable_media(post) {
            return post;
        }

        if let Some(linked) = post.pointer("/text_post_app_info/linked_inline_media") {
            if Self::has_usable_media(linked) {
                return linked;
            }
        }

        for key in ["quoted_post", "reposted_post"] {
            let path = format!("/text_post_app_info/share_info/{}", key);
            if let Some(nested) = post.pointer(&path) {
                if !nested.is_null() && Self::has_usable_media(nested) {
                    return nested;
                }
            }
        }

        post
    }

    /// Estrae i media (video/immagini) dal post
    pub(crate) fn extract_media_from_post(
        post: &serde_json::Value,
    ) -> anyhow::Result<ThreadsMedia> {
        // Controlla se è un carousel
        if let Some(carousel_media) = post.get("carousel_media").and_then(|v| v.as_array()) {
            if !carousel_media.is_empty() {
                let mut items = Vec::new();

                for media in carousel_media {
                    if let Some(item) = Self::extract_single_media(media)? {
                        items.push(item);
                    }
                }

                if !items.is_empty() {
                    return Ok(ThreadsMedia::Carousel { items });
                }
            }
        }

        // Media singolo
        if let Some(item) = Self::extract_single_media(post)? {
            return Ok(ThreadsMedia::Single {
                url: item.url,
                is_video: item.is_video,
                width: item.width,
                height: item.height,
            });
        }

        Err(anyhow!("No media found in Threads post"))
    }

    /// Estrae un singolo elemento media (video o immagine)
    fn extract_single_media(media: &serde_json::Value) -> anyhow::Result<Option<CarouselItem>> {
        // Le dimensioni stanno sul post, non sulle singole versioni
        let original_width = media
            .get("original_width")
            .and_then(|w| w.as_u64())
            .unwrap_or(0) as u32;
        let original_height = media
            .get("original_height")
            .and_then(|h| h.as_u64())
            .unwrap_or(0) as u32;

        // Video: video_versions ha entries {type, url}; type più basso = qualità migliore
        if let Some(video_versions) = media.get("video_versions").and_then(|v| v.as_array()) {
            if !video_versions.is_empty() {
                if let Some(url) = video_versions
                    .iter()
                    .min_by_key(|v| v.get("type").and_then(|t| t.as_u64()).unwrap_or(u64::MAX))
                    .and_then(|v| v.get("url"))
                    .and_then(|u| u.as_str())
                {
                    return Ok(Some(CarouselItem {
                        url: url.to_string(),
                        is_video: true,
                        width: original_width,
                        height: original_height,
                    }));
                }
            }
        }

        // Immagini: candidates ordinati per risoluzione decrescente
        if let Some(candidates) = media
            .pointer("/image_versions2/candidates")
            .and_then(|v| v.as_array())
        {
            if let Some(best_image) = candidates.iter().max_by_key(|c| {
                c.get("width").and_then(|w| w.as_u64()).unwrap_or(0)
                    * c.get("height").and_then(|h| h.as_u64()).unwrap_or(0)
            }) {
                if let Some(url) = best_image.get("url").and_then(|u| u.as_str()) {
                    let width = best_image
                        .get("width")
                        .and_then(|w| w.as_u64())
                        .unwrap_or(0) as u32;
                    let height = best_image
                        .get("height")
                        .and_then(|h| h.as_u64())
                        .unwrap_or(0) as u32;

                    return Ok(Some(CarouselItem {
                        url: url.to_string(),
                        is_video: false,
                        width,
                        height,
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Poster frame del video / migliore immagine come thumbnail
    fn best_thumbnail(media: &serde_json::Value) -> Option<String> {
        let candidates = media
            .pointer("/image_versions2/candidates")
            .and_then(|v| v.as_array())?;
        candidates
            .iter()
            .max_by_key(|c| {
                c.get("width").and_then(|w| w.as_u64()).unwrap_or(0)
                    * c.get("height").and_then(|h| h.as_u64()).unwrap_or(0)
            })
            .and_then(|c| c.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string())
    }

    /// Estrae metadata dal post (uploader, caption, etc.)
    fn extract_metadata(post: &serde_json::Value) -> (String, Option<u64>) {
        let uploader = post
            .pointer("/user/username")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let timestamp = post.get("taken_at").and_then(|v| v.as_u64());

        (uploader, timestamp)
    }

    fn threads_headers() -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::REFERER,
            "https://www.threads.com/".parse().unwrap(),
        );
        headers.insert(
            reqwest::header::ORIGIN,
            "https://www.threads.com".parse().unwrap(),
        );
        headers
    }
}

#[async_trait]
impl PlatformDownloader for ThreadsDownloader {
    fn name(&self) -> &str {
        "threads"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "threads.net"
                    || host.ends_with(".threads.net")
                    || host == "threads.com"
                    || host.ends_with(".threads.com");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let post_id = Self::extract_post_id(url)
            .ok_or_else(|| anyhow!("Could not extract post ID from Threads URL"))?;

        let filename_base = format!("threads_{}", post_id);

        // Fetch della pagina e ricerca del post nel JSON bootstrap
        let post = self.fetch_post(url, &post_id).await?;

        // Per post share/repost il media vive in un contenitore annidato
        let container = Self::resolve_media_container(&post);
        let thumbnail_url = Self::best_thumbnail(container);

        // Estrai metadata
        let (uploader, _timestamp) = Self::extract_metadata(&post);

        // Estrai media
        let media = Self::extract_media_from_post(container)?;

        match media {
            ThreadsMedia::Single {
                url,
                is_video,
                width,
                height,
            } => {
                let (media_type, format) = if is_video {
                    (MediaType::Video, "mp4")
                } else {
                    // Determina estensione dall'URL
                    let ext = if url.contains(".webp") { "webp" } else { "jpg" };
                    (MediaType::Photo, ext)
                };

                Ok(MediaInfo {
                    title: filename_base,
                    author: uploader,
                    platform: "threads".to_string(),
                    duration_seconds: None,
                    thumbnail_url,
                    available_qualities: vec![VideoQuality {
                        label: if width > 0 && height > 0 {
                            format!("{}x{}", width, height)
                        } else {
                            "original".to_string()
                        },
                        width,
                        height,
                        url,
                        format: format.to_string(),
                    }],
                    media_type,
                    file_size_bytes: None,
                })
            }
            ThreadsMedia::Carousel { items } => {
                let qualities: Vec<VideoQuality> = items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let format = if item.is_video {
                            "mp4".to_string()
                        } else if item.url.contains(".webp") {
                            "webp".to_string()
                        } else {
                            "jpg".to_string()
                        };

                        VideoQuality {
                            label: format!("media_{}", i + 1),
                            width: item.width,
                            height: item.height,
                            url: item.url.clone(),
                            format,
                        }
                    })
                    .collect();

                Ok(MediaInfo {
                    title: filename_base,
                    author: uploader,
                    platform: "threads".to_string(),
                    duration_seconds: None,
                    thumbnail_url,
                    available_qualities: qualities,
                    media_type: MediaType::Carousel,
                    file_size_bytes: None,
                })
            }
        }
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let count = info.available_qualities.len();

        if count == 0 {
            return Err(anyhow!("No downloadable media found in Threads post"));
        }

        if count == 1 {
            let quality = info.available_qualities.first().unwrap();
            let filename = format!(
                "{}.{}",
                sanitize_filename::sanitize(&info.title),
                quality.format
            );
            let output = opts.output_dir.join(&filename);

            let mut hdr_map = Self::threads_headers();
            crate::core::http_client::inject_ua_header(&mut hdr_map, opts.user_agent.as_deref());
            let headers = Some(hdr_map);

            let bytes = download_direct_with_headers(
                &self.client,
                &quality.url,
                &output,
                progress,
                headers,
                Some(&opts.cancel_token),
            )
            .await?;

            return Ok(DownloadResult {
                file_path: output,
                file_size_bytes: bytes,
                duration_seconds: 0.0,
                torrent_id: None,
            });
        }

        // Carousel: download di ogni elemento
        let mut total_bytes = 0u64;
        let mut last_path = opts.output_dir.clone();

        for (i, quality) in info.available_qualities.iter().enumerate() {
            let filename = format!(
                "{}_{}.{}",
                sanitize_filename::sanitize(&info.title),
                i + 1,
                quality.format,
            );
            let output = opts.output_dir.join(&filename);
            let (tx, _rx) = mpsc::channel(8);

            let mut hdr_map = Self::threads_headers();
            crate::core::http_client::inject_ua_header(&mut hdr_map, opts.user_agent.as_deref());
            let headers = Some(hdr_map);

            let bytes = download_direct_with_headers(
                &self.client,
                &quality.url,
                &output,
                tx,
                headers,
                Some(&opts.cancel_token),
            )
            .await?;

            total_bytes += bytes;
            last_path = output;

            let percent = ((i + 1) as f64 / count as f64) * 100.0;
            let _ = progress.send(ProgressUpdate::percent(percent)).await;
        }

        Ok(DownloadResult {
            file_path: last_path,
            file_size_bytes: total_bytes,
            duration_seconds: 0.0,
            torrent_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_post_id_from_user_post_url() {
        let url = "https://www.threads.com/@zuck/post/DHV7vTivqWD";
        assert_eq!(
            ThreadsDownloader::extract_post_id(url),
            Some("DHV7vTivqWD".to_string())
        );
    }

    #[test]
    fn extract_post_id_from_t_url() {
        let url = "https://www.threads.net/t/DHV7vTivqWD";
        assert_eq!(
            ThreadsDownloader::extract_post_id(url),
            Some("DHV7vTivqWD".to_string())
        );
    }

    #[test]
    fn extract_post_id_from_threads_net_user_post() {
        let url = "https://www.threads.net/@user/post/ABC123";
        assert_eq!(
            ThreadsDownloader::extract_post_id(url),
            Some("ABC123".to_string())
        );
    }

    #[test]
    fn extract_post_id_rejects_invalid_url() {
        let url = "https://www.threads.com/@user";
        assert_eq!(ThreadsDownloader::extract_post_id(url), None);
    }

    #[test]
    fn extract_post_id_rejects_non_threads_url() {
        let url = "https://www.instagram.com/p/ABC123";
        assert_eq!(ThreadsDownloader::extract_post_id(url), None);
    }

    #[test]
    fn can_handle_threads_com() {
        let downloader = ThreadsDownloader::new();
        assert!(downloader.can_handle("https://www.threads.com/@user/post/ABC123"));
    }

    #[test]
    fn can_handle_threads_net() {
        let downloader = ThreadsDownloader::new();
        assert!(downloader.can_handle("https://www.threads.net/t/ABC123"));
    }

    #[test]
    fn cannot_handle_instagram() {
        let downloader = ThreadsDownloader::new();
        assert!(!downloader.can_handle("https://www.instagram.com/p/ABC123"));
    }

    #[test]
    fn cannot_handle_twitter() {
        let downloader = ThreadsDownloader::new();
        assert!(!downloader.can_handle("https://x.com/user/status/123"));
    }

    #[test]
    fn extract_media_from_video_post() {
        let post = serde_json::json!({
            "code": "ABC123",
            "original_width": 1080,
            "original_height": 1920,
            "video_versions": [
                { "type": 102, "url": "https://scontent.cdninstagram.com/video_low.mp4" },
                { "type": 101, "url": "https://scontent.cdninstagram.com/video_best.mp4" }
            ]
        });

        let result = ThreadsDownloader::extract_media_from_post(&post).unwrap();
        match result {
            ThreadsMedia::Single {
                url,
                is_video,
                width,
                height,
            } => {
                assert_eq!(url, "https://scontent.cdninstagram.com/video_best.mp4");
                assert!(is_video);
                assert_eq!(width, 1080);
                assert_eq!(height, 1920);
            }
            _ => panic!("Expected Single media"),
        }
    }

    #[test]
    fn extract_media_from_image_post() {
        let post = serde_json::json!({
            "code": "ABC123",
            "image_versions2": {
                "candidates": [
                    {
                        "url": "https://scontent.cdninstagram.com/image.jpg",
                        "width": 1080,
                        "height": 1080
                    }
                ]
            }
        });

        let result = ThreadsDownloader::extract_media_from_post(&post).unwrap();
        match result {
            ThreadsMedia::Single {
                url,
                is_video,
                width,
                height,
            } => {
                assert_eq!(url, "https://scontent.cdninstagram.com/image.jpg");
                assert!(!is_video);
                assert_eq!(width, 1080);
                assert_eq!(height, 1080);
            }
            _ => panic!("Expected Single media"),
        }
    }

    #[test]
    fn extract_media_from_carousel_post() {
        let post = serde_json::json!({
            "code": "ABC123",
            "carousel_media": [
                {
                    "original_width": 1080,
                    "original_height": 1920,
                    "video_versions": [
                        { "type": 101, "url": "https://scontent.cdninstagram.com/video1.mp4" }
                    ]
                },
                {
                    "image_versions2": {
                        "candidates": [
                            {
                                "url": "https://scontent.cdninstagram.com/image1.jpg",
                                "width": 1080,
                                "height": 1080
                            }
                        ]
                    }
                }
            ]
        });

        let result = ThreadsDownloader::extract_media_from_post(&post).unwrap();
        match result {
            ThreadsMedia::Carousel { items } => {
                assert_eq!(items.len(), 2);
                assert!(items[0].is_video);
                assert!(!items[1].is_video);
            }
            _ => panic!("Expected Carousel media"),
        }
    }

    #[test]
    fn find_post_recursive_from_nested_structure() {
        let data = serde_json::json!({
            "require": [[
                "Preloader", null, null,
                [{ "__bbox": { "result": { "data": { "media": {
                    "code": "ABC123",
                    "video_versions": [],
                    "user": { "username": "testuser" }
                } } } } }]
            ]]
        });

        let result = ThreadsDownloader::find_post_recursive(&data, "ABC123");
        assert!(result.is_some());
        assert_eq!(
            result
                .unwrap()
                .pointer("/user/username")
                .and_then(|v| v.as_str()),
            Some("testuser")
        );
    }

    #[test]
    fn find_post_recursive_returns_none_for_wrong_id() {
        let data = serde_json::json!({
            "result": { "data": { "media": { "code": "XYZ789", "video_versions": [] } } }
        });

        assert!(ThreadsDownloader::find_post_recursive(&data, "ABC123").is_none());
    }

    #[test]
    fn find_post_recursive_ignores_dict_without_media_keys() {
        let data = serde_json::json!({
            "seo": { "code": "ABC123" },
            "result": { "data": { "media": {
                "code": "ABC123",
                "image_versions2": { "candidates": [] }
            } } }
        });

        let result = ThreadsDownloader::find_post_recursive(&data, "ABC123").unwrap();
        assert!(result.get("image_versions2").is_some());
    }

    #[test]
    fn resolve_media_container_follows_linked_inline_media() {
        // Post share (media_type=19): campi media diretti vuoti,
        // media reale in text_post_app_info.linked_inline_media
        let post = serde_json::json!({
            "code": "ABC123",
            "media_type": 19,
            "video_versions": null,
            "carousel_media": null,
            "image_versions2": { "candidates": [] },
            "text_post_app_info": {
                "linked_inline_media": {
                    "media_type": 2,
                    "original_width": 1080,
                    "original_height": 1920,
                    "video_versions": [
                        { "type": 101, "url": "https://scontent.cdninstagram.com/linked.mp4" }
                    ]
                }
            }
        });

        let container = ThreadsDownloader::resolve_media_container(&post);
        let result = ThreadsDownloader::extract_media_from_post(container).unwrap();
        match result {
            ThreadsMedia::Single {
                url,
                is_video,
                width,
                height,
            } => {
                assert_eq!(url, "https://scontent.cdninstagram.com/linked.mp4");
                assert!(is_video);
                assert_eq!(width, 1080);
                assert_eq!(height, 1920);
            }
            _ => panic!("Expected Single media from linked_inline_media"),
        }
    }

    #[test]
    fn resolve_media_container_returns_post_when_media_direct() {
        let post = serde_json::json!({
            "code": "ABC123",
            "video_versions": [
                { "type": 101, "url": "https://scontent.cdninstagram.com/direct.mp4" }
            ]
        });

        let container = ThreadsDownloader::resolve_media_container(&post);
        assert!(std::ptr::eq(container, &post));
    }
}
