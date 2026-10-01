use std::collections::HashMap;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Duration;

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;
use unicode_normalization::UnicodeNormalization;

use omniget_core::core::video::{m3u, xmltv};

use super::sources::{self, SourceInfo, SourceRow};
use super::{db, server, PlayMode, PlaybackInfo, SourceRef};

const EPG_TTL_MS: i64 = 12 * 3600 * 1000;
const EPG_PAST_MS: i64 = 6 * 3600 * 1000;
const EPG_AHEAD_MS: i64 = 7 * 24 * 3600 * 1000;
const PLAYLIST_LIMIT: Duration = Duration::from_secs(60);
const EPG_LIMIT: Duration = Duration::from_secs(180);
const XTREAM_UA: &str = "VLC/3.0.20 LibVLC/3.0.20";
const MAX_CHANNELS_LISTED: i64 = 5000;

#[derive(Serialize, Clone, Debug)]
pub struct EpgProgramme {
    pub channel: i64,
    pub start: i64,
    pub stop: i64,
    pub title: String,
    pub desc: Option<String>,
    pub category: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct IptvChannel {
    pub id: i64,
    pub source: i64,
    pub name: String,
    pub group: String,
    pub logo: Option<String>,
    pub tvg_id: Option<String>,
    pub favorite: bool,
    pub now: Option<EpgProgramme>,
    pub next: Option<EpgProgramme>,
}

#[derive(Serialize, Clone, Debug)]
pub struct IptvGroup {
    pub name: String,
    pub count: i64,
}

type EpgRow = (String, i64, i64, String, Option<String>, Option<String>);

type ChannelRow = (
    i64,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
);

struct NewChannel {
    name: String,
    group: String,
    logo: Option<String>,
    tvg_id: Option<String>,
    tvg_name: Option<String>,
    url: String,
    headers: Vec<(String, String)>,
}

pub fn norm_name(s: &str) -> String {
    let plain: String = s
        .nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase();
    let words: Vec<String> = plain
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    const TAGS: [&str; 12] = [
        "hd", "fhd", "uhd", "sd", "4k", "hevc", "h265", "h264", "1080p", "720p", "backup", "tv",
    ];
    let mut kept: Vec<&String> = words.iter().collect();
    while kept.len() > 1 && kept.last().is_some_and(|w| TAGS.contains(&w.as_str())) {
        kept.pop();
    }
    kept.into_iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("")
}

fn epg_path(source: i64) -> Option<PathBuf> {
    db::video_dir().map(|d| d.join("epg").join(format!("{source}.xml")))
}

fn is_remote(url: &str) -> bool {
    let l = url.trim().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

fn jstr(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

struct Xtream {
    server: String,
    user: String,
    pass: String,
    ext: String,
}

impl Xtream {
    fn of(row: &SourceRow) -> Result<Self, String> {
        let server = row.config["server"]
            .as_str()
            .unwrap_or(&row.url)
            .to_string();
        let user = row.config["username"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let pass = sources::secret_get(&format!("xtream:{}", row.id))
            .ok_or("Add this provider again: the password is missing")?;
        let ext = row.config["ext"].as_str().unwrap_or("m3u8").to_string();
        Ok(Self {
            server,
            user,
            pass,
            ext,
        })
    }

    fn api(&self, action: Option<&str>) -> String {
        let mut u = format!(
            "{}/player_api.php?username={}&password={}",
            self.server,
            urlencoding::encode(&self.user),
            urlencoding::encode(&self.pass)
        );
        if let Some(a) = action {
            u.push_str("&action=");
            u.push_str(a);
        }
        u
    }

    fn epg(&self) -> String {
        format!(
            "{}/xmltv.php?username={}&password={}",
            self.server,
            urlencoding::encode(&self.user),
            urlencoding::encode(&self.pass)
        )
    }

    fn stream(&self, id: &str) -> String {
        format!(
            "{}/live/{}/{}/{}.{}",
            self.server,
            urlencoding::encode(&self.user),
            urlencoding::encode(&self.pass),
            id,
            self.ext
        )
    }

    async fn get(&self, action: Option<&str>) -> Result<Value, String> {
        let resp = sources::http()
            .get(self.api(action))
            .header("User-Agent", XTREAM_UA)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(40))
            .send()
            .await
            .map_err(sources::net_err)?;
        if !resp.status().is_success() {
            return Err(sources::status_err(resp.status()));
        }
        resp.json()
            .await
            .map_err(|_| "Unexpected answer from the panel".to_string())
    }
}

async fn fetch_text(url: &str) -> Result<String, String> {
    let url = url.trim();
    if is_remote(url) {
        let resp = sources::http()
            .get(url)
            .timeout(PLAYLIST_LIMIT)
            .send()
            .await
            .map_err(sources::net_err)?;
        if !resp.status().is_success() {
            return Err(sources::status_err(resp.status()));
        }
        let bytes = resp.bytes().await.map_err(sources::net_err)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        let path = url.strip_prefix("file://").unwrap_or(url);
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| "Can't read that playlist file".to_string())?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

async fn m3u_channels(row: &SourceRow) -> Result<(Vec<NewChannel>, Vec<String>), String> {
    let text = fetch_text(&row.url).await?;
    let parsed = tokio::task::spawn_blocking(move || m3u::parse(&text))
        .await
        .map_err(|e| e.to_string())?;
    if parsed.entries.is_empty() {
        return Err("No channels in this playlist".into());
    }
    let channels = parsed
        .entries
        .into_iter()
        .filter(|e| !e.url.trim().is_empty())
        .map(|e| {
            let mut headers = Vec::new();
            if let Some(ua) = e.user_agent.filter(|v| !v.is_empty()) {
                headers.push(("User-Agent".to_string(), ua));
            }
            if let Some(r) = e.referrer.filter(|v| !v.is_empty()) {
                headers.push(("Referer".to_string(), r));
            }
            for (k, v) in e.headers {
                if !headers.iter().any(|(h, _)| h.eq_ignore_ascii_case(&k)) {
                    headers.push((k, v));
                }
            }
            NewChannel {
                name: if e.name.trim().is_empty() {
                    e.tvg_name.clone().unwrap_or_else(|| e.url.clone())
                } else {
                    e.name.trim().to_string()
                },
                group: e.group.unwrap_or_default().trim().to_string(),
                logo: e.tvg_logo.filter(|l| !l.is_empty()),
                tvg_id: e.tvg_id.filter(|t| !t.is_empty()),
                tvg_name: e.tvg_name.filter(|t| !t.is_empty()),
                url: e.url.trim().to_string(),
                headers,
            }
        })
        .collect();
    Ok((channels, parsed.epg_urls))
}

async fn xtream_channels(row: &SourceRow) -> Result<Vec<NewChannel>, String> {
    let x = Xtream::of(row)?;
    let cats = x
        .get(Some("get_live_categories"))
        .await
        .unwrap_or(Value::Null);
    let names: HashMap<String, String> = cats
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((jstr(&c["category_id"])?, jstr(&c["category_name"])?)))
        .collect();
    let streams = x.get(Some("get_live_streams")).await?;
    let list = streams.as_array().ok_or("The panel returned no channels")?;
    Ok(list
        .iter()
        .filter_map(|s| {
            let id = jstr(&s["stream_id"])?;
            Some(NewChannel {
                name: jstr(&s["name"]).unwrap_or_else(|| id.clone()),
                group: jstr(&s["category_id"])
                    .and_then(|c| names.get(&c).cloned())
                    .unwrap_or_default(),
                logo: jstr(&s["stream_icon"]),
                tvg_id: jstr(&s["epg_channel_id"]),
                tvg_name: None,
                url: format!("xtream:{id}"),
                headers: Vec::new(),
            })
        })
        .collect())
}

fn write_channels(source: i64, channels: Vec<NewChannel>) -> Result<usize, String> {
    let n = channels.len();
    sources::with_mut(|c| {
        let tx = c.transaction()?;
        tx.execute(
            "UPDATE iptv_channels SET position = -1 WHERE source = ?1",
            [source],
        )?;
        {
            let mut st = tx.prepare(
                "INSERT INTO iptv_channels(source, name, norm, grp, logo, tvg_id, url, headers_json, position)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(source, url) DO UPDATE SET name = excluded.name, norm = excluded.norm,
                   grp = excluded.grp, logo = excluded.logo, tvg_id = excluded.tvg_id,
                   headers_json = excluded.headers_json, position = excluded.position",
            )?;
            for (pos, ch) in channels.into_iter().enumerate() {
                let norm = norm_name(ch.tvg_name.as_deref().unwrap_or(&ch.name));
                st.execute(params![
                    source,
                    ch.name,
                    norm,
                    ch.group,
                    ch.logo,
                    ch.tvg_id,
                    ch.url,
                    serde_json::to_string(&ch.headers).unwrap_or_else(|_| "[]".into()),
                    pos as i64,
                ])?;
            }
        }
        tx.execute(
            "DELETE FROM iptv_channels WHERE source = ?1 AND position = -1",
            [source],
        )?;
        tx.commit()
    })?;
    Ok(n)
}

fn match_epg(source: i64) -> Result<(), String> {
    sources::with(|c| {
        c.execute(
            "UPDATE iptv_channels SET epg_id = COALESCE(
               (SELECT e.xml_id FROM epg_channels e WHERE e.source = iptv_channels.source
                  AND e.xml_id = iptv_channels.tvg_id LIMIT 1),
               (SELECT e.xml_id FROM epg_channels e WHERE e.source = iptv_channels.source
                  AND lower(e.xml_id) = lower(iptv_channels.tvg_id) LIMIT 1),
               (SELECT e.xml_id FROM epg_channels e WHERE e.source = iptv_channels.source
                  AND e.norm = iptv_channels.norm AND e.norm != '' LIMIT 1))
             WHERE source = ?1",
            [source],
        )
        .map(|_| ())
    })
}

async fn refresh_playlist(row: &SourceRow) -> Result<(), String> {
    let is_xtream = row.config["type"] == "xtream";
    let (channels, detected) = if is_xtream {
        (xtream_channels(row).await?, Vec::new())
    } else {
        m3u_channels(row).await?
    };
    let id = row.id;
    tokio::task::spawn_blocking(move || write_channels(id, channels))
        .await
        .map_err(|e| e.to_string())??;
    let mut cfg = sources::get(id)?.config;
    cfg["refreshed_at"] = json!(sources::now_ms());
    if let Some(first) = detected.into_iter().find(|u| is_remote(u)) {
        cfg["epg_detected"] = json!(first);
    }
    sources::set_config(id, &cfg)?;
    Ok(())
}

fn epg_url(row: &SourceRow) -> Option<String> {
    if row.config["type"] == "xtream" {
        return Xtream::of(row).ok().map(|x| x.epg());
    }
    row.config["epg_url"]
        .as_str()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| row.config["epg_detected"].as_str())
        .map(|u| u.trim().to_string())
}

async fn download(url: &str, dest: &PathBuf, xtream: bool) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    if !is_remote(url) {
        let path = url.strip_prefix("file://").unwrap_or(url);
        tokio::fs::copy(path, dest)
            .await
            .map_err(|_| "Can't read that guide file".to_string())?;
        return Ok(());
    }
    let mut req = sources::http().get(url).timeout(EPG_LIMIT);
    if xtream {
        req = req.header("User-Agent", XTREAM_UA);
    }
    let resp = req.send().await.map_err(sources::net_err)?;
    if !resp.status().is_success() {
        return Err(sources::status_err(resp.status()));
    }
    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(sources::net_err)?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    tokio::fs::rename(&tmp, dest)
        .await
        .map_err(|e| e.to_string())
}

