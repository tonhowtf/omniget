//! Real-time auto-save: reads the client's update channel and, when a new
//! message lands in a chat with an enabled rule, runs that rule right away
//! (a few seconds later, so an album arrives whole). The interval loop in
//! `watch.rs` stays as the fallback for anything missed (big channels do
//! not always push updates).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use grammers_client::grammers_tl_types as tl;
use grammers_session::updates::UpdatesLike;
use serde_json::json;
use tokio::sync::mpsc;

use super::auth::TelegramSessionHandle;
use super::events;
use super::watch;

static CONNECTED: AtomicBool = AtomicBool::new(false);
static LAST_EVENT_AT: AtomicI64 = AtomicI64::new(0);
static TRIGGERED: AtomicU64 = AtomicU64::new(0);
/// Every update batch Telegram pushed, rule or not (diagnostics).
static RECEIVED: AtomicU64 = AtomicU64::new(0);
static LAST_RECEIVED_AT: AtomicI64 = AtomicI64::new(0);

/// Waits this long after a new message before running the rule, so the
/// rest of an album (sent as separate messages) is there too.
const DEBOUNCE: Duration = Duration::from_secs(4);
/// `updates.getState` again this often, so Telegram keeps pushing.
const KEEPALIVE: Duration = Duration::from_secs(10 * 60);

/// The `realtime` setting (on by default).
pub fn enabled() -> bool {
    super::setting("realtime").as_bool().unwrap_or(true)
}

pub fn set_enabled(on: bool) -> serde_json::Value {
    super::save_setting("realtime", json!(on));
    emit_state();
    state()
}

/// `{enabled, connected, last_event_at, triggered}`.
pub fn state() -> serde_json::Value {
    let last = LAST_EVENT_AT.load(Ordering::Relaxed);
    let t = LAST_RECEIVED_AT.load(Ordering::Relaxed);
    let last_received = (t > 0).then_some(t);
    json!({
        "enabled": enabled(),
        "connected": CONNECTED.load(Ordering::Relaxed),
        "last_event_at": (last > 0).then_some(last),
        "triggered": TRIGGERED.load(Ordering::Relaxed),
        "received": RECEIVED.load(Ordering::Relaxed),
        "last_received_at": last_received,
    })
}

fn emit_state() {
    events::emit(
        "telegram-realtime",
        json!({ "enabled": enabled(), "connected": CONNECTED.load(Ordering::Relaxed) }),
    );
}

fn set_connected(on: bool) {
    if CONNECTED.swap(on, Ordering::Relaxed) != on {
        emit_state();
    }
}

fn peer_id(peer: &tl::enums::Peer) -> i64 {
    match peer {
        tl::enums::Peer::User(u) => u.user_id,
        tl::enums::Peer::Chat(c) => c.chat_id,
        tl::enums::Peer::Channel(c) => c.channel_id,
    }
}

/// `(chat id, message id)` of every new message in `update`; a
/// `ChannelTooLong` counts as "something new in this channel" (id 0 means
/// "unknown, check it").
fn new_messages(update: &tl::enums::Update, out: &mut Vec<(i64, i32)>) {
    let msg = match update {
        tl::enums::Update::NewMessage(u) => &u.message,
        tl::enums::Update::NewChannelMessage(u) => &u.message,
        tl::enums::Update::ChannelTooLong(u) => {
            out.push((u.channel_id, i32::MAX));
            return;
        }
        _ => return,
    };
    if let tl::enums::Message::Message(m) = msg {
        if m.media.is_some() {
            out.push((peer_id(&m.peer_id), m.id));
        }
    }
}

fn collect(updates: &UpdatesLike) -> Vec<(i64, i32)> {
    let mut out = Vec::new();
    if let UpdatesLike::Updates(u) = updates {
        match u {
            tl::enums::Updates::UpdateShort(s) => new_messages(&s.update, &mut out),
            tl::enums::Updates::Combined(c) => {
                for up in &c.updates {
                    new_messages(up, &mut out);
                }
            }
            tl::enums::Updates::Updates(c) => {
                for up in &c.updates {
                    new_messages(up, &mut out);
                }
            }
            _ => {}
        }
    }
    out
}

fn pending() -> &'static Mutex<HashSet<String>> {
    static P: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Runs `rule_id` after the debounce, once per burst of messages.
fn schedule(handle: TelegramSessionHandle, active: super::ActiveDownloads, rule_id: String) {
    {
        let mut p = pending().lock().unwrap_or_else(|e| e.into_inner());
        if !p.insert(rule_id.clone()) {
            return;
        }
    }
    tokio::spawn(async move {
        tokio::time::sleep(DEBOUNCE).await;
        pending()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&rule_id);
        TRIGGERED.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = Box::pin(watch::run_rule_with(&handle, &active, &rule_id, "realtime")).await
        {
            tracing::debug!("[tg-realtime] rule {} failed: {}", rule_id, e);
        }
    });
}

/// Takes the update channel once the account is signed in (while a login
/// is running the QR flow still reads it).
async fn take_channel(
    handle: &TelegramSessionHandle,
) -> Option<(
    grammers_client::Client,
    mpsc::UnboundedReceiver<UpdatesLike>,
)> {
    let mut g = handle.lock().await;
    if g.phone.is_empty() || g.login_token.is_some() || g.password_token.is_some() {
        return None;
    }
    let client = g.client.clone()?;
    let rx = g.updates_rx.take()?;
    Some((client, rx))
}

async fn keepalive(client: &grammers_client::Client) {
    if let Err(e) = client.invoke(&tl::functions::updates::GetState {}).await {
        tracing::debug!("[tg-realtime] updates.getState failed: {}", e);
    }
}

/// Starts the loop once. It waits for a signed-in client, drains its
/// update channel for as long as it lives, and picks up the next one after
/// a new login.
pub fn spawn(handle: TelegramSessionHandle, active: super::ActiveDownloads) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        loop {
            let Some((client, mut rx)) = take_channel(&handle).await else {
                set_connected(false);
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            };
            keepalive(&client).await;
            set_connected(true);
            let mut last_keepalive = std::time::Instant::now();
            loop {
                let next = tokio::time::timeout(Duration::from_secs(30), rx.recv()).await;
                match next {
                    Ok(Some(updates)) => {
                        RECEIVED.fetch_add(1, Ordering::Relaxed);
                        LAST_RECEIVED_AT.store(super::now_secs(), Ordering::Relaxed);
                        if !enabled() {
                            continue;
                        }
                        for (chat_id, msg_id) in collect(&updates) {
                            let rules = watch::rules_for_new_message(chat_id, msg_id);
                            if rules.is_empty() {
                                continue;
                            }
                            LAST_EVENT_AT.store(super::now_secs(), Ordering::Relaxed);
                            for id in rules {
                                schedule(handle.clone(), active.clone(), id);
                            }
                        }
                    }
                    // The client went away (logout, account switch).
                    Ok(None) => break,
                    Err(_) => {
                        // A new login put a fresh channel in the state.
                        if handle.lock().await.updates_rx.is_some() {
                            break;
                        }
                        if last_keepalive.elapsed() >= KEEPALIVE {
                            keepalive(&client).await;
                            last_keepalive = std::time::Instant::now();
                        }
                    }
                }
            }
            set_connected(false);
        }
    });
}
