use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use omniget_core::core::music::playlist_file;

const UA: &str = concat!(
    "OmniGet/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/tonhowtf/omniget )"
);

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(UA)
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LrclibRecord {
    pub id: i64,
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub album_name: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

pub async fn lrclib_find(
    artist: &str,
    title: &str,
    album: &str,
    duration_s: f64,
) -> Result<Vec<LrclibRecord>, String> {
    let mut out = Vec::new();
    if !artist.is_empty() && !title.is_empty() {
        let mut q = vec![
            ("artist_name", artist.to_string()),
            ("track_name", title.to_string()),
        ];
        if !album.is_empty() {
            q.push(("album_name", album.to_string()));
        }
        if duration_s > 0.0 {
            q.push(("duration", format!("{}", duration_s.round() as i64)));
        }
        let mut attempts = vec![q.clone()];
        if !album.is_empty() {
            attempts.push(q.into_iter().filter(|(k, _)| *k != "album_name").collect());
        }
        for q in attempts {
            let Ok(r) = client()
                .get("https://lrclib.net/api/get")
                .header("Lrclib-Client", UA)
                .query(&q)
                .send()
                .await
            else {
                continue;
            };
            if r.status().is_success() {
                if let Ok(rec) = r.json::<LrclibRecord>().await {
                    out.push(rec);
                    break;
                }
            }
        }
    }
    let mut q = vec![("track_name", title.to_string())];
    if !artist.is_empty() {
        q.push(("artist_name", artist.to_string()));
    }
    let resp = client()
        .get("https://lrclib.net/api/search")
        .header("Lrclib-Client", UA)
        .query(&q)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        let mut found: Vec<LrclibRecord> = resp.json().await.map_err(|e| e.to_string())?;
        found.retain(|f| !out.iter().any(|o| o.id == f.id));
        found.sort_by(|a, b| {
            let score = |r: &LrclibRecord| {
                let synced = if r
                    .synced_lyrics
                    .as_deref()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
                {
                    0.0
                } else {
                    100.0
                };
                let dist = match (r.duration, duration_s > 0.0) {
                    (Some(d), true) => (d - duration_s).abs(),
                    _ => 50.0,
                };
                synced + dist
            };
            score(a)
                .partial_cmp(&score(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.extend(found.into_iter().take(15));
    }
    Ok(out)
}

static MB_LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

async fn mb_throttle() {
    let lock = MB_LAST.get_or_init(|| Mutex::new(None));
    let mut last = lock.lock().await;
    if let Some(t) = *last {
        let wait = Duration::from_millis(1100).saturating_sub(t.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
    *last = Some(Instant::now());
}

#[derive(Debug, Clone, Serialize)]
pub struct MbCandidate {
    pub recording_id: String,
    pub release_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub length_ms: Option<u64>,
    pub score: u32,
    pub release_type: String,
    pub cover_url: String,
}

fn lucene_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "+-&|!(){}[]^\"~*?:\\/".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn credit(v: &serde_json::Value) -> String {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    format!(
                        "{}{}",
                        c["name"].as_str().unwrap_or_default(),
                        c["joinphrase"].as_str().unwrap_or_default()
                    )
                })
                .collect::<String>()
        })
        .unwrap_or_default()
}

pub async fn musicbrainz_search(
    artist: &str,
    title: &str,
    album: &str,
) -> Result<Vec<MbCandidate>, String> {
    let mut q = format!("recording:\"{}\"", lucene_escape(title));
    if !artist.is_empty() {
        q.push_str(&format!(" AND artist:\"{}\"", lucene_escape(artist)));
    }
    if !album.is_empty() {
        q.push_str(&format!(" AND release:\"{}\"", lucene_escape(album)));
    }
    mb_throttle().await;
    let v: serde_json::Value = client()
        .get("https://musicbrainz.org/ws/2/recording")
        .query(&[("query", q.as_str()), ("fmt", "json"), ("limit", "15")])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for rec in v["recordings"].as_array().cloned().unwrap_or_default() {
        let artist = credit(&rec["artist-credit"]);
        let releases = rec["releases"].as_array().cloned().unwrap_or_default();
        let mut rels = releases.clone();
        rels.sort_by_key(|r| {
            let official = r["status"].as_str() == Some("Official");
            let album = r["release-group"]["primary-type"].as_str() == Some("Album");
            let comp = r["release-group"]["secondary-types"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            (!official as u8, !album as u8, comp as u8)
        });
        for rel in rels.iter().take(3) {
            let media = &rel["media"][0];
            let rel_id = rel["id"].as_str().unwrap_or_default().to_string();
            out.push(MbCandidate {
                recording_id: rec["id"].as_str().unwrap_or_default().to_string(),
                release_id: rel_id.clone(),
                title: rec["title"].as_str().unwrap_or_default().to_string(),
                artist: artist.clone(),
                album: rel["title"].as_str().unwrap_or_default().to_string(),
                album_artist: {
                    let a = credit(&rel["artist-credit"]);
                    if a.is_empty() {
                        artist.clone()
                    } else {
                        a
                    }
                },
                year: rel["date"]
                    .as_str()
                    .and_then(|d| d.get(..4))
                    .and_then(|y| y.parse().ok()),
                track: media["track"][0]["number"]
                    .as_str()
                    .and_then(|n| n.parse().ok()),
                track_total: media["track-count"].as_u64().map(|n| n as u32),
                length_ms: rec["length"].as_u64(),
                score: rec["score"].as_u64().unwrap_or(0) as u32,
                release_type: rel["release-group"]["primary-type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                cover_url: if rel_id.is_empty() {
                    String::new()
                } else {
                    format!("https://coverartarchive.org/release/{rel_id}/front-500")
                },
            });
        }
        if releases.is_empty() {
            out.push(MbCandidate {
                recording_id: rec["id"].as_str().unwrap_or_default().to_string(),
                release_id: String::new(),
                title: rec["title"].as_str().unwrap_or_default().to_string(),
                artist: artist.clone(),
                album: String::new(),
                album_artist: String::new(),
                year: None,
                track: None,
                track_total: None,
                length_ms: rec["length"].as_u64(),
                score: rec["score"].as_u64().unwrap_or(0) as u32,
                release_type: String::new(),
                cover_url: String::new(),
            });
        }
    }
    Ok(out)
}

pub async fn fetch_image(url: &str) -> Result<Vec<u8>, String> {
    let u = url::Url::parse(url).map_err(|e| e.to_string())?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("unsupported url".into());
    }
    let r = client()
        .get(u)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let bytes = r.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("image too large".into());
    }
    Ok(bytes.to_vec())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RbStation {
    #[serde(default)]
    pub stationuuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub url_resolved: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub favicon: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub countrycode: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub bitrate: i64,
    #[serde(default)]
    pub hls: i64,
    #[serde(default)]
    pub votes: i64,
    #[serde(default)]
    pub clickcount: i64,
    #[serde(default)]
    pub lastcheckok: i64,
}

const RB_FALLBACK: &[&str] = &[
    "https://de1.api.radio-browser.info",
    "https://de2.api.radio-browser.info",
    "https://fi1.api.radio-browser.info",
    "https://nl1.api.radio-browser.info",
];

static RB_BASE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

async fn rb_base() -> String {
    let lock = RB_BASE.get_or_init(|| Mutex::new(None));
    let mut g = lock.lock().await;
    if let Some(b) = g.as_ref() {
        return b.clone();
    }
    let mut picked = None;
    if let Ok(r) = client()
        .get("https://all.api.radio-browser.info/json/servers")
        .timeout(Duration::from_secs(6))
        .send()
        .await
    {
        if let Ok(list) = r.json::<Vec<serde_json::Value>>().await {
            let names: Vec<String> = list
                .iter()
                .filter_map(|s| s["name"].as_str().map(|n| format!("https://{n}")))
                .collect();
            if !names.is_empty() {
                let i = rand::random_range(0..names.len());
                picked = Some(names[i].clone());
            }
        }
    }
    let base = picked.unwrap_or_else(|| RB_FALLBACK[0].to_string());
    *g = Some(base.clone());
    base
}

async fn rb_get(path: &str, query: &[(&str, String)]) -> Result<serde_json::Value, String> {
    let mut bases = vec![rb_base().await];
    bases.extend(RB_FALLBACK.iter().map(|s| s.to_string()));
    let mut last_err = String::new();
    for b in bases {
        match client().get(format!("{b}{path}")).query(query).send().await {
            Ok(r) if r.status().is_success() => {
                return r.json().await.map_err(|e| e.to_string());
            }
            Ok(r) => last_err = r.status().to_string(),
            Err(e) => last_err = e.to_string(),
        }
        if let Some(lock) = RB_BASE.get() {
            *lock.lock().await = None;
        }
    }
    Err(last_err)
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RadioQuery {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub countrycode: String,
    #[serde(default)]
    pub order: String,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

pub async fn radio_search(q: &RadioQuery) -> Result<Vec<RbStation>, String> {
    let order = match q.order.as_str() {
        "votes" | "name" | "bitrate" | "clicktrend" | "clickcount" => q.order.clone(),
        _ => "clickcount".to_string(),
    };
    let mut query = vec![
        ("hidebroken", "true".to_string()),
        ("order", order.clone()),
        ("reverse", (order != "name").to_string()),
        ("limit", q.limit.unwrap_or(60).min(300).to_string()),
        ("offset", q.offset.unwrap_or(0).to_string()),
    ];
    if !q.name.trim().is_empty() {
        query.push(("name", q.name.trim().to_string()));
    }
    if !q.tag.trim().is_empty() {
        query.push(("tag", q.tag.trim().to_lowercase()));
        query.push(("tagExact", "false".into()));
    }
    if !q.countrycode.trim().is_empty() {
        query.push(("countrycode", q.countrycode.trim().to_uppercase()));
    }
    let v = rb_get("/json/stations/search", &query).await?;
    serde_json::from_value(v).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbTag {
    pub name: String,
    pub stationcount: i64,
}

pub async fn radio_tags() -> Result<Vec<RbTag>, String> {
    let v = rb_get(
        "/json/tags",
        &[
            ("order", "stationcount".into()),
            ("reverse", "true".into()),
            ("limit", "48".into()),
            ("hidebroken", "true".into()),
        ],
    )
    .await?;
    serde_json::from_value(v).map_err(|e| e.to_string())
}

pub async fn radio_click(uuid: &str) {
    if uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') && !uuid.is_empty() {
        let _ = rb_get(&format!("/json/url/{uuid}"), &[]).await;
    }
}

pub async fn resolve_stream(url: &str) -> Result<String, String> {
    let lower = url.to_ascii_lowercase();
    let path = lower.split('?').next().unwrap_or(&lower).to_string();
    let looks_like_list =
        path.ends_with(".pls") || path.ends_with(".m3u") || path.ends_with(".asx");
    if !looks_like_list {
        return Ok(url.to_string());
    }
    let r = client()
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let text = r.text().await.map_err(|e| e.to_string())?;
    let entries = playlist_file::parse_any(&text, &path, None);
    entries
        .into_iter()
        .find(|e| e.is_url())
        .map(|e| e.location)
        .ok_or_else(|| "no stream in playlist".to_string())
}
