//! Links played through yt-dlp (YouTube and the sites it knows).
//!
//! `yt-dlp -J` gives the formats; we pick one the webview plays on its own
//! (progressive MP4 with H.264 + AAC, or a muxed HLS stream) and route it
//! through the local proxy with the headers yt-dlp says the CDN wants.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::AppHandle;

use super::{
    merged_prefs, remember_item, resume_for, server, Chapter, PlayMode, PlaybackInfo,
    PlaybackMediaInfo, SourceRef, SubtitleTrack,
};

const CACHE_TTL: Duration = Duration::from_secs(20 * 60);

struct Cached {
    at: Instant,
    json: Value,
}

fn cache() -> &'static Mutex<HashMap<String, Cached>> {
    static C: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Last playable URL per progress key (`url:...`), for frame capture,
/// transcription and silence detection of links.
struct Played {
    url: String,
    subs: Vec<(String, String)>,
}

fn played() -> &'static Mutex<HashMap<String, Played>> {
    static P: OnceLock<Mutex<HashMap<String, Played>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The proxied stream URL last resolved for a key (ffmpeg can read it).
pub fn stream_for_key(key: &str) -> Option<String> {
    played().lock().ok()?.get(key).map(|v| v.url.clone())
}

/// Subtitle `(lang, proxied url)` pairs last resolved for a key.
pub fn subs_for_key(key: &str) -> Vec<(String, String)> {
    played()
        .lock()
        .ok()
        .and_then(|m| m.get(key).map(|v| v.subs.clone()))
        .unwrap_or_default()
}

async fn info_json(url: &str) -> Result<Value, String> {
    if let Ok(c) = cache().lock() {
        if let Some(hit) = c.get(url).filter(|h| h.at.elapsed() < CACHE_TTL) {
            return Ok(hit.json.clone());
        }
    }
    let ytdlp = omniget_core::core::ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {e}"))?;
    let json =
        omniget_core::core::ytdlp::get_video_info(&ytdlp, url, &["--no-playlist".to_string()])
            .await
            .map_err(|e| {
                let msg = e.to_string();
                let short: String = msg
                    .lines()
                    .last()
                    .unwrap_or(&msg)
                    .chars()
                    .take(200)
                    .collect();
                format!("Couldn't open this link: {short}")
            })?;
    if let Ok(mut c) = cache().lock() {
        c.retain(|_, v| v.at.elapsed() < CACHE_TTL);
        c.insert(
            url.to_string(),
            Cached {
                at: Instant::now(),
                json: json.clone(),
            },
        );
    }
    Ok(json)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Progressive,
    Hls,
}

/// Candidates the webview can play without help, best first.
fn candidates(formats: &[Value]) -> Vec<(Kind, &Value)> {
    let mut out: Vec<(Kind, i64, i64, &Value)> = Vec::new();
    for f in formats {
        if f.get("has_drm").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        let proto = s(f, "protocol");
        let (v, a) = (s(f, "vcodec"), s(f, "acodec"));
        let url = s(f, "url");
        if url.is_empty() {
            continue;
        }
        let height = f.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
        let tbr = f.get("tbr").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
        let ext = s(f, "ext");
        let vok = v.starts_with("avc")
            || v.starts_with("h264")
            || (cfg!(target_os = "macos") && (v.starts_with("hvc") || v.starts_with("hev")));
        let aok = a.starts_with("mp4a") || a == "aac" || a == "mp3";
        // extractors that don't report codecs: trust mp4 + both streams
        let unknown = (v.is_empty() || a.is_empty()) && ext == "mp4";
        if proto.starts_with("m3u8") {
            // muxed variants only: the player needs audio in the same stream
            let muxed = v != "none" && a != "none";
            let codecs_ok = (v.is_empty() || vok) && (a.is_empty() || aok);
            if muxed && codecs_ok {
                out.push((Kind::Hls, height.min(1080), tbr, f));
            }
        } else if (proto.starts_with("http") || proto.is_empty())
            && (ext == "mp4" || ext == "m4v" || ext == "mov")
            && ((vok && aok) || unknown)
        {
            out.push((Kind::Progressive, height.min(1080), tbr, f));
        }
    }
    // prefer height (capped at 1080), then progressive over HLS, then bitrate
    out.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| (a.0 == Kind::Hls).cmp(&(b.0 == Kind::Hls)))
            .then(b.2.cmp(&a.2))
    });
    out.into_iter().map(|(k, _, _, f)| (k, f)).collect()
}

