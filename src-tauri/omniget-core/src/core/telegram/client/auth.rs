use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dashmap::DashMap;
use grammers_client::grammers_tl_types as tl;
use grammers_client::types::{LoginToken, PasswordToken};
use grammers_client::{Client, SignInError};
use grammers_mtsender::{ConnectionParams, InvocationError, SenderPool};
use grammers_session::storages::SqliteSession;
use grammers_session::updates::UpdatesLike;
use grammers_session::Session;
use qrcode::render::svg;
use qrcode::QrCode;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

pub type OpTokenMap = Arc<DashMap<String, CancellationToken>>;

pub fn register_op_token(map: &OpTokenMap, op_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    map.insert(op_id.to_string(), token.clone());
    token
}

pub fn unregister_op_token(map: &OpTokenMap, op_id: &str) {
    map.remove(op_id);
}

pub fn cancel_op_token(map: &OpTokenMap, op_id: &str) -> bool {
    if let Some((_, token)) = map.remove(op_id) {
        token.cancel();
        true
    } else {
        false
    }
}

#[derive(Debug, Clone)]
pub struct CachedMediaMeta {
    pub filename: String,
    pub mime: String,
    pub size: u64,
    pub fetched_at: Instant,
}

const META_MAX_ENTRIES: usize = 5000;

pub type MetadataCache = Arc<DashMap<(i64, i32), CachedMediaMeta>>;

pub fn metadata_put(cache: &MetadataCache, chat_id: i64, message_id: i32, meta: CachedMediaMeta) {
    if cache.len() >= META_MAX_ENTRIES {
        let mut oldest_key: Option<(i64, i32)> = None;
        let mut oldest_at: Option<Instant> = None;
        for r in cache.iter() {
            let at = r.value().fetched_at;
            if oldest_at.map(|o| at < o).unwrap_or(true) {
                oldest_at = Some(at);
                oldest_key = Some(*r.key());
            }
        }
        if let Some(k) = oldest_key {
            cache.remove(&k);
        }
    }
    cache.insert((chat_id, message_id), meta);
}

const API_ID: i32 = 15055931;
const API_HASH: &str = "021d433426cbb920eeb95164498fe3d3";

pub type TelegramSessionHandle = Arc<Mutex<TelegramState>>;

pub struct TelegramState {
    pub client: Option<Client>,
    pub phone: String,
    pub login_token: Option<LoginToken>,
    pub password_token: Option<PasswordToken>,
    pub updates_rx: Option<mpsc::UnboundedReceiver<UpdatesLike>>,
    pub peer_hashes: HashMap<i64, i64>,
    pub peer_photo_ids: HashMap<i64, i64>,
    pub metadata_cache: MetadataCache,
    pub takeout_session_id: Option<i64>,
    pub op_cancellation_tokens: OpTokenMap,
}

impl Default for TelegramState {
    fn default() -> Self {
        Self::new()
    }
}

impl TelegramState {
    pub fn new() -> Self {
        Self {
            client: None,
            phone: String::new(),
            login_token: None,
            password_token: None,
            updates_rx: None,
            peer_hashes: HashMap::new(),
            peer_photo_ids: HashMap::new(),
            metadata_cache: Arc::new(DashMap::new()),
            takeout_session_id: None,
            op_cancellation_tokens: Arc::new(DashMap::new()),
        }
    }
}

pub fn session_file_path() -> anyhow::Result<PathBuf> {
    super::migrate_legacy_data();
    Ok(crate::core::telegram::host::data_dir().join("telegram.session"))
}

fn open_session() -> anyhow::Result<Arc<SqliteSession>> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let session = SqliteSession::open(path)?;
    Ok(Arc::new(session))
}

pub fn current_home_dc_id() -> Option<i32> {
    let session = open_session().ok()?;
    let dc = session.home_dc_id();
    if dc <= 0 {
        None
    } else {
        Some(dc)
    }
}

/// Appended to every network failure: the Connection tab is where the
/// person picks another way out.
pub const NET_HINT: &str = " Open the Connection tab to pick another route.";

fn is_connection_error(e: &InvocationError) -> bool {
    matches!(
        e,
        InvocationError::Io(_)
            | InvocationError::Transport(_)
            | InvocationError::Dropped
            | InvocationError::InvalidDc
    )
}

pub fn describe_invocation_error(context: &str, e: &InvocationError) -> anyhow::Error {
    if is_connection_error(e) {
        anyhow!("{}: {}.{}", context, e, NET_HINT)
    } else {
        anyhow!("{}: {}", context, e)
    }
}

