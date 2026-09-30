//! YouTube (video face) browsing commands: home, search, watch page,
//! comments, channels, playlists, the account feeds and account actions.
//! Every list goes through the distraction filters with the local watch
//! history, in the context of the page it came from.

use omniget_core::core::tube::filters::{self, FilterCtx, FilterPrefs};
use omniget_core::core::tube::innertube;
use omniget_core::core::tube::parse::{music as pmusic, web};
use omniget_core::core::tube::{
    ChannelPage, CommentsPage, Item, Nav, Page, PlaylistPage, Shelf, ShelfKind, VideoItem,
    WatchInfo,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{account, http, local};

fn watched(id: &str) -> Option<u8> {
    local::watched_pct(id)
}

fn prefs() -> FilterPrefs {
    FilterPrefs::from_settings(&http::settings())
}

pub(crate) fn filter_page(page: &mut Page, ctx: FilterCtx) {
    filters::apply_page(page, &prefs(), ctx, &watched);
}

pub(crate) fn filter_items(items: &mut Vec<Item>, ctx: FilterCtx) {
    filters::apply_items(items, &prefs(), ctx, &watched);
}

async fn browse(browse_id: &str, params: Option<&str>) -> Result<Value, String> {
    http::web("browse", |c, v, hl, gl| {
        innertube::browse_body(c, v, hl, gl, browse_id, params)
    })
    .await
}

async fn continue_with(endpoint: &str, token: &str) -> Result<Value, String> {
    http::web(endpoint, |c, v, hl, gl| {
        innertube::continuation_body(c, v, hl, gl, token)
    })
    .await
}

fn is_channel_id(id: &str) -> bool {
    id.len() == 24 && id.starts_with("UC")
}

/// First `browseEndpoint.browseId` under `v`.
fn find_browse_id(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            if let Some(id) = map
                .get("browseEndpoint")
                .and_then(|b| b.get("browseId"))
                .and_then(Value::as_str)
            {
                return Some(id.to_string());
            }
            map.values().find_map(find_browse_id)
        }
        Value::Array(items) => items.iter().find_map(find_browse_id),
        _ => None,
    }
}

/// `UC...`, `@handle`, `c/name`, `user/name` or a full URL -> channel id.
async fn resolve_channel(id: &str) -> Result<String, String> {
    let id = id.trim();
    if is_channel_id(id) {
        return Ok(id.to_string());
    }
    let path = id
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("m.")
        .trim_start_matches("youtube.com/")
        .trim_start_matches('/');
    let path = path.strip_prefix("channel/").unwrap_or(path);
    if is_channel_id(path) {
        return Ok(path.to_string());
    }
    let url = format!("https://www.youtube.com/{path}");
    let json = http::web("navigation/resolve_url", |c, v, hl, gl| {
        http::body_with(c, v, hl, gl, json!({ "url": url }))
    })
    .await?;
    find_browse_id(&json)
        .filter(|b| b.starts_with("UC"))
        .ok_or_else(|| "channel not found".to_string())
}

#[tauri::command]
pub async fn tube_home(chip_token: Option<String>) -> Result<Page, String> {
    if http::settings().hide_home_feed {
        return Ok(Page::default());
    }
    let chip = chip_token.filter(|t| !t.is_empty());
    let is_chip = chip.is_some();
    let mut page = match chip {
        Some(token) => web::continuation(&continue_with("browse", &token).await?),
        None => web::home(&browse("FEwhat_to_watch", None).await?),
    };
    // Without an account (or history on the account) YouTube answers the home
    // with just a "try searching" nudge. Build one from what we know locally.
    if !is_chip && page.shelves.is_empty() {
        let local = local_home().await;
        if !local.is_empty() {
            page.shelves = local;
            page.header = None;
        }
    }
    filter_page(&mut page, FilterCtx::Home);
    Ok(page)
}

