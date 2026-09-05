//! Profile editor: everything the client lets the player change about how
//! they look to others without touching the game itself. The endpoints and
//! payloads mirror what league_profile_tool and league-tools send (both MIT).

use super::{ensure_enabled, get_client, lcu_get_raw, lcu_send, LcuClient};
use rand::RngExt;
use serde_json::{json, Value};

const QUEUES: [&str; 3] = ["RANKED_SOLO_5x5", "RANKED_FLEX_SR", "RANKED_TFT"];
const TIERS: [&str; 11] = [
    "UNRANKED",
    "IRON",
    "BRONZE",
    "SILVER",
    "GOLD",
    "PLATINUM",
    "EMERALD",
    "DIAMOND",
    "MASTER",
    "GRANDMASTER",
    "CHALLENGER",
];
const DIVISIONS: [&str; 5] = ["I", "II", "III", "IV", "NA"];
pub const STATUS_MAX_CHARS: usize = 255;

fn valid_tier(tier: &str) -> bool {
    TIERS.contains(&tier)
}

/// Apex tiers have no division; the client shows "NA" for them.
pub fn normalise_division(tier: &str, division: &str) -> String {
    if matches!(tier, "MASTER" | "GRANDMASTER" | "CHALLENGER" | "UNRANKED") {
        "NA".to_string()
    } else {
        division.to_string()
    }
}

async fn chat_me(client: &LcuClient) -> Result<Value, String> {
    lcu_get_raw(client, "/lol-chat/v1/me").await
}

/// The three challenge tokens (or fewer) plus the title the profile shows.
pub fn summary_preferences(summary: &Value) -> (Vec<i64>, String, String) {
    let tokens: Vec<i64> = summary
        .get("topChallenges")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("id").and_then(Value::as_i64))
                .filter(|id| *id > 0)
                .take(3)
                .collect()
        })
        .unwrap_or_default();
    let title = summary
        .get("title")
        .and_then(|t| {
            t.get("itemId")
                .and_then(Value::as_i64)
                .map(|id| id.to_string())
                .or_else(|| t.as_str().map(str::to_string))
        })
        .filter(|t| t != "-1")
        .unwrap_or_default();
    let banner = summary
        .get("bannerAccent")
        .and_then(|b| {
            b.as_str()
                .map(str::to_string)
                .or_else(|| b.as_i64().map(|n| n.to_string()))
        })
        .filter(|b| b != "-1" && !b.is_empty())
        .unwrap_or_else(|| "1".to_string());
    (tokens, title, banner)
}

/// One snapshot with everything the profile tab renders.
#[tauri::command]
pub async fn league_profile_state() -> Result<Value, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let me = chat_me(&client).await?;
    let summary = lcu_get_raw(
        &client,
        "/lol-challenges/v1/summary-player-data/local-player",
    )
    .await
    .unwrap_or(Value::Null);
    let (tokens, title, banner) = summary_preferences(&summary);
    let profile = lcu_get_raw(
        &client,
        "/lol-summoner/v1/current-summoner/summoner-profile",
    )
    .await
    .unwrap_or(Value::Null);
    let regalia = lcu_get_raw(&client, "/lol-regalia/v2/current-summoner/regalia")
        .await
        .unwrap_or(Value::Null);
    let wallet = lcu_get_raw(&client, "/lol-inventory/v1/wallet")
        .await
        .unwrap_or(Value::Null);
    let friends = lcu_get_raw(&client, "/lol-chat/v1/friends")
        .await
        .ok()
        .and_then(|f| f.as_array().map(|a| a.len()))
        .unwrap_or(0);
    let lol = me.get("lol").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "availability": me.get("availability"),
        "statusMessage": me.get("statusMessage"),
        "chatIcon": me.get("icon"),
        "rank": {
            "queue": lol.get("rankedLeagueQueue"),
            "tier": lol.get("rankedLeagueTier"),
            "division": lol.get("rankedLeagueDivision"),
        },
        "challengeCrystal": {
            "level": lol.get("challengeCrystalLevel"),
            "points": lol.get("challengePoints"),
        },
        "tokens": tokens,
        "title": title,
        "bannerAccent": banner,
        "backgroundSkinId": profile.get("backgroundSkinId"),
        "regalia": {
            "bannerType": regalia.get("bannerType"),
            "crestType": regalia.get("crestType"),
            "preferredBannerType": regalia.get("preferredBannerType"),
            "preferredCrestType": regalia.get("preferredCrestType"),
            "selectedPrestigeCrest": regalia.get("selectedPrestigeCrest"),
            "lastSeasonHighestRank": regalia.get("lastSeasonHighestRank"),
        },
        "wallet": {
            "rp": wallet.get("RP").or_else(|| wallet.get("rp")),
            "blueEssence": wallet.get("lol_blue_essence").or_else(|| wallet.get("ip")),
        },
        "friendsCount": friends,
    }))
}

