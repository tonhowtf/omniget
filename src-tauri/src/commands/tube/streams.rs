use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use omniget_core::core::tube::innertube;
use omniget_core::core::tube::player::{self, PickPrefs, Playability};
use omniget_core::core::tube::{Caption, Format, FormatKind, Storyboard, StreamSet, Thumb};
use serde_json::Value;
use tauri::AppHandle;

use super::server::{self, CaptionSrc, Extras};
use super::{account, http};

const PER_CLIENT: Duration = Duration::from_secs(6);
const BLOCK_FOR: Duration = Duration::from_secs(30 * 60);
const CACHE_MARGIN_MS: i64 = 5 * 60 * 1000;
const DEFAULT_TTL_MS: i64 = 5 * 3600 * 1000;

#[derive(Clone, Debug, Default)]
struct Resolved {
    video_id: String,
    duration_s: u32,
    client: String,
    ua: String,
    formats: Vec<Format>,
    loudness_db: Option<f32>,
    expires_ms: i64,
    is_live: bool,
    hls: Option<String>,
    captions: Vec<(String, String, bool, CaptionSrc)>,
    storyboard: Option<(String, Storyboard)>,
    title: Option<String>,
    author: Option<String>,
    thumbs: Vec<Thumb>,
}

pub(crate) struct Fresh {
    pub formats: Vec<Format>,
    pub ua: String,
    pub hls: Option<String>,
}

type Key = (String, bool);
type Cached = (StreamSet, String, Extras);

fn cache() -> &'static Mutex<HashMap<Key, Cached>> {
    static C: OnceLock<Mutex<HashMap<Key, Cached>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn blocklist() -> &'static Mutex<HashMap<(String, String), Instant>> {
    static B: OnceLock<Mutex<HashMap<(String, String), Instant>>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(HashMap::new()))
}

fn inflight(key: &Key) -> Arc<tokio::sync::Mutex<()>> {
    static F: OnceLock<Mutex<HashMap<Key, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let map = F.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut g) = map.lock() else {
        return Arc::new(tokio::sync::Mutex::new(()));
    };
    if g.len() > 256 {
        g.retain(|_, m| Arc::strong_count(m) > 1);
    }
    g.entry(key.clone()).or_default().clone()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn blocked(video_id: &str, client: &str) -> bool {
    let Ok(mut g) = blocklist().lock() else {
        return false;
    };
    g.retain(|_, at| at.elapsed() < BLOCK_FOR);
    g.contains_key(&(video_id.to_string(), client.to_string()))
}

fn block(video_id: &str, client: &str) {
    if let Ok(mut g) = blocklist().lock() {
        g.insert((video_id.to_string(), client.to_string()), Instant::now());
    }
}

pub fn forget_account_state() {
    if let Ok(mut g) = cache().lock() {
        g.clear();
    }
    if let Ok(mut g) = tracks().lock() {
        g.clear();
    }
}

fn pick_prefs() -> PickPrefs {
    let s = http::settings();
    PickPrefs {
        max_height: s.default_quality.parse::<u32>().ok(),
        codec: s.prefer_codec.clone(),
        // WHY: WKWebView and WebKitGTK are unreliable with Opus/VP9 in MSE.
        // WKWebView and WebKitGTK are unreliable with Opus/VP9 in MSE
        webkit: cfg!(any(target_os = "macos", target_os = "linux")),
        audio_quality: s.audio_quality.clone(),
        lang: None,
    }
}

fn url_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })
}

fn expires_from(formats: &[Format]) -> Option<i64> {
    formats
        .iter()
        .find_map(|f| url_param(&f.url, "expire")?.parse::<i64>().ok())
        .map(|s| s * 1000)
}

