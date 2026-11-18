use serde_json::{json, Map, Value};

use super::args::{opt_str, req_str, undo};
use super::model::{
    b, opt_s, page, s, u, Author, Comment, Media, MediaKind, Platform, Post, Profile, Stats,
};
use super::page as web;

const P: Platform = Platform::PhotoFeed;
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
            return Err("NOT_LOGGED_IN: Photo Feed asked to sign in again".into());
        }
        return Err(format!("REJECTED: Photo Feed refused the request: {msg}"));
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
        avatar: opt_s(&user["profile_pic_url_hd"])
            .or_else(|| opt_s(&user["hd_profile_pic_url_info"]["url"]))
            .or(a.avatar),
        bio: s(&user["biography"]),
        verified: a.verified,
        private: b(&user["is_private"]),
        followers: count("edge_followed_by", "follower_count"),
        following: count("edge_follow", "following_count"),
        posts: count("edge_owner_to_timeline_media", "media_count"),
        likes: None,
        followed_by_me: user["followed_by_viewer"]
            .as_bool()
            .or_else(|| user["friendship_status"]["following"].as_bool()),
        follows_me: user["follows_viewer"]
            .as_bool()
            .or_else(|| user["friendship_status"]["followed_by"].as_bool()),
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

const HASHTAG_QUERY_ID: &str = "37324993597144881";

const GQL: &str = r#"(async(name,vars,fallback)=>{
const r=window.require;
const mods=[name+'$Parameters',name+'_instagramRelayOperation'];
const has=m=>{try{return !!r(m)}catch(e){return false}};
if(!mods.some(has)){try{await new Promise((ok,no)=>{r('Bootloader').loadModules(mods,()=>ok(),'omniget');setTimeout(()=>no(),10000)})}catch(e){}}
let id=null,pv={};
try{const p=r(name+'$Parameters').params;id=p.id;for(const [k,v] of Object.entries(p.providedVariables||{})){try{pv[k]=v.get()}catch(e){}}}catch(e){}
if(!id){try{id=r(name+'_instagramRelayOperation')}catch(e){}}
if(!id)id=fallback;
if(!id)return {missing:name};
const dtsg=r('DTSGInitialData').token,lsd=r('LSD').token;
const csrf=(document.cookie.match(/csrftoken=([^;]+)/)||[])[1]||'';
const body=new URLSearchParams({fb_dtsg:dtsg,lsd,doc_id:id,variables:JSON.stringify({...vars,...pv}),fb_api_req_friendly_name:name,fb_api_caller_class:'RelayModern',server_timestamps:'true'});
const res=await fetch('/graphql/query',{method:'POST',credentials:'include',headers:{'content-type':'application/x-www-form-urlencoded','x-fb-lsd':lsd,'x-ig-app-id':'936619743392459','x-csrftoken':csrf,'x-fb-friendly-name':name},body});
const t=await res.text();
try{return {status:res.status,json:JSON.parse(t)}}catch(e){return {status:res.status,text:t.slice(0,200)}}
})"#;

async fn gql(name: &str, vars: Value) -> Result<Value, String> {
    gql_or(name, vars, None).await
}

async fn gql_or(name: &str, vars: Value, fallback: Option<&str>) -> Result<Value, String> {
    let b = web::ready(P).await?;
    let expr = format!(
        "{GQL}({},{},{})",
        Value::String(name.into()),
        vars,
        json!(fallback)
    );
    let r = web::eval(&b, P, &expr).await?;
    if r["missing"].is_string() {
        return Err(format!(
            "UPSTREAM_CHANGED: could not find the Photo Feed query {name} in the web app; reload instagram.com in the OmniGet browser and try again"
        ));
    }
    match r["status"].as_u64().unwrap_or(0) {
        200 => {}
        401 | 403 => return Err("NOT_LOGGED_IN: Photo Feed asked to sign in again".into()),
        429 => return Err(
            "RATE_LIMITED: Photo Feed is limiting requests from this account; wait a few minutes"
                .into(),
        ),
        st => {
            return Err(format!(
                "UPSTREAM_{st}: Photo Feed answered {st}: {}",
                s(&r["text"])
            ))
        }
    }
    let v = r["json"].clone();
    if v.is_null() {
        return Err(format!(
            "UPSTREAM_CHANGED: Photo Feed answered {name} with something that is not JSON"
        ));
    }
    if v["data"].is_null() {
        let msg = v["errors"][0]["message"]
            .as_str()
            .unwrap_or("no data")
            .to_string();
        return Err(format!("REJECTED: Photo Feed refused the request: {msg}"));
    }
    Ok(v)
}

