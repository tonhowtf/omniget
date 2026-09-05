//! Service Gateway Proxy: the private HTTP API the League Client itself uses
//! to talk to Riot's backend. The client hands out the tokens it uses, so the
//! same calls can be made directly, which reaches data the local API hides or
//! trims (full match history of any player, ranked stats, replays).
//!
//! Hosts come from League Akari's built-in table (packet-captured, MIT). They
//! are configuration, not protocol: a region missing here is unsupported, and
//! no host is ever guessed from a region name.

use super::{ensure_enabled, get_client, lcu_get_raw, LcuClient};
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug)]
pub struct Server {
    pub id: &'static str,
    /// Host for endpoints that take the entitlements token (match history).
    pub match_history: &'static str,
    /// Host for endpoints that take the league session token.
    pub common: &'static str,
    /// The region segment inside paths, when it differs from the id.
    pub region_path: Option<&'static str>,
}

const SERVERS: &[Server] = &[
    Server {
        id: "BR1",
        match_history: "https://usw2-red.pp.sgp.pvp.net",
        common: "https://br-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "NA1",
        match_history: "https://usw2-red.pp.sgp.pvp.net",
        common: "https://na-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "LA1",
        match_history: "https://usw2-red.pp.sgp.pvp.net",
        common: "https://lan-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "LA2",
        match_history: "https://usw2-red.pp.sgp.pvp.net",
        common: "https://las-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "PBE",
        match_history: "https://usw2-red.pp.sgp.pvp.net",
        common: "https://pbe-red.lol.sgp.pvp.net",
        region_path: Some("PBE1"),
    },
    Server {
        id: "EUW",
        match_history: "https://euc1-red.pp.sgp.pvp.net",
        common: "https://euw-red.lol.sgp.pvp.net",
        region_path: Some("EUW1"),
    },
    Server {
        id: "RU",
        match_history: "https://euc1-red.pp.sgp.pvp.net",
        common: "https://ru-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "TR1",
        match_history: "https://euc1-red.pp.sgp.pvp.net",
        common: "https://tr-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "JP",
        match_history: "https://apne1-red.pp.sgp.pvp.net",
        common: "https://jp-red.lol.sgp.pvp.net",
        region_path: Some("JP1"),
    },
    Server {
        id: "KR",
        match_history: "https://apne1-red.pp.sgp.pvp.net",
        common: "https://kr-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "OC1",
        match_history: "https://apse1-red.pp.sgp.pvp.net",
        common: "https://oce-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "TW2",
        match_history: "https://apse1-red.pp.sgp.pvp.net",
        common: "https://tw2-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "SG2",
        match_history: "https://apse1-red.pp.sgp.pvp.net",
        common: "https://sg2-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "PH2",
        match_history: "https://apse1-red.pp.sgp.pvp.net",
        common: "https://ph2-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "VN2",
        match_history: "https://apse1-red.pp.sgp.pvp.net",
        common: "https://vn2-red.lol.sgp.pvp.net",
        region_path: None,
    },
    Server {
        id: "TH2",
        match_history: "https://apse1-red.pp.sgp.pvp.net",
        common: "https://th2-red.lol.sgp.pvp.net",
        region_path: None,
    },
];

/// Normalises the region the client reports into a server id. Some clients
/// say "BR", others "BR1"; the table uses one spelling.
pub fn server_id(region: &str) -> String {
    let upper = region.trim().to_ascii_uppercase();
    match upper.as_str() {
        "NA" => "NA1",
        "BR" => "BR1",
        "TR" => "TR1",
        "LAN" => "LA1",
        "LAS" => "LA2",
        "OCE" => "OC1",
        "EUW1" => "EUW",
        "JP1" => "JP",
        other => other,
    }
    .to_string()
}

