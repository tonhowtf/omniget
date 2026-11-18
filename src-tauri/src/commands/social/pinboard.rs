use serde_json::{json, Value};

use super::args::{opt_str, req_str, undo};
use super::model::{
    b, opt_s, page, s, u, with_extra, Author, Comment, Media, MediaKind, Platform, Post, Profile,
    Stats,
};
use super::page as web;

const P: Platform = Platform::PinBoard;
const ORIGIN: &str = "https://www.pinterest.com";
const FOLLOWER_FIELDS: &str = "user.{id,username,full_name,about,image_medium_url,image_large_url,follower_count,pin_count,board_count,is_private_profile,verified_identity,explicitly_followed_by_me}";
const RESERVED: &[&str] = &[
    "pin", "search", "ideas", "resource", "settings", "today", "business", "login", "_",
];

fn friendly(e: String) -> String {
    if e.starts_with("UPSTREAM_404") {
        return "NOT_FOUND: Pin Boards has nothing at that address".into();
    }
    if e.starts_with("UPSTREAM_403") {
        return "REJECTED: Pin Boards refused the request (the session may need a new sign-in or lacks permission)".into();
    }
    e
}

pub fn request_data(options: Value, bookmark: Option<&str>) -> String {
    let mut options = options;
    if let Some(mark) = bookmark.filter(|m| !m.is_empty()) {
        options["bookmarks"] = json!([mark]);
    }
    json!({ "options": options, "context": {} }).to_string()
}

pub fn resource_url(name: &str, source: &str, options: Value, bookmark: Option<&str>) -> String {
    format!(
        "{ORIGIN}/resource/{name}Resource/get/?{}",
        web::query(&[
            ("source_url", source.to_string()),
            ("data", request_data(options, bookmark)),
        ])
    )
}

pub fn unwrap_response(v: &Value) -> Result<(Value, Option<String>), String> {
    let rr = &v["resource_response"];
    if !rr.is_object() {
        return Err("UPSTREAM_CHANGED: Pin Boards did not return a resource response".into());
    }
    let err = &rr["error"];
    if !err.is_null() || rr["status"].as_str().is_some_and(|x| x != "success") {
        let msg = opt_s(&err["message"])
            .or_else(|| opt_s(&rr["message"]))
            .unwrap_or_else(|| "request failed".into());
        let status = u(&err["http_status"]).or_else(|| u(&rr["http_status"]));
        return Err(match status {
            Some(401) => "NOT_LOGGED_IN: Pin Boards asked to sign in again".to_string(),
            Some(404) => "NOT_FOUND: Pin Boards has nothing at that address".to_string(),
            Some(429) => {
                "RATE_LIMITED: Pin Boards is limiting requests from this account; wait a few minutes"
                    .to_string()
            }
            _ => format!("REJECTED: Pin Boards refused the request: {msg}"),
        });
    }
    let bookmark = opt_s(&rr["bookmark"])
        .or_else(|| opt_s(&v["resource"]["options"]["bookmarks"][0]))
        .filter(|m| m != "-end-");
    Ok((rr["data"].clone(), bookmark))
}

async fn headers(post: bool) -> Result<serde_json::Map<String, Value>, String> {
    let browser = web::ready(P).await?;
    let csrf = web::cookie(&browser, P, "csrftoken")
        .await
        .unwrap_or_default();
    let mut h = web::headers(&[
        ("accept", "application/json, text/javascript, */*, q=0.01"),
        ("x-requested-with", "XMLHttpRequest"),
        ("x-csrftoken", &csrf),
        ("x-pinterest-appstate", "active"),
        ("x-pinterest-pws-handler", "www/[username].js"),
    ]);
    if post {
        h.insert(
            "content-type".into(),
            json!("application/x-www-form-urlencoded"),
        );
    }
    Ok(h)
}

async fn get(
    name: &str,
    source: &str,
    options: Value,
    bookmark: Option<&str>,
) -> Result<(Value, Option<String>), String> {
    let reply = web::fetch(
        P,
        "GET",
        &resource_url(name, source, options, bookmark).replacen(ORIGIN, &web::origin(P).await, 1),
        headers(false).await?,
        None,
    )
    .await
    .map_err(friendly)?;
    unwrap_response(&reply.json()?)
}