async fn user_id(username: &str) -> Result<String, String> {
    let v = gql(
        "PolarisSearchBoxRefetchableQuery",
        json!({ "data": { "context": "blended", "include_reel": "true", "query": username, "rank_token": "", "search_surface": "web_top_search" }, "hasQuery": true }),
    )
    .await?;
    let hit = v["data"]["xdt_api__v1__fbsearch__topsearch_connection"]["users"]
        .as_array()
        .and_then(|list| {
            list.iter()
                .map(|i| &i["user"])
                .find(|u_| s(&u_["username"]).eq_ignore_ascii_case(username))
                .and_then(|u_| opt_s(&u_["pk"]).or_else(|| opt_s(&u_["id"])))
        });
    if let Some(id) = hit {
        return Ok(id);
    }
    let (items, _) = user_timeline(username, None).await?;
    items
        .first()
        .map(|p| p.author.id.clone())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| format!("NOT_FOUND: @{username} does not exist on Photo Feed"))
}

async fn user(username: &str) -> Result<(Profile, Value), String> {
    let id = user_id(username).await?;
    let v = gql(
        "PolarisProfilePageContentQuery",
        json!({ "enable_integrity_filters": true, "id": id }),
    )
    .await?;
    let raw = v["data"]["user"].clone();
    let p = profile(&raw)
        .ok_or_else(|| format!("NOT_FOUND: @{username} does not exist on Photo Feed"))?;
    Ok((p, raw))
}

async fn user_timeline(
    username: &str,
    cursor: Option<String>,
) -> Result<(Vec<Post>, Option<String>), String> {
    let mut vars = json!({ "data": { "count": 12, "include_reel_media_seen_timestamp": true, "include_relationship_info": true, "latest_besties_reel_media": true, "latest_reel_media": true }, "username": username });
    if let Some(c) = cursor {
        vars["after"] = json!(c);
        vars["first"] = json!(12);
    }
    let v = gql("PolarisProfilePostsQuery", vars).await?;
    let conn = &v["data"]["xdt_api__v1__feed__user_timeline_graphql_connection"];
    let items = conn["edges"]
        .as_array()
        .map(|x| x.iter().filter_map(|e| media_post(&e["node"])).collect())
        .unwrap_or_default();
    let next = if conn["page_info"]["has_next_page"].as_bool() == Some(true) {
        opt_s(&conn["page_info"]["end_cursor"])
    } else {
        None
    };
    Ok((items, next))
}

pub fn follow_list(v: &Value) -> Option<(Vec<Profile>, Option<String>)> {
    let items = v["users"]
        .as_array()?
        .iter()
        .filter_map(|u_| {
            let mut p = profile(u_)?;
            if b(&u_["has_anonymous_profile_picture"]) {
                p.avatar = None;
            }
            Some(p)
        })
        .collect();
    Some((items, opt_s(&v["next_max_id"])))
}

fn follow_path(id: &str, kind: &str, cursor: Option<&str>, query: Option<&str>) -> String {
    let mut path = format!("/friendships/{id}/{kind}/?count=50");
    if kind == "followers" {
        path.push_str("&search_surface=follow_list_page");
    }
    if let Some(q) = query {
        path.push_str(&format!("&query={}", urlencoding::encode(q)));
    }
    if let Some(c) = cursor {
        path.push_str(&format!("&max_id={}", urlencoding::encode(c)));
    }
    path
}

fn soft_block(e: String) -> String {
    let page = e.starts_with("UPSTREAM_CHANGED") && e.contains("not JSON");
    if !page && !e.starts_with("RATE_LIMITED") {
        return e;
    }
    format!(
        "RATE_LIMITED: {}",
        json!({
            "message": if page {
                "Photo Feed answered the list request with a page instead of data, which is how it blocks an account that asked for lists too fast"
            } else {
                "Photo Feed is limiting list requests from this account (429)"
            },
            "retryable": true,
            "next_step": "Stop reading lists on this account and wait at least an hour before the next sync; do not retry in a loop.",
        })
    )
}

async fn follow_page(kind: &str, a: &Value) -> Result<Value, String> {
    let (id, cursor) = match opt_str(a, "cursor") {
        Some(c) => {
            let (id, rest) = c
                .split_once('~')
                .ok_or("INVALID_ARGUMENTS: cursor must be the next_cursor of the previous page")?;
            (id.to_string(), Some(rest.to_string()))
        }
        None => match opt_str(a, "username") {
            Some(name) => (user_id(&username_of(&name)).await?, None),
            None => {
                let b = web::ready(P).await?;
                let id = web::cookie(&b, P, "ds_user_id")
                    .await
                    .ok_or("NOT_LOGGED_IN: Photo Feed did not return the signed-in account")?;
                (id, None)
            }
        },
    };
    if !id.bytes().all(|c| c.is_ascii_digit()) {
        return Err(
            "INVALID_ARGUMENTS: cursor must be the next_cursor of the previous page".into(),
        );
    }
    let path = follow_path(&id, kind, cursor.as_deref(), opt_str(a, "query").as_deref());
    let v = get(&path).await.map_err(soft_block)?;
    let (items, next) = follow_list(&v).ok_or_else(|| {
        format!("UPSTREAM_CHANGED: Photo Feed answered the {kind} list without users")
    })?;
    Ok(page(items, next.map(|c| format!("{id}~{c}"))))
}

