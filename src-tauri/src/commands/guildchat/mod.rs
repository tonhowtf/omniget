pub mod auth;
pub mod download;
pub mod export;
pub mod gateway;
pub mod model;
pub mod props;
pub mod read;
pub mod rest;
pub mod ws;
pub mod zlib;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::secrets::{self, Namespace};
use model::{Message, Snapshot, User};
use rest::Rest;

const STORE_DIR: &str = "discord";

const SECRETS: Namespace = Namespace {
    service: "wtf.tonho.omniget.discord",
    dir_name: "discord_secrets",
    dir_env: "OMNIGET_DISCORD_SECRET_DIR",
};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct AccountIndex {
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    accounts: Vec<Account>,
}

#[derive(Clone)]
struct Live {
    account_id: String,
    rest: Arc<Rest>,
    snapshot: Arc<RwLock<Snapshot>>,
    cmds: mpsc::UnboundedSender<gateway::Command>,
    cancel: CancellationToken,
}

#[derive(Default)]
struct Core {
    live: Mutex<Option<Live>>,
    connection: Mutex<Value>,
    qr: Mutex<Option<CancellationToken>>,
    jobs: Mutex<HashMap<String, CancellationToken>>,
    acks: Mutex<read::AckQueue>,
    index_lock: Mutex<()>,
}

fn core() -> &'static Core {
    static CORE: OnceLock<Core> = OnceLock::new();
    CORE.get_or_init(Core::default)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn data_dir() -> PathBuf {
    let dir = omniget_core::core::paths::app_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(STORE_DIR);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn index_path() -> PathBuf {
    data_dir().join("accounts.json")
}

fn load_index() -> AccountIndex {
    std::fs::read(index_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_index(idx: &AccountIndex) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(idx).map_err(|e| e.to_string())?;
    secrets::write_private(&index_path(), &bytes)
}

fn token_account(id: &str) -> String {
    format!("token:{id}")
}

fn load_token(id: &str) -> Result<Option<String>, String> {
    secrets::load_secret(SECRETS, &token_account(id))
}

fn store_token(id: &str, token: &str) -> Result<(), String> {
    secrets::save_secret(SECRETS, &token_account(id), token)
}

fn emit(app: &AppHandle, event: &str, payload: Value) {
    if event == "guildchat-status" {
        *lock(&core().connection) = payload.clone();
    }
    let _ = app.emit(event, payload);
}

fn live() -> Result<Live, String> {
    lock(&core().live)
        .clone()
        .ok_or_else(|| "not_connected".to_string())
}

fn stop_live() {
    if let Some(l) = lock(&core().live).take() {
        l.cancel.cancel();
    }
}

fn start_live(app: &AppHandle, account_id: &str, token: String) -> Result<(), String> {
    stop_live();
    let cancel = CancellationToken::new();
    let unauthorized_app = app.clone();
    let unauthorized_cancel = cancel.clone();
    let rest = Arc::new(Rest::new(
        &token,
        Box::new(move || {
            emit(
                &unauthorized_app,
                "guildchat-status",
                json!({ "state": "failed", "reason": "unauthorized" }),
            );
            unauthorized_cancel.cancel();
        }),
    )?);
    let snapshot = Arc::new(RwLock::new(Snapshot::default()));
    let (tx, rx) = mpsc::unbounded_channel();
    let emit_app = app.clone();
    let token_owner = account_id.to_string();
    let hooks = gateway::Hooks {
        emit: Arc::new(move |e, p| emit(&emit_app, e, p)),
        snapshot: snapshot.clone(),
        on_token: Arc::new(move |fresh| {
            if auth::looks_like_token(&fresh) {
                if let Err(e) = store_token(&token_owner, &fresh) {
                    tracing::warn!("[guildchat] could not store refreshed token: {}", e);
                }
            }
        }),
    };
    let live = Live {
        account_id: account_id.to_string(),
        rest: rest.clone(),
        snapshot,
        cmds: tx,
        cancel: cancel.clone(),
    };
    *lock(&core().live) = Some(live);
    tauri::async_runtime::spawn(gateway::run(token, hooks, rx, cancel.clone()));
    tauri::async_runtime::spawn(ack_loop(rest, cancel));
    Ok(())
}

async fn send_ack(rest: &Rest, channel: &str, message: &str) {
    let path = format!("/channels/{channel}/messages/{message}/ack");
    if let Err(e) = rest
        .request(Method::POST, &path, Some(json!({ "token": Value::Null })))
        .await
    {
        tracing::debug!("[guildchat] read ack failed: {}", e);
    }
}

async fn ack_loop(rest: Arc<Rest>, cancel: CancellationToken) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tick.tick() => {}
        }
        let due = lock(&core().acks).due(Instant::now());
        for (c, m) in due {
            send_ack(&rest, &c, &m).await;
        }
    }
}

