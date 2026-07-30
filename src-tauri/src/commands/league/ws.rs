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

/// A queue pops after roughly twelve seconds without an answer, so a longer
/// delay would simply waste the queue slot.
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

/// A ready check is only worth accepting while it is still open and the user has
/// not answered it: an explicit decline must never be overridden.
fn should_accept_ready_check(state: &str, player_response: &str) -> bool {
    pending_ready_check(state) && player_response == "None"
}

/// A queue popping while the user looks at another window is exactly the moment
/// a notification earns its keep; firing one over a focused app would be noise.
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
                // The subscription is the whole firehose, and the client emits from
                // every plugin it runs. Scanning the raw text for a uri we handle is
                // far cheaper than parsing megabytes of JSON we would discard.
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

/// Pushes the current phase once after connecting, so automations and the UI
/// don't have to wait for the next transition to learn where the client is.
async fn seed_state(client: &LcuClient) {
    if let Ok(phase) = lcu_get_raw(client, "/lol-gameflow/v1/gameflow-phase").await {
        let phase = phase.as_str().unwrap_or("").to_string();
        emit("league-phase", json!(phase));
        on_phase(client, &phase).await;
    }
}

/// The client republishes the champion select session several times per second —
/// the phase timer alone changes on every tick. Re-emitting the whole object that
/// often floods the webview: every event replaces a large state object and forces
/// each mounted panel to re-render, which is enough to lock the UI thread.
///
/// The fingerprint covers what the UI actually reacts to, so a session that only
/// advanced its clock is dropped. A slower heartbeat still gets through, so a
/// missed change can never leave the panel stale for long.
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

/// True when a payload is worth pushing to the UI: its fingerprint changed, or
/// the heartbeat window elapsed. Every hot event goes through here — the client
/// republishes lobby and ready-check state as often as champion select, and each
/// one that reaches the webview forces a re-render.
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

/// The lobby republishes on every queue-estimate tick while searching, so the
/// fingerprint covers the membership and the queue, not the countdown.
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

/// The ready check ticks its own timer; only the state and the answer matter.
fn ready_check_fingerprint(data: &Value) -> String {
    format!(
        "{}:{}",
        data.get("state").and_then(Value::as_str).unwrap_or(""),
        data.get("playerResponse")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )
}

/// Uris this client acts on. Anything else the League client publishes is noise
/// for us.
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
                    // The client re-emits the ready check every tick, so the
                    // countdown must be started only once.
                    let client = client.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(delay as u64)).await;
                        ACCEPT_PENDING.store(false, Ordering::SeqCst);
                        // The user may have declined during the countdown, so
                        // the current state decides, not the event that started it.
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
            }
        }
    }
}

/// Answers incoming champion-trade requests according to the configured
/// strategy; anything other than "accept" or "decline" leaves them alone.
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
                // Without clearing it the client keeps the request pending in the
                // UI even though it was already answered.
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

/// Starting the search only works for the lobby leader, and the lobby takes a
/// moment to exist after play-again, hence the short retry.
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

/// Accepts position and pick-order swap requests, then clears the pending state
/// so the client stops showing the prompt.
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

/// Posts the configured greeting once per champion select. The chat room can
/// lag behind the session by a few seconds, so the send is retried briefly.
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

