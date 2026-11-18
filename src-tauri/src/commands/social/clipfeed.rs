use std::time::Duration;

use serde_json::{json, Value};

use super::args::{opt_str, req_str, undo};
use super::model::{
    b, opt_s, page, s, u, Author, Comment, Media, MediaKind, Platform, Post, Profile, Stats,
};
use super::page as web;

const P: Platform = Platform::ClipFeed;
const COMMON: &str = "aid=1988&app_name=tiktok_web&device_platform=web_pc";

fn xhr_script(method: &str, path: &str, body: Option<&str>) -> String {
    format!(
        "new Promise(r=>{{const x=new XMLHttpRequest();x.open({m},{u},true);x.withCredentials=true;\
         x.setRequestHeader('accept','application/json, text/plain, */*');\
         {ct}x.onload=()=>r({{status:x.status,body:x.responseText}});x.onerror=()=>r({{status:0,body:''}});\
         x.send({b});}})",
        m = Value::String(method.into()),
        u = Value::String(path.into()),
        ct = if body.is_some() { "x.setRequestHeader('content-type','application/x-www-form-urlencoded');" } else { "" },
        b = body.map(|b| Value::String(b.into()).to_string()).unwrap_or_else(|| "null".into()),
    )
}

const ENV: &str = "(()=>{let c=null;try{c=JSON.parse(document.getElementById('__UNIVERSAL_DATA_FOR_REHYDRATION__').textContent).__DEFAULT_SCOPE__['webapp.app-context']}catch(_){}\
    return {path:location.pathname,ua:navigator.userAgent,platform:navigator.platform,lang:navigator.language,\
    tz:Intl.DateTimeFormat().resolvedOptions().timeZone,wid:c&&c.wid,odin:c&&c.odinId,created:c&&c.webIdCreatedTime,\
    language:c&&c.language,region:c&&c.region}})()";

pub fn usable(env: &Value) -> bool {
    opt_s(&env["wid"]).is_some() && !s(&env["path"]).starts_with("/@")
}

pub fn device_query(env: &Value) -> Option<String> {
    let wid = opt_s(&env["wid"])?;
    let ua = s(&env["ua"]);
    let platform = s(&env["platform"]);
    let os = if platform.starts_with("Mac") {
        "mac"
    } else if platform.starts_with("Win") {
        "windows"
    } else {
        "linux"
    };
    let language = opt_s(&env["language"]).unwrap_or_else(|| "en".into());
    let region = s(&env["region"]);
    Some(web::query(&[
        ("WebIdLastTime", s(&env["created"])),
        ("app_language", language.clone()),
        ("browser_language", s(&env["lang"])),
        ("browser_name", "Mozilla".into()),
        ("browser_online", "true".into()),
        ("browser_platform", platform.clone()),
        (
            "browser_version",
            ua.strip_prefix("Mozilla/").unwrap_or(&ua).to_string(),
        ),
        ("channel", "tiktok_web".into()),
        ("cookie_enabled", "true".into()),
        ("data_collection_enabled", "true".into()),
        ("device_id", wid),
        ("focus_state", "true".into()),
        ("from_page", "user".into()),
        ("history_len", "3".into()),
        ("is_fullscreen", "false".into()),
        ("is_page_visible", "true".into()),
        ("language", language.clone()),
        ("odinId", s(&env["odin"])),
        ("os", os.into()),
        ("priority_region", region.clone()),
        ("referer", String::new()),
        ("region", region),
        ("screen_height", "1080".into()),
        ("screen_width", "1920".into()),
        ("tz_name", s(&env["tz"])),
        ("user_is_login", "true".into()),
        ("webcast_language", language),
    ]))
}

async fn feed_page(b: &omniget_core::core::global_dl::browser::Browser) -> Result<Value, String> {
    let home = format!("{}/", P.origin());
    let tab = b
        .tab_for_origin(&home)
        .ok_or("BROWSER_UNAVAILABLE: the Clip Feed tab closed")?;
    if let Ok(v) = b.eval(&tab.id, ENV, Duration::from_secs(5)).await {
        if usable(&v) {
            return Ok(v);
        }
    }
    b.navigate(&tab.id, &home).await?;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Ok(v) = b.eval(&tab.id, ENV, Duration::from_secs(5)).await {
            if usable(&v) {
                return Ok(v);
            }
        }
    }
    Err("UPSTREAM_CHANGED: the Clip Feed home page did not load in the OmniGet browser (open tiktok.com there; it may be showing a check)".into())
}