async fn validate(f: &Format, ua: &str) -> bool {
    let len = f
        .content_length
        .or_else(|| url_param(&f.url, "clen").and_then(|v| v.parse().ok()));
    let range = match len.filter(|n| *n > 256) {
        Some(n) => format!("bytes={}-{}", n - 256, n - 1),
        None => "bytes=0-255".to_string(),
    };
    let req = http::media_client()
        .get(&f.url)
        .header(reqwest::header::USER_AGENT, ua)
        .header(reqwest::header::RANGE, range)
        .timeout(Duration::from_secs(8));
    match req.send().await {
        Ok(r) => r.status().is_success(),
        Err(e) => {
            tracing::debug!("[tube] validate {} failed: {e}", http::redact(&f.url));
            false
        }
    }
}

enum Miss {
    Account(String),
    Final(String),
    Failed(String),
}

async fn ladder(video_id: &str, music: bool) -> Result<Resolved, Miss> {
    let names: &[&str] = if music {
        innertube::MUSIC_PLAYER_LADDER
    } else {
        innertube::PLAYER_LADDER
    };
    let (hl, gl) = http::locale();
    let prefs = pick_prefs();
    let mut account_needed: Option<String> = None;
    let mut last = String::from("no client could play this video");
    for name in names {
        if blocked(video_id, name) {
            continue;
        }
        let Some(c) = innertube::client(name) else {
            continue;
        };
        let visitor = http::visitor_data().await.ok();
        let body = innertube::player_body(&c, visitor.as_deref(), &hl, &gl, video_id, None);
        let started = Instant::now();
        let json = match tokio::time::timeout(
            PER_CLIENT,
            http::innertube(&c, "player", body, false),
        )
        .await
        {
            Ok(Ok(j)) => j,
            Ok(Err(e)) => {
                last = e;
                continue;
            }
            Err(_) => {
                last = format!("{name} timed out");
                continue;
            }
        };
        let p = player::parse_player(&json);
        match &p.playability {
            Playability::Ok => {}
            Playability::LiveNotStarted(m) => return Err(Miss::Final(m.clone())),
            Playability::LoginRequired(m) | Playability::AgeRestricted(m) => {
                account_needed = Some(m.clone());
                continue;
            }
            Playability::Unplayable(m) | Playability::Error(m) => {
                last = m.clone();
                continue;
            }
        }
        let formats: Vec<Format> = p
            .formats
            .iter()
            .filter(|f| !f.url.is_empty() && player::url_is_clean(&f.url))
            .cloned()
            .collect();
        if p.is_live {
            if p.hls_url.is_none() {
                last = "live stream without HLS".into();
                continue;
            }
        } else {
            let audio = player::pick_audio(&formats, &prefs);
            let video = if music {
                None
            } else {
                player::pick_video(&formats, &prefs)
            };
            let Some(audio) = audio else {
                last = format!("{name}: no playable audio");
                continue;
            };
            if !music && video.is_none() {
                last = format!("{name}: no playable video");
                continue;
            }
            let (a_ok, v_ok) = tokio::join!(validate(audio, &c.user_agent), async {
                match video {
                    Some(v) => validate(v, &c.user_agent).await,
                    None => true,
                }
            });
            if !(a_ok && v_ok) {
                tracing::info!("[tube] {name} URLs rejected for {video_id}, next client");
                block(video_id, name);
                last = format!("{name}: stream rejected");
                continue;
            }
        }
        tracing::info!(
            "[tube] {video_id} resolved by {name} in {} ms",
            started.elapsed().as_millis()
        );
        let expires_ms = p
            .expires_in_s
            .map(|s| now_ms() + (s as i64) * 1000)
            .or_else(|| expires_from(&formats))
            .unwrap_or_else(|| now_ms() + DEFAULT_TTL_MS);
        let storyboard = p
            .storyboard_spec
            .as_deref()
            .and_then(|spec| player::parse_storyboard(spec, p.duration_s));
        return Ok(Resolved {
            video_id: video_id.to_string(),
            duration_s: p.duration_s,
            client: c.name.clone(),
            ua: c.user_agent.clone(),
            formats,
            loudness_db: p.loudness_db,
            expires_ms,
            is_live: p.is_live,
            hls: p.hls_url.clone(),
            captions: p
                .captions
                .iter()
                .map(|rc| {
                    (
                        rc.lang.clone(),
                        rc.name.clone(),
                        rc.auto,
                        CaptionSrc {
                            url: rc.base_url.clone(),
                            raw: true,
                        },
                    )
                })
                .collect(),
            storyboard,
            title: p.title.clone(),
            author: p.author.clone(),
            thumbs: p.thumbs.clone(),
        });
    }
    Err(match account_needed {
        Some(m) => Miss::Account(m),
        None => Miss::Failed(last),
    })
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

fn mime_for(kind: FormatKind, ext: &str) -> String {
    let top = if kind == FormatKind::Audio {
        "audio"
    } else {
        "video"
    };
    let sub = match (kind, ext) {
        (FormatKind::Audio, "m4a") => "mp4",
        (_, "") => "mp4",
        (_, e) => e,
    };
    format!("{top}/{sub}")
}

fn itag_rank(f: &Format) -> (bool, bool) {
    (f.audio_default, !f.drc)
}

fn from_ytdlp(json: &Value, video_id: &str) -> Resolved {
    let mut by_itag: HashMap<u32, Format> = HashMap::new();
    let mut ua: Option<String> = None;
    let mut hls: Option<String> = None;
    for f in json
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let fid = s(f, "format_id").unwrap_or_default();
        let Some(itag) = fid.split('-').next().and_then(|x| x.parse::<u32>().ok()) else {
            continue;
        };
        let protocol = s(f, "protocol").unwrap_or_default();
        if protocol.contains("m3u8") {
            if hls.is_none() {
                hls = s(f, "manifest_url");
            }
            continue;
        }
        if !protocol.starts_with("http") {
            continue;
        }
        let Some(url) = s(f, "url") else {
            continue;
        };
        let vcodec = s(f, "vcodec").unwrap_or_else(|| "none".into());
        let acodec = s(f, "acodec").unwrap_or_else(|| "none".into());
        let kind = match (vcodec != "none", acodec != "none") {
            (true, true) => FormatKind::Muxed,
            (true, false) => FormatKind::Video,
            (false, true) => FormatKind::Audio,
            (false, false) => continue,
        };
        let ext = s(f, "ext").unwrap_or_default();
        let note = s(f, "format_note").unwrap_or_default();
        let height = f.get("height").and_then(Value::as_u64).map(|h| h as u32);
        let lang_pref = f
            .get("language_preference")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        let codecs = match kind {
            FormatKind::Video => vcodec.clone(),
            FormatKind::Audio => acodec.clone(),
            FormatKind::Muxed => format!("{vcodec}, {acodec}"),
        };
        if ua.is_none() {
            ua = f
                .pointer("/http_headers/User-Agent")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        let format = Format {
            itag,
            kind,
            mime: mime_for(kind, &ext),
            codecs,
            width: f.get("width").and_then(Value::as_u64).map(|w| w as u32),
            height,
            fps: f
                .get("fps")
                .and_then(Value::as_f64)
                .map(|x| x.round() as u32),
            bitrate: f
                .get("tbr")
                .and_then(Value::as_f64)
                .map(|k| (k * 1000.0) as u64)
                .unwrap_or(0),
            content_length: f
                .get("filesize")
                .and_then(Value::as_u64)
                .or_else(|| url_param(&url, "clen").and_then(|v| v.parse().ok())),
            quality_label: match kind {
                FormatKind::Audio => None,
                _ => height.map(|h| format!("{h}p")),
            },
            audio_lang: s(f, "language"),
            audio_name: (kind == FormatKind::Audio && !note.is_empty()).then(|| note.clone()),
            audio_default: kind != FormatKind::Audio
                || s(f, "language").is_none()
                || lang_pref >= 10
                || note.to_ascii_lowercase().contains("original"),
            auto_dubbed: note.to_ascii_lowercase().contains("dubbed"),
            hdr: s(f, "dynamic_range").is_some_and(|d| d != "SDR"),
            drc: fid.contains("drc"),
            init_range: None,
            index_range: None,
            url,
        };
        match by_itag.get(&itag) {
            Some(old) if itag_rank(old) >= itag_rank(&format) => {}
            _ => {
                by_itag.insert(itag, format);
            }
        }
    }
    let mut formats: Vec<Format> = by_itag.into_values().collect();
    formats.sort_by(|a, b| {
        (a.kind as u8, b.height, b.bitrate).cmp(&(b.kind as u8, a.height, a.bitrate))
    });

    let mut captions = Vec::new();
    let mut add_captions = |key: &str, auto: bool| {
        let Some(map) = json.get(key).and_then(Value::as_object) else {
            return;
        };
        for (lang, list) in map {
            let Some(entry) = list.as_array().and_then(|l| {
                l.iter()
                    .find(|e| e.get("ext").and_then(Value::as_str) == Some("vtt"))
            }) else {
                continue;
            };
            let Some(url) = s(entry, "url") else {
                continue;
            };
            if auto && url.contains("tlang=") {
                continue;
            }
            let name = s(entry, "name").unwrap_or_else(|| lang.clone());
            let lang = lang.trim_end_matches("-orig").to_string();
            captions.push((lang, name, auto, CaptionSrc { url, raw: false }));
        }
    };
    add_captions("subtitles", false);
    add_captions("automatic_captions", true);

    let is_live = json
        .get("is_live")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || s(json, "live_status").as_deref() == Some("is_live");
    let thumbs = json
        .get("thumbnails")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(Thumb {
                url: s(t, "url")?,
                width: t.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
                height: t.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
            })
        })
        .filter(|t| t.width > 0)
        .collect();
    let expires_ms = expires_from(&formats).unwrap_or_else(|| now_ms() + DEFAULT_TTL_MS);
    Resolved {
        video_id: video_id.to_string(),
        duration_s: json
            .get("duration")
            .and_then(Value::as_f64)
            .map(|d| d.round() as u32)
            .unwrap_or(0),
        client: "yt-dlp".into(),
        ua: ua.unwrap_or_else(|| http::BROWSER_UA.to_string()),
        formats,
        loudness_db: None,
        expires_ms,
        is_live,
        hls: s(json, "manifest_url").or(hls),
        captions,
        storyboard: None,
        title: s(json, "title"),
        author: s(json, "channel").or_else(|| s(json, "uploader")),
        thumbs,
    }
}

