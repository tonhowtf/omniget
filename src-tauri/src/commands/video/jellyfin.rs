use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::sources::{self, SourceInfo, SourceRow};
use super::{
    server, Chapter, PlayMode, PlaybackInfo, PlaybackMediaInfo, SkipKind, SourceRef, SubtitleTrack,
};

const TICKS_PER_MS: i64 = 10_000;
const CALL_LIMIT: Duration = Duration::from_secs(20);
const LIST_FIELDS: &str = "Overview,ChildCount,PrimaryImageAspectRatio,ProductionYear";

fn device_id() -> String {
    if let Some(id) = sources::kv_get("jf_device_id") {
        return id;
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    sources::kv_set("jf_device_id", &id);
    id
}

fn device_name() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let from_env = std::env::var("COMPUTERNAME")
            .ok()
            .or_else(|| std::env::var("HOSTNAME").ok());
        let name = from_env.or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
        });
        name.map(|n| n.trim().trim_end_matches(".local").to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "OmniGet Desktop".into())
    })
    .clone()
}

fn quote(v: &str) -> String {
    urlencoding::encode(v).into_owned()
}

fn auth_value(scheme: &str, token: Option<&str>) -> String {
    let mut s = format!(
        "{scheme} Client=\"OmniGet\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
        quote(&device_name()),
        device_id(),
        env!("CARGO_PKG_VERSION"),
    );
    if let Some(t) = token {
        s.push_str(&format!(", Token=\"{t}\""));
    }
    s
}

fn auth_headers(token: Option<&str>) -> Vec<(String, String)> {
    vec![
        ("Authorization".into(), auth_value("MediaBrowser", token)),
        ("X-Emby-Authorization".into(), auth_value("Emby", token)),
    ]
}

struct Conn {
    id: i64,
    base: String,
    user: String,
    token: String,
}

impl Conn {
    fn load(id: i64) -> Result<Self, String> {
        let row = sources::get_kind(id, "jellyfin")?;
        Self::from_row(&row)
    }