async fn call_api(method: &str, path: &str) -> Result<Value, String> {
    let b = web::ready(P).await?;
    feed_page(&b).await?;
    let sep = if path.contains('?') { '&' } else { '?' };
    let full = format!("{path}{sep}{COMMON}");
    let r = web::eval(
        &b,
        P,
        &xhr_script(method, &full, (method == "POST").then_some("")),
    )
    .await?;
    let status = r["status"].as_u64().unwrap_or(0);
    let body = r["body"].as_str().unwrap_or_default();
    if status == 0 {
        return Err("NETWORK: could not reach Clip Feed".into());
    }
    if status == 401 || status == 403 {
        return Err(format!(
            "NOT_LOGGED_IN: Clip Feed answered {status}; sign in again"
        ));
    }
    if status == 429 {
        return Err("RATE_LIMITED: Clip Feed is limiting this account; wait a few minutes".into());
    }
    if body.trim().is_empty() {
        return Err(format!("UPSTREAM_CHANGED: Clip Feed answered {status} with an empty body, which is how it refuses a request it does not trust; open tiktok.com in the OmniGet browser, solve any check it shows and try again"));
    }
    let v: Value = serde_json::from_str(body).map_err(|_| {
        format!("UPSTREAM_CHANGED: Clip Feed answered {status} with something that is not JSON")
    })?;
    let code = v
        .get("statusCode")
        .or_else(|| v.get("status_code"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if code != 0 {
        let msg = opt_s(&v["statusMsg"])
            .or_else(|| opt_s(&v["status_msg"]))
            .unwrap_or_else(|| format!("status {code}"));
        if code == 8 || msg.to_ascii_lowercase().contains("login") {
            return Err(format!(
                "NOT_LOGGED_IN: Clip Feed asked to sign in again ({msg})"
            ));
        }
        return Err(format!("REJECTED: Clip Feed refused the request: {msg}"));
    }
    Ok(v)
}

fn first_url(v: &Value) -> Option<String> {
    opt_s(v).or_else(|| {
        v["url_list"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(opt_s)
    })
}

pub fn author(a: &Value) -> Author {
    Author {
        id: opt_s(&a["id"])
            .or_else(|| opt_s(&a["uid"]))
            .unwrap_or_default(),
        handle: opt_s(&a["uniqueId"])
            .or_else(|| opt_s(&a["unique_id"]))
            .unwrap_or_default(),
        name: opt_s(&a["nickname"]).unwrap_or_default(),
        avatar: first_url(&a["avatarThumb"])
            .or_else(|| first_url(&a["avatar_thumb"]))
            .or_else(|| first_url(&a["avatarMedium"])),
        verified: b(&a["verified"]) || a["custom_verify"].as_str().is_some_and(|x| !x.is_empty()),
    }
}

fn stat(i: &Value, key: &str) -> Option<u64> {
    u(&i["stats"][key]).or_else(|| u(&i["statsV2"][key]))
}

pub fn item(i: &Value) -> Option<Post> {
    let id = s(&i["id"]);
    if id.is_empty() {
        return None;
    }
    let author = author(&i["author"]);
    let images: Vec<Media> = i["imagePost"]["images"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|img| {
                    Some(Media {
                        kind: MediaKind::Image,
                        url: img["imageURL"]["urlList"]
                            .as_array()?
                            .first()
                            .and_then(opt_s)?,
                        thumb: None,
                        width: u(&img["imageWidth"]),
                        height: u(&img["imageHeight"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let photo = !images.is_empty();
    let media = if photo {
        images
    } else {
        let v = &i["video"];
        opt_s(&v["playAddr"])
            .or_else(|| opt_s(&v["downloadAddr"]))
            .map(|url| Media {
                kind: MediaKind::Video,
                url,
                thumb: opt_s(&v["cover"]).or_else(|| opt_s(&v["originCover"])),
                width: u(&v["width"]),
                height: u(&v["height"]),
            })
            .into_iter()
            .collect()
    };
    Some(Post {
        url: format!(
            "https://www.tiktok.com/@{}/{}/{id}",
            if author.handle.is_empty() {
                "i"
            } else {
                &author.handle
            },
            if photo { "photo" } else { "video" }
        ),
        id,
        text: s(&i["desc"]),
        created_at: u(&i["createTime"]).map(|n| n as i64),
        media,
        stats: Stats {
            likes: stat(i, "diggCount"),
            replies: stat(i, "commentCount"),
            reposts: stat(i, "shareCount"),
            views: stat(i, "playCount"),
            saves: stat(i, "collectCount"),
        },
        liked: i["digged"].as_bool(),
        author,
        ..Default::default()
    })
}

pub fn profile(info: &Value) -> Option<Profile> {
    let user = &info["user"];
    let a = author(user);
    if a.handle.is_empty() {
        return None;
    }
    let st = |k: &str| u(&info["stats"][k]).or_else(|| u(&info["statsV2"][k]));
    Some(Profile {
        url: format!("https://www.tiktok.com/@{}", a.handle),
        id: a.id,
        handle: a.handle,
        name: a.name,
        avatar: first_url(&user["avatarLarger"]).or(a.avatar),
        bio: s(&user["signature"]),
        verified: a.verified,
        private: b(&user["privateAccount"]),
        followers: st("followerCount"),
        following: st("followingCount"),
        posts: st("videoCount"),
        likes: st("heartCount").or_else(|| st("heart")),
        followed_by_me: user["relation"].as_i64().map(|r| r == 1 || r == 2),
        follows_me: None,
        secondary_id: opt_s(&user["secUid"]),
    })
}

fn search_user(x: &Value) -> Option<Profile> {
    let i = &x["user_info"];
    let a = author(i);
    if a.handle.is_empty() {
        return None;
    }
    Some(Profile {
        url: format!("https://www.tiktok.com/@{}", a.handle),
        id: a.id,
        handle: a.handle,
        name: a.name,
        avatar: a.avatar,
        bio: s(&i["signature"]),
        verified: a.verified,
        followers: u(&i["follower_count"]),
        secondary_id: opt_s(&i["sec_uid"]),
        ..Default::default()
    })
}

fn comment(c: &Value) -> Comment {
    Comment {
        id: s(&c["cid"]),
        author: author(&c["user"]),
        text: s(&c["text"]),
        created_at: c["create_time"].as_i64(),
        likes: u(&c["digg_count"]),
        replies: u(&c["reply_comment_total"]),
    }
}

pub fn video_id(input: &str) -> Option<String> {
    let t = input.trim();
    if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
        return Some(t.to_string());
    }
    let re = regex::Regex::new(r"/(?:video|photo|v)/(\d+)").expect("static regex");
    re.captures(t).map(|c| c[1].to_string())
}

pub fn username_of(input: &str) -> String {
    let t = input.trim();
    let t = match t.split_once("tiktok.com/") {
        Some((_, rest)) => rest.split(['/', '?']).next().unwrap_or_default(),
        None => t,
    };
    t.trim_start_matches('@').to_string()
}

fn items(v: &Value, key: &str) -> Vec<Post> {
    v[key]
        .as_array()
        .map(|a| a.iter().filter_map(item).collect())
        .unwrap_or_default()
}

fn more(v: &Value, cursor_key: &str) -> Option<String> {
    let has = v["hasMore"]
        .as_bool()
        .or_else(|| v["has_more"].as_bool())
        .or_else(|| v["has_more"].as_i64().map(|n| n != 0))
        .unwrap_or(false);
    if has {
        opt_s(&v[cursor_key]).or_else(|| Some("more".into()))
    } else {
        None
    }
}

async fn page_scope(path: &str) -> Result<Value, String> {
    let b = web::ready(P).await?;
    let expr = format!(
        "(async()=>{{const r=await fetch({},{{credentials:'include'}});const h=await r.text();\
         const m=h.match(/<script id=\"__UNIVERSAL_DATA_FOR_REHYDRATION__\"[^>]*>([\\s\\S]*?)<\\/script>/);\
         if(!m)return {{status:r.status}};try{{return {{status:r.status,scope:JSON.parse(m[1]).__DEFAULT_SCOPE__}}}}catch(e){{return {{status:r.status}}}}}})()",
        Value::String(path.into())
    );
    let r = web::eval(&b, P, &expr).await?;
    if r["scope"].is_object() {
        return Ok(r["scope"].clone());
    }
    let tab = b
        .tab_for_origin(&format!("{}/", P.origin()))
        .ok_or("BROWSER_UNAVAILABLE: the Clip Feed tab closed")?;
    b.navigate(&tab.id, &format!("{}{path}", P.origin()))
        .await?;
    const READ: &str = "(()=>{const e=document.getElementById('__UNIVERSAL_DATA_FOR_REHYDRATION__');\
        if(!e)return null;try{return JSON.parse(e.textContent).__DEFAULT_SCOPE__}catch(_){return null}})()";
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Ok(v) = b.eval(&tab.id, READ, Duration::from_secs(5)).await {
            if v.is_object() {
                return Ok(v);
            }
        }
    }
    Err(format!(
        "UPSTREAM_CHANGED: Clip Feed answered {} without the page data (open tiktok.com in the OmniGet browser; it may be showing a check)",
        r["status"]
    ))
}

async fn user(username: &str) -> Result<Profile, String> {
    let scope = page_scope(&format!("/@{}", urlencoding::encode(username))).await?;
    let detail = &scope["webapp.user-detail"];
    profile(&detail["userInfo"])
        .ok_or_else(|| format!("NOT_FOUND: @{username} does not exist on Clip Feed"))
}

pub fn follow_list(v: &Value) -> Option<(Vec<Profile>, Option<String>)> {
    let list = v["userList"].as_array();
    if list.is_none() && v.get("hasMore").is_none() && v.get("total").is_none() {
        return None;
    }
    let items = list
        .map(|a| a.iter().filter_map(profile).collect())
        .unwrap_or_default();
    let next = more(v, "maxCursor").map(|max| {
        format!(
            "{max},{}",
            opt_s(&v["minCursor"]).unwrap_or_else(|| "0".into())
        )
    });
    Some((items, next))
}

async fn follow_page(scene: u8, a: &Value) -> Result<Value, String> {
    let (sec, max, min) = match opt_str(a, "cursor") {
        Some(c) => {
            let bad = "INVALID_ARGUMENTS: cursor must be the next_cursor of the previous page";
            let (sec, rest) = c.split_once('~').ok_or(bad)?;
            let (max, min) = rest.split_once(',').ok_or(bad)?;
            let digits =
                |x: &str| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit() || c == b'-');
            if !digits(max) || !digits(min) {
                return Err(bad.into());
            }
            (sec.to_string(), max.to_string(), min.to_string())
        }
        None => {
            let sec = match opt_str(a, "username") {
                Some(name) => user(&username_of(&name)).await?.secondary_id,
                None => opt_s(&me().await?["secondary_id"]),
            };
            let sec = sec.ok_or("UPSTREAM_CHANGED: Clip Feed did not return the profile secUid")?;
            (sec, "0".to_string(), "0".to_string())
        }
    };
    let v = call_api(
        "GET",
        &format!(
            "/api/user/list/?count=30&maxCursor={max}&minCursor={min}&scene={scene}&secUid={}",
            urlencoding::encode(&sec)
        ),
    )
    .await?;
    let (items, next) = follow_list(&v)
        .ok_or("UPSTREAM_CHANGED: Clip Feed answered the user list in a shape that is not known")?;
    Ok(page(items, next.map(|c| format!("{sec}~{c}"))))
}

pub fn posts_path(env: &Value, sec: &str, cursor: &str) -> Result<String, String> {
    let device = device_query(env)
        .ok_or("UPSTREAM_CHANGED: the Clip Feed page did not expose its device data")?;
    Ok(format!(
        "/api/post/item_list/?{device}&coverFormat=2&count=20&cursor={cursor}&secUid={}",
        urlencoding::encode(sec)
    ))
}

fn need_video(a: &Value) -> Result<String, String> {
    video_id(&req_str(a, "post")?).ok_or_else(|| "INVALID_ARGUMENTS: post must be a Clip Feed video id or a full tiktok.com/@user/video/... link".into())
}

fn cursor(a: &Value) -> String {
    opt_str(a, "cursor")
        .filter(|c| c.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or_else(|| "0".into())
}

pub async fn me() -> Result<Value, String> {
    let name = match call_api("GET", "/passport/web/account/info/").await {
        Ok(v) => opt_s(&v["data"]["username"]),
        Err(_) => None,
    };
    let name = match name {
        Some(n) => n,
        None => opt_s(&page_scope("/").await?["webapp.app-context"]["user"]["uniqueId"])
            .ok_or("NOT_LOGGED_IN: Clip Feed did not return the signed-in account")?,
    };
    Ok(json!(user(&name).await?))
}

pub async fn call(tool: &str, a: &Value) -> Result<Value, String> {
    match tool {
        "tt_me" => me().await,
        "tt_feed" => {
            let v = call_api("GET", "/api/recommend/item_list/?count=20&from_page=fyp").await?;
            Ok(page(items(&v, "itemList"), more(&v, "__none")))
        }
        "tt_profile" => Ok(json!(user(&username_of(&req_str(a, "username")?)).await?)),
        "tt_user_posts" => {
            let p = user(&username_of(&req_str(a, "username")?)).await?;
            let sec = p
                .secondary_id
                .clone()
                .ok_or("UPSTREAM_CHANGED: Clip Feed did not return the profile secUid")?;
            let b = web::ready(P).await?;
            let env = feed_page(&b).await?;
            let v = call_api("GET", &posts_path(&env, &sec, &cursor(a))?).await?;
            Ok(page(items(&v, "itemList"), more(&v, "cursor")))
        }
        "tt_post" => {
            let id = need_video(a)?;
            let v = call_api("GET", &format!("/api/item/detail/?itemId={id}")).await?;
            let post = item(&v["itemInfo"]["itemStruct"])
                .ok_or("NOT_FOUND: that Clip Feed post is not available")?;
            let c = call_api(
                "GET",
                &format!("/api/comment/list/?aweme_id={id}&count=20&cursor=0"),
            )
            .await?;
            let comments: Vec<Comment> = c["comments"]
                .as_array()
                .map(|x| x.iter().map(comment).collect())
                .unwrap_or_default();
            Ok(json!({ "post": post, "comments": page(comments, more(&c, "cursor")) }))
        }
        "tt_comments" => {
            let id = need_video(a)?;
            let c = call_api(
                "GET",
                &format!(
                    "/api/comment/list/?aweme_id={id}&count=20&cursor={}",
                    cursor(a)
                ),
            )
            .await?;
            let comments: Vec<Comment> = c["comments"]
                .as_array()
                .map(|x| x.iter().map(comment).collect())
                .unwrap_or_default();
            Ok(page(comments, more(&c, "cursor")))
        }
        "tt_search" => {
            let q = urlencoding::encode(&req_str(a, "query")?).into_owned();
            if opt_str(a, "kind").as_deref() == Some("users") {
                let v = call_api(
                    "GET",
                    &format!("/api/search/user/full/?keyword={q}&cursor={}", cursor(a)),
                )
                .await?;
                let users: Vec<Profile> = v["user_list"]
                    .as_array()
                    .map(|x| x.iter().filter_map(search_user).collect())
                    .unwrap_or_default();
                return Ok(page(users, more(&v, "cursor")));
            }
            let v = call_api(
                "GET",
                &format!("/api/search/general/full/?keyword={q}&offset={}", cursor(a)),
            )
            .await?;
            let posts: Vec<Post> = v["data"]
                .as_array()
                .map(|x| x.iter().filter_map(|d| item(&d["item"])).collect())
                .unwrap_or_else(|| items(&v, "item_list"));
            Ok(page(posts, more(&v, "cursor")))
        }
        "tt_like" => {
            let id = need_video(a)?;
            call_api(
                "POST",
                &format!(
                    "/api/commit/item/digg/?aweme_id={id}&type={}",
                    if undo(a) { 0 } else { 1 }
                ),
            )
            .await?;
            Ok(json!({ "ok": true, "liked": !undo(a) }))
        }
        "tt_comment" => {
            let id = need_video(a)?;
            let text = req_str(a, "text")?;
            if text.chars().count() > 150 {
                return Err(
                    "INVALID_ARGUMENTS: Clip Feed comments are limited to 150 characters".into(),
                );
            }
            let reply = opt_str(a, "reply_to")
                .filter(|r| r.bytes().all(|c| c.is_ascii_digit()))
                .unwrap_or_else(|| "0".into());
            let v = call_api("POST", &format!("/api/comment/publish/?aweme_id={id}&text={}&reply_id={reply}&text_extra=%5B%5D", urlencoding::encode(&text))).await?;
            Ok(json!({ "ok": true, "comment_id": opt_s(&v["comment"]["cid"]) }))
        }
        "tt_followers" => follow_page(67, a).await,
        "tt_following" => follow_page(21, a).await,
        "tt_follow" => {
            let p = user(&username_of(&req_str(a, "username")?)).await?;
            call_api(
                "POST",
                &format!("/api/commit/follow/user/?user_id={}&type={}&action_type={}&from=18&from_pre=0&channel_id=3", p.id, if undo(a) { 0 } else { 1 }, if undo(a) { 0 } else { 1 }),
            )
            .await?;
            Ok(json!({ "ok": true, "user": p.handle, "following": !undo(a) }))
        }
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_and_photo_posts() {
        let v = item(&json!({ "id": "7", "desc": "dança", "createTime": "100", "digged": false,
            "author": { "id": "1", "uniqueId": "ana", "nickname": "Ana", "avatarThumb": "a.jpg", "verified": true },
            "stats": { "diggCount": 5, "playCount": 50 }, "statsV2": { "commentCount": "2" },
            "video": { "playAddr": "v.mp4", "cover": "c.jpg" } })).unwrap();
        assert_eq!(v.url, "https://www.tiktok.com/@ana/video/7");
        assert_eq!(v.media[0].kind, MediaKind::Video);
        assert_eq!(v.stats.replies, Some(2));
        assert_eq!(v.created_at, Some(100));
        let p = item(&json!({ "id": "8", "author": { "uniqueId": "ana" },
            "imagePost": { "images": [{ "imageURL": { "urlList": ["i.jpg"] } }] } }))
        .unwrap();
        assert_eq!(p.url, "https://www.tiktok.com/@ana/photo/8");
        assert_eq!(p.media[0].url, "i.jpg");
    }

    #[test]
    fn reads_ids_users_and_cursors() {
        assert_eq!(
            video_id("https://www.tiktok.com/@a/video/123?x").as_deref(),
            Some("123")
        );
        assert_eq!(username_of("https://www.tiktok.com/@ana?lang=pt"), "ana");
        assert_eq!(username_of("@ana"), "ana");
        assert_eq!(
            more(&json!({ "hasMore": true, "cursor": "30" }), "cursor").as_deref(),
            Some("30")
        );
        assert_eq!(more(&json!({ "has_more": 0 }), "cursor"), None);
        let c = comment(
            &json!({ "cid": "9", "text": "oi", "user": { "uid": "3", "unique_id": "bia", "avatar_thumb": { "url_list": ["b.jpg"] } } }),
        );
        assert_eq!(c.author.handle, "bia");
        assert_eq!(c.author.avatar.as_deref(), Some("b.jpg"));
    }

    #[test]
    fn reads_a_followers_page() {
        let v = json!({ "statusCode": 0, "hasMore": true, "maxCursor": 1700000000, "minCursor": 30, "total": 2, "userList": [
            { "user": { "id": "1", "uniqueId": "ana", "nickname": "Ana", "signature": "oi", "secUid": "MS4", "verified": true, "privateAccount": true, "avatarThumb": "a.jpg" },
              "stats": { "followerCount": 9, "followingCount": 4, "videoCount": 3 } },
            { "user": { "id": "2", "nickname": "no handle" } } ] });
        let (items, next) = follow_list(&v).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].handle, "ana");
        assert_eq!(items[0].followers, Some(9));
        assert_eq!(items[0].posts, Some(3));
        assert!(items[0].private);
        assert_eq!(next.as_deref(), Some("1700000000,30"));
        let (last, end) =
            follow_list(&json!({ "statusCode": 0, "hasMore": false, "total": 2 })).unwrap();
        assert!(last.is_empty());
        assert_eq!(end, None);
        assert!(follow_list(&json!({ "statusCode": 0 })).is_none());
    }

    #[test]
    fn profile_keeps_the_sec_uid() {
        let p = profile(&json!({ "user": { "id": "1", "uniqueId": "ana", "secUid": "MS4", "privateAccount": false },
            "stats": { "followerCount": 9, "heartCount": 99, "videoCount": 3 } })).unwrap();
        assert_eq!(p.secondary_id.as_deref(), Some("MS4"));
        assert_eq!(p.likes, Some(99));
    }

    fn sample_env(path: &str) -> Value {
        json!({ "path": path, "ua": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36", "platform": "MacIntel", "lang": "en-US", "tz": "America/Recife",
            "wid": "7500000000000000001", "odin": "7500000000000000002", "created": "1785871370", "language": "en", "region": "BR" })
    }

    #[test]
    fn post_lists_carry_the_device_data_of_the_page() {
        let path = posts_path(&sample_env("/"), "MS4w+abc", "1788534031735").unwrap();
        assert!(path.starts_with("/api/post/item_list/?WebIdLastTime=1785871370&app_language=en&browser_language=en-US&browser_name=Mozilla"));
        assert!(path.contains("&browser_version=5.0%20%28Macintosh%3B%20Intel%20Mac%20OS%20X%2010_15_7%29%20AppleWebKit%2F537.36&"));
        assert!(path.contains("&device_id=7500000000000000001&"));
        assert!(path
            .contains("&odinId=7500000000000000002&os=mac&priority_region=BR&referer=&region=BR&"));
        assert!(path.contains("&tz_name=America%2FRecife&user_is_login=true&webcast_language=en"));
        assert!(path.ends_with("&coverFormat=2&count=20&cursor=1788534031735&secUid=MS4w%2Babc"));
        assert!(!path.contains("aid=1988"));
        assert!(posts_path(&json!({ "path": "/" }), "x", "0")
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
    }

    #[test]
    fn api_calls_wait_for_a_page_that_is_not_a_profile() {
        assert!(usable(&sample_env("/")));
        assert!(usable(&sample_env("/foryou")));
        assert!(!usable(&sample_env("/@someone")));
        assert!(!usable(&json!({ "path": "/" })));
    }

    #[test]
    fn reads_a_post_list_page() {
        let v = json!({ "statusCode": 0, "hasMore": true, "cursor": "1788534031735", "itemList": [
            { "id": "7600000000000000001", "desc": "a clip", "createTime": 1788468783, "digged": false,
              "author": { "id": "100", "uniqueId": "sample.account", "nickname": "Sample", "secUid": "MS4w", "verified": true, "avatarThumb": "https://p16.example/a.jpeg" },
              "stats": { "collectCount": 1188, "commentCount": 1395, "diggCount": 14000, "playCount": 256200, "shareCount": 755 },
              "statsV2": { "collectCount": "1188", "repostCount": "0" },
              "video": { "cover": "https://p16.example/c.jpeg", "playAddr": "https://v16.example/v.mp4", "width": 576, "height": 1024, "duration": 30 } } ] });
        let posts = items(&v, "itemList");
        assert_eq!(posts.len(), 1);
        assert_eq!(
            posts[0].url,
            "https://www.tiktok.com/@sample.account/video/7600000000000000001"
        );
        assert_eq!(posts[0].created_at, Some(1788468783));
        assert_eq!(posts[0].stats.views, Some(256200));
        assert_eq!(posts[0].stats.likes, Some(14000));
        assert_eq!(posts[0].stats.saves, Some(1188));
        assert_eq!(more(&v, "cursor").as_deref(), Some("1788534031735"));
    }

    #[test]
    fn xhr_script_quotes_everything() {
        let s = xhr_script("POST", "/api/x?a='1'", Some(""));
        assert!(s.contains(r#"x.open("POST","/api/x?a='1'",true)"#));
        assert!(s.contains("content-type"));
    }
}
