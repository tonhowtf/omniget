//! music.youtube.com (WEB_REMIX client) responses -> OmniTube model.

use serde_json::Value;

use crate::core::tube::model::*;
use crate::core::tube::walk::{
    self, browse_of, chips, collect_items, endpoint, find_continuation, find_first, find_key,
    log_unknown, runs, shelves_of, str_at, text, thumbs,
};

/// `sectionListRenderer` of the first tab of a single-column browse page.
fn section_list(json: &Value) -> Option<&Value> {
    json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer")
        .or_else(|| json.pointer("/contents/sectionListRenderer"))
        .or_else(|| find_first(json.get("contents")?, "sectionListRenderer"))
}

fn music_page(json: &Value, default_kind: ShelfKind) -> Page {
    let Some(sl) = section_list(json) else {
        return music_continuation(json);
    };
    let contents = sl.get("contents").unwrap_or(&Value::Null);
    let header = json
        .pointer("/header/musicHeaderRenderer/title")
        .and_then(text)
        .map(|title| PageHeader {
            title,
            ..Default::default()
        });
    Page {
        header,
        chips: chips(sl.get("header").unwrap_or(&Value::Null)),
        shelves: shelves_of(contents, default_kind),
        continuation: sl
            .get("continuations")
            .and_then(find_continuation)
            .or_else(|| find_continuation(contents).filter(|_| !contents.is_null())),
        corrected_query: None,
    }
}

/// `browse FEmusic_home`. The mood chips come in the page itself, with the
/// browse `params` as the chip token.
pub fn music_home(json: &Value) -> Page {
    let p = music_page(json, ShelfKind::Carousel);
    log_unknown("music_home");
    p
}

/// FEmusic_explore, FEmusic_moods_and_genres(_category), FEmusic_charts,
/// FEmusic_new_releases and any other browse page.
pub fn music_browse(json: &Value) -> Page {
    let mut p = music_page(json, ShelfKind::Carousel);
    // Loose navigation buttons (explore's top row) render as a mood grid.
    for s in &mut p.shelves {
        if s.items.iter().all(|i| matches!(i, Item::Mood(_))) {
            s.kind = ShelfKind::Moods;
        }
    }
    log_unknown("music_browse");
    p
}

/// `search` on music.youtube.com. The top result card becomes the first
/// shelf; filter chips carry the search `params`.
/// Since Sep 2026 the "All" search tab sends one `itemSectionRenderer` per
/// result instead of titled shelves: fold runs of untitled lists into one.
fn merge_untitled_lists(shelves: Vec<Shelf>) -> Vec<Shelf> {
    let mut out: Vec<Shelf> = Vec::with_capacity(shelves.len());
    for shelf in shelves {
        if let Some(last) = out.last_mut() {
            if last.id != "top_result"
                && last.title.is_none()
                && shelf.title.is_none()
                && last.kind == ShelfKind::List
                && shelf.kind == ShelfKind::List
            {
                last.items.extend(shelf.items);
                continue;
            }
        }
        out.push(shelf);
    }
    out
}

pub fn music_search(json: &Value) -> Page {
    let sl = json
        .pointer(
            "/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer",
        )
        .or_else(|| section_list(json));
    let Some(sl) = sl else {
        return music_continuation(json);
    };
    let contents = sl.get("contents").unwrap_or(&Value::Null);
    let p = Page {
        header: None,
        chips: chips(sl.get("header").unwrap_or(&Value::Null)),
        shelves: merge_untitled_lists(shelves_of(contents, ShelfKind::List)),
        continuation: find_continuation(contents),
        corrected_query: super::web::corrected_query(contents).or_else(|| {
            find_first(contents, "musicDidYouMeanRenderer")
                .or_else(|| find_first(contents, "musicDidYouMeanNotFoundRenderer"))
                .and_then(|r| r.get("correctedQuery"))
                .and_then(text)
        }),
    };
    log_unknown("music_search");
    p
}