/// Rank shown on the chat hovercard and social cards. Only the `lol` block is
/// patched, so presence data the client keeps there survives.
#[tauri::command]
pub async fn league_set_chat_rank(
    queue: String,
    tier: String,
    division: String,
) -> Result<Value, String> {
    ensure_enabled()?;
    if !QUEUES.contains(&queue.as_str()) {
        return Err("invalid queue".to_string());
    }
    if !valid_tier(&tier) {
        return Err("invalid tier".to_string());
    }
    let division = normalise_division(&tier, &division);
    if !DIVISIONS.contains(&division.as_str()) {
        return Err("invalid division".to_string());
    }
    let client = get_client().await?;
    let tier_value = if tier == "UNRANKED" {
        ""
    } else {
        tier.as_str()
    };
    lcu_send(
        &client,
        reqwest::Method::PUT,
        "/lol-chat/v1/me",
        Some(json!({
            "lol": {
                "rankedLeagueQueue": queue,
                "rankedLeagueTier": tier_value,
                "rankedLeagueDivision": division,
            }
        })),
    )
    .await
}

/// Puts the real rank back, read from the ranked service.
#[tauri::command]
pub async fn league_reset_chat_rank() -> Result<Value, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let stats = lcu_get_raw(&client, "/lol-ranked/v1/current-ranked-stats").await?;
    let entry = stats
        .get("highestRankedEntry")
        .cloned()
        .unwrap_or(Value::Null);
    let queue = entry
        .get("queueType")
        .and_then(Value::as_str)
        .filter(|q| QUEUES.contains(q))
        .unwrap_or("RANKED_SOLO_5x5")
        .to_string();
    let tier = entry
        .get("tier")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty() && *t != "NONE")
        .unwrap_or("UNRANKED")
        .to_string();
    let division = entry
        .get("division")
        .and_then(Value::as_str)
        .unwrap_or("NA")
        .to_string();
    league_set_chat_rank(queue, tier, division).await
}

/// The challenge crystal level and points shown next to the rank.
#[tauri::command]
pub async fn league_set_challenge_crystal(level: String, points: i64) -> Result<Value, String> {
    ensure_enabled()?;
    if !valid_tier(&level) {
        return Err("invalid tier".to_string());
    }
    if !(0..=1_000_000).contains(&points) {
        return Err("invalid points".to_string());
    }
    let client = get_client().await?;
    lcu_send(
        &client,
        reqwest::Method::PUT,
        "/lol-chat/v1/me",
        Some(json!({
            "lol": { "challengeCrystalLevel": level, "challengePoints": points }
        })),
    )
    .await
}

/// Icon shown in chat and hovercards. Unlike the profile icon, the chat
/// service accepts any icon id here.
#[tauri::command]
pub async fn league_set_chat_icon(icon_id: i64) -> Result<Value, String> {
    ensure_enabled()?;
    if icon_id < 0 {
        return Err("invalid icon".to_string());
    }
    let client = get_client().await?;
    lcu_send(
        &client,
        reqwest::Method::PUT,
        "/lol-chat/v1/me",
        Some(json!({ "icon": icon_id })),
    )
    .await
}