fn headers_of(f: &Value, top: &Value) -> Vec<(String, String)> {
    let h = f
        .get("http_headers")
        .or_else(|| top.get("http_headers"))
        .and_then(|v| v.as_object());
    h.map(|o| {
        o.iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
            .collect()
    })
    .unwrap_or_default()
}

fn subtitles(
    json: &Value,
    headers: &[(String, String)],
) -> (Vec<SubtitleTrack>, Vec<(String, String)>) {
    let mut out = Vec::new();
    let mut raw = Vec::new();
    let lang_of_video = s(json, "language").to_string();
    let mut add = |map: Option<&Value>, auto: bool| {
        let Some(obj) = map.and_then(|m| m.as_object()) else {
            return;
        };
        let mut langs: Vec<&String> = obj.keys().collect();
        langs.sort();
        for lang in langs {
            if lang == "live_chat" {
                continue;
            }
            // auto captions come translated to 100+ languages: keep the original
            if auto {
                let orig = lang.ends_with("-orig")
                    || (!lang_of_video.is_empty() && *lang == lang_of_video);
                if !orig {
                    continue;
                }
            }
            let Some(list) = obj[lang].as_array() else {
                continue;
            };
            let Some(vtt) = list.iter().find(|t| s(t, "ext") == "vtt") else {
                continue;
            };
            let u = s(vtt, "url");
            if u.is_empty() {
                continue;
            }
            let clean = lang.trim_end_matches("-orig").to_string();
            let proxied = server::register_remote(u, headers.to_vec());
            let name = s(vtt, "name");
            let label = if !name.is_empty() {
                name.to_string()
            } else {
                super::probe::lang_label(&clean)
            };
            raw.push((clean.clone(), proxied.clone()));
            out.push(SubtitleTrack {
                id: format!("{}{}", if auto { "auto:" } else { "sub:" }, lang),
                label: if auto {
                    format!("{label} (auto)")
                } else {
                    label
                },
                lang: clean,
                url: proxied,
                default: false,
                forced: false,
                source: if auto {
                    "generated".into()
                } else {
                    "remote".into()
                },
            });
        }
    };
    add(json.get("subtitles"), false);
    add(json.get("automatic_captions"), true);
    (out, raw)
}

