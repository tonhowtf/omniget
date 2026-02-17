use anyhow::anyhow;
use async_trait::async_trait;
use rand::RngExt;
use regex::Regex;
use tokio::sync::mpsc;

use crate::core::direct_downloader::download_direct_with_headers;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const IG_APP_ID: &str = "936619743392459";
const GQL_DOC_ID: &str = "8845758582119845";

pub struct InstagramDownloader {
    client: reqwest::Client,
    redirect_client: reqwest::Client,
}

enum InstagramMedia {
    Single {
        url: String,
        is_video: bool,
    },
    Carousel {
        items: Vec<CarouselItem>,
    },
}

struct CarouselItem {
    url: String,
    is_video: bool,
}

struct GqlParams {
    csrf_token: String,
    device_id: String,
    machine_id: String,
    lsd_token: String,
    app_id: String,
    haste_session: String,
    hsi: String,
    rollout_hash: String,
    spin_r: String,
    spin_b: String,
    spin_t: String,
    comet_req: String,
    jazoest: String,
    bloks_version_id: String,
}

impl InstagramDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        let redirect_client = reqwest::Client::builder()
            .user_agent("curl/7.88.1")
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        Self {
            client,
            redirect_client,
        }
    }

    fn extract_post_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        match segments.first() {
            Some(&"p") | Some(&"reel") | Some(&"tv") => {
                segments.get(1).map(|s| s.to_string())
            }
            _ => None,
        }
    }

    fn extract_share_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        if segments.first() == Some(&"share") {
            return segments.get(1).map(|s| s.to_string());
        }

        None
    }

    fn is_story_url(url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            let path = parsed.path().to_lowercase();
            return path.starts_with("/stories/");
        }
        false
    }

    async fn resolve_share_link(&self, share_id: &str) -> anyhow::Result<String> {
        let url = format!("https://www.instagram.com/share/{}/", share_id);

        let response = self.redirect_client.get(&url).send().await?;
        let final_url = response.url().to_string();

        if final_url.contains("/share/") || final_url == url {
            return Err(anyhow!("Não foi possível resolver o share link"));
        }

        Ok(final_url)
    }

    fn regex_extract(pattern: &str, text: &str) -> Option<String> {
        let re = Regex::new(pattern).ok()?;
        re.captures(text)?.get(1).map(|m| m.as_str().to_string())
    }

    fn extract_object_entry(name: &str, html: &str) -> Option<serde_json::Value> {
        let pattern = format!(r#"\["{}",.*?,(\{{.*?\}}),\d+\]"#, regex::escape(name));
        let re = Regex::new(&pattern).ok()?;
        let json_str = re.captures(html)?.get(1)?.as_str();
        serde_json::from_str(json_str).ok()
    }

    fn extract_number_from_query(name: &str, html: &str) -> Option<String> {
        let pattern = format!(r"{}=(\d+)", regex::escape(name));
        let re = Regex::new(&pattern).ok()?;
        re.captures(html)?.get(1).map(|m| m.as_str().to_string())
    }

    fn random_base64url(len: usize) -> String {
        let bytes: Vec<u8> = (0..len).map(|_| rand::rng().random::<u8>()).collect();
        base64_url_encode(&bytes)
    }

    fn random_alpha_string(len: usize) -> String {
        let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz".chars().collect();
        (0..len)
            .map(|_| chars[rand::rng().random_range(0..chars.len())])
            .collect()
    }

    async fn get_gql_params(&self, post_id: &str) -> anyhow::Result<GqlParams> {
        let url = format!("https://www.instagram.com/p/{}/", post_id);

        let response = self
            .client
            .get(&url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "en-GB,en;q=0.9")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .header("Sec-Fetch-User", "?1")
            .send()
            .await?;

        let html = response.text().await?;

        let csrf = Self::extract_object_entry("InstagramSecurityConfig", &html)
            .and_then(|v| v.get("csrf_token").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();

        let polaris = Self::extract_object_entry("PolarisSiteData", &html);
        let device_id = polaris
            .as_ref()
            .and_then(|v| v.get("device_id").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();
        let machine_id = polaris
            .as_ref()
            .and_then(|v| v.get("machine_id").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();

        let site_data = Self::extract_object_entry("SiteData", &html);
        let haste_session = site_data
            .as_ref()
            .and_then(|v| v.get("haste_session").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "20126.HYP:instagram_web_pkg.2.1...0".to_string());
        let hsi = site_data
            .as_ref()
            .and_then(|v| v.get("hsi").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "7436540909012459023".to_string());
        let spin_r = site_data
            .as_ref()
            .and_then(|v| v.get("__spin_r").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "1019933358".to_string());
        let spin_b = site_data
            .as_ref()
            .and_then(|v| v.get("__spin_b").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "trunk".to_string());
        let spin_t = site_data
            .as_ref()
            .and_then(|v| {
                v.get("__spin_t")
                    .and_then(|t| t.as_str().or_else(|| t.as_u64().map(|_| "")).map(|_| ()))
            })
            .map(|_| {
                site_data
                    .as_ref()
                    .and_then(|v| {
                        v.get("__spin_t").and_then(|t| {
                            t.as_str().map(|s| s.to_string()).or_else(|| {
                                t.as_u64().map(|n| n.to_string())
                            })
                        })
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_else(|| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                now.to_string()
            });

        let web_config = Self::extract_object_entry("DGWWebConfig", &html);
        let app_id = web_config
            .as_ref()
            .and_then(|v| {
                v.get("appId")
                    .and_then(|t| t.as_str().map(|s| s.to_string()).or_else(|| t.as_u64().map(|n| n.to_string())))
            })
            .unwrap_or_else(|| IG_APP_ID.to_string());

        let lsd = Self::extract_object_entry("LSD", &html)
            .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| Self::random_base64url(8));

        let bloks_version_id = Self::extract_object_entry("WebBloksVersioningID", &html)
            .and_then(|v| v.get("versioningID").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();

        let push_info = Self::extract_object_entry("InstagramWebPushInfo", &html);
        let rollout_hash = push_info
            .as_ref()
            .and_then(|v| v.get("rollout_hash").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "1019933358".to_string());

        let comet_req = Self::extract_number_from_query("__comet_req", &html)
            .unwrap_or_else(|| "7".to_string());

        let jazoest = Self::extract_number_from_query("jazoest", &html)
            .unwrap_or_else(|| {
                let val: u32 = rand::rng().random_range(1000..10000);
                val.to_string()
            });

        Ok(GqlParams {
            csrf_token: csrf,
            device_id,
            machine_id,
            lsd_token: lsd,
            app_id,
            haste_session,
            hsi,
            rollout_hash,
            spin_r,
            spin_b,
            spin_t,
            comet_req,
            jazoest,
            bloks_version_id,
        })
    }

    async fn request_gql(
        &self,
        post_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let params = self.get_gql_params(post_id).await?;

        let anon_cookie = [
            if !params.csrf_token.is_empty() {
                Some(format!("csrftoken={}", params.csrf_token))
            } else {
                None
            },
            if !params.device_id.is_empty() {
                Some(format!("ig_did={}", params.device_id))
            } else {
                None
            },
            Some("wd=1280x720".to_string()),
            Some("dpr=2".to_string()),
            if !params.machine_id.is_empty() {
                Some(format!("mid={}", params.machine_id))
            } else {
                None
            },
            Some("ig_nrcb=1".to_string()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("; ");

        let s_val = format!("::{}", Self::random_alpha_string(6));
        let dyn_val = Self::random_base64url(154);
        let csr_val = Self::random_base64url(154);

        let variables = serde_json::json!({
            "shortcode": post_id,
            "fetch_tagged_user_count": null,
            "hoisted_comment_id": null,
            "hoisted_reply_id": null
        });

        let body = format!(
            "__d=www&__a=1&__s={}&__hs={}&__req=b&__ccg=EXCELLENT&__rev={}&__hsi={}&__dyn={}&__csr={}&__user=0&__comet_req={}&av=0&dpr=2&lsd={}&jazoest={}&__spin_r={}&__spin_b={}&__spin_t={}&fb_api_caller_class=RelayModern&fb_api_req_friendly_name=PolarisPostActionLoadPostQueryQuery&variables={}&server_timestamps=true&doc_id={}",
            urlencoding::encode(&s_val),
            urlencoding::encode(&params.haste_session),
            urlencoding::encode(&params.rollout_hash),
            urlencoding::encode(&params.hsi),
            urlencoding::encode(&dyn_val),
            urlencoding::encode(&csr_val),
            urlencoding::encode(&params.comet_req),
            urlencoding::encode(&params.lsd_token),
            urlencoding::encode(&params.jazoest),
            urlencoding::encode(&params.spin_r),
            urlencoding::encode(&params.spin_b),
            urlencoding::encode(&params.spin_t),
            urlencoding::encode(&variables.to_string()),
            GQL_DOC_ID,
        );

        let response = self
            .client
            .post("https://www.instagram.com/graphql/query")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "en-GB,en;q=0.9")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "same-origin")
            .header("x-ig-app-id", &params.app_id)
            .header("X-FB-LSD", &params.lsd_token)
            .header("X-CSRFToken", &params.csrf_token)
            .header("X-FB-Friendly-Name", "PolarisPostActionLoadPostQueryQuery")
            .header("x-asbd-id", "129477")
            .header("X-Bloks-Version-Id", &params.bloks_version_id)
            .header("Cookie", &anon_cookie)
            .body(body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("Instagram GQL retornou HTTP {}", response.status()));
        }

        let json: serde_json::Value = response.json().await?;

        let data = json
            .get("data")
            .ok_or_else(|| anyhow!("Resposta GQL sem data"))?;

        let media = data
            .get("xdt_shortcode_media")
            .or_else(|| data.get("shortcode_media"));

        match media {
            Some(m) if !m.is_null() => Ok(m.clone()),
            _ => Err(anyhow!("Post não encontrado via GQL")),
        }
    }

    async fn request_embed(
        &self,
        post_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!(
            "https://www.instagram.com/p/{}/embed/captioned/",
            post_id
        );

        let response = self
            .client
            .get(&url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "en-GB,en;q=0.9")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .send()
            .await?;

        let html = response.text().await?;

        if let Some(json_str) = Self::regex_extract(
            r#""init",\[\],\[(.*?)\]\],"#,
            &html,
        ) {
            if let Ok(embed_data) = serde_json::from_str::<serde_json::Value>(&json_str) {
                if let Some(context_json) = embed_data
                    .get("contextJSON")
                    .and_then(|v| v.as_str())
                {
                    let context: serde_json::Value = serde_json::from_str(context_json)?;
                    return Ok(context);
                }
            }
        }

        if let Some(json_str) = Self::regex_extract(
            r#"window\.__additionalDataLoaded\('extra',\s*(\{.*?\})\s*\)"#,
            &html,
        ) {
            let data: serde_json::Value = serde_json::from_str(&json_str)?;
            return Ok(data);
        }

        Err(anyhow!("Não foi possível extrair dados do embed"))
    }

    fn extract_media_from_gql(data: &serde_json::Value) -> anyhow::Result<InstagramMedia> {
        let sidecar = data.get("edge_sidecar_to_children");

        if let Some(sidecar) = sidecar {
            if let Some(edges) = sidecar.get("edges").and_then(|v| v.as_array()) {
                let items: Vec<CarouselItem> = edges
                    .iter()
                    .filter_map(|edge| {
                        let node = edge.get("node")?;
                        let is_video = node
                            .get("is_video")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        let url = if is_video {
                            node.get("video_url").and_then(|v| v.as_str())
                        } else {
                            node.get("display_url").and_then(|v| v.as_str())
                        }?;

                        Some(CarouselItem {
                            url: url.to_string(),
                            is_video,
                        })
                    })
                    .collect();

                if !items.is_empty() {
                    return Ok(InstagramMedia::Carousel { items });
                }
            }
        }

        if let Some(video_url) = data.get("video_url").and_then(|v| v.as_str()) {
            return Ok(InstagramMedia::Single {
                url: video_url.to_string(),
                is_video: true,
            });
        }

        if let Some(display_url) = data.get("display_url").and_then(|v| v.as_str()) {
            return Ok(InstagramMedia::Single {
                url: display_url.to_string(),
                is_video: false,
            });
        }

        Err(anyhow!("Nenhuma mídia encontrada no post"))
    }

    fn instagram_headers() -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::REFERER, "https://www.instagram.com/".parse().unwrap());
        headers.insert(reqwest::header::ORIGIN, "https://www.instagram.com".parse().unwrap());
        headers
    }

    fn extract_media_from_embed(data: &serde_json::Value) -> anyhow::Result<InstagramMedia> {
        if let Some(video_url) = data
            .get("gql_data")
            .and_then(|g| {
                g.get("shortcode_media")
                    .or_else(|| g.get("xdt_shortcode_media"))
            })
        {
            return Self::extract_media_from_gql(video_url);
        }

        if let Some(video_url) = data.get("video_url").and_then(|v| v.as_str()) {
            return Ok(InstagramMedia::Single {
                url: video_url.to_string(),
                is_video: true,
            });
        }

        if let Some(display_url) = data
            .get("media")
            .and_then(|m| m.get("display_url"))
            .or_else(|| data.get("display_url"))
            .and_then(|v| v.as_str())
        {
            return Ok(InstagramMedia::Single {
                url: display_url.to_string(),
                is_video: false,
            });
        }

        Err(anyhow!("Nenhuma mídia encontrada no embed"))
    }
}

fn base64_url_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[async_trait]
impl PlatformDownloader for InstagramDownloader {
    fn name(&self) -> &str {
        "instagram"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "instagram.com"
                    || host.ends_with(".instagram.com")
                    || host == "ddinstagram.com"
                    || host.ends_with(".ddinstagram.com");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        if Self::is_story_url(url) {
            return Err(anyhow!("Instagram Stories requerem login. Esta funcionalidade será adicionada em breve."));
        }

        let post_id = if let Some(share_id) = Self::extract_share_id(url) {
            let resolved = self.resolve_share_link(&share_id).await?;
            Self::extract_post_id(&resolved)
                .ok_or_else(|| anyhow!("Não foi possível extrair o ID do post"))?
        } else {
            Self::extract_post_id(url)
                .ok_or_else(|| anyhow!("Não foi possível extrair o ID do post"))?
        };

        let filename_base = format!("instagram_{}", post_id);

        let media = match self.request_gql(&post_id).await {
            Ok(data) => Self::extract_media_from_gql(&data),
            Err(_gql_err) => {
                match self.request_embed(&post_id).await {
                    Ok(data) => Self::extract_media_from_embed(&data),
                    Err(_embed_err) => {
                        Err(anyhow!("Post não encontrado ou privado"))
                    }
                }
            }
        }?;

        match media {
            InstagramMedia::Single { url, is_video } => {
                let (media_type, format) = if is_video {
                    (MediaType::Video, "mp4")
                } else {
                    (MediaType::Photo, "jpg")
                };

                Ok(MediaInfo {
                    title: filename_base,
                    author: String::new(),
                    platform: "instagram".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
                    available_qualities: vec![VideoQuality {
                        label: "original".to_string(),
                        width: 0,
                        height: 0,
                        url,
                        format: format.to_string(),
                    }],
                    media_type,
                    file_size_bytes: None,
                })
            }
            InstagramMedia::Carousel { items } => {
                let qualities: Vec<VideoQuality> = items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let format = if item.is_video { "mp4" } else { "jpg" };
                        VideoQuality {
                            label: format!("media_{}", i + 1),
                            width: 0,
                            height: 0,
                            url: item.url.clone(),
                            format: format.to_string(),
                        }
                    })
                    .collect();

                Ok(MediaInfo {
                    title: filename_base,
                    author: String::new(),
                    platform: "instagram".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
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
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let count = info.available_qualities.len();

        if count == 1 {
            let quality = info.available_qualities.first().unwrap();
            let filename = format!(
                "{}.{}",
                sanitize_filename::sanitize(&info.title),
                quality.format
            );
            let output = opts.output_dir.join(&filename);

            let headers = if quality.format == "mp4" {
                Some(Self::instagram_headers())
            } else {
                None
            };

            let bytes = download_direct_with_headers(
                &self.client,
                &quality.url,
                &output,
                progress,
                headers,
                None,
            )
            .await?;

            return Ok(DownloadResult {
                file_path: output,
                file_size_bytes: bytes,
                duration_seconds: 0.0,
            });
        }

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

            let headers = if quality.format == "mp4" {
                Some(Self::instagram_headers())
            } else {
                None
            };

            let bytes = download_direct_with_headers(
                &self.client,
                &quality.url,
                &output,
                tx,
                headers,
                None,
            )
            .await?;

            total_bytes += bytes;
            last_path = output;

            let percent = ((i + 1) as f64 / count as f64) * 100.0;
            let _ = progress.send(percent).await;
        }

        Ok(DownloadResult {
            file_path: last_path,
            file_size_bytes: total_bytes,
            duration_seconds: 0.0,
        })
    }
}
