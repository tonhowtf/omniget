use serde_json::Value;

use crate::core::tube::model::*;
use crate::core::tube::walk::{
    self, best_thumb, browse_of, channel_from_runs, chips, collect_items, find_continuation,
    find_first, find_key, log_unknown, parse_count, parse_duration, runs, shelves_of, str_at, text,
    text_at, thumbs,
};

fn selected_tab(json: &Value) -> Option<&Value> {
    let tabs = json
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs")
        .and_then(Value::as_array)?;
    let renderers: Vec<&Value> = tabs
        .iter()
        .filter_map(|t| {
            t.get("tabRenderer")
                .or_else(|| t.get("expandableTabRenderer"))
        })
        .collect();
    renderers
        .iter()
        .find(|t| {
            t.get("selected").and_then(Value::as_bool) == Some(true) && t.get("content").is_some()
        })
        .or_else(|| renderers.iter().find(|t| t.get("content").is_some()))
        .and_then(|t| t.get("content"))
}

fn page_from(content: &Value, default_kind: ShelfKind) -> Page {
    let shelves = shelves_of(content, default_kind);
    let header = content
        .pointer("/richGridRenderer/header")
        .or_else(|| content.pointer("/sectionListRenderer/header"))
        .unwrap_or(&Value::Null);
    Page {
        header: None,
        chips: chips(header),
        shelves,
        continuation: find_continuation(content),
        corrected_query: None,
    }
}

pub fn home(json: &Value) -> Page {
    let Some(content) = selected_tab(json) else {
        return Page::default();
    };
    let mut page = page_from(content, ShelfKind::Grid);
    if page.shelves.is_empty() {
        if let Some(n) = find_first(content, "feedNudgeRenderer") {
            page.header = Some(PageHeader {
                title: n.get("title").and_then(text).unwrap_or_default(),
                subtitle: n.get("subtitle").and_then(text),
                ..Default::default()
            });
        }
    }
    log_unknown("home");
    page
}

pub fn search(json: &Value) -> Page {
    let primary = json
        .pointer("/contents/twoColumnSearchResultsRenderer/primaryContents")
        .unwrap_or(&Value::Null);
    let mut page = Page {
        shelves: shelves_of(primary, ShelfKind::List),
        continuation: find_continuation(primary),
        corrected_query: corrected_query(json),
        chips: chips(json.get("header").unwrap_or(&Value::Null)),
        header: None,
    };
    if page.chips.is_empty() {
        page.chips = chips(find_first(primary, "searchSubMenuRenderer").unwrap_or(&Value::Null));
    }
    log_unknown("search");
    page
}

pub(crate) fn corrected_query(json: &Value) -> Option<String> {
    for key in [
        "showingResultsForRenderer",
        "didYouMeanRenderer",
        "includingResultsForRenderer",
    ] {
        if let Some(r) = find_first(json, key) {
            if let Some(q) = r.get("correctedQuery").and_then(text) {
                return Some(q);
            }
        }
    }
    None
}

pub fn continuation(json: &Value) -> Page {
    let mut page = Page::default();
    let mut item_roots: Vec<&Value> = Vec::new();
    for key in [
        "onResponseReceivedActions",
        "onResponseReceivedCommands",
        "onResponseReceivedEndpoints",
    ] {
        let Some(actions) = json.get(key).and_then(Value::as_array) else {
            continue;
        };
        for a in actions {
            if let Some(r) = a.get("reloadContinuationItemsCommand") {
                if r.get("slot").and_then(Value::as_str) == Some("RELOAD_CONTINUATION_SLOT_HEADER")
                {
                    page.chips.extend(chips(r));
                    continue;
                }
                if let Some(items) = r.get("continuationItems") {
                    item_roots.push(items);
                }
            }
            if let Some(items) = a.pointer("/appendContinuationItemsAction/continuationItems") {
                item_roots.push(items);
            }
        }
    }
    if let Some(cc) = json.get("continuationContents") {
        item_roots.push(cc);
    }
    for root in item_roots {
        let grid = !find_key(root, "richItemRenderer").is_empty();
        let kind = if grid {
            ShelfKind::Grid
        } else {
            ShelfKind::List
        };
        page.shelves.extend(shelves_of(root, kind));
        if let Some(c) = find_continuation(root) {
            page.continuation = Some(c);
        }
    }
    page.shelves = walk::finalize(page.shelves);
    log_unknown("continuation");
    page
}

fn owner_badges(r: &Value) -> (bool, bool) {
    let styles: Vec<&str> = find_key(r.get("badges").unwrap_or(&Value::Null), "style")
        .into_iter()
        .filter_map(Value::as_str)
        .collect();
    let artist = styles.contains(&"BADGE_STYLE_TYPE_VERIFIED_ARTIST");
    (
        artist || styles.contains(&"BADGE_STYLE_TYPE_VERIFIED"),
        artist,
    )
}

fn handle_of(ep: &Value) -> Option<String> {
    str_at(walk::endpoint(ep), "/browseEndpoint/canonicalBaseUrl")
        .map(|u| u.trim_start_matches('/'))
        .filter(|u| u.starts_with('@'))
        .map(str::to_string)
}

fn subscribed_state(button: Option<&Value>) -> Option<bool> {
    let b = button?;
    if find_first(b, "signInEndpoint").is_some() {
        return None;
    }
    find_first(b, "subscribed").and_then(Value::as_bool)
}