fn need_pk(a: &Value) -> Result<String, String> {
    media_pk(&req_str(a, "post")?).ok_or_else(|| {
        "INVALID_ARGUMENTS: post must be a Photo Feed link, shortcode or media id".into()
    })
}

fn cursor_q(a: &Value, key: &str) -> String {
    opt_str(a, "cursor")
        .map(|c| format!("&{key}={}", urlencoding::encode(&c)))
        .unwrap_or_default()
}

pub async fn me() -> Result<Value, String> {
    let b = web::ready(P).await?;
    let id = web::cookie(&b, P, "ds_user_id")
        .await
        .ok_or("NOT_LOGGED_IN: Photo Feed did not return the signed-in account")?;
    let v = gql(
        "PolarisProfilePageContentQuery",
        json!({ "enable_integrity_filters": true, "id": id }),
    )
    .await?;
    profile(&v["data"]["user"])
        .map(|p| json!(p))
        .ok_or_else(|| "NOT_LOGGED_IN: Photo Feed did not return the signed-in account".into())
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

pub fn hashtag_media(v: &Value) -> Option<Vec<Post>> {
    let data = v["data"].as_object()?;
    let mut out = Vec::new();
    if let Some(edges) = data
        .get("xdt_fbsearch__top_serp_graphql")
        .and_then(|g| g["edges"].as_array())
    {
        for edge in edges {
            collect_media(&edge["node"]["items"], 0, &mut out);
        }
    }
    for tab in ["recent", "top"] {
        if let Some(sections) = data.get(tab).and_then(|t| t["sections"].as_array()) {
            for section in sections {
                collect_media(&section["layout_content"], 0, &mut out);
            }
        }
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    Some(out)
}

pub async fn hashtag_posts(tag: &str) -> Result<Vec<Post>, String> {
    let session = uuid::Uuid::new_v4().to_string();
    let v = gql_or(
        "PolarisKeywordSearchExplorePageRelayQuery",
        json!({ "query": format!("#{tag}"), "search_session_id": session, "serp_session_id": session }),
        Some(HASHTAG_QUERY_ID),
    )
    .await?;
    hashtag_media(&v).ok_or_else(|| {
        "UPSTREAM_CHANGED: Photo Feed answered the hashtag page without data".to_string()
    })
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
            let (items, next) =
                user_timeline(&username_of(&req_str(a, "username")?), opt_str(a, "cursor")).await?;
            Ok(page::<Post>(items, next))
        }
        "ig_post" => {
            let pk = need_pk(a)?;
            let v = get(&format!("/media/{pk}/info/")).await?;
            let post = v["items"]
                .as_array()
                .and_then(|x| x.first())
                .and_then(media_post);
            if post.is_none() {
                return Err("NOT_FOUND: that Photo Feed post is not available".into());
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
            let v = gql(
                "PolarisSearchBoxRefetchableQuery",
                json!({ "data": { "context": "blended", "include_reel": "true", "query": req_str(a, "query")?, "rank_token": "", "search_surface": "web_top_search" }, "hasQuery": true }),
            )
            .await?;
            let v = &v["data"]["xdt_api__v1__fbsearch__topsearch_connection"];
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
        "ig_followers" => follow_page("followers", a).await,
        "ig_following" => follow_page("following", a).await,
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
    fn a_list_request_can_search_by_name_and_a_block_is_a_rate_limit() {
        assert_eq!(
            follow_path("9", "following", None, None),
            "/friendships/9/following/?count=50"
        );
        assert_eq!(
            follow_path("9", "following", Some("QVF=="), Some("a")),
            "/friendships/9/following/?count=50&query=a&max_id=QVF%3D%3D"
        );
        assert_eq!(
            follow_path("9", "followers", Some("50"), None),
            "/friendships/9/followers/?count=50&search_surface=follow_list_page&max_id=50"
        );
        let html = soft_block(
            "UPSTREAM_CHANGED: the site answered 200 with something that is not JSON: <!DOCTYPE html>".into(),
        );
        let (code, rest) = html.split_once(": ").unwrap();
        assert_eq!(code, "RATE_LIMITED");
        let v: Value = serde_json::from_str(rest).unwrap();
        assert_eq!(v["retryable"], true);
        assert!(v["next_step"]
            .as_str()
            .unwrap()
            .contains("at least an hour"));
        assert!(
            soft_block("RATE_LIMITED: Photo Feed is limiting requests".into())
                .starts_with("RATE_LIMITED: {")
        );
        assert_eq!(soft_block("NOT_LOGGED_IN: x".into()), "NOT_LOGGED_IN: x");
        assert_eq!(
            soft_block(
                "UPSTREAM_CHANGED: Photo Feed answered the following list without users".into()
            ),
            "UPSTREAM_CHANGED: Photo Feed answered the following list without users"
        );
    }

    #[test]
    fn reads_a_followers_page() {
        let v = json!({ "status": "ok", "next_max_id": "50", "users": [
            { "pk": "1", "username": "ana", "full_name": "Ana", "is_private": true, "is_verified": true, "profile_pic_url": "a.jpg" },
            { "pk_id": "2", "username": "bia", "profile_pic_url": "default.jpg", "has_anonymous_profile_picture": true },
            { "username": "no_id" } ] });
        let (items, next) = follow_list(&v).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].handle, "ana");
        assert!(items[0].private && items[0].verified);
        assert_eq!(items[0].followers, None);
        assert_eq!(items[1].id, "2");
        assert_eq!(items[1].avatar, None);
        assert_eq!(next.as_deref(), Some("50"));
        let (last, end) = follow_list(&json!({ "users": [], "status": "ok" })).unwrap();
        assert!(last.is_empty());
        assert_eq!(end, None);
        assert!(follow_list(&json!({ "status": "ok" })).is_none());
    }

    #[test]
    fn reads_the_posts_of_a_hashtag_page() {
        let media = |pk: &str, at: i64| json!({ "media": { "pk": pk, "code": format!("C{pk}"), "taken_at": at, "image_versions2": { "candidates": [{ "url": "i.jpg" }] }, "user": { "pk": "1", "username": "ana" } } });
        let v = json!({ "data": { "name": "tauri",
            "top": { "sections": [{ "layout_content": { "medias": [media("1", 10), media("2", 30)] } }] },
            "recent": { "sections": [
                { "layout_content": { "medias": [media("2", 30)] } },
                { "layout_content": { "one_by_two_item": { "clips": { "items": [media("3", 20)] } } } } ] } } });
        let posts = hashtag_media(&v).unwrap();
        assert_eq!(
            posts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["2", "3", "1"]
        );
        assert_eq!(hashtag_media(&json!({ "status": "ok" })), None);
        assert_eq!(hashtag_media(&json!({ "data": {} })), Some(Vec::new()));
    }

    #[test]
    fn reads_the_posts_of_a_hashtag_search_page() {
        let v = json!({ "data": { "xdt_viewer": { "user": { "id": "1" } }, "xdt_fbsearch__top_serp_graphql": {
            "edges": [
                { "node": { "__typename": "XDTTopSerpMediaGridUnit", "items": [
                    { "__typename": "XDTMediaDict", "pk": "3500000000000000001", "id": "3500000000000000001_400", "code": "DEXsample1", "media_type": 2, "taken_at": 1793000000,
                      "user": { "pk": "400", "username": "sample.user", "id": "400", "full_name": "Sample", "profile_pic_url": "https://cdn.example/p.jpg", "is_verified": false, "is_private": false },
                      "image_versions2": { "candidates": [{ "height": 1920, "width": 1080, "url": "https://cdn.example/i.jpg" }] },
                      "caption": { "text": "a trip #travel" }, "video_versions": [{ "url": "https://cdn.example/v.mp4", "type": 101, "height": 1920, "width": 1080 }],
                      "carousel_media_count": null, "carousel_media": null, "like_count": 12, "comment_count": 3, "view_count": null },
                    { "__typename": "XDTMediaDict", "pk": "3500000000000000002", "id": "3500000000000000002_401", "code": "DEXsample2", "media_type": 1, "taken_at": 1793000500,
                      "user": { "pk": "401", "username": "other.user" }, "image_versions2": { "candidates": [{ "url": "https://cdn.example/j.jpg" }] }, "caption": null, "like_count": 1, "comment_count": 0 } ] } },
                { "node": { "__typename": "XDTTopSerpMediaGridUnit", "items": [
                    { "pk": "3500000000000000001", "code": "DEXsample1", "image_versions2": { "candidates": [{ "url": "https://cdn.example/i.jpg" }] }, "user": { "pk": "400", "username": "sample.user" } } ] } } ],
            "page_info": { "has_next_page": true, "end_cursor": "abc" } } } });
        let posts = hashtag_media(&v).unwrap();
        assert_eq!(
            posts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["3500000000000000002", "3500000000000000001"]
        );
        assert_eq!(posts[1].author.handle, "sample.user");
        assert_eq!(posts[1].text, "a trip #travel");
        assert_eq!(posts[1].stats.likes, Some(12));
        assert_eq!(posts[1].media[0].kind, MediaKind::Video);
        assert_eq!(posts[1].url, "https://www.instagram.com/p/DEXsample1/");
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
