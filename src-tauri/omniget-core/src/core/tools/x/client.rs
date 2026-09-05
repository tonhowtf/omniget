//! Cliente do GraphQL interno do X (estudo 67). Headers como o site: bearer
//! publico, `x-csrf-token` = cookie `ct0`, `x-twitter-auth-type` quando ha
//! `auth_token`, guest token quando nao ha sessao, `x-client-transaction-id`
//! quando da para gerar. 404 "Query not found" recarrega os query IDs dos
//! bundles e tenta de novo; 429 devolve `X_RATE_LIMIT:<segundos>`.

use std::time::{Duration, Instant};

use anyhow::anyhow;
use once_cell::sync::Lazy;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::txid::TxIdGen;

pub const BEARER: &str = "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
pub const LOGIN_REQUIRED: &str = "X_LOGIN_REQUIRED";

const BASE_FEATURES: &[(&str, bool)] = &[
    ("articles_preview_enabled", false),
    ("c9s_tweet_anatomy_moderator_badge_enabled", true),
    ("communities_web_enable_tweet_community_results_fetch", true),
    ("creator_subscriptions_quote_tweet_preview_enabled", false),
    ("creator_subscriptions_tweet_preview_api_enabled", true),
    ("freedom_of_speech_not_reach_fetch_enabled", true),
    (
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled",
        true,
    ),
    ("longform_notetweets_consumption_enabled", true),
    ("longform_notetweets_inline_media_enabled", true),
    ("longform_notetweets_rich_text_read_enabled", true),
    ("responsive_web_edit_tweet_api_enabled", true),
    ("responsive_web_enhance_cards_enabled", false),
    ("responsive_web_graphql_exclude_directive_enabled", true),
    (
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled",
        false,
    ),
    (
        "responsive_web_grok_community_note_auto_translation_is_enabled",
        false,
    ),
    ("responsive_web_graphql_timeline_navigation_enabled", true),
    ("responsive_web_grok_imagine_annotation_enabled", false),
    ("responsive_web_media_download_video_enabled", false),
    ("responsive_web_profile_redirect_enabled", true),
    (
        "responsive_web_twitter_article_tweet_consumption_enabled",
        true,
    ),
    ("rweb_tipjar_consumption_enabled", true),
    ("rweb_video_timestamps_enabled", true),
    ("standardized_nudges_misinfo", true),
    ("tweet_awards_web_tipping_enabled", false),
    (
        "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled",
        true,
    ),
    (
        "tweet_with_visibility_results_prefer_gql_media_interstitial_enabled",
        false,
    ),
    ("tweetypie_unmention_optimization_enabled", true),
    ("verified_phone_label_enabled", false),
    ("view_counts_everywhere_api_enabled", true),
    (
        "responsive_web_grok_analyze_button_fetch_trends_enabled",
        false,
    ),
    ("premium_content_api_read_enabled", false),
    (
        "profile_label_improvements_pcf_label_in_post_enabled",
        false,
    ),
    ("responsive_web_grok_share_attachment_enabled", false),
    ("responsive_web_grok_analyze_post_followups_enabled", false),
    ("responsive_web_grok_image_annotation_enabled", false),
    ("responsive_web_grok_analysis_button_from_backend", false),
    ("responsive_web_jetfuel_frame", false),
    ("rweb_video_screen_enabled", true),
    ("responsive_web_grok_show_grok_translated_post", true),
    ("hidden_profile_subscriptions_enabled", true),
    ("highlights_tweets_tab_ui_enabled", true),
    ("responsive_web_twitter_article_notes_tab_enabled", true),
    ("subscriptions_feature_can_gift_premium", true),
    (
        "subscriptions_verification_info_is_identity_verified_enabled",
        true,
    ),
    (
        "subscriptions_verification_info_verified_since_enabled",
        true,
    ),
];

/// Flags que o site manda em toda chamada (twscrape, 2026-08).
pub fn base_features() -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in BASE_FEATURES {
        m.insert((*k).to_string(), Value::Bool(*v));
    }
    Value::Object(m)
}

static GUEST: Lazy<Mutex<Option<(String, Instant)>>> = Lazy::new(|| Mutex::new(None));
#[allow(clippy::type_complexity)]
static TXID: Lazy<Mutex<Option<(u64, Option<TxIdGen>, Instant)>>> = Lazy::new(|| Mutex::new(None));