/// Unix seconds of the last pre-connect scan, so a dead network does not
/// trigger a scan on every single connect.
static LAST_PRECONNECT_SCAN: AtomicI64 = AtomicI64::new(0);
const PRECONNECT_SCAN_TIMEOUT: Duration = Duration::from_secs(25);

/// Before the first connect, scan so a route is already chosen. Called
/// outside the per-call timeouts so the scan does not eat into them.
pub async fn ensure_route() {
    let mgr = crate::core::telegram::net::manager::get();
    if mgr.has_working_route() {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let last = LAST_PRECONNECT_SCAN.load(Ordering::Relaxed);
    if now - last <= 60 {
        return;
    }
    LAST_PRECONNECT_SCAN.store(now, Ordering::Relaxed);
    tracing::info!("[tg-net] no proven route yet, scanning before connecting");
    if tokio::time::timeout(PRECONNECT_SCAN_TIMEOUT, mgr.scan())
        .await
        .is_err()
    {
        tracing::warn!(
            "[tg-net] pre-connect scan timed out after {}s",
            PRECONNECT_SCAN_TIMEOUT.as_secs()
        );
    }
}

/// Every MTProto connection goes through the local bridge of the connection
/// layer, which picks the route (direct, MTProxy, Shadowsocks, …) and swaps
/// it on failure.
async fn bridge_proxy_url() -> anyhow::Result<String> {
    ensure_route().await;
    crate::core::telegram::net::manager::get()
        .bridge_url()
        .await
        .map_err(|e| {
            anyhow!(
                "Cannot start the Telegram connection bridge: {e:#}.{}",
                NET_HINT
            )
        })
}

async fn connection_params() -> anyhow::Result<ConnectionParams> {
    let os_version = os_info::get();
    let proxy_url = bridge_proxy_url().await?;
    Ok(ConnectionParams {
        device_model: "Desktop".to_string(),
        system_version: format!("{} {}", os_version.os_type(), os_version.version()),
        app_version: format!("{} x64", env!("CARGO_PKG_VERSION")),
        system_lang_code: sys_locale::get_locale()
            .unwrap_or_else(|| "en".to_string())
            .split('-')
            .next()
            .unwrap_or("en")
            .to_string(),
        lang_code: "en".to_string(),
        proxy_url: Some(proxy_url),
        ..ConnectionParams::default()
    })
}

async fn create_client_with_updates(
) -> anyhow::Result<(Client, mpsc::UnboundedReceiver<UpdatesLike>)> {
    let params = connection_params().await?;
    let session = open_session()?;
    let pool = SenderPool::with_configuration(session, API_ID, params);
    let client = Client::new(&pool);
    let updates_rx = pool.updates;
    tokio::spawn(Box::pin(pool.runner.run()));
    Ok((client, updates_rx))
}

pub async fn delete_session() -> anyhow::Result<()> {
    let path = session_file_path()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub async fn check_session(handle: &TelegramSessionHandle) -> anyhow::Result<String> {
    ensure_route().await;
    let check = async {
        let (existing_client, existing_phone) = {
            let guard = handle.lock().await;
            (guard.client.clone(), guard.phone.clone())
        };

        if let Some(client) = existing_client {
            if client.is_authorized().await? {
                return Ok(existing_phone);
            }
        }

        // Keep the update channel: the real-time auto-save reads it.
        let (client, updates_rx) = create_client_with_updates().await?;
        if client.is_authorized().await? {
            let me = client.get_me().await?;
            let phone = me.phone().unwrap_or("").to_string();
            let mut guard = handle.lock().await;
            guard.client = Some(client);
            guard.updates_rx = Some(updates_rx);
            guard.phone = phone.clone();
            return Ok(phone);
        }

        Err(anyhow!("not_authenticated"))
    };

    tokio::time::timeout(std::time::Duration::from_secs(15), check)
        .await
        .map_err(|_| anyhow!("Session check timed out — please try again.{}", NET_HINT))?
}

const QR_NETWORK_TIMEOUT: Duration = Duration::from_secs(20);

fn qr_timeout_error() -> anyhow::Error {
    anyhow!(
        "Cannot reach Telegram (timed out after {}s).{}",
        QR_NETWORK_TIMEOUT.as_secs(),
        NET_HINT
    )
}

pub struct QrLoginResult {
    pub qr_svg: String,
    pub expires: i32,
}

fn token_to_qr(token_data: &tl::types::auth::LoginToken) -> anyhow::Result<QrLoginResult> {
    let token_b64 = URL_SAFE_NO_PAD.encode(&token_data.token);
    let qr_url = format!("tg://login?token={}", token_b64);

    let qr =
        QrCode::new(qr_url.as_bytes()).map_err(|e| anyhow!("Failed to generate QR code: {}", e))?;
    let qr_svg = qr
        .render::<svg::Color>()
        .min_dimensions(200, 200)
        .quiet_zone(false)
        .build();

    Ok(QrLoginResult {
        qr_svg,
        expires: token_data.expires,
    })
}

async fn export_login_token(client: &Client) -> anyhow::Result<tl::enums::auth::LoginToken> {
    let request = tl::functions::auth::ExportLoginToken {
        api_id: API_ID,
        api_hash: API_HASH.to_string(),
        except_ids: vec![],
    };
    client
        .invoke(&request)
        .await
        .map_err(|e| describe_invocation_error("Failed to export login token", &e))
}

fn drain_for_login_token(updates_rx: &mut mpsc::UnboundedReceiver<UpdatesLike>) -> bool {
    let mut found = false;
    let mut count = 0;
    while let Ok(update) = updates_rx.try_recv() {
        count += 1;
        let is_login = has_login_token_update(&update);
        tracing::info!(
            "[tg-qr] drained update #{}, is_login_token={}",
            count,
            is_login
        );
        if is_login {
            found = true;
        }
    }
    found
}

fn has_login_token_update(updates_like: &UpdatesLike) -> bool {
    match updates_like {
        UpdatesLike::Updates(tl::enums::Updates::UpdateShort(short)) => {
            matches!(short.update, tl::enums::Update::LoginToken)
        }
        UpdatesLike::Updates(tl::enums::Updates::Updates(updates)) => updates
            .updates
            .iter()
            .any(|u| matches!(u, tl::enums::Update::LoginToken)),
        UpdatesLike::Updates(tl::enums::Updates::Combined(combined)) => combined
            .updates
            .iter()
            .any(|u| matches!(u, tl::enums::Update::LoginToken)),
        _ => false,
    }
}

enum MigrateResult {
    NewToken(tl::types::auth::LoginToken),
    Success { phone: String },
    PasswordRequired { hint: String },
}

async fn handle_migrate(
    handle: &TelegramSessionHandle,
    old_client: &Client,
    migrate: tl::types::auth::LoginTokenMigrateTo,
) -> anyhow::Result<MigrateResult> {
    tracing::info!("[tg-qr] handling DC migration to DC {}", migrate.dc_id);
    old_client.disconnect();

    let session = open_session()?;
    session.set_home_dc_id(migrate.dc_id);

    let (new_client, new_updates_rx) = create_client_with_updates().await?;

    let mut guard = handle.lock().await;
    guard.client = Some(new_client.clone());
    guard.updates_rx = Some(new_updates_rx);
    drop(guard);

    let import_req = tl::functions::auth::ImportLoginToken {
        token: migrate.token,
    };

    match new_client.invoke(&import_req).await {
        Ok(tl::enums::auth::LoginToken::Token(token_data)) => {
            Ok(MigrateResult::NewToken(token_data))
        }
        Ok(tl::enums::auth::LoginToken::Success(_)) => {
            let me = new_client.get_me().await?;
            let phone = me.phone().unwrap_or("").to_string();
            let mut guard = handle.lock().await;
            guard.phone = phone.clone();
            Ok(MigrateResult::Success { phone })
        }
        Ok(tl::enums::auth::LoginToken::MigrateTo(_)) => Err(anyhow!("Multiple DC migrations")),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("SESSION_PASSWORD_NEEDED") {
                tracing::info!("[tg-qr] SESSION_PASSWORD_NEEDED after DC migration");
                let password: tl::types::account::Password = new_client
                    .invoke(&tl::functions::account::GetPassword {})
                    .await
                    .map_err(|e| anyhow!("Failed to get password info: {}", e))?
                    .into();
                let hint = password.hint.clone().unwrap_or_default();
                let pw_token = PasswordToken::new(password);
                let mut guard = handle.lock().await;
                guard.password_token = Some(pw_token);
                Ok(MigrateResult::PasswordRequired { hint })
            } else {
                Err(anyhow!("Import after DC migration failed: {}", err_str))
            }
        }
    }
}

pub async fn qr_login_start(handle: &TelegramSessionHandle) -> anyhow::Result<QrLoginResult> {
    // Mirror check_session: bound the network-bound work so the invoke always
    // resolves even when Telegram is unreachable (e.g. behind a proxy).
    ensure_route().await;
    let start = qr_login_start_inner(handle);
    tokio::time::timeout(QR_NETWORK_TIMEOUT, start)
        .await
        .map_err(|_| qr_timeout_error())?
}

async fn qr_login_start_inner(handle: &TelegramSessionHandle) -> anyhow::Result<QrLoginResult> {
    tracing::info!("[tg-qr] qr_login_start: creating client");
    let existing_client = {
        let guard = handle.lock().await;
        guard.client.clone()
    };

    if let Some(client) = existing_client {
        if client.is_authorized().await.unwrap_or(false) {
            let phone = client
                .get_me()
                .await
                .ok()
                .and_then(|me| me.phone().map(str::to_string))
                .unwrap_or_default();
            let mut guard = handle.lock().await;
            if !phone.is_empty() {
                guard.phone = phone;
            }
            return Err(anyhow!("already_authenticated"));
        }
    }

    let mut guard = handle.lock().await;
    if let Some(old_client) = guard.client.take() {
        old_client.disconnect();
    }
    guard.updates_rx = None;

    let (client, updates_rx) = create_client_with_updates().await?;
    guard.client = Some(client.clone());
    guard.updates_rx = Some(updates_rx);
    drop(guard);

    tracing::info!("[tg-qr] qr_login_start: calling ExportLoginToken");
    let result = export_login_token(&client).await?;

    match result {
        tl::enums::auth::LoginToken::Token(token_data) => token_to_qr(&token_data),
        tl::enums::auth::LoginToken::Success(_) => {
            let me = client.get_me().await?;
            let phone = me.phone().unwrap_or("").to_string();
            let mut guard = handle.lock().await;
            guard.phone = phone;
            Err(anyhow!("already_authenticated"))
        }
        tl::enums::auth::LoginToken::MigrateTo(migrate) => {
            match handle_migrate(handle, &client, migrate).await? {
                MigrateResult::NewToken(token_data) => token_to_qr(&token_data),
                MigrateResult::Success { .. } => Err(anyhow!("already_authenticated")),
                MigrateResult::PasswordRequired { .. } => Err(anyhow!("already_authenticated")),
            }
        }
    }
}

pub enum QrPollStatus {
    Waiting,
    Success { phone: String },
    PasswordRequired { hint: String },
}

pub async fn qr_login_poll(handle: &TelegramSessionHandle) -> anyhow::Result<QrPollStatus> {
    let mut guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow!("No active client"))?
        .clone();

    let got_update = if let Some(ref mut rx) = guard.updates_rx {
        drain_for_login_token(rx)
    } else {
        false
    };
    drop(guard);

    if !got_update {
        return Ok(QrPollStatus::Waiting);
    }

    tracing::info!("[tg-qr] updateLoginToken received, calling ExportLoginToken to finalize");

    // Mirror check_session: bound the network-bound finalization so the
    // invoke always resolves even when Telegram is unreachable.
    let finalize = qr_login_poll_finalize(handle, &client);
    tokio::time::timeout(QR_NETWORK_TIMEOUT, finalize)
        .await
        .map_err(|_| qr_timeout_error())?
}

