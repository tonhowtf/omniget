use omniget_core::models::progress::ProgressUpdate;
use regex::Regex;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};

use crate::core::direct_downloader;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

const GRAPHQL_URL: &str = "https://api.x.com/graphql/4Siu98E55GquhG52zHdY5w/TweetDetail";
const TOKEN_URL: &str = "https://api.x.com/1.1/guest/activate.json";
const BEARER: &str = "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

const TWEET_FEATURES: &str = r#"{"rweb_video_screen_enabled":false,"payments_enabled":false,"rweb_xchat_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_graphql_skip_user_profile_image_extensions_enabled":false,"premium_content_api_read_enabled":false,"communities_web_enable_tweet_community_results_fetch":true,"c9s_tweet_anatomy_moderator_badge_enabled":true,"responsive_web_grok_analyze_button_fetch_trends_enabled":false,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_jetfuel_frame":true,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"tweet_awards_web_tipping_enabled":false,"responsive_web_grok_show_grok_translated_post":false,"responsive_web_grok_analysis_button_from_backend":true,"creator_subscriptions_quote_tweet_preview_enabled":false,"freedom_of_speech_not_reach_fetch_enabled":true,"standardized_nudges_misinfo":true,"tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":true,"responsive_web_grok_imagine_annotation_enabled":true,"responsive_web_grok_community_note_auto_translation_is_enabled":false,"responsive_web_enhance_cards_enabled":false}"#;

const TWEET_FIELD_TOGGLES: &str = r#"{"withArticleRichContentState":true,"withArticlePlainText":false,"withGrokAnalyze":false,"withDisallowedReplyControls":false}"#;

pub struct TwitterDownloader {
    client: reqwest::Client,
    guest_token: Arc<Mutex<Option<String>>>,
}

enum TwitterMedia {
    Single(TwitterMediaItem),
    Multiple(Vec<TwitterMediaItem>),
}

struct TwitterMediaItem {
    media_type: TwitterMediaType,
    url: String,
    extension: String,
}

enum TwitterMediaType {
    Video,
    Photo,
    AnimatedGif,
}

impl Default for TwitterDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl TwitterDownloader {
    fn manual_cookie_string() -> Option<String> {
        let raw = crate::storage::config::load_settings_standalone()
            .advanced
            .twitter_manual_cookie;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        let parsed = crate::core::cookie_parser::parse_cookie_input(trimmed, "");
        if !parsed.cookie_string.trim().is_empty() {
            Some(parsed.cookie_string)
        } else {
            Some(trimmed.to_string())
        }
    }

    fn managed_cookie_string() -> Option<String> {
        for domain in ["x.com", "twitter.com"] {
            let slug = omniget_core::core::log_hook::current_cookie_slug();
            let path = crate::cookies::account_path_for_consumer(domain, slug.as_deref())
                .or_else(|| crate::cookies::account_path_for_consumer(domain, None));
            let Some(path) = path else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            if let Some(header) = Self::cookie_header_from_netscape(&content) {
                return Some(header);
            }
        }
        None
    }

