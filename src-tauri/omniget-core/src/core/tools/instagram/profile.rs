//! Perfil, posts, reels, marcações, salvos, stories e highlights. Os
//! endpoints e os cabeçalhos foram verificados numa sessão real em
//! 2026-09-05 (ver mod.rs); `feed/user/{id}/` morreu para a web, então os
//! posts vêm do GraphQL `PolarisProfilePostsQuery` + `…TabContentQuery_connection`.

use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::media::{parse_item, MediaItem};
use super::{b, s, u, IgClient, IgError};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserInfo {
    pub pk: String,
    pub username: String,
    pub full_name: String,
    pub biography: String,
    pub external_url: String,
    pub follower_count: u64,
    pub following_count: u64,
    pub media_count: u64,
    pub total_clips: u64,
    pub is_private: bool,
    pub is_verified: bool,
    pub is_business: bool,
    pub category: String,
    pub profile_pic_url: String,
    pub profile_pic_hd: String,
    pub followed_by_viewer: bool,
    pub follows_viewer: bool,
    pub has_highlights: bool,
    pub is_self: bool,
}

fn parse_user_info(v: &Value) -> UserInfo {
    let hd = v
        .get("hd_profile_pic_url_info")
        .and_then(|h| h.get("url"))
        .and_then(|x| x.as_str())
        .map(|x| x.to_string())
        .or_else(|| {
            v.get("hd_profile_pic_versions")
                .and_then(|a| a.as_array())
                .and_then(|a| a.last())
                .and_then(|x| x.get("url"))
                .and_then(|x| x.as_str())
                .map(|x| x.to_string())
        })
        .or_else(|| {
            v.get("profile_pic_url_hd")
                .and_then(|x| x.as_str())
                .map(|x| x.to_string())
        })
        .unwrap_or_else(|| s(v, "profile_pic_url"));
    let count = |k: &str, edge: &str| {
        let n = u(v, k);
        if n > 0 {
            n
        } else {
            v.get(edge)
                .and_then(|e| e.get("count"))
                .and_then(|c| c.as_u64())
                .unwrap_or(0)
        }
    };
    let pk = {
        let p = s(v, "pk");
        if p.is_empty() {
            s(v, "id")
        } else {
            p
        }
    };
    UserInfo {
        username: s(v, "username"),
        full_name: s(v, "full_name"),
        biography: s(v, "biography"),
        external_url: s(v, "external_url"),
        follower_count: count("follower_count", "edge_followed_by"),
        following_count: count("following_count", "edge_follow"),
        media_count: count("media_count", "edge_owner_to_timeline_media"),
        total_clips: u(v, "total_clips_count"),
        is_private: b(v, "is_private"),
        is_verified: b(v, "is_verified"),
        is_business: b(v, "is_business") || b(v, "is_business_account"),
        category: {
            let c = s(v, "category");
            if c.is_empty() {
                s(v, "category_name")
            } else {
                c
            }
        },
        profile_pic_url: s(v, "profile_pic_url"),
        profile_pic_hd: hd,
        followed_by_viewer: b(v, "followed_by_viewer")
            || v.get("friendship_status")
                .map(|f| b(f, "following"))
                .unwrap_or(false),
        follows_viewer: b(v, "follows_viewer")
            || v.get("friendship_status")
                .map(|f| b(f, "followed_by"))
                .unwrap_or(false),
        has_highlights: b(v, "has_highlight_reels") || b(v, "highlight_reel_count"),
        is_self: false,
        pk,
    }
}

/// `GET /api/v1/users/{id}/info/`
pub async fn user_by_id(client: &IgClient, id: &str) -> Result<UserInfo, IgError> {
    let json = client
        .get_json(&format!("/api/v1/users/{}/info/", id), &[])
        .await?;
    let user = json
        .get("user")
        .ok_or_else(|| IgError::NotFound(format!("usuario {}", id)))?;
    let mut info = parse_user_info(user);
    info.is_self = info.pk == client.session.user_id;
    Ok(info)
}