async fn fetch_me(token: &str) -> Result<User, String> {
    let rest = Rest::new(token, Box::new(|| {}))?;
    let v = rest.get("/users/@me").await?;
    model::parse_user(&v).ok_or_else(|| "unexpected_response".to_string())
}

async fn adopt_token(app: &AppHandle, raw: &str) -> Result<Account, String> {
    let token = auth::clean_token(raw);
    if !auth::looks_like_token(&token) {
        return Err("token_format".into());
    }
    let me = fetch_me(&token).await?;
    if me.bot {
        return Err("bot_token".into());
    }
    store_token(&me.id, &token)?;
    let account = Account {
        id: me.id.clone(),
        username: me.username.clone(),
        global_name: me.global_name.clone(),
        avatar: me.avatar.clone(),
    };
    {
        let _g = lock(&core().index_lock);
        let mut idx = load_index();
        idx.accounts.retain(|a| a.id != account.id);
        idx.accounts.push(account.clone());
        idx.active = Some(account.id.clone());
        save_index(&idx)?;
    }
    start_live(app, &account.id, token)?;
    Ok(account)
}

#[derive(Serialize)]
pub struct Status {
    accounts: Vec<Account>,
    active: Option<String>,
    connected_account: Option<String>,
    connection: Value,
}

fn status_now() -> Status {
    let idx = load_index();
    let connected = lock(&core().live).as_ref().map(|l| l.account_id.clone());
    let connection = if connected.is_some() {
        lock(&core().connection).clone()
    } else {
        json!({ "state": "offline" })
    };
    Status {
        accounts: idx.accounts,
        active: idx.active,
        connected_account: connected,
        connection,
    }
}

#[tauri::command]
pub fn guildchat_status() -> Status {
    status_now()
}

#[tauri::command]
pub async fn guildchat_connect(app: AppHandle) -> Result<Status, String> {
    let idx = load_index();
    let Some(active) = idx.active.clone() else {
        return Ok(status_now());
    };
    let already = lock(&core().live)
        .as_ref()
        .is_some_and(|l| l.account_id == active && !l.cancel.is_cancelled());
    if !already {
        let token = load_token(&active)?.ok_or_else(|| "no_token".to_string())?;
        start_live(&app, &active, token)?;
    }
    Ok(status_now())
}

#[tauri::command]
pub fn guildchat_disconnect() -> Status {
    stop_live();
    status_now()
}

#[tauri::command]
pub async fn guildchat_login_token(app: AppHandle, token: String) -> Result<Account, String> {
    adopt_token(&app, &token).await
}