    fn cookie_header_from_netscape(content: &str) -> Option<String> {
        let pairs: Vec<String> = content
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with("# Netscape") {
                    return None;
                }
                let normalized = trimmed.strip_prefix("#HttpOnly_").unwrap_or(trimmed);
                if normalized.starts_with('#') {
                    return None;
                }
                let parts: Vec<&str> = normalized.split('\t').collect();
                if parts.len() < 7 {
                    return None;
                }
                let name = parts[5].trim();
                let value = parts[6].trim();
                if name.is_empty() {
                    return None;
                }
                Some(format!("{}={}", name, value))
            })
            .collect();
        if pairs.is_empty() {
            None
        } else {
            Some(pairs.join("; "))
        }
    }

    fn auth_cookie_string() -> Option<String> {
        Self::managed_cookie_string().or_else(Self::manual_cookie_string)
    }

    fn cookie_value(cookie_header: &str, name: &str) -> Option<String> {
        cookie_header.split(';').find_map(|part| {
            let (k, v) = part.trim().split_once('=')?;
            (k.trim() == name).then(|| v.trim().to_string())
        })
    }

    fn request_cookie_header(guest_token: &str) -> String {
        let guest_cookie = format!(
            "guest_id={}",
            urlencoding::encode(&format!("v1:{}", guest_token))
        );

        if let Some(auth) = Self::auth_cookie_string() {
            format!("{}; {}", guest_cookie, auth)
        } else {
            guest_cookie
        }
    }

    fn clone_media_array(value: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
        value.as_array().filter(|items| !items.is_empty()).cloned()
    }

    fn find_first_array_for_key(
        value: &serde_json::Value,
        target_key: &str,
    ) -> Option<Vec<serde_json::Value>> {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(found) = map
                    .get(target_key)
                    .and_then(Self::clone_media_array)
                    .filter(|items| !items.is_empty())
                {
                    return Some(found);
                }

                for child in map.values() {
                    if let Some(found) = Self::find_first_array_for_key(child, target_key) {
                        return Some(found);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    if let Some(found) = Self::find_first_array_for_key(child, target_key) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }

        None
    }

    fn media_arrays_from_tweet_result(
        tweet_result: &serde_json::Value,
    ) -> Option<Vec<serde_json::Value>> {
        let candidate_paths = [
            "/legacy/extended_entities/media",
            "/tweet/legacy/extended_entities/media",
            "/legacy/retweeted_status_result/result/legacy/extended_entities/media",
            "/legacy/retweeted_status_result/result/tweet/legacy/extended_entities/media",
            "/tweet/legacy/retweeted_status_result/result/legacy/extended_entities/media",
            "/tweet/legacy/retweeted_status_result/result/tweet/legacy/extended_entities/media",
            "/legacy/quoted_status_result/result/legacy/extended_entities/media",
            "/legacy/quoted_status_result/result/tweet/legacy/extended_entities/media",
            "/tweet/legacy/quoted_status_result/result/legacy/extended_entities/media",
            "/tweet/legacy/quoted_status_result/result/tweet/legacy/extended_entities/media",
        ];

        for path in candidate_paths {
            if let Some(items) = tweet_result
                .pointer(path)
                .and_then(Self::clone_media_array)
                .filter(|items| !items.is_empty())
            {
                return Some(items);
            }
        }

        Self::find_first_array_for_key(tweet_result, "media")
    }

    fn infer_media_type(media_item: &serde_json::Value) -> Option<TwitterMediaType> {
        match media_item.get("type").and_then(|v| v.as_str()) {
            Some("photo") => return Some(TwitterMediaType::Photo),
            Some("video") => return Some(TwitterMediaType::Video),
            Some("animated_gif") => return Some(TwitterMediaType::AnimatedGif),
            _ => {}
        }

        if media_item
            .pointer("/video_info/variants")
            .and_then(|v| v.as_array())
            .is_some()
            || media_item
                .pointer("/video/variants")
                .and_then(|v| v.as_array())
                .is_some()
        {
            return Some(TwitterMediaType::Video);
        }

        if media_item
            .get("media_url_https")
            .and_then(|v| v.as_str())
            .is_some()
            || media_item
                .get("media_url")
                .and_then(|v| v.as_str())
                .is_some()
            || media_item.get("url").and_then(|v| v.as_str()).is_some()
        {
            return Some(TwitterMediaType::Photo);
        }

        None
    }

    pub fn new() -> Self {
        let mut builder = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15));

        if let Some(jar) = crate::core::cookie_parser::load_extension_cookies_for_domain("x.com") {
            builder = builder.cookie_provider(jar);
        }

        let client = builder.build().unwrap_or_default();
        Self {
            client,
            guest_token: Arc::new(Mutex::new(None)),
        }
    }

    fn extract_tweet_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        if segments.len() >= 3 && segments[1] == "status" {
            let id = segments[2];
            if id.chars().all(|c| c.is_ascii_digit()) {
                return Some(id.to_string());
            }
        }

        None
    }

    async fn get_guest_token(&self, force: bool) -> anyhow::Result<String> {
        if !force {
            let cached = self.guest_token.lock().await;
            if let Some(ref token) = *cached {
                return Ok(token.clone());
            }
        }

        let response = self
            .client
            .post(TOKEN_URL)
            .header("Authorization", BEARER)
            .header("x-twitter-client-language", "en")
            .header("x-twitter-active-user", "yes")
            .header("Accept-Language", "en")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Falha ao obter guest token: HTTP {}",
                response.status()
            ));
        }

        let json: serde_json::Value = response.json().await?;
        let token = json
            .get("guest_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Guest token ausente na resposta"))?
            .to_string();

        let mut cached = self.guest_token.lock().await;
        *cached = Some(token.clone());
        Ok(token)
    }

    async fn request_tweet(
        &self,
        tweet_id: &str,
        guest_token: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let variables = serde_json::json!({
            "focalTweetId": tweet_id,
            "with_rux_injections": false,
            "rankingMode": "Relevance",
            "includePromotedContent": true,
            "withCommunity": true,
            "withQuickPromoteEligibilityTweetFields": true,
            "withBirdwatchNotes": true,
            "withVoice": true
        });

        let url = format!(
            "{}?variables={}&features={}&fieldToggles={}",
            GRAPHQL_URL,
            urlencoding::encode(&variables.to_string()),
            urlencoding::encode(TWEET_FEATURES),
            urlencoding::encode(TWEET_FIELD_TOGGLES),
        );

        let cookie_val = Self::request_cookie_header(guest_token);
        let ct0 = Self::cookie_value(&cookie_val, "ct0");
        let has_auth_token = Self::cookie_value(&cookie_val, "auth_token").is_some();

        let mut request = self
            .client
            .get(&url)
            .header("Authorization", BEARER)
            .header("x-guest-token", guest_token)
            .header("x-twitter-client-language", "en")
            .header("x-twitter-active-user", "yes")
            .header("Accept-Language", "en")
            .header("Content-Type", "application/json")
            .header("Cookie", &cookie_val);
        if has_auth_token {
            request = request.header("x-twitter-auth-type", "OAuth2Session");
        }
        if let Some(ct0) = ct0 {
            request = request.header("x-csrf-token", ct0);
        }

        let response = request.send().await?;

        let status = response.status();
        tracing::debug!("[twitter] graphql tweet_id={} status={}", tweet_id, status);

        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(anyhow!("token_expired"));
        }

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(anyhow!("Post not available"));
        }

        if !status.is_success() {
            return Err(anyhow!("Twitter API retornou HTTP {}", status));
        }

        response.json().await.map_err(Into::into)
    }

    fn calculate_syndication_token(id: &str) -> String {
        let num: f64 = id.parse().unwrap_or(0.0);
        let raw = (num / 1e15) * std::f64::consts::PI;
        let base36 = Self::f64_to_base36(raw);
        base36
            .replace('.', "")
            .trim_start_matches('0')
            .trim_end_matches('0')
            .to_string()
    }

    fn f64_to_base36(value: f64) -> String {
        if value == 0.0 {
            return "0".to_string();
        }

        let integer_part = value as u64;
        let fractional_part = value - integer_part as f64;

        let mut int_str = if integer_part == 0 {
            "0".to_string()
        } else {
            let mut n = integer_part;
            let mut digits = Vec::new();
            while n > 0 {
                let rem = (n % 36) as u8;
                let ch = if rem < 10 {
                    (b'0' + rem) as char
                } else {
                    (b'a' + rem - 10) as char
                };
                digits.push(ch);
                n /= 36;
            }
            digits.reverse();
            digits.into_iter().collect()
        };

        if fractional_part > 0.0 {
            int_str.push('.');
            let mut frac = fractional_part;
            for _ in 0..12 {
                frac *= 36.0;
                let digit = frac as u8;
                let ch = if digit < 10 {
                    (b'0' + digit) as char
                } else {
                    (b'a' + digit - 10) as char
                };
                int_str.push(ch);
                frac -= digit as f64;
                if frac <= 0.0 {
                    break;
                }
            }
        }

        int_str
    }

    async fn request_syndication(&self, tweet_id: &str) -> anyhow::Result<serde_json::Value> {
        let token = Self::calculate_syndication_token(tweet_id);

        let url = format!(
            "https://cdn.syndication.twimg.com/tweet-result?id={}&token={}",
            tweet_id, token
        );

        let mut request = self.client.get(&url);
        if let Some(cookie) = Self::auth_cookie_string() {
            request = request.header("Cookie", cookie);
        }

        let response = request.send().await?;
        tracing::debug!(
            "[twitter] syndication tweet_id={} token={} status={}",
            tweet_id,
            token,
            response.status()
        );

        if !response.status().is_success() {
            return Err(anyhow!(
                "Syndication API retornou HTTP {}",
                response.status()
            ));
        }

        response.json().await.map_err(Into::into)
    }

    fn extract_graphql_media(
        json: &serde_json::Value,
        tweet_id: &str,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let instructions = json
            .pointer("/data/threaded_conversation_with_injections_v2/instructions")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Post not available"))?;

        let add_insn = instructions
            .iter()
            .find(|i| i.get("type").and_then(|v| v.as_str()) == Some("TimelineAddEntries"))
            .ok_or_else(|| anyhow!("Post not available"))?;

        let entry_id = format!("tweet-{}", tweet_id);
        let entries = add_insn
            .get("entries")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Post not available"))?;

        let tweet_result = entries
            .iter()
            .find(|e| e.get("entryId").and_then(|v| v.as_str()) == Some(&entry_id))
            .and_then(|e| e.pointer("/content/itemContent/tweet_results/result"))
            .ok_or_else(|| anyhow!("Post not available"))?;

        let typename = tweet_result
            .get("__typename")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        tracing::debug!(
            "[twitter] graphql media typename={} tweet_id={}",
            typename,
            tweet_id
        );

        match typename {
            "TweetUnavailable" | "TweetTombstone" => {
                let reason = tweet_result
                    .pointer("/result/reason")
                    .or_else(|| tweet_result.get("reason"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if reason == "Protected" {
                    return Err(anyhow!("Post privado"));
                }

                let tombstone_text = tweet_result
                    .pointer("/tombstone/text/text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                tracing::warn!(
                    "[twitter] graphql tombstone tweet_id={} reason='{}' tombstone_text='{}'",
                    tweet_id,
                    reason,
                    tombstone_text
                );

                if reason == "NsfwLoggedOut" || tombstone_text.starts_with("Age-restricted") {
                    return Err(anyhow!("Age-restricted content"));
                }

                Err(anyhow!("Post not available"))
            }
            "Tweet" | "TweetWithVisibilityResults" => {
                let media = Self::media_arrays_from_tweet_result(tweet_result)
                    .ok_or_else(|| anyhow!("No media found in tweet"))?;
                tracing::debug!(
                    "[twitter] graphql extracted {} media entries for tweet_id={}",
                    media.len(),
                    tweet_id
                );
                Ok(media)
            }
            _ => Err(anyhow!("Post not available")),
        }
    }

    fn extract_syndication_media(
        json: &serde_json::Value,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let typename = json
            .get("__typename")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if typename == "TweetTombstone" || typename == "TweetUnavailable" {
            tracing::warn!("[twitter] syndication tombstone typename={}", typename);
            return Err(anyhow!("Post not available"));
        }

        let media = json
            .get("mediaDetails")
            .and_then(Self::clone_media_array)
            .or_else(|| json.get("photos").and_then(Self::clone_media_array))
            .or_else(|| Self::find_first_array_for_key(json, "mediaDetails"))
            .or_else(|| Self::find_first_array_for_key(json, "photos"))
            .ok_or_else(|| anyhow!("No media found in tweet"))?;

        tracing::debug!(
            "[twitter] syndication extracted {} media entries",
            media.len()
        );
        Ok(media)
    }

    fn best_video_url(media_item: &serde_json::Value) -> Option<String> {
        let variants = media_item
            .pointer("/video_info/variants")
            .or_else(|| media_item.pointer("/video/variants"))
            .and_then(|v| v.as_array())?;

        let best_mp4 = variants
            .iter()
            .filter(|v| v.get("content_type").and_then(|c| c.as_str()) == Some("video/mp4"))
            .max_by_key(|v| v.get("bitrate").and_then(|b| b.as_u64()).unwrap_or(0))
            .and_then(|v| v.get("url").and_then(|u| u.as_str()))
            .map(|s| s.to_string());

        if best_mp4.is_some() {
            return best_mp4;
        }

        variants
            .iter()
            .filter_map(|v| v.get("url").and_then(|u| u.as_str()))
            .find(|url| url.contains(".m3u8") || url.contains("mpegurl"))
            .or_else(|| {
                variants
                    .iter()
                    .filter_map(|v| v.get("url").and_then(|u| u.as_str()))
                    .next()
            })
            .map(|s| s.to_string())
    }

    fn best_photo_url(media_item: &serde_json::Value) -> Option<(String, String)> {
        let base_url = media_item
            .get("media_url_https")
            .or_else(|| media_item.get("media_url"))
            .or_else(|| media_item.get("url"))
            .and_then(|v| v.as_str())?;
        Self::best_photo_url_from_str(base_url)
    }

    fn best_photo_url_from_str(base_url: &str) -> Option<(String, String)> {
        let cleaned = Self::decode_html_url(base_url);
        let extension = url::Url::parse(&cleaned)
            .ok()
            .and_then(|u| {
                u.path_segments()
                    .and_then(|segments| segments.last().map(|s| s.to_string()))
            })
            .and_then(|filename| {
                filename
                    .rsplit('.')
                    .next()
                    .and_then(|ext| ext.split('?').next())
                    .map(|ext| ext.to_string())
            })
            .filter(|ext| !ext.is_empty())
            .unwrap_or_else(|| "jpg".to_string());

        let url = if let Ok(mut parsed) = url::Url::parse(&cleaned) {
            let existing: Vec<(String, String)> = parsed
                .query_pairs()
                .filter(|(key, _)| key != "name")
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            parsed.set_query(None);
            {
                let mut qp = parsed.query_pairs_mut();
                for (key, value) in existing {
                    qp.append_pair(&key, &value);
                }
                qp.append_pair("name", "orig");
            }
            parsed.to_string()
        } else if cleaned.contains('?') {
            format!("{}&name=orig", cleaned)
        } else {
            format!("{}?name=orig", cleaned)
        };

        Some((url, extension))
    }

    fn decode_html_url(raw: &str) -> String {
        raw.replace("\\u0026", "&")
            .replace("&amp;", "&")
            .replace("&#38;", "&")
    }

    fn extract_html_photo_items(html: &str) -> Vec<TwitterMediaItem> {
        let re = match Regex::new(r#"https://pbs\.twimg\.com/media/[^\s"'<>\\]+"#) {
            Ok(re) => re,
            Err(_) => return Vec::new(),
        };
        let mut seen = std::collections::HashSet::new();
        let mut items = Vec::new();
        for m in re.find_iter(html) {
            let raw = m.as_str().trim_end_matches([',', ';', ')', ']']);
            if let Some((url, extension)) = Self::best_photo_url_from_str(raw) {
                if seen.insert(url.clone()) {
                    items.push(TwitterMediaItem {
                        media_type: TwitterMediaType::Photo,
                        url,
                        extension,
                    });
                }
            }
        }
        items
    }

    fn parse_media_items(media: &[serde_json::Value]) -> anyhow::Result<TwitterMedia> {
        let items: Vec<TwitterMediaItem> = media
            .iter()
            .filter_map(|m| match Self::infer_media_type(m)? {
                TwitterMediaType::Photo => {
                    let (url, ext) = Self::best_photo_url(m)?;
                    Some(TwitterMediaItem {
                        media_type: TwitterMediaType::Photo,
                        url,
                        extension: ext,
                    })
                }
                TwitterMediaType::Video => {
                    let url = Self::best_video_url(m)?;
                    let extension = if url.contains(".m3u8") || url.contains("mpegurl") {
                        "ytdlp"
                    } else {
                        "mp4"
                    };
                    Some(TwitterMediaItem {
                        media_type: TwitterMediaType::Video,
                        url,
                        extension: extension.to_string(),
                    })
                }
                TwitterMediaType::AnimatedGif => {
                    let url = Self::best_video_url(m)?;
                    Some(TwitterMediaItem {
                        media_type: TwitterMediaType::AnimatedGif,
                        url,
                        extension: "mp4".to_string(),
                    })
                }
            })
            .collect();

        if items.is_empty() {
            return Err(anyhow!("No media found in tweet"));
        }

        if items.len() == 1 {
            Ok(TwitterMedia::Single(items.into_iter().next().unwrap()))
        } else {
            Ok(TwitterMedia::Multiple(items))
        }
    }

    fn media_type_for_item(item: &TwitterMediaItem) -> MediaType {
        match item.media_type {
            TwitterMediaType::Video => MediaType::Video,
            TwitterMediaType::Photo => MediaType::Photo,
            TwitterMediaType::AnimatedGif => MediaType::Gif,
        }
    }

    fn media_info_from_twitter_media(
        filename_base: String,
        twitter_media: TwitterMedia,
    ) -> MediaInfo {
        match twitter_media {
            TwitterMedia::Single(item) => {
                let media_type = Self::media_type_for_item(&item);
                MediaInfo {
                    title: filename_base,
                    author: String::new(),
                    platform: "twitter".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
                    available_qualities: vec![VideoQuality {
                        label: "original".to_string(),
                        width: 0,
                        height: 0,
                        url: item.url,
                        format: item.extension,
                    }],
                    media_type,
                    file_size_bytes: None,
                }
            }
            TwitterMedia::Multiple(items) => {
                let qualities: Vec<VideoQuality> = items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| VideoQuality {
                        label: format!("media_{}", i + 1),
                        width: 0,
                        height: 0,
                        url: item.url.clone(),
                        format: item.extension.clone(),
                    })
                    .collect();

                MediaInfo {
                    title: filename_base,
                    author: String::new(),
                    platform: "twitter".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
                    available_qualities: qualities,
                    media_type: MediaType::Carousel,
                    file_size_bytes: None,
                }
            }
        }
    }
}