/// Nome → id: `web_profile_info` primeiro (traz follows_viewer); se ele
/// estiver limitado (429 é comum), `topsearch` e depois `users/{id}/info`.
pub async fn resolve_user(client: &IgClient, name_or_id: &str) -> Result<UserInfo, IgError> {
    let name = name_or_id.trim().trim_start_matches('@').to_lowercase();
    if name.is_empty() {
        return Err(IgError::NotFound("usuario vazio".into()));
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return user_by_id(client, &name).await;
    }
    if let Ok(json) = client
        .get_json(
            "/api/v1/users/web_profile_info/",
            &[("username", name.clone())],
        )
        .await
    {
        if let Some(user) = json
            .get("data")
            .and_then(|d| d.get("user"))
            .filter(|u| !u.is_null())
        {
            let mut info = parse_user_info(user);
            if info.profile_pic_hd.is_empty() || info.total_clips == 0 {
                if let Ok(full) = user_by_id(client, &info.pk).await {
                    info.profile_pic_hd = full.profile_pic_hd;
                    info.total_clips = full.total_clips;
                    info.category = full.category;
                }
            }
            info.is_self = info.pk == client.session.user_id;
            return Ok(info);
        }
    }
    let json = client
        .get_json(
            "/api/v1/web/search/topsearch/",
            &[("query", name.clone()), ("context", "blended".into())],
        )
        .await?;
    let users = json
        .get("users")
        .and_then(|u| u.as_array())
        .cloned()
        .unwrap_or_default();
    let hit = users
        .iter()
        .filter_map(|r| r.get("user"))
        .find(|u| s(u, "username").to_lowercase() == name)
        .ok_or_else(|| IgError::NotFound(format!("@{}", name)))?;
    let pk = s(hit, "pk");
    let mut info = user_by_id(client, &pk).await?;
    if let Some(fs) = hit.get("friendship_status") {
        info.followed_by_viewer = b(fs, "following");
        info.follows_viewer = b(fs, "followed_by");
    }
    Ok(info)
}

/// Quem está logado (pelo `ds_user_id` dos cookies).
pub async fn whoami(client: &IgClient) -> Result<UserInfo, IgError> {
    let id = client.session.user_id.clone();
    let mut me = user_by_id(client, &id).await?;
    me.is_self = true;
    Ok(me)
}

fn page_pause_check(flag: &AtomicBool) -> bool {
    !super::cancelled(flag)
}

/// Posts do perfil (fotos, vídeos e carrosséis) via GraphQL, mais novos primeiro.
pub async fn user_posts(
    client: &IgClient,
    username: &str,
    limit: usize,
    flag: &AtomicBool,
    progress: &super::super::ProgressFn,
    job: &str,
) -> Result<Vec<MediaItem>, IgError> {
    let data = json!({"count": 12, "include_reel_media_seen_timestamp": true, "include_relationship_info": true, "latest_besties_reel_media": true, "latest_reel_media": true});
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    let mut first = true;
    let id = format!("ig:{}", job);
    loop {
        let (name, vars) = if first {
            (
                "PolarisProfilePostsQuery",
                json!({"data": data, "username": username}),
            )
        } else {
            (
                "PolarisProfilePostsTabContentQuery_connection",
                json!({"after": after, "before": null, "data": data, "first": 12, "last": null, "username": username}),
            )
        };
        let json = client.graphql(name, vars).await?;
        let conn = json
            .get("data")
            .and_then(|d| d.get("xdt_api__v1__feed__user_timeline_graphql_connection"))
            .cloned()
            .unwrap_or(Value::Null);
        if conn.is_null() {
            if first {
                if let Some(user) = json.get("data").and_then(|d| d.get("xdt_viewer")) {
                    let _ = user;
                }
                return Err(IgError::Private);
            }
            break;
        }
        let edges = conn
            .get("edges")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        for e in &edges {
            if let Some(item) = e.get("node").and_then(parse_item) {
                out.push(item);
            }
        }
        super::super::report(
            progress,
            &id,
            "list",
            out.len() as u64,
            if limit > 0 { Some(limit as u64) } else { None },
            None,
        );
        let has_next = conn
            .get("page_info")
            .map(|p| b(p, "has_next_page"))
            .unwrap_or(false);
        after = conn
            .get("page_info")
            .and_then(|p| p.get("end_cursor"))
            .and_then(|c| c.as_str())
            .map(|c| c.to_string());
        first = false;
        if edges.is_empty()
            || !has_next
            || after.is_none()
            || (limit > 0 && out.len() >= limit)
            || !page_pause_check(flag)
        {
            break;
        }
        client.pause().await;
    }
    if limit > 0 {
        out.truncate(limit);
    }
    Ok(out)
}

