//! Renderer walker shared by the YouTube and YouTube Music parsers.
//!
//! Innertube responses change shape through A/B tests, so nothing here
//! deserializes into fixed structs: every renderer is read field by field
//! from a `serde_json::Value`, missing fields fall back to `None`, and an
//! unknown renderer is skipped (and counted) instead of failing the page.

use std::cell::RefCell;
use std::collections::BTreeMap;

use serde_json::Value;

use super::model::*;

const YT: &str = "https://www.youtube.com";

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// `simpleText`, `runs[].text` or the `content` of a view model.
pub fn text(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            if let Some(s) = o.get("simpleText").and_then(Value::as_str) {
                s.to_string()
            } else if let Some(runs) = o.get("runs").and_then(Value::as_array) {
                runs.iter()
                    .filter_map(|r| r.get("text").and_then(Value::as_str))
                    .collect::<String>()
            } else if let Some(s) = o.get("content").and_then(Value::as_str) {
                s.to_string()
            } else if let Some(t) = o.get("dynamicTextViewModel") {
                return t.get("text").and_then(text);
            } else {
                let t = o.get("text")?;
                return text(t);
            }
        }
        _ => return None,
    };
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Text at a JSON pointer (`/title`, `/header/0/title`...).
pub(crate) fn text_at(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr).and_then(text)
}

pub(crate) fn str_at<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Formatted text with links. Handles classic `runs` (with
/// `navigationEndpoint`) and view-model `content` + `commandRuns`, whose
/// indices count UTF-16 code units.
pub fn runs(v: &Value) -> Vec<TextRun> {
    if let Some(rs) = v.get("runs").and_then(Value::as_array) {
        return rs
            .iter()
            .filter_map(|r| {
                let t = r.get("text").and_then(Value::as_str)?;
                let mut run = TextRun {
                    text: t.to_string(),
                    bold: r.get("bold").and_then(Value::as_bool).unwrap_or(false),
                    ..Default::default()
                };
                if let Some(ep) = r.get("navigationEndpoint") {
                    apply_endpoint(&mut run, ep);
                }
                Some(run)
            })
            .collect();
    }
    if let Some(content) = v.get("content").and_then(Value::as_str) {
        let units: Vec<u16> = content.encode_utf16().collect();
        let mut cmds: Vec<(usize, usize, &Value)> = v
            .get("commandRuns")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        let start =
                            c.get("startIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
                        let len = c.get("length").and_then(Value::as_u64)? as usize;
                        Some((start, len, c))
                    })
                    .collect()
            })
            .unwrap_or_default();
        cmds.sort_by_key(|c| c.0);
        let mut out = Vec::new();
        let mut pos = 0usize;
        for (start, len, cmd) in cmds {
            let start = start.min(units.len());
            let end = (start + len).min(units.len());
            if start < pos || end <= start {
                continue;
            }
            if start > pos {
                out.push(TextRun {
                    text: String::from_utf16_lossy(&units[pos..start]),
                    ..Default::default()
                });
            }
            let mut run = TextRun {
                text: String::from_utf16_lossy(&units[start..end]),
                ..Default::default()
            };
            if let Some(tap) = cmd.get("onTap") {
                apply_endpoint(&mut run, tap);
            }
            out.push(run);
            pos = end;
        }
        if pos < units.len() {
            out.push(TextRun {
                text: String::from_utf16_lossy(&units[pos..]),
                ..Default::default()
            });
        }
        return out;
    }
    if let Some(t) = v.get("simpleText").and_then(Value::as_str) {
        return vec![TextRun {
            text: t.to_string(),
            ..Default::default()
        }];
    }
    Vec::new()
}

/// Unwraps `onTap` / `innertubeCommand` / `command` containers down to the
/// object that holds the actual `*Endpoint`.
pub(crate) fn endpoint(v: &Value) -> &Value {
    let mut cur = v;
    for _ in 0..4 {
        if let Some(n) = cur
            .get("onTap")
            .or_else(|| cur.get("innertubeCommand"))
            .or_else(|| cur.get("command"))
        {
            cur = n;
        } else {
            break;
        }
    }
    cur
}

fn apply_endpoint(run: &mut TextRun, ep: &Value) {
    let ep = endpoint(ep);
    let meta_url = str_at(ep, "/commandMetadata/webCommandMetadata/url").map(absolute_url);
    if let Some(w) = ep
        .get("watchEndpoint")
        .or_else(|| ep.get("reelWatchEndpoint"))
    {
        run.video_id = w.get("videoId").and_then(Value::as_str).map(str::to_string);
        run.t = w
            .get("startTimeSeconds")
            .and_then(Value::as_u64)
            .map(|t| t as u32);
        run.url = meta_url;
    } else if let Some(b) = ep.get("browseEndpoint") {
        let id = b.get("browseId").and_then(Value::as_str).unwrap_or("");
        if id.starts_with("UC") {
            run.channel_id = Some(id.to_string());
        }
        run.url = b
            .get("canonicalBaseUrl")
            .and_then(Value::as_str)
            .map(absolute_url)
            .or(meta_url);
    } else if let Some(u) = str_at(ep, "/urlEndpoint/url") {
        run.url = Some(unwrap_redirect(u));
    } else {
        run.url = meta_url;
    }
}

fn absolute_url(u: &str) -> String {
    if u.starts_with('/') {
        format!("{YT}{u}")
    } else {
        u.to_string()
    }
}

