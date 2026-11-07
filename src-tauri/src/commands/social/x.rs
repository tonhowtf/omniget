use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use omniget_core::platforms::twitter::{BEARER, TWEET_FEATURES};
use serde_json::{json, Map, Value};

use super::args::{opt_str, req_str, undo};
use super::model::{
    b, opt_s, page, s, u, Author, Media, MediaKind, Platform, Post, Profile, Stats,
};
use super::page as web;

const P: Platform = Platform::X;
const API: &str = "https://x.com/i/api";

#[derive(Clone, Debug)]
struct Op {
    id: String,
    features: Vec<String>,
    toggles: Vec<String>,
}

struct Ops {
    at: Instant,
    ops: HashMap<String, Op>,
    config: Map<String, Value>,
}

fn ops_cache() -> &'static Mutex<Option<Ops>> {
    static C: OnceLock<Mutex<Option<Ops>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

const DISCOVER: &str = r#"(async()=>{
const urls=new Set();
for(const s of document.querySelectorAll('script[src]'))urls.add(s.src);
for(const e of performance.getEntriesByType('resource'))urls.add(e.name);
const js=[...urls].filter(u=>/abs\.twimg\.com\/responsive-web\/client-web[^?]*\.js$/.test(u)).slice(0,60);
const ops={};
const re=/queryId:"([^"]+)",operationName:"([^"]+)",operationType:"[^"]+",metadata:\{featureSwitches:\[([^\]]*)\],fieldToggles:\[([^\]]*)\]/g;
const list=x=>x.split(',').map(v=>v.replace(/"/g,'').trim()).filter(Boolean);
const loose=/queryId:"([^"]+)",operationName:"([^"]+)"/g;
await Promise.all(js.map(async u=>{try{const t=await (await fetch(u)).text();let m;while((m=loose.exec(t))){if(!ops[m[2]])ops[m[2]]={id:m[1],features:[],toggles:[]};}while((m=re.exec(t))){ops[m[2]]={id:m[1],features:list(m[3]),toggles:list(m[4])};}}catch(e){}}));
const fs=(window.__INITIAL_STATE__&&window.__INITIAL_STATE__.featureSwitch)||{};
const config={};
for(const src of [fs.defaultConfig||{},(fs.user&&fs.user.config)||{}])for(const k in src){const v=src[k]&&src[k].value;if(typeof v==='boolean')config[k]=v;}
return {ops,config};
})()"#;

const FALLBACK_TWEET_DETAIL: &str = "4Siu98E55GquhG52zHdY5w";

async fn op(name: &str) -> Result<(Op, Map<String, Value>), String> {
    {
        let c = ops_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = c
            .as_ref()
            .filter(|c| c.at.elapsed() < Duration::from_secs(6 * 3600))
        {
            if let Some(o) = c.ops.get(name) {
                return Ok((o.clone(), c.config.clone()));
            }
        }
    }
    let b = web::ready(P).await?;
    let found = web::eval(&b, P, DISCOVER).await.unwrap_or(Value::Null);
    let mut ops = HashMap::new();
    if let Some(map) = found["ops"].as_object() {
        for (k, v) in map {
            let list = |key: &str| -> Vec<String> {
                v[key]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            if let Some(id) = v["id"].as_str() {
                ops.insert(
                    k.clone(),
                    Op {
                        id: id.to_string(),
                        features: list("features"),
                        toggles: list("toggles"),
                    },
                );
            }
        }
    }
    let config = found["config"].as_object().cloned().unwrap_or_default();
    let result = ops.get(name).cloned().or_else(|| {
        (name == "TweetDetail").then(|| Op {
            id: FALLBACK_TWEET_DETAIL.into(),
            features: Vec::new(),
            toggles: Vec::new(),
        })
    });
    if !ops.is_empty() {
        *ops_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some(Ops {
            at: Instant::now(),
            ops,
            config: config.clone(),
        });
    }
    result.map(|o| (o, config)).ok_or_else(|| {
        format!("UPSTREAM_CHANGED: could not find the X operation {name} in the web app; reload x.com in the OmniGet browser and try again")
    })
}

fn features(o: &Op, config: &Map<String, Value>) -> Value {
    let base: Map<String, Value> = serde_json::from_str(TWEET_FEATURES).unwrap_or_default();
    if o.features.is_empty() {
        return Value::Object(base);
    }
    let mut out = Map::new();
    for f in &o.features {
        let v = config
            .get(f)
            .or_else(|| base.get(f))
            .cloned()
            .unwrap_or(Value::Bool(true));
        out.insert(f.clone(), v);
    }
    Value::Object(out)
}

fn toggles(o: &Op) -> Value {
    let mut out = Map::new();
    for t in &o.toggles {
        let on = !matches!(
            t.as_str(),
            "withArticlePlainText" | "withGrokAnalyze" | "withDisallowedReplyControls"
        );
        out.insert(t.clone(), Value::Bool(on));
    }
    Value::Object(out)
}

async fn base_headers() -> Result<Map<String, Value>, String> {
    let b = web::ready(P).await?;
    let ct0 = web::cookie(&b, P, "ct0")
        .await
        .ok_or("NOT_LOGGED_IN: the X session has no ct0 cookie; sign in again")?;
    Ok(web::headers(&[
        ("authorization", BEARER),
        ("x-csrf-token", &ct0),
        ("x-twitter-auth-type", "OAuth2Session"),
        ("x-twitter-active-user", "yes"),
        ("x-twitter-client-language", "en"),
        ("content-type", "application/json"),
    ]))
}

fn check_errors(v: &Value) -> Result<(), String> {
    if v.get("data")
        .is_some_and(|d| !d.is_null() && d.as_object().is_some_and(|o| !o.is_empty()))
    {
        return Ok(());
    }
    if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
        return Err(format!(
            "REJECTED: X refused the request: {} (code {})",
            s(&e["message"]),
            s(&e["code"])
        ));
    }
    Ok(())
}

async fn gql(name: &str, variables: Value, mutation: bool) -> Result<Value, String> {
    let (o, config) = op(name).await?;
    let headers = base_headers().await?;
    let feats = features(&o, &config);
    let url = format!("{API}/graphql/{}/{name}", o.id);
    let post_body = || {
        let mut body = json!({ "variables": variables, "queryId": o.id });
        if !mutation || !o.features.is_empty() {
            body["features"] = feats.clone();
        }
        if !o.toggles.is_empty() {
            body["fieldToggles"] = toggles(&o);
        }
        body.to_string()
    };
    let reply = if mutation {
        web::fetch(P, "POST", &url, headers, Some(post_body())).await?
    } else {
        let mut q = vec![
            ("variables", variables.to_string()),
            ("features", feats.to_string()),
        ];
        if !o.toggles.is_empty() {
            q.push(("fieldToggles", toggles(&o).to_string()));
        }
        match web::fetch(
            P,
            "GET",
            &format!("{url}?{}", web::query(&q)),
            headers.clone(),
            None,
        )
        .await
        {
            Ok(r) => r,
            Err(e)
                if e.starts_with("UPSTREAM_404")
                    || e.starts_with("UPSTREAM_405")
                    || e.starts_with("UPSTREAM_422") =>
            {
                web::fetch(P, "POST", &url, headers, Some(post_body())).await?
            }
            Err(e) => return Err(e),
        }
    };
    let v = reply.json()?;
    check_errors(&v)?;
    Ok(v)
}

fn created(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_str(v.as_str()?, "%a %b %d %H:%M:%S %z %Y")
        .ok()
        .map(|d| d.timestamp())
}

pub fn author(r: &Value) -> Author {
    let r = if r["__typename"] == "UserUnavailable" {
        &Value::Null
    } else {
        r
    };
    let l = &r["legacy"];
    let handle = opt_s(&r["core"]["screen_name"]).unwrap_or_else(|| s(&l["screen_name"]));
    Author {
        id: s(&r["rest_id"]),
        name: opt_s(&r["core"]["name"]).unwrap_or_else(|| s(&l["name"])),
        handle,
        avatar: opt_s(&r["avatar"]["image_url"])
            .or_else(|| opt_s(&l["profile_image_url_https"]))
            .map(|a| a.replace("_normal.", "_bigger.")),
        verified: b(&r["is_blue_verified"])
            || b(&l["verified"])
            || b(&r["verification"]["verified"]),
    }
}

pub fn profile(r: &Value) -> Option<Profile> {
    if r["rest_id"].is_null() {
        return None;
    }
    let a = author(r);
    let l = &r["legacy"];
    let rel = &r["relationship_perspectives"];
    Some(Profile {
        url: format!("https://x.com/{}", a.handle),
        id: a.id,
        handle: a.handle,
        name: a.name,
        avatar: a.avatar.map(|x| x.replace("_bigger.", "_400x400.")),
        bio: opt_s(&r["profile_bio"]["description"]).unwrap_or_else(|| s(&l["description"])),
        verified: a.verified,
        private: b(&l["protected"]) || b(&r["privacy"]["protected"]),
        followers: u(&l["followers_count"]),
        following: u(&l["friends_count"]),
        posts: u(&l["statuses_count"]),
        likes: u(&l["favourites_count"]),
        followed_by_me: rel["following"]
            .as_bool()
            .or_else(|| l["following"].as_bool()),
        follows_me: rel["followed_by"]
            .as_bool()
            .or_else(|| l["followed_by"].as_bool()),
        secondary_id: None,
    })
}

fn media(l: &Value) -> Vec<Media> {
    let Some(items) = l["extended_entities"]["media"]
        .as_array()
        .or_else(|| l["entities"]["media"].as_array())
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|m| {
            let thumb = opt_s(&m["media_url_https"]);
            let width = u(&m["original_info"]["width"]);
            let height = u(&m["original_info"]["height"]);
            match m["type"].as_str()? {
                "photo" => Some(Media {
                    kind: MediaKind::Image,
                    url: thumb.clone()?,
                    thumb: None,
                    width,
                    height,
                }),
                "video" | "animated_gif" => {
                    let best = m["video_info"]["variants"]
                        .as_array()?
                        .iter()
                        .filter(|v| v["content_type"] == "video/mp4")
                        .max_by_key(|v| u(&v["bitrate"]).unwrap_or(0))?;
                    Some(Media {
                        kind: MediaKind::Video,
                        url: s(&best["url"]),
                        thumb,
                        width,
                        height,
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn clean_text(l: &Value, text: String) -> String {
    let mut t = text;
    if let Some(urls) = l["entities"]["urls"].as_array() {
        for x in urls {
            if let (Some(short), Some(long)) = (x["url"].as_str(), x["expanded_url"].as_str()) {
                t = t.replace(short, long);
            }
        }
    }
    if let Some(ms) = l["entities"]["media"].as_array() {
        for x in ms {
            if let Some(short) = x["url"].as_str() {
                t = t.replace(short, "");
            }
        }
    }
    t.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

pub fn tweet(v: &Value) -> Option<Post> {
    let r = if v["__typename"] == "TweetWithVisibilityResults" {
        &v["tweet"]
    } else {
        v
    };
    let l = &r["legacy"];
    if l.is_null() {
        return None;
    }
    let author = author(&r["core"]["user_results"]["result"]);
    let rt = &l["retweeted_status_result"]["result"];
    if !rt.is_null() {
        if let Some(mut inner) = tweet(rt) {
            inner.reposted_by = Some(author);
            return Some(inner);
        }
    }
    let id = opt_s(&r["rest_id"]).unwrap_or_else(|| s(&l["id_str"]));
    if id.is_empty() {
        return None;
    }
    let raw = opt_s(&r["note_tweet"]["note_tweet_results"]["result"]["text"])
        .unwrap_or_else(|| s(&l["full_text"]));
    Some(Post {
        url: format!(
            "https://x.com/{}/status/{id}",
            if author.handle.is_empty() {
                "i"
            } else {
                &author.handle
            }
        ),
        id,
        text: clean_text(l, raw),
        created_at: created(&l["created_at"]),
        media: media(l),
        stats: Stats {
            likes: u(&l["favorite_count"]),
            replies: u(&l["reply_count"]),
            reposts: u(&l["retweet_count"]).map(|n| n + u(&l["quote_count"]).unwrap_or(0)),
            views: u(&r["views"]["count"]),
            saves: u(&l["bookmark_count"]),
        },
        liked: l["favorited"].as_bool(),
        reposted: l["retweeted"].as_bool(),
        reply_to: opt_s(&l["in_reply_to_status_id_str"]),
        reposted_by: None,
        quoted: tweet(&r["quoted_status_result"]["result"]).map(Box::new),
        author,
    })
}

pub fn timeline(v: &Value) -> (Vec<Post>, Option<String>) {
    fn walk(
        v: &Value,
        out: &mut Vec<Post>,
        seen: &mut HashSet<String>,
        cursor: &mut Option<String>,
    ) {
        match v {
            Value::Object(o) => {
                if o.get("cursorType").and_then(Value::as_str) == Some("Bottom") {
                    if let Some(c) = o.get("value").and_then(Value::as_str) {
                        *cursor = Some(c.to_string());
                    }
                }
                if let Some(t) = o.get("tweet_results") {
                    let promoted = o.get("promotedMetadata").is_some_and(|p| !p.is_null());
                    if !promoted {
                        if let Some(p) = tweet(&t["result"]) {
                            if seen.insert(p.id.clone()) {
                                out.push(p);
                            }
                        }
                    }
                    return;
                }
                for x in o.values() {
                    walk(x, out, seen, cursor);
                }
            }
            Value::Array(a) => {
                for x in a {
                    walk(x, out, seen, cursor);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut cursor = None;
    walk(v, &mut out, &mut HashSet::new(), &mut cursor);
    (out, cursor)
}

pub fn tweet_id(input: &str) -> Option<String> {
    let t = input.trim();
    if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
        return Some(t.to_string());
    }
    let re = regex::Regex::new(r"/status(?:es)?/(\d+)").expect("static regex");
    re.captures(t).map(|c| c[1].to_string())
}

pub fn handle_of(input: &str) -> String {
    let t = input.trim().trim_start_matches('@');
    let t = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))
        .map(|rest| {
            rest.split('/')
                .nth(1)
                .unwrap_or_default()
                .split('?')
                .next()
                .unwrap_or_default()
        })
        .unwrap_or(t);
    t.to_string()
}

fn me_cache() -> &'static Mutex<Option<(String, Instant)>> {
    static C: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

pub async fn me_handle() -> Result<String, String> {
    if let Some((h, at)) = me_cache().lock().unwrap_or_else(|e| e.into_inner()).clone() {
        if at.elapsed() < Duration::from_secs(600) {
            return Ok(h);
        }
    }
    let headers = base_headers().await?;
    let v = web::fetch(
        P,
        "GET",
        &format!("{API}/1.1/account/settings.json"),
        headers,
        None,
    )
    .await?
    .json()?;
    let h = s(&v["screen_name"]);
    if h.is_empty() {
        return Err("NOT_LOGGED_IN: X did not return the signed-in account".into());
    }
    *me_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some((h.clone(), Instant::now()));
    Ok(h)
}

async fn user(handle: &str) -> Result<Profile, String> {
    let v = gql(
        "UserByScreenName",
        json!({ "screen_name": handle, "withSafetyModeUserFields": true, "withGrokTranslatedBio": false }),
        false,
    )
    .await?;
    profile(&v["data"]["user"]["result"])
        .ok_or_else(|| format!("NOT_FOUND: @{handle} does not exist on X or is suspended"))
}

fn with_cursor(mut vars: Value, a: &Value) -> Value {
    if let Some(c) = opt_str(a, "cursor") {
        vars["cursor"] = json!(c);
    }
    vars
}

async fn timeline_call(name: &str, vars: Value) -> Result<Value, String> {
    let v = gql(name, vars, false).await?;
    let (items, cursor) = timeline(&v);
    Ok(page(items, cursor))
}

pub async fn me() -> Result<Value, String> {
    let h = me_handle().await?;
    Ok(json!(user(&h).await?))
}

async fn mutate_ok(name: &str, vars: Value) -> Result<Value, String> {
    gql(name, vars, true).await?;
    Ok(json!({ "ok": true }))
}

async fn follow(handle: &str, off: bool) -> Result<Value, String> {
    let target = user(handle).await?;
    let mut headers = base_headers().await?;
    headers.insert(
        "content-type".into(),
        json!("application/x-www-form-urlencoded"),
    );
    let path = if off { "destroy" } else { "create" };
    web::fetch(
        P,
        "POST",
        &format!("{API}/1.1/friendships/{path}.json"),
        headers,
        Some(web::form(&[
            ("include_profile_interstitial_type", "1"),
            ("skip_status", "true"),
            ("user_id", &target.id),
        ])),
    )
    .await?;
    Ok(json!({ "ok": true, "user": target.handle, "following": !off }))
}

fn need_tweet(a: &Value, key: &str) -> Result<String, String> {
    let raw = req_str(a, key)?;
    tweet_id(&raw).ok_or_else(|| {
        format!("INVALID_ARGUMENTS: {key} must be a post id or an x.com/.../status/... link")
    })
}

pub async fn call(tool: &str, a: &Value) -> Result<Value, String> {
    match tool {
        "x_me" => me().await,
        "x_feed" => {
            let following = opt_str(a, "mode").as_deref() == Some("following");
            let name = if following {
                "HomeLatestTimeline"
            } else {
                "HomeTimeline"
            };
            timeline_call(
                name,
                with_cursor(
                    json!({ "count": 20, "includePromotedContent": false, "latestControlAvailable": true, "requestContext": "launch", "withCommunity": true }),
                    a,
                ),
            )
            .await
        }
        "x_profile" => Ok(json!(user(&handle_of(&req_str(a, "handle")?)).await?)),
        "x_user_posts" => {
            let p = user(&handle_of(&req_str(a, "handle")?)).await?;
            let replies = a["include_replies"].as_bool().unwrap_or(false);
            timeline_call(
                if replies { "UserTweetsAndReplies" } else { "UserTweets" },
                with_cursor(
                    json!({ "userId": p.id, "count": 20, "includePromotedContent": false, "withQuickPromoteEligibilityTweetFields": false, "withVoice": true, "withCommunity": true }),
                    a,
                ),
            )
            .await
        }
        "x_post" => {
            let id = need_tweet(a, "post")?;
            let v = gql(
                "TweetDetail",
                with_cursor(
                    json!({ "focalTweetId": id, "with_rux_injections": false, "rankingMode": "Relevance", "includePromotedContent": false, "withCommunity": true, "withQuickPromoteEligibilityTweetFields": true, "withBirdwatchNotes": true, "withVoice": true }),
                    a,
                ),
                false,
            )
            .await?;
            let (items, cursor) = timeline(&v);
            let focal = items.iter().find(|p| p.id == id).cloned();
            let replies: Vec<Post> = items.into_iter().filter(|p| p.id != id).collect();
            Ok(json!({ "post": focal, "replies": page(replies, cursor) }))
        }
        "x_search" => {
            let product = match opt_str(a, "mode").as_deref() {
                Some("top") => "Top",
                Some("media") => "Media",
                _ => "Latest",
            };
            timeline_call(
                "SearchTimeline",
                with_cursor(
                    json!({ "rawQuery": req_str(a, "query")?, "count": 20, "querySource": "typed_query", "product": product }),
                    a,
                ),
            )
            .await
        }
        "x_mentions" => {
            let h = me_handle().await?;
            timeline_call(
                "SearchTimeline",
                with_cursor(
                    json!({ "rawQuery": format!("@{h} -from:{h}"), "count": 20, "querySource": "typed_query", "product": "Latest" }),
                    a,
                ),
            )
            .await
        }
        "x_bookmarks" => {
            timeline_call(
                "Bookmarks",
                with_cursor(json!({ "count": 20, "includePromotedContent": false }), a),
            )
            .await
        }
        "x_create_post" => {
            let text = req_str(a, "text")?;
            if text.chars().count() > 4000 {
                return Err("INVALID_ARGUMENTS: text is longer than 4000 characters".into());
            }
            let mut vars = json!({ "tweet_text": text, "dark_request": false, "media": { "media_entities": [], "possibly_sensitive": false }, "semantic_annotation_ids": [] });
            if let Some(r) = opt_str(a, "reply_to") {
                let id =
                    tweet_id(&r).ok_or("INVALID_ARGUMENTS: reply_to must be a post id or link")?;
                vars["reply"] = json!({ "in_reply_to_tweet_id": id, "exclude_reply_user_ids": [] });
            }
            if let Some(q) = opt_str(a, "quote") {
                let id =
                    tweet_id(&q).ok_or("INVALID_ARGUMENTS: quote must be a post id or link")?;
                vars["attachment_url"] = json!(format!("https://x.com/i/status/{id}"));
            }
            let v = gql("CreateTweet", vars, true).await?;
            let post = tweet(&v["data"]["create_tweet"]["tweet_results"]["result"]);
            if post.is_none() {
                check_errors(&json!({ "errors": v["errors"] }))?;
            }
            Ok(json!({ "ok": true, "post": post }))
        }
        "x_like" => {
            let id = need_tweet(a, "post")?;
            mutate_ok(
                if undo(a) {
                    "UnfavoriteTweet"
                } else {
                    "FavoriteTweet"
                },
                json!({ "tweet_id": id }),
            )
            .await
        }
        "x_repost" => {
            let id = need_tweet(a, "post")?;
            if undo(a) {
                mutate_ok(
                    "DeleteRetweet",
                    json!({ "source_tweet_id": id, "dark_request": false }),
                )
                .await
            } else {
                mutate_ok(
                    "CreateRetweet",
                    json!({ "tweet_id": id, "dark_request": false }),
                )
                .await
            }
        }
        "x_bookmark" => {
            let id = need_tweet(a, "post")?;
            mutate_ok(
                if undo(a) {
                    "DeleteBookmark"
                } else {
                    "CreateBookmark"
                },
                json!({ "tweet_id": id }),
            )
            .await
        }
        "x_delete_post" => {
            let id = need_tweet(a, "post")?;
            mutate_ok(
                "DeleteTweet",
                json!({ "tweet_id": id, "dark_request": false }),
            )
            .await
        }
        "x_follow" => follow(&handle_of(&req_str(a, "handle")?), undo(a)).await,
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_json(handle: &str) -> Value {
        json!({ "__typename": "User", "rest_id": "42", "is_blue_verified": true,
            "core": { "screen_name": handle, "name": "Ana" },
            "avatar": { "image_url": "https://pbs.twimg.com/a_normal.jpg" },
            "legacy": { "description": "bio", "followers_count": 10, "friends_count": 3, "statuses_count": 7 } })
    }

    fn tweet_json(id: &str) -> Value {
        json!({ "__typename": "Tweet", "rest_id": id,
            "core": { "user_results": { "result": user_json("ana") } },
            "views": { "count": "99" },
            "legacy": { "full_text": "hi &amp; https://t.co/x https://t.co/m", "created_at": "Wed Oct 10 20:19:24 +0000 2018",
                "favorite_count": 5, "reply_count": 1, "retweet_count": 2, "quote_count": 1, "favorited": true,
                "entities": { "urls": [{ "url": "https://t.co/x", "expanded_url": "https://example.com" }], "media": [{ "url": "https://t.co/m" }] },
                "extended_entities": { "media": [
                    { "type": "photo", "media_url_https": "https://pbs.twimg.com/p.jpg" },
                    { "type": "video", "media_url_https": "https://pbs.twimg.com/t.jpg", "video_info": { "variants": [
                        { "content_type": "video/mp4", "bitrate": 100, "url": "low.mp4" },
                        { "content_type": "video/mp4", "bitrate": 900, "url": "high.mp4" },
                        { "content_type": "application/x-mpegURL", "url": "x.m3u8" } ] } } ] } } })
    }

    #[test]
    fn parses_a_tweet() {
        let p = tweet(&tweet_json("1")).unwrap();
        assert_eq!(p.text, "hi & https://example.com");
        assert_eq!(p.url, "https://x.com/ana/status/1");
        assert_eq!(
            p.author.avatar.as_deref(),
            Some("https://pbs.twimg.com/a_bigger.jpg")
        );
        assert_eq!(p.media.len(), 2);
        assert_eq!(p.media[1].url, "high.mp4");
        assert_eq!(p.stats.reposts, Some(3));
        assert_eq!(p.stats.views, Some(99));
        assert_eq!(p.created_at, Some(1539202764));
        assert_eq!(p.liked, Some(true));
    }

    #[test]
    fn walks_a_timeline_skipping_ads_and_keeping_the_cursor() {
        let mut rt = tweet_json("3");
        rt["legacy"]["retweeted_status_result"] = json!({ "result": tweet_json("4") });
        let v = json!({ "data": { "home": { "home_timeline_urt": { "instructions": [{ "type": "TimelineAddEntries", "entries": [
            { "content": { "itemContent": { "tweet_results": { "result": tweet_json("1") } } } },
            { "content": { "itemContent": { "tweet_results": { "result": tweet_json("2") }, "promotedMetadata": { "x": 1 } } } },
            { "content": { "itemContent": { "tweet_results": { "result": { "__typename": "TweetWithVisibilityResults", "tweet": rt } } } } },
            { "content": { "itemContent": { "tweet_results": { "result": tweet_json("1") } } } },
            { "content": { "cursorType": "Top", "value": "top" } },
            { "content": { "cursorType": "Bottom", "value": "next" } } ] }] } } } });
        let (items, cursor) = timeline(&v);
        assert_eq!(
            items.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "4"]
        );
        assert_eq!(
            items[1].reposted_by.as_ref().map(|a| a.id.as_str()),
            Some("42")
        );
        assert_eq!(cursor.as_deref(), Some("next"));
    }

    #[test]
    fn reads_ids_and_handles() {
        assert_eq!(
            tweet_id("https://x.com/a/status/123?s=20").as_deref(),
            Some("123")
        );
        assert_eq!(tweet_id("456").as_deref(), Some("456"));
        assert_eq!(tweet_id("nope"), None);
        assert_eq!(handle_of("@Ana"), "Ana");
        assert_eq!(handle_of("https://x.com/ana?lang=en"), "ana");
    }

    #[test]
    fn profile_reads_counts() {
        let p = profile(&user_json("ana")).unwrap();
        assert_eq!(p.followers, Some(10));
        assert_eq!(p.url, "https://x.com/ana");
        assert!(p.verified);
    }
}
