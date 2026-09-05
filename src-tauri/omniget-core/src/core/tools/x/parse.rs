//! Parser das respostas GraphQL do X (estudo 67). Segue o formato que o
//! twscrape (MIT) e o XActions (Apache-2) leem: `instructions[].entries[]`,
//! `itemContent.tweet_results.result` / `user_results.result`, cursores
//! `cursorType: "Bottom"`, `TweetWithVisibilityResults`, note tweets.

use serde_json::Value;

use super::{XMedia, XPost, XUser};

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn n(v: &Value, k: &str) -> u64 {
    match v.get(k) {
        Some(Value::Number(x)) => x.as_u64().unwrap_or(0),
        Some(Value::String(x)) => x.parse().unwrap_or(0),
        _ => 0,
    }
}

fn b(v: &Value, k: &str) -> Option<bool> {
    v.get(k).and_then(|x| x.as_bool())
}

pub fn parse_user(result: &Value) -> Option<XUser> {
    let legacy = result.get("legacy").cloned().unwrap_or(Value::Null);
    let core = result.get("core").cloned().unwrap_or(Value::Null);
    let handle = {
        let h = s(&core, "screen_name");
        if h.is_empty() {
            s(&legacy, "screen_name")
        } else {
            h
        }
    };
    if handle.is_empty() {
        return None;
    }
    let name = {
        let x = s(&core, "name");
        if x.is_empty() {
            s(&legacy, "name")
        } else {
            x
        }
    };
    let created = {
        let x = s(&core, "created_at");
        if x.is_empty() {
            s(&legacy, "created_at")
        } else {
            x
        }
    };
    let avatar = result
        .get("avatar")
        .and_then(|a| a.get("image_url"))
        .and_then(|u| u.as_str())
        .map(|u| u.to_string())
        .unwrap_or_else(|| s(&legacy, "profile_image_url_https"));
    let rel = result
        .get("relationship_perspectives")
        .cloned()
        .unwrap_or(Value::Null);
    let counts = result
        .get("relationship_counts")
        .cloned()
        .unwrap_or(Value::Null);
    let tweet_counts = result.get("tweet_counts").cloned().unwrap_or(Value::Null);
    let bio = result.get("profile_bio").cloned().unwrap_or(Value::Null);
    let website = bio
        .pointer("/entities/url/urls")
        .or_else(|| legacy.pointer("/entities/url/urls"))
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .map(|u| s(u, "expanded_url"))
        .unwrap_or_default();
    let pick = |new: u64, old: u64| if new > 0 { new } else { old };
    Some(XUser {
        id: s(result, "rest_id"),
        handle,
        name,
        avatar: avatar.replace("_normal.", "_400x400."),
        banner: result
            .get("banner")
            .map(|b| s(b, "image_url"))
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| s(&legacy, "profile_banner_url")),
        bio: {
            let b = s(&bio, "description");
            if b.is_empty() {
                s(&legacy, "description")
            } else {
                b
            }
        },
        location: {
            let l = result
                .get("location")
                .map(|l| s(l, "location"))
                .unwrap_or_default();
            if l.is_empty() {
                s(&legacy, "location")
            } else {
                l
            }
        },
        website,
        joined: super::parse_twitter_date(&created)
            .map(|(iso, _)| iso)
            .unwrap_or_default(),
        followers: pick(n(&counts, "followers"), n(&legacy, "followers_count")),
        following: pick(n(&counts, "following"), n(&legacy, "friends_count")),
        posts: pick(n(&tweet_counts, "tweets"), n(&legacy, "statuses_count")),
        likes: pick(
            result
                .get("action_counts")
                .map(|a| n(a, "favorites_count"))
                .unwrap_or(0),
            n(&legacy, "favourites_count"),
        ),
        media_count: pick(n(&tweet_counts, "media_tweets"), n(&legacy, "media_count")),
        verified: b(result, "is_blue_verified").unwrap_or(false)
            || result
                .get("verification")
                .and_then(|v| v.get("verified"))
                .and_then(|x| x.as_bool())
                .unwrap_or(false)
            || b(&legacy, "verified").unwrap_or(false),
        protected: result
            .get("privacy")
            .and_then(|p| p.get("protected"))
            .and_then(|x| x.as_bool())
            .or_else(|| b(&legacy, "protected"))
            .unwrap_or(false),
        followed_by_me: b(&rel, "following").or_else(|| b(&legacy, "following")),
        follows_me: b(&rel, "followed_by").or_else(|| b(&legacy, "followed_by")),
    })
}