pub fn server_for(region: &str) -> Option<Server> {
    let id = server_id(region);
    SERVERS.iter().copied().find(|s| s.id == id)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenKind {
    Entitlements,
    LeagueSession,
}

#[derive(Clone)]
struct Tokens {
    entitlements: String,
    league_session: String,
    fetched: std::time::Instant,
}

static TOKENS: Lazy<Mutex<Option<Tokens>>> = Lazy::new(|| Mutex::new(None));
const TOKEN_TTL: std::time::Duration = std::time::Duration::from_secs(300);

async fn tokens(client: &LcuClient) -> Result<Tokens, String> {
    {
        let cached = TOKENS.lock().await;
        if let Some(t) = cached.as_ref() {
            if t.fetched.elapsed() < TOKEN_TTL {
                return Ok(t.clone());
            }
        }
    }
    let entitlements = lcu_get_raw(client, "/entitlements/v1/token")
        .await?
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "no entitlements token".to_string())?;
    let session = lcu_get_raw(client, "/lol-league-session/v1/league-session-token").await?;
    let league_session = session
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "no league session token".to_string())?;
    let fresh = Tokens {
        entitlements,
        league_session,
        fetched: std::time::Instant::now(),
    };
    *TOKENS.lock().await = Some(fresh.clone());
    Ok(fresh)
}

pub async fn forget_tokens() {
    *TOKENS.lock().await = None;
}

static HTTP: Lazy<Option<reqwest::Client>> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()
});

fn enabled() -> bool {
    super::league_settings().sgp_enabled
}

struct Target {
    server: Server,
    tokens: Tokens,
}

async fn target(client: &LcuClient) -> Result<Target, String> {
    if !enabled() {
        return Err("sgp disabled".to_string());
    }
    let region = client
        .region
        .clone()
        .ok_or_else(|| "region unknown".to_string())?;
    let server =
        server_for(&region).ok_or_else(|| format!("region {} has no sgp config", region))?;
    let tokens = tokens(client).await?;
    Ok(Target { server, tokens })
}

fn region_segment(server: &Server) -> &'static str {
    server.region_path.unwrap_or(server.id)
}

async fn send(
    target: &Target,
    kind: TokenKind,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<reqwest::Response, String> {
    let http = HTTP
        .clone()
        .ok_or_else(|| "http client init failed".to_string())?;
    let (host, token) = match kind {
        TokenKind::Entitlements => (target.server.match_history, &target.tokens.entitlements),
        TokenKind::LeagueSession => (target.server.common, &target.tokens.league_session),
    };
    let mut request = http
        .request(method, format!("{}{}", host, path))
        .bearer_auth(token);
    if let Some(b) = body {
        request = request.json(&b);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("sgp request failed: {}", e))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        // A stale token is the common cause; the next call fetches a new one.
        forget_tokens().await;
    }
    if !status.is_success() {
        return Err(format!("sgp {} {}", status.as_u16(), path));
    }
    Ok(response)
}

async fn get_json(target: &Target, kind: TokenKind, path: &str) -> Result<Value, String> {
    send(target, kind, reqwest::Method::GET, path, None)
        .await?
        .json::<Value>()
        .await
        .map_err(|e| format!("sgp decode failed: {}", e))
}

/// Match history in the client's own (match-v5 style) shape.
async fn match_history_raw(
    target: &Target,
    puuid: &str,
    start: u32,
    count: u32,
) -> Result<Value, String> {
    get_json(
        target,
        TokenKind::Entitlements,
        &format!(
            "/match-history-query/v1/products/lol/player/{}/SUMMARY?startIndex={}&count={}",
            puuid, start, count
        ),
    )
    .await
}

