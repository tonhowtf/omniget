use super::{get_client, lcu_get_raw, lcu_post_raw, lcu_send, league_settings, LcuClient};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use once_cell::sync::OnceCell;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

static APP: OnceCell<tauri::AppHandle> = OnceCell::new();
static STARTED: AtomicBool = AtomicBool::new(false);
static WS_CONNECTED: AtomicBool = AtomicBool::new(false);
static MESSAGE_SENT: AtomicBool = AtomicBool::new(false);
static ACCEPT_PENDING: AtomicBool = AtomicBool::new(false);
static NOTIFIED_READY_CHECK: AtomicBool = AtomicBool::new(false);
#[allow(clippy::type_complexity)]
static LAST_EMITTED: once_cell::sync::Lazy<
    tokio::sync::Mutex<std::collections::HashMap<&'static str, (String, std::time::Instant)>>,
> = once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));
static TRADES_HANDLED: once_cell::sync::Lazy<tokio::sync::Mutex<std::collections::HashSet<i64>>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(std::collections::HashSet::new()));
#[allow(clippy::type_complexity)]
static SWAPS_HANDLED: once_cell::sync::Lazy<
    tokio::sync::Mutex<std::collections::HashSet<(&'static str, i64)>>,
> = once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(std::collections::HashSet::new()));

const MAX_ACCEPT_DELAY: u8 = 11;

pub fn is_connected() -> bool {
    WS_CONNECTED.load(Ordering::Relaxed)
}

fn pending_ready_check(state: &str) -> bool {
    state == "InProgress"
}

fn accept_delay_seconds(setting: u8) -> u8 {
    setting.min(MAX_ACCEPT_DELAY)
}

fn should_accept_ready_check(state: &str, player_response: &str) -> bool {
    pending_ready_check(state) && player_response == "None"
}

fn notify_ready_check() {
    if !league_settings().notify_ready_check {
        return;
    }
    let Some(app) = APP.get() else { return };
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if focused {
        return;
    }
    if let Err(e) = app
        .notification()
        .builder()
        .title("OmniGet")
        .body("League of Legends: match found")
        .show()
    {
        tracing::debug!("[league] ready check notification failed: {}", e);
    }
}

async fn accept_ready_check(client: &LcuClient) {
    match lcu_post_raw(client, "/lol-matchmaking/v1/ready-check/accept").await {
        Ok(_) => tracing::info!("[league] ready check accepted"),
        Err(e) => tracing::warn!("[league] auto-accept failed: {}", e),
    }
}

fn emit(event: &str, payload: Value) {
    if let Some(app) = APP.get() {
        let _ = app.emit(event, payload);
    }
}

pub fn start(app: tauri::AppHandle) {
    let _ = APP.set(app);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut backoff = 2u64;
        loop {
            if !league_settings().enabled {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                continue;
            }
            let client = match get_client().await {
                Ok(c) => c,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_secs(backoff.min(10))).await;
                    backoff = (backoff + 2).min(10);
                    continue;
                }
            };
            match run_session(&client).await {
                Ok(()) => {
                    tracing::info!("[league] websocket closed");
                }
                Err(e) => {
                    tracing::debug!("[league] websocket session ended: {}", e);
                }
            }
            if WS_CONNECTED.swap(false, Ordering::SeqCst) {
                emit(
                    "league-connected",
                    json!({ "connected": false, "port": Value::Null, "region": Value::Null }),
                );
            }
            {
                let mut cached = super::CACHED_CLIENT.lock().await;
                *cached = None;
            }
            super::sgp::forget_tokens().await;
            backoff = 2;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