fn chapters(json: &Value) -> Vec<Chapter> {
    let mut out: Vec<Chapter> = Vec::new();
    let panels = json.get("engagementPanels").and_then(Value::as_array);
    let mut source = None;
    for id in [
        "engagement-panel-macro-markers-description-chapters",
        "engagement-panel-macro-markers-auto-chapters",
    ] {
        source = panels.and_then(|ps| {
            ps.iter().find(|p| {
                str_at(p, "/engagementPanelSectionListRenderer/panelIdentifier") == Some(id)
                    || str_at(p, "/engagementPanelSectionListRenderer/targetId") == Some(id)
            })
        });
        if source.is_some() {
            break;
        }
    }
    if let Some(panel) = source {
        for m in find_key(panel, "macroMarkersListItemRenderer") {
            let Some(title) = m.get("title").and_then(text) else {
                continue;
            };
            let start = m
                .get("timeDescription")
                .and_then(text)
                .as_deref()
                .and_then(parse_duration)
                .or_else(|| {
                    find_first(m.get("onTap").unwrap_or(&Value::Null), "startTimeSeconds")
                        .and_then(Value::as_u64)
                        .map(|s| s as u32)
                });
            let Some(start_s) = start else { continue };
            out.push(Chapter {
                title,
                start_s,
                thumb: m.get("thumbnail").and_then(best_thumb),
            });
        }
    }
    if out.is_empty() {
        for c in find_key(json, "chapterRenderer") {
            let (Some(title), Some(ms)) = (
                c.get("title").and_then(text),
                c.get("timeRangeStartMillis").and_then(Value::as_u64),
            ) else {
                continue;
            };
            out.push(Chapter {
                title,
                start_s: (ms / 1000) as u32,
                thumb: c.get("thumbnail").and_then(best_thumb),
            });
        }
    }
    out.sort_by_key(|c| c.start_s);
    out.dedup_by_key(|c| c.start_s);
    out
}

fn comments_section(json: &Value) -> (Option<String>, Option<String>) {
    let results = json
        .pointer("/contents/twoColumnWatchNextResults/results/results/contents")
        .unwrap_or(&Value::Null);
    let mut token = None;
    for sec in find_key(results, "itemSectionRenderer") {
        let id = sec.get("sectionIdentifier").and_then(Value::as_str);
        let target = sec.get("targetId").and_then(Value::as_str);
        if id == Some("comment-item-section") || target == Some("comments-section") {
            token = find_continuation(sec);
        }
    }
    let panel = json
        .get("engagementPanels")
        .and_then(Value::as_array)
        .and_then(|ps| {
            ps.iter().find(|p| {
                str_at(p, "/engagementPanelSectionListRenderer/panelIdentifier")
                    == Some("engagement-panel-comments-section")
            })
        });
    if token.is_none() {
        token = panel.and_then(|p| {
            find_continuation(p.pointer("/engagementPanelSectionListRenderer/content")?)
        });
    }
    let count = panel
        .and_then(|p| p.pointer("/engagementPanelSectionListRenderer/header/engagementPanelTitleHeaderRenderer/contextualInfo"))
        .and_then(text)
        .or_else(|| {
            find_first(results, "commentsEntryPointHeaderRenderer")
                .and_then(|h| h.get("commentCount"))
                .and_then(text)
        });
    (token, count)
}

fn likes(json: &Value) -> (Option<u64>, Option<String>, Option<Rating>) {
    let Some(seg) = find_first(json, "segmentedLikeDislikeButtonViewModel") else {
        let t = find_key(json, "toggleButtonRenderer")
            .into_iter()
            .find(|b| str_at(b, "/defaultIcon/iconType") == Some("LIKE"));
        let label = t.and_then(|b| b.get("defaultText")).and_then(text);
        return (label.as_deref().and_then(parse_count), label, None);
    };
    let like = seg
        .pointer("/likeButtonViewModel/likeButtonViewModel")
        .unwrap_or(&Value::Null);
    let button = like
        .pointer(
            "/toggleButtonViewModel/toggleButtonViewModel/defaultButtonViewModel/buttonViewModel",
        )
        .unwrap_or(&Value::Null);
    let title = str_at(button, "/title").map(str::to_string);
    let a11y = str_at(button, "/accessibilityText");
    let count = a11y
        .and_then(parse_count)
        .or_else(|| title.as_deref().and_then(parse_count));
    let status = str_at(like, "/likeStatusEntity/likeStatus").or_else(|| {
        let key = str_at(like, "/likeStatusEntityKey")?;
        find_key(json.get("frameworkUpdates")?, "likeStatusEntity")
            .into_iter()
            .find(|e| e.get("key").and_then(Value::as_str) == Some(key))
            .and_then(|e| e.get("likeStatus"))
            .and_then(Value::as_str)
    });
    let rating = match status {
        Some("LIKE") => Some(Rating::Like),
        Some("DISLIKE") => Some(Rating::Dislike),
        _ => None,
    };
    (count, title, rating)
}