/// `continuationContents` (legacy) or `onResponseReceivedActions` (new
/// playlist pages).
pub fn music_continuation(json: &Value) -> Page {
    let mut shelves = Vec::new();
    let mut continuation = None;
    if let Some(cc) = json.get("continuationContents").and_then(Value::as_object) {
        for (k, c) in cc {
            let kind = match k.as_str() {
                "sectionListContinuation" => ShelfKind::Carousel,
                "gridContinuation" => ShelfKind::Carousel,
                _ => ShelfKind::List,
            };
            let inner = c.get("contents").or_else(|| c.get("items")).unwrap_or(c);
            shelves.extend(shelves_of(inner, kind));
            continuation = c
                .get("continuations")
                .and_then(find_continuation)
                .or_else(|| find_continuation(inner));
        }
    }
    for key in ["onResponseReceivedActions", "onResponseReceivedCommands"] {
        for a in json
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(items) = a.pointer("/appendContinuationItemsAction/continuationItems") {
                shelves.extend(shelves_of(items, ShelfKind::List));
                continuation = find_continuation(items);
            }
        }
    }
    log_unknown("music_continuation");
    Page {
        shelves: walk::finalize(shelves),
        continuation,
        ..Default::default()
    }
}

/// Album / playlist header: `musicResponsiveHeaderRenderer` (2024+) or the
/// older `musicDetailHeaderRenderer` / editable wrapper.
fn detail_header(json: &Value) -> Option<&Value> {
    find_first(json, "musicResponsiveHeaderRenderer")
        .or_else(|| json.pointer("/header/musicDetailHeaderRenderer"))
        .or_else(|| find_first(json, "musicDetailHeaderRenderer"))
}