async fn qr_login_poll_finalize(
    handle: &TelegramSessionHandle,
    client: &Client,
) -> anyhow::Result<QrPollStatus> {
    match export_login_token(client).await {
        Ok(tl::enums::auth::LoginToken::Success(success)) => {
            let phone = extract_phone_from_auth(&success.authorization);
            tracing::info!("[tg-qr] login success, phone={}", phone);
            let mut guard = handle.lock().await;
            guard.phone = phone.clone();
            // The update channel stays: the real-time auto-save reads it.
            Ok(QrPollStatus::Success { phone })
        }
        Ok(tl::enums::auth::LoginToken::Token(_)) => {
            tracing::info!("[tg-qr] got new token after update (not success yet), waiting");
            Ok(QrPollStatus::Waiting)
        }
        Ok(tl::enums::auth::LoginToken::MigrateTo(migrate)) => {
            tracing::info!("[tg-qr] got MigrateTo after update, migrating DC");
            match handle_migrate(handle, client, migrate).await? {
                MigrateResult::NewToken(_) => {
                    tracing::info!("[tg-qr] migration gave new token, waiting");
                    Ok(QrPollStatus::Waiting)
                }
                MigrateResult::Success { phone } => {
                    tracing::info!("[tg-qr] migration + login success, phone={}", phone);
                    Ok(QrPollStatus::Success { phone })
                }
                MigrateResult::PasswordRequired { hint } => {
                    tracing::info!("[tg-qr] migration → 2FA required");
                    Ok(QrPollStatus::PasswordRequired { hint })
                }
            }
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("SESSION_PASSWORD_NEEDED") {
                tracing::info!("[tg-qr] SESSION_PASSWORD_NEEDED, fetching password info for 2FA");
                let password: tl::types::account::Password = client
                    .invoke(&tl::functions::account::GetPassword {})
                    .await
                    .map_err(|e| anyhow!("Failed to get password info: {}", e))?
                    .into();
                let hint = password.hint.clone().unwrap_or_default();
                let pw_token = PasswordToken::new(password);
                tracing::info!("[tg-qr] got password token, hint={:?}", hint);
                let mut guard = handle.lock().await;
                guard.password_token = Some(pw_token);
                Ok(QrPollStatus::PasswordRequired { hint })
            } else {
                tracing::error!("[tg-qr] poll error: {}", err_str);
                Err(anyhow!("Poll error: {}", err_str))
            }
        }
    }
}

