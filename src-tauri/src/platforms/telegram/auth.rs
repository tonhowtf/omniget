use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use grammers_client::{Client, SignInError};
use grammers_client::grammers_tl_types as tl;
use grammers_client::types::{LoginToken, PasswordToken};
use grammers_mtsender::{ConnectionParams, SenderPool};
use grammers_session::Session;
use grammers_session::storages::SqliteSession;
use grammers_session::updates::UpdatesLike;
use qrcode::QrCode;
use qrcode::render::svg;
use tokio::sync::{Mutex, mpsc};

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
        }
    }
}

fn session_file_path() -> anyhow::Result<PathBuf> {
    let data_dir = dirs::data_dir()
        .ok_or_else(|| anyhow!("Cannot find app data directory"))?;
    Ok(data_dir.join("omniget").join("telegram.session"))
}

fn open_session() -> anyhow::Result<Arc<SqliteSession>> {
    let path = session_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let session = SqliteSession::open(path)?;
    Ok(Arc::new(session))
}

fn connection_params() -> ConnectionParams {
    let os_version = os_info::get();
    ConnectionParams {
        device_model: "Desktop".to_string(),
        system_version: format!("{} {}", os_version.os_type(), os_version.version()),
        app_version: format!("{} x64", env!("CARGO_PKG_VERSION")),
        system_lang_code: sys_locale::get_locale()
            .unwrap_or_else(|| "en".to_string())
            .split('-').next().unwrap_or("en").to_string(),
        lang_code: "en".to_string(),
        ..ConnectionParams::default()
    }
}

fn create_client_with_updates() -> anyhow::Result<(Client, mpsc::UnboundedReceiver<UpdatesLike>)> {
    let session = open_session()?;
    let pool = SenderPool::with_configuration(session, API_ID, connection_params());
    let client = Client::new(&pool);
    let updates_rx = pool.updates;
    tokio::spawn(pool.runner.run());
    Ok((client, updates_rx))
}

pub fn create_client() -> anyhow::Result<Client> {
    let (client, _updates_rx) = create_client_with_updates()?;
    Ok(client)
}

pub async fn delete_session() -> anyhow::Result<()> {
    let path = session_file_path()?;
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}

pub async fn check_session(handle: &TelegramSessionHandle) -> anyhow::Result<String> {
    let (existing_client, existing_phone) = {
        let guard = handle.lock().await;
        (guard.client.clone(), guard.phone.clone())
    };

    if let Some(client) = existing_client {
        if client.is_authorized().await? {
            return Ok(existing_phone);
        }
    }

    let client = create_client()?;
    if client.is_authorized().await? {
        let me = client.get_me().await?;
        let phone = me.phone().unwrap_or("").to_string();
        let mut guard = handle.lock().await;
        guard.client = Some(client);
        guard.phone = phone.clone();
        return Ok(phone);
    }

    Err(anyhow!("not_authenticated"))
}

pub struct QrLoginResult {
    pub qr_svg: String,
    pub expires: i32,
}

fn token_to_qr(token_data: &tl::types::auth::LoginToken) -> anyhow::Result<QrLoginResult> {
    let token_b64 = URL_SAFE_NO_PAD.encode(&token_data.token);
    let qr_url = format!("tg://login?token={}", token_b64);

    let qr = QrCode::new(qr_url.as_bytes())
        .map_err(|e| anyhow!("Failed to generate QR code: {}", e))?;
    let qr_svg = qr.render::<svg::Color>()
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
    client.invoke(&request).await
        .map_err(|e| anyhow!("Failed to export login token: {}", e))
}

fn drain_for_login_token(updates_rx: &mut mpsc::UnboundedReceiver<UpdatesLike>) -> bool {
    let mut found = false;
    let mut count = 0;
    while let Ok(update) = updates_rx.try_recv() {
        count += 1;
        let is_login = has_login_token_update(&update);
        tracing::info!("[tg-qr] drained update #{}, is_login_token={}", count, is_login);
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
        UpdatesLike::Updates(tl::enums::Updates::Updates(updates)) => {
            updates.updates.iter().any(|u| matches!(u, tl::enums::Update::LoginToken))
        }
        UpdatesLike::Updates(tl::enums::Updates::Combined(combined)) => {
            combined.updates.iter().any(|u| matches!(u, tl::enums::Update::LoginToken))
        }
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

    let (new_client, new_updates_rx) = create_client_with_updates()?;

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
        Ok(tl::enums::auth::LoginToken::MigrateTo(_)) => {
            Err(anyhow!("Multiple DC migrations"))
        }
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
                guard.updates_rx = None;
                Ok(MigrateResult::PasswordRequired { hint })
            } else {
                Err(anyhow!("Import after DC migration failed: {}", err_str))
            }
        }
    }
}