fn split_runs(v: &Value) -> Vec<String> {
    v.get("runs")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .map(str::trim)
                .filter(|t| !t.is_empty() && *t != "•" && *t != "·")
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn header_artists(h: &Value) -> Vec<ChannelRef> {
    let src = h
        .get("straplineTextOne")
        .or_else(|| h.get("subtitle"))
        .unwrap_or(&Value::Null);
    runs(src)
        .into_iter()
        .filter_map(|r| {
            let id = r.channel_id?;
            Some(ChannelRef {
                id,
                name: r.text,
                artist: true,
                ..Default::default()
            })
        })
        .collect()
}

fn header_description(h: &Value) -> Option<String> {
    h.pointer("/description/musicDescriptionShelfRenderer/description")
        .or_else(|| h.get("description"))
        .and_then(text)
}

fn play_playlist_id(v: &Value) -> Option<String> {
    for key in ["playNavigationEndpoint", "navigationEndpoint"] {
        for ep in find_key(v, key) {
            let ep = endpoint(ep);
            let id = ep
                .pointer("/watchEndpoint/playlistId")
                .or_else(|| ep.pointer("/watchPlaylistEndpoint/playlistId"))
                .and_then(Value::as_str);
            if let Some(id) = id {
                return Some(id.to_string());
            }
        }
    }
    None
}

fn tracks_of(json: &Value) -> (Vec<SongItem>, Option<String>) {
    let root = json
        .pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents")
        .or_else(|| section_list(json))
        .unwrap_or(&Value::Null);
    let shelf = find_first(root, "musicPlaylistShelfRenderer")
        .or_else(|| find_first(root, "musicShelfRenderer"))
        .unwrap_or(&Value::Null);
    let contents = shelf.get("contents").unwrap_or(&Value::Null);
    let songs = collect_items(contents)
        .into_iter()
        .filter_map(|i| match i {
            Item::Song(s) => Some(s),
            _ => None,
        })
        .collect();
    let cont = shelf
        .get("continuations")
        .and_then(find_continuation)
        .or_else(|| find_continuation(contents))
        .or_else(|| {
            find_continuation(
                root.get("sectionListRenderer")
                    .and_then(|s| s.get("continuations"))
                    .unwrap_or(&Value::Null),
            )
        });
    (songs, cont)
}

/// `browse MPREb_...`. Tracks inherit the album cover, name and artists
/// (the rows don't repeat them).
pub fn music_album(json: &Value, browse_id: &str) -> AlbumPage {
    let h = detail_header(json).unwrap_or(&Value::Null);
    let title = h.get("title").and_then(text).unwrap_or_default();
    let sub = split_runs(h.get("subtitle").unwrap_or(&Value::Null));
    let second = split_runs(h.get("secondSubtitle").unwrap_or(&Value::Null));
    let tn = thumbs(h.get("thumbnail").unwrap_or(&Value::Null));
    let mut artists = header_artists(h);
    if artists.is_empty() {
        artists = walk::channel_from_runs(h.get("straplineTextOne").unwrap_or(&Value::Null))
            .into_iter()
            .collect();
    }
    let (mut tracks, _) = tracks_of(json);
    let playlist_id = play_playlist_id(h.get("buttons").unwrap_or(&Value::Null))
        .or_else(|| {
            json.pointer("/microformat/microformatDataRenderer/urlCanonical")
                .and_then(Value::as_str)
                .and_then(|u| u.split("list=").nth(1))
                .map(|s| s.split('&').next().unwrap_or(s).to_string())
        })
        .or_else(|| {
            find_key(
                json.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents")
                    .unwrap_or(&Value::Null),
                "playlistId",
            )
            .into_iter()
            .find_map(Value::as_str)
            .map(str::to_string)
        });
    for t in &mut tracks {
        if t.thumbs.is_empty() {
            t.thumbs = tn.clone();
        }
        if t.album.is_none() {
            t.album = Some(AlbumRef {
                id: browse_id.to_string(),
                name: title.clone(),
            });
        }
        if t.artists.is_empty() || t.artists.iter().all(|a| a.id.is_empty()) {
            t.artists = artists.clone();
        }
    }
    let other_versions = find_key(
        json.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents")
            .unwrap_or(json),
        "musicCarouselShelfRenderer",
    )
    .into_iter()
    .flat_map(|c| collect_items(c.get("contents").unwrap_or(&Value::Null)))
    .filter_map(|i| match i {
        Item::Album(a) if a.id != browse_id => Some(a),
        _ => None,
    })
    .collect();
    let explicit = find_key(h, "musicInlineBadgeRenderer")
        .iter()
        .any(|b| str_at(b, "/icon/iconType") == Some("MUSIC_EXPLICIT_BADGE"));
    log_unknown("music_album");
    AlbumPage {
        id: browse_id.to_string(),
        playlist_id,
        title,
        album_type: sub.first().cloned(),
        year: sub
            .iter()
            .find(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()))
            .cloned(),
        artists,
        thumbs: tn,
        description: header_description(h),
        track_count_text: second.first().cloned(),
        duration_text: (second.len() > 1).then(|| second.last().cloned()).flatten(),
        tracks,
        explicit,
        other_versions,
    }
}