/// Home for people without an account: continue watching, the latest from
/// local subscriptions, and videos related to what was watched recently.
async fn local_home() -> Vec<Shelf> {
    let mut shelves = Vec::new();
    let history: Vec<Item> = local::local_history_page(40)
        .ok()
        .and_then(|p| p.shelves.into_iter().next())
        .map(|s| s.items)
        .unwrap_or_default();

    let resume: Vec<Item> = history
        .iter()
        .filter(
            |i| matches!(i, Item::Video(v) if v.watched_pct.is_some_and(|p| (3..90).contains(&p))),
        )
        .take(12)
        .cloned()
        .collect();
    if !resume.is_empty() {
        shelves.push(Shelf {
            id: "continue".into(),
            title: Some("tube.home.continue_watching".into()),
            kind: ShelfKind::Row,
            items: resume,
            ..Default::default()
        });
    }

    if let Ok(feed) = local::local_feed(false).await {
        let items: Vec<Item> = feed
            .shelves
            .into_iter()
            .flat_map(|s| s.items)
            .take(12)
            .collect();
        if !items.is_empty() {
            shelves.push(Shelf {
                id: "subscriptions".into(),
                title: Some("tube.home.from_subscriptions".into()),
                kind: ShelfKind::Row,
                items,
                more: Some(Nav {
                    browse_id: "FEsubscriptions".into(),
                    params: None,
                }),
                ..Default::default()
            });
        }
    }

    // Related videos of the last few distinct videos, interleaved.
    let seeds: Vec<String> = history
        .iter()
        .filter_map(|i| match i {
            Item::Video(v) => Some(v.id.clone()),
            _ => None,
        })
        .take(4)
        .collect();
    let mut lists = Vec::new();
    for id in &seeds {
        let json = http::web("next", |c, v, hl, gl| {
            innertube::next_body(c, v, hl, gl, Some(id), None, None, None)
        })
        .await;
        if let Ok(json) = json {
            lists.push(web::watch(&json).related);
        }
    }
    let mut seen: std::collections::HashSet<String> = seeds.into_iter().collect();
    let mut picks = Vec::new();
    let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
    for i in 0..longest {
        for list in &lists {
            if let Some(Item::Video(v)) = list.get(i) {
                if seen.insert(v.id.clone()) {
                    picks.push(Item::Video(v.clone()));
                }
            }
        }
    }
    picks.truncate(36);
    if !picks.is_empty() {
        shelves.push(Shelf {
            id: "for_you".into(),
            title: Some("tube.home.for_you".into()),
            kind: ShelfKind::Grid,
            items: picks,
            ..Default::default()
        });
    }
    shelves
}

#[tauri::command]
pub async fn tube_search(query: String, filter: Option<String>) -> Result<Page, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Ok(Page::default());
    }
    let params = filter.as_deref().and_then(innertube::search_params);
    let json = http::web("search", |c, v, hl, gl| {
        innertube::search_body(c, v, hl, gl, &query, params)
    })
    .await?;
    let mut page = web::search(&json);
    filter_page(&mut page, FilterCtx::Search);
    Ok(page)
}

#[tauri::command]
pub async fn tube_suggest(query: String, music: bool) -> Result<Vec<String>, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if music {
        // anonymous on purpose: typeahead must not land in the account's search history
        let json = http::anonymous(
            "WEB_REMIX",
            "music/get_search_suggestions",
            |c, v, hl, gl| http::body_with(c, v, hl, gl, json!({ "input": query })),
        )
        .await?;
        return Ok(pmusic::music_suggestions(&json));
    }
    let (hl, gl) = http::locale();
    let url = format!(
        "https://suggestqueries.google.com/complete/search?client=youtube&ds=yt&hl={}&gl={}&q={}",
        urlencoding::encode(&hl),
        urlencoding::encode(&gl),
        urlencoding::encode(&query)
    );
    Ok(web::suggestions(&http::get_text(&url).await?))
}

/// Endpoint, parser and filter context of a continuation token.
#[tauri::command]
pub async fn tube_continuation(kind: String, token: String) -> Result<Page, String> {
    let music = kind.starts_with("music_");
    let endpoint = match kind.as_str() {
        "search" | "music_search" => "search",
        "related" => "next",
        _ => "browse",
    };
    let json = if music {
        http::music(endpoint, |c, v, hl, gl| {
            innertube::continuation_body(c, v, hl, gl, &token)
        })
        .await?
    } else {
        continue_with(endpoint, &token).await?
    };
    let mut page = if music {
        pmusic::music_continuation(&json)
    } else {
        web::continuation(&json)
    };
    let ctx = match kind.as_str() {
        "home" => Some(FilterCtx::Home),
        "search" => Some(FilterCtx::Search),
        "feed" => Some(FilterCtx::Subscriptions),
        "channel" => Some(FilterCtx::Channel),
        "playlist" => Some(FilterCtx::Playlist),
        "related" => Some(FilterCtx::Related),
        // the history is what the user watched: filtering it would hide it
        "history" | "music_library" => None,
        k if k.starts_with("music_") => Some(FilterCtx::Music),
        _ => Some(FilterCtx::Home),
    };
    if let Some(ctx) = ctx {
        filter_page(&mut page, ctx);
    }
    Ok(page)
}

