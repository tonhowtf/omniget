use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};

use crate::core::direct_downloader;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const GRAPHQL_URL: &str =
    "https://api.x.com/graphql/4Siu98E55GquhG52zHdY5w/TweetDetail";
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
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();

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
            return Err(anyhow!("Falha ao obter guest token: HTTP {}", response.status()));
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

        let cookie_val = format!("guest_id={}", urlencoding::encode(&format!("v1:{}", guest_token)));

        let response = self
            .client
            .get(&url)
            .header("Authorization", BEARER)
            .header("x-guest-token", guest_token)
            .header("x-twitter-client-language", "en")
            .header("x-twitter-active-user", "yes")
            .header("Accept-Language", "en")
            .header("Content-Type", "application/json")
            .header("Cookie", &cookie_val)
            .send()
            .await?;

        let status = response.status();

        if status == reqwest::StatusCode::FORBIDDEN
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(anyhow!("token_expired"));
        }

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(anyhow!("Post não disponível"));
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

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Syndication API retornou HTTP {}", response.status()));
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
            .ok_or_else(|| anyhow!("Post não disponível"))?;

        let add_insn = instructions
            .iter()
            .find(|i| i.get("type").and_then(|v| v.as_str()) == Some("TimelineAddEntries"))
            .ok_or_else(|| anyhow!("Post não disponível"))?;

        let entry_id = format!("tweet-{}", tweet_id);
        let entries = add_insn
            .get("entries")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Post não disponível"))?;

        let tweet_result = entries
            .iter()
            .find(|e| e.get("entryId").and_then(|v| v.as_str()) == Some(&entry_id))
            .and_then(|e| e.pointer("/content/itemContent/tweet_results/result"))
            .ok_or_else(|| anyhow!("Post não disponível"))?;

        let typename = tweet_result
            .get("__typename")
            .and_then(|v| v.as_str())
            .unwrap_or("");

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

                if reason == "NsfwLoggedOut" || tombstone_text.starts_with("Age-restricted") {
                    return Err(anyhow!("Conteúdo restrito por idade"));
                }

                Err(anyhow!("Post não disponível"))
            }
            "Tweet" | "TweetWithVisibilityResults" => {
                let base_tweet = if typename == "TweetWithVisibilityResults" {
                    tweet_result.pointer("/tweet/legacy")
                } else {
                    tweet_result.get("legacy")
                };

                let base_tweet =
                    base_tweet.ok_or_else(|| anyhow!("Post não disponível"))?;

                let reposted_media = if typename == "TweetWithVisibilityResults" {
                    tweet_result
                        .pointer("/tweet/legacy/retweeted_status_result/result/tweet/legacy/extended_entities/media")
                } else {
                    tweet_result
                        .pointer("/legacy/retweeted_status_result/result/legacy/extended_entities/media")
                };

                let media = reposted_media
                    .or_else(|| base_tweet.pointer("/extended_entities/media"))
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| anyhow!("Nenhuma mídia encontrada no tweet"))?;

                Ok(media.clone())
            }
            _ => Err(anyhow!("Post não disponível")),
        }
    }

    fn extract_syndication_media(
        json: &serde_json::Value,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let media = json
            .get("mediaDetails")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Nenhuma mídia encontrada no tweet"))?;

        Ok(media.clone())
    }

    fn best_video_url(media_item: &serde_json::Value) -> Option<String> {
        let variants = media_item
            .pointer("/video_info/variants")
            .and_then(|v| v.as_array())?;

        variants
            .iter()
            .filter(|v| {
                v.get("content_type").and_then(|c| c.as_str()) == Some("video/mp4")
            })
            .max_by_key(|v| {
                v.get("bitrate")
                    .and_then(|b| b.as_u64())
                    .unwrap_or(0)
            })
            .and_then(|v| v.get("url").and_then(|u| u.as_str()))
            .map(|s| s.to_string())
    }

    fn parse_media_items(media: &[serde_json::Value]) -> anyhow::Result<TwitterMedia> {
        let items: Vec<TwitterMediaItem> = media
            .iter()
            .filter_map(|m| {
                let media_type_str = m.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match media_type_str {
                    "photo" => {
                        let base_url = m
                            .get("media_url_https")
                            .and_then(|v| v.as_str())?;
                        let url = format!("{}?name=4096x4096", base_url);
                        let ext = base_url
                            .rsplit('.')
                            .next()
                            .unwrap_or("jpg")
                            .to_string();
                        Some(TwitterMediaItem {
                            media_type: TwitterMediaType::Photo,
                            url,
                            extension: ext,
                        })
                    }
                    "video" => {
                        let url = Self::best_video_url(m)?;
                        Some(TwitterMediaItem {
                            media_type: TwitterMediaType::Video,
                            url,
                            extension: "mp4".to_string(),
                        })
                    }
                    "animated_gif" => {
                        let url = Self::best_video_url(m)?;
                        Some(TwitterMediaItem {
                            media_type: TwitterMediaType::AnimatedGif,
                            url,
                            extension: "mp4".to_string(),
                        })
                    }
                    _ => None,
                }
            })
            .collect();

        if items.is_empty() {
            return Err(anyhow!("Nenhuma mídia encontrada no tweet"));
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
        let tweet_id = Self::extract_tweet_id(url)
            .ok_or_else(|| anyhow!("Não foi possível extrair o ID do tweet"))?;

        let filename_base = format!("twitter_{}", tweet_id);

        let media_items = match self.try_graphql(&tweet_id).await {
            Ok(items) => items,
            Err(_) => {
                let syndication = self.request_syndication(&tweet_id).await?;
                Self::extract_syndication_media(&syndication)?
            }
        };

        let twitter_media = Self::parse_media_items(&media_items)?;

        match twitter_media {
            TwitterMedia::Single(item) => {
                let media_type = Self::media_type_for_item(&item);
                Ok(MediaInfo {
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
                })
            }
            TwitterMedia::Multiple(items) => {
                let has_video = items
                    .iter()
                    .any(|i| matches!(i.media_type, TwitterMediaType::Video | TwitterMediaType::AnimatedGif));

                let media_type = if has_video {
                    MediaType::Video
                } else {
                    MediaType::Carousel
                };

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

                Ok(MediaInfo {
                    title: filename_base,
                    author: String::new(),
                    platform: "twitter".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
                    available_qualities: qualities,
                    media_type,
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

            let bytes = direct_downloader::download_direct(
                &self.client,
                &quality.url,
                &output,
                progress,
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
                quality.format
            );
            let output = opts.output_dir.join(&filename);
            let (tx, _rx) = mpsc::channel(8);

            let bytes = direct_downloader::download_direct(
                &self.client,
                &quality.url,
                &output,
                tx,
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

impl TwitterDownloader {
    async fn try_graphql(
        &self,
        tweet_id: &str,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
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
