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

const P: Platform = Platform::ShortPost;
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

const RUNTIME: &str = "(()=>{const s=[...document.scripts].find(s=>!s.src&&/a\\.js\"/.test(s.textContent));return s?s.textContent:''})()";

pub fn lazy_chunk(operation: &str) -> Option<&'static str> {
    match operation {
        "Favoriters" | "Retweeters" => Some("TweetActivity"),
        _ => None,
    }
}

pub fn chunk_urls(runtime: &str, hint: &str) -> Vec<String> {
    let names = regex::Regex::new(r#"(\d+):"([A-Za-z0-9_.~/-]+)""#).expect("static regex");
    let hash = regex::Regex::new(r"^[a-f0-9]{16}$").expect("static regex");
    let mut hashes = HashMap::new();
    let mut wanted = Vec::new();
    for c in names.captures_iter(runtime) {
        if hash.is_match(&c[2]) {
            hashes.insert(c[1].to_string(), c[2].to_string());
        } else if c[2].contains(hint) {
            wanted.push((c[1].to_string(), c[2].to_string()));
        }
    }
    wanted
        .into_iter()
        .filter_map(|(id, name)| {
            Some(format!(
                "https://abs.twimg.com/responsive-web/client-web/{name}.{}a.js",
                hashes.get(&id)?
            ))
        })
        .collect()
}

const DISCOVER: &str = r#"(async(extra)=>{
const urls=new Set();
for(const s of document.querySelectorAll('script[src]'))urls.add(s.src);
for(const e of performance.getEntriesByType('resource'))urls.add(e.name);
const js=[...urls].filter(u=>/abs\.twimg\.com\/responsive-web\/client-web[^?]*\.js$/.test(u)).slice(0,60).concat(extra);
const ops={};
const re=/queryId:"([^"]+)",operationName:"([^"]+)",operationType:"[^"]+",metadata:\{featureSwitches:\[([^\]]*)\],fieldToggles:\[([^\]]*)\]/g;
const list=x=>x.split(',').map(v=>v.replace(/"/g,'').trim()).filter(Boolean);
const loose=/queryId:"([^"]+)",operationName:"([^"]+)"/g;
const scan=t=>{let m;while((m=loose.exec(t))){if(!ops[m[2]])ops[m[2]]={id:m[1],features:[],toggles:[]};}while((m=re.exec(t))){ops[m[2]]={id:m[1],features:list(m[3]),toggles:list(m[4])};}};
for(const k of Object.keys(window).filter(k=>k.startsWith('webpackChunk'))){for(const ch of window[k]||[]){const mods=ch&&ch[1];if(!mods)continue;for(const id in mods){try{const t=String(mods[id]);if(t.includes('queryId'))scan(t);}catch(e){}}}}
await Promise.all(js.map(async u=>{try{scan(await (await fetch(u)).text());}catch(e){}}));
const fs=(window.__INITIAL_STATE__&&window.__INITIAL_STATE__.featureSwitch)||{};
const config={};
for(const src of [fs.defaultConfig||{},(fs.user&&fs.user.config)||{}])for(const k in src){const v=src[k]&&src[k].value;if(typeof v==='boolean')config[k]=v;}
return {ops,config};
})"#;

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
    let mut found = web::eval(&b, P, &format!("{DISCOVER}([])"))
        .await
        .unwrap_or(Value::Null);
    if let Some(hint) = lazy_chunk(name).filter(|_| found["ops"][name].is_null()) {
        let runtime = web::eval(&b, P, RUNTIME).await.unwrap_or(Value::Null);
        let extra = chunk_urls(runtime.as_str().unwrap_or_default(), hint);
        if !extra.is_empty() {
            found = web::eval(&b, P, &format!("{DISCOVER}({})", json!(extra)))
                .await
                .unwrap_or(found);
        }
    }
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
        format!("UPSTREAM_CHANGED: could not find the Short Posts operation {name} in the web app; reload x.com in the OmniGet browser and try again")
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

pub(super) async fn base_headers() -> Result<Map<String, Value>, String> {
    let b = web::ready(P).await?;
    let ct0 = web::cookie(&b, P, "ct0")
        .await
        .ok_or("NOT_LOGGED_IN: the Short Posts session has no ct0 cookie; sign in again")?;
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
            "REJECTED: Short Posts refused the request: {} (code {})",
            s(&e["message"]),
            s(&e["code"])
        ));
    }
    Ok(())
}