#[tauri::command]
pub fn guildchat_qr_start(app: AppHandle) -> Result<(), String> {
    let cancel = CancellationToken::new();
    if let Some(old) = lock(&core().qr).replace(cancel.clone()) {
        old.cancel();
    }
    tauri::async_runtime::spawn(async move {
        let events = app.clone();
        let (tx, mut rx) = mpsc::unbounded_channel::<auth::QrEvent>();
        let runner = tauri::async_runtime::spawn(auth::run_qr(cancel, move |ev| {
            let _ = tx.send(ev);
        }));
        while let Some(ev) = rx.recv().await {
            let payload = match ev {
                auth::QrEvent::Code { svg, url } => {
                    json!({ "stage": "code", "svg": svg, "url": url })
                }
                auth::QrEvent::Scanned(user) => json!({ "stage": "scanned", "user": user }),
                auth::QrEvent::Cancelled => json!({ "stage": "cancelled" }),
                auth::QrEvent::Expired => json!({ "stage": "expired" }),
                auth::QrEvent::Failed(e) => json!({ "stage": "error", "error": e }),
                auth::QrEvent::Token(token) => match adopt_token(&events, &token).await {
                    Ok(account) => json!({ "stage": "done", "account": account }),
                    Err(e) => json!({ "stage": "error", "error": e }),
                },
            };
            let _ = events.emit("guildchat-qr", payload);
        }
        let _ = runner.await;
    });
    Ok(())
}

const BROWSER_TOKEN: &str = r#"(()=>{
if(location.pathname.startsWith('/login')||location.pathname.startsWith('/register'))return null;
try{const f=document.createElement('iframe');f.style.display='none';document.body.appendChild(f);const t=f.contentWindow.localStorage.getItem('token');f.remove();if(t)return t;}catch(e){}
try{const c=window.webpackChunkdiscord_app;if(c){let tok=null;c.push([[Symbol()],{},r=>{for(const m of Object.values(r.c||{})){const e=m&&m.exports;if(!e)continue;for(const k of Object.keys(e)){const x=e[k];if(x&&typeof x.getToken==='function'){try{tok=x.getToken()}catch(_){}}if(tok)break}if(tok)break}}]);if(tok)return tok;}}catch(e){}
return null;})()"#;

async fn browser_token() -> Option<String> {
    use omniget_core::core::global_dl::browser;
    let b = browser::current()?;
    let tab = b.tab_for_origin("https://discord.com/")?;
    let v = b
        .eval(&tab.id, BROWSER_TOKEN, Duration::from_secs(10))
        .await
        .ok()?;
    let t = auth::clean_token(v.as_str()?);
    auth::looks_like_token(&t).then_some(t)
}

#[tauri::command]
pub async fn guildchat_browser_login(app: AppHandle) -> Result<(), String> {
    let cancel = CancellationToken::new();
    if let Some(old) = lock(&core().qr).replace(cancel.clone()) {
        old.cancel();
    }
    crate::commands::global_dl::gd_browser_show(Some("https://discord.com/login".into()), None)
        .await?;
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        while !cancel.is_cancelled() && started.elapsed() < Duration::from_secs(900) {
            if let Some(token) = browser_token().await {
                let payload = match adopt_token(&app, &token).await {
                    Ok(account) => json!({ "stage": "done", "account": account }),
                    Err(e) => json!({ "stage": "error", "error": e }),
                };
                let _ = app.emit("guildchat-qr", payload);
                return;
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
        if !cancel.is_cancelled() {
            let _ = app.emit("guildchat-qr", json!({ "stage": "expired" }));
        }
    });
    Ok(())
}

#[tauri::command]
pub fn guildchat_qr_cancel() {
    if let Some(c) = lock(&core().qr).take() {
        c.cancel();
    }
}

#[tauri::command]
pub fn guildchat_logout(account_id: String) -> Result<Status, String> {
    if lock(&core().live)
        .as_ref()
        .is_some_and(|l| l.account_id == account_id)
    {
        stop_live();
    }
    secrets::delete_secret(SECRETS, &token_account(&account_id))?;
    let _g = lock(&core().index_lock);
    let mut idx = load_index();
    idx.accounts.retain(|a| a.id != account_id);
    if idx.active.as_deref() == Some(account_id.as_str()) {
        idx.active = idx.accounts.first().map(|a| a.id.clone());
    }
    save_index(&idx)?;
    drop(_g);
    Ok(status_now())
}

