//! FxTwitter API v2 (estudo 67): a via pública, sem login nem chave.
//! `https://api.fxtwitter.com/2/...`, 1000 req/min por IP. Tudo que é
//! público no X (post, thread, perfil, posts e mídia de um perfil, busca,
//! trends) sai daqui; a sessão só entra para dados privados.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{XMedia, XPost, XUser};

const BASE: &str = "https://api.fxtwitter.com/2";

fn client() -> anyhow::Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("OmniGet/0.9 (+https://github.com/tonhowtf/omniget)"),
    );
    Ok(
        crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(60))
            .build()?,
    )
}

async fn get(path: &str, query: &[(&str, String)]) -> anyhow::Result<Value> {
    let client = client()?;
    let url = format!("{}/{}", BASE, path.trim_start_matches('/'));
    let mut req = client.get(&url);
    let q: Vec<(&str, &str)> = query
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (*k, v.as_str()))
        .collect();
    if !q.is_empty() {
        req = req.query(&q);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("FxTwitter: resposta invalida ({})", e))?;
    let code = body
        .get("code")
        .and_then(|c| c.as_u64())
        .unwrap_or(status.as_u16() as u64);
    if code == 404 {
        return Err(anyhow!("nao encontrado no X (ou o post e privado)"));
    }
    if code == 401 {
        return Err(anyhow!("post ou perfil privado"));
    }
    if code == 429 {
        return Err(anyhow!(
            "FxTwitter: limite de requisicoes atingido, tente de novo em instantes"
        ));
    }
    if code >= 400 {
        let msg = body
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("erro");
        return Err(anyhow!("FxTwitter: {} ({})", msg, code));
    }
    Ok(body)
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k)
        .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64)))
        .unwrap_or(0)
}

pub fn user_from(v: &Value) -> XUser {
    XUser {
        id: s(v, "id"),
        handle: s(v, "screen_name"),
        name: s(v, "name"),
        avatar: s(v, "avatar_url").replace("_normal.", "_400x400."),
        banner: s(v, "banner_url"),
        bio: s(v, "description"),
        location: s(v, "location"),
        website: v
            .get("website")
            .and_then(|w| w.get("url"))
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string(),
        joined: super::parse_twitter_date(&s(v, "joined"))
            .map(|(iso, _)| iso)
            .unwrap_or_default(),
        followers: n(v, "followers"),
        following: n(v, "following"),
        posts: n(v, "statuses"),
        likes: n(v, "likes"),
        media_count: n(v, "media_count"),
        verified: v
            .get("verification")
            .and_then(|x| x.get("verified"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        protected: v
            .get("protected")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        followed_by_me: None,
        follows_me: None,
    }
}

fn media_from(v: &Value) -> Vec<XMedia> {
    let mut out = Vec::new();
    let Some(media) = v.get("media") else {
        return out;
    };
    for p in media
        .get("photos")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        out.push(XMedia {
            kind: if s(p, "type") == "gif" {
                "gif".into()
            } else {
                "photo".into()
            },
            url: super::photo_orig_url(&s(p, "url")),
            thumb: s(p, "url"),
            width: n(p, "width") as u32,
            height: n(p, "height") as u32,
            duration_ms: 0,
            alt: s(p, "altText"),
        });
    }
    for vd in media
        .get("videos")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        // o melhor mp4 e o de maior bitrate; `url` ja costuma ser ele
        let mut best = s(vd, "url");
        let mut best_rate = 0u64;
        for f in vd
            .get("formats")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
        {
            if s(f, "container") == "mp4" {
                let br = n(f, "bitrate");
                if br > best_rate {
                    best_rate = br;
                    best = s(f, "url");
                }
            }
        }
        out.push(XMedia {
            kind: if s(vd, "type") == "gif" {
                "gif".into()
            } else {
                "video".into()
            },
            url: best,
            thumb: s(vd, "thumbnail_url"),
            width: n(vd, "width") as u32,
            height: n(vd, "height") as u32,
            duration_ms: (vd.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0) * 1000.0)
                as u64,
            alt: String::new(),
        });
    }
    out
}

fn extract_entities(text: &str, raw: &Value) -> (Vec<String>, Vec<String>, Vec<String>) {
    let re_tag = regex::Regex::new(r"#([\p{L}\p{N}_]+)").unwrap();
    let re_at = regex::Regex::new(r"@([A-Za-z0-9_]{1,15})").unwrap();
    let hashtags = re_tag
        .captures_iter(text)
        .map(|c| c[1].to_string())
        .collect();
    let mentions = re_at
        .captures_iter(text)
        .map(|c| c[1].to_string())
        .collect();
    let mut links = Vec::new();
    for f in raw
        .get("facets")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        if s(f, "type") == "url" {
            let r = s(f, "replacement");
            if !r.is_empty() {
                links.push(r);
            }
        }
    }
    (hashtags, mentions, links)
}