fn import_epg(source: i64, path: PathBuf) -> Result<usize, String> {
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    sources::with(|c| c.execute("DELETE FROM epg_staging WHERE source = ?1", [source]))?;
    let now = sources::now_ms();
    let (from, to) = (now - EPG_PAST_MS, now + EPG_AHEAD_MS);
    let mut channels: Vec<(String, String)> = Vec::new();
    let mut batch: Vec<EpgRow> = Vec::new();
    let mut kept = 0usize;
    let mut failed: Option<String> = None;

    fn flush(source: i64, batch: &mut Vec<EpgRow>) -> Result<(), String> {
        if batch.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(batch);
        sources::with_mut(|c| {
            let tx = c.transaction()?;
            {
                let mut st = tx.prepare(
                    "INSERT INTO epg_staging(source, channel, start, stop, title, descr, category)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )?;
                for r in rows {
                    st.execute(params![source, r.0, r.1, r.2, r.3, r.4, r.5])?;
                }
            }
            tx.commit()
        })
    }

    let stats = xmltv::parse(BufReader::new(file), |ev| {
        if failed.is_some() {
            return;
        }
        match ev {
            xmltv::XmlEvent::Channel { id, names, .. } => {
                for n in names.iter() {
                    channels.push((id.clone(), norm_name(n)));
                }
                if names.is_empty() {
                    channels.push((id.clone(), norm_name(&id)));
                }
            }
            xmltv::XmlEvent::Programme {
                channel,
                start_ms,
                stop_ms,
                title,
                desc,
                category,
            } => {
                let stop = stop_ms.unwrap_or(start_ms + 3_600_000);
                if stop < from || start_ms > to || stop <= start_ms {
                    return;
                }
                batch.push((channel, start_ms, stop, title, desc, category));
                kept += 1;
                if batch.len() >= 5000 {
                    if let Err(e) = flush(source, &mut batch) {
                        failed = Some(e);
                    }
                }
            }
        }
    });
    if let Some(e) = failed {
        return Err(e);
    }
    flush(source, &mut batch)?;
    if stats.channels == 0 && stats.programmes == 0 {
        let _ = sources::with(|c| c.execute("DELETE FROM epg_staging WHERE source = ?1", [source]));
        return Err(stats.error.unwrap_or_else(|| "The guide is empty".into()));
    }
    sources::with_mut(|c| {
        let tx = c.transaction()?;
        tx.execute("DELETE FROM epg_channels WHERE source = ?1", [source])?;
        {
            let mut st =
                tx.prepare("INSERT INTO epg_channels(source, xml_id, norm) VALUES(?1, ?2, ?3)")?;
            for (id, n) in &channels {
                st.execute(params![source, id, n])?;
            }
        }
        tx.execute("DELETE FROM epg_programmes WHERE source = ?1", [source])?;
        tx.execute(
            "INSERT INTO epg_programmes(source, channel, start, stop, title, descr, category)
             SELECT source, channel, start, stop, title, descr, category FROM epg_staging WHERE source = ?1",
            [source],
        )?;
        tx.execute("DELETE FROM epg_staging WHERE source = ?1", [source])?;
        tx.commit()
    })?;
    match_epg(source)?;
    Ok(kept)
}

async fn refresh_epg(row: &SourceRow, force: bool) -> Result<(), String> {
    let Some(url) = epg_url(row) else {
        return Ok(());
    };
    let path = epg_path(row.id).ok_or("No data folder")?;
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| (age.as_millis() as i64) < EPG_TTL_MS);
    let has_rows: bool = sources::with(|c| {
        c.query_row(
            "SELECT EXISTS(SELECT 1 FROM epg_programmes WHERE source = ?1)",
            [row.id],
            |r| r.get(0),
        )
    })?;
    if fresh && has_rows && !force {
        return match_epg(row.id);
    }
    if !fresh || !path.exists() {
        download(&url, &path, row.config["type"] == "xtream").await?;
    }
    let id = row.id;
    tokio::task::spawn_blocking(move || import_epg(id, path))
        .await
        .map_err(|e| e.to_string())??;
    let mut cfg = sources::get(id)?.config;
    cfg["epg_refreshed_at"] = json!(sources::now_ms());
    sources::set_config(id, &cfg)?;
    Ok(())
}