#[tauri::command]
pub async fn guildchat_switch_account(
    app: AppHandle,
    account_id: String,
) -> Result<Status, String> {
    {
        let _g = lock(&core().index_lock);
        let mut idx = load_index();
        if !idx.accounts.iter().any(|a| a.id == account_id) {
            return Err("unknown_account".into());
        }
        idx.active = Some(account_id.clone());
        save_index(&idx)?;
    }
    stop_live();
    guildchat_connect(app).await
}

#[tauri::command]
pub fn guildchat_snapshot() -> Option<Snapshot> {
    let l = lock(&core().live).clone()?;
    let snap = l.snapshot.read().ok()?.clone();
    if snap.me.id.is_empty() {
        None
    } else {
        Some(snap)
    }
}

fn check_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 24 || !id.bytes().all(|b| b.is_ascii_digit()) {
        Err("bad_id".into())
    } else {
        Ok(())
    }
}

fn subscribe(l: &Live, channel_id: &str) {
    let guild = l
        .snapshot
        .read()
        .ok()
        .and_then(|s| s.channel(channel_id).and_then(|c| c.guild_id.clone()));
    if let Some(guild_id) = guild {
        let _ = l.cmds.send(gateway::Command::Subscribe {
            guild_id,
            channel_id: channel_id.to_string(),
        });
    }
}

#[tauri::command]
pub async fn guildchat_messages(
    channel_id: String,
    before: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<Message>, String> {
    check_id(&channel_id)?;
    let l = live()?;
    let limit = limit.unwrap_or(50).clamp(1, 100);
    let mut path = format!("/channels/{channel_id}/messages?limit={limit}");
    if let Some(b) = before.filter(|b| !b.is_empty()) {
        check_id(&b)?;
        path.push_str(&format!("&before={b}"));
    } else {
        subscribe(&l, &channel_id);
    }
    let v = l.rest.get(&path).await?;
    Ok(Message::list_from(&v))
}

fn now_nonce() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    model::snowflake_at(ms).to_string()
}

pub fn check_content(content: &str) -> Result<String, String> {
    let c = content.trim_end().to_string();
    if c.trim().is_empty() {
        return Err("empty".into());
    }
    if c.chars().count() > 2000 {
        return Err("too_long".into());
    }
    Ok(c)
}

#[tauri::command]
pub async fn guildchat_send(
    channel_id: String,
    content: String,
    reply_to: Option<String>,
) -> Result<Message, String> {
    check_id(&channel_id)?;
    let content = check_content(&content)?;
    let l = live()?;
    let mut body = json!({
        "content": content,
        "nonce": now_nonce(),
        "tts": false,
        "flags": 0,
    });
    if let Some(r) = reply_to.filter(|r| !r.is_empty()) {
        check_id(&r)?;
        let guild = l
            .snapshot
            .read()
            .ok()
            .and_then(|s| s.channel(&channel_id).and_then(|c| c.guild_id.clone()));
        body["message_reference"] =
            json!({ "message_id": r, "channel_id": channel_id, "guild_id": guild });
    }
    let v = l
        .rest
        .request(
            Method::POST,
            &format!("/channels/{channel_id}/messages"),
            Some(body),
        )
        .await?;
    Message::from_value(&v).ok_or_else(|| "unexpected_response".into())
}

#[tauri::command]
pub async fn guildchat_edit(
    channel_id: String,
    message_id: String,
    content: String,
) -> Result<Message, String> {
    check_id(&channel_id)?;
    check_id(&message_id)?;
    let content = check_content(&content)?;
    let l = live()?;
    let v = l
        .rest
        .request(
            Method::PATCH,
            &format!("/channels/{channel_id}/messages/{message_id}"),
            Some(json!({ "content": content })),
        )
        .await?;
    Message::from_value(&v).ok_or_else(|| "unexpected_response".into())
}