async fn change(name: &str, op: &str, source: &str, options: Value) -> Result<Value, String> {
    let data = request_data(options, None);
    let reply = web::fetch(
        P,
        "POST",
        &format!("{}/resource/{name}Resource/{op}/", web::origin(P).await),
        headers(true).await?,
        Some(web::form(&[("source_url", source), ("data", &data)])),
    )
    .await
    .map_err(friendly)?;
    Ok(unwrap_response(&reply.json()?)?.0)
}

fn rest(input: &str) -> &str {
    let t = input.trim();
    let t = match t.split_once("pinterest.com/") {
        Some((_, r)) => r,
        None => t,
    };
    t.split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/')
}

fn slug(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 120
        && !x.contains("..")
        && x.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '%'))
}

pub fn pin_id(input: &str) -> Option<String> {
    let t = input.trim();
    if !t.is_empty() && t.len() <= 24 && t.bytes().all(|c| c.is_ascii_digit()) {
        return Some(t.to_string());
    }
    let re = regex::Regex::new(r"/pin/(?:[^/?#]*--)?(\d{3,24})").expect("static regex");
    re.captures(t).map(|c| c[1].to_string())
}

pub fn user_of(input: &str) -> Option<String> {
    let name = rest(input.trim().trim_start_matches('@'))
        .split('/')
        .next()?;
    (slug(name) && !RESERVED.contains(&name)).then(|| name.to_string())
}

pub fn board_of(input: &str) -> Option<(String, String)> {
    let mut parts = rest(input).split('/');
    let user = parts.next()?;
    let board = parts.next()?;
    (slug(user) && slug(board) && !RESERVED.contains(&user) && !board.starts_with('_'))
        .then(|| (user.to_string(), board.to_string()))
}

fn author(v: &Value) -> Author {
    Author {
        id: s(&v["id"]),
        handle: s(&v["username"]),
        name: s(&v["full_name"]),
        avatar: opt_s(&v["image_medium_url"])
            .or_else(|| opt_s(&v["image_small_url"]))
            .or_else(|| opt_s(&v["image_large_url"])),
        verified: b(&v["verified_identity"]["verified"]) || b(&v["is_verified_merchant"]),
    }
}

fn video(p: &Value) -> Option<Media> {
    let lists = [
        &p["videos"]["video_list"],
        &p["story_pin_data"]["pages"][0]["blocks"][0]["video"]["video_list"],
    ];
    let list = lists.iter().find_map(|l| l.as_object())?;
    let mp4 = |v: &&Value| {
        v["url"]
            .as_str()
            .is_some_and(|x| x.split('?').next().unwrap_or(x).ends_with(".mp4"))
    };
    let best = list
        .get("V_720P")
        .filter(mp4)
        .or_else(|| {
            list.values()
                .filter(mp4)
                .max_by_key(|v| u(&v["width"]).unwrap_or(0))
        })
        .or_else(|| list.values().next())?;
    Some(Media {
        kind: MediaKind::Video,
        url: opt_s(&best["url"])?,
        thumb: opt_s(&best["thumbnail"]).or_else(|| opt_s(&p["images"]["orig"]["url"])),
        width: u(&best["width"]),
        height: u(&best["height"]),
    })
}

fn image(p: &Value) -> Option<Media> {
    let i = ["orig", "736x", "474x", "236x"]
        .iter()
        .map(|k| &p["images"][*k])
        .find(|i| i["url"].is_string())?;
    Some(Media {
        kind: MediaKind::Image,
        url: s(&i["url"]),
        thumb: opt_s(&p["images"]["236x"]["url"]),
        width: u(&i["width"]),
        height: u(&i["height"]),
    })
}