#[tauri::command]
pub async fn tube_watch(
    video_id: String,
    playlist_id: Option<String>,
    index: Option<u32>,
) -> Result<WatchInfo, String> {
    let json = http::web("next", |c, v, hl, gl| {
        innertube::next_body(
            c,
            v,
            hl,
            gl,
            Some(&video_id),
            playlist_id.as_deref(),
            index,
            None,
        )
    })
    .await?;
    let mut info = web::watch(&json);
    let s = http::settings();
    if s.hide_related {
        info.related.clear();
        info.related_continuation = None;
    } else {
        filter_items(&mut info.related, FilterCtx::Related);
    }
    if s.hide_comments {
        info.comments_token = None;
        info.comments_count_text = None;
    }
    if info.saved.is_none() && !account::is_logged_in() {
        info.saved = Some(local::watch_later_has(&info.id));
    }
    Ok(info)
}

#[tauri::command]
pub async fn tube_comments(token: String) -> Result<CommentsPage, String> {
    if http::settings().hide_comments {
        return Ok(CommentsPage::default());
    }
    Ok(web::comments(&continue_with("next", &token).await?))
}

/// Chip order on the Videos tab: Latest, Popular, Oldest.
fn sort_index(sort: &str) -> Option<usize> {
    match sort {
        "popular" => Some(1),
        "oldest" => Some(2),
        _ => None,
    }
}

#[tauri::command]
pub async fn tube_channel(
    id: String,
    tab: Option<String>,
    sort: Option<String>,
) -> Result<ChannelPage, String> {
    let channel_id = resolve_channel(&id).await?;
    let tab = tab.unwrap_or_else(|| "home".into());
    let params = innertube::channel_tab_params(&tab);
    let mut page = web::channel(&browse(&channel_id, params).await?);
    if let Some(i) = sort.as_deref().and_then(sort_index) {
        let token = page.page.chips.get(i).and_then(|c| c.token.clone());
        if let Some(token) = token {
            let sorted = web::continuation(&continue_with("browse", &token).await?);
            page.page.shelves = sorted.shelves;
            page.page.continuation = sorted.continuation;
            for (n, chip) in page.page.chips.iter_mut().enumerate() {
                chip.selected = n == i;
            }
        }
    }
    filter_page(&mut page.page, FilterCtx::Channel);
    if page.channel.subscribed.is_none() && !account::is_logged_in() {
        page.channel.subscribed = Some(local::is_subscribed(&page.channel.id));
    }
    Ok(page)
}

/// A channel's uploads (Videos tab, newest first). Used by the local
/// subscriptions feed when the RSS feed fails.
pub async fn channel_videos(channel_id: &str) -> Result<Vec<VideoItem>, String> {
    let json = browse(channel_id, innertube::channel_tab_params("videos")).await?;
    let page = web::channel(&json);
    Ok(page
        .page
        .shelves
        .into_iter()
        .flat_map(|s| s.items)
        .filter_map(|i| match i {
            Item::Video(v) => Some(v),
            _ => None,
        })
        .collect())
}

fn vl(id: &str) -> String {
    if id.starts_with("VL") {
        id.to_string()
    } else {
        format!("VL{id}")
    }
}

#[tauri::command]
pub async fn tube_playlist(id: String) -> Result<PlaylistPage, String> {
    let mut page = web::playlist(&browse(&vl(&id), None).await?);
    if page.id.is_empty() {
        page.id = id.trim_start_matches("VL").to_string();
    }
    filter_items(&mut page.items, FilterCtx::Playlist);
    Ok(page)
}

