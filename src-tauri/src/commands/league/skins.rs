use super::{ensure_enabled, get_client, lcu_get_raw, lcu_send, LcuClient};
use once_cell::sync::Lazy;
use rand::RngExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use tokio::sync::Mutex;

pub const HISTORY_WINDOW: usize = 5;

#[derive(Clone, Debug, Serialize)]
pub struct Chroma {
    pub id: i64,
    pub name: String,
    pub colors: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkinOption {
    pub id: i64,
    pub name: String,
    pub is_base: bool,
    pub chromas: Vec<Chroma>,
}

pub fn selectable_skins(carousel: &Value) -> Vec<SkinOption> {
    let usable = |s: &Value| {
        s.get("unlocked").and_then(Value::as_bool).unwrap_or(false)
            && !s.get("disabled").and_then(Value::as_bool).unwrap_or(false)
    };
    carousel
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|s| usable(s))
                .filter_map(|s| {
                    let chromas = s
                        .get("childSkins")
                        .and_then(Value::as_array)
                        .map(|kids| {
                            kids.iter()
                                .filter(|c| usable(c))
                                .filter_map(|c| {
                                    Some(Chroma {
                                        id: c.get("id").and_then(Value::as_i64)?,
                                        name: c
                                            .get("name")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                        colors: c
                                            .get("colors")
                                            .and_then(Value::as_array)
                                            .map(|a| {
                                                a.iter()
                                                    .filter_map(Value::as_str)
                                                    .map(str::to_string)
                                                    .collect()
                                            })
                                            .unwrap_or_default(),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(SkinOption {
                        id: s.get("id").and_then(Value::as_i64)?,
                        name: s
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        is_base: s.get("isBase").and_then(Value::as_bool).unwrap_or(false),
                        chromas,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Roll {
    pub skin_id: i64,
    pub skin_name: String,
    pub selected_id: i64,
    pub chroma_name: Option<String>,
}

pub fn roll<R: rand::Rng>(
    skins: &[SkinOption],
    recent: &[i64],
    include_base: bool,
    rng: &mut R,
) -> Option<Roll> {
    if skins.is_empty() {
        return None;
    }
    let non_base: Vec<&SkinOption> = skins.iter().filter(|s| !s.is_base).collect();
    let pool: Vec<&SkinOption> = if include_base || non_base.is_empty() {
        skins.iter().collect()
    } else {
        non_base
    };
    let fresh: Vec<&SkinOption> = pool
        .iter()
        .copied()
        .filter(|s| !recent.contains(&s.id))
        .collect();
    let pool = if fresh.is_empty() { pool } else { fresh };
    let skin = pool[rng.random_range(0..pool.len())];
    let slot = rng.random_range(0..=skin.chromas.len());
    let chroma = if slot == 0 {
        None
    } else {
        skin.chromas.get(slot - 1)
    };
    Some(Roll {
        skin_id: skin.id,
        skin_name: skin.name.clone(),
        selected_id: chroma.map(|c| c.id).unwrap_or(skin.id),
        chroma_name: chroma.map(|c| c.name.clone()),
    })
}

static HISTORY: Lazy<Mutex<HashMap<i64, VecDeque<i64>>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static ROLLED_FOR: Lazy<Mutex<Option<i64>>> = Lazy::new(|| Mutex::new(None));
static WARD_ROLLED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

pub async fn reset_session() {
    *ROLLED_FOR.lock().await = None;
    *WARD_ROLLED.lock().await = false;
}

async fn remember(champion_id: i64, skin_id: i64) {
    let mut history = HISTORY.lock().await;
    let entry = history.entry(champion_id).or_default();
    entry.push_back(skin_id);
    while entry.len() > HISTORY_WINDOW {
        entry.pop_front();
    }
}

async fn recent(champion_id: i64) -> Vec<i64> {
    HISTORY
        .lock()
        .await
        .get(&champion_id)
        .map(|h| h.iter().copied().collect())
        .unwrap_or_default()
}

async fn roll_and_apply(
    client: &LcuClient,
    champion_id: i64,
    include_base: bool,
) -> Result<Roll, String> {
    let carousel = lcu_get_raw(client, "/lol-champ-select/v1/skin-carousel-skins").await?;
    let skins = selectable_skins(&carousel);
    let history = recent(champion_id).await;
    let rolled = {
        let mut rng = rand::rng();
        roll(&skins, &history, include_base, &mut rng)
    }
    .ok_or_else(|| "no skin to roll".to_string())?;
    lcu_send(
        client,
        reqwest::Method::PATCH,
        "/lol-champ-select/v1/session/my-selection",
        Some(json!({ "selectedSkinId": rolled.selected_id })),
    )
    .await?;
    remember(champion_id, rolled.skin_id).await;
    tracing::info!(
        "[league] skin roulette: {} ({})",
        rolled.skin_name,
        rolled.chroma_name.as_deref().unwrap_or("no chroma")
    );
    Ok(rolled)
}

pub fn owned_ward_ids(inventory: &Value) -> Vec<i64> {
    inventory
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|e| e.get("owned").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|e| e.get("itemId").and_then(Value::as_i64))
                .filter(|id| *id >= 0)
                .collect()
        })
        .unwrap_or_default()
}

pub fn ward_slot(loadouts: &Value) -> Option<(String, i64)> {
    let arr = loadouts.as_array()?;
    for loadout in arr {
        let id = loadout.get("id").and_then(Value::as_str)?;
        if let Some(item) = loadout
            .get("loadout")
            .and_then(|l| l.get("WARD_SKIN_SLOT"))
            .and_then(|s| s.get("itemId"))
            .and_then(Value::as_i64)
        {
            return Some((id.to_string(), item));
        }
    }
    None
}

pub fn roll_ward<R: rand::Rng>(owned: &[i64], current: i64, rng: &mut R) -> Option<i64> {
    let pool: Vec<i64> = owned.iter().copied().filter(|id| *id != current).collect();
    if pool.is_empty() {
        return None;
    }
    Some(pool[rng.random_range(0..pool.len())])
}

async fn roll_ward_and_apply(client: &LcuClient) -> Result<i64, String> {
    let inventory = lcu_get_raw(client, "/lol-inventory/v2/inventory/WARD_SKIN").await?;
    let owned = owned_ward_ids(&inventory);
    let loadouts = lcu_get_raw(client, "/lol-loadouts/v4/loadouts/scope/account").await?;
    let (loadout_id, current) = ward_slot(&loadouts).ok_or_else(|| "no ward slot".to_string())?;
    let picked = {
        let mut rng = rand::rng();
        roll_ward(&owned, current, &mut rng)
    }
    .ok_or_else(|| "no ward skin to roll".to_string())?;
    lcu_send(
        client,
        reqwest::Method::PATCH,
        &format!("/lol-loadouts/v4/loadouts/{}", loadout_id),
        Some(json!({
            "loadout": {
                "WARD_SKIN_SLOT": {
                    "contentId": "",
                    "data": {},
                    "inventoryType": "WARD_SKIN",
                    "itemId": picked,
                }
            }
        })),
    )
    .await?;
    tracing::info!("[league] ward roulette: {} -> {}", current, picked);
    Ok(picked)
}

pub(crate) async fn on_champ_select(
    client: &LcuClient,
    settings: &omniget_core::models::settings::LeagueSettings,
    session: &Value,
) {
    if !settings.skin_roulette && !settings.ward_roulette {
        return;
    }
    let champion = super::champ_select::locked_champion(session);
    if champion <= 0 {
        return;
    }
    if settings.skin_roulette {
        let already = { *ROLLED_FOR.lock().await == Some(champion) };
        if !already {
            *ROLLED_FOR.lock().await = Some(champion);
            if let Err(e) =
                roll_and_apply(client, champion, settings.skin_roulette_include_base).await
            {
                tracing::debug!("[league] skin roulette skipped: {}", e);
            }
        }
    }
    if settings.ward_roulette {
        let already = { *WARD_ROLLED.lock().await };
        if !already {
            *WARD_ROLLED.lock().await = true;
            if let Err(e) = roll_ward_and_apply(client).await {
                tracing::debug!("[league] ward roulette skipped: {}", e);
            }
        }
    }
}

#[tauri::command]
pub async fn league_roll_skin(include_base: Option<bool>) -> Result<Roll, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    let session = lcu_get_raw(&client, "/lol-champ-select/v1/session").await?;
    let champion = super::champ_select::locked_champion(&session);
    if champion <= 0 {
        return Err("lock a champion first".to_string());
    }
    let include_base =
        include_base.unwrap_or_else(|| super::league_settings().skin_roulette_include_base);
    roll_and_apply(&client, champion, include_base).await
}

#[tauri::command]
pub async fn league_roll_ward() -> Result<i64, String> {
    ensure_enabled()?;
    let client = get_client().await?;
    roll_ward_and_apply(&client).await
}
