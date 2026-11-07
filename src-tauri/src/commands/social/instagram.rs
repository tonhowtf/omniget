use serde_json::{json, Map, Value};

use super::args::{opt_str, req_str, undo};
use super::model::{
    b, opt_s, page, s, u, Author, Comment, Media, MediaKind, Platform, Post, Profile, Stats,
};
use super::page as web;

const P: Platform = Platform::Instagram;
const API: &str = "https://www.instagram.com/api/v1";
const APP_ID: &str = "936619743392459";
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

async fn headers(post: bool) -> Result<Map<String, Value>, String> {
    let b = web::ready(P).await?;
    let csrf = web::cookie(&b, P, "csrftoken").await.unwrap_or_default();
    let mut h = web::headers(&[
        ("x-ig-app-id", APP_ID),
        ("x-csrftoken", &csrf),
        ("x-requested-with", "XMLHttpRequest"),
    ]);
    if post {
        h.insert(
            "content-type".into(),
            json!("application/x-www-form-urlencoded"),
        );
    }
    Ok(h)
}

fn ok(v: Value) -> Result<Value, String> {
    if v["status"] == "fail" {
        let msg = opt_s(&v["message"]).unwrap_or_else(|| "request failed".into());
        if v["require_login"].as_bool() == Some(true) || msg == "login_required" {
            return Err("NOT_LOGGED_IN: Instagram asked to sign in again".into());
        }
        return Err(format!("REJECTED: Instagram refused the request: {msg}"));
    }
    Ok(v)
}

async fn get(path: &str) -> Result<Value, String> {
    ok(web::fetch(
        P,
        "GET",
        &format!("{API}{path}"),
        headers(false).await?,
        None,
    )
    .await?
    .json()?)
}

async fn post(path: &str, form: String) -> Result<Value, String> {
    ok(web::fetch(
        P,
        "POST",
        &format!("{API}{path}"),
        headers(true).await?,
        Some(form),
    )
    .await?
    .json()?)
}

pub fn author(u_: &Value) -> Author {
    Author {
        id: opt_s(&u_["pk"])
            .or_else(|| opt_s(&u_["id"]))
            .or_else(|| opt_s(&u_["pk_id"]))
            .unwrap_or_default(),
        handle: s(&u_["username"]),
        name: s(&u_["full_name"]),
        avatar: opt_s(&u_["profile_pic_url"]),
        verified: b(&u_["is_verified"]),
    }
}

fn best_image(m: &Value) -> Option<(String, Option<u64>, Option<u64>)> {
    let c = m["image_versions2"]["candidates"].as_array()?.first()?;
    Some((s(&c["url"]), u(&c["width"]), u(&c["height"])))
}

fn one_media(m: &Value) -> Option<Media> {
    let img = best_image(m);
    if let Some(v) = m["video_versions"].as_array().and_then(|a| a.first()) {
        return Some(Media {
            kind: MediaKind::Video,
            url: s(&v["url"]),
            thumb: img.as_ref().map(|i| i.0.clone()),
            width: u(&v["width"]),
            height: u(&v["height"]),
        });
    }
    let (url, width, height) = img?;
    Some(Media {
        kind: MediaKind::Image,
        url,
        thumb: None,
        width,
        height,
    })
}

pub fn media_post(m: &Value) -> Option<Post> {
    let pk = opt_s(&m["pk"])
        .or_else(|| s(&m["id"]).split('_').next().map(str::to_string))
        .filter(|x| !x.is_empty())?;
    let code = s(&m["code"]);
    let kind = if m["product_type"] == "clips" {
        "reel"
    } else {
        "p"
    };
    let media = match m["carousel_media"].as_array() {
        Some(items) => items.iter().filter_map(one_media).collect(),
        None => one_media(m).into_iter().collect(),
    };
    Some(Post {
        url: if code.is_empty() {
            String::new()
        } else {
            format!("https://www.instagram.com/{kind}/{code}/")
        },
        id: pk,
        author: author(if m["user"].is_object() {
            &m["user"]
        } else {
            &m["owner"]
        }),
        text: s(&m["caption"]["text"]),
        created_at: m["taken_at"].as_i64(),
        media,
        stats: Stats {
            likes: u(&m["like_count"]),
            replies: u(&m["comment_count"]),
            reposts: None,
            views: u(&m["play_count"])
                .or_else(|| u(&m["ig_play_count"]))
                .or_else(|| u(&m["view_count"])),
            saves: None,
        },
        liked: m["has_liked"].as_bool(),
        ..Default::default()
    })
}