fn media_from_legacy(legacy: &Value) -> Vec<XMedia> {
    let arr = legacy
        .get("extended_entities")
        .and_then(|e| e.get("media"))
        .or_else(|| legacy.get("entities").and_then(|e| e.get("media")))
        .and_then(|m| m.as_array());
    let mut out = Vec::new();
    for m in arr.into_iter().flatten() {
        let kind = s(m, "type");
        let (w, h) = m
            .get("original_info")
            .map(|o| (n(o, "width") as u32, n(o, "height") as u32))
            .unwrap_or((0, 0));
        let alt = s(m, "ext_alt_text");
        let thumb = s(m, "media_url_https");
        match kind.as_str() {
            "photo" => out.push(XMedia {
                kind: "photo".into(),
                url: super::photo_orig_url(&thumb),
                thumb,
                width: w,
                height: h,
                duration_ms: 0,
                alt,
            }),
            "video" | "animated_gif" => {
                let info = m.get("video_info").cloned().unwrap_or(Value::Null);
                let mut best = String::new();
                let mut best_rate = 0u64;
                for v in info
                    .get("variants")
                    .and_then(|a| a.as_array())
                    .into_iter()
                    .flatten()
                {
                    if s(v, "content_type") == "video/mp4" {
                        let br = n(v, "bitrate");
                        if br >= best_rate {
                            best_rate = br;
                            best = s(v, "url");
                        }
                    }
                }
                if best.is_empty() {
                    continue;
                }
                out.push(XMedia {
                    kind: if kind == "video" {
                        "video".into()
                    } else {
                        "gif".into()
                    },
                    url: best,
                    thumb,
                    width: w,
                    height: h,
                    duration_ms: n(&info, "duration_millis"),
                    alt,
                });
            }
            _ => {}
        }
    }
    out
}

/// `tweet_results.result` → post. Reposts viram o post original com
/// `reposted_by`; `TweetWithVisibilityResults` e desembrulhado.
pub fn parse_tweet(result: &Value) -> Option<XPost> {
    let typename = s(result, "__typename");
    if typename == "TweetTombstone" || typename == "TweetUnavailable" {
        return None;
    }
    let tweet = if typename == "TweetWithVisibilityResults" {
        result.get("tweet")?
    } else {
        result
    };
    let legacy = tweet.get("legacy")?;
    if let Some(rt) = legacy
        .get("retweeted_status_result")
        .and_then(|r| r.get("result"))
    {
        let mut inner = parse_tweet(rt)?;
        let by = tweet
            .get("core")
            .and_then(|c| c.get("user_results"))
            .and_then(|u| u.get("result"))
            .and_then(parse_user);
        inner.reposted_by = by.map(|u| u.handle);
        return Some(inner);
    }
    let id = {
        let x = s(tweet, "rest_id");
        if x.is_empty() {
            s(legacy, "id_str")
        } else {
            x
        }
    };
    if id.is_empty() {
        return None;
    }
    let author = tweet
        .get("core")
        .and_then(|c| c.get("user_results"))
        .and_then(|u| u.get("result"))
        .and_then(parse_user)
        .unwrap_or_default();
    let mut text = tweet
        .get("note_tweet")
        .and_then(|nt| nt.get("note_tweet_results"))
        .and_then(|r| r.get("result"))
        .map(|r| s(r, "text"))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| s(legacy, "full_text"));
    let mut links = Vec::new();
    for u in legacy
        .get("entities")
        .and_then(|e| e.get("urls"))
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        let short = s(u, "url");
        let long = s(u, "expanded_url");
        if !short.is_empty() && !long.is_empty() {
            text = text.replace(&short, &long);
            links.push(long);
        }
    }
    // remove o t.co da midia no fim do texto
    for m in legacy
        .get("extended_entities")
        .and_then(|e| e.get("media"))
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        let short = s(m, "url");
        if !short.is_empty() {
            text = text.replace(&short, "").trim_end().to_string();
        }
    }
    let text = html_unescape(&text);
    let (created_at, ts) = super::parse_twitter_date(&s(legacy, "created_at")).unwrap_or_default();
    let views = tweet.get("views").map(|v| n(v, "count")).unwrap_or(0);
    let re_tag = regex::Regex::new(r"#([\p{L}\p{N}_]+)").unwrap();
    let re_at = regex::Regex::new(r"@([A-Za-z0-9_]{1,15})").unwrap();
    let source = regex::Regex::new(r"<[^>]+>")
        .unwrap()
        .replace_all(&s(tweet, "source"), "")
        .to_string();
    Some(XPost {
        url: format!(
            "https://x.com/{}/status/{}",
            if author.handle.is_empty() {
                "i"
            } else {
                &author.handle
            },
            id
        ),
        id,
        hashtags: re_tag
            .captures_iter(&text)
            .map(|c| c[1].to_string())
            .collect(),
        mentions: re_at
            .captures_iter(&text)
            .map(|c| c[1].to_string())
            .collect(),
        text,
        created_at,
        timestamp: ts,
        author,
        likes: n(legacy, "favorite_count"),
        reposts: n(legacy, "retweet_count"),
        replies: n(legacy, "reply_count"),
        quotes: n(legacy, "quote_count"),
        views,
        bookmarks: n(legacy, "bookmark_count"),
        lang: s(legacy, "lang"),
        media: media_from_legacy(legacy),
        quote: tweet
            .get("quoted_status_result")
            .and_then(|q| q.get("result"))
            .and_then(parse_tweet)
            .map(Box::new),
        reply_to_id: Some(s(legacy, "in_reply_to_status_id_str")).filter(|x| !x.is_empty()),
        reply_to_handle: Some(s(legacy, "in_reply_to_screen_name")).filter(|x| !x.is_empty()),
        conversation_id: Some(s(legacy, "conversation_id_str")).filter(|x| !x.is_empty()),
        reposted_by: None,
        source,
        links,
    })
}