fn refresh_stale_guides(source: Option<i64>) {
    let Ok(rows) = sources::list(Some("iptv")) else {
        return;
    };
    let now = sources::now_ms();
    for row in rows {
        if source.is_some_and(|s| s != row.id) || !row.enabled || epg_url(&row).is_none() {
            continue;
        }
        let at = row.config["epg_refreshed_at"].as_i64().unwrap_or(0);
        if now - at < EPG_TTL_MS {
            continue;
        }
        static RUNNING: std::sync::Mutex<Vec<i64>> = std::sync::Mutex::new(Vec::new());
        {
            let Ok(mut r) = RUNNING.lock() else { continue };
            if r.contains(&row.id) {
                continue;
            }
            r.push(row.id);
        }
        tokio::spawn(async move {
            if let Err(e) = refresh_epg(&row, false).await {
                tracing::warn!("[video] guide refresh for {} failed: {e}", row.id);
            }
            if let Ok(mut r) = RUNNING.lock() {
                r.retain(|x| *x != row.id);
            }
        });
    }
}

async fn refresh(id: i64, force_epg: bool) -> Result<(), String> {
    let row = sources::get_kind(id, "iptv")?;
    sources::tracked(id, PLAYLIST_LIMIT, refresh_playlist(&row)).await?;
    let row = sources::get(id)?;
    if let Err(e) = tokio::time::timeout(EPG_LIMIT * 2, refresh_epg(&row, force_epg))
        .await
        .unwrap_or_else(|_| Err("Timed out".into()))
    {
        tracing::warn!("[video] guide for source {id}: {e}");
    }
    Ok(())
}