fn chapters(json: &Value) -> Vec<Chapter> {
    json.get("chapters")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .map(|c| {
                    let st = c.get("start_time").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let en = c.get("end_time").and_then(|v| v.as_f64()).unwrap_or(st);
                    Chapter {
                        start_ms: (st * 1000.0) as i64,
                        end_ms: (en * 1000.0) as i64,
                        title: s(c, "title").to_string(),
                        kind: None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_default()
}

/// Resolves a page URL into playback, skipping the kinds in `avoid`.
async fn resolve_inner(
    app: &AppHandle,
    url: &str,
    avoid: &[Kind],
    reason_prefix: Option<&str>,
) -> Result<PlaybackInfo, String> {
    server::ensure(app).await?;
    let json = info_json(url).await?;
    let empty = Vec::new();
    let formats = json
        .get("formats")
        .and_then(|f| f.as_array())
        .unwrap_or(&empty);
    let is_live = json
        .get("is_live")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let picked = candidates(formats)
        .into_iter()
        .find(|(k, _)| !avoid.contains(k));
    let (kind, fmt, stream_url) = match picked {
        Some((k, f)) => (k, f, s(f, "url").to_string()),
        None => {
            // YouTube and others list HLS variants without audio, but each
            // points to a master playlist whose audio groups the player
            // picks up on its own (native HLS or hls.js)
            let master = formats.iter().find(|f| {
                s(f, "protocol").starts_with("m3u8")
                    && !s(f, "manifest_url").is_empty()
                    && !f.get("has_drm").and_then(|v| v.as_bool()).unwrap_or(false)
            });
            if let (Some(f), true) = (master, !avoid.contains(&Kind::Hls)) {
                (Kind::Hls, f, s(f, "manifest_url").to_string())
            } else {
                // single-format extractors put the media URL at the top level
                let top = s(&json, "url");
                if top.is_empty() || !avoid.is_empty() {
                    return Err("No format of this link plays here. Use Download and watch.".into());
                }
                let k = if s(&json, "protocol").starts_with("m3u8") || top.contains(".m3u8") {
                    Kind::Hls
                } else {
                    Kind::Progressive
                };
                (k, &json, top.to_string())
            }
        }
    };
    let headers = headers_of(fmt, &json);
    let proxied = server::register_remote(&stream_url, headers.clone());
    let page = {
        let w = s(&json, "webpage_url");
        if w.is_empty() {
            url.to_string()
        } else {
            w.to_string()
        }
    };
    // progress follows the page the user opened, so the key stays stable
    let key = format!("url:{url}");
    let title = {
        let t = s(&json, "title");
        if t.is_empty() {
            page.clone()
        } else {
            t.to_string()
        }
    };
    let uploader = [
        s(&json, "uploader"),
        s(&json, "channel"),
        s(&json, "extractor_key"),
    ]
    .into_iter()
    .find(|v| !v.is_empty())
    .map(str::to_string)
    .unwrap_or_else(|| host_of(url));
    let thumb = json
        .get("thumbnail")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let source = SourceRef::Url {
        url: url.to_string(),
    };
    let (subs, raw_subs) = subtitles(&json, &headers);
    let dur_ms = json
        .get("duration")
        .and_then(|v| v.as_f64())
        .map(|d| (d * 1000.0) as i64)
        .unwrap_or(0);
    let k2 = key.clone();
    let (t2, u2, th2, src2) = (
        title.clone(),
        uploader.clone(),
        thumb.clone(),
        source.clone(),
    );
    let (prefs, start_ms) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        remember_item(&k2, &t2, &u2, th2.as_deref(), &src2)?;
        let start = if is_live { 0 } else { resume_for(&k2)?.0 };
        Ok((merged_prefs(&[format!("media:{k2}")]), start))
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Ok(mut p) = played().lock() {
        p.insert(
            key.clone(),
            Played {
                url: proxied.clone(),
                subs: raw_subs,
            },
        );
    }
    let mut subs = subs;
    // subtitle_id "off" = turned off on purpose
    let off = prefs.subtitle_id.as_deref() == Some("off");
    let pick = prefs
        .subtitle_id
        .as_ref()
        .filter(|_| !off)
        .and_then(|sid| subs.iter().position(|t| &t.id == sid))
        .or_else(|| {
            if off {
                return None;
            }
            let want = prefs.subtitle_lang.as_ref()?.to_ascii_lowercase();
            subs.iter()
                .position(|t| t.lang.to_ascii_lowercase().starts_with(&want))
        });
    if let Some(i) = pick {
        subs[i].default = true;
    }
    let mode = if kind == Kind::Hls {
        PlayMode::Hls
    } else {
        PlayMode::Remote
    };
    let vcodec = s(fmt, "vcodec").to_string();
    let acodec = s(fmt, "acodec").to_string();
    let what = if kind == Kind::Hls {
        "HLS stream"
    } else {
        "MP4 stream"
    };
    let height = fmt.get("height").and_then(|v| v.as_i64());
    let reason = match reason_prefix {
        Some(p) => format!("{p}, playing the {what}"),
        None => match height {
            Some(h) => format!("Playing the {h}p {what} from {}", host_of(url)),
            None => format!("Playing the {what} from {}", host_of(url)),
        },
    };
    Ok(PlaybackInfo {
        key,
        title,
        subtitle: uploader,
        source,
        url: proxied,
        mode,
        seek_reloads: false,
        offset_ms: 0,
        reason,
        duration_ms: dur_ms,
        start_ms,
        is_live,
        subtitles: subs,
        audio_tracks: Vec::new(),
        chapters: chapters(&json),
        info: PlaybackMediaInfo {
            container: s(fmt, "ext").to_string(),
            vcodec,
            acodec,
            width: fmt.get("width").and_then(|v| v.as_i64()),
            height,
            bitrate: fmt
                .get("tbr")
                .and_then(|v| v.as_f64())
                .map(|t| (t * 1000.0) as i64),
        },
        prefs,
        next: None,
        prev: None,
        course_id: None,
    })
}

/// Resolves a page URL (YouTube and the sites yt-dlp knows) into playback.
pub async fn resolve(app: &AppHandle, url: &str) -> Result<PlaybackInfo, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web link (http or https)".into());
    }
    resolve_inner(app, url, &[], None).await
}

/// After the chosen format failed in the player: drop the cached formats
/// (the signed URLs may have expired) and try the other kind of stream.
pub async fn resolve_fallback(
    app: &AppHandle,
    url: &str,
    failed: PlayMode,
) -> Result<PlaybackInfo, String> {
    if let Ok(mut c) = cache().lock() {
        c.remove(url);
    }
    let avoid = match failed {
        PlayMode::Hls => vec![Kind::Hls],
        _ => vec![Kind::Progressive],
    };
    resolve_inner(app, url, &avoid, Some("The first stream failed")).await
}

#[tauri::command]
pub async fn video_link_resolve(app: AppHandle, url: String) -> Result<PlaybackInfo, String> {
    resolve(&app, &url).await
}