pub fn profile(user: &Value) -> Option<Profile> {
    let a = author(user);
    if a.id.is_empty() {
        return None;
    }
    let count = |edge: &str, flat: &str| u(&user[edge]["count"]).or_else(|| u(&user[flat]));
    Some(Profile {
        url: format!("https://www.instagram.com/{}/", a.handle),
        id: a.id,
        handle: a.handle,
        name: a.name,
        avatar: opt_s(&user["profile_pic_url_hd"]).or(a.avatar),
        bio: s(&user["biography"]),
        verified: a.verified,
        private: b(&user["is_private"]),
        followers: count("edge_followed_by", "follower_count"),
        following: count("edge_follow", "following_count"),
        posts: count("edge_owner_to_timeline_media", "media_count"),
        likes: None,
        followed_by_me: user["followed_by_viewer"].as_bool(),
        follows_me: user["follows_viewer"].as_bool(),
        secondary_id: None,
    })
}

fn comment(c: &Value) -> Comment {
    Comment {
        id: s(&c["pk"]),
        author: author(&c["user"]),
        text: s(&c["text"]),
        created_at: c["created_at"].as_i64(),
        likes: u(&c["comment_like_count"]),
        replies: u(&c["child_comment_count"]),
    }
}

pub fn shortcode_to_pk(code: &str) -> Option<String> {
    let code: String = code.chars().take(11).collect();
    if code.is_empty() {
        return None;
    }
    let mut n: u128 = 0;
    for c in code.bytes() {
        let i = ALPHABET.iter().position(|&a| a == c)? as u128;
        n = n.checked_mul(64)?.checked_add(i)?;
    }
    Some(n.to_string())
}

pub fn media_pk(input: &str) -> Option<String> {
    let t = input.trim();
    if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
        return Some(t.to_string());
    }
    if let Some(pk) = t
        .split('_')
        .next()
        .filter(|x| x.len() > 8 && x.bytes().all(|c| c.is_ascii_digit()))
    {
        return Some(pk.to_string());
    }
    let re = regex::Regex::new(r"instagram\.com/(?:[^/]+/)?(?:p|reel|reels|tv)/([A-Za-z0-9_-]+)")
        .expect("static regex");
    if let Some(c) = re.captures(t) {
        return shortcode_to_pk(&c[1]);
    }
    if !t.contains('/') && t.len() <= 14 {
        return shortcode_to_pk(t);
    }
    None
}

pub fn username_of(input: &str) -> String {
    let t = input.trim().trim_start_matches('@');
    match t.split_once("instagram.com/") {
        Some((_, rest)) => rest
            .split(['/', '?'])
            .next()
            .unwrap_or_default()
            .to_string(),
        None => t.to_string(),
    }
}

async fn user(username: &str) -> Result<(Profile, Value), String> {
    let v = get(&format!(
        "/users/web_profile_info/?username={}",
        urlencoding::encode(username)
    ))
    .await?;
    let raw = v["data"]["user"].clone();
    let p = profile(&raw)
        .ok_or_else(|| format!("NOT_FOUND: @{username} does not exist on Instagram"))?;
    Ok((p, raw))
}

fn need_pk(a: &Value) -> Result<String, String> {
    media_pk(&req_str(a, "post")?).ok_or_else(|| {
        "INVALID_ARGUMENTS: post must be an Instagram link, shortcode or media id".into()
    })
}

fn cursor_q(a: &Value, key: &str) -> String {
    opt_str(a, "cursor")
        .map(|c| format!("&{key}={}", urlencoding::encode(&c)))
        .unwrap_or_default()
}

pub async fn me() -> Result<Value, String> {
    if let Ok(v) = get("/accounts/edit/web_form_data/").await {
        if let Some(name) = opt_s(&v["form_data"]["username"]) {
            return Ok(json!(user(&name).await?.0));
        }
    }
    let b = web::ready(P).await?;
    let id = web::cookie(&b, P, "ds_user_id")
        .await
        .ok_or("NOT_LOGGED_IN: Instagram did not return the signed-in account")?;
    let v = get(&format!("/users/{id}/info/")).await?;
    let name = opt_s(&v["user"]["username"])
        .ok_or("NOT_LOGGED_IN: Instagram did not return the signed-in account")?;
    Ok(json!(user(&name).await?.0))
}

fn is_ad(item: &Value) -> bool {
    item.get("ad").is_some() || !item["ad_id"].is_null() || !item["injected"].is_null()
}