async fn via_ytdlp(video_id: &str, music: bool) -> Result<Resolved, String> {
    let bin = omniget_core::core::ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {e}"))?;
    let host = if music {
        "music.youtube.com"
    } else {
        "www.youtube.com"
    };
    let url = format!("https://{host}/watch?v={video_id}");
    let started = Instant::now();
    let json = omniget_core::core::ytdlp::get_video_info(&bin, &url, &[])
        .await
        .map_err(|e| e.to_string())?;
    let r = from_ytdlp(&json, video_id);
    tracing::info!(
        "[tube] {video_id} resolved by yt-dlp in {} ms ({} formats)",
        started.elapsed().as_millis(),
        r.formats.len()
    );
    if r.formats.is_empty() && r.hls.is_none() {
        return Err("yt-dlp found no playable format".into());
    }
    Ok(r)
}

fn wants_premium(music: bool) -> bool {
    music
        && http::settings().audio_quality == "premium"
        && account::is_logged_in()
        && account::cached_premium() != Some(false)
}

async fn resolve(video_id: &str, music: bool, force_ytdlp: bool) -> Result<Resolved, String> {
    if force_ytdlp {
        return via_ytdlp(video_id, music).await;
    }
    if wants_premium(music) {
        match via_ytdlp(video_id, music).await {
            Ok(r) => return Ok(r),
            Err(e) => tracing::info!("[tube] premium path failed for {video_id}: {e}"),
        }
    }
    match ladder(video_id, music).await {
        Ok(r) => Ok(r),
        Err(Miss::Final(m)) => Err(m),
        Err(Miss::Account(m)) | Err(Miss::Failed(m)) => {
            tracing::info!("[tube] ladder failed for {video_id} ({m}), trying yt-dlp");
            via_ytdlp(video_id, music)
                .await
                .map_err(|e| if e.is_empty() { m } else { e })
        }
    }
}