/// Honors one eligible ally from the end-of-game ballot. The lobby members the
/// player queued with come first; otherwise the first listed ally is used.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_and_unanswered_ready_check_is_accepted() {
        assert!(should_accept_ready_check("InProgress", "None"));
    }

    #[test]
    fn a_declined_ready_check_is_never_overridden() {
        assert!(!should_accept_ready_check("InProgress", "Declined"));
        assert!(!should_accept_ready_check("InProgress", "Accepted"));
    }

    #[test]
    fn a_closed_ready_check_is_left_alone() {
        for state in ["Invalid", "EveryoneReady", "StrangerNotReady", ""] {
            assert!(!should_accept_ready_check(state, "None"), "state {}", state);
            assert!(!pending_ready_check(state), "state {}", state);
        }
    }

    #[test]
    fn the_delay_is_capped_below_the_queue_timeout() {
        assert_eq!(accept_delay_seconds(255), MAX_ACCEPT_DELAY);
        assert_eq!(accept_delay_seconds(3), 3);
        assert_eq!(accept_delay_seconds(0), 0);
    }

    #[test]
    fn a_session_that_only_advanced_its_clock_has_the_same_fingerprint() {
        let base = json!({
            "localPlayerCellId": 2,
            "timer": { "adjustedTimeLeftInPhase": 27000, "phase": "BAN_PICK" },
            "actions": [[{ "id": 5, "championId": 64, "completed": false, "isInProgress": true }]],
            "myTeam": [{ "cellId": 2, "championId": 64, "championPickIntent": 0 }]
        });
        let mut ticked = base.clone();
        ticked["timer"]["adjustedTimeLeftInPhase"] = json!(100);
        assert_eq!(
            champ_select_fingerprint(&base),
            champ_select_fingerprint(&ticked),
            "the phase clock must not count as a change"
        );
    }

    #[test]
    fn anything_the_panels_read_changes_the_fingerprint() {
        let base = json!({
            "localPlayerCellId": 2,
            "actions": [[{ "id": 5, "championId": 64, "completed": false, "isInProgress": true }]],
            "myTeam": [{ "cellId": 2, "championId": 64, "championPickIntent": 0 }],
            "benchChampions": [{ "championId": 12 }],
            "trades": [{ "id": 1, "state": "AVAILABLE" }]
        });
        let fingerprint = champ_select_fingerprint(&base);

        let mut locked = base.clone();
        locked["actions"][0][0]["completed"] = json!(true);
        assert_ne!(fingerprint, champ_select_fingerprint(&locked));

        let mut hovered = base.clone();
        hovered["myTeam"][0]["championPickIntent"] = json!(99);
        assert_ne!(fingerprint, champ_select_fingerprint(&hovered));

        let mut bench = base.clone();
        bench["benchChampions"][0]["championId"] = json!(34);
        assert_ne!(fingerprint, champ_select_fingerprint(&bench));

        let mut trade = base.clone();
        trade["trades"][0]["state"] = json!("RECEIVED");
        assert_ne!(fingerprint, champ_select_fingerprint(&trade));
    }

    #[test]
    fn an_empty_session_is_fingerprinted_without_panicking() {
        assert!(!champ_select_fingerprint(&json!({})).is_empty());
        assert!(!champ_select_fingerprint(&Value::Null).is_empty());
    }

    #[test]
    fn only_messages_for_a_handled_uri_are_parsed() {
        assert!(is_interesting(
            r#"[8,"OnJsonApiEvent",{"uri":"/lol-champ-select/v1/session","eventType":"Update"}]"#
        ));
        assert!(is_interesting(
            r#"[8,"OnJsonApiEvent",{"uri":"/lol-lobby/v2/lobby"}]"#
        ));
        // The client publishes from every plugin it runs; none of this is ours.
        assert!(!is_interesting(
            r#"[8,"OnJsonApiEvent",{"uri":"/lol-hovercard/v1/friend-info/42"}]"#
        ));
        assert!(!is_interesting(
            r#"[8,"OnJsonApiEvent",{"uri":"/lol-loot/v1/player-loot-map"}]"#
        ));
        assert!(!is_interesting(""));
    }

    #[test]
    fn the_lobby_fingerprint_ignores_the_queue_countdown() {
        let base = json!({
            "gameConfig": { "queueId": 420 },
            "localMember": { "isLeader": true },
            "members": [{ "summonerId": 7, "firstPositionPreference": "JUNGLE", "ready": true }]
        });
        let mut ticked = base.clone();
        ticked["gameConfig"]["queueEstimate"] = json!(93);
        assert_eq!(lobby_fingerprint(&base), lobby_fingerprint(&ticked));

        let mut joined = base.clone();
        joined["members"][0]["summonerId"] = json!(8);
        assert_ne!(lobby_fingerprint(&base), lobby_fingerprint(&joined));

        let mut role = base.clone();
        role["members"][0]["firstPositionPreference"] = json!("MIDDLE");
        assert_ne!(lobby_fingerprint(&base), lobby_fingerprint(&role));
    }

    #[test]
    fn the_ready_check_fingerprint_tracks_the_answer_not_the_clock() {
        let base = json!({ "state": "InProgress", "playerResponse": "None", "timer": 8.4 });
        let mut ticked = base.clone();
        ticked["timer"] = json!(3.1);
        assert_eq!(
            ready_check_fingerprint(&base),
            ready_check_fingerprint(&ticked)
        );

        let mut accepted = base.clone();
        accepted["playerResponse"] = json!("Accepted");
        assert_ne!(
            ready_check_fingerprint(&base),
            ready_check_fingerprint(&accepted)
        );
    }
}