    fn from_row(row: &SourceRow) -> Result<Self, String> {
        let token =
            sources::secret_get(&format!("jf:{}", row.id)).ok_or("Sign in to this server again")?;
        Ok(Self {
            id: row.id,
            base: row.url.clone(),
            user: row
                .config
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            token,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn api_key(&self) -> String {
        quote(&self.token)
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        request(&self.base, method, path, Some(&self.token), body).await
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        self.send(reqwest::Method::GET, path, None).await
    }

    async fn get_either(&self, new: &str, legacy: &str) -> Result<Value, String> {
        match self.get(new).await {
            Err(e) if e == "Not found" => self.get(legacy).await,
            r => r,
        }
    }

    async fn call<T>(
        &self,
        fut: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        sources::tracked(self.id, CALL_LIMIT, fut).await
    }

    fn image(&self, item: &str, kind: &str, tag: Option<&str>, width: u32) -> String {
        let mut u = self.url(&format!(
            "/Items/{item}/Images/{kind}?maxWidth={width}&quality=90&ApiKey={}",
            self.api_key()
        ));
        if let Some(t) = tag {
            u.push_str(&format!("&tag={}", quote(t)));
        }
        u
    }
}

async fn request(
    base: &str,
    method: reqwest::Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut req = sources::http()
        .request(method, format!("{base}{path}"))
        .header("Accept", "application/json");
    for (k, v) in auth_headers(token) {
        req = req.header(k, v);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(sources::net_err)?;
    let status = resp.status();
    if !status.is_success() {
        return Err(sources::status_err(status));
    }
    let text = resp.text().await.map_err(sources::net_err)?;
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|_| "Unexpected answer from the server".to_string())
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

fn i(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(|x| x.as_i64())
}

#[derive(Serialize, Clone, Debug)]
pub struct JfView {
    pub id: String,
    pub name: String,
    pub collection_type: Option<String>,
    pub image: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct JfItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub year: Option<i64>,
    pub overview: Option<String>,
    pub image: Option<String>,
    pub backdrop: Option<String>,
    pub series_name: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub runtime_ms: Option<i64>,
    pub position_ms: Option<i64>,
    pub played: bool,
    pub child_count: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct JfItems {
    pub items: Vec<JfItem>,
    pub total: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct QuickConnect {
    pub code: String,
    pub secret: String,
}

fn map_item(c: &Conn, v: &Value) -> JfItem {
    let id = s(v, "Id").unwrap_or_default();
    let typ = s(v, "Type").unwrap_or_default();
    let tags = v.get("ImageTags");
    let primary_tag = tags.and_then(|t| t.get("Primary")).and_then(|t| t.as_str());
    let thumb_tag = tags.and_then(|t| t.get("Thumb")).and_then(|t| t.as_str());
    let image = if primary_tag.is_some() {
        Some(c.image(&id, "Primary", primary_tag, 400))
    } else if thumb_tag.is_some() {
        Some(c.image(&id, "Thumb", thumb_tag, 400))
    } else {
        let series_tag = v.get("SeriesPrimaryImageTag").and_then(|t| t.as_str());
        s(v, "SeriesId")
            .filter(|_| series_tag.is_some())
            .map(|series| c.image(&series, "Primary", series_tag, 400))
    };
    let backdrop = v
        .get("BackdropImageTags")
        .and_then(|b| b.as_array())
        .and_then(|a| a.first())
        .and_then(|t| t.as_str())
        .map(|t| c.image(&id, "Backdrop", Some(t), 1280))
        .or_else(|| {
            let parent = s(v, "ParentBackdropItemId")?;
            let tag = v
                .get("ParentBackdropImageTags")
                .and_then(|b| b.as_array())
                .and_then(|a| a.first())
                .and_then(|t| t.as_str());
            Some(c.image(&parent, "Backdrop", tag, 1280))
        });
    let user = v.get("UserData");
    let (season, episode) = if typ == "Season" {
        (i(v, "IndexNumber"), None)
    } else {
        (i(v, "ParentIndexNumber"), i(v, "IndexNumber"))
    };
    JfItem {
        name: s(v, "Name").unwrap_or_default(),
        year: i(v, "ProductionYear"),
        overview: s(v, "Overview").filter(|o| !o.trim().is_empty()),
        image,
        backdrop,
        series_name: s(v, "SeriesName"),
        season,
        episode,
        runtime_ms: i(v, "RunTimeTicks").map(|t| t / TICKS_PER_MS),
        position_ms: user
            .and_then(|u| u.get("PlaybackPositionTicks"))
            .and_then(|t| t.as_i64())
            .filter(|t| *t > 0)
            .map(|t| t / TICKS_PER_MS),
        played: user
            .and_then(|u| u.get("Played"))
            .and_then(|p| p.as_bool())
            .unwrap_or(false),
        child_count: i(v, "ChildCount"),
        typ,
        id,
    }
}

fn map_items(c: &Conn, v: &Value) -> Vec<JfItem> {
    let arr = v
        .get("Items")
        .and_then(|a| a.as_array())
        .or_else(|| v.as_array());
    arr.map(|a| a.iter().map(|x| map_item(c, x)).collect())
        .unwrap_or_default()
}

async fn server_public_info(base: &str) -> Value {
    request(
        base,
        reqwest::Method::GET,
        "/System/Info/Public",
        None,
        None,
    )
    .await
    .unwrap_or(Value::Null)
}

async fn save_auth(base: &str, auth: Value) -> Result<SourceInfo, String> {
    let token = s(&auth, "AccessToken").ok_or("The server did not return a token")?;
    let user = auth.get("User").cloned().unwrap_or(Value::Null);
    let user_id = s(&user, "Id").ok_or("The server did not return a user")?;
    let user_name = s(&user, "Name").unwrap_or_default();
    let info = server_public_info(base).await;
    let server_name = s(&info, "ServerName").unwrap_or_else(|| {
        url::Url::parse(base)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| base.to_string())
    });
    let product = s(&info, "ProductName").unwrap_or_else(|| "Jellyfin Server".into());
    let config = json!({
        "key": format!("{base}#{user_id}"),
        "user_id": user_id,
        "user": user_name,
        "server_name": server_name,
        "server_id": s(&auth, "ServerId").or_else(|| s(&info, "Id")),
        "product": product,
    });
    let name = if user_name.is_empty() {
        server_name.clone()
    } else {
        format!("{server_name} · {user_name}")
    };
    let id = sources::upsert("jellyfin", &name, base, &config)?;
    sources::secret_set(&format!("jf:{id}"), &token)?;
    sources::info_by_id(id)
}

#[tauri::command]
pub async fn video_jf_connect(
    server: String,
    username: String,
    password: String,
) -> Result<SourceInfo, String> {
    let base = sources::normalize_base(&server)?;
    if username.trim().is_empty() {
        return Err("Enter a user name".into());
    }
    let auth = request(
        &base,
        reqwest::Method::POST,
        "/Users/AuthenticateByName",
        None,
        Some(json!({ "Username": username.trim(), "Pw": password })),
    )
    .await
    .map_err(|e| {
        if e.starts_with("Sign-in expired") {
            "Wrong user name or password".to_string()
        } else {
            e
        }
    })?;
    save_auth(&base, auth).await
}

#[tauri::command]
pub async fn video_jf_quickconnect_start(server: String) -> Result<QuickConnect, String> {
    let base = sources::normalize_base(&server)?;
    let enabled = request(
        &base,
        reqwest::Method::GET,
        "/QuickConnect/Enabled",
        None,
        None,
    )
    .await?;
    if enabled.as_bool() == Some(false) {
        return Err("Quick Connect is off on this server".into());
    }
    let r = request(
        &base,
        reqwest::Method::POST,
        "/QuickConnect/Initiate",
        None,
        None,
    )
    .await?;
    Ok(QuickConnect {
        code: s(&r, "Code").ok_or("Quick Connect did not start")?,
        secret: s(&r, "Secret").ok_or("Quick Connect did not start")?,
    })
}

#[tauri::command]
pub async fn video_jf_quickconnect_poll(
    server: String,
    secret: String,
) -> Result<Option<SourceInfo>, String> {
    let base = sources::normalize_base(&server)?;
    let state = request(
        &base,
        reqwest::Method::GET,
        &format!("/QuickConnect/Connect?secret={}", quote(&secret)),
        None,
        None,
    )
    .await
    .map_err(|e| {
        if e == "Not found" {
            "The code expired, start again".to_string()
        } else {
            e
        }
    })?;
    if state.get("Authenticated").and_then(|a| a.as_bool()) != Some(true) {
        return Ok(None);
    }
    let auth = request(
        &base,
        reqwest::Method::POST,
        "/Users/AuthenticateWithQuickConnect",
        None,
        Some(json!({ "Secret": secret })),
    )
    .await?;
    save_auth(&base, auth).await.map(Some)
}

#[tauri::command]
pub async fn video_jf_views(source: i64) -> Result<Vec<JfView>, String> {
    let c = Conn::load(source)?;
    let v = c
        .call(c.get_either(
            &format!("/UserViews?userId={}", c.user),
            &format!("/Users/{}/Views", c.user),
        ))
        .await?;
    let items = v
        .get("Items")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(items
        .iter()
        .map(|x| {
            let id = s(x, "Id").unwrap_or_default();
            let tag = x
                .get("ImageTags")
                .and_then(|t| t.get("Primary"))
                .and_then(|t| t.as_str());
            JfView {
                name: s(x, "Name").unwrap_or_default(),
                collection_type: s(x, "CollectionType"),
                image: tag.map(|t| c.image(&id, "Primary", Some(t), 600)),
                id,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn video_jf_items(
    source: i64,
    parent_id: Option<String>,
    search: Option<String>,
    types: Option<Vec<String>>,
    start: Option<i64>,
    limit: Option<i64>,
) -> Result<JfItems, String> {
    let c = Conn::load(source)?;
    let search = search
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty());
    let types = types.filter(|t| !t.is_empty());
    let mut q = format!(
        "SortBy=SortName&SortOrder=Ascending&StartIndex={}&Limit={}&Fields={LIST_FIELDS}\
         &EnableImageTypes=Primary,Backdrop,Thumb&ImageTypeLimit=1&EnableTotalRecordCount=true",
        start.unwrap_or(0).max(0),
        limit.unwrap_or(60).clamp(1, 500),
    );
    if let Some(p) = &parent_id {
        q.push_str(&format!("&ParentId={}", quote(p)));
    }
    if let Some(t) = &types {
        q.push_str(&format!("&IncludeItemTypes={}", quote(&t.join(","))));
    }
    if let Some(term) = &search {
        q.push_str(&format!("&SearchTerm={}", quote(term)));
    }
    if types.is_some() || search.is_some() {
        q.push_str("&Recursive=true");
    }
    let v = c
        .call(c.get_either(
            &format!("/Items?userId={}&{q}", c.user),
            &format!("/Users/{}/Items?{q}", c.user),
        ))
        .await?;
    let items = map_items(&c, &v);
    let total = i(&v, "TotalRecordCount").unwrap_or(items.len() as i64);
    Ok(JfItems { items, total })
}

#[tauri::command]
pub async fn video_jf_resume(source: i64) -> Result<Vec<JfItem>, String> {
    let c = Conn::load(source)?;
    let q = format!("MediaTypes=Video&Limit=24&Fields={LIST_FIELDS}&EnableImageTypes=Primary,Backdrop,Thumb&ImageTypeLimit=1");
    let v = c
        .call(c.get_either(
            &format!("/UserItems/Resume?userId={}&{q}", c.user),
            &format!("/Users/{}/Items/Resume?{q}", c.user),
        ))
        .await?;
    Ok(map_items(&c, &v))
}

#[tauri::command]
pub async fn video_jf_nextup(source: i64) -> Result<Vec<JfItem>, String> {
    let c = Conn::load(source)?;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(365);
    let v = c
        .call(c.get(&format!(
            "/Shows/NextUp?userId={}&Limit=24&Fields={LIST_FIELDS}&EnableResumable=false\
             &EnableRewatching=false&DisableFirstEpisode=true&NextUpDateCutoff={}",
            c.user,
            quote(&cutoff.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        )))
        .await?;
    Ok(map_items(&c, &v))
}

async fn fetch_item(c: &Conn, id: &str) -> Result<Value, String> {
    c.get_either(
        &format!(
            "/Items/{id}?userId={}&Fields=Overview,Chapters,MediaSources,ChildCount",
            c.user
        ),
        &format!("/Users/{}/Items/{id}", c.user),
    )
    .await
}

#[tauri::command]
pub async fn video_jf_item(source: i64, id: String) -> Result<JfItem, String> {
    let c = Conn::load(source)?;
    let v = c.call(fetch_item(&c, &id)).await?;
    Ok(map_item(&c, &v))
}

#[tauri::command]
pub async fn video_jf_children(source: i64, id: String) -> Result<Vec<JfItem>, String> {
    let c = Conn::load(source)?;
    let item = c.call(fetch_item(&c, &id)).await?;
    let fields =
        format!("Fields={LIST_FIELDS}&EnableImageTypes=Primary,Backdrop,Thumb&ImageTypeLimit=1");
    let v = match s(&item, "Type").as_deref() {
        Some("Series") => {
            c.call(c.get(&format!("/Shows/{id}/Seasons?userId={}&{fields}", c.user)))
                .await?
        }
        Some("Season") => {
            let series = s(&item, "SeriesId").unwrap_or_default();
            c.call(c.get(&format!(
                "/Shows/{series}/Episodes?userId={}&SeasonId={id}&{fields}",
                c.user
            )))
            .await?
        }
        _ => {
            c.call(c.get_either(
                &format!(
                    "/Items?userId={}&ParentId={id}&SortBy=SortName&{fields}",
                    c.user
                ),
                &format!(
                    "/Users/{}/Items?ParentId={id}&SortBy=SortName&{fields}",
                    c.user
                ),
            ))
            .await?
        }
    };
    Ok(map_items(&c, &v))
}

#[derive(Clone)]
struct Session {
    media_source: String,
    play_session: String,
    method: &'static str,
    runtime_ms: i64,
}

fn sessions() -> &'static Mutex<HashMap<(i64, String), Session>> {
    static S: OnceLock<Mutex<HashMap<(i64, String), Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn device_profile() -> Value {
    let mut mp4_video = vec!["h264", "vp9", "av1"];
    if cfg!(target_os = "macos") {
        mp4_video.push("hevc");
    }
    json!({
        "Name": "OmniGet",
        "MaxStreamingBitrate": 120_000_000,
        "MaxStaticBitrate": 120_000_000,
        "MusicStreamingTranscodingBitrate": 192_000,
        "DirectPlayProfiles": [
            { "Container": "mp4,m4v", "Type": "Video", "VideoCodec": mp4_video.join(","), "AudioCodec": "aac,mp3,opus,flac" },
            { "Container": "webm", "Type": "Video", "VideoCodec": "vp8,vp9,av1", "AudioCodec": "vorbis,opus" },
            { "Container": "mp3,aac,m4a,flac,webm", "Type": "Audio" }
        ],
        "TranscodingProfiles": [
            { "Container": "ts", "Type": "Video", "VideoCodec": "h264", "AudioCodec": "aac,mp3",
              "Context": "Streaming", "Protocol": "hls", "MaxAudioChannels": "2",
              "MinSegments": 1, "BreakOnNonKeyFrames": true },
            { "Container": "aac", "Type": "Audio", "AudioCodec": "aac", "Context": "Streaming", "Protocol": "http" }
        ],
        "ContainerProfiles": [],
        "CodecProfiles": [],
        "SubtitleProfiles": [
            { "Format": "vtt", "Method": "External" },
            { "Format": "srt", "Method": "External" },
            { "Format": "subrip", "Method": "External" },
            { "Format": "ass", "Method": "External" },
            { "Format": "ssa", "Method": "External" }
        ]
    })
}

fn segment_kind(t: &Value) -> Option<SkipKind> {
    let name = match t {
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_i64()? {
            1 => "Commercial".into(),
            2 => "Preview".into(),
            3 => "Recap".into(),
            4 => "Outro".into(),
            5 => "Intro".into(),
            _ => return None,
        },
        _ => return None,
    };
    match name.as_str() {
        "Intro" => Some(SkipKind::Intro),
        "Outro" => Some(SkipKind::Outro),
        "Recap" => Some(SkipKind::Recap),
        "Preview" => Some(SkipKind::Preview),
        "Commercial" => Some(SkipKind::Sponsor),
        _ => None,
    }
}

fn segment_title(k: SkipKind) -> &'static str {
    match k {
        SkipKind::Intro => "Intro",
        SkipKind::Outro => "Credits",
        SkipKind::Recap => "Recap",
        SkipKind::Preview => "Preview",
        SkipKind::Sponsor => "Commercial",
    }
}

fn chapters_of(item: &Value, runtime_ms: i64, segments: &Value) -> Vec<Chapter> {
    let mut out: Vec<Chapter> = Vec::new();
    if let Some(arr) = item.get("Chapters").and_then(|c| c.as_array()) {
        let starts: Vec<(i64, String)> = arr
            .iter()
            .map(|c| {
                (
                    i(c, "StartPositionTicks").unwrap_or(0) / TICKS_PER_MS,
                    s(c, "Name").unwrap_or_default(),
                )
            })
            .collect();
        for (n, (start, name)) in starts.iter().enumerate() {
            let end = starts.get(n + 1).map(|x| x.0).unwrap_or(runtime_ms);
            if end > *start {
                out.push(Chapter {
                    start_ms: *start,
                    end_ms: end,
                    title: name.clone(),
                    kind: None,
                });
            }
        }
    }
    if let Some(arr) = segments.get("Items").and_then(|c| c.as_array()) {
        for seg in arr {
            let Some(kind) = seg.get("Type").and_then(segment_kind) else {
                continue;
            };
            let start = i(seg, "StartTicks").unwrap_or(0) / TICKS_PER_MS;
            let end = i(seg, "EndTicks").unwrap_or(0) / TICKS_PER_MS;
            if end > start {
                out.push(Chapter {
                    start_ms: start,
                    end_ms: end,
                    title: segment_title(kind).into(),
                    kind: Some(kind),
                });
            }
        }
    }
    out.sort_by_key(|c| c.start_ms);
    out
}

fn transcode_reason(ms: &Value) -> String {
    let reasons: Vec<String> = ms
        .get("TranscodeReasons")
        .map(|r| match r {
            Value::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect(),
            Value::String(s) => s.split(',').map(|x| x.trim().to_string()).collect(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    if reasons.is_empty() {
        return "Server converts this video".into();
    }
    let words: Vec<String> = reasons
        .iter()
        .map(|r| {
            let mut w = String::new();
            for (n, ch) in r.chars().enumerate() {
                if ch.is_uppercase() && n > 0 {
                    w.push(' ');
                }
                w.push(ch.to_ascii_lowercase());
            }
            w
        })
        .collect();
    format!("Server converts: {}", words.join(", "))
}

fn stream_codec(ms: &Value, kind: &str) -> (String, Option<i64>, Option<i64>) {
    let st = ms
        .get("MediaStreams")
        .and_then(|a| a.as_array())
        .and_then(|a| {
            a.iter()
                .find(|x| x.get("Type").and_then(|t| t.as_str()) == Some(kind))
        });
    match st {
        Some(x) => (
            s(x, "Codec").unwrap_or_default(),
            i(x, "Width"),
            i(x, "Height"),
        ),
        None => (String::new(), None, None),
    }
}

pub async fn play(app: &AppHandle, source: i64, item_id: &str) -> Result<PlaybackInfo, String> {
    let c = Conn::load(source)?;
    let item = c.call(fetch_item(&c, item_id)).await?;
    let runtime_ms = i(&item, "RunTimeTicks").unwrap_or(0) / TICKS_PER_MS;
    let key = format!("jf:{source}:{item_id}");
    let server_pos = item
        .get("UserData")
        .and_then(|u| u.get("PlaybackPositionTicks"))
        .and_then(|t| t.as_i64())
        .unwrap_or(0)
        / TICKS_PER_MS;
    let local_pos = super::resume_for(&key).map(|r| r.0).unwrap_or(0);
    let start_ms = if server_pos > 0 {
        (server_pos - 5_000).max(0)
    } else {
        local_pos
    };

    let pb = c
        .call(c.send(
            reqwest::Method::POST,
            &format!(
                "/Items/{item_id}/PlaybackInfo?userId={}&StartTimeTicks={}&IsPlayback=true\
                 &AutoOpenLiveStream=true&MaxStreamingBitrate=120000000",
                c.user,
                start_ms * TICKS_PER_MS
            ),
            Some(json!({
                "UserId": c.user,
                "DeviceProfile": device_profile(),
                "MaxStreamingBitrate": 120_000_000,
                "StartTimeTicks": start_ms * TICKS_PER_MS,
                "EnableDirectPlay": true,
                "EnableDirectStream": true,
                "EnableTranscoding": true,
                "AllowVideoStreamCopy": true,
                "AllowAudioStreamCopy": true,
                "AutoOpenLiveStream": true,
            })),
        ))
        .await?;
    if let Some(code) = s(&pb, "ErrorCode") {
        return Err(match code.as_str() {
            "NotAllowed" => "This account can't play here".into(),
            "NoCompatibleStream" => "No playable version on the server".into(),
            "RateLimitExceeded" => "The server is limiting playback".into(),
            _ => format!("Server refused playback ({code})"),
        });
    }
    let ms = pb
        .get("MediaSources")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .cloned()
        .ok_or("No playable version on the server")?;
    let ms_id = s(&ms, "Id").unwrap_or_else(|| item_id.to_string());
    let play_session = s(&pb, "PlaySessionId").unwrap_or_default();
    let container = s(&ms, "Container").unwrap_or_default();
    let direct = ms.get("SupportsDirectPlay").and_then(|b| b.as_bool()) == Some(true);
    server::ensure(app).await?;
    let (url, mode, method, reason) = if direct {
        let ext = container.split(',').next().unwrap_or("mp4").to_string();
        let direct_url = c.url(&format!(
            "/Videos/{item_id}/stream.{ext}?Static=true&MediaSourceId={}&DeviceId={}&PlaySessionId={}&ApiKey={}",
            quote(&ms_id),
            device_id(),
            quote(&play_session),
            c.api_key()
        ));
        (
            server::register_remote(&direct_url, auth_headers(Some(&c.token))),
            PlayMode::Remote,
            "DirectPlay",
            "Plays the original file from the server".to_string(),
        )
    } else {
        let t = s(&ms, "TranscodingUrl").ok_or("The server can't convert this video")?;
        let abs = if t.starts_with("http") { t } else { c.url(&t) };
        (
            server::register_remote(&abs, auth_headers(Some(&c.token))),
            PlayMode::Hls,
            "Transcode",
            transcode_reason(&ms),
        )
    };

    let mut subtitles = Vec::new();
    if let Some(streams) = ms.get("MediaStreams").and_then(|a| a.as_array()) {
        for st in streams {
            if s(st, "Type").as_deref() != Some("Subtitle") {
                continue;
            }
            if st.get("IsTextSubtitleStream").and_then(|b| b.as_bool()) == Some(false) {
                continue;
            }
            let index = i(st, "Index").unwrap_or(0);
            let path = match s(st, "DeliveryUrl") {
                Some(u) if s(st, "DeliveryMethod").as_deref() == Some("External") => {
                    let (p, _) = u.split_once('?').unwrap_or((u.as_str(), ""));
                    match p.rfind("/Stream.") {
                        Some(pos) => format!("{}/Stream.vtt", &p[..pos]),
                        None => p.to_string(),
                    }
                }
                _ => format!("/Videos/{item_id}/{ms_id}/Subtitles/{index}/0/Stream.vtt"),
            };
            let abs = if path.starts_with("http") {
                path
            } else {
                c.url(&path)
            };
            let lang = s(st, "Language").unwrap_or_default();
            subtitles.push(SubtitleTrack {
                id: format!("jf-{index}"),
                label: s(st, "DisplayTitle")
                    .or_else(|| s(st, "Title"))
                    .unwrap_or_else(|| lang.clone()),
                lang,
                url: server::register_remote(&abs, auth_headers(Some(&c.token))),
                default: st
                    .get("IsDefault")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false),
                forced: st
                    .get("IsForced")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false),
                source: "remote".into(),
            });
        }
    }

    let segments = c
        .get(&format!("/MediaSegments/{item_id}"))
        .await
        .unwrap_or(Value::Null);
    let chapters = chapters_of(&item, runtime_ms, &segments);

    let (mut prev, mut next) = (None, None);
    if s(&item, "Type").as_deref() == Some("Episode") {
        if let Some(series) = s(&item, "SeriesId") {
            if let Ok(adj) = c
                .get(&format!(
                    "/Shows/{series}/Episodes?userId={}&AdjacentTo={item_id}",
                    c.user
                ))
                .await
            {
                let ids: Vec<String> = adj
                    .get("Items")
                    .and_then(|a| a.as_array())
                    .map(|a| a.iter().filter_map(|x| s(x, "Id")).collect())
                    .unwrap_or_default();
                if let Some(pos) = ids.iter().position(|x| x == item_id) {
                    let to_ref = |id: &String| SourceRef::Jellyfin {
                        source,
                        item: id.clone(),
                    };
                    prev = pos.checked_sub(1).and_then(|p| ids.get(p)).map(to_ref);
                    next = ids.get(pos + 1).map(to_ref);
                }
            }
        }
    }

    let jf = map_item(&c, &item);
    let title = jf.name.clone();
    let subtitle = match (&jf.series_name, jf.season, jf.episode) {
        (Some(show), Some(se), Some(ep)) => format!("S{se:02}E{ep:02} · {show}"),
        (Some(show), _, _) => show.clone(),
        _ => jf.year.map(|y| y.to_string()).unwrap_or_default(),
    };
    let source_ref = SourceRef::Jellyfin {
        source,
        item: item_id.to_string(),
    };
    let _ = super::remember_item(&key, &title, &subtitle, jf.image.as_deref(), &source_ref);

    sessions().lock().map_err(|e| e.to_string())?.insert(
        (source, item_id.to_string()),
        Session {
            media_source: ms_id,
            play_session,
            method,
            runtime_ms,
        },
    );

    let (vcodec, width, height) = stream_codec(&ms, "Video");
    let (acodec, _, _) = stream_codec(&ms, "Audio");
    let mut out = PlaybackInfo::remote(key, source_ref, url);
    out.title = title;
    out.subtitle = subtitle;
    out.mode = mode;
    out.reason = reason;
    out.duration_ms = runtime_ms;
    out.start_ms = start_ms;
    out.is_live = s(&item, "Type").as_deref() == Some("TvChannel");
    out.subtitles = subtitles;
    out.chapters = chapters;
    out.info = PlaybackMediaInfo {
        container,
        vcodec,
        acodec,
        width,
        height,
        bitrate: i(&ms, "Bitrate"),
    };
    out.next = next;
    out.prev = prev;
    let mut scopes = Vec::new();
    if let Some(show) = &jf.series_name {
        scopes.push(format!("show:{show}"));
    }
    scopes.push(format!("media:{}", out.key));
    out.prefs = super::merged_prefs(&scopes);
    Ok(out)
}

#[tauri::command]
pub async fn video_jf_play(
    app: AppHandle,
    source: i64,
    item_id: String,
) -> Result<PlaybackInfo, String> {
    play(&app, source, &item_id).await
}

pub async fn report(
    source: i64,
    item_id: &str,
    event: &str,
    position_ms: i64,
    paused: bool,
) -> Result<(), String> {
    let session = sessions()
        .lock()
        .map_err(|e| e.to_string())?
        .get(&(source, item_id.to_string()))
        .cloned();
    let key = format!("jf:{source}:{item_id}");
    let runtime = session.as_ref().map(|s| s.runtime_ms).unwrap_or(0);
    let _ = super::progress_record(
        &key,
        position_ms.max(0),
        runtime,
        !paused && event != "stop",
    );

    let c = Conn::load(source)?;
    let mut body = json!({
        "ItemId": item_id,
        "PositionTicks": position_ms.max(0) * TICKS_PER_MS,
        "IsPaused": paused,
        "CanSeek": true,
        "PlayMethod": session.as_ref().map(|s| s.method).unwrap_or("DirectPlay"),
    });
    if let Some(s) = &session {
        body["MediaSourceId"] = json!(s.media_source);
        body["PlaySessionId"] = json!(s.play_session);
    }
    let path = match event {
        "start" => "/Sessions/Playing",
        "progress" => "/Sessions/Playing/Progress",
        "stop" => "/Sessions/Playing/Stopped",
        _ => return Err("Unknown report".into()),
    };
    c.call(c.send(reqwest::Method::POST, path, Some(body)))
        .await?;
    if event == "stop" {
        if let Some(s) = session {
            if s.method == "Transcode" {
                let _ = c
                    .send(
                        reqwest::Method::DELETE,
                        &format!(
                            "/Videos/ActiveEncodings?deviceId={}&playSessionId={}",
                            device_id(),
                            quote(&s.play_session)
                        ),
                        None,
                    )
                    .await;
            }
        }
        if let Ok(mut m) = sessions().lock() {
            m.remove(&(source, item_id.to_string()));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn video_jf_report(
    source: i64,
    item_id: String,
    event: String,
    position_ms: i64,
    paused: bool,
) -> Result<(), String> {
    report(source, &item_id, &event, position_ms, paused).await
}