/// `youtube.com/redirect?...&q=<target>` -> target.
fn unwrap_redirect(u: &str) -> String {
    let abs = absolute_url(u);
    if abs.contains("youtube.com/redirect") {
        if let Ok(parsed) = url::Url::parse(&abs) {
            if let Some((_, q)) = parsed.query_pairs().find(|(k, _)| k == "q") {
                return q.into_owned();
            }
        }
    }
    abs
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

const THUMB_WRAPPERS: &[&str] = &[
    "thumbnail",
    "image",
    "musicThumbnailRenderer",
    "thumbnailViewModel",
    "croppedSquareThumbnailRenderer",
    "decoratedAvatarViewModel",
    "avatar",
    "avatarViewModel",
    "collectionThumbnailViewModel",
    "primaryThumbnail",
    "thumbnailRenderer",
    "playlistVideoThumbnailRenderer",
    "playlistCustomThumbnailRenderer",
    "contentPreviewImageViewModel",
    "imageBannerViewModel",
    "backgroundImageConfig",
    "foregroundThumbnail",
    "channelThumbnailWithLinkRenderer",
    "musicAnimatedThumbnailRenderer",
    "backupRenderer",
];

/// `thumbnails[]`, `sources[]` or any of the usual wrappers around them.
/// Protocol-relative and `http:` URLs become `https:`.
pub fn thumbs(v: &Value) -> Vec<Thumb> {
    fn list(v: &Value, depth: u8) -> Option<&Vec<Value>> {
        if depth > 6 {
            return None;
        }
        if let Some(a) = v.as_array() {
            if a.iter().any(|t| t.get("url").is_some()) {
                return Some(a);
            }
            return None;
        }
        for k in ["thumbnails", "sources"] {
            if let Some(a) = v.get(k).and_then(Value::as_array) {
                if a.iter().any(|t| t.get("url").is_some()) {
                    return Some(a);
                }
            }
        }
        for k in THUMB_WRAPPERS {
            if let Some(n) = v.get(*k) {
                if let Some(a) = list(n, depth + 1) {
                    return Some(a);
                }
            }
        }
        // `thumbnails: [{thumbnails: [...]}]` (playlistRenderer)
        if let Some(first) = v.get("thumbnails").and_then(|a| a.get(0)) {
            return list(first, depth + 1);
        }
        None
    }
    list(v, 0)
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let url = t.get("url").and_then(Value::as_str)?;
                    Some(Thumb {
                        url: https(url),
                        width: t.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
                        height: t.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn https(u: &str) -> String {
    if let Some(rest) = u.strip_prefix("//") {
        format!("https://{rest}")
    } else if let Some(rest) = u.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        u.to_string()
    }
}

/// Largest image URL, for single-avatar fields.
pub(crate) fn best_thumb(v: &Value) -> Option<String> {
    thumbs(v).into_iter().max_by_key(|t| t.width).map(|t| t.url)
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

/// `"1:02:03"` -> 3723. Rejects anything that isn't `[[h:]m:]s` digits.
pub fn parse_duration(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total: u32 = 0;
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    for p in parts {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(p.parse::<u32>().ok()?)?;
    }
    Some(total)
}

/// Counts as the service writes them: `"1.2M views"`, `"1,234 views"`,
/// `"1,2 mi de visualizações"`, `"12 mil"`, `"8.66 million subscribers"`,
/// `"1,2 M de visualizaciones"`. Without a multiplier every separator is a
/// thousands separator; with one, a trailing `.`/`,` group of 1-2 digits is
/// the decimal part.
pub fn parse_count(s: &str) -> Option<u64> {
    let chars: Vec<char> = s.chars().collect();
    let start = chars.iter().position(|c| c.is_ascii_digit())?;
    let mut end = start;
    while end < chars.len() {
        let c = chars[end];
        if c.is_ascii_digit() {
            end += 1;
            continue;
        }
        let next_digit = chars.get(end + 1).is_some_and(|n| n.is_ascii_digit());
        if (c == '.' || c == ',' || c == '\'') && next_digit {
            end += 1;
            continue;
        }
        // "1 234 567" (fr/ru groups with a (narrow) no-break space)
        if matches!(c, '\u{a0}' | '\u{202f}' | ' ') && next_digit {
            let group = chars[end + 1..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();
            if group == 3 {
                end += 1;
                continue;
            }
        }
        break;
    }
    let num: String = chars[start..end]
        .iter()
        .filter(|c| !matches!(c, '\u{a0}' | '\u{202f}' | ' ' | '\''))
        .collect();

    let rest: String = chars[end..].iter().collect();
    let mut words = rest
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase);
    // Only a word glued to the number or right after one space counts.
    let gap = chars[end..]
        .iter()
        .take_while(|c| c.is_whitespace() || **c == '\u{a0}')
        .count();
    let word = if gap <= 1 { words.next() } else { None };
    let mult: f64 = match word.as_deref() {
        Some("k" | "thousand" | "tsd" | "tys" | "rb" | "b\u{0131}n") => 1e3,
        Some("mil") => {
            // es: "1,2 mil M" / "mil millones" = billions
            if words
                .next()
                .is_some_and(|w| w == "m" || w.starts_with("mill"))
            {
                1e9
            } else {
                1e3
            }
        }
        Some(
            "m" | "mi" | "mn" | "mio" | "mln" | "million" | "millions" | "mill" | "millón"
            | "millones" | "milhão" | "milhões" | "millionen",
        ) => 1e6,
        Some(
            "b" | "bn" | "bi" | "mrd" | "mld" | "billion" | "billions" | "bilhão" | "bilhões"
            | "milliarden",
        ) => 1e9,
        _ => 1.0,
    };
    if mult == 1.0 {
        return num
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .ok();
    }
    let last_sep = num.rfind(['.', ',']);
    let normalized = match last_sep {
        Some(i) if num.len() - i - 1 <= 2 => {
            let int: String = num[..i].chars().filter(char::is_ascii_digit).collect();
            format!("{int}.{}", &num[i + 1..])
        }
        _ => num.chars().filter(char::is_ascii_digit).collect(),
    };
    let n: f64 = normalized.parse().ok()?;
    Some((n * mult).round() as u64)
}

// ---------------------------------------------------------------------------
// Tree search
// ---------------------------------------------------------------------------

/// Every value stored under `key`, depth first, in document order.
pub fn find_key<'a>(v: &'a Value, key: &str) -> Vec<&'a Value> {
    fn rec<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
        match v {
            Value::Object(o) => {
                for (k, val) in o {
                    if k == key {
                        out.push(val);
                    }
                    rec(val, key, out);
                }
            }
            Value::Array(a) => {
                for i in a {
                    rec(i, key, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    rec(v, key, &mut out);
    out
}

pub(crate) fn find_first<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    fn rec<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(o) => {
                if let Some(x) = o.get(key) {
                    return Some(x);
                }
                o.values().find_map(|val| rec(val, key))
            }
            Value::Array(a) => a.iter().find_map(|i| rec(i, key)),
            _ => None,
        }
    }
    rec(v, key)
}

/// Token of a `continuationItemRenderer` / `nextContinuationData` /
/// `nextRadioContinuationData`. The shallowest one wins (the page's own
/// continuation, not a comment's replies), and the last among equals.
pub fn find_continuation(v: &Value) -> Option<String> {
    fn token_of(k: &str, val: &Value) -> Option<String> {
        match k {
            "continuationItemRenderer" => {
                str_at(val, "/continuationEndpoint/continuationCommand/token")
                    .or_else(|| {
                        str_at(
                            val,
                            "/button/buttonRenderer/command/continuationCommand/token",
                        )
                    })
                    .or_else(|| {
                        find_first(val, "continuationCommand")
                            .and_then(|c| c.get("token"))
                            .and_then(Value::as_str)
                    })
                    .map(str::to_string)
            }
            "nextContinuationData" | "nextRadioContinuationData" | "reloadContinuationData" => val
                .get("continuation")
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        }
    }
    fn rec(v: &Value, depth: usize, best: &mut Option<(usize, String)>) {
        match v {
            Value::Object(o) => {
                for (k, val) in o {
                    if k == "replies" || k == "commentRepliesRenderer" || k == "menu" {
                        continue;
                    }
                    if let Some(t) = token_of(k, val) {
                        if best.as_ref().is_none_or(|(d, _)| depth <= *d) {
                            *best = Some((depth, t));
                        }
                        continue;
                    }
                    rec(val, depth + 1, best);
                }
            }
            Value::Array(a) => {
                for i in a {
                    rec(i, depth + 1, best);
                }
            }
            _ => {}
        }
    }
    let mut best = None;
    rec(v, 0, &mut best);
    best.map(|(_, t)| t)
}

// ---------------------------------------------------------------------------
// Unknown renderer bookkeeping
// ---------------------------------------------------------------------------

thread_local! {
    static UNKNOWN: RefCell<BTreeMap<String, u32>> = const { RefCell::new(BTreeMap::new()) };
}

fn note_unknown(name: &str) {
    UNKNOWN.with(|u| *u.borrow_mut().entry(name.to_string()).or_default() += 1);
}

/// Renderers the walker met but doesn't know, since the last call.
pub fn take_unknown() -> Vec<(String, u32)> {
    UNKNOWN.with(|u| std::mem::take(&mut *u.borrow_mut()).into_iter().collect())
}

/// Logs (debug) and clears the unknown renderers of the parse that just ran.
pub(crate) fn log_unknown(page: &str) {
    let unknown = take_unknown();
    if !unknown.is_empty() {
        tracing::debug!(page, ?unknown, "tube: unknown renderers skipped");
    }
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// `(browseId, params, pageType)` of a browse endpoint anywhere in `ep`'s
/// command wrappers.
pub(crate) fn browse_of(ep: &Value) -> Option<(&str, Option<&str>, Option<&str>)> {
    let ep = endpoint(ep);
    let b = ep.get("browseEndpoint")?;
    let id = b.get("browseId").and_then(Value::as_str)?;
    let params = b.get("params").and_then(Value::as_str);
    let page_type = str_at(
        b,
        "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
    );
    Some((id, params, page_type))
}

pub(crate) fn nav_of(ep: &Value) -> Option<Nav> {
    browse_of(ep).map(|(id, params, _)| Nav {
        browse_id: id.to_string(),
        params: params.map(str::to_string),
    })
}

fn watch_of(ep: &Value) -> Option<&Value> {
    let ep = endpoint(ep);
    ep.get("watchEndpoint")
        .or_else(|| ep.get("reelWatchEndpoint"))
        .or_else(|| ep.get("watchPlaylistEndpoint"))
}

fn music_video_type(ep: &Value) -> Option<&str> {
    str_at(
        endpoint(ep),
        "/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
    )
}

fn handle_from_url(u: &str) -> Option<String> {
    u.trim_start_matches(YT)
        .trim_start_matches('/')
        .split('/')
        .next()
        .filter(|s| s.starts_with('@'))
        .map(str::to_string)
}

/// First run that browses to a channel: `ownerText`, `shortBylineText`...
pub(crate) fn channel_from_runs(v: &Value) -> Option<ChannelRef> {
    let rs = v.get("runs").and_then(Value::as_array)?;
    for r in rs {
        let Some(ep) = r.get("navigationEndpoint") else {
            continue;
        };
        if let Some((id, _, _)) = browse_of(ep) {
            if id.starts_with("UC") {
                let handle = str_at(endpoint(ep), "/browseEndpoint/canonicalBaseUrl")
                    .and_then(handle_from_url);
                return Some(ChannelRef {
                    id: id.to_string(),
                    name: r
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    handle,
                    ..Default::default()
                });
            }
        }
    }
    // A byline without a link still names the channel.
    let name = text(v)?;
    Some(ChannelRef {
        name,
        ..Default::default()
    })
}

fn badge_styles(v: &Value) -> Vec<String> {
    find_key(v, "metadataBadgeRenderer")
        .into_iter()
        .filter_map(|b| b.get("style").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn owner_flags(r: &Value) -> (bool, bool) {
    let mut verified = false;
    let mut artist = false;
    for key in ["ownerBadges", "badges"] {
        if let Some(b) = r.get(key) {
            for s in badge_styles(b) {
                match s.as_str() {
                    "BADGE_STYLE_TYPE_VERIFIED" => verified = true,
                    "BADGE_STYLE_TYPE_VERIFIED_ARTIST" => {
                        verified = true;
                        artist = true
                    }
                    _ => {}
                }
            }
        }
    }
    (verified, artist)
}

/// Verified / artist check from a view model's `attachmentRuns` icons.
fn attachment_flags(v: &Value) -> (bool, bool) {
    let mut verified = false;
    let mut artist = false;
    for name in find_key(v, "imageName") {
        match name.as_str() {
            Some("CHECK_CIRCLE_FILLED" | "CHECK_CIRCLE_THICK" | "CHECK") => verified = true,
            Some("AUDIO_BADGE" | "MUSIC_FILLED" | "OFFICIAL_ARTIST_BADGE") => {
                verified = true;
                artist = true
            }
            _ => {}
        }
    }
    (verified, artist)
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// Renderers that are never shown: ads, merch, promos. No item, no log.
const JUNK: &[&str] = &[
    "adSlotRenderer",
    "inFeedAdLayoutRenderer",
    "promotedSparklesWebRenderer",
    "promotedSparklesTextSearchRenderer",
    "promotedVideoRenderer",
    "searchPyvRenderer",
    "statementBannerRenderer",
    "bannerPromoRenderer",
    "brandVideoSingletonRenderer",
    "brandVideoShelfRenderer",
    "merchandiseShelfRenderer",
    "ticketShelfRenderer",
    "offerModuleRenderer",
    "donationShelfRenderer",
    "inlineSurveyRenderer",
    "compactPromotedVideoRenderer",
    "primetimePromoRenderer",
    "mealbarPromoRenderer",
    "movingThumbnailRenderer",
    "feedNudgeRenderer",
    "continuationItemRenderer",
    "messageRenderer",
    "musicTastebuilderShelfRenderer",
    "backstagePostThreadRenderer",
    "postRenderer",
    "sharedPostRenderer",
    "miniGameCardViewModel",
    "searchRefinementCardRenderer",
    "clarificationRenderer",
    "infoPanelContainerRenderer",
    "showingResultsForRenderer",
    "didYouMeanRenderer",
    "includingResultsForRenderer",
    "emergencyOneboxRenderer",
    "musicShelfDividerRenderer",
    "musicDescriptionShelfRenderer",
    "musicSideAlignedItemRenderer",
    "chipCloudRenderer",
    "chipCloudChipRenderer",
    "feedFilterChipBarRenderer",
    "chipBarViewModel",
    "sortFilterSubMenuRenderer",
    "macroMarkersListItemRenderer",
    "compactInfocardRenderer",
    "commentThreadRenderer",
    "commentsHeaderRenderer",
    "commentsEntryPointHeaderRenderer",
    "videoPrimaryInfoRenderer",
    "videoSecondaryInfoRenderer",
    "compositeVideoPrimaryInfoRenderer",
    "engagementPanelSectionListRenderer",
    "channelVideoPlayerRenderer",
    "channelFeaturedContentRenderer",
    "recognitionShelfRenderer",
    "channelAboutFullMetadataRenderer",
    "aboutChannelViewModel",
    "buttonRenderer",
    "buttonViewModel",
    "menuRenderer",
    "toggleButtonRenderer",
    "musicResponsiveHeaderRenderer",
    "musicImmersiveHeaderRenderer",
    "musicVisualHeaderRenderer",
    "musicDetailHeaderRenderer",
    "musicEditablePlaylistDetailHeaderRenderer",
    "musicHeaderRenderer",
    "playlistSidebarRenderer",
    "playlistHeaderRenderer",
    "pageHeaderRenderer",
    "c4TabbedHeaderRenderer",
    "feedTabbedHeaderRenderer",
    "searchSubMenuRenderer",
    "searchHeaderRenderer",
    "itemSectionHeaderRenderer",
    "sectionHeaderViewModel",
    "richListHeaderRenderer",
    "gridHeaderRenderer",
    "musicCarouselShelfBasicHeaderRenderer",
    "musicCardShelfHeaderBasicRenderer",
    "horizontalMovieListRenderer",
    "automixPreviewVideoRenderer",
    "musicItemThumbnailOverlayRenderer",
    "thumbnailOverlayTimeStatusRenderer",
];

/// Structural renderers the walker goes through without a shelf of their
/// own (unless they carry a header title, see `titled_shelf`).
const TRANSPARENT: &[&str] = &[
    "twoColumnBrowseResultsRenderer",
    "singleColumnBrowseResultsRenderer",
    "twoColumnSearchResultsRenderer",
    "tabbedSearchResultsRenderer",
    "twoColumnWatchNextResults",
    "tabRenderer",
    "expandableTabRenderer",
    "sectionListRenderer",
    "itemSectionRenderer",
    "richGridRenderer",
    "richSectionRenderer",
    "gridRenderer",
    "playlistVideoListRenderer",
    "secondarySearchContainerRenderer",
    "verticalListRenderer",
    "horizontalListRenderer",
    "expandedShelfContentsRenderer",
    "playlistPanelRenderer",
    "musicQueueRenderer",
    "playlistPanelVideoWrapperRenderer",
    "richMetadataRowRenderer",
    "appendContinuationItemsAction",
    "reloadContinuationItemsCommand",
];

/// Plain (non-renderer) keys that lead to items.
const DESCEND_KEYS: &[&str] = &[
    "contents",
    "content",
    "items",
    "tabs",
    "results",
    "continuationItems",
    "continuationContents",
    "onResponseReceivedActions",
    "onResponseReceivedCommands",
    "onResponseReceivedEndpoints",
    "primaryContents",
    "secondaryContents",
    "secondaryResults",
    "gridContinuation",
    "sectionListContinuation",
    "itemSectionContinuation",
    "richGridContinuation",
    "musicShelfContinuation",
    "musicPlaylistShelfContinuation",
    "playlistVideoListContinuation",
    "playlistPanelContinuation",
    "primaryRenderer",
    "listItems",
    "cards",
    "videos",
    "expandedContent",
];

/// Keys never worth descending into.
const SKIP_KEYS: &[&str] = &[
    "menu",
    "frameworkUpdates",
    "responseContext",
    "topbar",
    "microformat",
    "trackingParams",
    "loggingDirectives",
    "thumbnailOverlays",
    "navigationEndpoint",
    "serviceEndpoint",
    "inlinePlaybackEndpoint",
    "richThumbnail",
    "expandableMetadata",
    "subscribeButton",
    "header",
];

/// One renderer wrapper (`{"videoRenderer": {...}}`) -> `Item`.
pub fn item(v: &Value) -> Option<Item> {
    let o = v.as_object()?;
    o.iter().find_map(|(k, inner)| {
        if k == "richItemRenderer" {
            return inner.get("content").and_then(item);
        }
        item_from(k, inner)
    })
}

fn item_from(k: &str, r: &Value) -> Option<Item> {
    match k {
        "videoRenderer"
        | "compactVideoRenderer"
        | "gridVideoRenderer"
        | "playlistVideoRenderer"
        | "videoWithContextRenderer"
        | "endScreenVideoRenderer"
        | "movieRenderer"
        | "compactMovieRenderer"
        | "gridMovieRenderer" => video_like(k, r),
        "playlistPanelVideoRenderer" => {
            if music_video_type(r.get("navigationEndpoint").unwrap_or(&Value::Null)).is_some() {
                Some(Item::Song(song_from_panel(r)?))
            } else {
                video_like(k, r)
            }
        }
        "reelItemRenderer" => {
            let id = r.get("videoId").and_then(Value::as_str)?;
            Some(Item::Short(ShortItem {
                id: id.to_string(),
                title: r.get("headline").and_then(text).unwrap_or_default(),
                thumbs: thumbs(r.get("thumbnail").unwrap_or(&Value::Null)),
                views_text: r.get("viewCountText").and_then(text),
                channel: None,
            }))
        }
        "shortsLockupViewModel" | "shortsLockupViewModelV2" => shorts_lockup(r),
        "lockupViewModel" => lockup(r),
        "channelRenderer" | "gridChannelRenderer" | "compactChannelRenderer" => channel_renderer(r),
        "playlistRenderer"
        | "gridPlaylistRenderer"
        | "compactPlaylistRenderer"
        | "radioRenderer"
        | "compactRadioRenderer"
        | "gridRadioRenderer" => playlist_renderer(k, r),
        "musicResponsiveListItemRenderer" => music_list_item(r),
        "musicTwoRowItemRenderer" => music_two_row(r),
        "musicMultiRowListItemRenderer" => music_multi_row(r),
        "musicNavigationButtonRenderer" => mood_button(r),
        _ => None,
    }
}

fn is_item_renderer(k: &str) -> bool {
    matches!(
        k,
        "videoRenderer"
            | "compactVideoRenderer"
            | "gridVideoRenderer"
            | "playlistVideoRenderer"
            | "videoWithContextRenderer"
            | "endScreenVideoRenderer"
            | "movieRenderer"
            | "compactMovieRenderer"
            | "gridMovieRenderer"
            | "playlistPanelVideoRenderer"
            | "reelItemRenderer"
            | "shortsLockupViewModel"
            | "shortsLockupViewModelV2"
            | "lockupViewModel"
            | "channelRenderer"
            | "gridChannelRenderer"
            | "compactChannelRenderer"
            | "playlistRenderer"
            | "gridPlaylistRenderer"
            | "compactPlaylistRenderer"
            | "radioRenderer"
            | "compactRadioRenderer"
            | "gridRadioRenderer"
            | "musicResponsiveListItemRenderer"
            | "musicTwoRowItemRenderer"
            | "musicMultiRowListItemRenderer"
            | "musicNavigationButtonRenderer"
    )
}

/// Every item under `v`, in order, shelves flattened.
pub fn collect_items(v: &Value) -> Vec<Item> {
    let mut b = Builder::new(ShelfKind::List);
    b.visit(v);
    b.flush();
    b.shelves.into_iter().flat_map(|s| s.items).collect()
}

fn overlay_style(r: &Value) -> Option<&str> {
    r.get("thumbnailOverlays")?
        .as_array()?
        .iter()
        .find_map(|o| str_at(o, "/thumbnailOverlayTimeStatusRenderer/style"))
}

fn overlay_time(r: &Value) -> Option<String> {
    r.get("thumbnailOverlays")?
        .as_array()?
        .iter()
        .find_map(|o| text_at(o, "/thumbnailOverlayTimeStatusRenderer/text"))
}

fn not_interested_token(menu: &Value) -> Option<String> {
    // Classic menus: menuServiceItemRenderer with the NOT_INTERESTED icon.
    for it in find_key(menu, "menuServiceItemRenderer") {
        if str_at(it, "/icon/iconType") == Some("NOT_INTERESTED") {
            if let Some(t) = find_first(it, "feedbackToken").and_then(Value::as_str) {
                return Some(t.to_string());
            }
        }
    }
    // Lockup sheets: listItemViewModel with the same icon name.
    for it in find_key(menu, "listItemViewModel") {
        let icon = find_first(it, "imageName").and_then(Value::as_str);
        if icon == Some("NOT_INTERESTED") {
            if let Some(t) = find_first(it, "feedbackToken").and_then(Value::as_str) {
                return Some(t.to_string());
            }
        }
    }
    None
}

const STREAMED_PREFIXES: &[&str] = &["Streamed", "Transmitido", "Emitido", "Stream", "Diffusé"];

fn video_like(k: &str, r: &Value) -> Option<Item> {
    let id = r.get("videoId").and_then(Value::as_str)?.to_string();
    let nav = r.get("navigationEndpoint").unwrap_or(&Value::Null);
    let title = r
        .get("title")
        .and_then(text)
        .or_else(|| r.get("headline").and_then(text))
        .unwrap_or_default();
    let tn = thumbs(r.get("thumbnail").unwrap_or(&Value::Null));
    let style = overlay_style(r);
    let is_short = endpoint(nav).get("reelWatchEndpoint").is_some() || style == Some("SHORTS");
    let byline = ["ownerText", "shortBylineText", "longBylineText"]
        .iter()
        .find_map(|k| r.get(*k).and_then(channel_from_runs));
    let mut channel = byline.map(|mut c| {
        let (verified, artist) = owner_flags(r);
        c.verified = verified;
        c.artist = artist;
        c.avatar = r
            .get("channelThumbnailSupportedRenderers")
            .or_else(|| r.get("channelThumbnail"))
            .or_else(|| r.get("avatar"))
            .and_then(best_thumb);
        c
    });
    if channel.as_ref().is_some_and(|c| c.name.is_empty()) {
        channel = None;
    }
    let views_full = r.get("viewCountText").and_then(text);
    let views_text = r
        .get("shortViewCountText")
        .and_then(text)
        .or(views_full.clone());
    if is_short {
        return Some(Item::Short(ShortItem {
            id,
            title,
            thumbs: tn,
            views_text,
            channel,
        }));
    }
    let length = r
        .get("lengthText")
        .and_then(text)
        .or_else(|| overlay_time(r));
    let published_text = r.get("publishedTimeText").and_then(text);
    let mut badges = Vec::new();
    match style {
        Some("LIVE") => badges.push(Badge::Live),
        Some("UPCOMING") => badges.push(Badge::Upcoming),
        _ => {}
    }
    if r.get("upcomingEventData").is_some() && !badges.contains(&Badge::Upcoming) {
        badges.push(Badge::Upcoming);
    }
    for s in badge_styles(r.get("badges").unwrap_or(&Value::Null)) {
        match s.as_str() {
            "BADGE_STYLE_TYPE_LIVE_NOW" => badges.push(Badge::Live),
            "BADGE_STYLE_TYPE_MEMBERS_ONLY" => badges.push(Badge::Members),
            "BADGE_STYLE_TYPE_YPC" => badges.push(Badge::Paid),
            _ => {}
        }
    }
    for b in find_key(r.get("badges").unwrap_or(&Value::Null), "label") {
        match b.as_str() {
            Some("New") => badges.push(Badge::New),
            Some("4K") => badges.push(Badge::Hd4k),
            Some("CC") => badges.push(Badge::Cc),
            _ => {}
        }
    }
    if k.contains("ovie") {
        badges.push(Badge::Paid);
    }
    if published_text
        .as_deref()
        .is_some_and(|p| STREAMED_PREFIXES.iter().any(|s| p.starts_with(s)))
    {
        badges.push(Badge::Streamed);
    }
    badges.dedup();
    let watched_pct = find_first(r, "thumbnailOverlayResumePlaybackRenderer")
        .and_then(|o| o.get("percentDurationWatched"))
        .and_then(Value::as_u64)
        .map(|p| p.min(100) as u8);
    Some(Item::Video(VideoItem {
        id,
        title,
        channel,
        thumbs: tn,
        duration_s: length.as_deref().and_then(parse_duration),
        views: views_full.as_deref().and_then(parse_count),
        views_text,
        published_text,
        published_ms: None,
        badges,
        watched_pct,
        description_snippet: r
            .pointer("/detailedMetadataSnippets/0/snippetText")
            .or_else(|| r.get("descriptionSnippet"))
            .and_then(text),
        feedback_token: r.get("menu").and_then(not_interested_token),
        set_video_id: r
            .get("setVideoId")
            .and_then(Value::as_str)
            .map(str::to_string),
    }))
}

fn shorts_lockup(r: &Value) -> Option<Item> {
    let tap = r.get("onTap").unwrap_or(&Value::Null);
    let id = watch_of(tap)
        .and_then(|w| w.get("videoId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            r.get("entityId")
                .and_then(Value::as_str)
                .and_then(|e| e.strip_prefix("shorts-shelf-item-"))
                .map(str::to_string)
        })?;
    Some(Item::Short(ShortItem {
        id,
        title: text_at(r, "/overlayMetadata/primaryText").unwrap_or_default(),
        thumbs: thumbs(
            r.get("thumbnail")
                .or_else(|| r.get("thumbnailViewModel"))
                .unwrap_or(&Value::Null),
        ),
        views_text: text_at(r, "/overlayMetadata/secondaryText"),
        channel: None,
    }))
}

/// Overlay badge texts of a lockup thumbnail (duration, "LIVE", "Mix",
/// "50 videos") with their styles.
fn lockup_badges(img: &Value) -> Vec<(String, String)> {
    find_key(img, "thumbnailBadgeViewModel")
        .into_iter()
        .map(|b| {
            (
                b.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                b.get("badgeStyle")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect()
}

struct LockupMeta {
    channel: Option<ChannelRef>,
    stats: Vec<(String, Option<String>)>,
}

fn lockup_meta(md: &Value) -> LockupMeta {
    let rows = md
        .pointer("/metadata/contentMetadataViewModel/metadataRows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut channel: Option<ChannelRef> = None;
    let mut stats = Vec::new();
    let avatar = md.get("image");
    for (i, row) in rows.iter().enumerate() {
        let parts = row
            .get("metadataParts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if parts.is_empty() {
            continue;
        }
        let has_channel_link = parts.iter().any(|p| {
            find_key(p, "browseEndpoint").iter().any(|b| {
                b.get("browseId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("UC"))
            })
        });
        let is_byline = channel.is_none() && (has_channel_link || (i == 0 && rows.len() >= 2));
        if is_byline {
            let p = &parts[0];
            let t = p.get("text").unwrap_or(&Value::Null);
            let name = text(t).unwrap_or_default();
            let link = find_key(p, "browseEndpoint").into_iter().find(|b| {
                b.get("browseId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("UC"))
            });
            let avatar_link = avatar
                .map(|a| find_key(a, "browseEndpoint"))
                .unwrap_or_default()
                .into_iter()
                .find(|b| b.get("browseId").is_some());
            let b = link.or(avatar_link);
            let (verified, artist) = attachment_flags(t);
            channel = Some(ChannelRef {
                id: b
                    .and_then(|b| b.get("browseId"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                handle: b
                    .and_then(|b| b.get("canonicalBaseUrl"))
                    .and_then(Value::as_str)
                    .and_then(handle_from_url),
                name,
                avatar: avatar.and_then(best_thumb),
                verified,
                artist,
            });
            continue;
        }
        for p in &parts {
            if let Some(t) = p.get("text").and_then(text) {
                let a11y = p
                    .get("accessibilityLabel")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                stats.push((t, a11y));
            }
        }
    }
    LockupMeta { channel, stats }
}

fn looks_like_views(s: &str) -> bool {
    let l = s.to_lowercase();
    [
        "view",
        "visualiza",
        "vista",
        "watching",
        "assistindo",
        "mirando",
        "reproduc",
        "aufrufe",
        "vues",
    ]
    .iter()
    .any(|w| l.contains(w))
}

fn lockup(r: &Value) -> Option<Item> {
    let content_id = r.get("contentId").and_then(Value::as_str)?.to_string();
    let ctype = r.get("contentType").and_then(Value::as_str).unwrap_or("");
    let md = r
        .get("metadata")
        .and_then(|m| m.get("lockupMetadataViewModel"))?;
    let title = md.get("title").and_then(text).unwrap_or_default();
    let img = r.get("contentImage").unwrap_or(&Value::Null);
    let tap = r
        .pointer("/rendererContext/commandContext/onTap")
        .unwrap_or(&Value::Null);
    let meta = lockup_meta(md);
    let badges_raw = lockup_badges(img);

    if endpoint(tap).get("reelWatchEndpoint").is_some() {
        return Some(Item::Short(ShortItem {
            id: content_id,
            title,
            thumbs: thumbs(img),
            views_text: meta.stats.first().map(|s| s.0.clone()),
            channel: meta.channel,
        }));
    }

    let is_collection = img.get("collectionThumbnailViewModel").is_some()
        || matches!(
            ctype,
            "LOCKUP_CONTENT_TYPE_PLAYLIST"
                | "LOCKUP_CONTENT_TYPE_ALBUM"
                | "LOCKUP_CONTENT_TYPE_PODCAST"
        );
    if ctype == "LOCKUP_CONTENT_TYPE_CHANNEL"
        || (content_id.starts_with("UC") && content_id.len() == 24)
    {
        return Some(Item::Channel(ChannelItem {
            id: content_id,
            name: title,
            avatar: best_thumb(img),
            subscribers_text: meta.stats.first().map(|s| s.0.clone()),
            ..Default::default()
        }));
    }
    if is_collection {
        let w = watch_of(tap);
        let playlist_id = w
            .and_then(|w| w.get("playlistId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| content_id.clone());
        let first_video_id = w
            .and_then(|w| w.get("videoId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let count_text = badges_raw
            .iter()
            .map(|b| b.0.clone())
            .find(|t| !t.is_empty());
        let is_mix = playlist_id.starts_with("RD")
            || find_key(img, "imageName")
                .iter()
                .any(|n| n.as_str() == Some("MIX"));
        return Some(Item::Playlist(PlaylistItem {
            id: playlist_id,
            title,
            thumbs: thumbs(img),
            channel: meta.channel.filter(|c| !c.name.is_empty()),
            count_text,
            is_mix,
            first_video_id,
            is_music: false,
        }));
    }

    // Video
    let mut badges = Vec::new();
    let mut duration = None;
    for (t, style) in &badges_raw {
        if style.contains("LIVE") || t.eq_ignore_ascii_case("live") {
            badges.push(Badge::Live);
        } else if style.contains("UPCOMING") {
            badges.push(Badge::Upcoming);
        } else if let Some(d) = parse_duration(t) {
            duration = Some(d);
        } else if t.eq_ignore_ascii_case("shorts") {
            return Some(Item::Short(ShortItem {
                id: content_id,
                title,
                thumbs: thumbs(img),
                views_text: meta.stats.first().map(|s| s.0.clone()),
                channel: meta.channel,
            }));
        }
    }
    for b in find_key(md, "badgeViewModel") {
        let style = b.get("badgeStyle").and_then(Value::as_str).unwrap_or("");
        let label = b.get("badgeText").and_then(Value::as_str).unwrap_or("");
        if style.contains("MEMBERS") || label.to_lowercase().contains("members") {
            badges.push(Badge::Members);
        } else if label == "New" {
            badges.push(Badge::New);
        } else if label == "4K" {
            badges.push(Badge::Hd4k);
        } else if label == "CC" {
            badges.push(Badge::Cc);
        } else if style.contains("LIVE") {
            badges.push(Badge::Live);
        }
    }
    let mut views_text = None;
    let mut views = None;
    let mut published_text = None;
    let n = meta.stats.len();
    for (i, (t, a11y)) in meta.stats.iter().enumerate() {
        let viewish = looks_like_views(t)
            || a11y.as_deref().is_some_and(looks_like_views)
            || (n >= 2 && i == 0 && views_text.is_none());
        if viewish && views_text.is_none() {
            views_text = Some(t.clone());
            views = a11y
                .as_deref()
                .and_then(parse_count)
                .or_else(|| parse_count(t));
        } else if published_text.is_none() {
            published_text = Some(t.clone());
        }
    }
    if published_text
        .as_deref()
        .is_some_and(|p| STREAMED_PREFIXES.iter().any(|s| p.starts_with(s)))
    {
        badges.push(Badge::Streamed);
    }
    if badges.contains(&Badge::Live) || badges.contains(&Badge::Upcoming) {
        // "1.2K watching" / "Scheduled for ..." are not view counts.
        if badges.contains(&Badge::Upcoming) {
            published_text = views_text.take().or(published_text);
            views = None;
        }
    }
    badges.dedup();
    let watched_pct = find_first(img, "thumbnailOverlayProgressBarViewModel")
        .and_then(|p| p.get("startPercent"))
        .and_then(Value::as_u64)
        .map(|p| p.min(100) as u8);
    Some(Item::Video(VideoItem {
        id: content_id,
        title,
        channel: meta.channel.filter(|c| !c.name.is_empty()),
        thumbs: thumbs(img),
        duration_s: duration,
        views,
        views_text,
        published_text,
        published_ms: None,
        badges,
        watched_pct,
        description_snippet: None,
        feedback_token: md.get("menuButton").and_then(not_interested_token),
        set_video_id: None,
    }))
}

fn channel_renderer(r: &Value) -> Option<Item> {
    let id = r
        .get("channelId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| browse_of(r.get("navigationEndpoint")?).map(|(id, _, _)| id.to_string()))?;
    let name = r
        .get("title")
        .and_then(text)
        .or_else(|| r.get("displayName").and_then(text))
        .unwrap_or_default();
    let (verified, artist) = owner_flags(r);
    let sub = r.get("subscriberCountText").and_then(text);
    let vids = r.get("videoCountText").and_then(text);
    // A/B: the handle sometimes sits in subscriberCountText and the
    // subscriber count in videoCountText.
    let (handle, subscribers_text, video_count_text) = match (&sub, &vids) {
        (Some(s), v) if s.starts_with('@') => (Some(s.clone()), v.clone(), None),
        _ => (
            str_at(r, "/navigationEndpoint/browseEndpoint/canonicalBaseUrl")
                .and_then(handle_from_url),
            sub.clone(),
            vids.clone(),
        ),
    };
    let subscribed = r
        .pointer("/subscribeButton/subscribeButtonRenderer/subscribed")
        .and_then(Value::as_bool)
        .filter(|_| {
            r.get("subscribeButton")
                .is_none_or(|b| find_first(b, "signInEndpoint").is_none())
        });
    Some(Item::Channel(ChannelItem {
        id,
        name,
        handle,
        avatar: r.get("thumbnail").and_then(best_thumb),
        subscribers_text,
        video_count_text,
        description: r.get("descriptionSnippet").and_then(text),
        verified,
        artist,
        subscribed,
    }))
}

fn playlist_renderer(k: &str, r: &Value) -> Option<Item> {
    let id = r.get("playlistId").and_then(Value::as_str)?.to_string();
    let is_mix = k.contains("adio") || id.starts_with("RD");
    let tn = r
        .get("thumbnails")
        .and_then(|t| t.get(0))
        .map(thumbs)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            thumbs(
                r.get("thumbnail")
                    .or_else(|| r.get("thumbnailRenderer"))
                    .unwrap_or(&Value::Null),
            )
        });
    let count_text = r
        .get("videoCountText")
        .and_then(text)
        .or_else(|| r.get("videoCountShortText").and_then(text))
        .or_else(|| {
            r.get("videoCount")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            find_key(
                r.get("thumbnailOverlays").unwrap_or(&Value::Null),
                "thumbnailOverlayBottomPanelRenderer",
            )
            .first()
            .and_then(|p| p.get("text"))
            .and_then(text)
        });
    let channel = ["shortBylineText", "longBylineText"]
        .iter()
        .find_map(|k| r.get(*k).and_then(channel_from_runs));
    let first_video_id = r
        .get("navigationEndpoint")
        .and_then(watch_of)
        .and_then(|w| w.get("videoId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(Item::Playlist(PlaylistItem {
        id,
        title: r.get("title").and_then(text).unwrap_or_default(),
        thumbs: tn,
        channel,
        count_text,
        is_mix,
        first_video_id,
        is_music: false,
    }))
}

// --- YouTube Music ---------------------------------------------------------

fn is_explicit(r: &Value) -> bool {
    find_key(r, "musicInlineBadgeRenderer")
        .iter()
        .any(|b| str_at(b, "/icon/iconType") == Some("MUSIC_EXPLICIT_BADGE"))
}

fn flex_columns(r: &Value) -> Vec<&Value> {
    r.get("flexColumns")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| {
                    c.get("musicResponsiveListItemFlexColumnRenderer")
                        .and_then(|f| f.get("text"))
                        .unwrap_or(&Value::Null)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn run_array(v: &Value) -> Vec<&Value> {
    v.get("runs")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn is_separator(t: &str) -> bool {
    let t = t.trim();
    t.is_empty()
        || t == "•"
        || t == "·"
        || t == ","
        || t == "&"
        || t == "and"
        || t == "e"
        || t == "y"
}

fn is_year(t: &str) -> bool {
    t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit())
}

/// Run of a music byline that links to an artist / channel.
fn artist_from_run(r: &Value) -> Option<ChannelRef> {
    let ep = r.get("navigationEndpoint")?;
    let (id, _, page_type) = browse_of(ep)?;
    let artistish = matches!(
        page_type,
        Some(
            "MUSIC_PAGE_TYPE_ARTIST"
                | "MUSIC_PAGE_TYPE_USER_CHANNEL"
                | "MUSIC_PAGE_TYPE_LIBRARY_ARTIST"
        )
    ) || (page_type.is_none() && id.starts_with("UC"));
    artistish.then(|| ChannelRef {
        id: id.to_string(),
        name: r
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        artist: true,
        ..Default::default()
    })
}

fn album_from_run(r: &Value) -> Option<AlbumRef> {
    let (id, _, page_type) = browse_of(r.get("navigationEndpoint")?)?;
    (page_type == Some("MUSIC_PAGE_TYPE_ALBUM") || id.starts_with("MPRE")).then(|| AlbumRef {
        id: id.to_string(),
        name: r
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// Artists, album, duration and play count spread over the byline runs of
/// a song row (search, album, playlist, quick picks).
struct SongByline {
    artists: Vec<ChannelRef>,
    album: Option<AlbumRef>,
    duration_s: Option<u32>,
    plays_text: Option<String>,
    plain: Vec<String>,
}

fn song_byline<'a>(runs: impl Iterator<Item = &'a Value>) -> SongByline {
    let mut b = SongByline {
        artists: Vec::new(),
        album: None,
        duration_s: None,
        plays_text: None,
        plain: Vec::new(),
    };
    for r in runs {
        let t = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
        if is_separator(t) {
            continue;
        }
        if let Some(a) = artist_from_run(r) {
            b.artists.push(a);
        } else if let Some(al) = album_from_run(r) {
            b.album = Some(al);
        } else if let Some(d) = parse_duration(t) {
            b.duration_s = Some(d);
        } else if t.chars().any(|c| c.is_ascii_digit()) && !is_year(t) {
            b.plays_text = Some(t.to_string());
        } else if r.get("navigationEndpoint").is_none() {
            b.plain.push(t.to_string());
        }
    }
    b
}

fn music_list_item(r: &Value) -> Option<Item> {
    let cols = flex_columns(r);
    let title_col = cols.first().copied().unwrap_or(&Value::Null);
    let title = text(title_col).unwrap_or_default();
    let tn = thumbs(r.get("thumbnail").unwrap_or(&Value::Null));
    let first_run_nav = title_col.pointer("/runs/0/navigationEndpoint");
    let video_id = str_at(r, "/playlistItemData/videoId")
        .or_else(|| {
            first_run_nav
                .and_then(watch_of)
                .and_then(|w| w.get("videoId"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            find_first(
                r.get("overlay").unwrap_or(&Value::Null),
                "playNavigationEndpoint",
            )
            .and_then(watch_of)
            .and_then(|w| w.get("videoId"))
            .and_then(Value::as_str)
        });
    let browse = r
        .get("navigationEndpoint")
        .and_then(browse_of)
        .or_else(|| first_run_nav.and_then(browse_of));

    if let (Some(vid), None) = (video_id, browse) {
        let byline = song_byline(cols.iter().skip(1).flat_map(|c| run_array(c)));
        let fixed_duration = r
            .pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text")
            .and_then(text)
            .as_deref()
            .and_then(parse_duration);
        let vtype = first_run_nav.and_then(music_video_type).or_else(|| {
            find_first(
                r.get("overlay").unwrap_or(&Value::Null),
                "playNavigationEndpoint",
            )
            .and_then(music_video_type)
        });
        let mut artists = byline.artists;
        if artists.is_empty() {
            // "Various Artists" and podcast hosts come without a link.
            if let Some(name) = byline.plain.last() {
                artists.push(ChannelRef {
                    name: name.clone(),
                    ..Default::default()
                });
            }
        }
        let liked = find_first(r.get("menu").unwrap_or(&Value::Null), "likeButtonRenderer")
            .and_then(|l| l.get("likeStatus"))
            .and_then(Value::as_str)
            .and_then(|s| match s {
                "LIKE" => Some(true),
                "DISLIKE" => Some(false),
                _ => None,
            });
        return Some(Item::Song(SongItem {
            id: vid.to_string(),
            title,
            artists,
            album: byline.album,
            thumbs: tn,
            duration_s: fixed_duration.or(byline.duration_s),
            explicit: is_explicit(r),
            is_video: matches!(
                vtype,
                Some(
                    "MUSIC_VIDEO_TYPE_OMV"
                        | "MUSIC_VIDEO_TYPE_UGC"
                        | "MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC"
                )
            ),
            plays_text: byline.plays_text,
            set_video_id: str_at(r, "/playlistItemData/playlistSetVideoId").map(str::to_string),
            liked,
        }));
    }

    let (id, _, page_type) = browse?;
    let sub_runs: Vec<&Value> = cols.iter().skip(1).flat_map(|c| run_array(c)).collect();
    match page_type {
        Some("MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK") => {
            let byline = song_byline(sub_runs.iter().copied());
            Some(Item::Album(AlbumItem {
                id: id.to_string(),
                playlist_id: None,
                title,
                artists: byline.artists,
                year: sub_runs
                    .iter()
                    .filter_map(|r| r.get("text").and_then(Value::as_str))
                    .find(|t| is_year(t))
                    .map(str::to_string),
                thumbs: tn,
                album_type: byline.plain.first().cloned(),
                explicit: is_explicit(r),
            }))
        }
        Some(
            "MUSIC_PAGE_TYPE_ARTIST"
            | "MUSIC_PAGE_TYPE_USER_CHANNEL"
            | "MUSIC_PAGE_TYPE_LIBRARY_ARTIST",
        ) => Some(Item::Artist(ArtistItem {
            id: id.to_string(),
            name: title,
            thumbs: tn,
            subscribers_text: sub_runs
                .iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .find(|t| t.chars().any(|c| c.is_ascii_digit()))
                .map(str::to_string),
        })),
        _ => {
            let pid = id.strip_prefix("VL").unwrap_or(id).to_string();
            let channel = sub_runs
                .iter()
                .find_map(|r| artist_from_run(r))
                .or_else(|| {
                    sub_runs
                        .iter()
                        .filter_map(|r| r.get("text").and_then(Value::as_str))
                        .filter(|t| !is_separator(t))
                        .nth(1)
                        .map(|n| ChannelRef {
                            name: n.to_string(),
                            ..Default::default()
                        })
                });
            Some(Item::Playlist(PlaylistItem {
                is_mix: pid.starts_with("RD"),
                id: pid,
                title,
                thumbs: tn,
                channel,
                count_text: sub_runs
                    .iter()
                    .filter_map(|r| r.get("text").and_then(Value::as_str))
                    .rfind(|t| t.chars().any(|c| c.is_ascii_digit()))
                    .map(str::to_string),
                first_video_id: None,
                is_music: true,
            }))
        }
    }
}

fn music_two_row(r: &Value) -> Option<Item> {
    let title = r.get("title").and_then(text).unwrap_or_default();
    let tn = thumbs(r.get("thumbnailRenderer").unwrap_or(&Value::Null));
    let nav = r.get("navigationEndpoint").unwrap_or(&Value::Null);
    let sub = r.get("subtitle").unwrap_or(&Value::Null);
    let sub_runs = run_array(sub);
    let byline = song_byline(sub_runs.iter().copied());

    if let Some(w) = endpoint(nav).get("watchEndpoint") {
        let vid = w.get("videoId").and_then(Value::as_str)?;
        return Some(Item::Song(SongItem {
            id: vid.to_string(),
            title,
            artists: byline.artists,
            album: byline.album,
            thumbs: tn,
            duration_s: byline.duration_s,
            explicit: is_explicit(r),
            is_video: !matches!(music_video_type(nav), Some("MUSIC_VIDEO_TYPE_ATV")),
            plays_text: byline.plays_text,
            set_video_id: None,
            liked: None,
        }));
    }
    let (id, _, page_type) = browse_of(nav)?;
    let texts: Vec<&str> = sub_runs
        .iter()
        .filter_map(|r| r.get("text").and_then(Value::as_str))
        .filter(|t| !is_separator(t))
        .collect();
    match page_type {
        Some("MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK") => {
            let first_plain = sub_runs
                .first()
                .filter(|r| r.get("navigationEndpoint").is_none())
                .and_then(|r| r.get("text").and_then(Value::as_str))
                .filter(|t| !is_year(t) && !is_separator(t) && sub_runs.len() > 1);
            let playlist_id = find_first(
                r.get("thumbnailOverlay").unwrap_or(&Value::Null),
                "playNavigationEndpoint",
            )
            .and_then(watch_of)
            .and_then(|w| w.get("playlistId"))
            .and_then(Value::as_str)
            .map(str::to_string);
            Some(Item::Album(AlbumItem {
                id: id.to_string(),
                playlist_id,
                title,
                artists: byline.artists,
                year: texts.iter().find(|t| is_year(t)).map(|t| t.to_string()),
                thumbs: tn,
                album_type: first_plain.map(str::to_string),
                explicit: is_explicit(r),
            }))
        }
        Some(
            "MUSIC_PAGE_TYPE_ARTIST"
            | "MUSIC_PAGE_TYPE_USER_CHANNEL"
            | "MUSIC_PAGE_TYPE_LIBRARY_ARTIST",
        ) => Some(Item::Artist(ArtistItem {
            id: id.to_string(),
            name: title,
            thumbs: tn,
            subscribers_text: texts.last().map(|t| t.to_string()),
        })),
        _ => {
            let pid = id.strip_prefix("VL").unwrap_or(id).to_string();
            let channel = byline.artists.into_iter().next().or_else(|| {
                // "Playlist • YouTube Music • 50 songs": the owner is the
                // middle plain run.
                (texts.len() >= 3).then(|| ChannelRef {
                    name: texts[1].to_string(),
                    ..Default::default()
                })
            });
            Some(Item::Playlist(PlaylistItem {
                is_mix: pid.starts_with("RD"),
                id: pid,
                title,
                thumbs: tn,
                channel,
                count_text: texts
                    .iter()
                    .rev()
                    .find(|t| t.chars().any(|c| c.is_ascii_digit()))
                    .map(|t| t.to_string()),
                first_video_id: None,
                is_music: true,
            }))
        }
    }
}

fn music_multi_row(r: &Value) -> Option<Item> {
    let vid = find_first(r.get("onTap").unwrap_or(&Value::Null), "watchEndpoint")
        .or_else(|| find_first(r, "playNavigationEndpoint").and_then(watch_of))
        .and_then(|w| w.get("videoId"))
        .and_then(Value::as_str)?;
    let sub = r.get("subtitle").unwrap_or(&Value::Null);
    let byline = song_byline(run_array(sub).into_iter());
    Some(Item::Song(SongItem {
        id: vid.to_string(),
        title: r.get("title").and_then(text).unwrap_or_default(),
        artists: byline.artists,
        album: None,
        thumbs: thumbs(r.get("thumbnail").unwrap_or(&Value::Null)),
        duration_s: byline.duration_s,
        explicit: is_explicit(r),
        is_video: false,
        plays_text: r.get("secondTitle").and_then(text).or(byline.plays_text),
        set_video_id: None,
        liked: None,
    }))
}

fn mood_button(r: &Value) -> Option<Item> {
    let (id, params, _) = browse_of(r.get("clickCommand")?)?;
    let color = r
        .pointer("/solid/leftStripeColor")
        .and_then(Value::as_u64)
        .map(|argb| format!("#{:06x}", argb & 0xff_ffff));
    Some(Item::Mood(MoodItem {
        title: r.get("buttonText").and_then(text).unwrap_or_default(),
        browse_id: id.to_string(),
        params: params.map(str::to_string),
        color,
    }))
}

/// YT Music queue row (`next`). Also used by `item()` when a panel row
/// carries a music video type.
pub(crate) fn song_from_panel(r: &Value) -> Option<SongItem> {
    let r = r
        .get("primaryRenderer")
        .and_then(|p| p.get("playlistPanelVideoRenderer"))
        .unwrap_or(r);
    let id = r.get("videoId").and_then(Value::as_str)?.to_string();
    let nav = r.get("navigationEndpoint").unwrap_or(&Value::Null);
    let byline =
        song_byline(run_array(r.get("longBylineText").unwrap_or(&Value::Null)).into_iter());
    let mut artists = byline.artists;
    if artists.is_empty() {
        if let Some(name) = r.get("shortBylineText").and_then(text) {
            artists.push(ChannelRef {
                name,
                ..Default::default()
            });
        }
    }
    Some(SongItem {
        id,
        title: r.get("title").and_then(text).unwrap_or_default(),
        artists,
        album: byline.album,
        thumbs: thumbs(r.get("thumbnail").unwrap_or(&Value::Null)),
        duration_s: r
            .get("lengthText")
            .and_then(text)
            .as_deref()
            .and_then(parse_duration),
        explicit: is_explicit(r),
        is_video: matches!(
            music_video_type(nav),
            Some("MUSIC_VIDEO_TYPE_OMV" | "MUSIC_VIDEO_TYPE_UGC")
        ),
        plays_text: byline.plays_text,
        set_video_id: r
            .get("playlistSetVideoId")
            .and_then(Value::as_str)
            .map(str::to_string),
        liked: None,
    })
}

// ---------------------------------------------------------------------------
// Shelves
// ---------------------------------------------------------------------------

/// Groups items into shelves by their containers. Loose items (not inside
/// a titled container) become untitled shelves of `default_kind`.
pub(crate) struct Builder {
    pub shelves: Vec<Shelf>,
    loose: Vec<Item>,
    default_kind: ShelfKind,
}

impl Builder {
    pub fn new(default_kind: ShelfKind) -> Self {
        Self {
            shelves: Vec::new(),
            loose: Vec::new(),
            default_kind,
        }
    }

    pub fn flush(&mut self) {
        if !self.loose.is_empty() {
            let items = std::mem::take(&mut self.loose);
            self.shelves.push(Shelf {
                kind: self.default_kind,
                items,
                ..Default::default()
            });
        }
    }

    pub fn visit(&mut self, v: &Value) {
        match v {
            Value::Array(a) => {
                for i in a {
                    self.visit(i);
                }
            }
            Value::Object(o) => {
                for (k, inner) in o {
                    self.visit_key(k, inner);
                }
            }
            _ => {}
        }
    }

    fn visit_key(&mut self, k: &str, inner: &Value) {
        if SKIP_KEYS.contains(&k) || JUNK.contains(&k) {
            return;
        }
        if k == "richItemRenderer" {
            if let Some(c) = inner.get("content") {
                self.visit(c);
            }
            return;
        }
        if is_item_renderer(k) {
            if let Some(it) = item_from(k, inner) {
                self.loose.push(it);
            }
            return;
        }
        if let Some(shelf) = titled_shelf(k, inner) {
            self.flush();
            self.shelves.push(shelf);
            return;
        }
        let transparent = TRANSPARENT.contains(&k);
        let renderer_like = k.ends_with("Renderer") || k.ends_with("ViewModel");
        if renderer_like && !transparent {
            note_unknown(k);
        }
        if renderer_like || transparent || DESCEND_KEYS.contains(&k) {
            self.visit(inner);
        }
    }

    pub fn finish(mut self) -> Vec<Shelf> {
        self.flush();
        finalize(self.shelves)
    }
}

/// Drops empty shelves, fixes the kind of all-Shorts / all-mood shelves and
/// numbers them.
pub(crate) fn finalize(shelves: Vec<Shelf>) -> Vec<Shelf> {
    shelves
        .into_iter()
        .filter(|s| !s.items.is_empty())
        .enumerate()
        .map(|(i, mut s)| {
            if s.items.iter().all(|it| matches!(it, Item::Short(_))) {
                s.kind = ShelfKind::Shorts;
            } else if s.items.iter().all(|it| matches!(it, Item::Mood(_))) {
                s.kind = ShelfKind::Moods;
            }
            if s.id.is_empty() {
                s.id = format!("s{i}");
            }
            s
        })
        .collect()
}

/// Items of a container (no nested shelves kept).
fn container_items(v: &Value) -> Vec<Item> {
    collect_items(v)
}

fn shelf(title: Option<String>, kind: ShelfKind, items: Vec<Item>) -> Shelf {
    Shelf {
        title,
        kind,
        items,
        ..Default::default()
    }
}

fn titled_shelf(k: &str, r: &Value) -> Option<Shelf> {
    match k {
        "shelfRenderer" => {
            let content = r.get("content").unwrap_or(&Value::Null);
            let kind = if content.get("verticalListRenderer").is_some()
                || content.get("expandedShelfContentsRenderer").is_some()
            {
                ShelfKind::List
            } else {
                ShelfKind::Row
            };
            let mut s = shelf(
                r.get("title").and_then(text),
                kind,
                container_items(content),
            );
            s.subtitle = r.get("subtitle").and_then(text);
            s.more = r.get("endpoint").and_then(nav_of);
            Some(s)
        }
        "richShelfRenderer" => {
            let kind = if r.get("isShorts").and_then(Value::as_bool) == Some(true) {
                ShelfKind::Shorts
            } else {
                ShelfKind::Row
            };
            let mut s = shelf(
                r.get("title").and_then(text),
                kind,
                container_items(r.get("contents").unwrap_or(&Value::Null)),
            );
            s.subtitle = r.get("subtitle").and_then(text);
            s.more = r.get("endpoint").and_then(nav_of);
            Some(s)
        }
        "reelShelfRenderer" => Some(shelf(
            r.get("title").and_then(text),
            ShelfKind::Shorts,
            container_items(r.get("items").unwrap_or(&Value::Null)),
        )),
        "gridShelfViewModel" => Some(shelf(
            text_at(r, "/header/sectionHeaderViewModel/headline"),
            ShelfKind::Shorts,
            container_items(r.get("contents").unwrap_or(&Value::Null)),
        )),
        "horizontalCardListRenderer" => {
            let items = container_items(r.get("cards").unwrap_or(&Value::Null));
            Some(shelf(
                text_at(r, "/header/richListHeaderRenderer/title"),
                ShelfKind::Row,
                items,
            ))
        }
        "itemSectionRenderer" => {
            // History groups by day with an itemSectionHeaderRenderer.
            let title = text_at(r, "/header/itemSectionHeaderRenderer/title")?;
            Some(shelf(
                Some(title),
                ShelfKind::List,
                container_items(r.get("contents").unwrap_or(&Value::Null)),
            ))
        }
        "gridRenderer" => {
            let title = text_at(r, "/header/gridHeaderRenderer/title");
            let items = container_items(r.get("items").unwrap_or(&Value::Null));
            let all_music = !items.is_empty()
                && items.iter().all(|i| match i {
                    Item::Album(_) | Item::Artist(_) | Item::Song(_) | Item::Mood(_) => true,
                    Item::Playlist(p) => p.is_music,
                    _ => false,
                });
            if title.is_none() && !all_music {
                return None;
            }
            let kind = if items.iter().all(|i| matches!(i, Item::Mood(_))) {
                ShelfKind::Moods
            } else if all_music {
                ShelfKind::Carousel
            } else {
                ShelfKind::Grid
            };
            Some(shelf(title, kind, items))
        }
        "musicCarouselShelfRenderer" | "musicImmersiveCarouselShelfRenderer" => {
            let h = r
                .pointer("/header/musicCarouselShelfBasicHeaderRenderer")
                .or_else(|| r.pointer("/header/musicImmersiveCarouselShelfBasicHeaderRenderer"))
                .unwrap_or(&Value::Null);
            let contents = r.get("contents").unwrap_or(&Value::Null);
            let items = container_items(contents);
            let list_rows = find_key(contents, "musicResponsiveListItemRenderer").len();
            let kind = if list_rows > 0 && items.iter().all(|i| matches!(i, Item::Song(_))) {
                ShelfKind::QuickPicks
            } else {
                ShelfKind::Carousel
            };
            let mut s = shelf(h.get("title").and_then(text), kind, items);
            s.strapline = h.get("strapline").and_then(text);
            s.thumb = h.get("thumbnail").and_then(best_thumb);
            s.more = h
                .pointer("/moreContentButton/buttonRenderer/navigationEndpoint")
                .and_then(nav_of)
                .or_else(|| {
                    h.pointer("/title/runs/0/navigationEndpoint")
                        .and_then(nav_of)
                });
            Some(s)
        }
        "musicShelfRenderer" | "musicPlaylistShelfRenderer" => {
            let mut s = shelf(
                r.get("title").and_then(text),
                ShelfKind::List,
                container_items(r.get("contents").unwrap_or(&Value::Null)),
            );
            s.more = r.get("bottomEndpoint").and_then(nav_of).or_else(|| {
                r.pointer("/title/runs/0/navigationEndpoint")
                    .and_then(nav_of)
            });
            Some(s)
        }
        "musicCardShelfRenderer" => {
            let mut items = Vec::new();
            if let Some(top) = music_card_top(r) {
                items.push(top);
            }
            items.extend(container_items(r.get("contents").unwrap_or(&Value::Null)));
            let mut s = shelf(
                text_at(r, "/header/musicCardShelfHeaderBasicRenderer/title"),
                ShelfKind::List,
                items,
            );
            s.id = "top_result".into();
            Some(s)
        }
        _ => None,
    }
}

/// The big "Top result" card of YT Music search.
fn music_card_top(r: &Value) -> Option<Item> {
    let title_run = r.pointer("/title/runs/0")?;
    let title = r.get("title").and_then(text).unwrap_or_default();
    let tn = thumbs(r.get("thumbnail").unwrap_or(&Value::Null));
    let sub_runs = run_array(r.get("subtitle").unwrap_or(&Value::Null));
    let byline = song_byline(sub_runs.iter().copied());
    let nav = title_run
        .get("navigationEndpoint")
        .or_else(|| r.get("onTap"))
        .unwrap_or(&Value::Null);
    if let Some(w) = endpoint(nav).get("watchEndpoint") {
        return Some(Item::Song(SongItem {
            id: w.get("videoId").and_then(Value::as_str)?.to_string(),
            title,
            artists: byline.artists,
            album: byline.album,
            thumbs: tn,
            duration_s: byline.duration_s,
            explicit: is_explicit(r),
            is_video: !matches!(music_video_type(nav), Some("MUSIC_VIDEO_TYPE_ATV")),
            plays_text: byline.plays_text,
            set_video_id: None,
            liked: None,
        }));
    }
    let (id, _, page_type) = browse_of(nav)?;
    match page_type {
        Some("MUSIC_PAGE_TYPE_ALBUM") => Some(Item::Album(AlbumItem {
            id: id.to_string(),
            title,
            artists: byline.artists,
            thumbs: tn,
            album_type: byline.plain.first().cloned(),
            year: sub_runs
                .iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .find(|t| is_year(t))
                .map(str::to_string),
            ..Default::default()
        })),
        Some("MUSIC_PAGE_TYPE_PLAYLIST") => Some(Item::Playlist(PlaylistItem {
            id: id.strip_prefix("VL").unwrap_or(id).to_string(),
            title,
            thumbs: tn,
            is_music: true,
            ..Default::default()
        })),
        _ => Some(Item::Artist(ArtistItem {
            id: id.to_string(),
            name: title,
            thumbs: tn,
            subscribers_text: byline.plays_text,
        })),
    }
}

/// Shelves of any browse / search / continuation subtree.
pub(crate) fn shelves_of(v: &Value, default_kind: ShelfKind) -> Vec<Shelf> {
    let mut b = Builder::new(default_kind);
    b.visit(v);
    b.finish()
}

/// Filter chips: YouTube `chipCloudChipRenderer` / `chipViewModel`
/// (continuation token or search params) and YT Music chips (browse params).
pub(crate) fn chips(v: &Value) -> Vec<Chip> {
    let mut out = Vec::new();
    for c in find_key(v, "chipCloudChipRenderer") {
        let Some(label) = c.get("text").and_then(text) else {
            continue;
        };
        let ep = c
            .get("navigationEndpoint")
            .or_else(|| c.get("onDeselectedCommand"))
            .unwrap_or(&Value::Null);
        out.push(Chip {
            label,
            token: chip_token(ep),
            selected: c
                .get("isSelected")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    for c in find_key(v, "chipViewModel") {
        let Some(label) = c.get("text").and_then(text) else {
            continue;
        };
        out.push(Chip {
            label,
            token: chip_token(c.get("tapCommand").unwrap_or(&Value::Null)),
            selected: c.get("selected").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    out
}

fn chip_token(ep: &Value) -> Option<String> {
    let ep = endpoint(ep);
    ep.pointer("/continuationCommand/token")
        .or_else(|| ep.pointer("/browseEndpoint/params"))
        .or_else(|| ep.pointer("/searchEndpoint/params"))
        .or_else(|| find_first(ep, "continuationCommand").and_then(|c| c.get("token")))
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod testutil {
    use serde_json::Value;
    use std::io::Read;
    use std::path::PathBuf;

    /// Loads `parse/fixtures/<name>.json.gz`.
    pub fn fixture(name: &str) -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/core/tube/parse/fixtures")
            .join(format!("{name}.json.gz"));
        let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut s = String::new();
        flate2::read::GzDecoder::new(file)
            .read_to_string(&mut s)
            .expect("gunzip fixture");
        serde_json::from_str(&s).expect("fixture json")
    }

    /// JSON from the studied repos (rustypipe / ytmapi-rs test data), which
    /// stay outside the repository. `None` when the file isn't there.
    pub fn external(rel: &str) -> Option<Value> {
        let base = std::env::var("OMNITUBE_STUDIES")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../omniget/estudos/102-youtube/repos")
            });
        let path = base.join(rel);
        let text = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&text).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::fixture;
    use super::*;
    use serde_json::json;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("1:02:03"), Some(3723));
        assert_eq!(parse_duration("4:36"), Some(276));
        assert_eq!(parse_duration("0:07"), Some(7));
        assert_eq!(parse_duration("LIVE"), None);
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("1:2:3:4"), None);
    }

    #[test]
    fn counts_en() {
        assert_eq!(parse_count("1,234 views"), Some(1234));
        assert_eq!(parse_count("24,560,729 views"), Some(24_560_729));
        assert_eq!(parse_count("1.2M views"), Some(1_200_000));
        assert_eq!(parse_count("631K views"), Some(631_000));
        assert_eq!(parse_count("8.66M subscribers"), Some(8_660_000));
        assert_eq!(parse_count("11 million views"), Some(11_000_000));
        assert_eq!(parse_count("1.3B"), Some(1_300_000_000));
        assert_eq!(parse_count("1 view"), Some(1));
        assert_eq!(parse_count("No views"), None);
        assert_eq!(
            parse_count("like this video along with 563,597 other people"),
            Some(563_597)
        );
    }

    #[test]
    fn counts_pt_es() {
        assert_eq!(parse_count("1,2 mi de visualizações"), Some(1_200_000));
        assert_eq!(parse_count("12 mil visualizações"), Some(12_000));
        assert_eq!(parse_count("1.234 visualizações"), Some(1234));
        assert_eq!(parse_count("2,5 bi de visualizações"), Some(2_500_000_000));
        assert_eq!(parse_count("1,2 M de visualizaciones"), Some(1_200_000));
        assert_eq!(
            parse_count("3,4 mil M de visualizaciones"),
            Some(3_400_000_000)
        );
        assert_eq!(parse_count("850 mil suscriptores"), Some(850_000));
        assert_eq!(parse_count("1\u{a0}234\u{a0}567 vues"), Some(1_234_567));
    }

    #[test]
    fn text_shapes() {
        assert_eq!(text(&json!({"simpleText": " hi "})), Some("hi".into()));
        assert_eq!(
            text(&json!({"runs": [{"text": "a"}, {"text": "b"}]})),
            Some("ab".into())
        );
        assert_eq!(text(&json!({"content": "vm"})), Some("vm".into()));
        assert_eq!(
            text(&json!({"dynamicTextViewModel": {"text": {"content": "t"}}})),
            Some("t".into())
        );
        assert_eq!(text(&json!({"runs": []})), None);
    }

    #[test]
    fn command_runs_use_utf16_offsets() {
        // "😀 go" : the emoji is two UTF-16 units.
        let v = json!({
            "content": "😀 go here now",
            "commandRuns": [{"startIndex": 6, "length": 4, "onTap": {"innertubeCommand": {
                "urlEndpoint": {"url": "https://www.youtube.com/redirect?event=x&q=https%3A%2F%2Fexample.com%2Fa&v=1"}}}}]
        });
        let r = runs(&v);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].text, "😀 go ");
        assert_eq!(r[1].text, "here");
        assert_eq!(r[1].url.as_deref(), Some("https://example.com/a"));
        assert_eq!(r[2].text, " now");
    }

    #[test]
    fn runs_with_endpoints() {
        let v = json!({"runs": [
            {"text": "1:23", "navigationEndpoint": {"watchEndpoint": {"videoId": "abc", "startTimeSeconds": 83}}},
            {"text": " by "},
            {"text": "Chan", "navigationEndpoint": {"browseEndpoint": {"browseId": "UC123", "canonicalBaseUrl": "/@chan"}}}
        ]});
        let r = runs(&v);
        assert_eq!(r[0].video_id.as_deref(), Some("abc"));
        assert_eq!(r[0].t, Some(83));
        assert_eq!(r[2].channel_id.as_deref(), Some("UC123"));
        assert_eq!(r[2].url.as_deref(), Some("https://www.youtube.com/@chan"));
    }

    #[test]
    fn thumbs_force_https() {
        let v = json!({"thumbnail": {"thumbnails": [{"url": "//yt3.ggpht.com/a", "width": 88, "height": 88}]}});
        assert_eq!(thumbs(&v)[0].url, "https://yt3.ggpht.com/a");
        let vm = json!({"image": {"sources": [{"url": "http://i.ytimg.com/x.jpg", "width": 1}]}});
        assert_eq!(thumbs(&vm)[0].url, "https://i.ytimg.com/x.jpg");
    }

    #[test]
    fn ads_never_become_items() {
        let v = json!({"contents": [
            {"adSlotRenderer": {"videoId": "x"}},
            {"searchPyvRenderer": {"ads": [{"videoRenderer": {"videoId": "ad"}}]}},
            {"videoRenderer": {"videoId": "ok", "title": {"simpleText": "t"}}}
        ]});
        let items = collect_items(&v);
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0], Item::Video(v) if v.id == "ok"));
    }

    #[test]
    fn unknown_renderers_are_counted_not_fatal() {
        take_unknown();
        let v = json!({"contents": [{"brandNewRenderer": {"x": 1}}, {"videoRenderer": {"videoId": "a"}}]});
        assert_eq!(collect_items(&v).len(), 1);
        assert!(take_unknown().iter().any(|(k, _)| k == "brandNewRenderer"));
    }

    #[test]
    fn search_items_have_the_expected_shapes() {
        let j = fixture("web_search");
        let items = collect_items(&j["contents"]);
        let video = items
            .iter()
            .find_map(|i| match i {
                Item::Video(v) => Some(v),
                _ => None,
            })
            .expect("a video");
        assert_eq!(video.id.len(), 11);
        assert!(!video.title.is_empty());
        assert!(video.duration_s.unwrap() > 0);
        assert!(video.views.unwrap() > 0);
        let ch = video.channel.as_ref().unwrap();
        assert!(ch.id.starts_with("UC"));
        assert!(ch.avatar.as_deref().unwrap().starts_with("https://"));
        assert!(items.iter().any(|i| matches!(i, Item::Short(_))));
        assert!(items.iter().any(|i| matches!(i, Item::Playlist(_))));
    }

    #[test]
    fn related_lockups_become_videos() {
        let j = fixture("web_next_chapters");
        let sec = j
            .pointer("/contents/twoColumnWatchNextResults/secondaryResults")
            .unwrap();
        let items = collect_items(sec);
        assert!(items.len() >= 5);
        let Item::Video(v) = &items[0] else {
            panic!("expected a video: {:?}", items[0])
        };
        assert!(v.duration_s.is_some());
        assert!(v.views.is_some(), "{v:?}");
        assert!(v.published_text.is_some());
        let ch = v.channel.as_ref().unwrap();
        assert!(ch.id.starts_with("UC"), "{ch:?}");
        assert!(find_continuation(sec).is_some());
    }

    #[test]
    fn channel_search_handles_swapped_fields() {
        let j = fixture("web_search_channels");
        let ch = collect_items(&j["contents"])
            .into_iter()
            .find_map(|i| match i {
                Item::Channel(c) => Some(c),
                _ => None,
            })
            .unwrap();
        assert!(ch.handle.as_deref().unwrap().starts_with('@'));
        assert!(ch
            .subscribers_text
            .as_deref()
            .unwrap()
            .contains("subscriber"));
        assert!(ch.verified);
        assert_eq!(ch.subscribed, None, "anonymous: unknown");
    }

    #[test]
    fn music_rows() {
        let j = fixture("music_album");
        let songs: Vec<_> = collect_items(&j["contents"])
            .into_iter()
            .filter_map(|i| match i {
                Item::Song(s) => Some(s),
                _ => None,
            })
            .collect();
        assert!(!songs.is_empty());
        assert!(songs[0].duration_s.is_some());
        assert!(songs[0].set_video_id.is_some());
        assert!(songs[0].artists[0].id.starts_with("UC"));
    }

    #[test]
    fn continuation_prefers_the_page_level_token() {
        let j = fixture("web_comments");
        let page = find_continuation(&j["onResponseReceivedEndpoints"]).unwrap();
        let replies: Vec<String> = find_key(&j, "commentRepliesRenderer")
            .iter()
            .filter_map(|r| {
                find_first(r, "token")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        assert!(!replies.contains(&page));
    }
}