fn open(r: Resolved, music: bool, info: &server::ServerInfo) -> (StreamSet, String, Extras) {
    let session = server::new_session_id();
    let base = server::session_base(info, &session);
    let token = info.token.clone();
    let captions: Vec<Caption> = r
        .captions
        .iter()
        .enumerate()
        .map(|(i, (lang, name, auto, _))| Caption {
            lang: lang.clone(),
            name: name.clone(),
            auto: *auto,
            url: format!("{base}/cc/{i}.vtt?k={token}"),
        })
        .collect();
    let (sb_template, storyboard) = match r.storyboard {
        Some((tpl, mut sb)) => {
            sb.url = format!("{base}/sb/$M.jpg?k={token}");
            (Some(tpl), Some(sb))
        }
        None => (None, None),
    };
    let set = StreamSet {
        session,
        video_id: r.video_id,
        duration_s: r.duration_s,
        client: r.client,
        formats: r.formats,
        loudness_db: r.loudness_db,
        expires_ms: r.expires_ms,
        is_live: r.is_live,
        hls: r.hls.as_ref().map(|_| format!("{base}/hls.m3u8?k={token}")),
        captions,
        storyboard,
        base,
        token,
        title: r.title,
        author: r.author,
        thumbs: r.thumbs,
    };
    let extras = Extras {
        music,
        hls: r.hls,
        captions: r.captions.into_iter().map(|(.., src)| src).collect(),
        storyboard: sb_template,
    };
    server::register_full(&set, set.formats.clone(), r.ua.clone(), extras.clone());
    (set, r.ua, extras)
}