#[tauri::command]
pub async fn guildchat_delete(channel_id: String, message_id: String) -> Result<(), String> {
    check_id(&channel_id)?;
    check_id(&message_id)?;
    let l = live()?;
    l.rest
        .request(
            Method::DELETE,
            &format!("/channels/{channel_id}/messages/{message_id}"),
            None,
        )
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn guildchat_ack(channel_id: String, message_id: String) -> Result<(), String> {
    check_id(&channel_id)?;
    check_id(&message_id)?;
    let l = live()?;
    if let Ok(mut s) = l.snapshot.write() {
        let e = s.read_states.entry(channel_id.clone()).or_default();
        e.last_message_id = Some(message_id.clone());
        e.mention_count = 0;
    }
    let now = lock(&core().acks).offer(&channel_id, &message_id, Instant::now());
    if let Some(m) = now {
        send_ack(&l.rest, &channel_id, &m).await;
    }
    Ok(())
}

fn output_root(app: &AppHandle, dir: Option<String>) -> PathBuf {
    match dir.filter(|d| !d.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            crate::storage::config::load_settings(app)
                .download
                .default_output_dir
        }
    }
}

fn begin_job(job_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    if let Some(old) = lock(&core().jobs).insert(job_id.to_string(), token.clone()) {
        old.cancel();
    }
    token
}

fn end_job(job_id: &str) {
    lock(&core().jobs).remove(job_id);
}

#[tauri::command]
pub fn guildchat_cancel(job_id: String) {
    if let Some(t) = lock(&core().jobs).remove(&job_id) {
        t.cancel();
    }
}

#[derive(Serialize)]
pub struct Saved {
    folder: String,
    paths: Vec<String>,
    failed: usize,
    skipped: usize,
    cancelled: bool,
}

fn titles(l: &Live, channel_id: &str) -> (String, String) {
    l.snapshot
        .read()
        .map(|s| s.channel_title(channel_id))
        .unwrap_or_else(|_| (model::STORE_LABEL.into(), channel_id.to_string()))
}

#[tauri::command]
pub async fn guildchat_download_message(
    app: AppHandle,
    channel_id: String,
    message_id: String,
    dir: Option<String>,
) -> Result<Saved, String> {
    check_id(&channel_id)?;
    check_id(&message_id)?;
    let l = live()?;
    let v = l
        .rest
        .get(&format!(
            "/channels/{channel_id}/messages?limit=1&around={message_id}"
        ))
        .await?;
    let msgs: Vec<Message> = Message::list_from(&v)
        .into_iter()
        .filter(|m| m.id == message_id)
        .collect();
    if msgs.is_empty() {
        return Err("not_found:message".into());
    }
    let (place, channel) = titles(&l, &channel_id);
    let folder = download::channel_folder(&output_root(&app, dir), &place, &channel);
    let filter = download::MediaFilter {
        include_embeds: true,
        ..Default::default()
    };
    let plan = download::plan(&msgs, &filter, &folder);
    if plan.is_empty() {
        return Err("no_media".into());
    }
    let progress = Arc::new(download::Progress::new());
    let cancel = CancellationToken::new();
    let paths = download::save_items(l.rest.cdn(), plan, &progress, &cancel, &|| {}).await;
    Ok(Saved {
        folder: folder.to_string_lossy().into_owned(),
        paths: paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        failed: progress.failed.load(std::sync::atomic::Ordering::Relaxed),
        skipped: progress.skipped.load(std::sync::atomic::Ordering::Relaxed),
        cancelled: false,
    })
}