fn watch_playlist(json: &Value) -> Option<WatchPlaylist> {
    let p = json.pointer("/contents/twoColumnWatchNextResults/playlist/playlist")?;
    let id = p.get("playlistId").and_then(Value::as_str)?.to_string();
    let items = p
        .get("contents")
        .map(collect_items)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i {
            Item::Video(v) => Some(v),
            Item::Song(s) => Some(VideoItem {
                id: s.id,
                title: s.title,
                channel: s.artists.into_iter().next(),
                thumbs: s.thumbs,
                duration_s: s.duration_s,
                ..Default::default()
            }),
            _ => None,
        })
        .collect();
    Some(WatchPlaylist {
        is_mix: id.starts_with("RD") || p.get("isInfinite").and_then(Value::as_bool) == Some(true),
        title: p.get("title").and_then(text).unwrap_or_default(),
        owner: p.get("ownerName").and_then(text),
        index: p
            .get("currentIndex")
            .or_else(|| p.get("localCurrentIndex"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        total: p
            .get("totalVideos")
            .and_then(Value::as_u64)
            .map(|n| n as u32),
        items,
        id,
    })
}

pub fn watch(json: &Value) -> WatchInfo {
    let results = json
        .pointer("/contents/twoColumnWatchNextResults/results/results/contents")
        .unwrap_or(&Value::Null);
    let primary = find_first(results, "videoPrimaryInfoRenderer").unwrap_or(&Value::Null);
    let secondary = find_first(results, "videoSecondaryInfoRenderer").unwrap_or(&Value::Null);
    let id = str_at(json, "/currentVideoEndpoint/watchEndpoint/videoId")
        .map(str::to_string)
        .unwrap_or_default();

    let vc = primary
        .pointer("/viewCount/videoViewCountRenderer")
        .unwrap_or(&Value::Null);
    let views_text = vc.get("viewCount").and_then(text);
    let is_live = vc.get("isLive").and_then(Value::as_bool).unwrap_or(false);
    let mut badges = Vec::new();
    if is_live {
        badges.push(Badge::Live);
    }
    for s in find_key(primary.get("badges").unwrap_or(&Value::Null), "style") {
        if s.as_str() == Some("BADGE_STYLE_TYPE_MEMBERS_ONLY") {
            badges.push(Badge::Members);
        }
    }

    let owner = secondary
        .pointer("/owner/videoOwnerRenderer")
        .unwrap_or(&Value::Null);
    let channel = owner.get("title").and_then(channel_from_runs).map(|c| {
        let (verified, artist) = owner_badges(owner);
        ChannelItem {
            handle: owner.get("navigationEndpoint").and_then(handle_of),
            id: c.id,
            name: c.name,
            avatar: owner.get("thumbnail").and_then(best_thumb),
            subscribers_text: owner.get("subscriberCountText").and_then(text),
            verified,
            artist,
            subscribed: subscribed_state(secondary.get("subscribeButton")),
            ..Default::default()
        }
    });

    let mut description = secondary
        .get("attributedDescription")
        .map(runs)
        .unwrap_or_default();
    if description.is_empty() {
        description = secondary.get("description").map(runs).unwrap_or_default();
    }
    if description.is_empty() {
        if let Some(d) = find_first(json, "expandableVideoDescriptionBodyRenderer") {
            description = d
                .get("attributedDescriptionBodyText")
                .or_else(|| d.get("descriptionBodyText"))
                .map(runs)
                .unwrap_or_default();
        }
    }

    let (likes, likes_text, rating) = likes(results);
    let secondary_results = json
        .pointer("/contents/twoColumnWatchNextResults/secondaryResults")
        .unwrap_or(&Value::Null);
    let (comments_token, comments_count_text) = comments_section(json);

    let info = WatchInfo {
        id,
        title: primary.get("title").and_then(text).unwrap_or_default(),
        channel,
        views: views_text.as_deref().and_then(parse_count),
        views_text: vc.get("shortViewCount").and_then(text).or(views_text),
        published_text: primary.get("relativeDateText").and_then(text),
        date_text: primary.get("dateText").and_then(text),
        description,
        likes,
        likes_text,
        rating,
        chapters: chapters(json),
        related: collect_items(secondary_results),
        related_continuation: find_continuation(secondary_results),
        comments_token,
        comments_count_text,
        comments_sorts: comment_sorts(json),
        playlist: watch_playlist(json),
        is_live,
        badges,
        saved: None,
    };
    log_unknown("watch");
    info
}

pub fn comment_sorts(json: &Value) -> Vec<CommentSort> {
    let Some(menu) = find_first(json, "sortFilterSubMenuRenderer") else {
        return Vec::new();
    };
    menu.get("subMenuItems")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|it| {
                    let token = find_first(it, "continuationCommand")
                        .and_then(|c| c.get("token"))
                        .and_then(Value::as_str)
                        .or_else(|| {
                            str_at(it, "/continuation/reloadContinuationData/continuation")
                        })?;
                    Some(CommentSort {
                        label: it.get("title").and_then(text).unwrap_or_default(),
                        token: token.to_string(),
                        selected: it.get("selected").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn entity_map(json: &Value) -> std::collections::HashMap<&str, &Value> {
    let mut map = std::collections::HashMap::new();
    if let Some(ms) = json
        .pointer("/frameworkUpdates/entityBatchUpdate/mutations")
        .and_then(Value::as_array)
    {
        for m in ms {
            if let (Some(k), Some(p)) =
                (m.get("entityKey").and_then(Value::as_str), m.get("payload"))
            {
                map.insert(k, p);
            }
        }
    }
    map
}

fn replies_token(thread: &Value) -> Option<String> {
    let r = thread.pointer("/replies/commentRepliesRenderer")?;
    find_first(r, "continuationCommand")
        .and_then(|c| c.get("token"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn comment_from_vm(
    vm: &Value,
    entities: &std::collections::HashMap<&str, &Value>,
    thread: Option<&Value>,
) -> Option<Comment> {
    let key = vm.get("commentKey").and_then(Value::as_str)?;
    let payload = entities.get(key)?.get("commentEntityPayload")?;
    let props = payload.get("properties").unwrap_or(&Value::Null);
    let author = payload.get("author").unwrap_or(&Value::Null);
    let toolbar = payload.get("toolbar").unwrap_or(&Value::Null);
    let heart = vm
        .get("toolbarStateKey")
        .and_then(Value::as_str)
        .and_then(|k| entities.get(k))
        .and_then(|p| p.pointer("/engagementToolbarStateEntityPayload/heartState"))
        .and_then(Value::as_str);
    let reply_count = str_at(toolbar, "/replyCount")
        .and_then(parse_count)
        .unwrap_or(0) as u32;
    let likes_text = str_at(toolbar, "/likeCountNotliked")
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "0")
        .map(str::to_string);
    let pinned = vm.get("pinnedText").is_some()
        || props.get("pinnedText").is_some()
        || vm.pointer("/rendererContext/pinnedText").is_some();
    Some(Comment {
        id: props
            .get("commentId")
            .or_else(|| vm.get("commentId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        author: ChannelRef {
            id: author
                .get("channelId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            name: author
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            handle: author
                .get("displayName")
                .and_then(Value::as_str)
                .filter(|n| n.starts_with('@'))
                .map(str::to_string),
            avatar: author
                .get("avatarThumbnailUrl")
                .and_then(Value::as_str)
                .map(str::to_string),
            verified: author
                .get("isVerified")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            artist: author
                .get("isArtist")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        text: props.get("content").map(runs).unwrap_or_default(),
        published_text: str_at(props, "/publishedTime").map(str::to_string),
        likes_text,
        reply_count,
        replies_token: thread.and_then(replies_token),
        pinned,
        hearted: heart == Some("TOOLBAR_HEART_STATE_HEARTED"),
        is_owner: author
            .get("isCreator")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn comment_from_renderer(c: &Value, thread: Option<&Value>) -> Option<Comment> {
    let author_ep = c.get("authorEndpoint").unwrap_or(&Value::Null);
    let author_id = browse_of(author_ep)
        .map(|(id, _, _)| id.to_string())
        .unwrap_or_default();
    let name = c.get("authorText").and_then(text).unwrap_or_default();
    Some(Comment {
        id: c.get("commentId").and_then(Value::as_str)?.to_string(),
        author: ChannelRef {
            id: author_id,
            handle: name.starts_with('@').then(|| name.clone()),
            name,
            avatar: c.get("authorThumbnail").and_then(best_thumb),
            verified: c.get("authorCommentBadge").is_some(),
            artist: false,
        },
        text: c.get("contentText").map(runs).unwrap_or_default(),
        published_text: c.get("publishedTimeText").and_then(text),
        likes_text: c.get("voteCount").and_then(text),
        reply_count: c.get("replyCount").and_then(Value::as_u64).unwrap_or(0) as u32,
        replies_token: thread.and_then(replies_token),
        pinned: c.get("pinnedCommentBadge").is_some(),
        hearted: find_first(c, "creatorHeartRenderer")
            .and_then(|h| h.get("isHearted"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_owner: c
            .get("authorIsChannelOwner")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

pub fn comments(json: &Value) -> CommentsPage {
    let entities = entity_map(json);
    let mut page = CommentsPage {
        sorts: comment_sorts(json),
        ..Default::default()
    };
    let mut items: Vec<&Value> = Vec::new();
    for key in [
        "onResponseReceivedEndpoints",
        "onResponseReceivedActions",
        "onResponseReceivedCommands",
    ] {
        for a in json
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let list = a
                .pointer("/reloadContinuationItemsCommand/continuationItems")
                .or_else(|| a.pointer("/appendContinuationItemsAction/continuationItems"))
                .and_then(Value::as_array);
            items.extend(list.into_iter().flatten());
        }
    }
    for it in items {
        if let Some(h) = it.get("commentsHeaderRenderer") {
            page.count_text = h
                .get("countText")
                .and_then(text)
                .or_else(|| h.get("commentsCount").and_then(text));
            continue;
        }
        if let Some(thread) = it.get("commentThreadRenderer") {
            let c = thread
                .pointer("/commentViewModel/commentViewModel")
                .and_then(|vm| comment_from_vm(vm, &entities, Some(thread)))
                .or_else(|| {
                    thread
                        .pointer("/comment/commentRenderer")
                        .and_then(|c| comment_from_renderer(c, Some(thread)))
                });
            page.items.extend(c);
            continue;
        }
        if let Some(vm) = it.get("commentViewModel") {
            page.items.extend(comment_from_vm(vm, &entities, None));
            continue;
        }
        if let Some(c) = it.get("commentRenderer") {
            page.items.extend(comment_from_renderer(c, None));
            continue;
        }
        if let Some(c) = it.get("continuationItemRenderer") {
            page.continuation =
                walk::find_continuation(&serde_json::json!({ "continuationItemRenderer": c }));
        }
    }
    page
}

fn channel_tab_id(tab: &Value) -> String {
    let url = str_at(tab, "/endpoint/commandMetadata/webCommandMetadata/url").unwrap_or("");
    let last = url.rsplit('/').next().unwrap_or("");
    let id = if last.starts_with('@') || last.starts_with("UC") || last.is_empty() {
        tab.get("title")
            .and_then(text)
            .unwrap_or_default()
            .to_lowercase()
    } else {
        last.to_string()
    };
    match id.as_str() {
        "featured" => "home".into(),
        "posts" | "community" => "community".into(),
        "live" => "streams".into(),
        "released" => "releases".into(),
        _ => id,
    }
}

fn metadata_parts(vm: &Value) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for row in vm
        .pointer("/metadata/contentMetadataViewModel/metadataRows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for p in row
            .get("metadataParts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let t = p.get("text").and_then(text).or_else(|| {
                p.pointer("/avatarStack/avatarStackViewModel/text")
                    .and_then(text)
            });
            if let Some(t) = t {
                out.push((
                    t,
                    p.get("accessibilityLabel")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                ));
            }
        }
    }
    out
}

pub fn channel(json: &Value) -> ChannelPage {
    let meta = json
        .pointer("/metadata/channelMetadataRenderer")
        .unwrap_or(&Value::Null);
    let mut ch = ChannelItem {
        id: str_at(meta, "/externalId").unwrap_or("").to_string(),
        name: str_at(meta, "/title").unwrap_or("").to_string(),
        description: str_at(meta, "/description").map(str::to_string),
        avatar: meta.get("avatar").and_then(best_thumb),
        ..Default::default()
    };
    let mut banner = Vec::new();

    if let Some(vm) = json.pointer("/header/pageHeaderRenderer/content/pageHeaderViewModel") {
        if let Some(t) = vm.get("title").and_then(text) {
            ch.name = t;
        }
        let (verified, artist) = {
            let icons: Vec<&str> = find_key(vm.get("title").unwrap_or(&Value::Null), "imageName")
                .into_iter()
                .filter_map(Value::as_str)
                .collect();
            let artist = icons
                .iter()
                .any(|i| i.contains("AUDIO") || i.contains("MUSIC"));
            (artist || icons.iter().any(|i| i.contains("CHECK")), artist)
        };
        ch.verified = verified;
        ch.artist = artist;
        if let Some(a) = vm.get("image").and_then(best_thumb) {
            ch.avatar = Some(a);
        }
        let parts = metadata_parts(vm);
        let mut rest = Vec::new();
        for (t, a11y) in parts {
            if t.starts_with('@') {
                ch.handle = Some(t);
            } else {
                rest.push((t, a11y));
            }
        }
        let mut numbered = rest
            .into_iter()
            .filter(|(t, _)| t.chars().any(|c| c.is_ascii_digit()));
        ch.subscribers_text = numbered.next().map(|p| p.0);
        ch.video_count_text = numbered.next().map(|p| p.0);
        if let Some(d) = text_at(vm, "/description/descriptionPreviewViewModel/description") {
            ch.description = Some(d);
        }
        banner = vm.get("banner").map(thumbs).unwrap_or_default();
        ch.subscribed = subscribed_state(find_first(vm, "subscribeButtonViewModel"));
    } else if let Some(h) = json.pointer("/header/c4TabbedHeaderRenderer") {
        if let Some(id) = str_at(h, "/channelId") {
            ch.id = id.to_string();
        }
        if let Some(t) = h.get("title").and_then(text) {
            ch.name = t;
        }
        ch.avatar = h.get("avatar").and_then(best_thumb).or(ch.avatar);
        ch.handle = h.get("channelHandleText").and_then(text);
        ch.subscribers_text = h.get("subscriberCountText").and_then(text);
        ch.video_count_text = h.get("videosCountText").and_then(text);
        let (verified, artist) = owner_badges(h);
        ch.verified = verified;
        ch.artist = artist;
        banner = h.get("banner").map(thumbs).unwrap_or_default();
        ch.subscribed = subscribed_state(h.get("subscribeButton"));
    }

    let tabs: Vec<ChannelTab> = json
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("tabRenderer"))
        .filter(|t| t.get("title").is_some())
        .map(|t| ChannelTab {
            id: channel_tab_id(t),
            title: t.get("title").and_then(text).unwrap_or_default(),
            params: str_at(t, "/endpoint/browseEndpoint/params").map(str::to_string),
            selected: t.get("selected").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect();

    let page = selected_tab(json)
        .map(|c| {
            let grid = !find_key(c, "richGridRenderer").is_empty();
            page_from(
                c,
                if grid {
                    ShelfKind::Grid
                } else {
                    ShelfKind::Row
                },
            )
        })
        .unwrap_or_default();
    log_unknown("channel");
    ChannelPage {
        channel: ch,
        banner,
        tabs,
        page,
    }
}

pub fn resolve_url(json: &Value) -> Option<(String, Option<String>)> {
    let ep = json.get("endpoint")?;
    browse_of(ep).map(|(id, params, _)| (id.to_string(), params.map(str::to_string)))
}

pub fn playlist(json: &Value) -> PlaylistPage {
    let mut p = PlaylistPage::default();
    let md = json
        .pointer("/metadata/playlistMetadataRenderer")
        .unwrap_or(&Value::Null);
    p.title = str_at(md, "/title").unwrap_or("").to_string();
    p.description = str_at(md, "/description").map(str::to_string);

    if let Some(h) = json.pointer("/header/playlistHeaderRenderer") {
        p.id = str_at(h, "/playlistId").unwrap_or("").to_string();
        if let Some(t) = h.get("title").and_then(text) {
            p.title = t;
        }
        p.owner = h.get("ownerText").and_then(channel_from_runs);
        p.description = h.get("descriptionText").and_then(text).or(p.description);
        p.count_text = h.get("numVideosText").and_then(text);
        p.views_text = h.get("viewCountText").and_then(text);
        p.updated_text = h
            .get("byline")
            .and_then(Value::as_array)
            .and_then(|b| b.last())
            .and_then(|b| b.pointer("/playlistBylineRenderer/text"))
            .and_then(text);
        p.thumbs = h
            .get("playlistHeaderBanner")
            .map(thumbs)
            .unwrap_or_default();
        p.editable = h
            .get("isEditable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    } else if let Some(vm) = json.pointer("/header/pageHeaderRenderer/content/pageHeaderViewModel")
    {
        if let Some(t) = vm.get("title").and_then(text) {
            p.title = t;
        }
        if let Some(d) = text_at(vm, "/description/descriptionPreviewViewModel/description") {
            p.description = Some(d);
        }
        p.thumbs = vm.get("heroImage").map(thumbs).unwrap_or_default();
        for row in vm
            .pointer("/metadata/contentMetadataViewModel/metadataRows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for part in row
                .get("metadataParts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let t = part
                    .pointer("/avatarStack/avatarStackViewModel/text")
                    .or_else(|| part.get("text"));
                let Some(tv) = t else { continue };
                let link = find_key(tv, "browseEndpoint").into_iter().find(|b| {
                    b.get("browseId")
                        .and_then(Value::as_str)
                        .is_some_and(|i| i.starts_with("UC"))
                });
                if p.owner.is_none() && (link.is_some() || part.get("avatarStack").is_some()) {
                    let name = text(tv).unwrap_or_default();
                    let name = name
                        .strip_prefix("by ")
                        .or_else(|| name.strip_prefix("por "))
                        .unwrap_or(&name)
                        .to_string();
                    p.owner = Some(ChannelRef {
                        id: link
                            .and_then(|b| b.get("browseId"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        handle: link
                            .and_then(|b| b.get("canonicalBaseUrl"))
                            .and_then(Value::as_str)
                            .map(|u| u.trim_start_matches('/'))
                            .filter(|u| u.starts_with('@'))
                            .map(str::to_string),
                        name,
                        avatar: part.get("avatarStack").and_then(best_thumb),
                        ..Default::default()
                    });
                    continue;
                }
                let Some(s) = text(tv) else { continue };
                if !s.chars().any(|c| c.is_ascii_digit()) {
                    continue;
                }
                if p.count_text.is_none()
                    && !s.to_lowercase().contains("view")
                    && !s.contains("visualiza")
                {
                    p.count_text = Some(s);
                } else if p.views_text.is_none() {
                    p.views_text = Some(s);
                } else if p.updated_text.is_none() {
                    p.updated_text = Some(s);
                }
            }
        }
        p.editable = find_first(vm, "editPlaylistEndpoint").is_some()
            || find_key(vm, "iconName")
                .iter()
                .any(|n| n.as_str() == Some("EDIT"));
    }
    if let Some(side) = json.pointer("/sidebar/playlistSidebarRenderer") {
        if p.thumbs.is_empty() {
            p.thumbs = find_first(side, "playlistSidebarPrimaryInfoRenderer")
                .and_then(|r| r.get("thumbnailRenderer"))
                .map(thumbs)
                .unwrap_or_default();
        }
        if p.owner.is_none() {
            p.owner = find_first(side, "videoOwnerRenderer")
                .and_then(|o| o.get("title"))
                .and_then(channel_from_runs);
        }
    }

    let content = selected_tab(json).unwrap_or(&Value::Null);
    p.items = collect_items(content);
    p.continuation = find_continuation(content);
    if p.id.is_empty() {
        p.id = json
            .pointer("/microformat/microformatDataRenderer/urlCanonical")
            .and_then(Value::as_str)
            .and_then(|u| u.split("list=").nth(1))
            .map(|s| s.split('&').next().unwrap_or(s).to_string())
            .or_else(|| {
                find_key(content, "playlistId")
                    .into_iter()
                    .find_map(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
    }
    log_unknown("playlist");
    p
}

pub fn feed(json: &Value) -> Page {
    let Some(content) = selected_tab(json) else {
        return continuation(json);
    };
    let grid = !find_key(content, "richItemRenderer").is_empty();
    let mut page = page_from(
        content,
        if grid {
            ShelfKind::Grid
        } else {
            ShelfKind::List
        },
    );
    let title = json
        .pointer("/header/pageHeaderRenderer/pageTitle")
        .and_then(text)
        .or_else(|| {
            json.pointer("/header/feedTabbedHeaderRenderer/title")
                .and_then(text)
        })
        .or_else(|| {
            json.pointer("/header/pageHeaderRenderer/content/pageHeaderViewModel/title")
                .and_then(text)
        });
    if let Some(title) = title {
        page.header = Some(PageHeader {
            title,
            ..Default::default()
        });
    }
    log_unknown("feed");
    page
}

pub fn account_menu(json: &Value) -> Account {
    let Some(h) = find_first(json, "activeAccountHeaderRenderer") else {
        return Account::default();
    };
    let channel_id = find_key(json, "browseEndpoint")
        .into_iter()
        .filter_map(|b| b.get("browseId").and_then(Value::as_str))
        .find(|id| id.starts_with("UC"))
        .map(str::to_string);
    Account {
        logged_in: true,
        name: h.get("accountName").and_then(text),
        handle: h.get("channelHandle").and_then(text),
        avatar: h.get("accountPhoto").and_then(best_thumb),
        email: h.get("email").and_then(text),
        channel_id,
        premium: None,
        slug: None,
    }
}

pub fn suggestions(body: &str) -> Vec<String> {
    let (Some(start), Some(end)) = (body.find('['), body.rfind(']')) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&body[start..=end]) else {
        return Vec::new();
    };
    v.get(1)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().or_else(|| s.get(0).and_then(Value::as_str)))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_playlists(json: &Value) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = find_key(json, "playlistAddToOptionRenderer")
        .into_iter()
        .filter_map(|o| {
            Some((
                o.get("playlistId").and_then(Value::as_str)?.to_string(),
                o.get("title").and_then(text).unwrap_or_default(),
                o.get("containsSelectedVideos").and_then(Value::as_str) == Some("ALL"),
            ))
        })
        .collect();
    if out.is_empty() {
        for it in find_key(json, "listItemViewModel") {
            let Some(id) = find_key(it, "playlistId")
                .into_iter()
                .find_map(Value::as_str)
            else {
                continue;
            };
            let title = it.get("title").and_then(text).unwrap_or_default();
            let contains = find_key(it, "imageName")
                .iter()
                .any(|n| n.as_str() == Some("CHECK_BOX") || n.as_str() == Some("BOOKMARK"));
            out.push((id.to_string(), title, contains));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tube::walk::testutil::{external, fixture};
    use serde_json::json;

    fn videos(items: &[Item]) -> Vec<&VideoItem> {
        items
            .iter()
            .filter_map(|i| match i {
                Item::Video(v) => Some(v),
                _ => None,
            })
            .collect()
    }

    fn all_items(p: &Page) -> Vec<Item> {
        p.shelves.iter().flat_map(|s| s.items.clone()).collect()
    }

    #[test]
    fn anonymous_home_is_a_nudge() {
        for f in ["web_home", "web_home_pt"] {
            let p = home(&fixture(f));
            assert!(p.shelves.is_empty());
            assert!(!p.header.unwrap().title.is_empty());
        }
    }

    #[test]
    fn populated_home_from_rustypipe() {
        let Some(j) = external("ThetaDev__rustypipe/testfiles/trends/startpage.json") else {
            eprintln!("skip: rustypipe testfiles not present");
            return;
        };
        let p = home(&j);
        let items = all_items(&p);
        assert!(videos(&items).len() >= 10, "{}", items.len());
        assert!(p.continuation.is_some());
        let v = videos(&items)[0];
        assert!(v.channel.as_ref().unwrap().id.starts_with("UC"));
        assert!(v.duration_s.is_some());
        if let Some(c) = external("ThetaDev__rustypipe/testfiles/trends/startpage_cont.json") {
            let p = continuation(&c);
            assert!(!all_items(&p).is_empty());
            assert!(p.continuation.is_some());
        }
    }

    #[test]
    fn search_en() {
        let p = search(&fixture("web_search"));
        let items = all_items(&p);
        assert!(videos(&items).len() >= 5);
        assert!(p.continuation.is_some());
        assert!(!p.chips.is_empty());
        assert!(p.shelves.iter().any(|s| s.kind == ShelfKind::Shorts));
        assert_eq!(p.corrected_query, None);
        let v = videos(&items)[0];
        assert!(v.description_snippet.is_some());
        assert!(v.published_text.is_some());
    }

    #[test]
    fn search_filtered_and_localized() {
        let p = search(&fixture("web_search_videos"));
        assert!(videos(&all_items(&p)).len() >= 8);
        let pt = search(&fixture("web_search_pt"));
        let items = all_items(&pt);
        let vs = videos(&items);
        assert!(vs.len() >= 5);
        assert!(
            vs.iter().filter(|v| v.views.is_some()).count() >= vs.len() / 2,
            "{:?}",
            vs.iter().map(|v| &v.views_text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_corrected_query() {
        let p = search(&fixture("web_search_typo"));
        assert!(p.corrected_query.is_some(), "no corrected query");
    }

    #[test]
    fn search_channels_and_playlists() {
        let items = all_items(&search(&fixture("web_search_channels")));
        assert!(
            items
                .iter()
                .filter(|i| matches!(i, Item::Channel(_)))
                .count()
                >= 3
        );
        let items = all_items(&search(&fixture("web_search_playlists")));
        let pls: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Playlist(p) => Some(p),
                _ => None,
            })
            .collect();
        assert!(pls.len() >= 3);
        assert!(pls.iter().all(|p| !p.id.is_empty() && !p.title.is_empty()));
        assert!(pls.iter().any(|p| p.count_text.is_some()));
    }

    #[test]
    fn search_continuation() {
        let p = continuation(&fixture("web_search_cont"));
        assert!(videos(&all_items(&p)).len() >= 5);
        assert!(p.continuation.is_some());
    }

    #[test]
    fn watch_with_chapters() {
        let w = watch(&fixture("web_next_chapters"));
        assert_eq!(w.id, "aircAruvnKk");
        assert!(w.title.contains("neural network"));
        let ch = w.channel.as_ref().unwrap();
        assert_eq!(ch.id, "UCYO_jab_esuFRV4b17AJtAw");
        assert_eq!(ch.handle.as_deref(), Some("@3blue1brown"));
        assert!(ch.verified);
        assert!(ch.subscribers_text.is_some());
        assert_eq!(ch.subscribed, None);
        assert!(w.views.unwrap() > 1_000_000);
        assert!(w.date_text.is_some());
        assert!(w.published_text.is_some());
        assert!(w.likes.unwrap() > 1000);
        assert!(w.likes_text.is_some());
        assert!(w.chapters.len() >= 5, "{:?}", w.chapters);
        assert_eq!(w.chapters[0].start_s, 0);
        assert!(w.chapters.windows(2).all(|c| c[0].start_s < c[1].start_s));
        assert!(w.description.len() > 1);
        assert!(w.description.iter().any(|r| r
            .url
            .as_deref()
            .is_some_and(|u| u.starts_with("https://www.patreon.com"))));
        assert!(w.related.len() >= 5);
        assert!(w.related_continuation.is_some());
        assert!(w.comments_token.is_some());
        assert!(w.comments_count_text.is_some());
        assert!(w.playlist.is_none());
        assert_eq!(comment_sorts(&fixture("web_next_chapters")).len(), 2);
    }

    #[test]
    fn watch_in_playlist() {
        let w = watch(&fixture("web_next_playlist"));
        let pl = w.playlist.unwrap();
        assert_eq!(pl.id, "PLZHQObOWTQDNU6R1_67000Dx_ZCJB-3pi");
        assert_eq!(pl.index, 1);
        assert_eq!(pl.total, Some(10));
        assert!(pl.items.len() >= 5);
        assert!(!pl.is_mix);
        assert_eq!(pl.owner.as_deref(), Some("3Blue1Brown"));
    }

    #[test]
    fn watch_pt() {
        let w = watch(&fixture("web_next_pt"));
        assert!(w.views.unwrap() > 1_000_000, "{:?}", w.views_text);
        assert!(w.likes.unwrap() > 1000);
        assert!(!w.chapters.is_empty());
    }

    #[test]
    fn watch_details_from_rustypipe() {
        for f in [
            "video_details_chapters",
            "video_details_live",
            "video_details_20231103_likes",
        ] {
            let Some(j) = external(&format!(
                "ThetaDev__rustypipe/testfiles/video_details/{f}.json"
            )) else {
                eprintln!("skip: rustypipe testfiles not present");
                return;
            };
            let w = watch(&j);
            assert!(!w.title.is_empty(), "{f}");
            assert!(w.channel.is_some(), "{f}");
            assert!(!w.related.is_empty(), "{f}");
            if f.contains("chapters") {
                assert!(!w.chapters.is_empty());
            }
            if f.contains("live") {
                assert!(w.is_live);
            }
        }
    }

    #[test]
    fn related_continuation() {
        let p = continuation(&fixture("web_related_cont"));
        assert!(videos(&all_items(&p)).len() >= 5);
    }

    #[test]
    fn comments_view_models() {
        let c = comments(&fixture("web_comments"));
        assert!(c.items.len() >= 10);
        assert!(c.continuation.is_some());
        let first = &c.items[0];
        assert!(!first.id.is_empty());
        assert!(first.author.id.starts_with("UC"));
        assert!(first.author.handle.as_deref().unwrap().starts_with('@'));
        assert!(!first.text.is_empty());
        assert!(first.published_text.is_some());
        assert!(first.likes_text.is_some());
        assert!(c
            .items
            .iter()
            .any(|c| c.reply_count > 0 && c.replies_token.is_some()));
        assert!(c.items.iter().any(|c| c.hearted));
        let replies = comments(&fixture("web_comments_replies"));
        assert!(replies.items.len() >= 5);
        assert!(replies.items.iter().all(|r| r.replies_token.is_none()));
    }

    #[test]
    fn comments_from_rustypipe() {
        for f in [
            "comments_top",
            "comments_latest",
            "comments_20240401_frameworkupd",
        ] {
            let Some(j) = external(&format!(
                "ThetaDev__rustypipe/testfiles/video_details/{f}.json"
            )) else {
                eprintln!("skip: rustypipe testfiles not present");
                return;
            };
            let c = comments(&j);
            assert!(c.items.len() >= 10, "{f}: {}", c.items.len());
            assert!(c.items.iter().all(|c| !c.author.name.is_empty()), "{f}");
        }
    }

    #[test]
    fn channel_videos_tab() {
        let c = channel(&fixture("web_channel_videos"));
        assert_eq!(c.channel.id, "UCYO_jab_esuFRV4b17AJtAw");
        assert_eq!(c.channel.name, "3Blue1Brown");
        assert_eq!(c.channel.handle.as_deref(), Some("@3blue1brown"));
        assert!(c
            .channel
            .subscribers_text
            .as_deref()
            .unwrap()
            .contains("subscribers"));
        assert!(c.channel.video_count_text.is_some());
        assert!(c.channel.avatar.is_some());
        assert!(c.channel.description.is_some());
        assert!(!c.banner.is_empty());
        let ids: Vec<&str> = c.tabs.iter().map(|t| t.id.as_str()).collect();
        assert!(
            ids.contains(&"home")
                && ids.contains(&"videos")
                && ids.contains(&"shorts")
                && ids.contains(&"playlists"),
            "{ids:?}"
        );
        assert!(c
            .tabs
            .iter()
            .any(|t| t.id == "videos" && t.selected && t.params.is_some()));
        assert!(c.page.chips.len() >= 3);
        assert!(videos(&all_items(&c.page)).len() >= 8);
        assert!(c.page.continuation.is_some());
    }

    #[test]
    fn channel_home_handle_and_shorts() {
        let c = channel(&fixture("web_channel_home"));
        assert!(c.page.shelves.len() >= 2);
        assert!(c.page.shelves.iter().any(|s| s.title.is_some()));
        let h = channel(&fixture("web_channel_handle"));
        assert_eq!(h.channel.id, "UCXuqSBlHAE6Xw-yeJA0Tunw");
        assert!(h.channel.verified);
        let s = channel(&fixture("web_channel_shorts"));
        assert!(
            all_items(&s.page)
                .iter()
                .filter(|i| matches!(i, Item::Short(_)))
                .count()
                >= 5
        );
        let pl = channel(&fixture("web_channel_playlists"));
        assert!(
            all_items(&pl.page)
                .iter()
                .filter(|i| matches!(i, Item::Playlist(_)))
                .count()
                >= 5
        );
        let pt = channel(&fixture("web_channel_pt"));
        assert!(pt.channel.subscribers_text.is_some());
        assert!(videos(&all_items(&pt.page))
            .iter()
            .any(|v| v.views.is_some()));
        let cont = continuation(&fixture("web_channel_videos_cont"));
        assert!(!all_items(&cont).is_empty());
    }

    #[test]
    fn resolve_handle() {
        let (id, _) = resolve_url(&fixture("web_resolve_handle")).unwrap();
        assert_eq!(id, "UCXuqSBlHAE6Xw-yeJA0Tunw");
    }

    #[test]
    fn channel_from_rustypipe_old_header() {
        let Some(j) = external("ThetaDev__rustypipe/testfiles/channel/channel_videos_base.json")
        else {
            eprintln!("skip: rustypipe testfiles not present");
            return;
        };
        let c = channel(&j);
        assert!(!c.channel.name.is_empty());
        assert!(!c.channel.id.is_empty());
        assert!(!all_items(&c.page).is_empty());
    }

    #[test]
    fn playlist_page() {
        let p = playlist(&fixture("web_playlist"));
        assert_eq!(p.id, "PLZHQObOWTQDNU6R1_67000Dx_ZCJB-3pi");
        assert_eq!(p.title, "Neural networks");
        assert_eq!(p.owner.as_ref().unwrap().id, "UCYO_jab_esuFRV4b17AJtAw");
        assert!(p.count_text.is_some());
        assert!(p.views_text.is_some());
        assert!(!p.thumbs.is_empty());
        assert!(p.items.len() >= 8);
        assert!(!p.editable);

        let l = playlist(&fixture("web_playlist_long"));
        assert_eq!(l.title, "Google Search Stories");
        let owner = l.owner.unwrap();
        assert_eq!(owner.name, "Google Search Stories");
        assert!(owner.id.starts_with("UC"));
        assert!(l.count_text.as_deref().unwrap().contains("videos"));
        assert!(l.views_text.as_deref().unwrap().contains("views"));
        assert!(l.continuation.is_some());
        assert!(!l.items.is_empty());
    }

    #[test]
    fn playlist_from_rustypipe() {
        for f in [
            "playlist_short",
            "playlist_20241011_pageheader",
            "playlist_20250316_cmdexecutor",
        ] {
            let Some(j) = external(&format!("ThetaDev__rustypipe/testfiles/playlist/{f}.json"))
            else {
                eprintln!("skip: rustypipe testfiles not present");
                return;
            };
            let p = playlist(&j);
            assert!(!p.title.is_empty(), "{f}");
            assert!(!p.items.is_empty(), "{f}");
            assert!(!p.id.is_empty(), "{f}");
        }
    }

    #[test]
    fn logged_in_feeds_from_rustypipe() {
        let files = [
            "userdata/subscription_feed.json",
            "userdata/history.json",
            "userdata/subscriptions.json",
        ];
        for f in files {
            let Some(j) = external(&format!("ThetaDev__rustypipe/testfiles/{f}")) else {
                eprintln!("skip: rustypipe testfiles not present");
                return;
            };
            let p = feed(&j);
            let items = all_items(&p);
            assert!(!items.is_empty(), "{f}");
            if f.contains("history") {
                assert!(
                    p.shelves.iter().any(|s| s.title.is_some()),
                    "history groups by day"
                );
            }
            if f.contains("subscriptions.json") {
                assert!(items.iter().any(|i| matches!(i, Item::Channel(_))));
            }
        }
    }

    #[test]
    fn account() {
        let j = json!({"actions": [{"openPopupAction": {"popup": {"multiPageMenuRenderer": {
            "header": {"activeAccountHeaderRenderer": {
                "accountName": {"simpleText": "Ana"},
                "email": {"simpleText": "ana@example.com"},
                "channelHandle": {"simpleText": "@ana"},
                "accountPhoto": {"thumbnails": [{"url": "https://yt3.ggpht.com/a=s88", "width": 88, "height": 88}]}
            }},
            "sections": [{"multiPageMenuSectionRenderer": {"items": [{"compactLinkRenderer": {
                "navigationEndpoint": {"browseEndpoint": {"browseId": "UCabcdefghijklmnopqrstuv"}}}}]}}]
        }}}}]});
        let a = account_menu(&j);
        assert!(a.logged_in);
        assert_eq!(a.name.as_deref(), Some("Ana"));
        assert_eq!(a.handle.as_deref(), Some("@ana"));
        assert_eq!(a.channel_id.as_deref(), Some("UCabcdefghijklmnopqrstuv"));
        assert!(!account_menu(&json!({})).logged_in);
    }

    #[test]
    fn suggestion_formats() {
        let jsonp = r#"window.google.ac.h(["rust lang",[["rust language",0,[512,433,131]],["rust language programming",0,[512]]],{"k":1}])"#;
        assert_eq!(
            suggestions(jsonp),
            vec!["rust language", "rust language programming"]
        );
        let plain = r#"["rust lang",["rust language","rust lang"],[],{}]"#;
        assert_eq!(suggestions(plain), vec!["rust language", "rust lang"]);
        assert!(suggestions("garbage").is_empty());
    }

    #[test]
    fn add_to_playlist() {
        let j = json!({"contents": [{"addToPlaylistRenderer": {"playlists": [
            {"playlistAddToOptionRenderer": {"playlistId": "WL", "title": {"simpleText": "Watch later"}, "containsSelectedVideos": "NONE"}},
            {"playlistAddToOptionRenderer": {"playlistId": "PLx", "title": {"simpleText": "Mine"}, "containsSelectedVideos": "ALL"}}
        ]}}]});
        assert_eq!(
            save_playlists(&j),
            vec![
                ("WL".into(), "Watch later".into(), false),
                ("PLx".into(), "Mine".into(), true)
            ]
        );
    }
}