#[tauri::command]
pub async fn tube_subscriptions_feed(refresh: bool) -> Result<Page, String> {
    let mut page = if account::is_logged_in() {
        web::feed(&browse("FEsubscriptions", None).await?)
    } else {
        local::local_feed(refresh).await?
    };
    filter_page(&mut page, FilterCtx::Subscriptions);
    Ok(page)
}

#[tauri::command]
pub async fn tube_library() -> Result<Page, String> {
    if !account::is_logged_in() {
        return local::local_library_page().await;
    }
    // "You" replaced the old Library page; keep the old id as a fallback
    let page = web::feed(&browse("FEyou", None).await?);
    if !page.shelves.is_empty() {
        return Ok(page);
    }
    Ok(web::feed(&browse("FElibrary", None).await?))
}

#[tauri::command]
pub async fn tube_history() -> Result<Page, String> {
    if !account::is_logged_in() {
        return local::local_history_page(200);
    }
    Ok(web::feed(&browse("FEhistory", None).await?))
}

#[tauri::command]
pub async fn tube_shorts() -> Result<Page, String> {
    if http::settings().hide_shorts {
        return Ok(Page::default());
    }
    let mut page = web::feed(&browse("FEshorts", None).await?);
    filter_page(&mut page, FilterCtx::Home);
    Ok(page)
}

// ---------- account actions ----------

#[derive(Debug, Default, Clone, Deserialize)]
pub struct ActionTarget {
    #[serde(default)]
    pub video_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub playlist_id: Option<String>,
    #[serde(default)]
    pub set_video_id: Option<String>,
    #[serde(default)]
    pub feedback_token: Option<String>,
    /// Card metadata, so local lists (watch later) can show the item.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

fn need<'a>(v: &'a Option<String>, what: &str) -> Result<&'a str, String> {
    v.as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing {what}"))
}

async fn call(music: bool, endpoint: &str, extra: Value) -> Result<(), String> {
    let f = move |c: &innertube::ClientDef, v: Option<&str>, hl: &str, gl: &str| {
        http::body_with(c, v, hl, gl, extra)
    };
    if music {
        http::music(endpoint, f).await?;
    } else {
        http::web(endpoint, f).await?;
    }
    Ok(())
}

fn edit_playlist(playlist_id: &str, action: Value) -> Value {
    json!({ "playlistId": playlist_id, "actions": [action] })
}