/// `browse UC...` on music.youtube.com.
pub fn music_artist(json: &Value, browse_id: &str) -> ArtistPage {
    let h = json
        .pointer("/header/musicImmersiveHeaderRenderer")
        .or_else(|| json.pointer("/header/musicVisualHeaderRenderer"))
        .or_else(|| json.pointer("/header/musicHeaderRenderer"))
        .unwrap_or(&Value::Null);
    let sub = h
        .pointer("/subscriptionButton/subscribeButtonRenderer")
        .unwrap_or(&Value::Null);
    let mut tn = thumbs(h.get("thumbnail").unwrap_or(&Value::Null));
    if tn.is_empty() {
        tn = thumbs(h.get("foregroundThumbnail").unwrap_or(&Value::Null));
    }
    let subscribed = sub.get("subscribed").and_then(Value::as_bool).filter(|_| {
        find_first(sub, "signInEndpoint").is_none() && find_first(sub, "modalEndpoint").is_none()
    });
    let playlist_of = |key: &str| {
        h.pointer(&format!("/{key}/buttonRenderer/navigationEndpoint"))
            .and_then(|ep| {
                let ep = endpoint(ep);
                ep.pointer("/watchEndpoint/playlistId")
                    .or_else(|| ep.pointer("/watchPlaylistEndpoint/playlistId"))
            })
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let sl = section_list(json);
    let contents = sl.and_then(|s| s.get("contents")).unwrap_or(&Value::Null);
    let description = h.get("description").and_then(text).or_else(|| {
        find_first(contents, "musicDescriptionShelfRenderer")
            .and_then(|d| d.get("description"))
            .and_then(text)
    });
    let page = ArtistPage {
        id: browse_id.to_string(),
        name: h.get("title").and_then(text).unwrap_or_default(),
        thumbs: tn,
        description,
        subscribers_text: sub
            .get("subscriberCountText")
            .and_then(text)
            .or_else(|| sub.get("longSubscriberCountText").and_then(text))
            .or_else(|| h.get("monthlyListenerCount").and_then(text)),
        subscribed,
        channel_id: sub
            .get("channelId")
            .and_then(Value::as_str)
            .map(str::to_string),
        shuffle_playlist_id: playlist_of("playButton"),
        radio_playlist_id: playlist_of("startRadioButton"),
        shelves: shelves_of(contents, ShelfKind::Carousel),
    };
    log_unknown("music_artist");
    page
}

/// `browse VL<id>` on music.youtube.com.
pub fn music_playlist(json: &Value, id: &str) -> PlaylistPage {
    let editable = find_first(json, "musicEditablePlaylistDetailHeaderRenderer").is_some();
    let h = detail_header(json).unwrap_or(&Value::Null);
    let second = split_runs(h.get("secondSubtitle").unwrap_or(&Value::Null));
    let owner = h
        .get("straplineTextOne")
        .and_then(walk::channel_from_runs)
        .or_else(|| {
            h.pointer("/facepile/avatarStackViewModel/text")
                .and_then(text)
                .map(|name| ChannelRef {
                    name,
                    ..Default::default()
                })
        })
        .or_else(|| {
            // Old header: "Playlist • Owner • 2024"
            let sub = h.get("subtitle")?;
            walk::channel_from_runs(sub).filter(|c| !c.id.is_empty())
        });
    let (views_text, count_text, duration_text) = match second.len() {
        0 => (None, None, None),
        1 => (None, second.first().cloned(), None),
        2 => (None, second.first().cloned(), second.get(1).cloned()),
        _ => (
            second.first().cloned(),
            second.get(1).cloned(),
            second.last().cloned(),
        ),
    };
    let (tracks, continuation) = tracks_of(json);
    log_unknown("music_playlist");
    PlaylistPage {
        id: id.strip_prefix("VL").unwrap_or(id).to_string(),
        title: h.get("title").and_then(text).unwrap_or_default(),
        owner,
        thumbs: thumbs(h.get("thumbnail").unwrap_or(&Value::Null)),
        description: header_description(h),
        count_text,
        views_text,
        updated_text: None,
        items: tracks.into_iter().map(Item::Song).collect(),
        continuation,
        editable,
        is_music: true,
        duration_text,
    }
}

fn tab_browse_id(json: &Value, page_type: &str, prefix: &str) -> Option<String> {
    find_key(
        json.pointer("/contents/singleColumnMusicWatchNextResultsRenderer")
            .unwrap_or(json),
        "tabRenderer",
    )
    .into_iter()
    .filter_map(|t| t.get("endpoint").and_then(browse_of))
    .find(|(id, _, pt)| *pt == Some(page_type) || id.starts_with(prefix))
    .map(|(id, _, _)| id.to_string())
}

/// `next` on music.youtube.com (song, radio or its continuation).
pub fn music_queue(json: &Value) -> QueuePage {
    let panel = find_first(json, "playlistPanelRenderer")
        .or_else(|| json.pointer("/continuationContents/playlistPanelContinuation"))
        .unwrap_or(&Value::Null);
    let tracks = panel
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            c.get("playlistPanelVideoRenderer").or_else(|| {
                c.pointer(
                    "/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer",
                )
            })
        })
        .filter_map(walk::song_from_panel)
        .collect();
    QueuePage {
        playlist_id: panel
            .get("playlistId")
            .and_then(Value::as_str)
            .map(str::to_string),
        tracks,
        continuation: panel.get("continuations").and_then(find_continuation),
        lyrics_browse_id: tab_browse_id(json, "MUSIC_PAGE_TYPE_TRACK_LYRICS", "MPLY"),
        related_browse_id: tab_browse_id(json, "MUSIC_PAGE_TYPE_TRACK_RELATED", "MPTR"),
    }
}