fn extract_phone_from_auth(auth: &tl::enums::auth::Authorization) -> String {
    match auth {
        tl::enums::auth::Authorization::Authorization(a) => match &a.user {
            tl::enums::User::User(u) => u.phone.clone().unwrap_or_default(),
            tl::enums::User::Empty(_) => String::new(),
        },
        tl::enums::auth::Authorization::SignUpRequired(_) => String::new(),
    }
}

pub async fn send_code(handle: &TelegramSessionHandle, phone: &str) -> anyhow::Result<()> {
    let mut guard = handle.lock().await;
    let client = if let Some(ref c) = guard.client {
        c.clone()
    } else {
        let (c, updates_rx) = create_client_with_updates().await?;
        guard.client = Some(c.clone());
        guard.updates_rx = Some(updates_rx);
        c
    };
    guard.phone = phone.to_string();
    drop(guard);

    let token = client
        .request_login_code(phone, API_HASH)
        .await
        .map_err(|e| describe_invocation_error("Failed to send code", &e))?;

    let mut guard = handle.lock().await;
    guard.login_token = Some(token);
    Ok(())
}

pub async fn verify_code(
    handle: &TelegramSessionHandle,
    code: &str,
) -> Result<String, VerifyError> {
    let guard = handle.lock().await;
    let client = guard.client.as_ref().ok_or(VerifyError::NoSession)?.clone();
    drop(guard);

    let mut guard = handle.lock().await;
    let token = guard
        .login_token
        .take()
        .ok_or(VerifyError::Other("No login token available".to_string()))?;
    drop(guard);

    match client.sign_in(&token, code).await {
        Ok(user) => {
            let phone = user.phone().unwrap_or("").to_string();
            let mut guard = handle.lock().await;
            guard.phone = phone.clone();
            guard.password_token = None;
            Ok(phone)
        }
        Err(SignInError::PasswordRequired(pw_token)) => {
            let hint = pw_token.hint().unwrap_or("").to_string();
            let mut guard = handle.lock().await;
            guard.password_token = Some(pw_token);
            Err(VerifyError::PasswordRequired { hint })
        }
        Err(SignInError::InvalidCode) => {
            let mut guard = handle.lock().await;
            guard.login_token = Some(token);
            Err(VerifyError::InvalidCode)
        }
        Err(e) => Err(VerifyError::Other(e.to_string())),
    }
}

