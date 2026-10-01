use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::AppHandle;

use omniget_core::core::video::{stremio, subs};

use super::sources::{self, SourceRow};
use super::{server, PlayMode, PlaybackInfo, SourceRef, SubtitleTrack};

const CALL_LIMIT: Duration = Duration::from_secs(12);
const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_SUB_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize, Clone, Debug)]
pub struct AddonExtra {
    pub name: String,
    pub required: bool,
    pub options: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AddonCatalog {
    pub addon: i64,
    #[serde(rename = "type")]
    pub typ: String,
    pub id: String,
    pub name: String,
    pub extra: Vec<AddonExtra>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AddonInfo {
    pub id: i64,
    pub manifest_url: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub logo: Option<String>,
    pub types: Vec<String>,
    pub catalogs: Vec<AddonCatalog>,
    pub resources: Vec<String>,
    pub enabled: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct AddonVideo {
    pub id: String,
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub released: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AddonMeta {
    pub addon: i64,
    pub id: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub name: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
    pub description: Option<String>,
    pub release_info: Option<String>,
    pub genres: Vec<String>,
    pub videos: Vec<AddonVideo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AddonStream {
    pub addon: i64,
    pub addon_name: String,
    pub index: i64,
    pub name: String,
    pub title: String,
    pub kind: String,
    pub url: String,
    pub playable: bool,
}

struct Addon {
    row: SourceRow,
    manifest: stremio::Manifest,
}

impl Addon {
    fn of(row: SourceRow) -> Option<Self> {
        let manifest = serde_json::from_value(row.config.get("manifest")?.clone()).ok()?;
        Some(Self { row, manifest })
    }

    fn base(&self) -> String {
        stremio::addon_base(&self.row.url)
    }

    fn info(&self) -> AddonInfo {
        let m = &self.manifest;
        AddonInfo {
            id: self.row.id,
            manifest_url: self.row.url.clone(),
            name: if m.name.is_empty() {
                self.row.name.clone()
            } else {
                m.name.clone()
            },
            version: m.version.clone(),
            description: m.description.clone(),
            logo: m.logo.clone().filter(|l| !l.is_empty()),
            types: m.types.clone(),
            catalogs: m
                .catalogs
                .iter()
                .map(|c| AddonCatalog {
                    addon: self.row.id,
                    typ: c.type_.clone(),
                    id: c.id.clone(),
                    name: c.name.clone().unwrap_or_else(|| c.id.clone()),
                    extra: c
                        .extras()
                        .into_iter()
                        .map(|e| AddonExtra {
                            name: e.name,
                            required: e.is_required,
                            options: e.options,
                        })
                        .collect(),
                })
                .collect(),
            resources: m.resource_names(),
            enabled: self.row.enabled,
        }
    }
}

fn load(id: i64) -> Result<Addon, String> {
    Addon::of(sources::get_kind(id, "addon")?)
        .ok_or_else(|| "Add-on data is damaged, install it again".into())
}

fn installed() -> Result<Vec<Addon>, String> {
    Ok(sources::list(Some("addon"))?
        .into_iter()
        .filter_map(Addon::of)
        .collect())
}

async fn fetch<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let resp = sources::http()
        .get(url)
        .header("Accept", "application/json")
        .timeout(CALL_LIMIT)
        .send()
        .await
        .map_err(sources::net_err)?;
    if !resp.status().is_success() {
        return Err(sources::status_err(resp.status()));
    }
    if resp.content_length().is_some_and(|n| n as usize > MAX_BODY) {
        return Err("The add-on answered with too much data".into());
    }
    let bytes = resp.bytes().await.map_err(sources::net_err)?;
    if bytes.len() > MAX_BODY {
        return Err("The add-on answered with too much data".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "The add-on answered in an unknown format".to_string())
}

#[tauri::command]
pub async fn video_addon_install(manifest_url: String) -> Result<AddonInfo, String> {
    let url = stremio::normalize_manifest_url(&manifest_url);
    let parsed = url::Url::parse(&url).map_err(|_| "Invalid add-on link".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Invalid add-on link".into());
    }
    if parsed.path().contains("/stremio/v1") {
        return Err("This old add-on format is not supported".into());
    }
    if !parsed.path().ends_with("/manifest.json") {
        return Err("The link must end in /manifest.json".into());
    }
    let manifest: stremio::Manifest = fetch(&url).await?;
    if manifest.id.trim().is_empty()
        || manifest.name.trim().is_empty()
        || manifest.resources.is_empty()
    {
        return Err("This is not a valid add-on manifest".into());
    }
    if stremio::manifest_is_p2p(&manifest) {
        return Err("Peer-to-peer add-ons are not supported".into());
    }
    if manifest.behavior_hints.configuration_required {
        return Err(
            "This add-on needs setup first: configure it on its page and paste the final link"
                .into(),
        );
    }
    let config = json!({
        "key": url,
        "addon_id": manifest.id,
        "manifest": manifest,
    });
    let id = sources::upsert("addon", &manifest.name, &url, &config)?;
    Ok(load(id)?.info())
}

#[tauri::command]
pub async fn video_addons() -> Result<Vec<AddonInfo>, String> {
    Ok(installed()?.iter().map(Addon::info).collect())
}

#[tauri::command]
pub async fn video_addon_remove(id: i64) -> Result<(), String> {
    sources::get_kind(id, "addon")?;
    sources::video_source_remove(id).await
}

fn meta_of(addon: i64, m: stremio::MetaPreview) -> AddonMeta {
    AddonMeta {
        addon,
        id: m.id,
        typ: m.type_,
        name: m.name,
        poster: m.poster.filter(|p| !p.is_empty()),
        background: m.background.filter(|p| !p.is_empty()),
        logo: m.logo.filter(|p| !p.is_empty()),
        description: m.description.filter(|p| !p.is_empty()),
        release_info: m.release_info,
        genres: m.genres,
        videos: Vec::new(),
    }
}

fn detail_of(addon: i64, m: stremio::MetaDetail) -> AddonMeta {
    let mut videos: Vec<AddonVideo> = m
        .videos
        .iter()
        .map(|v| AddonVideo {
            id: v.id.clone(),
            title: v.display_title().to_string(),
            season: v.season,
            episode: v.episode_no(),
            released: v.released.clone(),
            thumbnail: v.thumbnail.clone().filter(|t| !t.is_empty()),
        })
        .collect();
    videos.sort_by_key(|v| (v.season.unwrap_or(0) == 0, v.season, v.episode));
    AddonMeta {
        addon,
        id: m.id,
        typ: m.type_,
        name: m.name,
        poster: m.poster.filter(|p| !p.is_empty()),
        background: m.background.filter(|p| !p.is_empty()),
        logo: m.logo.filter(|p| !p.is_empty()),
        description: m.description.filter(|p| !p.is_empty()),
        release_info: m.release_info.or(m.released),
        genres: m.genres,
        videos,
    }
}

#[tauri::command]
pub async fn video_addon_catalog(
    addon: i64,
    r#type: String,
    id: String,
    extra: Option<Vec<(String, String)>>,
    skip: Option<i64>,
) -> Result<Vec<AddonMeta>, String> {
    let a = load(addon)?;
    if !a.row.enabled {
        return Err("This add-on is turned off".into());
    }
    let mut extra: Vec<(String, String)> = extra
        .unwrap_or_default()
        .into_iter()
        .filter(|(k, v)| !k.is_empty() && !v.is_empty() && k != "skip")
        .collect();
    if let Some(s) = skip.filter(|s| *s > 0) {
        extra.push(("skip".into(), s.to_string()));
    }
    let url = stremio::catalog_url(&a.base(), &r#type, &id, &extra);
    let resp: stremio::CatalogResponse = sources::tracked(addon, CALL_LIMIT, fetch(&url)).await?;
    Ok(resp
        .metas
        .into_iter()
        .filter(|m| !m.id.is_empty())
        .map(|m| meta_of(addon, m))
        .collect())
}

#[tauri::command]
pub async fn video_addon_meta(r#type: String, id: String) -> Result<AddonMeta, String> {
    let candidates: Vec<Addon> = installed()?
        .into_iter()
        .filter(|a| !sources::skip_for_now(&a.row))
        .filter(|a| stremio::supports(&a.manifest, "meta", &r#type, &id))
        .collect();
    if candidates.is_empty() {
        return Err("No add-on has details for this".into());
    }
    let calls = candidates.iter().map(|a| {
        let url = stremio::meta_url(&a.base(), &r#type, &id);
        let aid = a.row.id;
        async move {
            let r: Result<stremio::MetaResponse, String> =
                sources::tracked(aid, CALL_LIMIT, fetch(&url)).await;
            (aid, r)
        }
    });
    let mut last_err = None;
    for (aid, r) in futures::future::join_all(calls).await {
        match r {
            Ok(resp) => {
                if let Some(meta) = resp.meta.filter(|m| !m.id.is_empty() || !m.name.is_empty()) {
                    let mut out = detail_of(aid, meta);
                    if out.id.is_empty() {
                        out.id = id.clone();
                    }
                    if out.typ.is_empty() {
                        out.typ = r#type.clone();
                    }
                    return Ok(out);
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| "No add-on has details for this".into()))
}

type StreamCache = Mutex<HashMap<(i64, String, String), Vec<stremio::Stream>>>;

fn stream_cache() -> &'static StreamCache {
    static C: OnceLock<StreamCache> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn to_public(a: &Addon, index: usize, s: &stremio::Stream) -> Option<AddonStream> {
    let (kind, url, playable) = match stremio::classify_from(&a.manifest, s) {
        stremio::StreamKind::Url(u) => ("url", u, true),
        stremio::StreamKind::YouTube(id) => ("youtube", id, true),
        stremio::StreamKind::External(u) => ("external", u, false),
        stremio::StreamKind::Refused(_) => return None,
    };
    let name = if s.name.trim().is_empty() {
        a.manifest.name.clone()
    } else {
        s.name.trim().to_string()
    };
    Some(AddonStream {
        addon: a.row.id,
        addon_name: a.manifest.name.clone(),
        index: index as i64,
        name,
        title: s.label().trim().to_string(),
        kind: kind.into(),
        url,
        playable,
    })
}

async fn streams_of(a: &Addon, typ: &str, id: &str) -> Result<Vec<stremio::Stream>, String> {
    let url = stremio::stream_url(&a.base(), typ, id);
    let resp: stremio::StreamsResponse =
        sources::tracked(a.row.id, CALL_LIMIT, fetch(&url)).await?;
    if let Ok(mut c) = stream_cache().lock() {
        if c.len() > 200 {
            c.clear();
        }
        c.insert(
            (a.row.id, typ.to_string(), id.to_string()),
            resp.streams.clone(),
        );
    }
    Ok(resp.streams)
}

#[tauri::command]
pub async fn video_addon_streams(r#type: String, id: String) -> Result<Vec<AddonStream>, String> {
    let candidates: Vec<Addon> = installed()?
        .into_iter()
        .filter(|a| !sources::skip_for_now(&a.row))
        .filter(|a| stremio::supports(&a.manifest, "stream", &r#type, &id))
        .collect();
    let calls = candidates.iter().map(|a| {
        let (typ, id) = (r#type.clone(), id.clone());
        async move { (a, streams_of(a, &typ, &id).await) }
    });
    let mut out = Vec::new();
    for (a, r) in futures::future::join_all(calls).await {
        match r {
            Ok(list) => out.extend(
                list.iter()
                    .enumerate()
                    .filter_map(|(n, s)| to_public(a, n, s)),
            ),
            Err(e) => tracing::info!("[video] add-on {} streams: {e}", a.row.id),
        }
    }
    Ok(out)
}

async fn subtitle_track(n: usize, s: &stremio::Subtitle) -> Option<SubtitleTrack> {
    let resp = sources::http()
        .get(&s.url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > MAX_SUB_BYTES {
        return None;
    }
    let cues = subs::parse_auto(&bytes);
    if cues.is_empty() {
        return None;
    }
    let vtt = subs::to_vtt(&cues, 0);
    Some(SubtitleTrack {
        id: format!("addon-{n}"),
        label: s
            .label
            .clone()
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| s.lang.clone()),
        lang: s.lang.clone(),
        url: format!(
            "data:text/vtt;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(vtt.as_bytes())
        ),
        default: false,
        forced: false,
        source: "remote".into(),
    })
}

async fn raw_stream(a: &Addon, typ: &str, id: &str, index: i64) -> Result<stremio::Stream, String> {
    let cached = stream_cache()
        .lock()
        .ok()
        .and_then(|c| c.get(&(a.row.id, typ.to_string(), id.to_string())).cloned());
    let list = match cached {
        Some(l) => l,
        None => streams_of(a, typ, id).await?,
    };
    list.into_iter()
        .nth(index.max(0) as usize)
        .ok_or_else(|| "That stream is gone, look for streams again".into())
}

async fn play_stream(
    app: &AppHandle,
    stream: &AddonStream,
    typ: &str,
    id: &str,
    title: &str,
) -> Result<PlaybackInfo, String> {
    let key = format!("addon:{typ}:{id}");
    let source_ref = SourceRef::Addon {
        addon: stream.addon,
        typ: typ.to_string(),
        id: id.to_string(),
        stream_index: Some(stream.index),
    };
    let a = load(stream.addon)?;
    let raw = raw_stream(&a, typ, id, stream.index).await?;
    let kind = stremio::classify_from(&a.manifest, &raw);
    let mut info = match kind {
        stremio::StreamKind::External(_) => return Err("This one opens in the browser".into()),
        stremio::StreamKind::Refused(_) => {
            return Err("This kind of stream is not supported".into())
        }
        stremio::StreamKind::YouTube(yt) => {
            let mut info =
                super::link::resolve(app, &format!("https://www.youtube.com/watch?v={yt}")).await?;
            info.key = key.clone();
            info.source = source_ref.clone();
            info
        }
        stremio::StreamKind::Url(url) => {
            let headers: Vec<(String, String)> = raw
                .behavior_hints
                .proxy_headers
                .as_ref()
                .map(|h| {
                    h.request
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            server::ensure(app).await?;
            let lower = url.to_ascii_lowercase();
            let is_hls = lower.split('?').next().unwrap_or("").ends_with(".m3u8");
            let mut info = PlaybackInfo::remote(
                key.clone(),
                source_ref.clone(),
                server::register_remote(&url, headers),
            );
            info.mode = if is_hls {
                PlayMode::Hls
            } else {
                PlayMode::Remote
            };
            info.reason = if raw.behavior_hints.not_web_ready {
                "Add-on stream through the local proxy (may not play in the app)".into()
            } else {
                "Add-on stream through the local proxy".into()
            };
            let (start, _) = super::resume_for(&key).unwrap_or((0, None));
            info.start_ms = start;
            info
        }
    };
    if !title.trim().is_empty() {
        info.title = title.trim().to_string();
    }
    if info.subtitle.is_empty() {
        info.subtitle = stream.addon_name.clone();
    }
    let subs = futures::future::join_all(
        raw.subtitles
            .iter()
            .take(12)
            .enumerate()
            .map(|(n, s)| subtitle_track(n, s)),
    )
    .await;
    info.subtitles.extend(subs.into_iter().flatten());
    info.prefs = super::merged_prefs(&[format!("media:{key}")]);
    let _ = super::remember_item(&key, &info.title, &info.subtitle, None, &source_ref);
    Ok(info)
}

#[tauri::command]
pub async fn video_addon_play(
    app: AppHandle,
    stream: AddonStream,
    r#type: String,
    id: String,
    title: String,
) -> Result<PlaybackInfo, String> {
    play_stream(&app, &stream, &r#type, &id, &title).await
}

pub async fn replay(
    app: &AppHandle,
    addon: i64,
    typ: &str,
    id: &str,
    stream_index: Option<i64>,
) -> Result<PlaybackInfo, String> {
    let a = load(addon)?;
    let list = streams_of(&a, typ, id).await?;
    let public: Vec<AddonStream> = list
        .iter()
        .enumerate()
        .filter_map(|(n, s)| to_public(&a, n, s))
        .filter(|s| s.playable)
        .collect();
    let pick = stream_index
        .and_then(|i| public.iter().find(|s| s.index == i))
        .or_else(|| public.first())
        .cloned()
        .ok_or("This add-on has no playable stream for it now")?;
    let title = super::db::with(|c| {
        use rusqlite::OptionalExtension;
        c.query_row(
            "SELECT title FROM items_meta WHERE key = ?1",
            [format!("addon:{typ}:{id}")],
            |r| r.get::<_, String>(0),
        )
        .optional()
    })
    .ok()
    .flatten()
    .unwrap_or_default();
    play_stream(app, &pick, typ, id, &title).await
}