#[async_trait]
impl PlatformDownloader for TwitterDownloader {
    fn name(&self) -> &str {
        "twitter"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "twitter.com"
                    || host.ends_with(".twitter.com")
                    || host == "x.com"
                    || host.ends_with(".x.com")
                    || host == "vxtwitter.com"
                    || host.ends_with(".vxtwitter.com")
                    || host == "fixvx.com"
                    || host.ends_with(".fixvx.com");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        match self.native_get_media_info(url).await {
            Ok(info) => Ok(info),
            Err(native_err) => {
                tracing::warn!(
                    "[twitter] native failed: {}, trying yt-dlp fallback",
                    native_err
                );
                match self.fallback_ytdlp(url).await {
                    Ok(info) => Ok(info),
                    Err(fallback_err) => {
                        tracing::warn!(
                            "[twitter] yt-dlp fallback failed after native error: {}",
                            fallback_err
                        );
                        Err(anyhow!(
                            "Twitter extraction failed. native='{}'; ytdlp='{}'",
                            native_err,
                            fallback_err
                        ))
                    }
                }
            }
        }
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        if let Some(quality) = info.available_qualities.first() {
            if quality.format == "ytdlp" {
                let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
                let mut extra_flags = Vec::new();
                if let Some(cookie) = Self::auth_cookie_string() {
                    extra_flags.push("--add-headers".to_string());
                    extra_flags.push(format!("Cookie:{}", cookie));
                }
                return crate::core::ytdlp::download_video(
                    &ytdlp_path,
                    &quality.url,
                    &opts.output_dir,
                    None,
                    progress,
                    opts.download_mode.as_deref(),
                    opts.format_id.as_deref(),
                    opts.filename_template.as_deref(),
                    opts.referer.as_deref().or(Some("https://x.com/")),
                    opts.cancel_token.clone(),
                    None,
                    opts.concurrent_fragments,
                    false,
                    &extra_flags,
                    opts.audio_format.as_deref(),
                )
                .await;
            }
        }

        let count = info.available_qualities.len();

        if count == 0 {
            anyhow::bail!(
                "No downloadable media found for this tweet (it may be text-only, protected, or deleted)"
            );
        }

        if count == 1 {
            let quality = info.available_qualities.first().unwrap();
            let filename = format!(
                "{}.{}",
                sanitize_filename::sanitize(&info.title),
                quality.format
            );
            let output = opts.output_dir.join(&filename);

            let bytes = direct_downloader::download_direct(
                &self.client,
                &quality.url,
                &output,
                progress,
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

        let mut total_bytes = 0u64;
        let mut last_path = opts.output_dir.clone();

        for (i, quality) in info.available_qualities.iter().enumerate() {
            let filename = format!(
                "{}_{}.{}",
                sanitize_filename::sanitize(&info.title),
                i + 1,
                quality.format
            );
            let output = opts.output_dir.join(&filename);
            let (tx, _rx) = mpsc::channel(8);

            let bytes = direct_downloader::download_direct(
                &self.client,
                &quality.url,
                &output,
                tx,
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

impl TwitterDownloader {
    async fn fallback_ytdlp(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
        let mut extra_flags = vec![
            "--referer".to_string(),
            "https://x.com/".to_string(),
            "--add-headers".to_string(),
            "Referer:https://x.com/".to_string(),
        ];
        if let Some(cookie) = Self::auth_cookie_string() {
            extra_flags.push("--add-headers".to_string());
            extra_flags.push(format!("Cookie:{}", cookie));
        }
        let json = crate::core::ytdlp::get_video_info(&ytdlp_path, url, &extra_flags).await?;
        crate::platforms::generic_ytdlp::GenericYtdlpDownloader::parse_video_info(&json)
    }

    async fn native_get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let tweet_id =
            Self::extract_tweet_id(url).ok_or_else(|| anyhow!("Could not extract tweet ID"))?;
        tracing::debug!(
            "[twitter] native_get_media_info tweet_id={} url={}",
            tweet_id,
            url
        );

        let filename_base = format!("twitter_{}", tweet_id);

        let media_items = match self.try_graphql(&tweet_id).await {
            Ok(items) => items,
            Err(graphql_err) => {
                tracing::warn!(
                    "[twitter] graphql lookup failed for tweet_id={}: {}",
                    tweet_id,
                    graphql_err
                );
                match self.request_syndication(&tweet_id).await {
                    Ok(syndication) => match Self::extract_syndication_media(&syndication) {
                        Ok(items) => items,
                        Err(syndication_extract_err) => {
                            tracing::warn!(
                                "[twitter] syndication media extraction failed for tweet_id={}: {}",
                                tweet_id,
                                syndication_extract_err
                            );
                            match self.request_html_media(url).await {
                                Ok(items) => items,
                                Err(html_err) => {
                                    return Err(anyhow!(
                                        "Post not available; graphql='{}'; syndication_extract='{}'; html='{}'",
                                        graphql_err,
                                        syndication_extract_err,
                                        html_err
                                    ));
                                }
                            }
                        }
                    },
                    Err(syndication_err) => {
                        tracing::warn!(
                            "[twitter] syndication lookup failed for tweet_id={}: {}",
                            tweet_id,
                            syndication_err
                        );
                        match self.request_html_media(url).await {
                            Ok(items) => items,
                            Err(html_err) => {
                                return Err(anyhow!(
                                    "Post not available; graphql='{}'; syndication='{}'; html='{}'",
                                    graphql_err,
                                    syndication_err,
                                    html_err
                                ));
                            }
                        }
                    }
                }
            }
        };

        let twitter_media = Self::parse_media_items(&media_items)?;

        Ok(Self::media_info_from_twitter_media(
            filename_base,
            twitter_media,
        ))
    }

    async fn request_html_media(&self, url: &str) -> anyhow::Result<Vec<serde_json::Value>> {
        let mut request = self
            .client
            .get(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept-Language", "en")
            .header("Referer", "https://x.com/");
        if let Some(cookie) = Self::auth_cookie_string() {
            request = request.header("Cookie", cookie);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(anyhow!("HTML request returned HTTP {}", response.status()));
        }
        let html = response.text().await?;
        let items = Self::extract_html_photo_items(&html);
        if items.is_empty() {
            return Err(anyhow!("No photo URLs found in HTML"));
        }
        tracing::debug!("[twitter] html extracted {} photo entries", items.len());
        Ok(items
            .into_iter()
            .map(|item| {
                serde_json::json!({
                    "type": "photo",
                    "media_url_https": item.url,
                })
            })
            .collect())
    }

    async fn try_graphql(&self, tweet_id: &str) -> anyhow::Result<Vec<serde_json::Value>> {
        let token = self.get_guest_token(false).await?;

        match self.request_tweet(tweet_id, &token).await {
            Ok(json) => Self::extract_graphql_media(&json, tweet_id),
            Err(e) if e.to_string() == "token_expired" => {
                let new_token = self.get_guest_token(true).await?;
                let json = self.request_tweet(tweet_id, &new_token).await?;
                Self::extract_graphql_media(&json, tweet_id)
            }
            Err(e) => Err(e),
        }
    }
}
