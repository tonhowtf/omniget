use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use grammers_client::{Client, SignInError};
use grammers_client::types::{LoginToken, PasswordToken};
use grammers_client::mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use tokio::sync::Mutex;

const API_ID: i32 = 0;
const API_HASH: &str = "";

pub type TelegramSessionHandle = Arc<Mutex<TelegramState>>;

pub struct TelegramState {
    pub client: Option<Client>,
    pub phone: String,
    pub login_token: Option<LoginToken>,
    pub password_token: Option<PasswordToken>,
}

impl TelegramState {
    pub fn new() -> Self {
        Self {
            client: None,
            phone: String::new(),
            login_token: None,
            password_token: None,
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

pub fn create_client() -> anyhow::Result<Client> {
    let session = open_session()?;
    let pool = SenderPool::new(session, API_ID);
    let client = Client::new(&pool);
    tokio::spawn(pool.runner.run());
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
    let guard = handle.lock().await;
    if let Some(client) = guard.client.as_ref() {
        if client.is_authorized().await? {
            return Ok(guard.phone.clone());
        }
    }
    drop(guard);

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

    match client.check_password(pw_token, password.as_bytes()).await {
        Ok(user) => {
            let phone = user.phone().unwrap_or("").to_string();
            let mut guard = handle.lock().await;
            guard.phone = phone.clone();
            Ok(phone)
        }
        Err(SignInError::InvalidPassword) => {
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