async fn streams(video_id: &str, music: bool, force_ytdlp: bool) -> Result<StreamSet, String> {
    if !valid_id(video_id) {
        return Err("invalid video id".into());
    }
    let info = server::start().await?;
    let key: Key = (video_id.to_string(), music);
    let gate = inflight(&key);
    let _held = gate.lock().await;
    if !force_ytdlp {
        let hit = cache()
            .lock()
            .ok()
            .and_then(|g| g.get(&key).cloned())
            .filter(|(set, ..)| set.expires_ms - CACHE_MARGIN_MS > now_ms());
        if let Some((set, ua, extras)) = hit {
            if !server::has_session(&set.session) {
                server::register_full(&set, set.formats.clone(), ua, extras);
            }
            return Ok(set);
        }
    }
    let r = resolve(video_id, music, force_ytdlp).await?;
    let (set, ua, extras) = open(r, music, &info);
    if let Ok(mut g) = cache().lock() {
        g.retain(|_, (s, ..)| s.expires_ms > now_ms());
        g.insert(key, (set.clone(), ua, extras));
    }
    Ok(set)
}

#[tauri::command]
pub async fn tube_streams(
    app: AppHandle,
    video_id: String,
    music: bool,
) -> Result<StreamSet, String> {
    server::ensure(&app).await?;
    streams(&video_id, music, false).await
}

#[tauri::command]
pub async fn tube_streams_fallback(
    app: AppHandle,
    video_id: String,
    music: bool,
) -> Result<StreamSet, String> {
    server::ensure(&app).await?;
    streams(&video_id, music, true).await
}

pub async fn resolve_for_download(video_id: &str, music: bool) -> Result<StreamSet, String> {
    streams(video_id, music, false).await
}

pub(crate) async fn refresh(video_id: &str, music: bool) -> Result<Fresh, String> {
    if let Ok(mut g) = cache().lock() {
        g.remove(&(video_id.to_string(), music));
    }
    let r = resolve(video_id, music, false).await?;
    Ok(Fresh {
        formats: r.formats,
        ua: r.ua,
        hls: r.hls,
    })
}

