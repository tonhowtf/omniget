use omniget_core::core::tube::filters::FilterCtx;
use omniget_core::core::tube::parse::music as pm;
use omniget_core::core::tube::{
    innertube, lrc, AlbumPage, ArtistPage, Lyrics, Page, PlaylistPage, QueuePage, Shelf, ShelfKind,
};
use serde_json::{json, Value};

use super::browse::{filter_items, filter_page};
use super::{account, http};

async fn browse(browse_id: &str, params: Option<&str>) -> Result<Value, String> {
    http::music("browse", |c, v, hl, gl| {
        innertube::browse_body(c, v, hl, gl, browse_id, params)
    })
    .await
}

#[tauri::command]
pub async fn tube_music_home(params: Option<String>) -> Result<Page, String> {
    let json = browse("FEmusic_home", params.as_deref().filter(|p| !p.is_empty())).await?;
    let mut page = pm::music_home(&json);
    filter_page(&mut page, FilterCtx::Music);
    Ok(page)
}

#[tauri::command]
pub async fn tube_music_explore() -> Result<Page, String> {
    let mut page = pm::music_browse(&browse("FEmusic_explore", None).await?);
    filter_page(&mut page, FilterCtx::Music);
    Ok(page)
}

#[tauri::command]
pub async fn tube_music_browse(browse_id: String, params: Option<String>) -> Result<Page, String> {
    let json = browse(&browse_id, params.as_deref().filter(|p| !p.is_empty())).await?;
    let mut page = pm::music_browse(&json);
    filter_page(&mut page, FilterCtx::Music);
    Ok(page)
}

#[tauri::command]
pub async fn tube_music_search(query: String, filter: Option<String>) -> Result<Page, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Ok(Page::default());
    }
    let params = filter.as_deref().and_then(innertube::music_search_params);
    let json = http::music("search", |c, v, hl, gl| {
        innertube::search_body(c, v, hl, gl, &query, params)
    })
    .await?;
    let mut page = pm::music_search(&json);
    filter_page(&mut page, FilterCtx::Music);
    Ok(page)
}

#[tauri::command]
pub async fn tube_music_album(browse_id: String) -> Result<AlbumPage, String> {
    Ok(pm::music_album(
        &browse(&browse_id, None).await?,
        &browse_id,
    ))
}

#[tauri::command]
pub async fn tube_music_artist(browse_id: String) -> Result<ArtistPage, String> {
    let mut page = pm::music_artist(&browse(&browse_id, None).await?, &browse_id);
    for shelf in page.shelves.iter_mut() {
        filter_items(&mut shelf.items, FilterCtx::Music);
    }
    page.shelves.retain(|s| !s.items.is_empty());
    Ok(page)
}

fn vl(id: &str) -> String {
    if id.starts_with("VL") {
        id.to_string()
    } else {
        format!("VL{id}")
    }
}

#[tauri::command]
pub async fn tube_music_playlist(id: String) -> Result<PlaylistPage, String> {
    let bare = id.trim_start_matches("VL").to_string();
    let mut page = pm::music_playlist(&browse(&vl(&id), None).await?, &bare);
    filter_items(&mut page.items, FilterCtx::Music);
    Ok(page)
}

fn queue_fields(body: &mut Value) {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("isAudioOnly".into(), json!(true));
        obj.insert("enablePersistentPlaylistPanel".into(), json!(true));
        obj.insert("tunerSettingValue".into(), json!("AUTOMIX_SETTING_NORMAL"));
    }
}

async fn next(
    video_id: Option<&str>,
    playlist_id: Option<&str>,
    params: Option<&str>,
    continuation: Option<&str>,
) -> Result<Value, String> {
    http::music("next", |c, v, hl, gl| {
        let mut body = match continuation {
            Some(token) => innertube::continuation_body(c, v, hl, gl, token),
            None => innertube::next_body(c, v, hl, gl, video_id, playlist_id, None, params),
        };
        queue_fields(&mut body);
        body
    })
    .await
}

