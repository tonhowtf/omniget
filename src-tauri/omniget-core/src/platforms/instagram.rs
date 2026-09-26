use crate::models::progress::ProgressUpdate;
use anyhow::anyhow;
use async_trait::async_trait;
use rand::RngExt;
use regex::Regex;
use tokio::sync::mpsc;

use crate::core::direct_downloader::download_direct_with_headers;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const IG_APP_ID: &str = "936619743392459";
const GQL_DOC_ID: &str = "8845758582119845";

const MOBILE_UA: &str = "Instagram 275.0.0.27.98 Android (33/13; 280dpi; 720x1423; Xiaomi; Redmi 7; onclite; qcom; en_US; 458229237)";
const SHORTCODE_ALPHABET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub struct InstagramDownloader {
    client: reqwest::Client,
    redirect_client: reqwest::Client,
    /// Extension cookies for instagram.com were loaded into `client`. The
    /// mobile API answers `login_required` (HTTP 403) to anonymous calls even
    /// for public posts (measured 26/09), so it is only tried with a session.
    has_session: bool,
}

/// What the /embed/captioned/ page said about a post.
enum EmbedOutcome {
    /// contextJSON or additionalData with the post.
    Data(serde_json::Value),
    /// The embed rendered its "broken media" card (`contextJSON: null`):
    /// Instagram refuses to embed the post without a login.
    Broken,
    /// Neither marker: an unknown shell; says nothing about the post.
    Missing,
}

/// What the logged-out /p/{id}/ page says about the post.
#[derive(Debug, PartialEq)]
enum PostPage {
    /// Login form or the SSR error page with no og:* tags: Instagram hides
    /// the post from anonymous visitors (private, age-gated or removed).
    Hidden,
    /// A single-video post whose og:video is on the page.
    Video(String),
    /// A post page with nothing we can take directly.
    Visible,
}

/// Metadata requests only; a stalled page must not hold the cascade for the
/// client's 120 s download timeout.
const META_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

enum InstagramMedia {
    Single { url: String, is_video: bool },
    Carousel { items: Vec<CarouselItem> },
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

impl Default for InstagramDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl InstagramDownloader {
    pub fn new() -> Self {
        let mut builder = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15));

        let mut has_session = false;
        if let Some(jar) =
            crate::core::cookie_parser::load_extension_cookies_for_domain("instagram.com")
        {
            builder = builder.cookie_provider(jar);
            has_session = true;
        }

        let client = builder.build().unwrap_or_default();

        let redirect_client =
            crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
                .user_agent("curl/7.88.1")
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default();