pub(super) async fn gql(name: &str, variables: Value, mutation: bool) -> Result<Value, String> {
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

pub fn user_list(v: &Value) -> (Vec<Profile>, Option<String>) {
    fn walk(
        v: &Value,
        out: &mut Vec<Profile>,
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
                if o.get("itemType").and_then(Value::as_str) == Some("TimelineUser") {
                    let r = &o["user_results"]["result"];
                    if let Some(mut p) = profile(r) {
                        if b(&r["legacy"]["default_profile_image"]) {
                            p.avatar = None;
                        }
                        if seen.insert(p.id.clone()) {
                            out.push(p);
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
    let cursor = cursor.filter(|c| !out.is_empty() && !c.starts_with("0|"));
    (out, cursor)
}

pub fn users_timeline(v: &Value) -> (Vec<Profile>, Option<String>) {
    fn walk(
        v: &Value,
        out: &mut Vec<Profile>,
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
                if let Some(r) = o.get("user_results") {
                    if let Some(p) = profile(&r["result"]) {
                        if seen.insert(p.id.clone()) {
                            out.push(p);
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
    if out.is_empty() {
        cursor = None;
    }
    (out, cursor)
}

pub fn quotes_query(id: &str) -> String {
    format!("quoted_tweet_id:{id}")
}

pub async fn engagement_users(
    likes: bool,
    id: &str,
    cursor: Option<String>,
) -> Result<(Vec<Profile>, Option<String>), String> {
    let (name, key) = if likes {
        ("Favoriters", "favoriters_timeline")
    } else {
        ("Retweeters", "retweeters_timeline")
    };
    let mut vars = json!({ "tweetId": id, "count": 20, "includePromotedContent": true });
    if let Some(c) = cursor {
        vars["cursor"] = json!(c);
    }
    let v = gql(name, vars, false).await?;
    if !v["data"][key].is_object() {
        return Err(format!(
            "UPSTREAM_CHANGED: Short Posts answered {name} without data.{key}"
        ));
    }
    Ok(users_timeline(&v["data"][key]))
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

fn viewer_id(twid: &str) -> Option<String> {
    let raw = urlencoding::decode(twid).ok()?.into_owned();
    let id = raw.trim_matches('"').trim_start_matches("u=").to_string();
    (!id.is_empty() && id.bytes().all(|c| c.is_ascii_digit())).then_some(id)
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
    let b = web::ready(P).await?;
    let twid = web::cookie(&b, P, "twid")
        .await
        .ok_or("NOT_LOGGED_IN: the Short Posts session has no twid cookie; sign in again")?;
    let id = viewer_id(&twid)
        .ok_or("NOT_LOGGED_IN: Short Posts did not return the signed-in account")?;
    let v = gql(
        "UserByRestId",
        json!({ "userId": id, "withSafetyModeUserFields": true }),
        false,
    )
    .await?;
    let h = profile(&v["data"]["user"]["result"])
        .map(|p| p.handle)
        .unwrap_or_default();
    if h.is_empty() {
        return Err("NOT_LOGGED_IN: Short Posts did not return the signed-in account".into());
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
    profile(&v["data"]["user"]["result"]).ok_or_else(|| {
        format!("NOT_FOUND: @{handle} does not exist on Short Posts or is suspended")
    })
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

async fn follow_list(name: &str, a: &Value) -> Result<Value, String> {
    let (id, cursor) = match opt_str(a, "cursor") {
        Some(c) => {
            let (id, rest) = c
                .split_once('~')
                .ok_or("INVALID_ARGUMENTS: cursor must be the next_cursor of the previous page")?;
            (id.to_string(), Some(rest.to_string()))
        }
        None => {
            let h = match opt_str(a, "handle") {
                Some(h) => handle_of(&h),
                None => me_handle().await?,
            };
            (user(&h).await?.id, None)
        }
    };
    let mut vars = json!({ "userId": id, "count": 50, "includePromotedContent": false });
    if let Some(c) = cursor {
        vars["cursor"] = json!(c);
    }
    let v = gql(name, vars, false).await?;
    if v["data"]["user"]["result"]["timeline"].is_null() {
        return Err(format!(
            "UPSTREAM_CHANGED: Short Posts answered {name} without a user list"
        ));
    }
    let (items, next) = user_list(&v);
    Ok(with_locations(
        page(items, next.map(|c| format!("{id}~{c}"))),
        &v,
    ))
}

pub fn with_locations(mut page: Value, raw: &Value) -> Value {
    fn walk(v: &Value, out: &mut std::collections::HashMap<String, String>) {
        match v {
            Value::Object(o) => {
                if let Some(r) = o.get("user_results").map(|u| &u["result"]) {
                    let place = opt_s(&r["location"]["location"])
                        .or_else(|| opt_s(&r["legacy"]["location"]))
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty());
                    if let (Some(id), Some(place)) = (r["rest_id"].as_str(), place) {
                        out.insert(id.to_string(), place);
                    }
                    return;
                }
                o.values().for_each(|x| walk(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut places = std::collections::HashMap::new();
    walk(raw, &mut places);
    if let Some(items) = page["items"].as_array_mut() {
        for item in items {
            let place = item["id"].as_str().and_then(|id| places.get(id)).cloned();
            if let Some(place) = place {
                item["location"] = json!(place);
            }
        }
    }
    page
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
            let media: Vec<Value> = a["media_ids"]
                .as_array()
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| id.as_str())
                        .map(|id| json!({ "media_id": id, "tagged_users": [] }))
                        .collect()
                })
                .unwrap_or_default();
            let mut vars = json!({ "tweet_text": text, "dark_request": false, "media": { "media_entities": media, "possibly_sensitive": false }, "semantic_annotation_ids": [] });
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
        "x_followers" => follow_list("Followers", a).await,
        "x_following" => follow_list("Following", a).await,
        "x_assistant" | "x_assistant_chats" | "x_assistant_chat" => {
            super::assistant::call(tool, a).await
        }
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
        assert_eq!(
            viewer_id("u%3D1856424640672944128").as_deref(),
            Some("1856424640672944128")
        );
        assert_eq!(viewer_id("\"u=42\"").as_deref(), Some("42"));
        assert_eq!(viewer_id("guest"), None);
    }

    #[test]
    fn reads_a_followers_page() {
        let entry = |id: &str, handle: &str| {
            let mut u_ = user_json(handle);
            u_["rest_id"] = json!(id);
            json!({ "entryId": format!("user-{id}"), "content": { "itemContent": { "itemType": "TimelineUser", "user_results": { "result": u_ } } } })
        };
        let mut egg = entry("3", "egg");
        egg["content"]["itemContent"]["user_results"]["result"]["legacy"]
            ["default_profile_image"] = json!(true);
        let page = |entries: Vec<Value>| json!({ "data": { "user": { "result": { "timeline": { "timeline": { "instructions": [{ "type": "TimelineAddEntries", "entries": entries }] } } } } } });
        let (items, cursor) = user_list(&page(vec![
            entry("1", "ana"),
            entry("2", "bia"),
            egg,
            entry("1", "ana"),
            json!({ "content": { "itemContent": { "itemType": "TimelineUser", "user_results": { "result": { "__typename": "UserUnavailable" } } } } }),
            json!({ "content": { "cursorType": "Bottom", "value": "1789|abc" } }),
            json!({ "content": { "cursorType": "Top", "value": "-1|x" } }),
        ]));
        assert_eq!(
            items.iter().map(|p| p.handle.as_str()).collect::<Vec<_>>(),
            vec!["ana", "bia", "egg"]
        );
        assert_eq!(items[0].followers, Some(10));
        assert!(items[0].avatar.is_some());
        assert_eq!(items[2].avatar, None);
        assert_eq!(cursor.as_deref(), Some("1789|abc"));
        let (_, end) = user_list(&page(vec![
            entry("1", "ana"),
            json!({ "content": { "cursorType": "Bottom", "value": "0|abc" } }),
        ]));
        assert_eq!(end, None);
        let (none, end) = user_list(&page(vec![
            json!({ "content": { "cursorType": "Bottom", "value": "1790|abc" } }),
        ]));
        assert!(none.is_empty());
        assert_eq!(end, None);
    }

    #[test]
    fn finds_the_lazy_chunk_that_defines_the_engagement_operations() {
        let runtime = r#"t.u=e=>(({1001:"bundle.TweetActivity",1002:"shared~bundle.QuoteTweetActivity~bundle.TweetActivity",1003:"bundle.UserLists",1004:"icons/IconX-js"})[e]||e)+"."+({1001:"aaaaaaaaaaaaaaaa",1002:"bbbbbbbbbbbbbbbb",1003:"cccccccccccccccc",1005:"dddddddddddddddd"})[e]+"a.js""#;
        assert_eq!(
            chunk_urls(runtime, "TweetActivity"),
            vec![
                "https://abs.twimg.com/responsive-web/client-web/bundle.TweetActivity.aaaaaaaaaaaaaaaaa.js",
                "https://abs.twimg.com/responsive-web/client-web/shared~bundle.QuoteTweetActivity~bundle.TweetActivity.bbbbbbbbbbbbbbbba.js"
            ]
        );
        assert!(chunk_urls(runtime, "IconX").is_empty());
        assert!(chunk_urls("", "TweetActivity").is_empty());
        assert_eq!(lazy_chunk("Retweeters"), Some("TweetActivity"));
        assert_eq!(lazy_chunk("Favoriters"), Some("TweetActivity"));
        assert_eq!(lazy_chunk("TweetDetail"), None);
    }

    #[test]
    fn reads_the_accounts_of_an_engagement_list() {
        let mut ana = user_json("ana");
        ana["relationship_perspectives"] = json!({ "followed_by": true, "following": false });
        let mut bob = user_json("bob");
        bob["rest_id"] = json!("43");
        let v = json!({ "timeline": { "instructions": [{ "type": "TimelineAddEntries", "entries": [
            { "content": { "itemContent": { "itemType": "TimelineUser", "user_results": { "result": ana.clone() } } } },
            { "content": { "itemContent": { "itemType": "TimelineUser", "user_results": { "result": bob } } } },
            { "content": { "itemContent": { "itemType": "TimelineUser", "user_results": { "result": ana } } } },
            { "content": { "itemContent": { "itemType": "TimelineUser", "user_results": { "result": { "__typename": "UserUnavailable" } } } } },
            { "content": { "cursorType": "Top", "value": "top" } },
            { "content": { "cursorType": "Bottom", "value": "next" } } ] }] } });
        let (users, cursor) = users_timeline(&v);
        assert_eq!(
            users.iter().map(|p| p.handle.as_str()).collect::<Vec<_>>(),
            vec!["ana", "bob"]
        );
        assert_eq!(users[0].follows_me, Some(true));
        assert_eq!(users[1].follows_me, None);
        assert_eq!(cursor.as_deref(), Some("next"));
        let end = json!({ "timeline": { "instructions": [{ "entries": [
            { "content": { "cursorType": "Bottom", "value": "again" } } ] }] } });
        assert_eq!(users_timeline(&end), (Vec::new(), None));
        assert_eq!(quotes_query("77"), "quoted_tweet_id:77");
    }

    #[test]
    fn follower_pages_carry_the_location_of_each_account_when_x_sends_one() {
        let raw = json!({ "data": { "list": [
            { "itemType": "TimelineUser", "user_results": { "result": { "rest_id": "1", "legacy": { "location": " Lisboa " } } } },
            { "itemType": "TimelineUser", "user_results": { "result": { "rest_id": "2", "location": { "location": "Recife, Brasil" }, "legacy": { "location": "old" } } } },
            { "itemType": "TimelineUser", "user_results": { "result": { "rest_id": "3", "legacy": { "location": "" } } } }
        ] } });
        let page = json!({ "count": 3, "items": [{ "id": "1" }, { "id": "2" }, { "id": "3" }, { "id": "4" }] });
        let out = with_locations(page, &raw);
        assert_eq!(out["items"][0]["location"], "Lisboa");
        assert_eq!(out["items"][1]["location"], "Recife, Brasil");
        assert!(out["items"][2].get("location").is_none());
        assert!(out["items"][3].get("location").is_none());
        assert_eq!(out["count"], 3);
    }

    #[test]
    fn profile_reads_counts() {
        let p = profile(&user_json("ana")).unwrap();
        assert_eq!(p.followers, Some(10));
        assert_eq!(p.url, "https://x.com/ana");
        assert!(p.verified);
    }
}