#[tauri::command]
pub async fn guildchat_download_channel(
    app: AppHandle,
    job_id: String,
    channel_id: String,
    dir: Option<String>,
    filter: download::MediaFilter,
) -> Result<Saved, String> {
    check_id(&channel_id)?;
    let l = live()?;
    let cancel = begin_job(&job_id);
    let (place, channel) = titles(&l, &channel_id);
    let folder = download::channel_folder(&output_root(&app, dir), &place, &channel);
    let progress = Arc::new(download::Progress::new());
    let last_emit = Mutex::new(Instant::now() - Duration::from_secs(1));
    let tick = |phase: &str, force: bool| {
        let mut at = lock(&last_emit);
        if force || at.elapsed() >= Duration::from_millis(250) {
            *at = Instant::now();
            let _ = app.emit("guildchat-download-progress", progress.json(&job_id, phase));
        }
    };
    tick("scan", true);
    let mut queue: Vec<(model::MediaItem, PathBuf)> = Vec::new();
    let walk = download::walk_history(
        &l.rest,
        &channel_id,
        filter.after_ms,
        filter.before_ms,
        &cancel,
        |page| {
            progress
                .scanned
                .fetch_add(page.len(), std::sync::atomic::Ordering::Relaxed);
            let items = download::plan(&page, &filter, &folder);
            progress
                .found
                .fetch_add(items.len(), std::sync::atomic::Ordering::Relaxed);
            queue.extend(items);
            tick("scan", false);
        },
    )
    .await;
    if let Err(e) = walk {
        end_job(&job_id);
        let _ = app.emit(
            "guildchat-download-progress",
            json!({ "job_id": job_id, "phase": "error", "error": e }),
        );
        return Err(e);
    }
    let paths = if cancel.is_cancelled() {
        Vec::new()
    } else {
        tick("download", true);
        let step = || tick("download", false);
        download::save_items(l.rest.cdn(), queue, &progress, &cancel, &step).await
    };
    let cancelled = cancel.is_cancelled();
    end_job(&job_id);
    tick(if cancelled { "cancelled" } else { "done" }, true);
    Ok(Saved {
        folder: folder.to_string_lossy().into_owned(),
        paths: paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        failed: progress.failed.load(std::sync::atomic::Ordering::Relaxed),
        skipped: progress.skipped.load(std::sync::atomic::Ordering::Relaxed),
        cancelled,
    })
}

#[derive(Deserialize)]
pub struct ExportOptions {
    #[serde(default)]
    formats: Vec<String>,
    #[serde(default)]
    after_ms: Option<u64>,
    #[serde(default)]
    before_ms: Option<u64>,
    #[serde(default)]
    include_media: bool,
}

#[derive(Serialize)]
pub struct Exported {
    folder: String,
    files: Vec<String>,
    messages: usize,
    cancelled: bool,
}