        Self {
            client,
            redirect_client,
            has_session,
        }
    }

    /// Numeric media id (pk) of a shortcode: base-64 digits in Instagram's
    /// alphabet (gallery-dl instagram.py:1323 id_from_shortcode, yt-dlp
    /// instagram.py:37 _id_to_pk). Private shortcodes carry a 28-char suffix.
    fn media_id_from_shortcode(shortcode: &str) -> Option<u64> {
        let code = if shortcode.len() > 28 {
            &shortcode[..shortcode.len() - 28]
        } else {
            shortcode
        };
        if code.is_empty() {
            return None;
        }
        let mut n: u64 = 0;
        for b in code.bytes() {
            let digit = SHORTCODE_ALPHABET.iter().position(|c| *c == b)? as u64;
            n = n.checked_mul(64)?.checked_add(digit)?;
        }
        Some(n)
    }

    fn extract_post_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        match segments.first() {
            Some(&"p") | Some(&"reel") | Some(&"reels") | Some(&"tv") => {
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

    fn is_reel_url(url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            let path = parsed.path().to_lowercase();
            return path.starts_with("/reel/") || path.starts_with("/reels/");
        }
        false
    }

    async fn resolve_share_link(&self, share_id: &str) -> anyhow::Result<String> {
        let url = format!("https://www.instagram.com/share/{}/", share_id);

        let response = self.redirect_client.get(&url).send().await?;
        let final_url = response.url().to_string();

        if final_url.contains("/share/") || final_url == url {
            return Err(anyhow!("Could not resolve share link"));
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

    fn is_login_redirect(html: &str) -> bool {
        let lower = html.to_lowercase();
        lower.contains("/accounts/login")
            || lower.contains("\"loginpage\"")
            || (lower.contains("\"require_login\"") && lower.contains("true"))
    }

    /// The post page is the login form. A logged-out public post page also
    /// links to /accounts/login and mentions "loginPage" (26/09, 939 KB), so
    /// the text markers only count when the GQL tokens are absent.
    fn is_login_wall(final_path: &str, html: &str) -> bool {
        if final_path.starts_with("/accounts/login") {
            return true;
        }
        let has_tokens = html.contains("\"PolarisSiteData\"") || html.contains("[\"LSD\"");
        !has_tokens && Self::is_login_redirect(html)
    }

    /// The logged-out /p/{id}/ page: (final path, html). It carries the GQL
    /// tokens and, for a visible video, og:video; for a post hidden from
    /// anonymous visitors it is Instagram's error page.
    async fn fetch_post_page(&self, post_id: &str) -> anyhow::Result<(String, String)> {
        let url = format!("https://www.instagram.com/p/{}/", post_id);

        let response = self
            .client
            .get(&url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-GB,en;q=0.9")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .header("Sec-Fetch-User", "?1")
            .timeout(META_TIMEOUT)
            .send()
            .await?;

        let final_path = response.url().path().to_string();
        let html = response.text().await?;
        Ok((final_path, html))
    }

    /// What the logged-out post page says about the post.
    fn post_page_verdict(final_path: &str, html: &str, reel_hint: bool) -> PostPage {
        if Self::is_login_wall(final_path, html) {
            return PostPage::Hidden;
        }
        // Route config of the SSR page (26/09): a visible post renders
        // "postPage"; /p/BkfuX9UB-eK (login-locked) renders "httpErrorPage"
        // with page_type MEDIA and no og:* tags at all.
        if html.contains("\"pageID\":\"httpErrorPage\"") && !html.contains("property=\"og:") {
            return PostPage::Hidden;
        }
        let og = |prop: &str| {
            Self::regex_extract(
                &format!(
                    r#"<meta property="{}" content="([^"]+)""#,
                    regex::escape(prop)
                ),
                html,
            )
            .map(|v| v.replace("&amp;", "&"))
        };
        // og:video only names the first video: trust it for single-video
        // posts (reel/tv URL, or the page's canonical is a reel), never for a
        // /p/ post that may be a carousel.
        let canonical_reel = Self::regex_extract(r#"<link rel="canonical" href="([^"]+)""#, html)
            .map(|c| c.contains("/reel/") || c.contains("/tv/"))
            .unwrap_or(false);
        if reel_hint || canonical_reel {
            if let Some(url) = og("og:video:secure_url").or_else(|| og("og:video")) {
                if url.starts_with("https://") {
                    return PostPage::Video(url);
                }
            }
        }
        PostPage::Visible
    }

    fn gql_params_from_page(final_path: &str, html: &str) -> anyhow::Result<GqlParams> {
        let html = html.to_string();
        if Self::is_login_wall(final_path, &html) {
            return Err(anyhow!(
                "Instagram redirecionou para login — post pode ser privado"
            ));
        }

        let csrf = Self::extract_object_entry("InstagramSecurityConfig", &html)
            .and_then(|v| {
                v.get("csrf_token")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();

        let polaris = Self::extract_object_entry("PolarisSiteData", &html);
        let device_id = polaris
            .as_ref()
            .and_then(|v| {
                v.get("device_id")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        let machine_id = polaris
            .as_ref()
            .and_then(|v| {
                v.get("machine_id")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();

        let site_data = Self::extract_object_entry("SiteData", &html);
        let haste_session = site_data
            .as_ref()
            .and_then(|v| {
                v.get("haste_session")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "20126.HYP:instagram_web_pkg.2.1...0".to_string());
        let hsi = site_data
            .as_ref()
            .and_then(|v| v.get("hsi").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "7436540909012459023".to_string());
        let spin_r = site_data
            .as_ref()
            .and_then(|v| {
                v.get("__spin_r")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "1019933358".to_string());
        let spin_b = site_data
            .as_ref()
            .and_then(|v| {
                v.get("__spin_b")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
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
                            t.as_str()
                                .map(|s| s.to_string())
                                .or_else(|| t.as_u64().map(|n| n.to_string()))
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
                v.get("appId").and_then(|t| {
                    t.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| t.as_u64().map(|n| n.to_string()))
                })
            })
            .unwrap_or_else(|| IG_APP_ID.to_string());

        let lsd = Self::extract_object_entry("LSD", &html)
            .and_then(|v| {
                v.get("token")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| Self::random_base64url(8));

        let bloks_version_id = Self::extract_object_entry("WebBloksVersioningID", &html)
            .and_then(|v| {
                v.get("versioningID")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();

        let push_info = Self::extract_object_entry("InstagramWebPushInfo", &html);
        let rollout_hash = push_info
            .as_ref()
            .and_then(|v| {
                v.get("rollout_hash")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "1019933358".to_string());

        let comet_req = Self::extract_number_from_query("__comet_req", &html)
            .unwrap_or_else(|| "7".to_string());

        let jazoest = Self::extract_number_from_query("jazoest", &html).unwrap_or_else(|| {
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

    /// `Ok(None)`: GQL answered and the post has no media for this session
    /// (`xdt_shortcode_media: null`), which is how a login-locked post looks.
    #[cfg(test)]
    async fn request_gql(&self, post_id: &str) -> anyhow::Result<Option<serde_json::Value>> {
        let (path, html) = self.fetch_post_page(post_id).await?;
        let params = Self::gql_params_from_page(&path, &html)?;
        self.request_gql_with(post_id, &params).await
    }

    async fn request_gql_with(
        &self,
        post_id: &str,
        params: &GqlParams,
    ) -> anyhow::Result<Option<serde_json::Value>> {
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
            .header("Accept", "*/*")
            .header("Accept-Language", "en-GB,en;q=0.9")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Sec-Fetch-Dest", "empty")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Site", "same-origin")
            .header("X-Requested-With", "XMLHttpRequest")
            .header("x-ig-app-id", &params.app_id)
            .header("X-FB-LSD", &params.lsd_token)
            .header("X-CSRFToken", &params.csrf_token)
            .header("X-FB-Friendly-Name", "PolarisPostActionLoadPostQueryQuery")
            .header("x-asbd-id", "129477")
            .header("X-Bloks-Version-Id", &params.bloks_version_id)
            .header("Referer", "https://www.instagram.com/")
            .header("Cookie", &anon_cookie)
            .body(body)
            .timeout(META_TIMEOUT)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("Instagram GQL retornou HTTP {}", response.status()));
        }

        let json: serde_json::Value = response.json().await?;
        Self::parse_gql_response(&json)
    }

    fn parse_gql_response(json: &serde_json::Value) -> anyhow::Result<Option<serde_json::Value>> {
        let data = json
            .get("data")
            .ok_or_else(|| anyhow!("Instagram GQL answer has no data"))?;
        let media = data
            .get("xdt_shortcode_media")
            .or_else(|| data.get("shortcode_media"));
        match media {
            Some(m) if !m.is_null() => Ok(Some(m.clone())),
            _ => Ok(None),
        }
    }

    /// cobalt instagram.js:125-136: i.instagram.com/api/v1/media/{id}/info/
    /// with the Android app headers (instagram.js:14-24).
    async fn request_mobile_api(&self, media_id: u64) -> anyhow::Result<serde_json::Value> {
        let response = self
            .client
            .get(format!(
                "https://i.instagram.com/api/v1/media/{}/info/",
                media_id
            ))
            .header("User-Agent", MOBILE_UA)
            .header("x-ig-app-locale", "en_US")
            .header("x-ig-device-locale", "en_US")
            .header("x-ig-mapped-locale", "en_US")
            .header("Accept-Language", "en-US")
            .header("x-fb-http-engine", "Liger")
            .header("x-fb-client-ip", "True")
            .header("x-fb-server-cluster", "True")
            .timeout(META_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(anyhow!("Instagram mobile API returned HTTP {}", status));
        }
        Ok(response.json().await?)
    }

    fn largest_video(versions: &serde_json::Value) -> Option<String> {
        versions
            .as_array()?
            .iter()
            .max_by_key(|v| {
                v.get("width").and_then(|w| w.as_u64()).unwrap_or(0)
                    * v.get("height").and_then(|h| h.as_u64()).unwrap_or(0)
            })
            .and_then(|v| v.get("url").and_then(|u| u.as_str()))
            .map(str::to_string)
    }

    fn first_image(item: &serde_json::Value) -> Option<String> {
        item.pointer("/image_versions2/candidates/0/url")
            .and_then(|u| u.as_str())
            .map(str::to_string)
    }

    /// cobalt instagram.js extractNewPost: items[0] of the mobile API.
    fn extract_media_from_mobile(json: &serde_json::Value) -> anyhow::Result<InstagramMedia> {
        let item = json
            .pointer("/items/0")
            .ok_or_else(|| anyhow!("Instagram mobile API answer has no items"))?;
        if let Some(carousel) = item.get("carousel_media").and_then(|c| c.as_array()) {
            let items: Vec<CarouselItem> = carousel
                .iter()
                .filter_map(|e| {
                    if let Some(url) = e.get("video_versions").and_then(Self::largest_video) {
                        return Some(CarouselItem {
                            url,
                            is_video: true,
                        });
                    }
                    Self::first_image(e).map(|url| CarouselItem {
                        url,
                        is_video: false,
                    })
                })
                .collect();
            if !items.is_empty() {
                return Ok(InstagramMedia::Carousel { items });
            }
        }
        if let Some(url) = item.get("video_versions").and_then(Self::largest_video) {
            return Ok(InstagramMedia::Single {
                url,
                is_video: true,
            });
        }
        if let Some(url) = Self::first_image(item) {
            return Ok(InstagramMedia::Single {
                url,
                is_video: false,
            });
        }
        Err(anyhow!("No media found in mobile API item"))
    }

    fn login_locked_error(post_id: &str) -> anyhow::Error {
        anyhow!(
            "Instagram sent an empty media response for {}: this post requires login. Import cookies for this site in Settings → Cookies, then retry.",
            post_id
        )
    }

    /// Strong evidence only: the embed rendered its broken-media card AND the
    /// post page itself is the login form or the error page. A broken embed
    /// alone is not enough (owners can disable embedding).
    fn hidden_without_login(embed_broken: bool, page: &PostPage, has_session: bool) -> bool {
        embed_broken && !has_session && *page == PostPage::Hidden
    }

    fn parse_embed_html(html: &str) -> EmbedOutcome {
        if let Some(json_str) = Self::regex_extract(r#""init",\[\],\[(.*?)\]\],"#, html) {
            if let Ok(embed_data) = serde_json::from_str::<serde_json::Value>(&json_str) {
                match embed_data.get("contextJSON") {
                    Some(serde_json::Value::String(context_json)) => {
                        if let Ok(context) = serde_json::from_str(context_json) {
                            return EmbedOutcome::Data(context);
                        }
                    }
                    Some(serde_json::Value::Null) => return EmbedOutcome::Broken,
                    _ => {}
                }
            }
        }

        if let Some(json_str) = Self::regex_extract(
            r#"window\.__additionalDataLoaded\('extra',\s*(\{.*?\})\s*\)"#,
            html,
        ) {
            if let Ok(data) = serde_json::from_str(&json_str) {
                return EmbedOutcome::Data(data);
            }
        }

        if html.contains("EmbedBrokenMedia") {
            return EmbedOutcome::Broken;
        }
        EmbedOutcome::Missing
    }

    async fn request_embed(&self, post_id: &str) -> anyhow::Result<EmbedOutcome> {
        let url = format!("https://www.instagram.com/p/{}/embed/captioned/", post_id);

        let response = self
            .client
            .get(&url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-GB,en;q=0.9")
            .header("Sec-Fetch-Dest", "iframe")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "cross-site")
            .header("Referer", "https://www.instagram.com/")
            .timeout(META_TIMEOUT)
            .send()
            .await?;

        let html = response.text().await?;
        Ok(Self::parse_embed_html(&html))
    }

    async fn fallback_ytdlp(&self, url: &str, post_id: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
        let json = crate::core::ytdlp::get_video_info(&ytdlp_path, url, &[]).await?;
        let mut info =
            crate::platforms::generic_ytdlp::GenericYtdlpDownloader::parse_video_info(&json)?;

        info.title = format!("instagram_{}", post_id);
        info.platform = "instagram".to_string();

        let post_url = format!("https://www.instagram.com/p/{}/", post_id);
        for q in &mut info.available_qualities {
            q.format = "ytdlp".to_string();
            q.url = post_url.clone();
        }

        Ok(info)
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

        Err(anyhow!("No media found in post"))
    }

    fn instagram_headers() -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::REFERER,
            "https://www.instagram.com/".parse().unwrap(),
        );
        headers.insert(
            reqwest::header::ORIGIN,
            "https://www.instagram.com".parse().unwrap(),
        );
        headers
    }

    fn post_url_from_title(title: &str) -> Option<String> {
        let post_id = title.strip_prefix("instagram_")?;
        if post_id.is_empty() {
            return None;
        }
        Some(format!("https://www.instagram.com/p/{}/", post_id))
    }

    fn resolve_fallback_post_url(info: &MediaInfo, opts: &DownloadOptions) -> Option<String> {
        if let Some(url) = Self::post_url_from_title(&info.title) {
            return Some(url);
        }
        if let Some(page_url) = opts.page_url.as_deref() {
            if Self::extract_post_id(page_url).is_some() {
                return Some(page_url.to_string());
            }
            if let Some(share_id) = Self::extract_share_id(page_url) {
                return Some(format!("https://www.instagram.com/share/{}/", share_id));
            }
        }
        None
    }

    fn is_html_block_error(err: &anyhow::Error) -> bool {
        let msg = err.to_string();
        msg.contains("HTML instead of media")
    }

    async fn ytdlp_download_post(
        post_url: &str,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
        crate::core::ytdlp::download_video(
            &ytdlp_path,
            post_url,
            &opts.output_dir,
            None,
            progress,
            opts.download_mode.as_deref(),
            opts.format_id.as_deref(),
            opts.filename_template.as_deref(),
            opts.referer
                .as_deref()
                .or(Some("https://www.instagram.com/")),
            opts.cancel_token.clone(),
            None,
            opts.concurrent_fragments,
            false,
            &[],
            opts.audio_format.as_deref(),
        )
        .await
    }

    fn extract_media_from_embed(data: &serde_json::Value) -> anyhow::Result<InstagramMedia> {
        if let Some(video_url) = data.get("gql_data").and_then(|g| {
            g.get("shortcode_media")
                .or_else(|| g.get("xdt_shortcode_media"))
        }) {
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

        Err(anyhow!("No media found in embed"))
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
            return Err(anyhow!(
                "Instagram Stories are not supported. Only public posts, reels and carousels."
            ));
        }

        let post_id = if let Some(share_id) = Self::extract_share_id(url) {
            let resolved = self.resolve_share_link(&share_id).await?;
            Self::extract_post_id(&resolved).ok_or_else(|| anyhow!("Could not extract post ID"))?
        } else {
            Self::extract_post_id(url).ok_or_else(|| anyhow!("Could not extract post ID"))?
        };

        let filename_base = format!("instagram_{}", post_id);

        // cobalt getPost order (instagram.js:416-451): mobile API, embed, GQL,
        // each only while the previous one found nothing; yt-dlp last. Every
        // native failure is kept for the operator log.
        let is_reel = Self::is_reel_url(url);
        // A Reel whose metadata only has display_url got the cover image,
        // not the video: keep looking.
        let usable = |m: &InstagramMedia| {
            !(is_reel
                && matches!(
                    m,
                    InstagramMedia::Single {
                        is_video: false,
                        ..
                    }
                ))
        };
        let mut attempts: Vec<String> = Vec::new();
        let mut media: Option<InstagramMedia> = None;

        if self.has_session {
            match Self::media_id_from_shortcode(&post_id) {
                Some(media_id) => match self.request_mobile_api(media_id).await {
                    Ok(json) => match Self::extract_media_from_mobile(&json) {
                        Ok(m) if usable(&m) => media = Some(m),
                        Ok(_) => attempts.push("mobile api: reel cover only".into()),
                        Err(e) => attempts.push(format!("mobile api: {e}")),
                    },
                    Err(e) => attempts.push(format!("mobile api: {e}")),
                },
                None => attempts.push("mobile api: shortcode is not decodable".into()),
            }
        }

        let mut embed_broken = false;
        if media.is_none() {
            match self.request_embed(&post_id).await {
                Ok(EmbedOutcome::Data(data)) => match Self::extract_media_from_embed(&data) {
                    Ok(m) if usable(&m) => media = Some(m),
                    Ok(_) => attempts.push("embed: reel cover only".into()),
                    Err(e) => attempts.push(format!("embed: {e}")),
                },
                Ok(EmbedOutcome::Broken) => {
                    embed_broken = true;
                    attempts.push("embed: broken media card (contextJSON null)".into());
                }
                Ok(EmbedOutcome::Missing) => {
                    attempts.push("embed: page without contextJSON/additionalData".into())
                }
                Err(e) => attempts.push(format!("embed: {e}")),
            }
        }

        let mut gql_empty = false;
        let mut page_video: Option<String> = None;
        if media.is_none() {
            let reel_hint = is_reel || url.to_ascii_lowercase().contains("/tv/");
            match self.fetch_post_page(&post_id).await {
                Ok((path, html)) => {
                    let verdict = Self::post_page_verdict(&path, &html, reel_hint);
                    // Embed card "broken" + the post page is Instagram's error
                    // page: hidden from anonymous visitors. yt-dlp reaches the
                    // same "empty media response ... login" verdict ~12 s
                    // later (bench 26/09, /tv/BkfuX9UB-eK).
                    if Self::hidden_without_login(embed_broken, &verdict, self.has_session) {
                        tracing::warn!(
                            "[instagram] {} hidden from anonymous visitors: {}; post page is the error page",
                            post_id,
                            attempts.join("; ")
                        );
                        return Err(Self::login_locked_error(&post_id));
                    }
                    if let PostPage::Video(v) = verdict {
                        page_video = Some(v);
                    }
                    match Self::gql_params_from_page(&path, &html) {
                        Ok(params) => match self.request_gql_with(&post_id, &params).await {
                            Ok(Some(data)) => match Self::extract_media_from_gql(&data) {
                                Ok(m) if usable(&m) => media = Some(m),
                                Ok(_) => attempts.push("gql: reel cover only".into()),
                                Err(e) => attempts.push(format!("gql: {e}")),
                            },
                            Ok(None) => {
                                gql_empty = true;
                                attempts.push("gql: shortcode_media null".into());
                            }
                            Err(e) => attempts.push(format!("gql: {e}")),
                        },
                        Err(e) => attempts.push(format!("gql: {e}")),
                    }
                }
                Err(e) => attempts.push(format!("post page: {e}")),
            }
        }

        // Anonymous GQL answers 401 even for public posts (26/09); the
        // logged-out page of a single-video post still names its mp4.
        if media.is_none() {
            if let Some(v) = page_video {
                attempts.push("post page: og:video".into());
                media = Some(InstagramMedia::Single {
                    url: v,
                    is_video: true,
                });
            }
        }

        let media = match media {
            Some(m) => m,
            None => {
                tracing::warn!(
                    "[instagram] native paths found no media for {}: {}",
                    post_id,
                    attempts.join("; ")
                );
                // Both web paths answered and both said "nothing without a
                // login": the same verdict yt-dlp reaches ("empty media
                // response") after ~12 s more (bench 26/09, /tv/BkfuX9UB-eK).
                if embed_broken && gql_empty && !self.has_session {
                    return Err(Self::login_locked_error(&post_id));
                }
                return self.fallback_ytdlp(url, &post_id).await;
            }
        };

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
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let count = info.available_qualities.len();

        if count == 1 {
            let quality = info.available_qualities.first().unwrap();

            if quality.format == "ytdlp" {
                return Self::ytdlp_download_post(&quality.url, opts, progress).await;
            }

            let filename = format!(
                "{}.{}",
                sanitize_filename::sanitize(&info.title),
                quality.format
            );
            let output = opts.output_dir.join(&filename);

            let mut hdr_map = Self::instagram_headers();
            crate::core::http_client::inject_ua_header(&mut hdr_map, opts.user_agent.as_deref());
            let headers = Some(hdr_map);

            match download_direct_with_headers(
                &self.client,
                &quality.url,
                &output,
                progress.clone(),
                headers,
                Some(&opts.cancel_token),
            )
            .await
            {
                Ok(bytes) => {
                    return Ok(DownloadResult {
                        file_path: output,
                        file_size_bytes: bytes,
                        duration_seconds: 0.0,
                        torrent_id: None,
                    });
                }
                Err(e) => {
                    if Self::is_html_block_error(&e) {
                        if let Some(post_url) = Self::resolve_fallback_post_url(info, opts) {
                            tracing::warn!(
                                "[instagram] direct CDN fetch returned HTML for {}; falling back to yt-dlp",
                                post_url
                            );
                            return Self::ytdlp_download_post(&post_url, opts, progress).await;
                        }
                    }
                    return Err(e);
                }
            }
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

            let mut hdr_map = Self::instagram_headers();
            crate::core::http_client::inject_ua_header(&mut hdr_map, opts.user_agent.as_deref());
            let headers = Some(hdr_map);

            match download_direct_with_headers(
                &self.client,
                &quality.url,
                &output,
                tx,
                headers,
                Some(&opts.cancel_token),
            )
            .await
            {
                Ok(bytes) => {
                    total_bytes += bytes;
                    last_path = output;

                    let percent = ((i + 1) as f64 / count as f64) * 100.0;
                    let _ = progress.send(ProgressUpdate::percent(percent)).await;
                }
                Err(e) => {
                    if Self::is_html_block_error(&e) {
                        if let Some(post_url) = Self::resolve_fallback_post_url(info, opts) {
                            tracing::warn!(
                                "[instagram] carousel item {}/{} returned HTML for {}; falling back to yt-dlp for full post",
                                i + 1,
                                count,
                                post_url
                            );
                            return Self::ytdlp_download_post(&post_url, opts, progress).await;
                        }
                    }
                    return Err(e);
                }
            }
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
    use std::path::PathBuf;
    use tokio_util::sync::CancellationToken;

    fn make_opts(page_url: Option<&str>) -> DownloadOptions {
        DownloadOptions {
            quality: None,
            output_dir: PathBuf::from("."),
            filename_template: None,
            download_subtitles: false,
            include_auto_subtitles: false,
            download_mode: None,
            audio_format: None,
            format_id: None,
            referer: None,
            extra_headers: None,
            page_url: page_url.map(String::from),
            user_agent: None,
            cancel_token: CancellationToken::new(),
            concurrent_fragments: 1,
            ytdlp_path: None,
            torrent_listen_port: None,
            torrent_id_slot: None,
            custom_ytdlp_args: None,
            torrent_files: None,
            torrent_auto_trackers: false,
            torrent_upnp: false,
        }
    }

    fn make_info(title: &str) -> MediaInfo {
        MediaInfo {
            title: title.to_string(),
            author: String::new(),
            platform: "instagram".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: vec![],
            media_type: MediaType::Video,
            file_size_bytes: None,
        }
    }

    #[test]
    fn is_reel_url_accepts_reel_paths() {
        assert!(InstagramDownloader::is_reel_url(
            "https://www.instagram.com/reel/Dbi_MeVOphM/"
        ));
        assert!(InstagramDownloader::is_reel_url(
            "https://www.instagram.com/reels/Dbi_MeVOphM/"
        ));
    }

    #[test]
    fn is_reel_url_rejects_regular_posts() {
        assert!(!InstagramDownloader::is_reel_url(
            "https://www.instagram.com/p/Dbi_MeVOphM/"
        ));
    }

    #[test]
    fn is_reel_url_handles_case_insensitive_paths() {
        assert!(InstagramDownloader::is_reel_url(
            "https://www.instagram.com/REEL/Dbi_MeVOphM/"
        ));
    }

    #[test]
    fn post_url_from_title_accepts_prefixed_title() {
        assert_eq!(
            InstagramDownloader::post_url_from_title("instagram_ABC123"),
            Some("https://www.instagram.com/p/ABC123/".to_string())
        );
    }

    #[test]
    fn post_url_from_title_rejects_yt_dlp_default_title() {
        assert_eq!(
            InstagramDownloader::post_url_from_title("Video by savadgee"),
            None
        );
    }

    #[test]
    fn post_url_from_title_rejects_empty_post_id() {
        assert_eq!(InstagramDownloader::post_url_from_title("instagram_"), None);
    }

    #[test]
    fn resolve_fallback_prefers_title_prefix() {
        let info = make_info("instagram_ABC123");
        let opts = make_opts(Some("https://www.instagram.com/p/XYZ999/"));
        assert_eq!(
            InstagramDownloader::resolve_fallback_post_url(&info, &opts),
            Some("https://www.instagram.com/p/ABC123/".to_string())
        );
    }

    #[test]
    fn resolve_fallback_uses_page_url_for_yt_dlp_titles() {
        let info = make_info("Video by savadgee");
        let opts = make_opts(Some("https://www.instagram.com/reel/XYZ999/"));
        assert_eq!(
            InstagramDownloader::resolve_fallback_post_url(&info, &opts),
            Some("https://www.instagram.com/reel/XYZ999/".to_string())
        );
    }

    #[test]
    fn resolve_fallback_uses_page_url_share_link() {
        let info = make_info("Video by savadgee");
        let opts = make_opts(Some("https://www.instagram.com/share/ABCxyz/"));
        assert_eq!(
            InstagramDownloader::resolve_fallback_post_url(&info, &opts),
            Some("https://www.instagram.com/share/ABCxyz/".to_string())
        );
    }

    #[test]
    fn resolve_fallback_returns_none_when_page_url_not_instagram() {
        let info = make_info("Video by savadgee");
        let opts = make_opts(Some("https://example.com/foo"));
        assert_eq!(
            InstagramDownloader::resolve_fallback_post_url(&info, &opts),
            None
        );
    }

    #[test]
    fn resolve_fallback_returns_none_when_no_sources_available() {
        let info = make_info("Video by savadgee");
        let opts = make_opts(None);
        assert_eq!(
            InstagramDownloader::resolve_fallback_post_url(&info, &opts),
            None
        );
    }

    #[test]
    fn is_html_block_error_matches_direct_downloader_message() {
        let err = anyhow!("Server returned HTML instead of media — URL may have expired");
        assert!(InstagramDownloader::is_html_block_error(&err));
    }

    #[test]
    fn is_html_block_error_rejects_unrelated_error() {
        let err = anyhow!("HTTP 404 downloading url");
        assert!(!InstagramDownloader::is_html_block_error(&err));
    }

    // Bench 26/09: a logged-out public post page (/p/aye83DjauH/, 939 KB)
    // links to /accounts/login and has "loginPage" in its bootstrap, but also
    // carries the GQL tokens. The old check called it a login wall, so the
    // GQL path was never tried.
    #[test]
    fn public_post_page_with_login_links_is_not_a_login_wall() {
        let html = r#"<a href="/accounts/login/?next=%2Fp%2Faye83DjauH%2F">Log in</a>
            ["PolarisSiteData",[],{"device_id":"D1","machine_id":"M1"},1]
            ["LSD",[],{"token":"AVqxyz"},2]"loginPage":{}"#;
        assert!(!InstagramDownloader::is_login_wall("/p/aye83DjauH/", html));
    }

    #[test]
    fn login_redirect_is_a_login_wall() {
        assert!(InstagramDownloader::is_login_wall(
            "/accounts/login/",
            "<html></html>"
        ));
        // No GQL tokens and a login page marker: the page is the login form.
        assert!(InstagramDownloader::is_login_wall(
            "/p/X/",
            r#"<div>"loginPage"</div><a href="/accounts/login/">"#
        ));
    }

    #[test]
    fn media_id_from_shortcode_matches_gallery_dl_and_yt_dlp() {
        // util.bdecode with the Instagram alphabet (gallery-dl instagram.py:1323).
        assert_eq!(
            InstagramDownloader::media_id_from_shortcode("aye83DjauH"),
            Some(482584233761418119)
        );
        assert_eq!(
            InstagramDownloader::media_id_from_shortcode("BkfuX9UB-eK"),
            Some(1810369531748018058)
        );
        // Private-post shortcodes carry a 28-char suffix that is not part of the id.
        let long = format!("aye83DjauH{}", "A".repeat(28));
        assert_eq!(
            InstagramDownloader::media_id_from_shortcode(&long),
            Some(482584233761418119)
        );
        assert_eq!(InstagramDownloader::media_id_from_shortcode("bad!"), None);
    }

    // Captured 26/09 from /p/aye83DjauH/embed/captioned/ (trimmed).
    const EMBED_OK: &str = r#"<script>requireLazy([],function(){s.handle({"define":[],"require":[["PolarisEmbedSimple","init",[],[{"isRichEmbed":true,"isSidecar":false,"isGuideEmbed":false,"isProfileEmbed":false,"contextJSON":"{\"context\":{\"type\":\"GraphVideo\",\"shortcode\":\"aye83DjauH\"},\"gql_data\":{\"shortcode_media\":{\"__typename\":\"GraphVideo\",\"is_video\":true,\"display_url\":\"https://cdn/x.jpg\",\"video_url\":\"https://cdn/x.mp4\"}}}"}]],["Other","init",[],[{}]]]})})</script>"#;
    // Captured 26/09 from /p/BkfuX9UB-eK/embed/captioned/ (login-locked post).
    const EMBED_BROKEN: &str = r#"<div class="_aa4c"><div class="EmbedBrokenMedia"><div class="ebmLogo"></div></div></div><script>s.handle({"require":[["PolarisEmbedSimple","init",[],[{"isRichEmbed":false,"isSidecar":false,"isGuideEmbed":false,"isProfileEmbed":false,"contextJSON":null}]],["X","init",[],[{}]]]})</script>"#;

    #[test]
    fn embed_with_context_json_yields_video() {
        let data = match InstagramDownloader::parse_embed_html(EMBED_OK) {
            EmbedOutcome::Data(d) => d,
            _ => panic!("expected data"),
        };
        match InstagramDownloader::extract_media_from_embed(&data).unwrap() {
            InstagramMedia::Single { url, is_video } => {
                assert!(is_video);
                assert_eq!(url, "https://cdn/x.mp4");
            }
            _ => panic!("expected single"),
        }
    }

    #[test]
    fn embed_with_null_context_is_broken_not_a_parse_error() {
        assert!(matches!(
            InstagramDownloader::parse_embed_html(EMBED_BROKEN),
            EmbedOutcome::Broken
        ));
        assert!(matches!(
            InstagramDownloader::parse_embed_html("<html>shell</html>"),
            EmbedOutcome::Missing
        ));
    }

    #[test]
    fn gql_null_media_is_a_definitive_empty_answer() {
        let null = serde_json::json!({"data":{"xdt_shortcode_media":null},"status":"ok"});
        assert!(InstagramDownloader::parse_gql_response(&null)
            .unwrap()
            .is_none());
        let ok =
            serde_json::json!({"data":{"xdt_shortcode_media":{"video_url":"https://cdn/v.mp4"}}});
        assert!(InstagramDownloader::parse_gql_response(&ok)
            .unwrap()
            .is_some());
        let err = serde_json::json!({"message":"execution error","status":"fail"});
        assert!(InstagramDownloader::parse_gql_response(&err).is_err());
    }

    // Shape of i.instagram.com/api/v1/media/{id}/info/ items[0]
    // (cobalt instagram.js:400-413, extractNewPost).
    #[test]
    fn mobile_api_item_picks_largest_video_and_carousel() {
        let video = serde_json::json!({"items":[{"video_versions":[
            {"width":480,"height":854,"url":"https://cdn/small.mp4"},
            {"width":720,"height":1280,"url":"https://cdn/big.mp4"}],
            "image_versions2":{"candidates":[{"url":"https://cdn/c.jpg"}]}}]});
        match InstagramDownloader::extract_media_from_mobile(&video).unwrap() {
            InstagramMedia::Single { url, is_video } => {
                assert!(is_video);
                assert_eq!(url, "https://cdn/big.mp4");
            }
            _ => panic!("expected single"),
        }
        let carousel = serde_json::json!({"items":[{"carousel_media":[
            {"image_versions2":{"candidates":[{"url":"https://cdn/1.jpg"}]}},
            {"image_versions2":{"candidates":[{"url":"https://cdn/2.jpg"}]},
             "video_versions":[{"width":1,"height":1,"url":"https://cdn/2.mp4"}]}]}]});
        match InstagramDownloader::extract_media_from_mobile(&carousel).unwrap() {
            InstagramMedia::Carousel { items } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[1].url, "https://cdn/2.mp4");
                assert!(items[1].is_video && !items[0].is_video);
            }
            _ => panic!("expected carousel"),
        }
        let login = serde_json::json!({"message":"login_required","status":"fail"});
        assert!(InstagramDownloader::extract_media_from_mobile(&login).is_err());
    }

    // Excerpts of the logged-out /p/ pages fetched 26/09 (642 KB / 940 KB).
    const PAGE_LOCKED: &str = r#"["LSD",[],{"token":"X"},323]"PolarisSiteData"{"props":{"page_logging":{"name":"httpErrorPage","params":{}},"failure_reason":null,"restricted_age":null,"page_type":"MEDIA","show_lox_redesigned_404_page":true},"entryPoint":{"__dr":"PolarisErrorRoot.entrypoint"}},"polarisRouteConfig":{"pageID":"httpErrorPage"},"url":"\/p\/BkfuX9UB-eK\/"<title>Instagram</title>"#;
    const PAGE_PUBLIC_REEL: &str = r#"<title>Instagram</title><link rel="canonical" href="https://www.instagram.com/reel/aye83DjauH/" /><meta property="og:type" content="video" /><meta property="og:video:secure_url" content="https://instagram.frec52-1.fna.fbcdn.net/o1/v/t16/f2/m84/AQO9.mp4?_nc_cat=111&amp;_nc_sid=5e9851&amp;_nc_ht=instagram.frec52-1.fna.fbcdn.net" />["LSD",[],{"token":"X"},323]"polarisRouteConfig":{"pageID":"postPage"}"#;

    #[test]
    fn locked_post_page_is_hidden() {
        assert_eq!(
            InstagramDownloader::post_page_verdict("/p/BkfuX9UB-eK/", PAGE_LOCKED, true),
            PostPage::Hidden
        );
    }

    #[test]
    fn public_reel_page_yields_og_video_unescaped() {
        match InstagramDownloader::post_page_verdict("/p/aye83DjauH/", PAGE_PUBLIC_REEL, false) {
            PostPage::Video(u) => {
                assert!(
                    u.starts_with("https://instagram.frec52-1.fna.fbcdn.net/"),
                    "{u}"
                );
                assert!(u.contains("&_nc_sid=5e9851") && !u.contains("&amp;"), "{u}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn og_video_of_a_possible_carousel_is_not_trusted() {
        let page = PAGE_PUBLIC_REEL.replace("/reel/aye83DjauH/", "/p/aye83DjauH/");
        assert_eq!(
            InstagramDownloader::post_page_verdict("/p/aye83DjauH/", &page, false),
            PostPage::Visible
        );
    }

    /// The old cascade only stopped early on "GQL null", which never happens
    /// anonymously (401), so /tv/BkfuX9UB-eK still paid ~12 s of yt-dlp.
    #[test]
    fn broken_embed_plus_error_page_stops_before_ytdlp() {
        let hidden = InstagramDownloader::post_page_verdict("/p/BkfuX9UB-eK/", PAGE_LOCKED, true);
        assert!(InstagramDownloader::hidden_without_login(
            true, &hidden, false
        ));
        // Weak evidence alone keeps the yt-dlp fallback.
        assert!(!InstagramDownloader::hidden_without_login(
            false, &hidden, false
        ));
        assert!(!InstagramDownloader::hidden_without_login(
            true,
            &PostPage::Visible,
            false
        ));
        assert!(!InstagramDownloader::hidden_without_login(
            true, &hidden, true
        ));
        assert!(matches!(
            InstagramDownloader::parse_embed_html(EMBED_BROKEN),
            EmbedOutcome::Broken
        ));
    }

    #[test]
    fn broken_embed_and_empty_gql_mean_login_and_classify_as_auth() {
        let err = InstagramDownloader::login_locked_error("BkfuX9UB-eK");
        assert_eq!(
            crate::core::errors::classify_download_error(&format!("{err:#}")).0,
            "auth_required"
        );
    }

    /// Live, 2 requests (embed + post page), no cookies: the locked /tv/ post
    /// must answer auth-required without spawning yt-dlp.
    #[tokio::test]
    #[ignore]
    async fn instagram_live_locked_tv_stops_early() {
        let dl = InstagramDownloader::new();
        let t = std::time::Instant::now();
        let err = dl
            .get_media_info("https://www.instagram.com/tv/BkfuX9UB-eK/")
            .await
            .unwrap_err();
        eprintln!("locked tv: {:?} -> {err:#}", t.elapsed());
        assert!(t.elapsed() < std::time::Duration::from_secs(8));
        assert_eq!(
            crate::core::errors::classify_download_error(&format!("{err:#}")).0,
            "auth_required"
        );
    }

    /// Live: `cargo test --lib -p omniget-core instagram_live -- --ignored`.
    /// 4 requests, no cookies.
    #[tokio::test]
    #[ignore]
    async fn instagram_live_gql_public_vs_locked() {
        let dl = InstagramDownloader::new();
        let public = dl.request_gql("aye83DjauH").await;
        eprintln!("public: {:?}", public.as_ref().map(|o| o.is_some()));
        // 26/09 from this network: anonymous GQL answered HTTP 401 even for
        // the public post, so no assertion on the outcome here.
        let locked = dl.request_gql("BkfuX9UB-eK").await;
        eprintln!("locked: {:?}", locked.as_ref().map(|o| o.is_some()));
    }
}