#[tauri::command]
pub async fn video_iptv_add(
    name: String,
    url: String,
    epg_url: Option<String>,
) -> Result<SourceInfo, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("Enter a playlist address or file".into());
    }
    if !is_remote(&url) {
        let path = url.strip_prefix("file://").unwrap_or(&url);
        if !std::path::Path::new(path).is_file() {
            return Err("Can't read that playlist file".into());
        }
    }
    let label = if name.trim().is_empty() {
        url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .or_else(|| {
                std::path::Path::new(&url)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "IPTV".into())
    } else {
        name.trim().to_string()
    };
    let config = json!({
        "key": url,
        "type": "m3u",
        "epg_url": epg_url.map(|e| e.trim().to_string()).filter(|e| !e.is_empty()),
    });
    let id = sources::upsert("iptv", &label, &url, &config)?;
    if let Err(e) = refresh(id, true).await {
        let has_channels: bool = sources::with(|c| {
            c.query_row(
                "SELECT EXISTS(SELECT 1 FROM iptv_channels WHERE source = ?1)",
                [id],
                |r| r.get(0),
            )
        })
        .unwrap_or(false);
        if !has_channels {
            let _ = sources::video_source_remove(id).await;
            return Err(e);
        }
    }
    sources::info_by_id(id)
}

