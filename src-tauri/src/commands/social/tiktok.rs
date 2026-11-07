use serde_json::{json, Value};

use super::args::{opt_str, req_str, undo};
use super::model::{
    b, opt_s, page, s, u, Author, Comment, Media, MediaKind, Platform, Post, Profile, Stats,
};
use super::page as web;

const P: Platform = Platform::Tiktok;
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

async fn call_api(method: &str, path: &str) -> Result<Value, String> {
    let b = web::ready(P).await?;
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
        return Err("NETWORK: could not reach TikTok".into());
    }
    if status == 401 || status == 403 {
        return Err(format!(
            "NOT_LOGGED_IN: TikTok answered {status}; sign in again"
        ));
    }
    if status == 429 {
        return Err("RATE_LIMITED: TikTok is limiting this account; wait a few minutes".into());
    }
    if body.trim().is_empty() {
        return Err(format!("UPSTREAM_CHANGED: TikTok answered {status} with an empty body (the page may need a reload or a captcha)"));
    }
    let v: Value = serde_json::from_str(body).map_err(|_| {
        format!("UPSTREAM_CHANGED: TikTok answered {status} with something that is not JSON")
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
                "NOT_LOGGED_IN: TikTok asked to sign in again ({msg})"
            ));
        }
        return Err(format!("REJECTED: TikTok refused the request: {msg}"));
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

async fn user(username: &str) -> Result<Profile, String> {
    let v = call_api(
        "GET",
        &format!(
            "/api/user/detail/?uniqueId={}",
            urlencoding::encode(username)
        ),
    )
    .await?;
    profile(&v["userInfo"])
        .ok_or_else(|| format!("NOT_FOUND: @{username} does not exist on TikTok"))
}

fn need_video(a: &Value) -> Result<String, String> {
    video_id(&req_str(a, "post")?).ok_or_else(|| "INVALID_ARGUMENTS: post must be a TikTok video id or a full tiktok.com/@user/video/... link".into())
}

fn cursor(a: &Value) -> String {
    opt_str(a, "cursor")
        .filter(|c| c.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or_else(|| "0".into())
}

pub async fn me() -> Result<Value, String> {
    let v = call_api("GET", "/passport/web/account/info/").await?;
    let name = opt_s(&v["data"]["username"])
        .ok_or("NOT_LOGGED_IN: TikTok did not return the signed-in account")?;
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
                .ok_or("UPSTREAM_CHANGED: TikTok did not return the profile secUid")?;
            let v = call_api(
                "GET",
                &format!(
                    "/api/post/item_list/?secUid={}&count=20&cursor={}",
                    urlencoding::encode(&sec),
                    cursor(a)
                ),
            )
            .await?;
            Ok(page(items(&v, "itemList"), more(&v, "cursor")))
        }
        "tt_post" => {
            let id = need_video(a)?;
            let v = call_api("GET", &format!("/api/item/detail/?itemId={id}")).await?;
            let post = item(&v["itemInfo"]["itemStruct"])
                .ok_or("NOT_FOUND: that TikTok post is not available")?;
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
                    "INVALID_ARGUMENTS: TikTok comments are limited to 150 characters".into(),
                );
            }
            let reply = opt_str(a, "reply_to")
                .filter(|r| r.bytes().all(|c| c.is_ascii_digit()))
                .unwrap_or_else(|| "0".into());
            let v = call_api("POST", &format!("/api/comment/publish/?aweme_id={id}&text={}&reply_id={reply}&text_extra=%5B%5D", urlencoding::encode(&text))).await?;
            Ok(json!({ "ok": true, "comment_id": opt_s(&v["comment"]["cid"]) }))
        }
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
    fn profile_keeps_the_sec_uid() {
        let p = profile(&json!({ "user": { "id": "1", "uniqueId": "ana", "secUid": "MS4", "privateAccount": false },
            "stats": { "followerCount": 9, "heartCount": 99, "videoCount": 3 } })).unwrap();
        assert_eq!(p.secondary_id.as_deref(), Some("MS4"));
        assert_eq!(p.likes, Some(99));
    }

    #[test]
    fn xhr_script_quotes_everything() {
        let s = xhr_script("POST", "/api/x?a='1'", Some(""));
        assert!(s.contains(r#"x.open("POST","/api/x?a='1'",true)"#));
        assert!(s.contains("content-type"));
    }
}