/// Lyrics tab (`browse MPLY...`): plain text from
/// `musicDescriptionShelfRenderer`, or timed lines (`timedLyricsModel`)
/// when the client gets them. `None` when the song has no lyrics.
pub fn music_lyrics(json: &Value) -> Option<Lyrics> {
    if let Some(timed) = find_first(json, "timedLyricsModel") {
        let lines: Vec<LyricLine> = timed
            .pointer("/lyricsData/timedLyricsData")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let text = l.get("lyricLine").and_then(Value::as_str)?.to_string();
                let start = l
                    .pointer("/cueRange/startTimeMilliseconds")
                    .and_then(|v| {
                        v.as_str()
                            .and_then(|s| s.parse().ok())
                            .or_else(|| v.as_u64())
                    })
                    .map(|n: u64| n as u32);
                Some(LyricLine {
                    start_ms: start,
                    text,
                })
            })
            .collect();
        if !lines.is_empty() {
            return Some(Lyrics {
                source: "youtube".into(),
                synced: lines.iter().any(|l| l.start_ms.is_some()),
                lines,
                footer: timed.pointer("/lyricsData/sourceMessage").and_then(text),
            });
        }
    }
    let shelf = find_first(json.get("contents")?, "musicDescriptionShelfRenderer")?;
    let body = shelf.get("description").and_then(text)?;
    Some(Lyrics {
        source: "youtube".into(),
        synced: false,
        lines: body
            .lines()
            .map(|l| LyricLine {
                start_ms: None,
                text: l.trim_end().to_string(),
            })
            .collect(),
        footer: shelf.get("footer").and_then(text),
    })
}

/// Related tab (`browse MPTR...`).
pub fn music_related(json: &Value) -> Page {
    let p = music_page(json, ShelfKind::Carousel);
    log_unknown("music_related");
    p
}

/// Library pages (FEmusic_liked_playlists, FEmusic_liked_videos,
/// FEmusic_library_corpus_track_artists, FEmusic_liked_albums,
/// FEmusic_history...). Song lists come as List shelves, cards as grids.
pub fn music_library(json: &Value) -> Page {
    let p = if json.get("continuationContents").is_some()
        || json.get("onResponseReceivedActions").is_some()
    {
        music_continuation(json)
    } else {
        let sl = section_list(json).or_else(|| {
            find_first(
                json.pointer("/contents/twoColumnBrowseResultsRenderer")?,
                "sectionListRenderer",
            )
        });
        let Some(sl) = sl else {
            return Page::default();
        };
        let contents = sl.get("contents").unwrap_or(&Value::Null);
        let mut page = Page {
            chips: chips(sl.get("header").unwrap_or(&Value::Null)),
            shelves: shelves_of(contents, ShelfKind::Carousel),
            continuation: find_continuation(contents),
            ..Default::default()
        };
        if let Some(t) = json
            .pointer("/header/musicHeaderRenderer/title")
            .and_then(text)
        {
            page.header = Some(PageHeader {
                title: t,
                ..Default::default()
            });
        }
        page
    };
    log_unknown("music_library");
    p
}