/// Challenge medals the account has earned, and the titles it may wear.
#[tauri::command]
pub async fn league_challenges() -> Result<Value, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let challenges = lcu_get_raw(&client, "/lol-challenges/v1/challenges/local-player").await?;
    let entries: Vec<Value> = match &challenges {
        Value::Array(arr) => arr.clone(),
        Value::Object(map) => map.values().cloned().collect(),
        _ => Vec::new(),
    };
    let mut tokens: Vec<Value> = entries
        .iter()
        .filter_map(|c| {
            let id = c.get("id").and_then(Value::as_i64)?;
            let level = c
                .get("currentLevel")
                .and_then(Value::as_str)
                .unwrap_or("NONE");
            if id <= 0 || level == "NONE" {
                return None;
            }
            let icon = c
                .get("levelToIconPath")
                .and_then(|m| m.get(level))
                .and_then(Value::as_str)
                .unwrap_or("");
            Some(json!({
                "id": id,
                "name": c.get("name").and_then(Value::as_str).unwrap_or(""),
                "description": c.get("description").and_then(Value::as_str).unwrap_or(""),
                "level": level,
                "iconPath": icon,
                "category": c.get("category").and_then(Value::as_str).unwrap_or(""),
            }))
        })
        .collect();
    tokens.sort_by(|a, b| {
        a.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("name").and_then(Value::as_str).unwrap_or(""))
    });
    let titles = lcu_get_raw(&client, "/lol-challenges/v2/titles/local-player")
        .await
        .unwrap_or(Value::Null);
    let titles: Vec<Value> = titles
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let id = t
                        .get("itemId")
                        .and_then(Value::as_i64)
                        .map(|n| n.to_string())
                        .or_else(|| t.get("id").and_then(Value::as_str).map(str::to_string))?;
                    Some(json!({
                        "id": id,
                        "name": t.get("name").and_then(Value::as_str).unwrap_or(""),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "tokens": tokens, "titles": titles }))
}

/// Applies the three medals, the title and the banner accent in one call.
/// Anything not given keeps its current value.
#[tauri::command]
pub async fn league_set_challenge_prefs(
    challenge_ids: Option<Vec<i64>>,
    title: Option<String>,
    banner_accent: Option<String>,
) -> Result<Value, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let summary = lcu_get_raw(
        &client,
        "/lol-challenges/v1/summary-player-data/local-player",
    )
    .await
    .unwrap_or(Value::Null);
    let (current_tokens, current_title, current_banner) = summary_preferences(&summary);
    let ids = challenge_ids.unwrap_or(current_tokens);
    if ids.len() > 3 || ids.iter().any(|id| *id < 0) {
        return Err("invalid tokens".to_string());
    }
    let title = title.unwrap_or(current_title);
    if !title.is_empty() && !title.chars().all(|c| c.is_ascii_digit()) {
        return Err("invalid title".to_string());
    }
    let banner = banner_accent.unwrap_or(current_banner);
    if !banner.chars().all(|c| c.is_ascii_digit()) {
        return Err("invalid banner".to_string());
    }
    lcu_send(
        &client,
        reqwest::Method::POST,
        "/lol-challenges/v1/update-player-preferences",
        Some(json!({
            "challengeIds": ids,
            "title": title,
            "bannerAccent": banner,
        })),
    )
    .await
}

/// Banner and crest preferences shown on the profile and in loading screens.
#[tauri::command]
pub async fn league_set_regalia(
    banner_type: Option<String>,
    crest_type: Option<String>,
    prestige_crest: Option<i64>,
) -> Result<Value, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let current = lcu_get_raw(&client, "/lol-regalia/v2/current-summoner/regalia").await?;
    let mut body = json!({
        "preferredBannerType": current.get("preferredBannerType").cloned().unwrap_or(json!("")),
        "preferredCrestType": current.get("preferredCrestType").cloned().unwrap_or(json!("")),
        "selectedPrestigeCrest": current.get("selectedPrestigeCrest").cloned().unwrap_or(json!(0)),
    });
    if let Some(b) = banner_type {
        body["preferredBannerType"] = json!(b);
    }
    if let Some(c) = crest_type {
        body["preferredCrestType"] = json!(c);
    }
    if let Some(p) = prestige_crest {
        body["selectedPrestigeCrest"] = json!(p);
    }
    lcu_send(
        &client,
        reqwest::Method::PUT,
        "/lol-regalia/v2/current-summoner/regalia",
        Some(body),
    )
    .await
}

/// The friends list, trimmed to what the manager shows.
#[tauri::command]
pub async fn league_friends() -> Result<Value, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let friends = lcu_get_raw(&client, "/lol-chat/v1/friends").await?;
    let list: Vec<Value> = friends
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    Some(json!({
                        "id": f.get("id").and_then(Value::as_str)?,
                        "puuid": f.get("puuid").and_then(Value::as_str).unwrap_or(""),
                        "gameName": f.get("gameName").and_then(Value::as_str).unwrap_or(""),
                        "gameTag": f.get("gameTag").and_then(Value::as_str).unwrap_or(""),
                        "name": f.get("name").and_then(Value::as_str).unwrap_or(""),
                        "availability": f.get("availability").and_then(Value::as_str).unwrap_or(""),
                        "icon": f.get("icon").and_then(Value::as_i64).unwrap_or(0),
                        "note": f.get("note").and_then(Value::as_str).unwrap_or(""),
                        "lastSeen": f.get("lastSeenOnlineTimestamp").and_then(Value::as_str).unwrap_or(""),
                        "groupName": f.get("groupName").and_then(Value::as_str).unwrap_or(""),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "friends": list }))
}