pub fn collect_media(v: &Value, depth: u8, out: &mut Vec<Post>) {
    if depth > 5 || is_ad(v) {
        return;
    }
    if v.get("code").is_some()
        && (v.get("image_versions2").is_some() || v.get("carousel_media").is_some())
    {
        if let Some(p) = media_post(v) {
            if !out.iter().any(|x| x.id == p.id) {
                out.push(p);
            }
        }
        return;
    }
    match v {
        Value::Object(o) => o.values().for_each(|x| collect_media(x, depth + 1, out)),
        Value::Array(a) => a.iter().for_each(|x| collect_media(x, depth + 1, out)),
        _ => {}
    }
}

pub async fn call(tool: &str, a: &Value) -> Result<Value, String> {
    match tool {
        "ig_me" => me().await,
        "ig_feed" => {
            let mut f = vec![
                ("reason", "cold_start_fetch".to_string()),
                ("is_pull_to_refresh", "0".to_string()),
            ];
            if let Some(c) = opt_str(a, "cursor") {
                f = vec![("reason", "pagination".to_string()), ("max_id", c)];
            }
            let v = post("/feed/timeline/", web::query(&f)).await?;
            let mut items = Vec::new();
            if let Some(list) = v["feed_items"].as_array().or_else(|| v["items"].as_array()) {
                for item in list {
                    collect_media(item, 0, &mut items);
                }
            }
            Ok(page(items, opt_s(&v["next_max_id"])))
        }
        "ig_profile" => Ok(json!(user(&username_of(&req_str(a, "username")?)).await?.0)),
        "ig_user_posts" => {
            let (p, _) = user(&username_of(&req_str(a, "username")?)).await?;
            let v = get(&format!(
                "/feed/user/{}/?count=12{}",
                p.id,
                cursor_q(a, "max_id")
            ))
            .await?;
            let items = v["items"]
                .as_array()
                .map(|x| x.iter().filter_map(media_post).collect())
                .unwrap_or_default();
            let more = v["more_available"].as_bool().unwrap_or(false);
            Ok(page::<Post>(
                items,
                if more { opt_s(&v["next_max_id"]) } else { None },
            ))
        }
        "ig_post" => {
            let pk = need_pk(a)?;
            let v = get(&format!("/media/{pk}/info/")).await?;
            let post = v["items"]
                .as_array()
                .and_then(|x| x.first())
                .and_then(media_post);
            if post.is_none() {
                return Err("NOT_FOUND: that Instagram post is not available".into());
            }
            let c = get(&format!(
                "/media/{pk}/comments/?can_support_threading=true&permalink_enabled=false"
            ))
            .await?;
            let comments: Vec<Comment> = c["comments"]
                .as_array()
                .map(|x| x.iter().map(comment).collect())
                .unwrap_or_default();
            Ok(json!({ "post": post, "comments": page(comments, opt_s(&c["next_min_id"])) }))
        }
        "ig_comments" => {
            let pk = need_pk(a)?;
            let c = get(&format!(
                "/media/{pk}/comments/?can_support_threading=true&permalink_enabled=false{}",
                cursor_q(a, "min_id")
            ))
            .await?;
            let comments: Vec<Comment> = c["comments"]
                .as_array()
                .map(|x| x.iter().map(comment).collect())
                .unwrap_or_default();
            Ok(page(comments, opt_s(&c["next_min_id"])))
        }
        "ig_search" => {
            let q = urlencoding::encode(&req_str(a, "query")?).into_owned();
            let path =
                format!("/web/search/topsearch/?context=blended&include_reel=true&query={q}");
            let v = match get(&path).await {
                Ok(v) => v,
                Err(e) if e.starts_with("UPSTREAM_404") => {
                    match web::fetch(P, "GET", &format!("https://www.instagram.com{path}"), headers(false).await?, None).await {
                        Ok(r) => ok(r.json()?)?,
                        Err(e) if e.starts_with("UPSTREAM_404") => {
                            return Err("UPSTREAM_CHANGED: Instagram does not answer account search on the web for this session; open a profile by its username with ig_profile".into())
                        }
                        Err(e) => return Err(e),
                    }
                }
                Err(e) => return Err(e),
            };
            let users: Vec<Profile> = v["users"]
                .as_array()
                .map(|x| x.iter().filter_map(|i| profile(&i["user"])).collect())
                .unwrap_or_default();
            let tags: Vec<Value> = v["hashtags"]
                .as_array()
                .map(|x| x.iter().map(|h| json!({ "name": s(&h["hashtag"]["name"]), "posts": u(&h["hashtag"]["media_count"]) })).collect())
                .unwrap_or_default();
            Ok(json!({ "users": users, "hashtags": tags }))
        }
        "ig_saved" => {
            let v = get(&format!(
                "/feed/saved/posts/?{}",
                cursor_q(a, "max_id").trim_start_matches('&')
            ))
            .await?;
            let items: Vec<Post> = v["items"]
                .as_array()
                .map(|x| x.iter().filter_map(|i| media_post(&i["media"])).collect())
                .unwrap_or_default();
            let more = v["more_available"].as_bool().unwrap_or(false);
            Ok(page(
                items,
                if more { opt_s(&v["next_max_id"]) } else { None },
            ))
        }
        "ig_like" => {
            let pk = need_pk(a)?;
            post(
                &format!(
                    "/web/likes/{pk}/{}/",
                    if undo(a) { "unlike" } else { "like" }
                ),
                String::new(),
            )
            .await?;
            Ok(json!({ "ok": true, "liked": !undo(a) }))
        }
        "ig_comment" => {
            let pk = need_pk(a)?;
            let text = req_str(a, "text")?;
            if text.chars().count() > 2200 {
                return Err("INVALID_ARGUMENTS: text is longer than 2200 characters".into());
            }
            let mut f = vec![("comment_text", text)];
            if let Some(r) = opt_str(a, "reply_to") {
                f.push(("replied_to_comment_id", r));
            }
            let v = post(&format!("/web/comments/{pk}/add/"), web::query(&f)).await?;
            Ok(json!({ "ok": true, "comment_id": opt_s(&v["id"]).or_else(|| opt_s(&v["pk"])) }))
        }
        "ig_follow" => {
            let (p, _) = user(&username_of(&req_str(a, "username")?)).await?;
            post(
                &format!(
                    "/web/friendships/{}/{}/",
                    p.id,
                    if undo(a) { "unfollow" } else { "follow" }
                ),
                String::new(),
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
    fn shortcodes_become_media_ids() {
        assert_eq!(shortcode_to_pk("B").as_deref(), Some("1"));
        assert_eq!(
            media_pk("https://www.instagram.com/p/BA/").as_deref(),
            Some("64")
        );
        assert_eq!(
            media_pk("https://www.instagram.com/ana/reel/BA/?x=1").as_deref(),
            Some("64")
        );
        assert_eq!(
            media_pk("3140000000000000000_123").as_deref(),
            Some("3140000000000000000")
        );
        assert_eq!(username_of("https://www.instagram.com/ana/"), "ana");
        assert_eq!(username_of("@ana"), "ana");
    }

    #[test]
    fn parses_a_carousel() {
        let m = json!({ "pk": "9", "code": "Cx", "taken_at": 10, "like_count": 3, "comment_count": 1, "has_liked": false,
            "caption": { "text": "oi" }, "user": { "pk": "1", "username": "ana", "full_name": "Ana", "is_verified": true },
            "carousel_media": [
                { "image_versions2": { "candidates": [{ "url": "a.jpg", "width": 10, "height": 20 }] } },
                { "image_versions2": { "candidates": [{ "url": "t.jpg" }] }, "video_versions": [{ "url": "v.mp4" }] } ] });
        let p = media_post(&m).unwrap();
        assert_eq!(p.url, "https://www.instagram.com/p/Cx/");
        assert_eq!(p.media.len(), 2);
        assert_eq!(p.media[1].kind, MediaKind::Video);
        assert_eq!(p.media[1].thumb.as_deref(), Some("t.jpg"));
        assert_eq!(p.author.handle, "ana");
        assert_eq!(p.text, "oi");
    }

    #[test]
    fn parses_profile() {
        let p = profile(&json!({ "id": "5", "username": "ana", "edge_followed_by": { "count": 7 }, "is_private": true })).unwrap();
        assert_eq!(p.followers, Some(7));
        assert!(p.private);
    }

    #[test]
    fn feed_finds_media_inside_suggestions_and_skips_ads() {
        let m = |pk: &str| json!({ "pk": pk, "code": format!("C{pk}"), "image_versions2": { "candidates": [{ "url": "a.jpg" }] }, "user": { "pk": "1", "username": "ana" } });
        let items = json!([
            { "media_or_ad": m("1") },
            { "explore_story": { "media": m("2"), "more": [m("3"), m("2")] } },
            { "ad": { "media": m("4") } },
            { "media_or_ad": { "ad_id": "x", "pk": "5", "code": "C5", "image_versions2": {} } }
        ]);
        let mut out = Vec::new();
        for i in items.as_array().unwrap() {
            collect_media(i, 0, &mut out);
        }
        assert_eq!(
            out.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "2", "3"]
        );
    }
}