/// `music/get_search_suggestions`: text suggestions only (the second
/// section holds item shortcuts).
pub fn music_suggestions(json: &Value) -> Vec<String> {
    find_key(json, "searchSuggestionRenderer")
        .into_iter()
        .chain(find_key(json, "historySuggestionRenderer"))
        .filter_map(|s| s.get("suggestion").and_then(text))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tube::walk::testutil::{external, fixture};

    fn items(p: &Page) -> Vec<Item> {
        p.shelves.iter().flat_map(|s| s.items.clone()).collect()
    }

    #[test]
    fn home_quick_picks_and_chips() {
        for f in ["music_home", "music_home_pt"] {
            let p = music_home(&fixture(f));
            assert!(p.chips.len() >= 3, "{f}");
            assert!(p.chips.iter().all(|c| c.token.is_some()), "{f}");
            assert!(p.shelves.len() >= 2, "{f}");
            let qp = p
                .shelves
                .iter()
                .find(|s| s.kind == ShelfKind::QuickPicks)
                .expect("quick picks");
            let Item::Song(s) = &qp.items[0] else {
                panic!()
            };
            assert!(!s.artists.is_empty() && s.artists[0].id.starts_with("UC"));
            assert!(s.album.is_some());
            assert!(s.plays_text.is_some());
            assert!(p.shelves.iter().any(|s| s.kind == ShelfKind::Carousel));
            assert!(p.continuation.is_some(), "{f}");
        }
        let c = music_continuation(&fixture("music_home_cont"));
        assert!(!c.shelves.is_empty());
    }

    #[test]
    fn explore_moods_charts() {
        let e = music_browse(&fixture("music_explore"));
        assert!(e.shelves.iter().any(|s| s.kind == ShelfKind::Moods));
        assert!(e
            .shelves
            .iter()
            .any(|s| s.kind == ShelfKind::Carousel && s.title.is_some()));

        let m = music_browse(&fixture("music_moods"));
        let all = items(&m);
        let moods: Vec<&MoodItem> = all
            .iter()
            .filter_map(|i| match i {
                Item::Mood(m) => Some(m),
                _ => None,
            })
            .collect();
        assert!(moods.len() >= 10);
        assert!(moods.iter().all(|m| m.params.is_some()));
        assert!(moods.iter().all(|m| m
            .color
            .as_deref()
            .is_some_and(|c| c.len() == 7 && c.starts_with('#'))));
        assert!(m.header.is_some());

        let cat = music_browse(&fixture("music_mood_category"));
        assert!(cat.shelves.len() >= 2);
        assert!(items(&cat)
            .iter()
            .any(|i| matches!(i, Item::Playlist(p) if p.is_music)));

        let ch = music_browse(&fixture("music_charts"));
        assert!(!ch.shelves.is_empty());
        assert!(items(&ch).len() >= 5);

        let nr = music_browse(&fixture("music_new_releases"));
        assert!(items(&nr)
            .iter()
            .any(|i| matches!(i, Item::Album(a) if !a.artists.is_empty())));
    }

    #[test]
    fn search_top_result_and_filters() {
        let p = music_search(&fixture("music_search"));
        assert!(p.chips.len() >= 4);
        assert!(p.chips.iter().all(|c| c.token.is_some()));
        let top = &p.shelves[0];
        assert!(
            matches!(top.items[0], Item::Artist(ref a) if a.name == "Daft Punk"),
            "{:?}",
            top.items[0]
        );
        // top result + the flat list of the new layout
        assert!(p.shelves.len() >= 2);
        assert!(p.shelves[1].items.len() >= 5);

        let songs = music_search(&fixture("music_search_songs"));
        let all = items(&songs);
        let s: Vec<&SongItem> = all
            .iter()
            .filter_map(|i| match i {
                Item::Song(s) => Some(s),
                _ => None,
            })
            .collect();
        assert!(s.len() >= 8);
        assert!(s
            .iter()
            .all(|s| s.duration_s.is_some() && !s.artists.is_empty()));
        assert!(s.iter().any(|s| s.album.is_some()));
        assert!(songs.continuation.is_some());

        let albums = items(&music_search(&fixture("music_search_albums")));
        let a: Vec<&AlbumItem> = albums
            .iter()
            .filter_map(|i| match i {
                Item::Album(a) => Some(a),
                _ => None,
            })
            .collect();
        assert!(a.len() >= 5);
        assert!(a.iter().all(|a| a.id.starts_with("MPRE")));
        assert!(a.iter().any(|a| a.year.is_some()));
    }

    #[test]
    fn album() {
        let a = music_album(&fixture("music_album"), "MPREb_K8qWMWVqXGi");
        assert_eq!(a.title, "Random Access Memories");
        assert_eq!(a.album_type.as_deref(), Some("Album"));
        assert_eq!(a.year.as_deref(), Some("2013"));
        assert_eq!(a.artists[0].name, "Daft Punk");
        assert!(a.artists[0].id.starts_with("UC"));
        assert!(a.playlist_id.as_deref().unwrap().starts_with("OLAK"));
        assert!(a.description.is_some());
        assert!(a.track_count_text.as_deref().unwrap().contains("songs"));
        assert!(a.duration_text.is_some());
        assert!(a.tracks.len() >= 10);
        let t = &a.tracks[0];
        assert_eq!(t.album.as_ref().unwrap().id, "MPREb_K8qWMWVqXGi");
        assert!(!t.thumbs.is_empty());
        assert!(t.duration_s.is_some());
        assert!(!a.other_versions.is_empty());
    }

    #[test]
    fn artist() {
        let a = music_artist(&fixture("music_artist"), "UCRr1xG_2WIDs18a6cIiCxeA");
        assert_eq!(a.name, "Daft Punk");
        assert!(!a.thumbs.is_empty());
        assert!(a.description.is_some());
        assert!(a.subscribers_text.is_some());
        assert!(a.channel_id.is_some());
        assert!(a.shuffle_playlist_id.is_some());
        assert!(a.radio_playlist_id.is_some());
        assert!(a.shelves.iter().any(|s| s.kind == ShelfKind::List));
        assert!(a.shelves.iter().any(|s| s.kind == ShelfKind::Carousel));
        assert!(a
            .shelves
            .iter()
            .flat_map(|s| &s.items)
            .any(|i| matches!(i, Item::Album(_))));
    }

    #[test]
    fn playlists() {
        let p = music_playlist(
            &fixture("music_playlist"),
            "VLRDCLAK5uy_ntOL7kXTRxvIvmIsrYzKQ_MEpBdIywpQc",
        );
        assert_eq!(p.id, "RDCLAK5uy_ntOL7kXTRxvIvmIsrYzKQ_MEpBdIywpQc");
        assert!(!p.title.is_empty());
        assert!(p.is_music);
        assert!(p.items.len() >= 10);
        assert!(!p.thumbs.is_empty());
        assert!(!p.editable);

        let u = music_playlist(
            &fixture("music_playlist_ugc"),
            "PL4fGSI1pDJn6puJdseH2Rt9sMvt9E2M4i",
        );
        assert!(u.items.len() >= 10);
        assert!(u.count_text.is_some());

        let l = music_playlist(
            &fixture("music_playlist_long"),
            "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI",
        );
        assert!(l.continuation.is_some());
        let c = music_continuation(&fixture("music_playlist_cont"));
        assert!(
            items(&c)
                .iter()
                .filter(|i| matches!(i, Item::Song(_)))
                .count()
                >= 5
        );
    }

    #[test]
    fn queue_radio_and_tabs() {
        let q = music_queue(&fixture("music_next_radio"));
        assert!(q.tracks.len() >= 10);
        assert_eq!(q.playlist_id.as_deref(), Some("RDAMVM5NV6Rdv1a3I"));
        assert!(q.continuation.is_some());
        assert!(q.lyrics_browse_id.as_deref().unwrap().starts_with("MPLY"));
        assert!(q.related_browse_id.as_deref().unwrap().starts_with("MPTR"));
        let t = &q.tracks[1];
        assert!(t.duration_s.is_some() && !t.artists.is_empty() && t.set_video_id.is_some());

        let c = music_queue(&fixture("music_next_radio_cont"));
        assert!(c.tracks.len() >= 10);
        assert!(c.continuation.is_some());

        let single = music_queue(&fixture("music_next"));
        assert_eq!(single.tracks.len(), 1);
        assert!(single.lyrics_browse_id.is_some());
    }

    #[test]
    fn lyrics() {
        assert!(
            music_lyrics(&fixture("music_lyrics")).is_none(),
            "anonymous: not available"
        );
        let Some(j) = external("nick42d__youtui/ytmapi-rs/test_json/get_lyrics_20231219.json")
        else {
            eprintln!("skip: ytmapi-rs test_json not present");
            return;
        };
        let l = music_lyrics(&j).unwrap();
        assert!(l.lines.len() > 5);
        assert!(!l.synced);
        assert!(l.footer.is_some());
    }

    #[test]
    fn related_tab() {
        let r = music_related(&fixture("music_related"));
        assert!(r.shelves.len() >= 2);
    }

    #[test]
    fn suggestions() {
        let s = music_suggestions(&fixture("music_suggest"));
        assert!(s.len() >= 3);
        assert!(s.iter().all(|s| s.to_lowercase().contains("daft")));
    }

    #[test]
    fn library_from_references() {
        let rp = [
            "ThetaDev__rustypipe/testfiles/music_userdata/saved_playlists.json",
            "ThetaDev__rustypipe/testfiles/music_userdata/saved_albums.json",
            "ThetaDev__rustypipe/testfiles/music_userdata/saved_artists.json",
            "ThetaDev__rustypipe/testfiles/music_userdata/saved_tracks.json",
            "ThetaDev__rustypipe/testfiles/music_userdata/music_history.json",
            "nick42d__youtui/ytmapi-rs/test_json/get_library_playlists.json",
            "nick42d__youtui/ytmapi-rs/test_json/get_library_songs_20240701.json",
            "nick42d__youtui/ytmapi-rs/test_json/get_library_albums_20240701.json",
            "nick42d__youtui/ytmapi-rs/test_json/get_library_artists.json",
            "nick42d__youtui/ytmapi-rs/test_json/get_history_20240713.json",
        ];
        let mut seen = 0;
        for f in rp {
            let Some(j) = external(f) else { continue };
            seen += 1;
            let p = music_library(&j);
            assert!(!items(&p).is_empty(), "{f}");
            if f.contains("tracks") || f.contains("songs") || f.contains("history") {
                assert!(items(&p).iter().any(|i| matches!(i, Item::Song(_))), "{f}");
            }
            if f.contains("albums") {
                assert!(items(&p).iter().any(|i| matches!(i, Item::Album(_))), "{f}");
            }
            if f.contains("artists") {
                assert!(
                    items(&p).iter().any(|i| matches!(i, Item::Artist(_))),
                    "{f}"
                );
            }
        }
        if seen == 0 {
            eprintln!("skip: reference fixtures not present");
        }
    }

    #[test]
    fn references_album_artist_playlist() {
        if let Some(j) = external("nick42d__youtui/ytmapi-rs/test_json/get_artist_20250310.json") {
            let a = music_artist(&j, "UC");
            assert!(!a.name.is_empty());
            assert!(!a.shelves.is_empty());
        }
        if let Some(j) = external("nick42d__youtui/ytmapi-rs/test_json/get_album_20240724.json") {
            let a = music_album(&j, "MPREb_x");
            assert!(!a.title.is_empty());
            assert!(!a.tracks.is_empty());
        }
        if let Some(j) = external("nick42d__youtui/ytmapi-rs/test_json/get_playlist_20250604.json")
        {
            let p = music_playlist(&j, "VLPLx");
            assert!(!p.title.is_empty());
            assert!(!p.items.is_empty());
        }
        if let Some(j) =
            external("nick42d__youtui/ytmapi-rs/test_json/get_watch_playlist_20250630.json")
        {
            let q = music_queue(&j);
            assert!(!q.tracks.is_empty());
        }
    }
}