pub fn pin(p: &Value) -> Option<Value> {
    let id = opt_s(&p["id"]).filter(|i| i.bytes().all(|c| c.is_ascii_digit()))?;
    if p["type"].as_str().is_some_and(|t| t != "pin") {
        return None;
    }
    let owner = ["native_creator", "pinner", "origin_pinner"]
        .iter()
        .map(|k| &p[*k])
        .find(|x| x["username"].is_string())
        .map(author)
        .unwrap_or_default();
    let title = opt_s(&p["title"]).or_else(|| opt_s(&p["grid_title"]));
    let stats = &p["aggregated_pin_data"]["aggregated_stats"];
    let reactions: Option<u64> = p["reaction_counts"]
        .as_object()
        .map(|o| o.values().filter_map(u).sum());
    let board = &p["board"];
    Some(with_extra(
        &Post {
            url: format!("{ORIGIN}/pin/{id}/"),
            id,
            author: owner,
            text: opt_s(&p["description"])
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .or_else(|| title.clone())
                .unwrap_or_default(),
            created_at: date(&p["created_at"]),
            media: video(p).or_else(|| image(p)).into_iter().collect(),
            stats: Stats {
                likes: reactions.filter(|n| *n > 0),
                replies: u(&p["comment_count"])
                    .or_else(|| u(&p["aggregated_pin_data"]["comment_count"])),
                saves: u(&stats["saves"]).or_else(|| u(&p["repin_count"])),
                ..Default::default()
            },
            ..Default::default()
        },
        json!({
            "kind": "pin",
            "title": title,
            "link": opt_s(&p["link"]),
            "domain": opt_s(&p["domain"]),
            "color": opt_s(&p["dominant_color"]),
            "board": board["id"].is_string().then(|| json!({ "id": s(&board["id"]), "name": s(&board["name"]), "url": opt_s(&board["url"]).map(|x| format!("{ORIGIN}{x}")) })),
            "aggregated_pin_id": opt_s(&p["aggregated_pin_data"]["id"]),
        }),
    ))
}

fn date(v: &Value) -> Option<i64> {
    let raw = v.as_str()?;
    chrono::DateTime::parse_from_rfc2822(raw)
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(raw))
        .ok()
        .map(|d| d.timestamp())
}

pub fn user_profile(v: &Value) -> Option<Value> {
    let name = opt_s(&v["username"])?;
    let a = author(v);
    Some(with_extra(
        &Profile {
            id: a.id,
            url: format!("{ORIGIN}/{name}/"),
            handle: name,
            name: a.name,
            avatar: opt_s(&v["image_xlarge_url"])
                .or_else(|| opt_s(&v["image_large_url"]))
                .or(a.avatar),
            bio: s(&v["about"]),
            verified: a.verified,
            private: b(&v["is_private_profile"]),
            followers: u(&v["follower_count"]),
            following: u(&v["following_count"]),
            posts: u(&v["pin_count"]),
            followed_by_me: v["explicitly_followed_by_me"].as_bool(),
            ..Default::default()
        },
        json!({
            "kind": "user",
            "boards": u(&v["board_count"]),
            "website": opt_s(&v["website_url"]),
            "monthly_views": u(&v["profile_views"]),
        }),
    ))
}

pub fn board_profile(v: &Value) -> Option<Value> {
    let id = opt_s(&v["id"])?;
    let path = s(&v["url"]);
    let (user, name) = board_of(&path)?;
    Some(with_extra(
        &Profile {
            id,
            url: format!("{ORIGIN}/{user}/{name}/"),
            handle: format!("{user}/{name}"),
            name: s(&v["name"]),
            avatar: opt_s(&v["image_cover_url"])
                .or_else(|| opt_s(&v["image_thumbnail_url"]))
                .or_else(|| opt_s(&v["cover_images"]["400x300"]["url"])),
            bio: s(&v["description"]),
            private: opt_s(&v["privacy"]).is_some_and(|x| x != "public"),
            followers: u(&v["follower_count"]),
            posts: u(&v["pin_count"]),
            followed_by_me: v["followed_by_me"].as_bool(),
            ..Default::default()
        },
        json!({
            "kind": "board",
            "owner": opt_s(&v["owner"]["username"]).unwrap_or(user),
            "sections": u(&v["section_count"]),
        }),
    ))
}

