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

pub fn matches_word(text: &str, word: &str, whole_word: bool) -> bool {
    contains_folded(&fold(text), &fold(word.trim()), whole_word)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tube::model::{
        ChannelItem, ChannelRef, PlaylistItem, Shelf, ShelfKind, ShortItem, SongItem, VideoItem,
    };
    use crate::models::settings::TubeBlockedChannel;

    fn ch(id: &str, handle: Option<&str>) -> Option<ChannelRef> {
        Some(ChannelRef {
            id: id.into(),
            name: "N".into(),
            handle: handle.map(Into::into),
            ..Default::default()
        })
    }

    fn video(id: &str, title: &str) -> Item {
        Item::Video(VideoItem {
            id: id.into(),
            title: title.into(),
            channel: ch("UCa", Some("@alpha")),
            duration_s: Some(600),
            ..Default::default()
        })
    }

    fn with_badge(id: &str, b: Badge) -> Item {
        match video(id, "t") {
            Item::Video(mut v) => {
                v.badges.push(b);
                Item::Video(v)
            }
            _ => unreachable!(),
        }
    }

    fn short(id: &str) -> Item {
        Item::Short(ShortItem {
            id: id.into(),
            title: "s".into(),
            ..Default::default()
        })
    }

    fn none(_: &str) -> Option<u8> {
        None
    }

    #[test]
    fn fold_and_whole_word() {
        assert_eq!(fold("ÁRVORE Çà"), "arvore ca");
        assert!(matches_word("O melhor CAFÉ do mundo", "cafe", true));
        assert!(matches_word("Café!", "CAFÉ", true));
        assert!(!matches_word("cafeteria", "cafe", true));
        assert!(matches_word("cafeteria", "cafe", false));
        assert!(!matches_word("minecrafting", "minecraft", true));
        assert!(matches_word("#minecraft live", "minecraft", true));
        assert!(matches_word("top 10 games", "top 10", true));
        assert!(!matches_word("top 100 games", "top 10", true));
        assert!(!matches_word("x_minecraft", "minecraft", true));
        assert!(matches_word("日本語のタイトル", "タイトル", false));
        assert!(matches_word("C++ tutorial", "c++", true));
        assert!(!matches_word("anything", "", true));
    }

    #[test]
    fn from_settings_maps_fields() {
        let s = TubeSettings {
            hide_shorts: true,
            blocked_channels: vec![
                TubeBlockedChannel {
                    id: "UCx".into(),
                    name: "X".into(),
                },
                TubeBlockedChannel {
                    id: " ".into(),
                    name: String::new(),
                },
            ],
            blocked_words: vec![" spoiler ".into(), "".into()],
            ..TubeSettings::default()
        };
        let p = FilterPrefs::from_settings(&s);
        assert!(p.hide_shorts);
        assert!(p.hide_members);
        assert_eq!(p.blocked_channels, vec!["UCx"]);
        assert_eq!(p.blocked_words, vec!["spoiler"]);
        assert_eq!(p.watched_threshold, 85);
    }

    #[test]
    fn type_filters() {
        let p = FilterPrefs {
            hide_shorts: true,
            hide_live: true,
            hide_upcoming: true,
            hide_members: true,
            hide_mixes: true,
            ..Default::default()
        };
        let ctx = FilterCtx::Home;
        assert!(!passes(&short("s"), &p, ctx, &none));
        assert!(!passes(&with_badge("a", Badge::Live), &p, ctx, &none));
        assert!(!passes(&with_badge("a", Badge::Upcoming), &p, ctx, &none));
        assert!(!passes(&with_badge("a", Badge::Premiere), &p, ctx, &none));
        assert!(!passes(&with_badge("a", Badge::Members), &p, ctx, &none));
        assert!(passes(&with_badge("a", Badge::Hd4k), &p, ctx, &none));
        let mix = Item::Playlist(PlaylistItem {
            id: "RDx".into(),
            title: "Mix".into(),
            is_mix: true,
            ..Default::default()
        });
        assert!(!passes(&mix, &p, ctx, &none));
        assert!(passes(&short("s"), &FilterPrefs::default(), ctx, &none));
    }

    #[test]
    fn duration_limits_skip_live() {
        let p = FilterPrefs {
            min_duration_s: 120,
            max_duration_s: 3600,
            ..Default::default()
        };
        let mk = |d: u32| match video("a", "t") {
            Item::Video(mut v) => {
                v.duration_s = Some(d);
                Item::Video(v)
            }
            _ => unreachable!(),
        };
        assert!(!passes(&mk(60), &p, FilterCtx::Search, &none));
        assert!(passes(&mk(600), &p, FilterCtx::Search, &none));
        assert!(!passes(&mk(7200), &p, FilterCtx::Search, &none));
        assert!(passes(
            &with_badge("l", Badge::Live),
            &p,
            FilterCtx::Search,
            &none
        ));
    }

    #[test]
    fn blocked_channels_and_contexts() {
        let p = FilterPrefs {
            blocked_channels: vec!["UCa".into()],
            ..Default::default()
        };
        let v = video("a", "t");
        assert!(!passes(&v, &p, FilterCtx::Home, &none));
        assert!(!passes(&v, &p, FilterCtx::Related, &none));
        assert!(passes(&v, &p, FilterCtx::Subscriptions, &none));
        assert!(passes(&v, &p, FilterCtx::Channel, &none));
        let by_handle = FilterPrefs {
            blocked_channels: vec!["@Alpha".into()],
            ..Default::default()
        };
        assert!(!passes(&v, &by_handle, FilterCtx::Search, &none));
        let channel = Item::Channel(ChannelItem {
            id: "UCa".into(),
            name: "Alpha".into(),
            ..Default::default()
        });
        assert!(!passes(&channel, &p, FilterCtx::Search, &none));
        let song = Item::Song(SongItem {
            id: "s".into(),
            title: "Song".into(),
            artists: vec![ChannelRef {
                id: "UCa".into(),
                name: "Alpha".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(!passes(&song, &p, FilterCtx::Music, &none));
    }

    #[test]
    fn words_whole_and_substring() {
        let p = FilterPrefs {
            blocked_words: vec!["Spoiler".into()],
            whole_word: true,
            ..Default::default()
        };
        assert!(!passes(
            &video("a", "Big SPOILER inside"),
            &p,
            FilterCtx::Home,
            &none
        ));
        assert!(passes(
            &video("a", "spoilers everywhere"),
            &p,
            FilterCtx::Home,
            &none
        ));
        let sub = FilterPrefs {
            whole_word: false,
            ..p.clone()
        };
        assert!(!passes(
            &video("a", "spoilers everywhere"),
            &sub,
            FilterCtx::Home,
            &none
        ));
        // Words also apply on YT Music, where type filters do not.
        let music = FilterPrefs {
            hide_shorts: true,
            ..p
        };
        assert!(passes(&short("x"), &music, FilterCtx::Music, &none));
    }

    #[test]
    fn watched_threshold_and_playlist_exception() {
        let p = FilterPrefs {
            hide_watched: true,
            watched_threshold: 85,
            ..Default::default()
        };
        let watched = |id: &str| match id {
            "seen" => Some(90),
            "half" => Some(50),
            _ => None,
        };
        assert!(!passes(&video("seen", "t"), &p, FilterCtx::Home, &watched));
        assert!(passes(&video("half", "t"), &p, FilterCtx::Home, &watched));
        assert!(passes(
            &video("seen", "t"),
            &p,
            FilterCtx::Playlist,
            &watched
        ));
        let account_pct = match video("acc", "t") {
            Item::Video(mut v) => {
                v.watched_pct = Some(100);
                Item::Video(v)
            }
            _ => unreachable!(),
        };
        assert!(!passes(&account_pct, &p, FilterCtx::Subscriptions, &none));
    }

    fn shelf(id: &str, items: Vec<Item>, kind: ShelfKind) -> Shelf {
        Shelf {
            id: id.into(),
            items,
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn page_drops_emptied_shelves_only() {
        let p = FilterPrefs {
            hide_shorts: true,
            ..Default::default()
        };
        let mut page = Page {
            shelves: vec![
                shelf("0", vec![video("a", "t"), short("s1")], ShelfKind::Grid),
                shelf("1", vec![short("s2"), short("s3")], ShelfKind::Shorts),
                shelf("2", vec![], ShelfKind::Grid),
            ],
            ..Default::default()
        };
        apply_page(&mut page, &p, FilterCtx::Home, &none);
        let ids: Vec<&str> = page.shelves.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["0", "2"]);
        assert_eq!(page.shelves[0].items.len(), 1);
    }

    #[test]
    fn channel_shorts_and_members_tabs_keep_their_items() {
        let p = FilterPrefs {
            hide_shorts: true,
            hide_members: true,
            ..Default::default()
        };
        let mut shorts_tab = Page {
            shelves: vec![shelf("0", vec![short("a"), short("b")], ShelfKind::Grid)],
            ..Default::default()
        };
        apply_page(&mut shorts_tab, &p, FilterCtx::Channel, &none);
        assert_eq!(shorts_tab.shelves[0].items.len(), 2);

        let mut members_tab = vec![
            with_badge("a", Badge::Members),
            with_badge("b", Badge::Members),
        ];
        apply_items(&mut members_tab, &p, FilterCtx::Channel, &none);
        assert_eq!(members_tab.len(), 2);

        // Channel home with mixed content still loses them.
        let mut home = vec![video("v", "t"), short("s"), with_badge("m", Badge::Members)];
        apply_items(&mut home, &p, FilterCtx::Channel, &none);
        assert_eq!(home.len(), 1);

        // Outside a channel, an all-Shorts list is still filtered.
        let mut search = vec![short("a"), short("b")];
        apply_items(&mut search, &p, FilterCtx::Search, &none);
        assert!(search.is_empty());
    }
}