struct Track {
    playback: String,
    watchtime: String,
    cpn: String,
    registered: bool,
    last_s: f64,
    at: Instant,
}

fn tracks() -> &'static Mutex<HashMap<String, Track>> {
    static T: OnceLock<Mutex<HashMap<String, Track>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cpn() -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..16]);
    bytes
        .iter()
        .map(|b| CHARS[(b & 63) as usize] as char)
        .collect()
}

fn stats_url(url: &str, cpn: &str, extra: &[(&str, String)]) -> String {
    let mut u = url.replacen("://s.youtube.com/", "://www.youtube.com/", 1);
    let mut add = |k: &str, v: &str| {
        if url_param(&u, k).is_none() {
            let sep = if u.contains('?') { '&' } else { '?' };
            u.push(sep);
            u.push_str(k);
            u.push('=');
            u.push_str(&urlencoding::encode(v));
        }
    };
    add("ver", "2");
    add("c", "WEB");
    add("cpn", cpn);
    for (k, v) in extra {
        add(k, v);
    }
    u
}

async fn stats_get(url: &str) -> Result<(), String> {
    let jar = account::jar().ok_or("not signed in")?;
    let mut req = http::client()
        .get(url)
        .header(reqwest::header::USER_AGENT, http::BROWSER_UA);
    for (k, v) in http::auth_headers(&jar, "https://www.youtube.com").unwrap_or_default() {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|_| "network error".to_string())?;
    http::absorb_cookies("www.youtube.com", resp.headers());
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status().as_u16()))
    }
}

async fn report_inner(video_id: &str, position_s: f64) -> Result<(), String> {
    if !http::settings().send_history || !account::is_logged_in() || !valid_id(video_id) {
        return Ok(());
    }
    let known = tracks()
        .lock()
        .map(|g| g.contains_key(video_id))
        .unwrap_or(false);
    if !known {
        let json = http::web("player", |c, v, hl, gl| {
            innertube::player_body(c, v, hl, gl, video_id, None)
        })
        .await?;
        let t = player::parse_player(&json)
            .tracking
            .ok_or("no playback tracking for this video")?;
        let (Some(playback), Some(watchtime)) = (t.playback_url, t.watchtime_url) else {
            return Err("incomplete playback tracking".into());
        };
        if let Ok(mut g) = tracks().lock() {
            g.retain(|_, t| t.at.elapsed() < Duration::from_secs(6 * 3600));
            g.insert(
                video_id.to_string(),
                Track {
                    playback,
                    watchtime,
                    cpn: cpn(),
                    registered: false,
                    last_s: 0.0,
                    at: Instant::now(),
                },
            );
        }
    }
    let (playback, watchtime, cpn, registered, last) = {
        let g = tracks().lock().map_err(|_| "lock")?;
        let t = g.get(video_id).ok_or("no playback session")?;
        (
            t.playback.clone(),
            t.watchtime.clone(),
            t.cpn.clone(),
            t.registered,
            t.last_s,
        )
    };
    if !registered {
        stats_get(&stats_url(&playback, &cpn, &[])).await?;
        if let Ok(mut g) = tracks().lock() {
            if let Some(t) = g.get_mut(video_id) {
                t.registered = true;
            }
        }
    }
    let pos = position_s.max(0.0);
    if pos > last {
        let extra = [
            ("st", format!("{last:.3}")),
            ("et", format!("{pos:.3}")),
            ("cmt", format!("{pos:.3}")),
        ];
        stats_get(&stats_url(&watchtime, &cpn, &extra)).await?;
        if let Ok(mut g) = tracks().lock() {
            if let Some(t) = g.get_mut(video_id) {
                t.last_s = pos;
                t.at = Instant::now();
            }
        }
    }
    Ok(())
}

pub async fn report_watchtime(video_id: &str, position_s: f64) {
    if let Err(e) = report_inner(video_id, position_s).await {
        tracing::debug!("[tube] watch time for {video_id} not sent: {e}");
    }
}