/// Removes friends one by one, spaced out so the chat service does not choke.
/// Returns how many went through and which ids failed.
#[tauri::command]
pub async fn league_remove_friends(ids: Vec<String>) -> Result<Value, String> {
    ensure_enabled()?;
    if ids.is_empty() {
        return Err("no friends selected".to_string());
    }
    let client = get_client().await?;
    let mut removed = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        if id.is_empty() || id.contains('/') || id.contains("..") {
            failed.push(id.clone());
            continue;
        }
        if index > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        match lcu_send(
            &client,
            reqwest::Method::DELETE,
            &format!("/lol-chat/v1/friends/{}", id),
            None,
        )
        .await
        {
            Ok(_) => removed += 1,
            Err(e) => {
                tracing::debug!("[league] remove friend {} failed: {}", id, e);
                failed.push(id.clone());
            }
        }
    }
    Ok(json!({ "removed": removed, "failed": failed }))
}

/// Champions the account can play right now, for the raffle.
#[tauri::command]
pub async fn league_random_champion(class: Option<String>) -> Result<Value, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let owned = lcu_get_raw(&client, "/lol-champions/v1/owned-champions-minimal").await?;
    let class = class.unwrap_or_default().to_ascii_lowercase();
    let candidates: Vec<&Value> = owned
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|c| c.get("id").and_then(Value::as_i64).unwrap_or(0) > 0)
                .filter(|c| {
                    class.is_empty()
                        || c.get("roles")
                            .and_then(Value::as_array)
                            .map(|roles| {
                                roles
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .any(|r| r.eq_ignore_ascii_case(&class))
                            })
                            .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    if candidates.is_empty() {
        return Err("no champion matches".to_string());
    }
    let picked = {
        let mut rng = rand::rng();
        candidates[rng.random_range(0..candidates.len())]
    };
    Ok(json!({
        "id": picked.get("id"),
        "name": picked.get("name"),
        "alias": picked.get("alias"),
        "roles": picked.get("roles"),
        "candidates": candidates.len(),
    }))
}

/// Hovers (or locks) a champion in the local player's open pick action.
#[tauri::command]
pub async fn league_declare_champion(
    champion_id: i64,
    lock: Option<bool>,
) -> Result<Value, String> {
    ensure_enabled()?;
    if champion_id <= 0 {
        return Err("invalid champion".to_string());
    }
    let client = get_client().await?;
    let session = lcu_get_raw(&client, "/lol-champ-select/v1/session").await?;
    let cell = session
        .get("localPlayerCellId")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let (action_id, completed) = super::champ_select::local_pick_action(&session, cell)
        .ok_or_else(|| "no pick action open".to_string())?;
    if completed {
        return Err("pick already locked".to_string());
    }
    lcu_send(
        &client,
        reqwest::Method::PATCH,
        &format!("/lol-champ-select/v1/session/actions/{}", action_id),
        Some(json!({ "championId": champion_id, "completed": lock.unwrap_or(false) })),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apex_tiers_drop_the_division() {
        assert_eq!(normalise_division("MASTER", "II"), "NA");
        assert_eq!(normalise_division("GOLD", "II"), "II");
        assert_eq!(normalise_division("UNRANKED", "I"), "NA");
    }

    #[test]
    fn the_summary_preferences_are_read_with_their_defaults() {
        let summary = json!({
            "topChallenges": [{ "id": 1 }, { "id": 2 }, { "id": 0 }, { "id": 3 }, { "id": 4 }],
            "title": { "itemId": 77 },
            "bannerAccent": "5"
        });
        assert_eq!(
            summary_preferences(&summary),
            (vec![1, 2, 3], "77".to_string(), "5".to_string())
        );
        let empty = json!({ "title": { "itemId": -1 }, "bannerAccent": "-1" });
        assert_eq!(
            summary_preferences(&empty),
            (vec![], String::new(), "1".to_string())
        );
        assert_eq!(
            summary_preferences(&Value::Null),
            (vec![], String::new(), "1".to_string())
        );
    }
}
