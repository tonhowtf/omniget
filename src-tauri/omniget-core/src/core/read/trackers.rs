use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TrackHit {
    pub id: i64,
    pub title: String,
    pub cover: Option<String>,
    pub url: String,
    pub chapters: Option<i64>,
    pub status: String,
}

const ANILIST: &str = "https://graphql.anilist.co";

async fn anilist(
    http: &reqwest::Client,
    token: &str,
    query: &str,
    vars: Value,
) -> Result<Value, String> {
    let mut r = http
        .post(ANILIST)
        .json(&json!({ "query": query, "variables": vars }));
    if !token.is_empty() {
        r = r.bearer_auth(token);
    }
    let v: Value = r
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
        return Err(format!(
            "AniList: {}",
            e["message"].as_str().unwrap_or("error")
        ));
    }
    Ok(v["data"].clone())
}

pub async fn anilist_viewer(http: &reqwest::Client, token: &str) -> Result<String, String> {
    let d = anilist(http, token, "query { Viewer { name } }", json!({})).await?;
    Ok(d["Viewer"]["name"].as_str().unwrap_or("").to_string())
}

pub async fn anilist_search(http: &reqwest::Client, query: &str) -> Result<Vec<TrackHit>, String> {
    let d = anilist(
        http,
        "",
        "query($s: String) { Page(perPage: 12) { media(search: $s, type: MANGA) { id siteUrl chapters status title { romaji english } coverImage { medium } } } }",
        json!({ "s": query }),
    )
    .await?;
    Ok(d["Page"]["media"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| TrackHit {
            id: m["id"].as_i64().unwrap_or(0),
            title: m["title"]["english"]
                .as_str()
                .or(m["title"]["romaji"].as_str())
                .unwrap_or("")
                .to_string(),
            cover: m["coverImage"]["medium"].as_str().map(str::to_string),
            url: m["siteUrl"].as_str().unwrap_or("").to_string(),
            chapters: m["chapters"].as_i64(),
            status: m["status"].as_str().unwrap_or("").to_string(),
        })
        .collect())
}

pub async fn anilist_progress(
    http: &reqwest::Client,
    token: &str,
    media: i64,
    chapter: i64,
    completed: bool,
) -> Result<(), String> {
    anilist(
        http,
        token,
        "mutation($id: Int, $p: Int, $st: MediaListStatus) { SaveMediaListEntry(mediaId: $id, progress: $p, status: $st) { id } }",
        json!({ "id": media, "p": chapter, "st": if completed { "COMPLETED" } else { "CURRENT" } }),
    )
    .await
    .map(|_| ())
}

const MU: &str = "https://api.mangaupdates.com/v1";

pub async fn mangaupdates_login(
    http: &reqwest::Client,
    user: &str,
    pass: &str,
) -> Result<String, String> {
    let v: Value = http
        .put(format!("{MU}/account/login"))
        .json(&json!({ "username": user, "password": pass }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    v["context"]["session_token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| v["reason"].as_str().unwrap_or("login failed").to_string())
}

pub async fn mangaupdates_search(
    http: &reqwest::Client,
    query: &str,
) -> Result<Vec<TrackHit>, String> {
    let v: Value = http
        .post(format!("{MU}/series/search"))
        .json(&json!({ "search": query, "perpage": 12 }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let rec = &r["record"];
            TrackHit {
                id: rec["series_id"].as_i64().unwrap_or(0),
                title: crate::core::read::xml::unescape(rec["title"].as_str().unwrap_or("")),
                cover: rec["image"]["url"]["thumb"].as_str().map(str::to_string),
                url: rec["url"].as_str().unwrap_or("").to_string(),
                chapters: None,
                status: rec["year"].as_str().unwrap_or("").to_string(),
            }
        })
        .collect())
}

pub async fn mangaupdates_progress(
    http: &reqwest::Client,
    token: &str,
    series: i64,
    chapter: i64,
) -> Result<(), String> {
    let body =
        json!([{ "series": { "id": series }, "list_id": 0, "status": { "chapter": chapter } }]);
    let up = http
        .post(format!("{MU}/lists/series/update"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if up.status().is_success() {
        return Ok(());
    }
    let add = http
        .post(format!("{MU}/lists/series"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if add.status().is_success() {
        Ok(())
    } else {
        Err(format!("MangaUpdates: {}", add.status()))
    }
}