/// Reshapes one SGP game into the layout the local match-history endpoint
/// uses, so every stat routine in this module keeps working on either source.
pub fn game_to_local(game: &Value, puuid: &str) -> Option<Value> {
    let json = game.get("json").unwrap_or(game);
    let participants = json.get("participants")?.as_array()?;
    let me = participants
        .iter()
        .find(|p| p.get("puuid").and_then(Value::as_str) == Some(puuid))
        .or_else(|| participants.first())?;
    let num = |key: &str| me.get(key).cloned().unwrap_or(json!(0));
    let mut stats = json!({
        "win": me.get("win"),
        "kills": num("kills"),
        "deaths": num("deaths"),
        "assists": num("assists"),
        "champLevel": num("champLevel"),
        "goldEarned": num("goldEarned"),
        "totalDamageDealtToChampions": num("totalDamageDealtToChampions"),
        "totalDamageTaken": num("totalDamageTaken"),
        "totalMinionsKilled": num("totalMinionsKilled"),
        "neutralMinionsKilled": num("neutralMinionsKilled"),
        "visionScore": num("visionScore"),
        "wardsPlaced": num("wardsPlaced"),
        "wardsKilled": num("wardsKilled"),
        "firstBloodKill": me.get("firstBloodKill"),
        "largestMultiKill": num("largestMultiKill"),
    });
    for i in 0..7 {
        let key = format!("item{}", i);
        stats[&key] = num(&key);
    }
    let identity = json!({
        "participantId": 1,
        "player": {
            "puuid": me.get("puuid"),
            "gameName": me.get("riotIdGameName"),
            "tagLine": me.get("riotIdTagline"),
            "summonerName": me.get("summonerName"),
        }
    });
    let participant = json!({
        "participantId": 1,
        "teamId": me.get("teamId"),
        "championId": me.get("championId"),
        "spell1Id": me.get("summoner1Id"),
        "spell2Id": me.get("summoner2Id"),
        "timeline": {
            "lane": me.get("lane"),
            "role": me.get("role"),
            "teamPosition": me.get("teamPosition"),
        },
        "stats": stats,
    });
    Some(json!({
        "gameId": json.get("gameId"),
        "gameCreation": json.get("gameCreation"),
        "gameDuration": json.get("gameDuration"),
        "gameMode": json.get("gameMode"),
        "gameType": json.get("gameType"),
        "queueId": json.get("queueId"),
        "mapId": json.get("mapId"),
        "platformId": json.get("platformId"),
        "source": "sgp",
        "participantIdentities": [identity],
        "participants": [participant],
    }))
}

/// Games for a player from the backend, already in the local shape. Empty
/// (not an error) when SGP is off or the region is unsupported, so callers can
/// fall through quietly.
pub(crate) async fn match_history_local(
    client: &LcuClient,
    puuid: &str,
    start: u32,
    count: u32,
) -> Result<Vec<Value>, String> {
    let target = target(client).await?;
    let raw = match_history_raw(&target, puuid, start, count.min(50)).await?;
    Ok(raw
        .get("games")
        .and_then(Value::as_array)
        .map(|games| {
            games
                .iter()
                .filter_map(|g| game_to_local(g, puuid))
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Serialize)]
pub struct SgpStatus {
    pub enabled: bool,
    pub supported: bool,
    pub server: Option<String>,
    pub region: Option<String>,
    pub tokens_ready: bool,
}

#[tauri::command]
pub async fn league_sgp_status() -> Result<SgpStatus, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let region = client.region.clone();
    let server = region.as_deref().and_then(server_for);
    let tokens_ready = if enabled() && server.is_some() {
        tokens(&client).await.is_ok()
    } else {
        false
    };
    Ok(SgpStatus {
        enabled: enabled(),
        supported: server.is_some(),
        server: server.map(|s| s.id.to_string()),
        region,
        tokens_ready,
    })
}

/// Match history for any player, straight from the backend.
#[tauri::command]
pub async fn league_sgp_match_history(
    puuid: String,
    start: Option<u32>,
    count: Option<u32>,
) -> Result<Value, String> {
    ensure_enabled()?;
    if puuid.is_empty() || !puuid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("invalid player".to_string());
    }
    let client = get_client().await?;
    let games =
        match_history_local(&client, &puuid, start.unwrap_or(0), count.unwrap_or(20)).await?;
    Ok(json!({ "games": { "games": games }, "source": "sgp" }))
}

/// Ranked stats straight from the leagues service, including previous
/// season peaks the local endpoint leaves out.
#[tauri::command]
pub async fn league_sgp_ranked(puuid: String) -> Result<Value, String> {
    ensure_enabled()?;
    if puuid.is_empty() || !puuid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("invalid player".to_string());
    }
    let client = get_client().await?;
    let target = target(&client).await?;
    get_json(
        &target,
        TokenKind::LeagueSession,
        &format!("/leagues-ledge/v2/rankedStats/puuid/{}", puuid),
    )
    .await
}