fn write_file(path: &Path, text: &[u8]) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn guildchat_export(
    app: AppHandle,
    job_id: String,
    channel_id: String,
    dir: Option<String>,
    options: ExportOptions,
) -> Result<Exported, String> {
    check_id(&channel_id)?;
    let l = live()?;
    let mut formats: Vec<String> = options
        .formats
        .iter()
        .filter(|f| matches!(f.as_str(), "json" | "html" | "txt"))
        .cloned()
        .collect();
    if formats.is_empty() {
        formats = vec!["html".into(), "json".into()];
    }
    let cancel = begin_job(&job_id);
    let (place, channel) = titles(&l, &channel_id);
    let folder = download::channel_folder(&output_root(&app, dir), &place, &channel);
    let mut msgs: Vec<Message> = Vec::new();
    let mut last = Instant::now();
    let walk = download::walk_history(
        &l.rest,
        &channel_id,
        options.after_ms,
        options.before_ms,
        &cancel,
        |page| {
            msgs.extend(page);
            if last.elapsed() >= Duration::from_millis(250) {
                last = Instant::now();
                let _ = app.emit(
                    "guildchat-export-progress",
                    json!({ "job_id": job_id, "phase": "scan", "scanned": msgs.len() }),
                );
            }
        },
    )
    .await;
    if let Err(e) = walk {
        end_job(&job_id);
        let _ = app.emit(
            "guildchat-export-progress",
            json!({ "job_id": job_id, "phase": "error", "error": e }),
        );
        return Err(e);
    }
    if cancel.is_cancelled() {
        end_job(&job_id);
        let _ = app.emit(
            "guildchat-export-progress",
            json!({ "job_id": job_id, "phase": "cancelled", "scanned": msgs.len() }),
        );
        return Ok(Exported {
            folder: folder.to_string_lossy().into_owned(),
            files: Vec::new(),
            messages: msgs.len(),
            cancelled: true,
        });
    }
    msgs.reverse();
    let date = chrono::Local::now();
    let base = format!(
        "{} - {} - {}",
        download::clean_segment(&place, model::STORE_LABEL),
        download::clean_segment(&channel, "channel"),
        date.format("%Y-%m-%d_%H-%M")
    );
    let mut local: HashMap<String, String> = HashMap::new();
    if options.include_media {
        let media_dir = folder.join(format!("{base}_files"));
        let filter = download::MediaFilter {
            include_embeds: false,
            ..Default::default()
        };
        let plan = download::plan(&msgs, &filter, &media_dir);
        let progress = Arc::new(download::Progress::new());
        progress
            .found
            .store(plan.len(), std::sync::atomic::Ordering::Relaxed);
        let urls: Vec<(String, PathBuf)> = plan
            .iter()
            .map(|(i, p)| (i.url.clone(), p.clone()))
            .collect();
        let step = || {
            let _ = app.emit("guildchat-export-progress", progress.json(&job_id, "media"));
        };
        let saved = download::save_items(l.rest.cdn(), plan, &progress, &cancel, &step).await;
        for (url, path) in urls {
            if saved.contains(&path) {
                if let (Some(dir_name), Some(file)) = (media_dir.file_name(), path.file_name()) {
                    local.insert(
                        url,
                        format!("{}/{}", dir_name.to_string_lossy(), file.to_string_lossy()),
                    );
                }
            }
        }
    }
    let at = date.format("%Y-%m-%d %H:%M").to_string();
    let meta = export::Meta {
        place: &place,
        channel: &channel,
        exported_at: &at,
    };
    let offset = *date.offset();
    let channel_info = l
        .snapshot
        .read()
        .ok()
        .and_then(|s| s.channel(&channel_id).cloned());
    let mut files = Vec::new();
    for f in &formats {
        let path = folder.join(format!("{base}.{f}"));
        let bytes = match f.as_str() {
            "json" => {
                serde_json::to_vec_pretty(&export::to_json(&meta, channel_info.as_ref(), &msgs))
                    .map_err(|e| e.to_string())?
            }
            "txt" => export::to_txt(&meta, &msgs, &offset, &local).into_bytes(),
            _ => export::to_html(&meta, &msgs, &offset, &local).into_bytes(),
        };
        write_file(&path, &bytes)?;
        files.push(path.to_string_lossy().into_owned());
    }
    end_job(&job_id);
    let _ = app.emit(
        "guildchat-export-progress",
        json!({ "job_id": job_id, "phase": "done", "scanned": msgs.len() }),
    );
    Ok(Exported {
        folder: folder.to_string_lossy().into_owned(),
        files,
        messages: msgs.len(),
        cancelled: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_rules() {
        assert_eq!(check_content("hi  \n").unwrap(), "hi");
        assert_eq!(check_content("   ").unwrap_err(), "empty");
        assert_eq!(check_content(&"a".repeat(2001)).unwrap_err(), "too_long");
        assert!(check_content(&"é".repeat(2000)).is_ok());
    }

    #[test]
    fn ids_are_digits_only() {
        assert!(check_id("123").is_ok());
        assert!(check_id("").is_err());
        assert!(check_id("12/../x").is_err());
        assert!(check_id(&"1".repeat(30)).is_err());
    }

    #[test]
    fn nonce_is_a_current_snowflake() {
        let n = now_nonce();
        let ms = model::snowflake_ms(&n).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(now.abs_diff(ms) < 5_000);
    }
}
