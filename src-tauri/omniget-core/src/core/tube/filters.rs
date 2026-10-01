//! The one distraction filter for every OmniTube list. It removes items
//! and keeps the structure (FilterTube `filter_logic.js`, MIT): a shelf
//! only goes away when filtering left it empty. Ads never reach here; the
//! renderer walker drops them.
//!
//! Context exceptions (from `re/_distracoes-catalogo.md`):
//! - a channel's Shorts tab keeps its Shorts, and its Members tab keeps
//!   members-only videos (detected as "every item is of that kind");
//! - a blocked channel still shows in Subscriptions and on its own page;
//! - YT Music only gets channel and word blocks.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use super::model::{Badge, ChannelRef, Item, Page};
use crate::models::settings::TubeSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCtx {
    Home,
    Search,
    Related,
    Channel,
    Subscriptions,
    Playlist,
    Music,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterPrefs {
    pub hide_shorts: bool,
    pub hide_live: bool,
    pub hide_upcoming: bool,
    pub hide_members: bool,
    pub hide_mixes: bool,
    pub hide_playables: bool,
    pub hide_posts: bool,
    pub hide_watched: bool,
    pub watched_threshold: u8,
    /// Channel ids (`UC...`) or handles (`@name`).
    pub blocked_channels: Vec<String>,
    pub blocked_words: Vec<String>,
    pub whole_word: bool,
    /// 0 = no limit.
    pub min_duration_s: u32,
    pub max_duration_s: u32,
}

impl FilterPrefs {
    pub fn from_settings(s: &TubeSettings) -> Self {
        Self {
            hide_shorts: s.hide_shorts,
            hide_live: s.hide_live,
            hide_upcoming: s.hide_upcoming,
            hide_members: s.hide_members,
            hide_mixes: s.hide_mixes,
            hide_playables: s.hide_playables,
            hide_posts: s.hide_posts,
            hide_watched: s.hide_watched,
            watched_threshold: s.watched_threshold,
            blocked_channels: s
                .blocked_channels
                .iter()
                .map(|c| c.id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect(),
            blocked_words: s
                .blocked_words
                .iter()
                .map(|w| w.trim().to_string())
                .filter(|w| !w.is_empty())
                .collect(),
            whole_word: s.whole_word,
            min_duration_s: s.min_duration_s,
            max_duration_s: s.max_duration_s,
        }
    }
}

/// Lowercase, accents stripped (NFD minus combining marks).
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `needle` inside `hay`, both already folded. Whole word = Unicode
/// letter/number boundaries on both sides (FilterTube "Exact").
fn contains_folded(hay: &str, needle: &str, whole_word: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    if !whole_word {
        return hay.contains(needle);
    }
    let mut start = 0;
    while let Some(pos) = hay[start..].find(needle) {
        let at = start + pos;
        let end = at + needle.len();
        let before_ok = hay[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c))
            || !needle.chars().next().is_some_and(is_word_char);
        let after_ok = hay[end..].chars().next().is_none_or(|c| !is_word_char(c))
            || !needle.chars().next_back().is_some_and(is_word_char);
        if before_ok && after_ok {
            return true;
        }
        start = at + hay[at..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

fn title_of(item: &Item) -> &str {
    match item {
        Item::Video(v) => &v.title,
        Item::Short(s) => &s.title,
        Item::Channel(c) => &c.name,
        Item::Playlist(p) => &p.title,
        Item::Song(s) => &s.title,
        Item::Album(a) => &a.title,
        Item::Artist(a) => &a.name,
        Item::Mood(m) => &m.title,
    }
}

fn one(c: &Option<ChannelRef>) -> Vec<(&str, Option<&str>)> {
    c.as_ref()
        .map(|c| vec![(c.id.as_str(), c.handle.as_deref())])
        .unwrap_or_default()
}

/// (id, handle) of every channel an item belongs to.
fn channels_of(item: &Item) -> Vec<(&str, Option<&str>)> {
    match item {
        Item::Video(v) => one(&v.channel),
        Item::Short(s) => one(&s.channel),
        Item::Playlist(p) => one(&p.channel),
        Item::Channel(c) => vec![(c.id.as_str(), c.handle.as_deref())],
        Item::Artist(a) => vec![(a.id.as_str(), None)],
        Item::Song(s) => s
            .artists
            .iter()
            .map(|a| (a.id.as_str(), a.handle.as_deref()))
            .collect(),
        Item::Album(a) => a
            .artists
            .iter()
            .map(|a| (a.id.as_str(), a.handle.as_deref()))
            .collect(),
        Item::Mood(_) => Vec::new(),
    }
}

fn is_blocked_channel(item: &Item, blocked: &[String]) -> bool {
    if blocked.is_empty() {
        return false;
    }
    channels_of(item).into_iter().any(|(id, handle)| {
        blocked.iter().any(|b| match b.strip_prefix('@') {
            Some(want) => {
                handle.is_some_and(|h| h.trim_start_matches('@').eq_ignore_ascii_case(want))
            }
            None => !id.is_empty() && b == id,
        })
    })
}

fn has_badge(item: &Item, badge: Badge) -> bool {
    matches!(item, Item::Video(v) if v.badges.contains(&badge))
}

fn video_id(item: &Item) -> Option<&str> {
    match item {
        Item::Video(v) => Some(&v.id),
        Item::Short(s) => Some(&s.id),
        _ => None,
    }
}

/// Tab-level exceptions computed once per list.
#[derive(Clone, Copy, Default)]
struct Exempt {
    shorts: bool,
    members: bool,
}

fn exemptions<'a>(items: impl Iterator<Item = &'a Item>, ctx: FilterCtx) -> Exempt {
    if ctx != FilterCtx::Channel {
        return Exempt::default();
    }
    let mut any = false;
    let mut all_shorts = true;
    let mut all_members = true;
    for it in items {
        any = true;
        all_shorts &= matches!(it, Item::Short(_));
        all_members &= has_badge(it, Badge::Members);
    }
    Exempt {
        shorts: any && all_shorts,
        members: any && all_members,
    }
}

fn passes_with(
    item: &Item,
    p: &FilterPrefs,
    ctx: FilterCtx,
    watched: &dyn Fn(&str) -> Option<u8>,
    ex: Exempt,
) -> bool {
    let music = ctx == FilterCtx::Music;
    if !matches!(ctx, FilterCtx::Subscriptions | FilterCtx::Channel)
        && is_blocked_channel(item, &p.blocked_channels)
    {
        return false;
    }
    if !matches!(item, Item::Mood(_)) {
        let title = fold(title_of(item));
        if p.blocked_words
            .iter()
            .any(|w| contains_folded(&title, &fold(w), p.whole_word))
        {
            return false;
        }
    }
    if music {
        return true;
    }
    if p.hide_shorts && matches!(item, Item::Short(_)) && !ex.shorts {
        return false;
    }
    if p.hide_mixes && matches!(item, Item::Playlist(pl) if pl.is_mix) {
        return false;
    }
    if let Item::Video(v) = item {
        if p.hide_live && v.badges.contains(&Badge::Live) {
            return false;
        }
        if p.hide_upcoming
            && (v.badges.contains(&Badge::Upcoming) || v.badges.contains(&Badge::Premiere))
        {
            return false;
        }
        if p.hide_members && v.badges.contains(&Badge::Members) && !ex.members {
            return false;
        }
        let live = v.badges.contains(&Badge::Live) || v.badges.contains(&Badge::Upcoming);
        if let (Some(d), false) = (v.duration_s, live) {
            if p.min_duration_s > 0 && d < p.min_duration_s {
                return false;
            }
            if p.max_duration_s > 0 && d > p.max_duration_s {
                return false;
            }
        }
    }
    if p.hide_watched && !matches!(ctx, FilterCtx::Playlist) {
        if let Some(id) = video_id(item) {
            let pct = watched(id).or(match item {
                Item::Video(v) => v.watched_pct,
                _ => None,
            });
            if pct.is_some_and(|pct| pct >= p.watched_threshold.max(1)) {
                return false;
            }
        }
    }
    true
}

pub fn passes(
    item: &Item,
    p: &FilterPrefs,
    ctx: FilterCtx,
    watched: &dyn Fn(&str) -> Option<u8>,
) -> bool {
    passes_with(item, p, ctx, watched, Exempt::default())
}

pub fn apply_items(
    items: &mut Vec<Item>,
    p: &FilterPrefs,
    ctx: FilterCtx,
    watched: &dyn Fn(&str) -> Option<u8>,
) {
    let ex = exemptions(items.iter(), ctx);
    items.retain(|it| passes_with(it, p, ctx, watched, ex));
}

/// Filters every shelf and drops the ones filtering emptied. Shelves that
/// arrived empty (a header-only section) are left alone.
pub fn apply_page(
    page: &mut Page,
    p: &FilterPrefs,
    ctx: FilterCtx,
    watched: &dyn Fn(&str) -> Option<u8>,
) {
    let ex = exemptions(page.shelves.iter().flat_map(|s| s.items.iter()), ctx);
    page.shelves.retain_mut(|shelf| {
        if shelf.items.is_empty() {
            return true;
        }
        shelf
            .items
            .retain(|it| passes_with(it, p, ctx, watched, ex));
        !shelf.items.is_empty()
    });
}
