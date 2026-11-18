use base64::Engine;
use serde_json::{json, Value};

use super::args::{opt_str, req_str, undo};
use super::model::{
    b, opt_s, page, s, u, with_extra, Author, Comment, Media, MediaKind, Platform, Post, Profile,
    Stats,
};
use super::page as web;

const P: Platform = Platform::Forums;
const ORIGIN: &str = "https://www.reddit.com";
const LIMIT: &str = "25";
const TIMES: &[&str] = &["hour", "day", "week", "month", "year", "all"];

type Params = Vec<(&'static str, String)>;

pub fn token_is_user(jwt: &str) -> bool {
    let Some(payload) = jwt.split('.').nth(1) else {
        return false;
    };
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .is_some_and(|v| v["sub"] == "user")
}

fn friendly(e: String) -> String {
    if e.starts_with("UPSTREAM_404") {
        return "NOT_FOUND: Forums has nothing at that address".into();
    }
    if e.starts_with("UPSTREAM_403") {
        return "REJECTED: Forums refused the request (a private or banned community, or the session lacks permission)".into();
    }
    e
}

pub fn oauth_url(path: &str, query: &str) -> String {
    format!(
        "https://oauth.reddit.com{}?{query}",
        path.strip_suffix(".json").unwrap_or(path)
    )
}

async fn bearer() -> Option<String> {
    let b = web::running().await?;
    web::cookie(&b, P, "token_v2").await
}

async fn get(path: &str, mut params: Vec<(&str, String)>) -> Result<Value, String> {
    web::ready(P).await?;
    params.push(("raw_json", "1".into()));
    let query = web::query(&params);
    let first = web::fetch(
        P,
        "GET",
        &format!("{ORIGIN}{path}?{query}"),
        web::headers(&[("accept", "application/json")]),
        None,
    )
    .await;
    let reply = match first {
        Err(e) if e.starts_with("UPSTREAM_403") => match bearer().await {
            Some(token) => web::fetch(
                P,
                "GET",
                &oauth_url(path, &query),
                web::headers(&[
                    ("accept", "application/json"),
                    ("authorization", &format!("Bearer {token}")),
                ]),
                None,
            )
            .await
            .map_err(friendly)?,
            None => return Err(friendly(e)),
        },
        other => other.map_err(friendly)?,
    };
    reply.json()
}

pub fn account(v: &Value) -> Option<(String, String)> {
    let d = if v["data"].is_object() { &v["data"] } else { v };
    Some((opt_s(&d["name"])?, s(&d["modhash"])))
}

async fn session() -> Result<(String, String), String> {
    account(&get("/api/me.json", Vec::new()).await?).ok_or_else(|| {
        "NOT_LOGGED_IN: Forums did not return the signed-in account; sign in again in the Forums screen".into()
    })
}

pub fn api_result(v: Value) -> Result<Value, String> {
    if let Some(first) = v["json"]["errors"].as_array().and_then(|a| a.first()) {
        let code = s(&first[0]);
        let msg = s(&first[1]);
        return Err(match code.as_str() {
            "RATELIMIT" => format!("RATE_LIMITED: Forums asked to slow down: {msg}"),
            "USER_REQUIRED" => "NOT_LOGGED_IN: Forums asked to sign in again".to_string(),
            _ => format!("REJECTED: Forums refused the request: {code} {msg}"),
        });
    }
    if v["success"].as_bool() == Some(false) {
        return Err("REJECTED: Forums refused the request".into());
    }
    Ok(if v["json"]["data"].is_object() {
        v["json"]["data"].clone()
    } else {
        v
    })
}

async fn post(path: &str, pairs: &[(&str, &str)]) -> Result<Value, String> {
    let (_, modhash) = session().await?;
    let mut form: Vec<(&str, &str)> = pairs.to_vec();
    form.extend([
        ("api_type", "json"),
        ("raw_json", "1"),
        ("uh", modhash.as_str()),
    ]);
    let mut headers = web::headers(&[
        ("accept", "application/json"),
        ("content-type", "application/x-www-form-urlencoded"),
    ]);
    if !modhash.is_empty() {
        headers.insert("x-modhash".into(), json!(modhash));
    }
    let reply = web::fetch(
        P,
        "POST",
        &format!("{ORIGIN}{path}"),
        headers,
        Some(web::form(&form)),
    )
    .await
    .map_err(friendly)?;
    if reply.body.trim().is_empty() {
        return Ok(json!({}));
    }
    api_result(reply.json()?)
}

fn rest(input: &str) -> &str {
    let t = input.trim();
    let t = match t.split_once("reddit.com/") {
        Some((_, r)) => r,
        None => t,
    };
    t.split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/')
}

fn word(x: &str, max: usize, dash: bool) -> bool {
    !x.is_empty()
        && x.len() <= max
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || (dash && c == '-'))
}

pub fn sub_of(input: &str) -> Option<String> {
    let r = rest(input);
    let name = r.strip_prefix("r/").unwrap_or(r).split('/').next()?;
    word(name, 21, false).then(|| name.to_string())
}

pub fn user_of(input: &str) -> Option<String> {
    let r = rest(input);
    let name = ["user/", "u/", "@"]
        .iter()
        .find_map(|p| r.strip_prefix(p))
        .unwrap_or(r)
        .split('/')
        .next()?;
    word(name, 20, true).then(|| name.to_string())
}

fn is_sub(input: &str) -> bool {
    rest(input).starts_with("r/")
}