pub struct XClient {
    pub http: reqwest::Client,
    cookie: Option<String>,
}

fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|p| {
        let (k, v) = p.trim().split_once('=')?;
        (k.trim() == name).then(|| v.trim().to_string())
    })
}

impl XClient {
    pub fn new() -> anyhow::Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_static(USER_AGENT),
        );
        let http = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .default_headers(headers)
            .cookie_store(false)
            .timeout(Duration::from_secs(90))
            .build()?;
        let cookie =
            crate::platforms::cookie_provider::cookie_header_for_domains(&["x.com", "twitter.com"])
                .map(|c| c.trim().trim_end_matches(';').to_string())
                .filter(|c| !c.is_empty());
        Ok(Self { http, cookie })
    }

    pub fn cookie(&self) -> Option<&str> {
        self.cookie.as_deref()
    }

    fn ct0(&self) -> Option<String> {
        self.cookie.as_deref().and_then(|c| cookie_value(c, "ct0"))
    }

    pub fn authed(&self) -> bool {
        self.cookie
            .as_deref()
            .map(|c| cookie_value(c, "auth_token").is_some() && cookie_value(c, "ct0").is_some())
            .unwrap_or(false)
    }

    pub fn require_login(&self) -> anyhow::Result<()> {
        if self.authed() {
            Ok(())
        } else {
            Err(anyhow!(LOGIN_REQUIRED))
        }
    }

    /// Id numerico do usuario logado (cookie `twid=u%3D<id>`).
    pub fn user_id(&self) -> Option<String> {
        let raw = cookie_value(self.cookie.as_deref()?, "twid")?;
        let dec = urlencoding::decode(&raw)
            .map(|c| c.to_string())
            .unwrap_or(raw);
        let id = dec
            .trim()
            .trim_start_matches("u=")
            .trim_matches('"')
            .to_string();
        (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then_some(id)
    }

    async fn guest_token(&self, force: bool) -> anyhow::Result<String> {
        let mut g = GUEST.lock().await;
        if !force {
            if let Some((tok, at)) = g.as_ref() {
                if at.elapsed() < Duration::from_secs(2 * 3600) {
                    return Ok(tok.clone());
                }
            }
        }
        let resp = self
            .http
            .post("https://api.x.com/1.1/guest/activate.json")
            .header("Authorization", BEARER)
            .header("x-twitter-client-language", "en")
            .header("x-twitter-active-user", "yes")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "X nao entregou guest token: HTTP {}",
                resp.status()
            ));
        }
        let v: Value = resp.json().await?;
        let tok = v
            .get("guest_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| anyhow!("guest token ausente"))?
            .to_string();
        *g = Some((tok.clone(), Instant::now()));
        Ok(tok)
    }

    async fn tx_header(&self, method: &str, path: &str) -> Option<String> {
        if !self.authed() {
            return None;
        }
        let key = hash_str(self.cookie.as_deref().unwrap_or(""));
        let mut slot = TXID.lock().await;
        let fresh = match slot.as_ref() {
            Some((k, _, at)) => *k == key && at.elapsed() < Duration::from_secs(3600),
            None => false,
        };
        if !fresh {
            let gen = match TxIdGen::create(&self.http, self.cookie.as_deref()).await {
                Ok(g) => Some(g),
                Err(e) => {
                    tracing::debug!("[x] sem x-client-transaction-id: {}", e);
                    None
                }
            };
            *slot = Some((key, gen, Instant::now()));
        }
        slot.as_ref()
            .and_then(|(_, g, _)| g.as_ref())
            .map(|g| g.calc(method, path))
    }

    async fn headers(&self, method: &str, path: &str) -> anyhow::Result<HeaderMap> {
        let mut h = HeaderMap::new();
        h.insert("Authorization", HeaderValue::from_static(BEARER));
        h.insert("x-twitter-active-user", HeaderValue::from_static("yes"));
        h.insert("x-twitter-client-language", HeaderValue::from_static("en"));
        h.insert("Accept", HeaderValue::from_static("*/*"));
        h.insert(
            "Accept-Language",
            HeaderValue::from_static("en-US,en;q=0.9"),
        );
        h.insert("Origin", HeaderValue::from_static("https://x.com"));
        h.insert("Referer", HeaderValue::from_static("https://x.com/"));
        if self.authed() {
            h.insert(
                "Cookie",
                HeaderValue::from_str(self.cookie.as_deref().unwrap_or(""))?,
            );
            if let Some(ct0) = self.ct0() {
                h.insert("x-csrf-token", HeaderValue::from_str(&ct0)?);
            }
            h.insert(
                "x-twitter-auth-type",
                HeaderValue::from_static("OAuth2Session"),
            );
            if let Some(tx) = self.tx_header(method, path).await {
                if let Ok(v) = HeaderValue::from_str(&tx) {
                    h.insert("x-client-transaction-id", v);
                }
            }
        } else {
            let tok = self.guest_token(false).await?;
            h.insert("x-guest-token", HeaderValue::from_str(&tok)?);
            h.insert(
                "Cookie",
                HeaderValue::from_str(&format!("guest_id=v1%3A{}", tok))?,
            );
        }
        Ok(h)
    }

    fn gql_url(&self, id: &str, op: &str) -> (String, String) {
        let path = format!("/i/api/graphql/{}/{}", id, op);
        if self.authed() {
            (format!("https://x.com{}", path), path)
        } else {
            (format!("https://api.x.com/graphql/{}/{}", id, op), path)
        }
    }

    async fn check(resp: reqwest::Response, op: &str) -> anyhow::Result<Result<Value, String>> {
        let status = resp.status();
        if status.as_u16() == 429 {
            let reset = resp
                .headers()
                .get("x-rate-limit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<i64>().ok())
                .map(|r| (r - chrono::Utc::now().timestamp()).max(1))
                .unwrap_or(900);
            return Err(anyhow!("X_RATE_LIMIT:{}", reset));
        }
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 404 {
            return Ok(Err("not_found".into()));
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Ok(Err(format!("auth:{}", status.as_u16())));
        }
        if !status.is_success() {
            return Err(anyhow!(
                "X {}: HTTP {} {}",
                op,
                status,
                text.chars().take(200).collect::<String>()
            ));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| anyhow!("X {}: resposta invalida ({})", op, e))?;
        if v.get("data")
            .map(|d| d.is_null() || d.as_object().map(|o| o.is_empty()).unwrap_or(false))
            .unwrap_or(true)
        {
            if let Some(msg) = v
                .get("errors")
                .and_then(|e| e.as_array())
                .and_then(|a| a.first())
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
            {
                if msg.contains("Query not found") || msg.contains("query_id") {
                    return Ok(Err("not_found".into()));
                }
                return Err(anyhow!("X {}: {}", op, msg));
            }
        }
        Ok(Ok(v))
    }

    async fn refresh_ids(&self) -> anyhow::Result<()> {
        super::query_ids::refresh(&self.http, self.cookie.as_deref())
            .await
            .map(|_| ())
    }

    pub async fn gql_get(
        &self,
        op: &str,
        variables: Value,
        extra_features: Value,
        field_toggles: Option<Value>,
    ) -> anyhow::Result<Value> {
        let mut features = base_features();
        if let (Some(base), Some(extra)) = (features.as_object_mut(), extra_features.as_object()) {
            for (k, v) in extra {
                base.insert(k.clone(), v.clone());
            }
        }
        let mut tries = 0;
        loop {
            tries += 1;
            let id = super::query_ids::id_for(op)
                .ok_or_else(|| anyhow!("operacao {} desconhecida", op))?;
            let (url, path) = self.gql_url(&id, op);
            let mut query: Vec<(&str, String)> = vec![
                ("variables", variables.to_string()),
                ("features", features.to_string()),
            ];
            if let Some(ft) = &field_toggles {
                query.push(("fieldToggles", ft.to_string()));
            }
            let headers = self.headers("GET", &path).await?;
            let resp = self
                .http
                .get(&url)
                .headers(headers)
                .query(&query)
                .send()
                .await?;
            match Self::check(resp, op).await? {
                Ok(v) => return Ok(v),
                Err(kind) if tries < 3 => {
                    if kind == "not_found" {
                        tracing::info!("[x] {} 404: recarregando query ids", op);
                        self.refresh_ids().await?;
                    } else if !self.authed() {
                        self.guest_token(true).await?;
                    } else {
                        return Err(anyhow!(
                            "X {}: HTTP {} (sessao expirada? entre de novo no X)",
                            op,
                            kind.trim_start_matches("auth:")
                        ));
                    }
                }
                Err(kind) => return Err(anyhow!("X {}: {}", op, kind)),
            }
        }
    }

    pub async fn gql_post(
        &self,
        op: &str,
        variables: Value,
        features: Option<Value>,
    ) -> anyhow::Result<Value> {
        self.require_login()?;
        let mut tries = 0;
        loop {
            tries += 1;
            let id = super::query_ids::id_for(op)
                .ok_or_else(|| anyhow!("operacao {} desconhecida", op))?;
            let (url, path) = self.gql_url(&id, op);
            let mut body = json!({ "variables": variables, "queryId": id });
            if let Some(f) = &features {
                body["features"] = f.clone();
            }
            let headers = self.headers("POST", &path).await?;
            let resp = self
                .http
                .post(&url)
                .headers(headers)
                .json(&body)
                .send()
                .await?;
            match Self::check(resp, op).await? {
                Ok(v) => return Ok(v),
                Err(kind) if kind == "not_found" && tries < 2 => self.refresh_ids().await?,
                Err(kind) => return Err(anyhow!("X {}: {}", op, kind)),
            }
        }
    }

    /// REST v1.1 (ex.: `friendships/destroy.json`), corpo em formulario.
    pub async fn rest_post_form(&self, path: &str, form: &[(&str, &str)]) -> anyhow::Result<Value> {
        self.require_login()?;
        let full = format!("/i/api/1.1/{}", path.trim_start_matches('/'));
        let url = format!("https://x.com{}", full);
        let mut headers = self.headers("POST", &full).await?;
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let resp = self
            .http
            .post(&url)
            .headers(headers)
            .form(form)
            .send()
            .await?;
        match Self::check(resp, path).await? {
            Ok(v) => Ok(v),
            Err(kind) => Err(anyhow!("X {}: {}", path, kind)),
        }
    }

    /// POST JSON cru (Grok `add_response.json`); devolve a resposta para o
    /// chamador ler o stream.
    pub async fn post_json_raw(
        &self,
        url: &str,
        body: &Value,
        extra: &[(&str, &str)],
    ) -> anyhow::Result<reqwest::Response> {
        self.require_login()?;
        let path = url::Url::parse(url)
            .map(|u| u.path().to_string())
            .unwrap_or_default();
        let mut headers = self.headers("POST", &path).await?;
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        for (k, v) in extra {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(name, val);
            }
        }
        let resp = self
            .http
            .post(url)
            .headers(headers)
            .json(body)
            .send()
            .await?;
        if resp.status().as_u16() == 429 {
            return Err(anyhow!("X_RATE_LIMIT:60"));
        }
        if !resp.status().is_success() {
            let st = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "X: HTTP {} {}",
                st,
                text.chars().take(300).collect::<String>()
            ));
        }
        Ok(resp)
    }

    /// Pagina uma timeline: `on_page` recebe a resposta e devolve quantos
    /// itens novos extraiu. Para no fim, no limite, em 3 paginas vazias, em
    /// cursor repetido ou quando `job` for cancelado.
    pub async fn paginate<F>(
        &self,
        op: &str,
        mut variables: Value,
        extra_features: Value,
        limit: usize,
        job: &str,
        mut on_page: F,
    ) -> anyhow::Result<usize>
    where
        F: FnMut(&Value) -> usize,
    {
        let mut total = 0usize;
        let mut cursor: Option<String> = None;
        let mut empty = 0;
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        loop {
            if super::cancelled(job) {
                break;
            }
            if let Some(c) = &cursor {
                variables["cursor"] = Value::String(c.clone());
            }
            let page = self
                .gql_get(op, variables.clone(), extra_features.clone(), None)
                .await?;
            let got = on_page(&page);
            total += got;
            let next = super::parse::bottom_cursor(&page);
            let ids = super::parse::entry_ids(&page);
            let stalled = ids.iter().all(|i| seen.contains(i)) && !ids.is_empty();
            seen.extend(ids);
            if got == 0 {
                empty += 1;
            } else {
                empty = 0;
            }
            match next {
                Some(c)
                    if Some(&c) != cursor.as_ref()
                        && empty < 3
                        && !stalled
                        && (limit == 0 || total < limit) =>
                {
                    cursor = Some(c)
                }
                _ => break,
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Ok(total)
    }
}