#[tauri::command]
pub async fn video_iptv_add_xtream(
    name: String,
    server: String,
    username: String,
    password: String,
) -> Result<SourceInfo, String> {
    let base = sources::normalize_base(&server)?;
    let base = base.trim_end_matches("/player_api.php").to_string();
    let user = username.trim().to_string();
    if user.is_empty() || password.is_empty() {
        return Err("Enter the user name and password".into());
    }
    let probe = Xtream {
        server: base.clone(),
        user: user.clone(),
        pass: password.clone(),
        ext: "m3u8".into(),
    };
    let login = probe.get(None).await?;
    let info = &login["user_info"];
    if info["auth"]
        .as_i64()
        .or_else(|| jstr(&info["auth"]).and_then(|s| s.parse().ok()))
        != Some(1)
    {
        return Err("Wrong user name or password".into());
    }
    if let Some(status) = jstr(&info["status"]) {
        if !status.eq_ignore_ascii_case("active") {
            return Err(format!("Account status: {status}"));
        }
    }
    let formats: Vec<String> = info["allowed_output_formats"]
        .as_array()
        .map(|a| a.iter().filter_map(jstr).collect())
        .unwrap_or_default();
    let ext = if formats.is_empty() || formats.iter().any(|f| f == "m3u8") {
        "m3u8"
    } else {
        "ts"
    };
    let expires = jstr(&info["exp_date"])
        .and_then(|s| s.parse::<i64>().ok())
        .map(|s| s * 1000);
    let label = if name.trim().is_empty() {
        url::Url::parse(&base)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "Xtream".into())
    } else {
        name.trim().to_string()
    };
    let config = json!({
        "key": format!("{base}#{user}"),
        "type": "xtream",
        "server": base,
        "username": user,
        "ext": ext,
        "expires_at": expires,
    });
    let id = sources::upsert("iptv", &label, &base, &config)?;
    sources::secret_set(&format!("xtream:{id}"), &password)?;
    if let Err(e) = refresh(id, true).await {
        let _ = sources::video_source_remove(id).await;
        return Err(e);
    }
    sources::info_by_id(id)
}