pub fn post_from(v: &Value) -> Option<XPost> {
    if v.get("type").and_then(|t| t.as_str()) == Some("tombstone") {
        return None;
    }
    let id = s(v, "id");
    if id.is_empty() {
        return None;
    }
    let text = s(v, "text");
    let raw = v.get("raw_text").cloned().unwrap_or(Value::Null);
    let (hashtags, mentions, links) = extract_entities(&text, &raw);
    let ts = v
        .get("created_timestamp")
        .and_then(|t| t.as_i64())
        .unwrap_or(0);
    let created_at = if ts > 0 {
        super::iso_from_timestamp(ts)
    } else {
        super::parse_twitter_date(&s(v, "created_at"))
            .map(|(iso, _)| iso)
            .unwrap_or_default()
    };
    let replying = v.get("replying_to").filter(|r| r.is_object());
    Some(XPost {
        id: id.clone(),
        url: if s(v, "url").is_empty() {
            format!("https://x.com/i/status/{}", id)
        } else {
            s(v, "url")
        },
        text,
        created_at,
        timestamp: ts,
        author: v.get("author").map(user_from).unwrap_or_default(),
        likes: n(v, "likes"),
        reposts: n(v, "reposts"),
        replies: n(v, "replies"),
        quotes: n(v, "quotes"),
        views: n(v, "views"),
        bookmarks: n(v, "bookmarks"),
        lang: s(v, "lang"),
        media: media_from(v),
        quote: v.get("quote").and_then(post_from).map(Box::new),
        reply_to_id: replying.map(|r| s(r, "status")).filter(|x| !x.is_empty()),
        reply_to_handle: replying
            .map(|r| s(r, "screen_name"))
            .filter(|x| !x.is_empty()),
        conversation_id: None,
        reposted_by: v
            .get("reposted_by")
            .filter(|r| r.is_object())
            .map(|r| s(r, "screen_name")),
        source: s(v, "source"),
        hashtags,
        mentions,
        links,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub cursor: Option<String>,
}

fn bottom_cursor(v: &Value) -> Option<String> {
    v.get("cursor")
        .and_then(|c| {
            c.get("bottom")
                .and_then(|b| b.as_str())
                .map(|x| x.to_string())
                .or_else(|| c.as_str().map(|x| x.to_string()))
        })
        .filter(|c| !c.is_empty())
}

fn results_page(v: &Value) -> Page<XPost> {
    let items = v
        .get("results")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(post_from)
        .collect();
    Page {
        items,
        cursor: bottom_cursor(v),
    }
}

pub async fn status(id: &str) -> anyhow::Result<XPost> {
    let v = get(&format!("status/{}", id), &[]).await?;
    v.get("status")
        .and_then(post_from)
        .ok_or_else(|| anyhow!("post indisponivel"))
}

/// Thread do autor (o proprio FxTwitter monta a sequencia). Devolve o post
/// focal e a lista completa (vazia quando nao e thread).
pub async fn thread(id: &str) -> anyhow::Result<(XPost, Vec<XPost>, bool)> {
    let v = get(&format!("thread/{}", id), &[]).await?;
    let focal = v
        .get("status")
        .and_then(post_from)
        .ok_or_else(|| anyhow!("post indisponivel"))?;
    let (posts, truncated) = match v.get("thread") {
        Some(t) if t.is_object() => (
            t.get("statuses")
                .and_then(|a| a.as_array())
                .into_iter()
                .flatten()
                .filter_map(post_from)
                .collect(),
            t.get("truncated")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
        ),
        Some(t) if t.is_array() => (
            t.as_array().unwrap().iter().filter_map(post_from).collect(),
            false,
        ),
        _ => (Vec::new(), false),
    };
    Ok((focal, posts, truncated))
}

pub async fn conversation(id: &str, cursor: Option<&str>) -> anyhow::Result<Page<XPost>> {
    let v = get(
        &format!("conversation/{}", id),
        &[("cursor", cursor.unwrap_or("").to_string())],
    )
    .await?;
    let items = v
        .get("replies")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(post_from)
        .collect();
    Ok(Page {
        items,
        cursor: bottom_cursor(&v),
    })
}

pub async fn profile(handle: &str) -> anyhow::Result<XUser> {
    let v = get(&format!("profile/{}", handle), &[]).await?;
    v.get("user")
        .map(user_from)
        .ok_or_else(|| anyhow!("perfil indisponivel"))
}

pub async fn profile_statuses(
    handle: &str,
    cursor: Option<&str>,
    with_replies: bool,
) -> anyhow::Result<Page<XPost>> {
    let v = get(
        &format!("profile/{}/statuses", handle),
        &[
            ("cursor", cursor.unwrap_or("").to_string()),
            ("count", "100".to_string()),
            (
                "with_replies",
                if with_replies {
                    "true".to_string()
                } else {
                    String::new()
                },
            ),
        ],
    )
    .await?;
    Ok(results_page(&v))
}

pub async fn profile_media(handle: &str, cursor: Option<&str>) -> anyhow::Result<Page<XPost>> {
    let v = get(
        &format!("profile/{}/media", handle),
        &[
            ("cursor", cursor.unwrap_or("").to_string()),
            ("count", "100".to_string()),
        ],
    )
    .await?;
    Ok(results_page(&v))
}

pub async fn profile_relationship(
    handle: &str,
    which: &str,
    cursor: Option<&str>,
) -> anyhow::Result<Page<XUser>> {
    let v = get(
        &format!("profile/{}/{}", handle, which),
        &[("cursor", cursor.unwrap_or("").to_string())],
    )
    .await?;
    let items = v
        .get("results")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .map(user_from)
        .collect();
    Ok(Page {
        items,
        cursor: bottom_cursor(&v),
    })
}

/// `feed`: "latest" | "top" (o que a busca do X chama de Latest/Top).
pub async fn search(q: &str, feed: &str, cursor: Option<&str>) -> anyhow::Result<Page<XPost>> {
    let v = get(
        "search",
        &[
            ("q", q.to_string()),
            ("feed", feed.to_string()),
            ("count", "50".to_string()),
            ("cursor", cursor.unwrap_or("").to_string()),
        ],
    )
    .await?;
    Ok(results_page(&v))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trend {
    pub name: String,
    pub context: String,
    pub rank: Option<u64>,
}

pub async fn trends() -> anyhow::Result<Vec<Trend>> {
    let v = get("trends", &[]).await?;
    Ok(v.get("trends")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .map(|t| Trend {
            name: s(t, "name"),
            context: s(t, "context"),
            rank: t.get("rank").and_then(|r| r.as_u64()),
        })
        .collect())
}