pub fn comment(c: &Value) -> Option<Comment> {
    let id = opt_s(&c["id"])?;
    Some(Comment {
        id,
        author: author(if c["user"].is_object() {
            &c["user"]
        } else {
            &c["commenter"]
        }),
        text: s(&c["text"]),
        created_at: date(&c["created_at"]),
        likes: u(&c["reaction_counts"]["1"]).or_else(|| u(&c["helpful_count"])),
        replies: u(&c["comment_count"]),
    })
}

pub fn item(v: &Value) -> Option<Value> {
    match v["type"].as_str() {
        Some("board") => board_profile(v),
        Some("user") => user_profile(v),
        _ => pin(v),
    }
}

pub fn items(data: &Value) -> Result<Vec<Value>, String> {
    let list = if data["results"].is_array() {
        &data["results"]
    } else {
        data
    };
    list.as_array()
        .map(|a| a.iter().filter_map(item).collect())
        .ok_or_else(|| "UPSTREAM_CHANGED: Pin Boards did not return a list".into())
}

fn mark(a: &Value) -> Option<String> {
    opt_str(a, "cursor").filter(|c| c.len() <= 6000)
}

async fn feed(name: &str, source: &str, options: Value, a: &Value) -> Result<Value, String> {
    let (data, next) = get(name, source, options, mark(a).as_deref()).await?;
    Ok(page(items(&data)?, next))
}

async fn me_data() -> Result<Value, String> {
    let (data, _) = get("UserSettings", "/settings/", json!({}), None).await?;
    if !data["username"].is_string() {
        return Err("NOT_LOGGED_IN: Pin Boards did not return the signed-in account; sign in again in the Pin Boards screen".into());
    }
    Ok(data)
}

async fn user_data(name: &str) -> Result<Value, String> {
    let (data, _) = get(
        "User",
        &format!("/{name}/"),
        json!({ "username": name, "field_set_key": "profile" }),
        None,
    )
    .await?;
    if !data["username"].is_string() {
        return Err(format!("NOT_FOUND: {name} does not exist on Pin Boards"));
    }
    Ok(data)
}

async fn board_data(user: &str, name: &str) -> Result<Value, String> {
    let (data, _) = get(
        "Board",
        &format!("/{user}/{name}/"),
        json!({ "username": user, "slug": name, "field_set_key": "detailed" }),
        None,
    )
    .await?;
    if !data["id"].is_string() {
        return Err(format!(
            "NOT_FOUND: the board {user}/{name} does not exist on Pin Boards"
        ));
    }
    Ok(data)
}

async fn board_pins(user: &str, name: &str, id: &str, a: &Value) -> Result<Value, String> {
    let path = format!("/{user}/{name}/");
    feed(
        "BoardFeed",
        &path,
        json!({ "board_id": id, "board_url": path, "field_set_key": "react_grid_pin", "page_size": 25, "layout": "default" }),
        a,
    )
    .await
}

async fn who(a: &Value) -> Result<String, String> {
    match opt_str(a, "username") {
        Some(n) => user_of(&n).ok_or_else(|| {
            "INVALID_ARGUMENTS: username must be a Pin Boards username or profile link".into()
        }),
        None => Ok(s(&me_data().await?["username"])),
    }
}

pub fn follow_request(followers: bool, name: &str) -> (&'static str, String, Value) {
    if followers {
        return (
            "Api",
            format!("/{name}/_followers/"),
            json!({ "url": format!("/v3/users/{name}/followers/"), "data": { "page_size": 50, "fields": FOLLOWER_FIELDS } }),
        );
    }
    (
        "UserFollowing",
        format!("/{name}/_following/"),
        json!({ "username": name, "page_size": 50, "explicit_following": true }),
    )
}

fn need_pin(a: &Value) -> Result<String, String> {
    pin_id(&req_str(a, "post")?).ok_or_else(|| {
        "INVALID_ARGUMENTS: post must be a pin id or a pinterest.com/pin/... link".into()
    })
}

fn need_board(a: &Value, key: &str) -> Result<(String, String), String> {
    board_of(&req_str(a, key)?).ok_or_else(|| {
        format!("INVALID_ARGUMENTS: {key} must be a board as username/board-name or its link")
    })
}