#[tauri::command]
pub async fn video_iptv_refresh(source: i64) -> Result<(), String> {
    refresh(source, true).await
}

fn programme(r: &rusqlite::Row, channel: i64) -> rusqlite::Result<EpgProgramme> {
    Ok(EpgProgramme {
        channel,
        start: r.get(0)?,
        stop: r.get(1)?,
        title: r.get(2)?,
        desc: r.get(3)?,
        category: r.get(4)?,
    })
}

#[tauri::command]
pub async fn video_iptv_groups(source: Option<i64>) -> Result<Vec<IptvGroup>, String> {
    sources::with(|c| {
        let mut st = c.prepare(
            "SELECT ch.grp, COUNT(*) FROM iptv_channels ch
             JOIN sources s ON s.id = ch.source AND s.enabled = 1
             WHERE ?1 IS NULL OR ch.source = ?1
             GROUP BY ch.grp ORDER BY MIN(s.position), MIN(ch.position)",
        )?;
        let rows = st
            .query_map(params![source], |r| {
                Ok(IptvGroup {
                    name: r.get(0)?,
                    count: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

#[tauri::command]
pub async fn video_iptv_channels(
    source: Option<i64>,
    group: Option<String>,
    search: Option<String>,
    favorites: Option<bool>,
) -> Result<Vec<IptvChannel>, String> {
    refresh_stale_guides(source);
    let search = search
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let norm_search = search.as_deref().map(norm_name);
    let favorites = favorites.unwrap_or(false);
    tokio::task::spawn_blocking(move || {
        let now = sources::now_ms();
        sources::with(|c| {
            let mut st = c.prepare(
                "SELECT ch.id, ch.source, ch.name, ch.grp, ch.logo, ch.tvg_id, ch.favorite, ch.epg_id
                 FROM iptv_channels ch JOIN sources s ON s.id = ch.source AND s.enabled = 1
                 WHERE (?1 IS NULL OR ch.source = ?1)
                   AND (?2 IS NULL OR ch.grp = ?2)
                   AND (?3 IS NULL OR lower(ch.name) LIKE '%' || ?3 || '%' OR (?4 != '' AND ch.norm LIKE '%' || ?4 || '%'))
                   AND (?5 = 0 OR ch.favorite = 1)
                 ORDER BY s.position, ch.position LIMIT ?6",
            )?;
            let rows: Vec<(IptvChannel, Option<String>)> = st
                .query_map(
                    params![
                        source,
                        group,
                        search,
                        norm_search.clone().unwrap_or_default(),
                        favorites as i64,
                        MAX_CHANNELS_LISTED
                    ],
                    |r| {
                        Ok((
                            IptvChannel {
                                id: r.get(0)?,
                                source: r.get(1)?,
                                name: r.get(2)?,
                                group: r.get(3)?,
                                logo: r.get(4)?,
                                tvg_id: r.get(5)?,
                                favorite: r.get::<_, i64>(6)? != 0,
                                now: None,
                                next: None,
                            },
                            r.get(7)?,
                        ))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut now_st = c.prepare_cached(
                "SELECT start, stop, title, descr, category FROM epg_programmes
                 WHERE source = ?1 AND channel = ?2 AND start <= ?3 AND stop > ?3
                 ORDER BY start DESC LIMIT 1",
            )?;
            let mut next_st = c.prepare_cached(
                "SELECT start, stop, title, descr, category FROM epg_programmes
                 WHERE source = ?1 AND channel = ?2 AND start > ?3 ORDER BY start LIMIT 1",
            )?;
            let mut out = Vec::with_capacity(rows.len());
            for (mut ch, epg) in rows {
                if let Some(e) = epg {
                    ch.now = now_st
                        .query_row(params![ch.source, e, now], |r| programme(r, ch.id))
                        .optional()?;
                    ch.next = next_st
                        .query_row(params![ch.source, e, now], |r| programme(r, ch.id))
                        .optional()?;
                }
                out.push(ch);
            }
            Ok(out)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn video_iptv_epg(
    channel_ids: Vec<i64>,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<EpgProgramme>, String> {
    tokio::task::spawn_blocking(move || {
        sources::with(|c| {
            let mut ch_st = c.prepare_cached(
                "SELECT source, epg_id FROM iptv_channels WHERE id = ?1 AND epg_id IS NOT NULL",
            )?;
            let mut st = c.prepare_cached(
                "SELECT start, stop, title, descr, category FROM epg_programmes
                 WHERE source = ?1 AND channel = ?2 AND stop > ?3 AND start < ?4 ORDER BY start",
            )?;
            let mut out = Vec::new();
            for id in channel_ids.iter().take(500) {
                let Some((source, epg)) = ch_st
                    .query_row([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                    .optional()?
                else {
                    continue;
                };
                let rows = st
                    .query_map(params![source, epg, from_ms, to_ms], |r| programme(r, *id))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                out.extend(rows);
            }
            Ok(out)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn video_iptv_favorite(channel_id: i64, fav: bool) -> Result<(), String> {
    sources::with(|c| {
        c.execute(
            "UPDATE iptv_channels SET favorite = ?2 WHERE id = ?1",
            params![channel_id, fav as i64],
        )
        .map(|_| ())
    })
}

pub async fn play(app: &AppHandle, channel: i64) -> Result<PlaybackInfo, String> {
    let row: Option<ChannelRow> = sources::with(|c| {
        c.query_row(
                "SELECT source, name, grp, logo, url, headers_json, epg_id FROM iptv_channels WHERE id = ?1",
                [channel],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .optional()
    })?;
    let (source, name, group, logo, url, headers_json, epg) = row.ok_or("Channel not found")?;
    let src = sources::get_kind(source, "iptv")?;
    if !src.enabled {
        return Err("This source is turned off".into());
    }
    let mut headers: Vec<(String, String)> =
        serde_json::from_str(&headers_json).unwrap_or_default();
    let stream_url = match url.strip_prefix("xtream:") {
        Some(id) => {
            let x = Xtream::of(&src)?;
            if !headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
            {
                headers.push(("User-Agent".into(), XTREAM_UA.into()));
            }
            x.stream(id)
        }
        None => url,
    };
    if !is_remote(&stream_url) {
        return Err("This channel's address is not http".into());
    }
    server::ensure(app).await?;
    let proxied = server::register_remote(&stream_url, headers);
    let now = sources::now_ms();
    let now_title: Option<String> = match &epg {
        Some(e) => sources::with(|c| {
            c.query_row(
                "SELECT title FROM epg_programmes WHERE source = ?1 AND channel = ?2
                 AND start <= ?3 AND stop > ?3 ORDER BY start DESC LIMIT 1",
                params![source, e, now],
                |r| r.get(0),
            )
            .optional()
        })
        .ok()
        .flatten(),
        None => None,
    };
    let key = format!("iptv:{channel}");
    let source_ref = SourceRef::Iptv { channel };
    let subtitle = now_title.unwrap_or_else(|| {
        if group.is_empty() {
            src.name.clone()
        } else {
            group.clone()
        }
    });
    let _ = super::remember_item(&key, &name, &subtitle, logo.as_deref(), &source_ref);
    sources::kv_set("iptv_last_channel", &channel.to_string());

    let is_hls = stream_url.to_ascii_lowercase().contains(".m3u8")
        || stream_url.to_ascii_lowercase().contains("m3u8");
    let mut info = PlaybackInfo::remote(key, source_ref, proxied);
    info.title = name;
    info.subtitle = subtitle;
    info.is_live = true;
    info.prefs = super::merged_prefs(&[format!("media:{}", info.key)]);
    info.mode = if is_hls {
        PlayMode::Hls
    } else {
        PlayMode::Remote
    };
    info.reason = if is_hls {
        "Live channel (HLS) through the local proxy".into()
    } else {
        "Live channel through the local proxy".into()
    };
    info.info.container = if is_hls { "hls".into() } else { String::new() };
    Ok(info)
}

#[tauri::command]
pub async fn video_iptv_play(app: AppHandle, channel_id: i64) -> Result<PlaybackInfo, String> {
    play(&app, channel_id).await
}