pub async fn qr_login_start(handle: &TelegramSessionHandle) -> anyhow::Result<QrLoginResult> {
    tracing::info!("[tg-qr] qr_login_start: creating client");
    let mut guard = handle.lock().await;

    if let Some(old_client) = guard.client.take() {
        old_client.disconnect();
    }
    guard.updates_rx = None;

    let (client, updates_rx) = create_client_with_updates()?;
    guard.client = Some(client.clone());
    guard.updates_rx = Some(updates_rx);
    drop(guard);

    tracing::info!("[tg-qr] qr_login_start: calling ExportLoginToken");
    let result = export_login_token(&client).await?;

    match result {
        tl::enums::auth::LoginToken::Token(token_data) => {
            token_to_qr(&token_data)
        }
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
    let client = guard.client.as_ref()
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

    match export_login_token(&client).await {
        Ok(tl::enums::auth::LoginToken::Success(success)) => {
            let phone = extract_phone_from_auth(&success.authorization);
            tracing::info!("[tg-qr] login success, phone={}", phone);
            let mut guard = handle.lock().await;
            guard.phone = phone.clone();
            guard.updates_rx = None;
            Ok(QrPollStatus::Success { phone })
        }
        Ok(tl::enums::auth::LoginToken::Token(_)) => {
            tracing::info!("[tg-qr] got new token after update (not success yet), waiting");
            Ok(QrPollStatus::Waiting)
        }
        Ok(tl::enums::auth::LoginToken::MigrateTo(migrate)) => {
            tracing::info!("[tg-qr] got MigrateTo after update, migrating DC");
            match handle_migrate(handle, &client, migrate).await? {
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
                guard.updates_rx = None;
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
        tl::enums::auth::Authorization::Authorization(a) => {
            match &a.user {
                tl::enums::User::User(u) => {
                    u.phone.clone().unwrap_or_default()
                }
                tl::enums::User::Empty(_) => String::new(),
            }
        }
        tl::enums::auth::Authorization::SignUpRequired(_) => String::new(),
    }
}

pub async fn send_code(handle: &TelegramSessionHandle, phone: &str) -> anyhow::Result<()> {
    let mut guard = handle.lock().await;
    let client = if let Some(ref c) = guard.client {
        c.clone()
    } else {
        let c = create_client()?;
        guard.client = Some(c.clone());
        c
    };
    guard.phone = phone.to_string();
    drop(guard);

    let token = client.request_login_code(phone, API_HASH).await
        .map_err(|e| anyhow!("Failed to send code: {}", e))?;

    let mut guard = handle.lock().await;
    guard.login_token = Some(token);
    Ok(())
}

pub async fn verify_code(
    handle: &TelegramSessionHandle,
    code: &str,
) -> Result<String, VerifyError> {
    let guard = handle.lock().await;
    let client = guard.client.as_ref()
        .ok_or(VerifyError::NoSession)?
        .clone();
    drop(guard);

    let mut guard = handle.lock().await;
    let token = guard.login_token.take()
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
        Err(e) => {
            Err(VerifyError::Other(e.to_string()))
        }
    }
}

pub async fn verify_password(
    handle: &TelegramSessionHandle,
    password: &str,
) -> anyhow::Result<String> {
    let guard = handle.lock().await;
    let client = guard.client.as_ref()
        .ok_or_else(|| anyhow!("No active session"))?
        .clone();
    drop(guard);

    let mut guard = handle.lock().await;
    let pw_token = guard.password_token.take()
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
        Err(e) => {
            Err(anyhow!("{}", e))
        }
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
    PasswordRequired {
        hint: String,
    },
    NoSession,
    Other(String),
}