async fn pin_comments(p: &Value) -> Result<Value, String> {
    let Some(aggregated) = opt_s(&p["extra"]["aggregated_pin_id"]) else {
        return Ok(page(Vec::<Comment>::new(), None));
    };
    let (data, _) = get(
        "UnifiedComments",
        &format!("/pin/{}/", s(&p["id"])),
        json!({ "aggregated_pin_id": aggregated, "comment_featured_ids": [], "page_size": 25, "redux_normalize_feed": true }),
        None,
    )
    .await?;
    let list: Vec<Comment> = data
        .as_array()
        .map(|a| a.iter().filter_map(comment).collect())
        .unwrap_or_default();
    Ok(page(list, None))
}

pub async fn call(tool: &str, a: &Value) -> Result<Value, String> {
    match tool {
        "pin_me" => user_profile(&me_data().await?)
            .ok_or_else(|| "UPSTREAM_CHANGED: Pin Boards did not return the account".into()),
        "pin_feed" => {
            feed(
                "UserHomefeed",
                "/",
                json!({ "field_set_key": "hf_grid", "in_nux": false, "in_news_hub": false, "static_feed": false }),
                a,
            )
            .await
        }
        "pin_search" => {
            let query = req_str(a, "query")?;
            let kind = opt_str(a, "kind").unwrap_or_else(|| "pins".into());
            if !["pins", "boards", "users"].contains(&kind.as_str()) {
                return Err("INVALID_ARGUMENTS: kind must be pins, boards or users".into());
            }
            let source = format!("/search/{kind}/?q={}&rs=typed", urlencoding::encode(&query));
            feed(
                "BaseSearch",
                &source,
                json!({ "query": query, "scope": kind, "rs": "typed", "redux_normalize_feed": true, "source_url": source }),
                a,
            )
            .await
        }
        "pin_pin" => {
            let id = need_pin(a)?;
            let (data, _) = get(
                "Pin",
                &format!("/pin/{id}/"),
                json!({ "id": id, "field_set_key": "detailed" }),
                None,
            )
            .await?;
            let post = pin(&data).ok_or("NOT_FOUND: that pin is not available")?;
            let mut out = json!({ "post": post });
            match pin_comments(&out["post"]).await {
                Ok(c) => out["comments"] = c,
                Err(e) => {
                    out["comments"] = page(Vec::<Comment>::new(), None);
                    out["comments_unavailable"] = json!(e);
                }
            }
            Ok(out)
        }
        "pin_user" => {
            let raw = req_str(a, "username")?;
            if let Some((user, name)) = board_of(&raw) {
                return board_profile(&board_data(&user, &name).await?)
                    .ok_or_else(|| "UPSTREAM_CHANGED: Pin Boards did not return the board".into());
            }
            user_profile(&user_data(&who(a).await?).await?)
                .ok_or_else(|| "UPSTREAM_CHANGED: Pin Boards did not return the profile".into())
        }
        "pin_user_posts" => {
            let raw = opt_str(a, "username").unwrap_or_default();
            if let Some((user, name)) = board_of(&raw) {
                let id = s(&board_data(&user, &name).await?["id"]);
                return board_pins(&user, &name, &id, a).await;
            }
            let name = who(a).await?;
            feed(
                "UserActivityPins",
                &format!("/{name}/_created/"),
                json!({ "username": name, "field_set_key": "grid_item", "exclude_add_pin_rep": true, "is_own_profile_pins": false }),
                a,
            )
            .await
        }
        "pin_boards" => {
            let name = who(a).await?;
            let (data, next) = get(
                "Boards",
                &format!("/{name}/_saved/"),
                json!({ "username": name, "field_set_key": "profile_grid_item", "page_size": 25, "privacy_filter": "all", "sort": "last_pinned_to", "include_archived": true }),
                mark(a).as_deref(),
            )
            .await?;
            let boards: Vec<Value> = data
                .as_array()
                .ok_or("UPSTREAM_CHANGED: Pin Boards did not return a list")?
                .iter()
                .filter_map(board_profile)
                .collect();
            Ok(page(boards, next))
        }
        "pin_board" => {
            let (user, name) = need_board(a, "board")?;
            let data = board_data(&user, &name).await?;
            let pins = board_pins(&user, &name, &s(&data["id"]), a).await?;
            Ok(json!({ "board": board_profile(&data), "pins": pins }))
        }
        "pin_followers" | "pin_following" => {
            let name = who(a).await?;
            let (resource, source, options) = follow_request(tool == "pin_followers", &name);
            let (data, next) = get(resource, &source, options, mark(a).as_deref()).await?;
            let users: Vec<Value> = data
                .as_array()
                .ok_or("UPSTREAM_CHANGED: Pin Boards did not return a list")?
                .iter()
                .filter_map(user_profile)
                .collect();
            Ok(page(users, next))
        }
        "pin_save" => {
            let id = need_pin(a)?;
            let source = format!("/pin/{id}/");
            if undo(a) {
                change("Pin", "delete", &source, json!({ "id": id })).await?;
                return Ok(json!({ "ok": true, "pin": id, "saved": false }));
            }
            let raw = req_str(a, "board")?;
            let board = if raw.bytes().all(|c| c.is_ascii_digit()) {
                raw
            } else {
                let (user, name) = need_board(a, "board")?;
                s(&board_data(&user, &name).await?["id"])
            };
            let v = change(
                "Repin",
                "create",
                &source,
                json!({ "pin_id": id, "board_id": board, "is_buyable_pin": false }),
            )
            .await?;
            Ok(json!({ "ok": true, "pin": id, "saved": true, "saved_pin_id": opt_s(&v["id"]), "board_id": board }))
        }
        "pin_follow" => {
            let name = user_of(&req_str(a, "username")?)
                .ok_or("INVALID_ARGUMENTS: username must be a Pin Boards username or profile link")?;
            let id = s(&user_data(&name).await?["id"]);
            let op = if undo(a) { "delete" } else { "create" };
            change("UserFollow", op, &format!("/{name}/"), json!({ "user_id": id })).await?;
            Ok(json!({ "ok": true, "user": name, "following": !undo(a) }))
        }
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_resource_urls_with_bookmarks() {
        let url = resource_url("Pin", "/pin/1/", json!({ "id": "1" }), None);
        assert_eq!(
            url,
            "https://www.pinterest.com/resource/PinResource/get/?source_url=%2Fpin%2F1%2F&data=%7B%22context%22%3A%7B%7D%2C%22options%22%3A%7B%22id%22%3A%221%22%7D%7D"
        );
        let data: Value =
            serde_json::from_str(&request_data(json!({ "username": "ana" }), Some("Y2Jx")))
                .unwrap();
        assert_eq!(data["options"]["bookmarks"][0], "Y2Jx");
        assert_eq!(data["options"]["username"], "ana");
        let plain: Value = serde_json::from_str(&request_data(json!({}), Some(""))).unwrap();
        assert!(plain["options"].get("bookmarks").is_none());
    }

    #[test]
    fn unwraps_responses_and_errors() {
        let (data, next) = unwrap_response(&json!({ "resource_response": { "status": "success", "code": 0, "data": [1], "bookmark": "abc" } })).unwrap();
        assert_eq!(data, json!([1]));
        assert_eq!(next.as_deref(), Some("abc"));
        let (_, end) = unwrap_response(&json!({ "resource_response": { "status": "success", "data": [], "bookmark": "-end-" } })).unwrap();
        assert_eq!(end, None);
        let (_, old) = unwrap_response(&json!({ "resource_response": { "status": "success", "data": [] }, "resource": { "options": { "bookmarks": ["zz"] } } })).unwrap();
        assert_eq!(old.as_deref(), Some("zz"));
        let e = |v: Value| unwrap_response(&v).unwrap_err();
        assert!(e(json!({ "resource_response": { "status": "failure", "error": { "http_status": 401, "message": "Authorization failed." } } })).starts_with("NOT_LOGGED_IN"));
        assert!(e(
            json!({ "resource_response": { "status": "failure", "error": { "http_status": 404 } } })
        )
        .starts_with("NOT_FOUND"));
        assert!(e(json!({ "resource_response": { "status": "failure", "error": { "http_status": 400, "message": "Invalid board" } } })).starts_with("REJECTED: Pin Boards refused the request: Invalid board"));
        assert!(e(json!({ "html": "<html>" })).starts_with("UPSTREAM_CHANGED"));
    }

    #[test]
    fn reads_pins_users_and_boards_from_links() {
        assert_eq!(pin_id("123456789").as_deref(), Some("123456789"));
        assert_eq!(
            pin_id("https://www.pinterest.com/pin/123456789/?x=1").as_deref(),
            Some("123456789")
        );
        assert_eq!(
            pin_id("https://br.pinterest.com/pin/some-title--987654/").as_deref(),
            Some("987654")
        );
        assert_eq!(pin_id("abc"), None);
        assert_eq!(user_of("@ana").as_deref(), Some("ana"));
        assert_eq!(
            user_of("https://www.pinterest.com/ana/_saved/").as_deref(),
            Some("ana")
        );
        assert_eq!(user_of("https://www.pinterest.com/pin/1/"), None);
        assert_eq!(user_of("a b"), None);
        assert_eq!(
            board_of("ana/home-ideas"),
            Some(("ana".into(), "home-ideas".into()))
        );
        assert_eq!(
            board_of("https://www.pinterest.com/ana/home-ideas/"),
            Some(("ana".into(), "home-ideas".into()))
        );
        assert_eq!(board_of("https://www.pinterest.com/ana/_saved/"), None);
        assert_eq!(board_of("pin/123"), None);
        assert_eq!(board_of("ana"), None);
        assert_eq!(board_of("ana/../x"), None);
    }

    #[test]
    fn parses_image_and_video_pins() {
        let p = pin(&json!({ "id": "111", "type": "pin", "grid_title": "Cozy desk", "description": " a desk ", "link": "https://blog.example/desk", "domain": "blog.example", "dominant_color": "#aabbcc", "created_at": "Tue, 04 Nov 2025 12:00:00 +0000",
            "images": { "236x": { "url": "s.jpg", "width": 236, "height": 300 }, "orig": { "url": "o.jpg", "width": 1000, "height": 1500 } },
            "pinner": { "id": "9", "username": "ana", "full_name": "Ana", "image_medium_url": "a.jpg" },
            "board": { "id": "55", "name": "Home", "url": "/ana/home/" }, "repin_count": 7, "comment_count": 2,
            "reaction_counts": { "1": 3, "5": 1 }, "aggregated_pin_data": { "id": "777", "aggregated_stats": { "saves": 40, "done": 0 } } })).unwrap();
        assert_eq!(p["url"], "https://www.pinterest.com/pin/111/");
        assert_eq!(p["text"], "a desk");
        assert_eq!(p["author"]["handle"], "ana");
        assert_eq!(p["media"][0]["url"], "o.jpg");
        assert_eq!(p["media"][0]["thumb"], "s.jpg");
        assert_eq!(p["stats"]["saves"], 40);
        assert_eq!(p["stats"]["likes"], 4);
        assert_eq!(p["created_at"], 1762257600);
        assert_eq!(p["extra"]["title"], "Cozy desk");
        assert_eq!(
            p["extra"]["board"]["url"],
            "https://www.pinterest.com/ana/home/"
        );
        assert_eq!(p["extra"]["aggregated_pin_id"], "777");
        let v = pin(&json!({ "id": "222", "images": { "orig": { "url": "o.jpg" } }, "videos": { "video_list": { "V_HLSV4": { "url": "x.m3u8", "width": 1080 }, "V_720P": { "url": "v.mp4?x=1", "width": 720, "height": 1280, "thumbnail": "t.jpg" } } } })).unwrap();
        assert_eq!(v["media"][0]["kind"], "video");
        assert_eq!(v["media"][0]["url"], "v.mp4?x=1");
        assert_eq!(v["text"], "");
        assert!(pin(&json!({ "id": "333", "type": "story" })).is_none());
        assert!(pin(&json!({ "id": "not-a-pin" })).is_none());
    }

    #[test]
    fn parses_users_boards_comments_and_mixed_results() {
        let user = json!({ "type": "user", "id": "9", "username": "ana", "full_name": "Ana", "about": "hi", "image_xlarge_url": "x.jpg", "follower_count": 10, "following_count": 4, "pin_count": 90, "board_count": 3, "explicitly_followed_by_me": true, "verified_identity": { "verified": true } });
        let board = json!({ "type": "board", "id": "55", "name": "Home ideas", "url": "/ana/home-ideas/", "description": "d", "pin_count": 12, "follower_count": 2, "privacy": "secret", "image_cover_url": "c.jpg", "owner": { "username": "ana" } });
        let list = items(&json!({ "results": [user, board, { "type": "pin", "id": "1" }, { "type": "module" }] })).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["handle"], "ana");
        assert_eq!(list[0]["posts"], 90);
        assert_eq!(list[0]["verified"], true);
        assert_eq!(list[0]["followed_by_me"], true);
        assert_eq!(list[0]["extra"]["boards"], 3);
        assert_eq!(list[1]["handle"], "ana/home-ideas");
        assert_eq!(list[1]["private"], true);
        assert_eq!(list[1]["extra"]["kind"], "board");
        assert_eq!(list[2]["id"], "1");
        assert_eq!(items(&json!([{ "id": "5" }])).unwrap().len(), 1);
        assert!(items(&json!({ "nope": 1 }))
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
        let c = comment(&json!({ "id": "c1", "text": "love it", "user": { "username": "bia" }, "created_at": "Tue, 04 Nov 2025 12:00:00 +0000", "reaction_counts": { "1": 2 }, "comment_count": 1 })).unwrap();
        assert_eq!(c.author.handle, "bia");
        assert_eq!(c.likes, Some(2));
        assert_eq!(c.replies, Some(1));
        assert!(comment(&json!({})).is_none());
    }

    #[test]
    fn followers_come_from_the_generic_api_resource() {
        let (resource, source, options) = follow_request(true, "ana");
        assert_eq!(resource, "Api");
        assert_eq!(source, "/ana/_followers/");
        assert_eq!(options["url"], "/v3/users/ana/followers/");
        assert_eq!(options["data"]["page_size"], 50);
        assert!(options["data"]["fields"]
            .as_str()
            .unwrap()
            .starts_with("user.{id,username,"));
        let (resource, source, options) = follow_request(false, "ana");
        assert_eq!(resource, "UserFollowing");
        assert_eq!(source, "/ana/_following/");
        assert_eq!(options["username"], "ana");
        assert_eq!(options["explicit_following"], true);
        let reply = json!({ "resource_response": { "status": "success", "code": 0, "bookmark": "Pz9Nakl5", "data": [
            { "node_id": "VXNlcjox", "follower_count": 12, "image_medium_url": "https://i.pinimg.com/75x75_RS/aa/bb/cc.jpg", "id": "1000000000000000665", "full_name": "Sample Person", "pin_count": 340, "verified_identity": {}, "is_private_profile": false, "username": "sampleperson", "explicitly_followed_by_me": false },
            { "node_id": "VXNlcjoy", "follower_count": 0, "image_medium_url": "https://i.pinimg.com/75x75_RS/dd/ee/ff.jpg", "id": "1000000000000000569", "full_name": "", "pin_count": 0, "verified_identity": { "verified": true }, "is_private_profile": true, "username": "another_one", "explicitly_followed_by_me": true } ] },
            "resource": { "name": "ApiResource", "options": { "bookmarks": ["Pz9Nakl5"] } } });
        let (data, next) = unwrap_response(&reply).unwrap();
        assert_eq!(next.as_deref(), Some("Pz9Nakl5"));
        let users: Vec<Value> = data
            .as_array()
            .unwrap()
            .iter()
            .filter_map(user_profile)
            .collect();
        assert_eq!(users.len(), 2);
        assert_eq!(users[0]["handle"], "sampleperson");
        assert_eq!(users[0]["followers"], 12);
        assert_eq!(users[0]["posts"], 340);
        assert_eq!(users[0]["followed_by_me"], false);
        assert_eq!(users[1]["private"], true);
        assert_eq!(users[1]["verified"], true);
    }

    #[test]
    fn friendly_maps_http_statuses() {
        assert!(friendly("UPSTREAM_404: x".into()).starts_with("NOT_FOUND"));
        assert!(friendly("UPSTREAM_403: x".into()).starts_with("REJECTED"));
    }
}