/// like, subscribe, watch later, playlist edits and feedback on the account.
/// Without an account, subscriptions and watch later go to the local lists;
/// the rest is a no-op (the card hides itself on the front).
#[tauri::command]
pub async fn tube_action(action: String, target: ActionTarget) -> Result<(), String> {
    let logged = account::is_logged_in();
    match action.as_str() {
        "subscribe" if !logged => {
            let id = need(&target.channel_id, "channel id")?;
            local::subs_add(
                id,
                target.channel_name.as_deref().unwrap_or(id),
                target.avatar.as_deref(),
            )
        }
        "unsubscribe" if !logged => local::subs_remove(need(&target.channel_id, "channel id")?),
        "watch_later_add" if !logged => {
            let id = need(&target.video_id, "video id")?;
            local::watch_later_add(&VideoItem {
                id: id.to_string(),
                title: target.title.clone().unwrap_or_default(),
                channel: target.channel_id.as_ref().map(|cid| {
                    omniget_core::core::tube::ChannelRef {
                        id: cid.clone(),
                        name: target.channel_name.clone().unwrap_or_default(),
                        avatar: target.avatar.clone(),
                        ..Default::default()
                    }
                }),
                thumbs: vec![omniget_core::core::tube::Thumb {
                    url: format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"),
                    width: 320,
                    height: 180,
                }],
                ..Default::default()
            })
        }
        "watch_later_remove" if !logged => {
            local::watch_later_remove(need(&target.video_id, "video id")?)
        }
        _ if !logged => Ok(()),
        "like" | "dislike" | "remove_like" | "music_like" | "music_dislike"
        | "music_remove_like" => {
            let id = need(&target.video_id, "video id")?;
            let music = action.starts_with("music_");
            let endpoint = match action.trim_start_matches("music_") {
                "like" => "like/like",
                "dislike" => "like/dislike",
                _ => "like/removelike",
            };
            call(music, endpoint, json!({ "target": { "videoId": id } })).await
        }
        "subscribe" | "unsubscribe" => {
            let id = need(&target.channel_id, "channel id")?;
            call(
                false,
                &format!("subscription/{action}"),
                json!({ "channelIds": [id] }),
            )
            .await
        }
        "watch_later_add" => {
            let id = need(&target.video_id, "video id")?;
            call(
                false,
                "browse/edit_playlist",
                edit_playlist(
                    "WL",
                    json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": id }),
                ),
            )
            .await
        }
        "watch_later_remove" => {
            let id = need(&target.video_id, "video id")?;
            call(
                false,
                "browse/edit_playlist",
                edit_playlist(
                    "WL",
                    json!({ "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": id }),
                ),
            )
            .await
        }
        "playlist_add" => {
            let pl = need(&target.playlist_id, "playlist id")?;
            let id = need(&target.video_id, "video id")?;
            call(
                false,
                "browse/edit_playlist",
                edit_playlist(
                    pl,
                    json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": id }),
                ),
            )
            .await
        }
        "playlist_remove" => {
            let pl = need(&target.playlist_id, "playlist id")?;
            let op = match target.set_video_id.as_deref().filter(|s| !s.is_empty()) {
                Some(set_id) => json!({
                    "action": "ACTION_REMOVE_VIDEO",
                    "setVideoId": set_id,
                    "removedVideoId": target.video_id,
                }),
                None => json!({
                    "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID",
                    "removedVideoId": need(&target.video_id, "video id")?,
                }),
            };
            call(false, "browse/edit_playlist", edit_playlist(pl, op)).await
        }
        "not_interested" | "dont_recommend_channel" | "library_add" | "library_remove" => {
            let token = need(&target.feedback_token, "feedback token")?;
            let music = action.starts_with("library_");
            call(music, "feedback", json!({ "feedbackTokens": [token] })).await
        }
        other => Err(format!("unknown action {other}")),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SavePlaylist {
    pub id: String,
    pub title: String,
    pub contains: bool,
}

#[tauri::command]
pub async fn tube_save_playlists(video_id: String) -> Result<Vec<SavePlaylist>, String> {
    if !account::is_logged_in() {
        return Ok(Vec::new());
    }
    let json = http::web("playlist/get_add_to_playlist", |c, v, hl, gl| {
        http::body_with(c, v, hl, gl, json!({ "videoIds": [video_id] }))
    })
    .await?;
    Ok(web::save_playlists(&json)
        .into_iter()
        .map(|(id, title, contains)| SavePlaylist {
            id,
            title,
            contains,
        })
        .collect())
}

#[tauri::command]
pub async fn tube_playlist_create(
    title: String,
    video_ids: Vec<String>,
    music: bool,
) -> Result<String, String> {
    if !account::is_logged_in() {
        return Err("sign in to create playlists".into());
    }
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("missing title".into());
    }
    let extra = json!({ "title": title, "videoIds": video_ids, "privacyStatus": "PRIVATE" });
    let f = move |c: &innertube::ClientDef, v: Option<&str>, hl: &str, gl: &str| {
        http::body_with(c, v, hl, gl, extra)
    };
    let json = if music {
        http::music("playlist/create", f).await?
    } else {
        http::web("playlist/create", f).await?
    };
    json.get("playlistId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "YouTube did not create the playlist".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_ids_and_browse_lookup() {
        assert!(is_channel_id("UCuAXFkgsw1L7xaCfnd5JJOw"));
        assert!(!is_channel_id("@handle"));
        let v =
            json!({"endpoint": {"commandMetadata": {}, "browseEndpoint": {"browseId": "UCabc"}}});
        assert_eq!(find_browse_id(&v).as_deref(), Some("UCabc"));
    }

    #[test]
    fn playlist_ids_get_the_vl_prefix_once() {
        assert_eq!(vl("PL123"), "VLPL123");
        assert_eq!(vl("VLPL123"), "VLPL123");
    }

    #[test]
    fn action_target_accepts_the_front_shape() {
        let t: ActionTarget =
            serde_json::from_value(json!({"video_id": "abc", "channel_id": null})).unwrap();
        assert_eq!(t.video_id.as_deref(), Some("abc"));
        assert!(need(&t.channel_id, "channel id").is_err());
    }
}