pub async fn verify_password(
    handle: &TelegramSessionHandle,
    password: &str,
) -> anyhow::Result<String> {
    let guard = handle.lock().await;
    let client = guard
        .client
        .as_ref()
        .ok_or_else(|| anyhow!("No active session"))?
        .clone();
    drop(guard);

    let mut guard = handle.lock().await;
    let pw_token = guard
        .password_token
        .take()
        .ok_or_else(|| anyhow!("No password token available"))?;
    drop(guard);

    let pw_token_backup = pw_token.clone();

    match client.check_password(pw_token, password.as_bytes()).await {
        Ok(user) => {
            let phone = user.phone().unwrap_or("").to_string();
            let mut guard = handle.lock().await;
            guard.phone = phone.clone();
            Ok(phone)
        }
        Err(SignInError::InvalidPassword) => {
            let mut guard = handle.lock().await;
            guard.password_token = Some(pw_token_backup);
            Err(anyhow!("invalid_password"))
        }
        Err(e) => Err(anyhow!("{}", e)),
    }
}

pub async fn logout(handle: &TelegramSessionHandle) -> anyhow::Result<()> {
    let mut guard = handle.lock().await;
    if let Some(client) = guard.client.take() {
        let _ = client.sign_out().await;
        client.disconnect();
    }
    guard.phone.clear();
    guard.login_token = None;
    guard.password_token = None;
    guard.updates_rx = None;
    drop(guard);
    delete_session().await?;
    Ok(())
}

pub enum VerifyError {
    InvalidCode,
    PasswordRequired { hint: String },
    NoSession,
    Other(String),
}
