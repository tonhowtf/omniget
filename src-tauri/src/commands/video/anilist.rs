use std::time::Duration;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::sources;

const API: &str = "https://graphql.anilist.co";
const TOKEN_ACCOUNT: &str = "anilist";
const MEDIA_TTL_MS: i64 = 6 * 3600 * 1000;

const MEDIA_FIELDS: &str = "id idMal title { userPreferred romaji english native } \
    coverImage { extraLarge large } bannerImage episodes status seasonYear format genres \
    description(asHtml: false) nextAiringEpisode { episode airingAt } \
    mediaListEntry { id status progress score }";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AniAiring {
    pub episode: i64,
    pub airing_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AniListEntry {
    pub status: String,
    pub progress: i64,
    pub score: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AniMedia {
    pub id: i64,
    pub id_mal: Option<i64>,
    pub title: String,
    pub title_native: Option<String>,
    pub cover: Option<String>,
    pub banner: Option<String>,
    pub episodes: Option<i64>,
    pub status: Option<String>,
    pub season_year: Option<i64>,
    pub format: Option<String>,
    pub genres: Vec<String>,
    pub description: Option<String>,
    pub next_airing: Option<AniAiring>,
    pub list: Option<AniListEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AniViewer {
    pub id: i64,
    pub name: String,
    pub avatar: Option<String>,
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

fn map_media(v: &Value, entry_override: Option<&Value>) -> AniMedia {
    let t = &v["title"];
    let title = ["userPreferred", "english", "romaji", "native"]
        .iter()
        .find_map(|k| t[*k].as_str().filter(|s| !s.is_empty()))
        .unwrap_or_default()
        .to_string();
    let entry = entry_override.unwrap_or(&v["mediaListEntry"]);
    AniMedia {
        id: v["id"].as_i64().unwrap_or(0),
        id_mal: v["idMal"].as_i64(),
        title,
        title_native: t["native"].as_str().map(str::to_string),
        cover: v["coverImage"]["extraLarge"]
            .as_str()
            .or_else(|| v["coverImage"]["large"].as_str())
            .map(str::to_string),
        banner: v["bannerImage"].as_str().map(str::to_string),
        episodes: v["episodes"].as_i64(),
        status: v["status"].as_str().map(str::to_string),
        season_year: v["seasonYear"].as_i64(),
        format: v["format"].as_str().map(str::to_string),
        genres: v["genres"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|g| g.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        description: v["description"]
            .as_str()
            .map(strip_html)
            .filter(|d| !d.is_empty()),
        next_airing: v["nextAiringEpisode"]["episode"]
            .as_i64()
            .map(|ep| AniAiring {
                episode: ep,
                airing_at: v["nextAiringEpisode"]["airingAt"].as_i64().unwrap_or(0) * 1000,
            }),
        list: entry["status"].as_str().map(|st| AniListEntry {
            status: st.to_string(),
            progress: entry["progress"].as_i64().unwrap_or(0),
            score: entry["score"].as_f64().filter(|s| *s > 0.0),
        }),
    }
}

enum GqlError {
    Transient(String),
    Permanent(String),
}

impl GqlError {
    fn text(self) -> String {
        match self {
            GqlError::Transient(s) | GqlError::Permanent(s) => s,
        }
    }
}

async fn gql_raw(query: &str, variables: Value, auth: bool) -> Result<Value, GqlError> {
    let mut req = sources::http()
        .post(API)
        .timeout(Duration::from_secs(15))
        .header("Accept", "application/json")
        .json(&json!({ "query": query, "variables": variables }));
    if auth {
        let token = sources::secret_get(TOKEN_ACCOUNT)
            .ok_or_else(|| GqlError::Permanent("Connect AniList first".into()))?;
        req = req.bearer_auth(token);
    } else if let Some(token) = sources::secret_get(TOKEN_ACCOUNT) {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| GqlError::Transient(sources::net_err(e)))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| GqlError::Transient(sources::net_err(e)))?;
    if let Some(err) = body["errors"].as_array().and_then(|a| a.first()) {
        let msg = err["message"]
            .as_str()
            .unwrap_or("AniList error")
            .to_string();
        let code = err["status"].as_u64().unwrap_or(status.as_u16() as u64);
        let msg = if code == 401 || msg.contains("Invalid token") {
            "AniList token not accepted".to_string()
        } else {
            msg
        };
        return Err(if code == 429 || code >= 500 {
            GqlError::Transient(msg)
        } else {
            GqlError::Permanent(msg)
        });
    }
    if !status.is_success() {
        let msg = sources::status_err(status);
        return Err(if status.as_u16() == 429 || status.is_server_error() {
            GqlError::Transient(msg)
        } else {
            GqlError::Permanent(msg)
        });
    }
    Ok(body["data"].clone())
}

async fn gql(query: &str, variables: Value, auth: bool) -> Result<Value, String> {
    gql_raw(query, variables, auth)
        .await
        .map_err(GqlError::text)
}

async fn fetch_viewer() -> Result<AniViewer, String> {
    let d = gql(
        "query { Viewer { id name avatar { medium large } } }",
        json!({}),
        true,
    )
    .await?;
    let v = &d["Viewer"];
    let viewer = AniViewer {
        id: v["id"].as_i64().ok_or("AniList token not accepted")?,
        name: v["name"].as_str().unwrap_or_default().to_string(),
        avatar: v["avatar"]["large"]
            .as_str()
            .or_else(|| v["avatar"]["medium"].as_str())
            .map(str::to_string),
    };
    if let Ok(s) = serde_json::to_string(&viewer) {
        sources::kv_set("anilist_viewer", &s);
    }
    Ok(viewer)
}

fn cached_viewer() -> Option<AniViewer> {
    sources::kv_get("anilist_viewer").and_then(|s| serde_json::from_str(&s).ok())
}

#[tauri::command]
pub async fn video_anilist_set_token(token: String) -> Result<(), String> {
    let raw = token.trim();
    let token = raw
        .split_once("access_token=")
        .map(|(_, rest)| rest.split('&').next().unwrap_or(rest))
        .unwrap_or(raw)
        .trim()
        .to_string();
    if token.is_empty() {
        sources::secret_del(TOKEN_ACCOUNT);
        sources::kv_del("anilist_viewer");
        return Ok(());
    }
    let previous = sources::secret_get(TOKEN_ACCOUNT);
    sources::secret_set(TOKEN_ACCOUNT, &token)?;
    if let Err(e) = fetch_viewer().await {
        match previous {
            Some(p) => sources::secret_set(TOKEN_ACCOUNT, &p)?,
            None => sources::secret_del(TOKEN_ACCOUNT),
        }
        return Err(e);
    }
    let _ = flush_outbox().await;
    Ok(())
}

#[tauri::command]
pub async fn video_anilist_viewer() -> Result<Option<AniViewer>, String> {
    if sources::secret_get(TOKEN_ACCOUNT).is_none() {
        return Ok(None);
    }
    match fetch_viewer().await {
        Ok(v) => {
            let _ = flush_outbox().await;
            Ok(Some(v))
        }
        Err(e) if e == "AniList token not accepted" => Err(e),
        Err(_) => Ok(cached_viewer()),
    }
}

fn cache_media(v: &Value) {
    if let Some(id) = v["id"].as_i64() {
        sources::cache_put(&format!("ani:media:{id}"), &v.to_string());
    }
}

#[tauri::command]
pub async fn video_anilist_search(q: String) -> Result<Vec<AniMedia>, String> {
    let q = q.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let query = format!(
        "query ($q: String) {{ Page(perPage: 30) {{ media(search: $q, type: ANIME, sort: SEARCH_MATCH, isAdult: false) {{ {MEDIA_FIELDS} }} }} }}"
    );
    let d = gql(&query, json!({ "q": q }), false).await?;
    let arr = d["Page"]["media"].as_array().cloned().unwrap_or_default();
    arr.iter().for_each(cache_media);
    Ok(arr.iter().map(|m| map_media(m, None)).collect())
}

async fn media_value(id: i64) -> Result<Value, String> {
    let query = format!("query ($id: Int) {{ Media(id: $id, type: ANIME) {{ {MEDIA_FIELDS} }} }}");
    match gql(&query, json!({ "id": id }), false).await {
        Ok(d) => {
            let m = d["Media"].clone();
            if m.is_null() {
                return Err("Not found on AniList".into());
            }
            cache_media(&m);
            Ok(m)
        }
        Err(e) => sources::cache_get(&format!("ani:media:{id}"), -1)
            .and_then(|s| serde_json::from_str(&s).ok())
            .ok_or(e),
    }
}

#[tauri::command]
pub async fn video_anilist_media(id: i64) -> Result<AniMedia, String> {
    let _ = flush_outbox().await;
    let v = match sources::cache_get(&format!("ani:media:{id}"), MEDIA_TTL_MS)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(|_| sources::secret_get(TOKEN_ACCOUNT).is_none())
    {
        Some(v) => v,
        None => media_value(id).await?,
    };
    Ok(with_pending(map_media(&v, None)))
}

#[tauri::command]
pub async fn video_anilist_list(status: Option<String>) -> Result<Vec<AniMedia>, String> {
    let viewer = match cached_viewer() {
        Some(v) => v,
        None => fetch_viewer().await?,
    };
    let _ = flush_outbox().await;
    let status = status.filter(|s| !s.is_empty()).map(|s| s.to_uppercase());
    let query = format!(
        "query ($user: Int, $status: MediaListStatus) {{ MediaListCollection(userId: $user, type: ANIME, status: $status, sort: UPDATED_TIME_DESC) {{ lists {{ entries {{ status progress score media {{ {MEDIA_FIELDS} }} }} }} }} }}"
    );
    let d = gql(&query, json!({ "user": viewer.id, "status": status }), true).await?;
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for list in d["MediaListCollection"]["lists"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for e in list["entries"].as_array().into_iter().flatten() {
            let m = &e["media"];
            let id = m["id"].as_i64().unwrap_or(0);
            if !seen.insert(id) {
                continue;
            }
            cache_media(m);
            out.push(with_pending(map_media(m, Some(e))));
        }
    }
    Ok(out)
}

#[derive(Serialize, Deserialize)]
struct ProgressJob {
    media_id: i64,
    episode: i64,
    episodes: Option<i64>,
}

fn with_pending(mut m: AniMedia) -> AniMedia {
    let pending: Option<String> = sources::with(|c| {
        use rusqlite::OptionalExtension;
        c.query_row(
            "SELECT payload FROM sync_outbox WHERE dedupe = ?1",
            [format!("anilist:progress:{}", m.id)],
            |r| r.get(0),
        )
        .optional()
    })
    .ok()
    .flatten();
    if let Some(job) = pending.and_then(|p| serde_json::from_str::<ProgressJob>(&p).ok()) {
        let done = m
            .episodes
            .or(job.episodes)
            .is_some_and(|n| job.episode >= n);
        let entry = m.list.get_or_insert(AniListEntry {
            status: "CURRENT".into(),
            progress: 0,
            score: None,
        });
        if job.episode > entry.progress {
            entry.progress = job.episode;
            if done {
                entry.status = "COMPLETED".into();
            } else if entry.status != "REPEATING" {
                entry.status = "CURRENT".into();
            }
        }
    }
    m
}

fn backoff_ms(attempts: i64) -> i64 {
    let mins = 1i64 << attempts.clamp(0, 9);
    (mins * 60_000).min(6 * 3600 * 1000)
}

async fn send_progress(job: &ProgressJob) -> Result<(), GqlError> {
    let d = gql_raw(
        "query ($id: Int) { Media(id: $id) { episodes mediaListEntry { status progress } } }",
        json!({ "id": job.media_id }),
        true,
    )
    .await?;
    let media = &d["Media"];
    let current = media["mediaListEntry"]["progress"].as_i64().unwrap_or(0);
    let current_status = media["mediaListEntry"]["status"].as_str().unwrap_or("");
    if current >= job.episode && !current_status.is_empty() {
        return Ok(());
    }
    let total = media["episodes"].as_i64().or(job.episodes);
    let status = if total.is_some_and(|n| job.episode >= n) {
        "COMPLETED"
    } else if current_status == "REPEATING" {
        "REPEATING"
    } else {
        "CURRENT"
    };
    gql_raw(
        "mutation ($id: Int, $progress: Int, $status: MediaListStatus) { SaveMediaListEntry(mediaId: $id, progress: $progress, status: $status) { id status progress } }",
        json!({ "id": job.media_id, "progress": job.episode, "status": status }),
        true,
    )
    .await?;
    Ok(())
}

async fn flush_outbox() -> Result<usize, String> {
    if sources::secret_get(TOKEN_ACCOUNT).is_none() {
        return Ok(0);
    }
    let now = sources::now_ms();
    let due: Vec<(i64, String, i64)> = sources::with(|c| {
        let mut st = c.prepare(
            "SELECT id, payload, attempts FROM sync_outbox
             WHERE kind = 'anilist_progress' AND next_at <= ?1 ORDER BY id LIMIT 20",
        )?;
        let rows = st
            .query_map([now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })?;
    for (id, payload, attempts) in due {
        let Ok(job) = serde_json::from_str::<ProgressJob>(&payload) else {
            let _ = sources::with(|c| c.execute("DELETE FROM sync_outbox WHERE id = ?1", [id]));
            continue;
        };
        match send_progress(&job).await {
            Ok(()) => {
                let _ = sources::with(|c| c.execute("DELETE FROM sync_outbox WHERE id = ?1", [id]));
            }
            Err(GqlError::Permanent(e)) => {
                tracing::warn!("[video] AniList dropped progress for {}: {e}", job.media_id);
                let _ = sources::with(|c| c.execute("DELETE FROM sync_outbox WHERE id = ?1", [id]));
            }
            Err(GqlError::Transient(e)) => {
                let _ = sources::with(|c| {
                    c.execute(
                        "UPDATE sync_outbox SET attempts = attempts + 1, next_at = ?2, last_error = ?3 WHERE id = ?1",
                        params![id, sources::now_ms() + backoff_ms(attempts + 1), e],
                    )
                });
                break;
            }
        }
    }
    sources::with(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM sync_outbox WHERE kind = 'anilist_progress'",
            [],
            |r| r.get::<_, i64>(0),
        )
    })
    .map(|n| n as usize)
}

#[tauri::command]
pub async fn video_anilist_progress(media_id: i64, episode: i64) -> Result<AniMedia, String> {
    if sources::secret_get(TOKEN_ACCOUNT).is_none() {
        return Err("Connect AniList first".into());
    }
    if episode < 0 {
        return Err("Invalid episode".into());
    }
    let known = sources::cache_get(&format!("ani:media:{media_id}"), -1)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let job = ProgressJob {
        media_id,
        episode,
        episodes: known.as_ref().and_then(|m| m["episodes"].as_i64()),
    };
    let payload = serde_json::to_string(&job).map_err(|e| e.to_string())?;
    let now = sources::now_ms();
    sources::with(|c| {
        c.execute(
            "INSERT INTO sync_outbox(kind, dedupe, payload, attempts, next_at, created_at)
             VALUES('anilist_progress', ?1, ?2, 0, ?3, ?3)
             ON CONFLICT(dedupe) DO UPDATE SET payload = excluded.payload, attempts = 0,
               next_at = excluded.next_at, last_error = NULL
             WHERE json_extract(sync_outbox.payload, '$.episode') <= ?4",
            params![
                format!("anilist:progress:{media_id}"),
                payload,
                now,
                episode
            ],
        )
    })?;
    let _ = flush_outbox().await;
    let v = match media_value(media_id).await {
        Ok(v) => v,
        Err(e) => known.ok_or(e)?,
    };
    Ok(with_pending(map_media(&v, None)))
}