/// Summoner records for a batch of puuids (names, levels, privacy).
#[tauri::command]
pub async fn league_sgp_summoners(puuids: Vec<String>) -> Result<Value, String> {
    ensure_enabled()?;
    if puuids.is_empty() || puuids.len() > 20 {
        return Err("give between 1 and 20 players".to_string());
    }
    let client = get_client().await?;
    let target = target(&client).await?;
    let path = format!(
        "/summoner-ledge/v1/regions/{}/summoners/puuids",
        region_segment(&target.server)
    );
    send(
        &target,
        TokenKind::LeagueSession,
        reqwest::Method::POST,
        &path,
        Some(json!(puuids)),
    )
    .await?
    .json::<Value>()
    .await
    .map_err(|e| format!("sgp decode failed: {}", e))
}

/// Downloads the replay file for a game into the default download folder.
#[tauri::command]
pub async fn league_sgp_download_replay(game_id: i64) -> Result<String, String> {
    ensure_enabled()?;
    if game_id <= 0 {
        return Err("invalid game".to_string());
    }
    let client = get_client().await?;
    let target = target(&client).await?;
    let region = region_segment(&target.server);
    let path = format!(
        "/match-history-query/v3/product/lol/matchId/{}_{}/infoType/replay",
        region, game_id
    );
    let response = send(
        &target,
        TokenKind::Entitlements,
        reqwest::Method::GET,
        &path,
        None,
    )
    .await?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("replay download failed: {}", e))?;
    if bytes.is_empty() {
        return Err("replay not available".to_string());
    }
    let settings = crate::storage::config::load_settings_standalone();
    let dir = settings.download.default_output_dir.join("League replays");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("cannot create folder: {}", e))?;
    let file = dir.join(format!("{}-{}.rofl", region, game_id));
    tokio::fs::write(&file, &bytes)
        .await
        .map_err(|e| format!("cannot write replay: {}", e))?;
    Ok(file.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_spellings_collapse_onto_the_table() {
        assert_eq!(server_id("br"), "BR1");
        assert_eq!(server_id("BR1"), "BR1");
        assert_eq!(server_id("EUW1"), "EUW");
        assert_eq!(server_id("euw"), "EUW");
        assert_eq!(server_id("OCE"), "OC1");
        assert!(server_for("BR").is_some());
        assert!(
            server_for("EUN1").is_none(),
            "no host is guessed for a region outside the table"
        );
        assert_eq!(server_for("EUW").unwrap().region_path, Some("EUW1"));
        assert_eq!(region_segment(&server_for("BR1").unwrap()), "BR1");
    }

    #[test]
    fn a_backend_game_is_reshaped_for_the_local_stat_routines() {
        let game = json!({
            "metadata": {},
            "json": {
                "gameId": 42, "gameCreation": 1, "gameDuration": 1800, "queueId": 420, "gameMode": "CLASSIC",
                "participants": [
                    { "puuid": "other", "championId": 1, "win": false, "kills": 0, "deaths": 9, "assists": 0 },
                    { "puuid": "me", "championId": 7, "win": true, "kills": 5, "deaths": 2, "assists": 8,
                      "goldEarned": 12000, "totalDamageDealtToChampions": 20000, "summoner1Id": 4, "summoner2Id": 12,
                      "lane": "MIDDLE", "role": "SOLO", "teamPosition": "MIDDLE", "item0": 3, "riotIdGameName": "Me", "riotIdTagline": "BR1" }
                ]
            }
        });
        let local = game_to_local(&game, "me").unwrap();
        assert_eq!(local["gameId"], 42);
        assert_eq!(local["queueId"], 420);
        let p = &local["participants"][0];
        assert_eq!(p["championId"], 7);
        assert_eq!(p["spell1Id"], 4);
        assert_eq!(p["stats"]["win"], true);
        assert_eq!(p["stats"]["kills"], 5);
        assert_eq!(p["stats"]["goldEarned"], 12000);
        assert_eq!(p["stats"]["item0"], 3);
        assert_eq!(p["stats"]["item5"], 0);
        assert_eq!(p["timeline"]["lane"], "MIDDLE");
        assert_eq!(local["participantIdentities"][0]["player"]["puuid"], "me");
        assert_eq!(
            local["participantIdentities"][0]["player"]["gameName"],
            "Me"
        );
        assert!(game_to_local(&json!({ "json": {} }), "me").is_none());
    }
}