async fn run_session(client: &LcuClient) -> Result<(), String> {
    let url = format!("wss://127.0.0.1:{}", client.port);
    let mut request = url
        .into_client_request()
        .map_err(|e| format!("bad websocket url: {}", e))?;
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("riot:{}", client.token));
    request.headers_mut().insert(
        "Authorization",
        format!("Basic {}", auth)
            .parse()
            .map_err(|_| "bad auth header".to_string())?,
    );
    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("tls setup failed: {}", e))?;
    let (stream, _) = tokio_tungstenite::connect_async_tls_with_config(
        request,
        None,
        false,
        Some(Connector::NativeTls(tls)),
    )
    .await
    .map_err(|e| format!("websocket connect failed: {}", e))?;

    let (mut write, mut read) = stream.split();
    write
        .send(Message::Text("[5, \"OnJsonApiEvent\"]".into()))
        .await
        .map_err(|e| format!("subscribe failed: {}", e))?;

    WS_CONNECTED.store(true, Ordering::SeqCst);
    tracing::info!("[league] websocket connected on port {}", client.port);
    emit(
        "league-connected",
        json!({ "connected": true, "port": client.port, "region": client.region }),
    );
    seed_state(client).await;

    while let Some(message) = read.next().await {
        let message = message.map_err(|e| format!("websocket read failed: {}", e))?;
        match message {
            Message::Text(text) => {
                if text.is_empty() {
                    continue;
                }
                if !is_interesting(&text) {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    handle_event(client, &value).await;
                }
            }
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

async fn seed_state(client: &LcuClient) {
    if let Ok(phase) = lcu_get_raw(client, "/lol-gameflow/v1/gameflow-phase").await {
        let phase = phase.as_str().unwrap_or("").to_string();
        emit("league-phase", json!(phase));
        on_phase(client, &phase).await;
    }
}

fn champ_select_fingerprint(session: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(
        session
            .get("localPlayerCellId")
            .and_then(Value::as_i64)
            .unwrap_or(-1)
            .to_string(),
    );
    if let Some(groups) = session.get("actions").and_then(Value::as_array) {
        for group in groups {
            for action in group.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                parts.push(format!(
                    "a{}:{}:{}:{}",
                    action.get("id").and_then(Value::as_i64).unwrap_or(-1),
                    action
                        .get("championId")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    action
                        .get("completed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    action
                        .get("isInProgress")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ));
            }
        }
    }
    for key in ["myTeam", "theirTeam"] {
        for member in session
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or(&vec![])
        {
            parts.push(format!(
                "m{}:{}:{}",
                member.get("cellId").and_then(Value::as_i64).unwrap_or(-1),
                member
                    .get("championId")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                member
                    .get("championPickIntent")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            ));
        }
    }
    for key in [
        "benchChampions",
        "trades",
        "positionSwaps",
        "pickOrderSwaps",
    ] {
        for entry in session
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or(&vec![])
        {
            parts.push(format!(
                "{}{}:{}:{}",
                key,
                entry.get("id").and_then(Value::as_i64).unwrap_or(-1),
                entry.get("championId").and_then(Value::as_i64).unwrap_or(0),
                entry.get("state").and_then(Value::as_str).unwrap_or(""),
            ));
        }
    }
    parts.join("|")
}

async fn should_emit(event: &'static str, fingerprint: String) -> bool {
    const HEARTBEAT: std::time::Duration = std::time::Duration::from_millis(1500);
    let mut seen = LAST_EMITTED.lock().await;
    match seen.get(event) {
        Some((previous, at)) if previous == &fingerprint && at.elapsed() < HEARTBEAT => false,
        _ => {
            seen.insert(event, (fingerprint, std::time::Instant::now()));
            true
        }
    }
}

fn lobby_fingerprint(lobby: &Value) -> String {
    let mut parts = vec![
        lobby
            .get("gameConfig")
            .and_then(|c| c.get("queueId"))
            .and_then(Value::as_i64)
            .unwrap_or(-1)
            .to_string(),
        lobby
            .get("localMember")
            .and_then(|m| m.get("isLeader"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .to_string(),
    ];
    for member in lobby
        .get("members")
        .and_then(Value::as_array)
        .unwrap_or(&vec![])
    {
        parts.push(format!(
            "{}:{}:{}",
            member
                .get("summonerId")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            member
                .get("firstPositionPreference")
                .and_then(Value::as_str)
                .unwrap_or(""),
            member
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ));
    }
    parts.join("|")
}

fn ready_check_fingerprint(data: &Value) -> String {
    format!(
        "{}:{}",
        data.get("state").and_then(Value::as_str).unwrap_or(""),
        data.get("playerResponse")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )
}

const HANDLED_URIS: [&str; 5] = [
    "/lol-gameflow/v1/gameflow-phase",
    "/lol-matchmaking/v1/ready-check",
    "/lol-champ-select/v1/session",
    "/lol-lobby/v2/lobby",
    "/lol-honor-v2/v1/ballot",
];

fn is_interesting(text: &str) -> bool {
    HANDLED_URIS.iter().any(|uri| text.contains(uri))
}

async fn handle_event(client: &LcuClient, value: &Value) {
    let payload = match value.as_array().and_then(|a| a.get(2)) {
        Some(p) => p,
        None => return,
    };
    let uri = payload.get("uri").and_then(Value::as_str).unwrap_or("");
    let event_type = payload
        .get("eventType")
        .and_then(Value::as_str)
        .unwrap_or("");
    let data = payload.get("data").cloned().unwrap_or(Value::Null);

    match uri {
        "/lol-gameflow/v1/gameflow-phase" => {
            let phase = data.as_str().unwrap_or("").to_string();
            emit("league-phase", json!(phase));
            on_phase(client, &phase).await;
        }
        "/lol-matchmaking/v1/ready-check" => {
            let state = data.get("state").and_then(Value::as_str).unwrap_or("");
            let response = data
                .get("playerResponse")
                .and_then(Value::as_str)
                .unwrap_or("");
            if should_emit("league-ready-check", ready_check_fingerprint(&data)).await {
                emit("league-ready-check", data.clone());
            }
            if !pending_ready_check(state) {
                ACCEPT_PENDING.store(false, Ordering::SeqCst);
                NOTIFIED_READY_CHECK.store(false, Ordering::SeqCst);
            } else if should_accept_ready_check(state, response)
                && !NOTIFIED_READY_CHECK.swap(true, Ordering::SeqCst)
            {
                notify_ready_check();
            }
            if should_accept_ready_check(state, response)
                && super::AUTO_ACCEPT.load(Ordering::Relaxed)
            {
                let delay = accept_delay_seconds(league_settings().auto_accept_delay);
                if delay == 0 {
                    accept_ready_check(client).await;
                } else if !ACCEPT_PENDING.swap(true, Ordering::SeqCst) {
                    let client = client.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(delay as u64)).await;
                        ACCEPT_PENDING.store(false, Ordering::SeqCst);
                        match lcu_get_raw(&client, "/lol-matchmaking/v1/ready-check").await {
                            Ok(current) => {
                                let state =
                                    current.get("state").and_then(Value::as_str).unwrap_or("");
                                let response = current
                                    .get("playerResponse")
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                if should_accept_ready_check(state, response) {
                                    accept_ready_check(&client).await;
                                } else {
                                    tracing::debug!(
                                        "[league] auto-accept skipped: no longer pending"
                                    );
                                }
                            }
                            Err(e) => tracing::debug!("[league] ready check re-read failed: {}", e),
                        }
                    });
                }
            }
        }
        "/lol-champ-select/v1/session" => {
            if event_type == "Delete" {
                super::CS_HANDLED.lock().await.clear();
                super::CS_FIRST_SEEN.lock().await.clear();
                super::CS_DECLARED.lock().await.clear();
                TRADES_HANDLED.lock().await.clear();
                SWAPS_HANDLED.lock().await.clear();
                MESSAGE_SENT.store(false, Ordering::SeqCst);
                super::skins::reset_session().await;
                LAST_EMITTED.lock().await.remove("league-champ-select");
                emit("league-champ-select", Value::Null);
                return;
            }
            if should_emit("league-champ-select", champ_select_fingerprint(&data)).await {
                emit("league-champ-select", data.clone());
            }
            let settings = league_settings();
            if settings.auto_pick || settings.auto_ban {
                if let Err(e) = super::handle_champ_select(client, &settings, &data).await {
                    tracing::debug!("[league] champ select handling: {}", e);
                }
            }
            handle_trades(client, &settings, &data).await;
            handle_swaps(client, &settings, &data).await;
            send_auto_message(client, &settings);
            super::skins::on_champ_select(client, &settings, &data).await;
        }
        "/lol-lobby/v2/lobby" => {
            if event_type == "Delete" {
                LAST_EMITTED.lock().await.remove("league-lobby");
                emit("league-lobby", Value::Null);
            } else if should_emit("league-lobby", lobby_fingerprint(&data)).await {
                emit("league-lobby", data.clone());
            }
        }
        "/lol-honor-v2/v1/ballot" => {
            if event_type != "Delete" && league_settings().auto_honor {
                honor_from_ballot(client, &data).await;
            }
        }
        _ => {}
    }
}

async fn on_phase(client: &LcuClient, phase: &str) {
    let settings = league_settings();
    match phase {
        "Reconnect" => {
            if settings.auto_reconnect {
                match lcu_post_raw(client, "/lol-gameflow/v1/reconnect").await {
                    Ok(_) => tracing::info!("[league] reconnected to the game"),
                    Err(e) => tracing::warn!("[league] auto-reconnect failed: {}", e),
                }
            }
        }
        "EndOfGame" => {
            if settings.auto_play_again {
                match lcu_post_raw(client, "/lol-lobby/v2/play-again").await {
                    Ok(_) => {
                        tracing::info!("[league] queued play again");
                        if settings.auto_requeue {
                            requeue_if_leader(client).await;
                        }
                    }
                    Err(e) => tracing::debug!("[league] auto play-again failed: {}", e),
                }
            }
        }
        "PreEndOfGame" => {
            if settings.auto_honor {
                if let Ok(ballot) = lcu_get_raw(client, "/lol-honor-v2/v1/ballot").await {
                    honor_from_ballot(client, &ballot).await;
                }
            }
        }
        _ => {
            if phase != "ChampSelect" {
                super::CS_HANDLED.lock().await.clear();
                super::CS_FIRST_SEEN.lock().await.clear();
                super::CS_DECLARED.lock().await.clear();
                TRADES_HANDLED.lock().await.clear();
                SWAPS_HANDLED.lock().await.clear();
                MESSAGE_SENT.store(false, Ordering::SeqCst);
                super::skins::reset_session().await;
            }
        }
    }
}

async fn handle_trades(
    client: &LcuClient,
    settings: &omniget_core::models::settings::LeagueSettings,
    session: &Value,
) {
    let strategy = settings.auto_trade.as_str();
    if strategy != "accept" && strategy != "decline" {
        return;
    }
    let trades: Vec<Value> = session
        .get("trades")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for trade in trades {
        let state = trade.get("state").and_then(Value::as_str).unwrap_or("");
        let id = trade.get("id").and_then(Value::as_i64).unwrap_or(-1);
        if state != "RECEIVED" || id < 0 {
            continue;
        }
        {
            let handled = TRADES_HANDLED.lock().await;
            if handled.contains(&id) {
                continue;
            }
        }
        let path = format!(
            "/lol-champ-select/v1/session/trades/{}/{}",
            id,
            if strategy == "accept" {
                "accept"
            } else {
                "decline"
            }
        );
        match lcu_post_raw(client, &path).await {
            Ok(_) => {
                TRADES_HANDLED.lock().await.insert(id);
                tracing::info!("[league] trade request {}: {}ed", id, strategy);
                let cleared = lcu_post_raw(
                    client,
                    &format!("/lol-champ-select/v1/ongoing-trade/{}/clear", id),
                )
                .await;
                if let Err(e) = cleared {
                    tracing::debug!("[league] clearing trade {} failed: {}", id, e);
                }
            }
            Err(e) => tracing::debug!("[league] trade {} failed: {}", strategy, e),
        }
    }
}

async fn requeue_if_leader(client: &LcuClient) {
    for attempt in 0..3 {
        tokio::time::sleep(std::time::Duration::from_millis(700 * (attempt + 1))).await;
        let Ok(lobby) = lcu_get_raw(client, "/lol-lobby/v2/lobby").await else {
            continue;
        };
        if !super::lobby::is_leader(&lobby) {
            tracing::debug!("[league] not the lobby leader, skipping requeue");
            return;
        }
        match lcu_post_raw(client, "/lol-lobby/v2/lobby/matchmaking/search").await {
            Ok(_) => {
                tracing::info!("[league] requeued after play again");
                return;
            }
            Err(e) => tracing::debug!("[league] requeue attempt failed: {}", e),
        }
    }
}

async fn handle_swaps(
    client: &LcuClient,
    settings: &omniget_core::models::settings::LeagueSettings,
    session: &Value,
) {
    if !settings.auto_accept_swaps {
        return;
    }
    for (kind, id) in super::lobby::pending_swaps(session) {
        {
            let handled = SWAPS_HANDLED.lock().await;
            if handled.contains(&(kind, id)) {
                continue;
            }
        }
        let path = format!("/lol-champ-select/v1/session/{}/{}/accept", kind, id);
        match lcu_post_raw(client, &path).await {
            Ok(_) => {
                SWAPS_HANDLED.lock().await.insert((kind, id));
                tracing::info!("[league] accepted {} request {}", kind, id);
                let cleared = lcu_post_raw(
                    client,
                    &format!("/lol-champ-select/v1/ongoing-swap/{}/clear", id),
                )
                .await;
                if let Err(e) = cleared {
                    tracing::debug!("[league] clearing swap {} failed: {}", id, e);
                }
            }
            Err(e) => tracing::debug!("[league] accepting {} failed: {}", kind, e),
        }
    }
}

fn send_auto_message(
    client: &LcuClient,
    settings: &omniget_core::models::settings::LeagueSettings,
) {
    let text = settings.auto_message.trim().to_string();
    if text.is_empty() || MESSAGE_SENT.swap(true, Ordering::SeqCst) {
        return;
    }
    let client = client.clone();
    tauri::async_runtime::spawn(async move {
        for _ in 0..6 {
            if super::send_champ_select_chat(&client, &text).await.is_ok() {
                tracing::info!("[league] champ select greeting sent");
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        tracing::debug!("[league] champ select greeting never sent");
    });
}

async fn honor_from_ballot(client: &LcuClient, ballot: &Value) {
    let eligible: Vec<Value> = ballot
        .get("eligibleAllies")
        .or_else(|| ballot.get("eligiblePlayers"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if eligible.is_empty() {
        return;
    }
    let already_voted = ballot
        .get("honoredPlayers")
        .and_then(Value::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if already_voted {
        return;
    }

    let lobby_members: Vec<String> = lcu_get_raw(client, "/lol-lobby/v2/lobby/members")
        .await
        .ok()
        .and_then(|m| {
            m.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|p| p.get("puuid").and_then(Value::as_str).map(String::from))
                    .collect()
            })
        })
        .unwrap_or_default();

    let choice = eligible
        .iter()
        .find(|p| {
            p.get("puuid")
                .and_then(Value::as_str)
                .map(|id| lobby_members.iter().any(|m| m == id))
                .unwrap_or(false)
        })
        .or_else(|| eligible.first());
    let choice = match choice {
        Some(c) => c,
        None => return,
    };
    let puuid = choice.get("puuid").and_then(Value::as_str).unwrap_or("");
    let summoner_id = choice
        .get("summonerId")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let modern = lcu_send(
        client,
        reqwest::Method::POST,
        "/lol-honor/v1/honor",
        Some(json!({ "recipientPuuid": puuid, "honorType": "HEART" })),
    )
    .await;
    if modern.is_ok() {
        tracing::info!("[league] honored a teammate");
        return;
    }
    let legacy = lcu_send(
        client,
        reqwest::Method::POST,
        "/lol-honor-v2/v1/honor-player",
        Some(json!({ "summonerId": summoner_id, "honorCategory": "HEART" })),
    )
    .await;
    match legacy {
        Ok(_) => tracing::info!("[league] honored a teammate"),
        Err(e) => tracing::debug!("[league] auto-honor failed: {}", e),
    }
}