fn html_unescape(t: &str) -> String {
    t.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Todos os `itemContent` de uma resposta de timeline, na ordem do documento
/// (entries diretas e items de modulos).
fn item_contents(v: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<Value>) {
        match v {
            Value::Object(map) => {
                if let Some(ic) = map.get("itemContent") {
                    out.push(ic.clone());
                }
                for (_, child) in map {
                    walk(child, out);
                }
            }
            Value::Array(arr) => {
                for child in arr {
                    walk(child, out);
                }
            }
            _ => {}
        }
    }
    walk(v, &mut out);
    out
}

pub fn tweets_from(v: &Value) -> Vec<XPost> {
    let mut out: Vec<XPost> = item_contents(v)
        .iter()
        .filter_map(|ic| {
            ic.get("tweet_results")
                .and_then(|t| t.get("result"))
                .and_then(parse_tweet)
        })
        .collect();
    if out.is_empty() {
        // respostas de um unico post (`data.tweetResult.result`)
        if let Some(r) = v.pointer("/data/tweetResult/result").and_then(parse_tweet) {
            out.push(r);
        }
    }
    super::dedup_posts(out)
}

pub fn users_from(v: &Value) -> Vec<XUser> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<XUser> = item_contents(v)
        .iter()
        .filter_map(|ic| {
            ic.get("user_results")
                .and_then(|u| u.get("result"))
                .and_then(parse_user)
        })
        .filter(|u| seen.insert(u.id.clone()))
        .collect();
    if out.is_empty() {
        if let Some(u) = v.pointer("/data/user/result").and_then(parse_user) {
            out.push(u);
        }
    }
    out
}

/// Cursor de "mais" (`cursorType: Bottom`); as comunidades usam `next_cursor`.
pub fn bottom_cursor(v: &Value) -> Option<String> {
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::Object(map) => {
                if map.get("cursorType").and_then(|c| c.as_str()) == Some("Bottom") {
                    if let Some(val) = map.get("value").and_then(|x| x.as_str()) {
                        return Some(val.to_string());
                    }
                }
                if let Some(val) = map.get("next_cursor").and_then(|x| x.as_str()) {
                    return Some(val.to_string());
                }
                for (_, child) in map {
                    if let Some(c) = walk(child) {
                        return Some(c);
                    }
                }
                None
            }
            Value::Array(arr) => arr.iter().find_map(walk),
            _ => None,
        }
    }
    walk(v).filter(|c| !c.is_empty())
}

/// Ids dos `entries` com conteudo (sem cursores e modulos vazios), para
/// detectar pagina repetida.
pub fn entry_ids(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                if let Some(id) = map.get("entryId").and_then(|x| x.as_str()) {
                    if !id.starts_with("cursor-")
                        && !id.starts_with("who-to-follow")
                        && !id.starts_with("messageprompt")
                    {
                        out.push(id.to_string());
                    }
                }
                for (_, child) in map {
                    walk(child, out);
                }
            }
            Value::Array(arr) => arr.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    walk(v, &mut out);
    out
}