#[tauri::command]
pub async fn tube_music_next(
    video_id: String,
    playlist_id: Option<String>,
    radio: bool,
    continuation: Option<String>,
) -> Result<QueuePage, String> {
    let radio_id = format!("RDAMVM{video_id}");
    let (pid, params) = if radio {
        (Some(radio_id.as_str()), Some("wAEB"))
    } else {
        (playlist_id.as_deref().filter(|p| !p.is_empty()), None)
    };
    let json = next(
        Some(&video_id),
        pid,
        params,
        continuation.as_deref().filter(|c| !c.is_empty()),
    )
    .await?;
    let mut queue = pm::music_queue(&json);
    if queue.playlist_id.is_none() {
        queue.playlist_id = pid.map(str::to_string);
    }
    Ok(queue)
}

async fn youtube_lyrics(video_id: &str) -> Result<Option<Lyrics>, String> {
    let queue = pm::music_queue(&next(Some(video_id), None, None, None).await?);
    let Some(id) = queue.lyrics_browse_id else {
        return Ok(None);
    };
    Ok(pm::music_lyrics(&browse(&id, None).await?))
}

async fn lrclib_lyrics(
    title: &str,
    artist: &str,
    duration_s: u32,
) -> Result<Option<Lyrics>, String> {
    if title.trim().is_empty() {
        return Ok(None);
    }
    let mut url = format!(
        "https://lrclib.net/api/search?track_name={}",
        urlencoding::encode(title.trim())
    );
    if !artist.trim().is_empty() {
        url.push_str(&format!(
            "&artist_name={}",
            urlencoding::encode(artist.trim())
        ));
    }
    Ok(lrc::lrclib_pick(&http::get_json(&url).await?, duration_s))
}

#[tauri::command]
pub async fn tube_music_lyrics(
    video_id: String,
    title: String,
    artist: String,
    duration_s: f64,
) -> Result<Option<Lyrics>, String> {
    let duration = duration_s.max(0.0).round() as u32;
    match http::settings().lyrics_provider.as_str() {
        "off" => Ok(None),
        "youtube" => youtube_lyrics(&video_id).await,
        "lrclib" => lrclib_lyrics(&title, &artist, duration).await,
        _ => {
            let yt = youtube_lyrics(&video_id).await.ok().flatten();
            if yt.as_ref().is_some_and(|l| l.synced) {
                return Ok(yt);
            }
            let lrclib = lrclib_lyrics(&title, &artist, duration)
                .await
                .ok()
                .flatten();
            match (yt, lrclib) {
                (_, Some(l)) if l.synced => Ok(Some(l)),
                (Some(y), _) => Ok(Some(y)),
                (None, l) => Ok(l),
            }
        }
    }
}

#[tauri::command]
pub async fn tube_music_related(browse_id: String) -> Result<Page, String> {
    let mut page = pm::music_related(&browse(&browse_id, None).await?);
    filter_page(&mut page, FilterCtx::Music);
    Ok(page)
}

fn library_browse_id(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "playlists" => "FEmusic_liked_playlists",
        "songs" => "FEmusic_liked_videos",
        "albums" => "FEmusic_liked_albums",
        "artists" => "FEmusic_library_corpus_track_artists",
        "subscriptions" => "FEmusic_library_corpus_artists",
        "history" => "FEmusic_history",
        "liked" => "VLLM",
        _ => return None,
    })
}

#[tauri::command]
pub async fn tube_music_library(kind: String) -> Result<Page, String> {
    if !account::is_logged_in() {
        return Ok(Page::default());
    }
    let id = library_browse_id(&kind).ok_or("unknown library section")?;
    let json = browse(id, None).await?;
    if id == "VLLM" {
        let pl = pm::music_playlist(&json, "LM");
        return Ok(Page {
            shelves: vec![Shelf {
                id: "liked".into(),
                title: Some(pl.title),
                kind: ShelfKind::List,
                items: pl.items,
                ..Default::default()
            }],
            continuation: pl.continuation,
            ..Default::default()
        });
    }
    Ok(pm::music_library(&json))
}