pub fn post_id(input: &str) -> Option<String> {
    let t = input.trim();
    let re = regex::Regex::new(r"(?:/comments/|redd\.it/)([a-z0-9]+)").expect("static regex");
    let id = match re.captures(t) {
        Some(c) => c[1].to_string(),
        None => t.strip_prefix("t3_").unwrap_or(t).to_string(),
    };
    (!id.is_empty()
        && id.len() <= 12
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
    .then_some(id)
}

pub fn fullname(input: &str) -> Option<String> {
    let t = input.trim();
    if let Some(id) = t.strip_prefix("t1_") {
        return word(id, 12, false).then(|| t.to_string());
    }
    post_id(t).map(|id| format!("t3_{id}"))
}

fn after(a: &Value) -> Option<String> {
    opt_str(a, "cursor").filter(|c| {
        c.len() <= 20
            && c.split_once('_')
                .is_some_and(|(k, id)| k.len() == 2 && k.starts_with('t') && word(id, 14, false))
    })
}

pub fn feed_path(kind: &str, sort: Option<&str>) -> Result<String, String> {
    let base = match kind {
        "home" => "",
        "popular" => "/r/popular",
        "all" => "/r/all",
        _ => return Err("INVALID_ARGUMENTS: feed must be home, popular or all".into()),
    };
    let sort = sort.unwrap_or(if kind == "home" { "best" } else { "hot" });
    let allowed: &[&str] = if kind == "home" {
        &["best", "hot", "new", "top", "rising"]
    } else {
        &["hot", "new", "top", "rising"]
    };
    if !allowed.contains(&sort) {
        return Err(format!(
            "INVALID_ARGUMENTS: sort for the {kind} feed must be one of {}",
            allowed.join(", ")
        ));
    }
    Ok(format!("{base}/{sort}.json"))
}

pub fn listing_params(
    sort: &str,
    time: Option<&str>,
    cursor: Option<String>,
) -> Result<Params, String> {
    let mut out = vec![("limit", LIMIT.to_string())];
    if let Some(t) = time {
        if !TIMES.contains(&t) {
            return Err(format!(
                "INVALID_ARGUMENTS: time must be one of {}",
                TIMES.join(", ")
            ));
        }
        if ["top", "controversial", "relevance", "comments"].contains(&sort) {
            out.push(("t", t.to_string()));
        }
    }
    if let Some(c) = cursor {
        out.push(("after", c));
    }
    Ok(out)
}

pub fn search_request(
    query: &str,
    kind: &str,
    sort: Option<&str>,
    time: Option<&str>,
    subreddit: Option<&str>,
    cursor: Option<String>,
) -> Result<(String, Params), String> {
    let kind_param = match kind {
        "posts" => "link",
        "subreddits" => "sr",
        "users" => "user",
        _ => return Err("INVALID_ARGUMENTS: kind must be posts, subreddits or users".into()),
    };
    let sort = sort.unwrap_or("relevance");
    if !["relevance", "hot", "top", "new", "comments"].contains(&sort) {
        return Err("INVALID_ARGUMENTS: sort must be relevance, hot, top, new or comments".into());
    }
    let mut params = listing_params(sort, time, cursor)?;
    params.insert(0, ("q", query.to_string()));
    params.push(("type", kind_param.to_string()));
    params.push(("sort", sort.to_string()));
    let path = match subreddit {
        Some(x) => {
            let name = sub_of(x)
                .ok_or("INVALID_ARGUMENTS: subreddit must be a community name such as r/rust")?;
            params.push(("restrict_sr", "1".to_string()));
            format!("/r/{name}/search.json")
        }
        None => "/search.json".to_string(),
    };
    Ok((path, params))
}

pub fn search_cursor(a: &Value) -> Option<(bool, String)> {
    if let Some(c) = after(a) {
        return Some((false, c));
    }
    opt_str(a, "cursor")
        .filter(|c| {
            c.len() <= 2000
                && c.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=_-".contains(&b))
        })
        .map(|c| (true, c))
}

pub fn search_page_url(
    query: &str,
    kind: &str,
    sort: Option<&str>,
    time: Option<&str>,
    subreddit: Option<&str>,
    cursor: Option<&str>,
) -> Result<String, String> {
    let kind_param = match kind {
        "posts" => "posts",
        "subreddits" => "communities",
        "users" => "people",
        _ => return Err("INVALID_ARGUMENTS: kind must be posts, subreddits or users".into()),
    };
    let sort = sort.unwrap_or("relevance");
    if !["relevance", "hot", "top", "new", "comments"].contains(&sort) {
        return Err("INVALID_ARGUMENTS: sort must be relevance, hot, top, new or comments".into());
    }
    let time = listing_params(sort, time, None)?
        .into_iter()
        .find(|(k, _)| *k == "t");
    let mut params = vec![("q", query.to_string())];
    let path = match subreddit.filter(|_| kind == "posts") {
        Some(x) => {
            let name = sub_of(x)
                .ok_or("INVALID_ARGUMENTS: subreddit must be a community name such as r/rust")?;
            format!("/r/{name}/search/")
        }
        None => {
            params.push(("type", kind_param.to_string()));
            "/search/".to_string()
        }
    };
    params.push(("sort", sort.to_string()));
    params.extend(time);
    let prefix = match cursor {
        Some(c) => {
            params.push(("cursor", c.to_string()));
            "/svc/shreddit"
        }
        None => "",
    };
    Ok(format!("{ORIGIN}{prefix}{path}?{}", web::query(&params)))
}

fn unescape(x: &str) -> String {
    let re =
        regex::Regex::new(r"&(#x[0-9a-fA-F]+|#[0-9]+|quot|amp|lt|gt|apos);").expect("static regex");
    re.replace_all(x, |c: &regex::Captures| {
        let e = &c[1];
        let code = match e {
            "quot" => Some('"'),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "apos" => Some('\''),
            _ => e
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| e.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        code.map(String::from).unwrap_or_else(|| c[0].to_string())
    })
    .into_owned()
}

fn grab(chunk: &str, pattern: &str) -> Option<String> {
    regex::Regex::new(pattern)
        .ok()?
        .captures(chunk)
        .map(|c| unescape(c[1].trim()))
        .filter(|x| !x.is_empty())
}

fn tracked(chunk: &str) -> Value {
    grab(chunk, r#"data-faceplate-tracking-context="([^"]*)""#)
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null)
}

fn counted(chunk: &str, label: &str) -> Option<u64> {
    grab(
        chunk,
        &format!(r#"number="(\d+)"[^>]*>\s*</faceplate-number>\s*{label}"#),
    )
    .and_then(|n| n.parse().ok())
}

fn units<'a>(html: &'a str, testid: &str) -> Vec<&'a str> {
    html.split(&format!(r#"data-testid="{testid}""#))
        .skip(1)
        .collect()
}

fn search_post(chunk: &str) -> Option<Value> {
    let t = tracked(chunk);
    let permalink = grab(chunk, r#"data-testid="post-title"\s+href="([^"]+)""#)?;
    let id = post_id(&permalink)?;
    let created = grab(chunk, r#"<faceplate-timeago[^>]*\sts="([^"]+)""#)
        .and_then(|ts| chrono::DateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%S%.f%z").ok())
        .map(|d| d.timestamp());
    let thumb = grab(
        chunk,
        r#"data-testid="search_post_thumbnail"[^>]*\ssrc="([^"]+)""#,
    );
    link(&json!({
        "id": id,
        "permalink": permalink,
        "title": opt_s(&t["post"]["title"])
            .or_else(|| grab(chunk, r#"data-testid="post-title"[^>]*aria-label="([^"]*)""#)),
        "author": t["profile"]["name"],
        "author_fullname": t["profile"]["id"],
        "subreddit": t["subreddit"]["name"],
        "is_self": true,
        "score": counted(chunk, "votes?"),
        "num_comments": counted(chunk, "comments?"),
        "created_utc": created,
        "over_18": t["post"]["nsfw"],
        "spoiler": t["post"]["spoiler"],
        "preview": thumb.map(|url| json!({ "images": [{ "source": { "url": url } }] })),
    }))
}

fn search_community(chunk: &str) -> Option<Value> {
    let t = tracked(chunk);
    let name = opt_s(&t["subreddit"]["name"])?;
    let head = chunk
        .split("search-community-title")
        .next()
        .unwrap_or(chunk);
    Some(with_extra(
        &Profile {
            id: s(&t["subreddit"]["id"]),
            url: format!("{ORIGIN}/r/{name}/"),
            handle: format!("r/{name}"),
            avatar: grab(head, r#"<img[^>]*\ssrc="([^"]+)""#),
            bio: grab(
                chunk,
                r#"data-testid="search-subreddit-desc-text"[^>]*>([^<]*)<"#,
            )
            .unwrap_or_default(),
            ..Default::default()
        },
        json!({
            "kind": "subreddit",
            "weekly_visitors": counted(chunk, "weekly visitors"),
            "nsfw": t["subreddit"]["nsfw"].as_bool().filter(|x| *x),
        }),
    ))
}

fn search_person(chunk: &str) -> Option<Value> {
    let t = tracked(chunk);
    let name = opt_s(&t["profile"]["name"])?;
    let head = chunk
        .split("search-community-title")
        .next()
        .unwrap_or(chunk);
    Some(with_extra(
        &Profile {
            id: s(&t["profile"]["id"]),
            url: format!("{ORIGIN}/user/{name}/"),
            handle: name,
            avatar: grab(head, r#"<img[^>]*\ssrc="([^"]+)""#),
            bio: grab(
                chunk,
                r#"data-testid="search-subreddit-desc-text"[^>]*>([^<]*)<"#,
            )
            .unwrap_or_default(),
            likes: counted(chunk, "karma"),
            ..Default::default()
        },
        json!({ "kind": "user" }),
    ))
}

pub fn search_html(html: &str, kind: &str) -> Result<Value, String> {
    if !html.contains("shreddit") && !html.contains("faceplate") {
        return Err("UPSTREAM_CHANGED: Forums answered the search page without results markup (open reddit.com in the OmniGet browser; it may be showing a check)".into());
    }
    let items: Vec<Value> = match kind {
        "subreddits" => units(html, "search-community")
            .into_iter()
            .filter_map(search_community)
            .collect(),
        "users" => units(html, "search-author")
            .into_iter()
            .filter_map(search_person)
            .collect(),
        _ => units(html, "search-post-unit")
            .into_iter()
            .filter_map(search_post)
            .collect(),
    };
    let next = grab(
        html,
        r#"<faceplate-partial[^>]*\ssrc="/svc/shreddit/[^"]*search/\?[^"]*cursor=([^"&]+)"#,
    )
    .and_then(|c| urlencoding::decode(&c).ok().map(|d| d.into_owned()))
    .filter(|_| !items.is_empty());
    let mut out = page(items, next);
    out["source"] = json!("web_page");
    Ok(out)
}

async fn search_page(a: &Value, cursor: Option<&str>) -> Result<Value, String> {
    let kind = opt_str(a, "kind").unwrap_or_else(|| "posts".into());
    let url = search_page_url(
        &req_str(a, "query")?,
        &kind,
        opt_str(a, "sort").as_deref(),
        opt_str(a, "time").as_deref(),
        opt_str(a, "subreddit").as_deref(),
        cursor,
    )?;
    web::ready(P).await?;
    let reply = web::fetch(
        P,
        "GET",
        &url,
        web::headers(&[("accept", "text/html")]),
        None,
    )
    .await
    .map_err(friendly)?;
    search_html(&reply.body, &kind)
}

async fn search(a: &Value) -> Result<Value, String> {
    static JSON_REFUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    use std::sync::atomic::Ordering::Relaxed;
    let cursor = search_cursor(a);
    if let Some((true, c)) = &cursor {
        return search_page(a, Some(c)).await;
    }
    let (path, params) = search_request(
        &req_str(a, "query")?,
        opt_str(a, "kind").as_deref().unwrap_or("posts"),
        opt_str(a, "sort").as_deref(),
        opt_str(a, "time").as_deref(),
        opt_str(a, "subreddit").as_deref(),
        cursor.map(|(_, c)| c),
    )?;
    if !JSON_REFUSED.load(Relaxed) {
        match get(&path, params).await {
            Err(e) if e.starts_with("REJECTED") => JSON_REFUSED.store(true, Relaxed),
            other => return listing(&other?),
        }
    }
    search_page(a, None).await
}

fn clean(x: &Value) -> Option<String> {
    opt_s(x).map(|v| v.replace("&amp;", "&"))
}

fn secs(v: &Value) -> Option<i64> {
    v.as_f64().map(|f| f as i64)
}

fn media(d: &Value) -> Vec<Media> {
    let preview = &d["preview"]["images"][0]["source"];
    let video = [&d["secure_media"], &d["media"]]
        .iter()
        .find_map(|m| clean(&m["reddit_video"]["fallback_url"]));
    if let Some(url) = video {
        return vec![Media {
            kind: MediaKind::Video,
            url,
            thumb: clean(&preview["url"]),
            width: u(&preview["width"]),
            height: u(&preview["height"]),
        }];
    }
    if let Some(items) = d["gallery_data"]["items"].as_array() {
        let found: Vec<Media> = items
            .iter()
            .filter_map(|i| {
                let m = &d["media_metadata"][i["media_id"].as_str()?]["s"];
                Some(Media {
                    kind: MediaKind::Image,
                    url: clean(&m["u"]).or_else(|| clean(&m["gif"]))?,
                    thumb: None,
                    width: u(&m["x"]),
                    height: u(&m["y"]),
                })
            })
            .collect();
        if !found.is_empty() {
            return found;
        }
    }
    if let Some(url) = clean(&preview["url"]) {
        return vec![Media {
            kind: MediaKind::Image,
            url,
            thumb: None,
            width: u(&preview["width"]),
            height: u(&preview["height"]),
        }];
    }
    if d["post_hint"] == "image" {
        if let Some(url) = clean(&d["url"]) {
            return vec![Media {
                kind: MediaKind::Image,
                url,
                thumb: None,
                width: None,
                height: None,
            }];
        }
    }
    Vec::new()
}

fn author(d: &Value) -> Author {
    let name = s(&d["author"]);
    Author {
        id: s(&d["author_fullname"]),
        handle: name,
        name: String::new(),
        avatar: None,
        verified: false,
    }
}

fn vote(d: &Value) -> i64 {
    match d["likes"].as_bool() {
        Some(true) => 1,
        Some(false) => -1,
        None => 0,
    }
}

pub fn link(d: &Value) -> Option<Value> {
    let id = opt_s(&d["id"])?;
    let external = clean(&d["url"]).filter(|_| !b(&d["is_self"]));
    Some(with_extra(
        &Post {
            url: format!("{ORIGIN}{}", s(&d["permalink"])),
            id,
            author: author(d),
            text: s(&d["selftext"]),
            created_at: secs(&d["created_utc"]),
            media: media(d),
            stats: Stats {
                likes: u(&d["score"]),
                replies: u(&d["num_comments"]),
                ..Default::default()
            },
            liked: d["likes"].as_bool(),
            ..Default::default()
        },
        json!({
            "kind": "link",
            "title": opt_s(&d["title"]),
            "subreddit": opt_s(&d["subreddit"]),
            "link": external,
            "domain": opt_s(&d["domain"]),
            "upvote_ratio": d["upvote_ratio"].as_f64(),
            "flair": opt_s(&d["link_flair_text"]),
            "vote": vote(d),
            "saved": d["saved"].as_bool(),
            "nsfw": d["over_18"].as_bool().filter(|x| *x),
            "spoiler": d["spoiler"].as_bool().filter(|x| *x),
            "pinned": d["stickied"].as_bool().filter(|x| *x),
        }),
    ))
}

fn comment_post(d: &Value) -> Option<Value> {
    let id = opt_s(&d["id"])?;
    Some(with_extra(
        &Post {
            url: format!("{ORIGIN}{}", s(&d["permalink"])),
            id: format!("t1_{id}"),
            author: author(d),
            text: s(&d["body"]),
            created_at: secs(&d["created_utc"]),
            stats: Stats {
                likes: u(&d["score"]),
                ..Default::default()
            },
            liked: d["likes"].as_bool(),
            reply_to: opt_s(&d["link_id"]),
            ..Default::default()
        },
        json!({
            "kind": "comment",
            "title": opt_s(&d["link_title"]),
            "subreddit": opt_s(&d["subreddit"]),
            "vote": vote(d),
            "saved": d["saved"].as_bool(),
        }),
    ))
}

pub fn user_profile(d: &Value) -> Option<Value> {
    let name = opt_s(&d["name"])?;
    let sub = &d["subreddit"];
    let karma = u(&d["total_karma"])
        .or_else(|| Some(u(&d["link_karma"])? + u(&d["comment_karma"]).unwrap_or(0)));
    Some(with_extra(
        &Profile {
            id: opt_s(&d["id"])
                .map(|i| format!("t2_{i}"))
                .unwrap_or_default(),
            url: format!("{ORIGIN}/user/{name}/"),
            handle: name,
            name: s(&sub["title"]),
            avatar: clean(&d["snoovatar_img"]).or_else(|| clean(&d["icon_img"])),
            bio: s(&sub["public_description"]),
            verified: b(&d["verified"]),
            followers: u(&sub["subscribers"]),
            likes: karma,
            ..Default::default()
        },
        json!({
            "kind": "user",
            "link_karma": u(&d["link_karma"]),
            "comment_karma": u(&d["comment_karma"]),
            "created_at": secs(&d["created_utc"]),
            "suspended": d["is_suspended"].as_bool().filter(|x| *x),
            "nsfw": sub["over_18"].as_bool().filter(|x| *x),
        }),
    ))
}

pub fn sub_profile(d: &Value) -> Option<Value> {
    let name = opt_s(&d["display_name"])?;
    Some(with_extra(
        &Profile {
            id: s(&d["name"]),
            url: format!("{ORIGIN}/r/{name}/"),
            handle: format!("r/{name}"),
            name: s(&d["title"]),
            avatar: clean(&d["community_icon"]).or_else(|| clean(&d["icon_img"])),
            bio: s(&d["public_description"]),
            private: opt_s(&d["subreddit_type"]).is_some_and(|t| t != "public"),
            followers: u(&d["subscribers"]),
            followed_by_me: d["user_is_subscriber"].as_bool(),
            ..Default::default()
        },
        json!({
            "kind": "subreddit",
            "active": u(&d["accounts_active"]).or_else(|| u(&d["active_user_count"])),
            "created_at": secs(&d["created_utc"]),
            "nsfw": d["over18"].as_bool().filter(|x| *x),
        }),
    ))
}

pub fn thing(t: &Value) -> Option<Value> {
    let d = &t["data"];
    match t["kind"].as_str()? {
        "t3" => link(d),
        "t1" => comment_post(d),
        "t5" => sub_profile(d),
        "t2" => user_profile(d),
        _ => None,
    }
}

pub fn listing(v: &Value) -> Result<Value, String> {
    let children = v["data"]["children"]
        .as_array()
        .ok_or("UPSTREAM_CHANGED: Forums did not return a listing")?;
    Ok(page(
        children.iter().filter_map(thing).collect::<Vec<_>>(),
        opt_s(&v["data"]["after"]),
    ))
}

fn walk(
    children: &[Value],
    depth: usize,
    max_depth: usize,
    max_count: usize,
    out: &mut Vec<Value>,
    truncated: &mut bool,
) {
    for c in children {
        let d = &c["data"];
        match c["kind"].as_str() {
            Some("more") => {
                if u(&d["count"]).unwrap_or(0) > 0 {
                    *truncated = true;
                }
            }
            Some("t1") => {
                if out.len() >= max_count {
                    *truncated = true;
                    return;
                }
                let replies = d["replies"]["data"]["children"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let direct = replies.iter().filter(|r| r["kind"] == "t1").count() as u64;
                out.push(with_extra(
                    &Comment {
                        id: s(&d["id"]),
                        author: author(d),
                        text: s(&d["body"]),
                        created_at: secs(&d["created_utc"]),
                        likes: u(&d["score"]),
                        replies: Some(direct),
                    },
                    json!({
                        "depth": depth,
                        "parent": opt_s(&d["parent_id"]),
                        "op": d["is_submitter"].as_bool().filter(|x| *x),
                        "vote": vote(d),
                    }),
                ));
                if depth + 1 < max_depth {
                    walk(replies, depth + 1, max_depth, max_count, out, truncated);
                } else if !replies.is_empty() {
                    *truncated = true;
                }
            }
            _ => {}
        }
    }
}

pub fn comment_tree(listing: &Value, max_depth: usize, max_count: usize) -> (Vec<Value>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    if let Some(children) = listing["data"]["children"].as_array() {
        walk(
            children,
            0,
            max_depth.max(1),
            max_count,
            &mut out,
            &mut truncated,
        );
    }
    (out, truncated)
}

pub fn post_detail(v: &Value, max_depth: usize, max_count: usize) -> Result<Value, String> {
    let post = v[0]["data"]["children"][0]["data"]
        .as_object()
        .and_then(|_| link(&v[0]["data"]["children"][0]["data"]))
        .ok_or("UPSTREAM_CHANGED: Forums did not return the post")?;
    let (comments, truncated) = comment_tree(&v[1], max_depth, max_count);
    let total = post["stats"]["replies"].as_u64();
    Ok(json!({
        "post": post,
        "comments": {
            "count": comments.len(),
            "returned": comments.len(),
            "total": total,
            "truncated": truncated,
            "items": comments,
            "next_cursor": Value::Null,
        },
    }))
}

fn bounded(a: &Value, key: &str, default: u64, max: u64) -> usize {
    u(&a[key]).unwrap_or(default).clamp(1, max) as usize
}

async fn sub_about(name: &str) -> Result<Value, String> {
    let v = get(&format!("/r/{name}/about.json"), Vec::new()).await?;
    sub_profile(&v["data"]).ok_or_else(|| format!("NOT_FOUND: r/{name} does not exist on Forums"))
}

async fn sub_posts(name: &str, a: &Value) -> Result<Value, String> {
    let sort = opt_str(a, "sort").unwrap_or_else(|| "hot".into());
    if !["hot", "new", "top", "rising", "controversial"].contains(&sort.as_str()) {
        return Err(
            "INVALID_ARGUMENTS: sort must be hot, new, top, rising or controversial".into(),
        );
    }
    let params = listing_params(&sort, opt_str(a, "time").as_deref(), after(a))?;
    listing(&get(&format!("/r/{name}/{sort}.json"), params).await?)
}

async fn user_about(name: &str) -> Result<Value, String> {
    let v = get(&format!("/user/{name}/about.json"), Vec::new()).await?;
    user_profile(&v["data"]).ok_or_else(|| format!("NOT_FOUND: u/{name} does not exist on Forums"))
}

fn need_user(a: &Value) -> Result<String, String> {
    user_of(&req_str(a, "username")?).ok_or_else(|| {
        "INVALID_ARGUMENTS: username must be a Forums user name, or r/name for a community".into()
    })
}

fn need_sub(a: &Value, key: &str) -> Result<String, String> {
    sub_of(&req_str(a, key)?)
        .ok_or_else(|| format!("INVALID_ARGUMENTS: {key} must be a community name such as r/rust"))
}

fn need_post(a: &Value) -> Result<String, String> {
    post_id(&req_str(a, "post")?)
        .ok_or_else(|| "INVALID_ARGUMENTS: post must be a Forums post link or id".into())
}

fn need_fullname(a: &Value) -> Result<String, String> {
    fullname(&req_str(a, "post")?).ok_or_else(|| {
        "INVALID_ARGUMENTS: post must be a Forums post link or id, or a comment id starting with t1_".into()
    })
}

pub async fn call(tool: &str, a: &Value) -> Result<Value, String> {
    match tool {
        "rd_me" => user_about(&session().await?.0).await,
        "rd_feed" => {
            let kind = opt_str(a, "feed").unwrap_or_else(|| "home".into());
            let path = feed_path(&kind, opt_str(a, "sort").as_deref())?;
            let sort = path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .trim_end_matches(".json")
                .to_string();
            let params = listing_params(&sort, opt_str(a, "time").as_deref(), after(a))?;
            listing(&get(&path, params).await?)
        }
        "rd_subreddit" => {
            let name = need_sub(a, "subreddit")?;
            let about = sub_about(&name).await?;
            Ok(json!({ "subreddit": about, "posts": sub_posts(&name, a).await? }))
        }
        "rd_post" => {
            let id = need_post(a)?;
            let depth = bounded(a, "depth", 3, 8);
            let limit = bounded(a, "limit", 50, 200);
            let mut params = vec![("limit", limit.to_string()), ("depth", depth.to_string())];
            if let Some(sort) = opt_str(a, "sort") {
                if !["confidence", "top", "new", "controversial", "old", "qa"]
                    .contains(&sort.as_str())
                {
                    return Err("INVALID_ARGUMENTS: sort must be confidence, top, new, controversial, old or qa".into());
                }
                params.push(("sort", sort));
            }
            post_detail(
                &get(&format!("/comments/{id}.json"), params).await?,
                depth,
                limit,
            )
        }
        "rd_search" => search(a).await,
        "rd_user" => {
            let raw = req_str(a, "username")?;
            if is_sub(&raw) {
                return sub_about(&need_sub(a, "username")?).await;
            }
            user_about(&need_user(a)?).await
        }
        "rd_user_posts" => {
            let raw = req_str(a, "username")?;
            if is_sub(&raw) {
                return sub_posts(&need_sub(a, "username")?, a).await;
            }
            let name = need_user(a)?;
            let kind = opt_str(a, "kind").unwrap_or_else(|| "posts".into());
            let segment = match kind.as_str() {
                "posts" => "submitted",
                "comments" => "comments",
                "all" => "overview",
                _ => return Err("INVALID_ARGUMENTS: kind must be posts, comments or all".into()),
            };
            let sort = opt_str(a, "sort").unwrap_or_else(|| "new".into());
            if !["new", "hot", "top", "controversial"].contains(&sort.as_str()) {
                return Err(
                    "INVALID_ARGUMENTS: sort must be new, hot, top or controversial".into(),
                );
            }
            let mut params = listing_params(&sort, opt_str(a, "time").as_deref(), after(a))?;
            params.push(("sort", sort));
            listing(&get(&format!("/user/{name}/{segment}.json"), params).await?)
        }
        "rd_saved" => {
            let (name, _) = session().await?;
            let params = listing_params("new", None, after(a))?;
            listing(&get(&format!("/user/{name}/saved.json"), params).await?)
        }
        "rd_subscriptions" => {
            let mut params = listing_params("new", None, after(a))?;
            params[0].1 = "100".into();
            listing(&get("/subreddits/mine/subscriber.json", params).await?)
        }
        "rd_vote" => {
            let id = need_fullname(a)?;
            let direction = opt_str(a, "direction").unwrap_or_else(|| "up".into());
            let dir = match direction.as_str() {
                "up" => "1",
                "down" => "-1",
                "clear" => "0",
                _ => return Err("INVALID_ARGUMENTS: direction must be up, down or clear".into()),
            };
            post("/api/vote", &[("id", &id), ("dir", dir)]).await?;
            Ok(json!({ "ok": true, "id": id, "direction": direction }))
        }
        "rd_comment" => {
            let parent = match opt_str(a, "reply_to") {
                Some(c) => fullname(&format!("t1_{}", c.trim_start_matches("t1_")))
                    .ok_or("INVALID_ARGUMENTS: reply_to must be a comment id")?,
                None => need_fullname(a)?,
            };
            let text = req_str(a, "text")?;
            let v = post("/api/comment", &[("thing_id", &parent), ("text", &text)]).await?;
            Ok(json!({ "ok": true, "comment_id": opt_s(&v["things"][0]["data"]["id"]) }))
        }
        "rd_submit" => {
            let sub = need_sub(a, "subreddit")?;
            let title = req_str(a, "title")?;
            let v = match (opt_str(a, "url"), opt_str(a, "text")) {
                (Some(_), Some(_)) => return Err(
                    "INVALID_ARGUMENTS: pass text for a text post or url for a link post, not both"
                        .into(),
                ),
                (Some(url), None) => {
                    if !url.starts_with("https://") && !url.starts_with("http://") {
                        return Err(
                            "INVALID_ARGUMENTS: url must start with http:// or https://".into()
                        );
                    }
                    post(
                        "/api/submit",
                        &[
                            ("sr", &sub),
                            ("kind", "link"),
                            ("title", &title),
                            ("url", &url),
                            ("resubmit", "true"),
                        ],
                    )
                    .await?
                }
                (None, text) => {
                    post(
                        "/api/submit",
                        &[
                            ("sr", &sub),
                            ("kind", "self"),
                            ("title", &title),
                            ("text", text.as_deref().unwrap_or_default()),
                        ],
                    )
                    .await?
                }
            };
            Ok(json!({ "ok": true, "id": opt_s(&v["id"]), "url": opt_s(&v["url"]) }))
        }
        "rd_subscribe" => {
            let sub = need_sub(a, "subreddit")?;
            let action = if undo(a) { "unsub" } else { "sub" };
            post("/api/subscribe", &[("action", action), ("sr_name", &sub)]).await?;
            Ok(json!({ "ok": true, "subreddit": sub, "subscribed": !undo(a) }))
        }
        "rd_save" => {
            let id = need_fullname(a)?;
            let path = if undo(a) { "/api/unsave" } else { "/api/save" };
            post(path, &[("id", &id)]).await?;
            Ok(json!({ "ok": true, "id": id, "saved": !undo(a) }))
        }
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(payload: Value) -> String {
        let enc = |x: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(x);
        format!("{}.{}.sig", enc("{}"), enc(&payload.to_string()))
    }

    #[test]
    fn token_v2_tells_a_user_from_a_visitor() {
        assert!(token_is_user(&jwt(
            json!({ "sub": "user", "aid": "t2_abc" })
        )));
        assert!(!token_is_user(&jwt(
            json!({ "sub": "loid", "lid": "t2_x" })
        )));
        assert!(!token_is_user("garbage"));
        assert!(!token_is_user(""));
    }

    #[test]
    fn reads_names_and_ids_from_links() {
        assert_eq!(sub_of("r/rust").as_deref(), Some("rust"));
        assert_eq!(
            sub_of("https://www.reddit.com/r/Rust/top/?t=day").as_deref(),
            Some("Rust")
        );
        assert_eq!(sub_of("rust").as_deref(), Some("rust"));
        assert_eq!(sub_of("r/has space"), None);
        assert_eq!(user_of("u/spez").as_deref(), Some("spez"));
        assert_eq!(
            user_of("https://www.reddit.com/user/some-one/comments").as_deref(),
            Some("some-one")
        );
        assert_eq!(user_of("a/../b").as_deref(), Some("a"));
        assert_eq!(user_of("../x"), None);
        assert!(is_sub("https://reddit.com/r/rust"));
        assert!(!is_sub("spez"));
        assert_eq!(
            post_id("https://www.reddit.com/r/rust/comments/1abc2d/some_title/").as_deref(),
            Some("1abc2d")
        );
        assert_eq!(post_id("https://redd.it/1abc2d").as_deref(), Some("1abc2d"));
        assert_eq!(post_id("t3_1abc2d").as_deref(), Some("1abc2d"));
        assert_eq!(post_id("1ABC"), None);
        assert_eq!(fullname("1abc2d").as_deref(), Some("t3_1abc2d"));
        assert_eq!(fullname("t1_k9x").as_deref(), Some("t1_k9x"));
        assert_eq!(fullname("t1_k9x&dir=1"), None);
    }

    #[test]
    fn builds_feed_paths_and_listing_params() {
        assert_eq!(feed_path("home", None).unwrap(), "/best.json");
        assert_eq!(feed_path("popular", None).unwrap(), "/r/popular/hot.json");
        assert_eq!(feed_path("all", Some("top")).unwrap(), "/r/all/top.json");
        assert!(feed_path("all", Some("best")).is_err());
        assert!(feed_path("mine", None).is_err());
        assert_eq!(
            listing_params("top", Some("week"), Some("t3_abc".into())).unwrap(),
            vec![
                ("limit", "25".to_string()),
                ("t", "week".to_string()),
                ("after", "t3_abc".to_string())
            ]
        );
        assert_eq!(
            listing_params("hot", Some("week"), None).unwrap(),
            vec![("limit", "25".to_string())]
        );
        assert!(listing_params("top", Some("decade"), None).is_err());
        assert_eq!(
            after(&json!({ "cursor": "t3_1ab" })).as_deref(),
            Some("t3_1ab")
        );
        assert_eq!(after(&json!({ "cursor": "x&y=1" })), None);
    }

    #[test]
    fn builds_search_requests() {
        let (path, params) = search_request(
            "tauri app",
            "posts",
            Some("top"),
            Some("month"),
            Some("r/rust"),
            None,
        )
        .unwrap();
        assert_eq!(path, "/r/rust/search.json");
        assert_eq!(
            web::query(&params),
            "q=tauri%20app&limit=25&t=month&type=link&sort=top&restrict_sr=1"
        );
        let (path, params) = search_request("rust", "subreddits", None, None, None, None).unwrap();
        assert_eq!(path, "/search.json");
        assert_eq!(
            web::query(&params),
            "q=rust&limit=25&type=sr&sort=relevance"
        );
        assert!(search_request("x", "users", None, None, None, None).is_ok());
        assert!(search_request("x", "pins", None, None, None, None).is_err());
        assert!(search_request("x", "posts", Some("best"), None, None, None).is_err());
        assert!(search_request("x", "posts", None, None, Some("bad name"), None).is_err());
    }

    const SEARCH_POSTS: &str = r##"<main><div class="flex" data-testid="search-post-unit"> <search-telemetry-tracker data-faceplate-tracking-context="{&quot;action_info&quot;:{&quot;position&quot;:0,&quot;type&quot;:&quot;post&quot;},&quot;post&quot;:{&quot;id&quot;:&quot;t3_1ab2cd3&quot;,&quot;nsfw&quot;:false,&quot;spoiler&quot;:true,&quot;title&quot;:&quot;Rust &amp; Tauri: a &#39;native&#39; rewrite&quot;},&quot;profile&quot;:{&quot;id&quot;:&quot;t2_abc123&quot;,&quot;name&quot;:&quot;some_author&quot;},&quot;search&quot;:{&quot;range&quot;:&quot;month&quot;,&quot;sort&quot;:&quot;TOP&quot;},&quot;subreddit&quot;:{&quot;id&quot;:&quot;t5_2abcd&quot;,&quot;name&quot;:&quot;example_sub&quot;,&quot;nsfw&quot;:false,&quot;quarantined&quot;:false}}" click-events="search/click/post"><h2 class="m-0"> <a data-testid="post-title" href="/r/example_sub/comments/1ab2cd3/rust_tauri_a_native_rewrite/" class="absolute inset-0" aria-label="Rust &amp; Tauri: a &#39;native&#39; rewrite"> <faceplate-screen-reader-content>Rust &amp; Tauri</faceplate-screen-reader-content> </a> </h2></search-telemetry-tracker> <div class="w-full" data-testid="sdui-post-unit"> <a class="flex" href="/r/example_sub/"> <img src="https://styles.redditmedia.com/t5_2abcd/styles/communityIcon_x.png?width=64&amp;s=1" alt=""> <span class="truncate">r/example_sub</span></a> <span class="mx-2xs">·</span><faceplate-timeago format="narrow" ts="2026-11-04T21:50:25.996000+0000"></faceplate-timeago> <a data-testid="post-title-text" id="search-post-title-t3_1ab2cd3" href="/r/example_sub/comments/1ab2cd3/rust_tauri_a_native_rewrite/"> Rust &amp; Tauri </a> <div class="text-12" data-testid="search-counter-row"> <span><faceplate-number number="3273" pretty=""></faceplate-number> votes</span><span class="mx-2xs">·</span><span><faceplate-number number="578" pretty=""></faceplate-number> comments</span> </div> </div><a rpl="" href="/r/example_sub/comments/1ab2cd3/rust_tauri_a_native_rewrite/"><faceplate-img alt="x" data-testid="search_post_thumbnail" class="block" src="https://preview.redd.it/abc.png?width=140&amp;height=140&amp;s=9"></faceplate-img></a> </div>
<div class="flex" data-testid="search-post-unit"> <search-telemetry-tracker data-faceplate-tracking-context="{&quot;post&quot;:{&quot;id&quot;:&quot;t3_1zz9yy8&quot;,&quot;nsfw&quot;:true,&quot;spoiler&quot;:false,&quot;title&quot;:&quot;Second post&quot;},&quot;profile&quot;:{&quot;id&quot;:&quot;t2_def456&quot;,&quot;name&quot;:&quot;other-author&quot;},&quot;subreddit&quot;:{&quot;id&quot;:&quot;t5_3efgh&quot;,&quot;name&quot;:&quot;another&quot;,&quot;nsfw&quot;:true}}" click-events="search/click/post"><h2> <a data-testid="post-title" href="/r/another/comments/1zz9yy8/second_post/" aria-label="Second post"> </a> </h2></search-telemetry-tracker> <faceplate-timeago ts="2026-11-01T08:00:00.000000+0000"></faceplate-timeago> <div data-testid="search-counter-row"> <span><faceplate-number number="1" pretty=""></faceplate-number> vote</span><span>·</span><span><faceplate-number number="0" pretty=""></faceplate-number> comments</span> </div> </div>
<faceplate-partial loading="lazy" src="/svc/shreddit/search/?q=tauri&amp;type=posts&amp;sort=top&amp;t=month&amp;cursor=eyJjYW5kaWRhdGVzIjoiNyJ9%3D"></faceplate-partial>
<aside><div class="flex" data-testid="search-community"> <search-telemetry-tracker data-faceplate-tracking-context="{&quot;subreddit&quot;:{&quot;id&quot;:&quot;t5_9side&quot;,&quot;name&quot;:&quot;sidebar_sub&quot;,&quot;nsfw&quot;:false}}"></search-telemetry-tracker></div></aside></main>"##;

    const SEARCH_COMMUNITIES: &str = r##"<div class="flex" data-testid="search-community"> <search-telemetry-tracker data-faceplate-tracking-context="{&quot;action_info&quot;:{&quot;position&quot;:1},&quot;subreddit&quot;:{&quot;id&quot;:&quot;t5_4abcd2&quot;,&quot;name&quot;:&quot;ExampleKit&quot;,&quot;nsfw&quot;:false,&quot;quarantined&quot;:false}}" click-events="search/click/subreddit"><a href="/r/ExampleKit/" class="absolute"> <faceplate-screen-reader-content>ExampleKit</faceplate-screen-reader-content> </a></search-telemetry-tracker> <span rpl=""> <img src="https://styles.redditmedia.com/t5_4abcd2/styles/communityIcon_y.png?width=64&amp;frame=1" alt="" width="48"> </span> <h2 class="m-0"><span data-testid="search-community-title" id="search-community-title-t5_4abcd2" dir="auto">r/ExampleKit</span></h2> <div> <p class="m-0" data-testid="search-subreddit-desc-text" dir="auto"> anything about kits &amp; tools </p> <div class="flex-wrap"> <span><faceplate-number number="1200" pretty=""></faceplate-number> weekly visitors</span> <span>·</span> <span> <faceplate-number number="34" pretty=""></faceplate-number> weekly contributions</span> </div> </div> <shreddit-join-button subreddit-id="t5_4abcd2" name="ExampleKit"></shreddit-join-button> </div>
<div class="flex" data-testid="search-community"> <search-telemetry-tracker data-faceplate-tracking-context="{&quot;subreddit&quot;:{&quot;id&quot;:&quot;t5_5noicon&quot;,&quot;name&quot;:&quot;plain&quot;,&quot;nsfw&quot;:true}}"></search-telemetry-tracker> <svg rpl=""></svg> <span data-testid="search-community-title">r/plain</span> </div>
<faceplate-partial src="/svc/shreddit/search/?q=kit&amp;type=communities&amp;cursor=Y29tbQ%3D%3D" loading="lazy"></faceplate-partial>"##;

    const SEARCH_PEOPLE: &str = r##"<div class="flex" data-testid="search-author"> <search-telemetry-tracker data-faceplate-tracking-context="{&quot;action_info&quot;:{&quot;position&quot;:1},&quot;profile&quot;:{&quot;id&quot;:&quot;t2_5abcdef&quot;,&quot;name&quot;:&quot;sample-person&quot;},&quot;search&quot;:{&quot;safe_search&quot;:&quot;OFF&quot;}}" click-events="search/click/people"><a href="/user/sample-person/" class="absolute"> </a></search-telemetry-tracker> <span class="snoovatar"><img src="https://www.redditstatic.com/avatars/defaults/v2/avatar_default_2.png" alt="sample-person u/sample-person avatar"> </span> <h2><span data-testid="search-community-title" id="search-results-people-t2_5abcdef"><span data-testid="nameplate"> <span class="truncate">u/sample-person</span> </span></span></h2> <div> <p class="m-0"><span> <faceplate-number number="26830" pretty=""></faceplate-number> karma </span></p><p class="m-0" data-testid="search-subreddit-desc-text" dir="auto"> just a sample bio </p> </div> </div>"##;

    #[test]
    fn reads_posts_from_the_search_page_markup() {
        let v = search_html(SEARCH_POSTS, "posts").unwrap();
        assert_eq!(v["count"], 2);
        assert_eq!(v["source"], "web_page");
        assert_eq!(v["next_cursor"], "eyJjYW5kaWRhdGVzIjoiNyJ9=");
        let p = &v["items"][0];
        assert_eq!(p["id"], "1ab2cd3");
        assert_eq!(
            p["url"],
            "https://www.reddit.com/r/example_sub/comments/1ab2cd3/rust_tauri_a_native_rewrite/"
        );
        assert_eq!(p["extra"]["title"], "Rust & Tauri: a 'native' rewrite");
        assert_eq!(p["extra"]["subreddit"], "example_sub");
        assert_eq!(p["extra"]["spoiler"], true);
        assert!(p["extra"].get("nsfw").is_none());
        assert_eq!(p["author"]["handle"], "some_author");
        assert_eq!(p["author"]["id"], "t2_abc123");
        assert_eq!(p["stats"]["likes"], 3273);
        assert_eq!(p["stats"]["replies"], 578);
        assert_eq!(p["created_at"], 1793829025);
        assert_eq!(
            p["media"][0]["url"],
            "https://preview.redd.it/abc.png?width=140&height=140&s=9"
        );
        let q = &v["items"][1];
        assert_eq!(q["id"], "1zz9yy8");
        assert_eq!(q["extra"]["nsfw"], true);
        assert_eq!(q["stats"]["likes"], 1);
        assert_eq!(q["stats"]["replies"], 0);
        assert!(q["media"].as_array().unwrap().is_empty());
    }

    #[test]
    fn reads_communities_and_people_from_the_search_page_markup() {
        let v = search_html(SEARCH_COMMUNITIES, "subreddits").unwrap();
        assert_eq!(v["count"], 2);
        assert_eq!(v["next_cursor"], "Y29tbQ==");
        let c = &v["items"][0];
        assert_eq!(c["handle"], "r/ExampleKit");
        assert_eq!(c["id"], "t5_4abcd2");
        assert_eq!(c["url"], "https://www.reddit.com/r/ExampleKit/");
        assert_eq!(c["bio"], "anything about kits & tools");
        assert_eq!(
            c["avatar"],
            "https://styles.redditmedia.com/t5_4abcd2/styles/communityIcon_y.png?width=64&frame=1"
        );
        assert_eq!(c["extra"]["weekly_visitors"], 1200);
        assert_eq!(c["extra"]["kind"], "subreddit");
        assert_eq!(v["items"][1]["handle"], "r/plain");
        assert_eq!(v["items"][1]["avatar"], Value::Null);
        assert_eq!(v["items"][1]["extra"]["nsfw"], true);
        let u = search_html(SEARCH_PEOPLE, "users").unwrap();
        assert_eq!(u["count"], 1);
        assert_eq!(u["next_cursor"], Value::Null);
        let p = &u["items"][0];
        assert_eq!(p["handle"], "sample-person");
        assert_eq!(p["id"], "t2_5abcdef");
        assert_eq!(p["likes"], 26830);
        assert_eq!(p["bio"], "just a sample bio");
        assert_eq!(p["extra"]["kind"], "user");
        assert_eq!(search_html(SEARCH_POSTS, "subreddits").unwrap()["count"], 1);
    }

    #[test]
    fn a_page_that_is_not_the_search_page_is_reported() {
        assert!(search_html("<html><body>blocked</body></html>", "posts")
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
        let empty = search_html("<shreddit-app><main></main></shreddit-app>", "posts").unwrap();
        assert_eq!(empty["count"], 0);
        assert_eq!(empty["next_cursor"], Value::Null);
    }

    #[test]
    fn builds_search_page_addresses() {
        assert_eq!(
            search_page_url("tauri app", "posts", Some("top"), Some("month"), None, None).unwrap(),
            "https://www.reddit.com/search/?q=tauri%20app&type=posts&sort=top&t=month"
        );
        assert_eq!(
            search_page_url("kit", "subreddits", None, None, None, Some("Y29tbQ==")).unwrap(),
            "https://www.reddit.com/svc/shreddit/search/?q=kit&type=communities&sort=relevance&cursor=Y29tbQ%3D%3D"
        );
        assert_eq!(
            search_page_url("x", "users", None, None, Some("r/rust"), None).unwrap(),
            "https://www.reddit.com/search/?q=x&type=people&sort=relevance"
        );
        assert_eq!(
            search_page_url(
                "tauri",
                "posts",
                Some("new"),
                Some("week"),
                Some("r/rust"),
                Some("abc")
            )
            .unwrap(),
            "https://www.reddit.com/svc/shreddit/r/rust/search/?q=tauri&sort=new&cursor=abc"
        );
        assert!(search_page_url("x", "pins", None, None, None, None).is_err());
        assert!(search_page_url("x", "posts", Some("best"), None, None, None).is_err());
        assert!(search_page_url("x", "posts", None, Some("decade"), None, None).is_err());
    }

    #[test]
    fn tells_a_listing_cursor_from_a_search_page_cursor() {
        assert_eq!(
            search_cursor(&json!({ "cursor": "t3_1ab" })),
            Some((false, "t3_1ab".to_string()))
        );
        assert_eq!(
            search_cursor(&json!({ "cursor": "eyJjYW5kaWRhdGVzIjoiNyJ9=" })),
            Some((true, "eyJjYW5kaWRhdGVzIjoiNyJ9=".to_string()))
        );
        assert_eq!(search_cursor(&json!({ "cursor": "a b&c=1" })), None);
        assert_eq!(search_cursor(&json!({})), None);
    }

    fn sample_link() -> Value {
        json!({ "id": "1abc2d", "name": "t3_1abc2d", "title": "Rust 2.0 &amp; beyond", "selftext": "", "author": "ana", "author_fullname": "t2_9", "subreddit": "rust", "permalink": "/r/rust/comments/1abc2d/rust_20/", "url": "https://blog.example/post?a=1&amp;b=2", "domain": "blog.example", "is_self": false, "score": 1200, "num_comments": 88, "upvote_ratio": 0.97, "created_utc": 1767312000.0, "likes": true, "saved": false, "over_18": false, "stickied": false,
            "preview": { "images": [{ "source": { "url": "https://preview.redd.it/x.jpg?s=1&amp;w=2", "width": 800, "height": 600 } }] } })
    }

    #[test]
    fn parses_a_link_post() {
        let p = link(&sample_link()).unwrap();
        assert_eq!(p["id"], "1abc2d");
        assert_eq!(
            p["url"],
            "https://www.reddit.com/r/rust/comments/1abc2d/rust_20/"
        );
        assert_eq!(p["author"]["handle"], "ana");
        assert_eq!(p["stats"]["likes"], 1200);
        assert_eq!(p["stats"]["replies"], 88);
        assert_eq!(p["created_at"], 1767312000);
        assert_eq!(p["liked"], true);
        assert_eq!(
            p["media"][0]["url"],
            "https://preview.redd.it/x.jpg?s=1&w=2"
        );
        assert_eq!(p["extra"]["title"], "Rust 2.0 &amp; beyond");
        assert_eq!(p["extra"]["subreddit"], "rust");
        assert_eq!(p["extra"]["link"], "https://blog.example/post?a=1&b=2");
        assert_eq!(p["extra"]["vote"], 1);
        assert!(p["extra"].get("nsfw").is_none());
        assert!(link(&json!({})).is_none());
    }

    #[test]
    fn parses_video_gallery_and_self_posts() {
        let v = link(&json!({ "id": "v1", "permalink": "/r/a/comments/v1/x/", "is_self": false, "url": "https://v.redd.it/zz", "secure_media": { "reddit_video": { "fallback_url": "https://v.redd.it/zz/DASH_720.mp4?source=fallback" } } })).unwrap();
        assert_eq!(v["media"][0]["kind"], "video");
        let g = link(&json!({ "id": "g1", "permalink": "/r/a/comments/g1/x/", "is_self": false,
            "gallery_data": { "items": [{ "media_id": "m2" }, { "media_id": "m1" }, { "media_id": "gone" }] },
            "media_metadata": { "m1": { "s": { "u": "https://i.redd.it/1.jpg", "x": 10, "y": 20 } }, "m2": { "s": { "u": "https://i.redd.it/2.jpg" } } } })).unwrap();
        assert_eq!(g["media"].as_array().unwrap().len(), 2);
        assert_eq!(g["media"][0]["url"], "https://i.redd.it/2.jpg");
        let t = link(&json!({ "id": "s1", "permalink": "/r/a/comments/s1/x/", "is_self": true, "selftext": "hello", "url": "https://www.reddit.com/r/a/comments/s1/x/", "likes": null })).unwrap();
        assert_eq!(t["text"], "hello");
        assert!(t["extra"].get("link").is_none());
        assert_eq!(t["extra"]["vote"], 0);
        assert!(t["media"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parses_users_subreddits_and_mixed_listings() {
        let user = json!({ "kind": "t2", "data": { "id": "9", "name": "ana", "icon_img": "https://a/b.png?x=1&amp;y=2", "snoovatar_img": "", "link_karma": 10, "comment_karma": 5, "total_karma": 15, "created_utc": 1500000000.0, "verified": true, "subreddit": { "title": "Ana", "public_description": "hi", "subscribers": 3 } } });
        let sub = json!({ "kind": "t5", "data": { "name": "t5_2", "display_name": "rust", "title": "Rust", "public_description": "lang", "subscribers": 300000, "accounts_active": 900, "user_is_subscriber": true, "subreddit_type": "public", "community_icon": "" , "icon_img": "i.png" } });
        let comment = json!({ "kind": "t1", "data": { "id": "c1", "author": "bia", "body": "nice", "permalink": "/r/rust/comments/1/x/c1/", "link_id": "t3_1", "link_title": "Post", "score": 4 } });
        let v = listing(&json!({ "kind": "Listing", "data": { "after": "t3_zz", "children": [user, sub, comment, { "kind": "t3", "data": sample_link() }, { "kind": "more", "data": {} }] } })).unwrap();
        assert_eq!(v["count"], 4);
        assert_eq!(v["next_cursor"], "t3_zz");
        let u = &v["items"][0];
        assert_eq!(u["id"], "t2_9");
        assert_eq!(u["avatar"], "https://a/b.png?x=1&y=2");
        assert_eq!(u["likes"], 15);
        assert_eq!(u["followers"], 3);
        assert_eq!(u["extra"]["kind"], "user");
        let r = &v["items"][1];
        assert_eq!(r["handle"], "r/rust");
        assert_eq!(r["followed_by_me"], true);
        assert_eq!(r["avatar"], "i.png");
        assert_eq!(r["extra"]["active"], 900);
        let c = &v["items"][2];
        assert_eq!(c["id"], "t1_c1");
        assert_eq!(c["extra"]["kind"], "comment");
        assert_eq!(c["reply_to"], "t3_1");
        assert!(listing(&json!({ "error": 404 }))
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
    }

    fn reply(id: &str, replies: Value) -> Value {
        json!({ "kind": "t1", "data": { "id": id, "author": "u", "body": id, "parent_id": "t3_p", "score": 1, "replies": replies } })
    }

    fn nest(children: Vec<Value>) -> Value {
        json!({ "kind": "Listing", "data": { "children": children } })
    }

    #[test]
    fn comment_tree_is_flattened_depth_first_and_capped() {
        let tree = nest(vec![
            reply(
                "a",
                nest(vec![
                    reply("a1", nest(vec![reply("a1x", json!(""))])),
                    reply("a2", json!("")),
                ]),
            ),
            reply("b", json!("")),
            json!({ "kind": "more", "data": { "count": 12, "children": ["z"] } }),
        ]);
        let (all, cut) = comment_tree(&tree, 8, 50);
        let ids: Vec<&str> = all.iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a", "a1", "a1x", "a2", "b"]);
        assert_eq!(all[0]["replies"], 2);
        assert_eq!(all[0]["extra"]["depth"], 0);
        assert_eq!(all[2]["extra"]["depth"], 2);
        assert!(cut);
        let (shallow, cut) = comment_tree(&tree, 2, 50);
        let ids: Vec<&str> = shallow.iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a", "a1", "a2", "b"]);
        assert!(cut);
        let (few, cut) = comment_tree(&tree, 8, 2);
        assert_eq!(few.len(), 2);
        assert!(cut);
        let (whole, cut) = comment_tree(
            &nest(vec![reply("a", json!("")), reply("b", json!(""))]),
            3,
            50,
        );
        assert_eq!(whole.len(), 2);
        assert!(!cut);
    }

    #[test]
    fn post_detail_reports_totals() {
        let v = json!([
            nest(vec![json!({ "kind": "t3", "data": sample_link() })]),
            nest(vec![reply("a", json!(""))])
        ]);
        let d = post_detail(&v, 3, 50).unwrap();
        assert_eq!(d["post"]["id"], "1abc2d");
        assert_eq!(d["comments"]["returned"], 1);
        assert_eq!(d["comments"]["total"], 88);
        assert_eq!(d["comments"]["truncated"], false);
        assert!(post_detail(&json!({}), 3, 50)
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
    }

    #[test]
    fn reads_the_account_and_api_errors() {
        assert_eq!(
            account(&json!({ "kind": "t2", "data": { "name": "ana", "modhash": "mh" } })),
            Some(("ana".into(), "mh".into()))
        );
        assert_eq!(
            account(&json!({ "name": "ana" })),
            Some(("ana".into(), String::new()))
        );
        assert_eq!(account(&json!({})), None);
        let ok =
            api_result(json!({ "json": { "errors": [], "data": { "id": "t3_x", "url": "u" } } }))
                .unwrap();
        assert_eq!(ok["id"], "t3_x");
        assert!(api_result(
            json!({ "json": { "errors": [["RATELIMIT", "try again in 9 minutes", "ratelimit"]] } })
        )
        .unwrap_err()
        .starts_with("RATE_LIMITED"));
        assert!(
            api_result(json!({ "json": { "errors": [["SUBREDDIT_NOEXIST", "nope", "sr"]] } }))
                .unwrap_err()
                .starts_with("REJECTED: Forums refused the request: SUBREDDIT_NOEXIST")
        );
        assert!(
            api_result(json!({ "json": { "errors": [["USER_REQUIRED", "log in", null]] } }))
                .unwrap_err()
                .starts_with("NOT_LOGGED_IN")
        );
        assert!(api_result(json!({})).is_ok());
    }

    #[test]
    fn a_refused_listing_is_retried_on_the_oauth_host_without_the_json_suffix() {
        assert_eq!(
            oauth_url("/search.json", "q=rust&raw_json=1"),
            "https://oauth.reddit.com/search?q=rust&raw_json=1"
        );
        assert_eq!(
            oauth_url("/r/rust/about", "raw_json=1"),
            "https://oauth.reddit.com/r/rust/about?raw_json=1"
        );
    }

    #[test]
    fn friendly_maps_http_statuses() {
        assert!(friendly("UPSTREAM_404: Forums answered 404".into()).starts_with("NOT_FOUND"));
        assert!(friendly("UPSTREAM_403: x".into()).starts_with("REJECTED"));
        assert_eq!(friendly("NETWORK: x".into()), "NETWORK: x");
    }
}