/// Reels do perfil (`POST /api/v1/clips/user/`).
pub async fn user_reels(
    client: &IgClient,
    user_id: &str,
    limit: usize,
    flag: &AtomicBool,
    progress: &super::super::ProgressFn,
    job: &str,
) -> Result<Vec<MediaItem>, IgError> {
    let mut out = Vec::new();
    let mut max_id: Option<String> = None;
    let id = format!("ig:{}", job);
    loop {
        let mut form = vec![
            ("target_user_id", user_id.to_string()),
            ("page_size", "12".to_string()),
            ("include_feed_video", "true".to_string()),
        ];
        if let Some(m) = &max_id {
            form.push(("max_id", m.clone()));
        }
        let json = client.post_form("/api/v1/clips/user/", &form).await?;
        let items = json
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default();
        for it in &items {
            if let Some(item) = it.get("media").and_then(parse_item) {
                out.push(item);
            }
        }
        super::super::report(
            progress,
            &id,
            "list",
            out.len() as u64,
            if limit > 0 { Some(limit as u64) } else { None },
            None,
        );
        let paging = json.get("paging_info").cloned().unwrap_or(Value::Null);
        max_id = paging
            .get("max_id")
            .and_then(|m| m.as_str())
            .map(|m| m.to_string());
        if items.is_empty()
            || !b(&paging, "more_available")
            || max_id.is_none()
            || (limit > 0 && out.len() >= limit)
            || !page_pause_check(flag)
        {
            break;
        }
        client.pause().await;
    }
    if limit > 0 {
        out.truncate(limit);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn rest_feed(
    client: &IgClient,
    path: &str,
    count: u32,
    limit: usize,
    flag: &AtomicBool,
    progress: &super::super::ProgressFn,
    job: &str,
    wrapped: bool,
) -> Result<Vec<MediaItem>, IgError> {
    let mut out = Vec::new();
    let mut max_id: Option<String> = None;
    let id = format!("ig:{}", job);
    loop {
        let mut q = vec![("count", count.to_string())];
        if let Some(m) = &max_id {
            q.push(("max_id", m.clone()));
        }
        let json = client.get_json(path, &q).await?;
        let items = json
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default();
        for it in &items {
            let node = if wrapped {
                it.get("media").unwrap_or(it)
            } else {
                it
            };
            if let Some(item) = parse_item(node) {
                out.push(item);
            }
        }
        super::super::report(
            progress,
            &id,
            "list",
            out.len() as u64,
            if limit > 0 { Some(limit as u64) } else { None },
            None,
        );
        max_id = json.get("next_max_id").and_then(|m| match m {
            Value::String(x) => Some(x.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        });
        if items.is_empty()
            || !b(&json, "more_available")
            || max_id.is_none()
            || (limit > 0 && out.len() >= limit)
            || !page_pause_check(flag)
        {
            break;
        }
        client.pause().await;
    }
    if limit > 0 {
        out.truncate(limit);
    }
    Ok(out)
}

/// Posts em que o perfil foi marcado.
pub async fn user_tagged(
    client: &IgClient,
    user_id: &str,
    limit: usize,
    flag: &AtomicBool,
    progress: &super::super::ProgressFn,
    job: &str,
) -> Result<Vec<MediaItem>, IgError> {
    rest_feed(
        client,
        &format!("/api/v1/usertags/{}/feed/", user_id),
        20,
        limit,
        flag,
        progress,
        job,
        false,
    )
    .await
}

/// Posts salvos da própria conta (todos, sem separar por coleção).
pub async fn saved(
    client: &IgClient,
    limit: usize,
    flag: &AtomicBool,
    progress: &super::super::ProgressFn,
    job: &str,
) -> Result<Vec<MediaItem>, IgError> {
    rest_feed(
        client,
        "/api/v1/feed/saved/posts/",
        30,
        limit,
        flag,
        progress,
        job,
        true,
    )
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Highlight {
    pub id: String,
    pub title: String,
    pub cover: String,
    pub media_count: u64,
    pub created_at: i64,
}

/// `GET /api/v1/highlights/{id}/highlights_tray/`
pub async fn highlights(client: &IgClient, user_id: &str) -> Result<Vec<Highlight>, IgError> {
    let json = client
        .get_json(
            &format!("/api/v1/highlights/{}/highlights_tray/", user_id),
            &[],
        )
        .await?;
    let tray = json
        .get("tray")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(tray
        .iter()
        .map(|h| Highlight {
            id: s(h, "id").trim_start_matches("highlight:").to_string(),
            title: s(h, "title"),
            cover: h
                .get("cover_media")
                .and_then(|c| {
                    c.get("cropped_image_version")
                        .or(c.get("full_image_version"))
                })
                .and_then(|i| i.get("url"))
                .and_then(|x| x.as_str())
                .map(|x| x.to_string())
                .unwrap_or_default(),
            media_count: u(h, "media_count"),
            created_at: h.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct Reel {
    pub id: String,
    pub title: Option<String>,
    pub username: String,
    pub user_id: String,
    pub items: Vec<MediaItem>,
    pub expiring_at: Option<i64>,
    /// `true` quando o story é só para melhores amigos.
    pub close_friends: bool,
}

/// `GET /api/v1/feed/reels_media/?reel_ids=…` para stories (id do usuário)
/// ou highlights (`highlight:<id>`). Buscar assim não marca o story como visto.
pub async fn reels_media(client: &IgClient, reel_ids: &[String]) -> Result<Vec<Reel>, IgError> {
    let q: Vec<(&str, String)> = reel_ids.iter().map(|r| ("reel_ids", r.clone())).collect();
    let json = client.get_json("/api/v1/feed/reels_media/", &q).await?;
    let reels = json
        .get("reels_media")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for reel in reels {
        let user = reel.get("user").cloned().unwrap_or(Value::Null);
        let username = s(&user, "username");
        let title = reel
            .get("title")
            .and_then(|t| t.as_str())
            .map(|t| t.to_string());
        let mut items = Vec::new();
        for it in reel
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let mut node = it.clone();
            if node.get("user").is_none() {
                node["user"] = user.clone();
            }
            if let Some(mut item) = parse_item(&node) {
                item.title = title.clone();
                if item.username.is_empty() {
                    item.username = username.clone();
                }
                items.push(item);
            }
        }
        let close_friends = reel
            .get("items")
            .and_then(|i| i.as_array())
            .map(|a| a.iter().any(|x| s(x, "audience") == "besties"))
            .unwrap_or(false);
        out.push(Reel {
            id: s(&reel, "id"),
            title,
            username,
            user_id: s(&user, "pk"),
            items,
            expiring_at: reel.get("expiring_at").and_then(|x| x.as_i64()),
            close_friends,
        });
    }
    Ok(out)
}

pub async fn stories_of(client: &IgClient, user_id: &str) -> Result<Vec<MediaItem>, IgError> {
    let reels = reels_media(client, &[user_id.to_string()]).await?;
    Ok(reels.into_iter().flat_map(|r| r.items).collect())
}

pub async fn highlight_items(
    client: &IgClient,
    highlight_id: &str,
) -> Result<Vec<MediaItem>, IgError> {
    let id = if highlight_id.starts_with("highlight:") {
        highlight_id.to_string()
    } else {
        format!("highlight:{}", highlight_id)
    };
    let reels = reels_media(client, &[id]).await?;
    Ok(reels.into_iter().flat_map(|r| r.items).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct TrayEntry {
    pub user_id: String,
    pub username: String,
    pub full_name: String,
    pub profile_pic_url: String,
    pub latest_reel_media: i64,
    pub seen: i64,
    pub close_friends: bool,
}

/// Bandeja de stories da própria conta (quem tem story agora).
pub async fn stories_tray(client: &IgClient) -> Result<Vec<TrayEntry>, IgError> {
    let json = client.get_json("/api/v1/feed/reels_tray/", &[]).await?;
    let tray = json
        .get("tray")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(tray
        .iter()
        .map(|t| {
            let user = t.get("user").cloned().unwrap_or(Value::Null);
            TrayEntry {
                user_id: s(&user, "pk"),
                username: s(&user, "username"),
                full_name: s(&user, "full_name"),
                profile_pic_url: s(&user, "profile_pic_url"),
                latest_reel_media: t
                    .get("latest_reel_media")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0),
                seen: t.get("seen").and_then(|x| x.as_i64()).unwrap_or(0),
                close_friends: b(t, "has_besties_media"),
            }
        })
        .collect())
}

/// Quem viu um story seu (`/api/v1/media/{pk}/list_reel_media_viewer/`).
pub async fn story_viewers(
    client: &IgClient,
    story_pk: &str,
    flag: &AtomicBool,
) -> Result<(u64, Vec<super::follow::MiniUser>), IgError> {
    let mut out = Vec::new();
    let mut max_id: Option<String> = None;
    let mut total = 0u64;
    loop {
        let mut q: Vec<(&str, String)> = Vec::new();
        if let Some(m) = &max_id {
            q.push(("max_id", m.clone()));
        }
        let json = client
            .get_json(
                &format!("/api/v1/media/{}/list_reel_media_viewer/", story_pk),
                &q,
            )
            .await?;
        total = total.max(u(&json, "total_viewer_count"));
        let users = json
            .get("users")
            .and_then(|u| u.as_array())
            .cloned()
            .unwrap_or_default();
        for us in &users {
            out.push(super::follow::MiniUser::from_value(us));
        }
        max_id = json
            .get("next_max_id")
            .and_then(|m| m.as_str())
            .map(|m| m.to_string());
        if users.is_empty() || max_id.is_none() || super::cancelled(flag) {
            break;
        }
        client.pause().await;
    }
    Ok((total, out))
}

#[derive(Debug, Clone, Serialize)]
pub struct Friendship {
    pub following: bool,
    pub followed_by: bool,
    pub blocking: bool,
    pub is_bestie: bool,
    pub is_private: bool,
    pub incoming_request: bool,
    pub outgoing_request: bool,
    pub is_restricted: bool,
    pub muting: bool,
}

pub async fn friendship(client: &IgClient, user_id: &str) -> Result<Friendship, IgError> {
    let v = client
        .get_json(&format!("/api/v1/friendships/show/{}/", user_id), &[])
        .await?;
    Ok(Friendship {
        following: b(&v, "following"),
        followed_by: b(&v, "followed_by"),
        blocking: b(&v, "blocking"),
        is_bestie: b(&v, "is_bestie"),
        is_private: b(&v, "is_private"),
        incoming_request: b(&v, "incoming_request"),
        outgoing_request: b(&v, "outgoing_request"),
        is_restricted: b(&v, "is_restricted"),
        muting: b(&v, "muting"),
    })
}
