use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

use super::api::{check_api_response, ApiClient, Result};

const FINGER_SPI_URL: &str = "https://api.bilibili.com/x/frontend/finger/spi";
const GEN_TICKET_URL: &str =
    "https://api.bilibili.com/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket";
const EXCLIMB_URL: &str = "https://api.bilibili.com/x/internal/gaia-gateway/ExClimbWuzhi";
const ANONYMOUS_SLUG: &str = "_anonymous";
const HMAC_KEY: &[u8] = b"XgwSnGZ1p";
const TICKET_TTL_SECS: u64 = 3 * 24 * 60 * 60;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnonymousCookieState {
    pub uuid: String,
    pub b_lsid: String,
    pub b_nut: String,
    pub buvid3: String,
    pub buvid4: String,
    pub buvid_fp: String,
    pub bili_ticket: String,
    pub bili_ticket_expires: u64,
    pub last_bootstrap_secs: u64,
}

impl AnonymousCookieState {
    pub fn is_fresh(&self) -> bool {
        if self.bili_ticket.is_empty() {
            return false;
        }
        now_secs() < self.bili_ticket_expires.saturating_sub(60 * 60)
    }
}

pub fn meta_file_path() -> Option<std::path::PathBuf> {
    let root = crate::core::paths::app_data_dir()?;
    Some(
        root.join("cookies")
            .join("bilibili.com")
            .join("_anonymous_meta.json"),
    )
}

pub fn load_state() -> AnonymousCookieState {
    let path = match meta_file_path() {
        Some(p) => p,
        None => return AnonymousCookieState::default(),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return AnonymousCookieState::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_state(state: &AnonymousCookieState) -> std::io::Result<()> {
    let path = meta_file_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "app_data_dir unavailable")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let serialized = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(path, serialized)
}

pub async fn ensure_fresh() -> Result<AnonymousCookieState> {
    let cached = load_state();
    if cached.is_fresh() {
        write_netscape_for_anonymous(&cached).ok();
        return Ok(cached);
    }
    bootstrap_anonymous().await
}

pub async fn bootstrap_anonymous() -> Result<AnonymousCookieState> {
    let ua = super::api::DEFAULT_USER_AGENT;
    let now_ms = now_ms();
    let uuid = gen_uuid_b_lsid();
    let b_lsid = gen_b_lsid();
    let b_nut = now_ms.to_string();
    let buvid_fp = gen_buvid_fp(ua);

    let mut state = AnonymousCookieState {
        uuid: uuid.clone(),
        b_lsid: b_lsid.clone(),
        b_nut: b_nut.clone(),
        buvid_fp: buvid_fp.clone(),
        ..AnonymousCookieState::default()
    };

    let prelim = ApiClient::new()?
        .with_user_agent(ua)
        .with_raw_cookies(seed_cookie_header(&state));

    match prelim.get_json(FINGER_SPI_URL).await {
        Ok(raw) => {
            if let Ok(data) = check_api_response(&raw) {
                state.buvid3 = data
                    .get("b_3")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                state.buvid4 = data
                    .get("b_4")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
            }
        }
        Err(e) => {
            tracing::warn!("[bilibili] finger/spi failed: {:?}", e);
        }
    }

    let ticket_cookies = seed_cookie_header(&state);
    let ticket_client = ApiClient::new()?
        .with_user_agent(ua)
        .with_raw_cookies(ticket_cookies);

    let ts = now_secs();
    let hexsign = hmac_sha256_hex(HMAC_KEY, format!("ts{}", ts).as_bytes());
    let ticket_url = format!(
        "{}?key_id=ec02&hexsign={}&context%5Bts%5D={}&csrf=",
        GEN_TICKET_URL, hexsign, ts
    );
    match ticket_client.post_form(&ticket_url, &[]).await {
        Ok(raw) => {
            if let Ok(data) = check_api_response(&raw) {
                state.bili_ticket = data
                    .get("ticket")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let created_at = data.get("created_at").and_then(Value::as_u64).unwrap_or(ts);
                let ttl = data
                    .get("ttl")
                    .and_then(Value::as_u64)
                    .unwrap_or(TICKET_TTL_SECS);
                state.bili_ticket_expires = created_at + ttl;
            }
        }
        Err(e) => {
            tracing::warn!("[bilibili] bili_ticket generation failed: {:?}", e);
        }
    }

    if !state.buvid3.is_empty() {
        let exclimb_client = ApiClient::new()?
            .with_user_agent(ua)
            .with_raw_cookies(seed_cookie_header(&state));
        let payload = exclimb_payload(ua, &state.uuid);
        match exclimb_client.post_json(EXCLIMB_URL, &payload).await {
            Ok(_) => {}
            Err(e) => {
                tracing::debug!("[bilibili] ExClimbWuzhi ack failed (non-fatal): {:?}", e);
            }
        }
    }

    state.last_bootstrap_secs = now_secs();
    save_state(&state).ok();
    write_netscape_for_anonymous(&state).ok();
    Ok(state)
}

pub fn build_cookie_header(state: &AnonymousCookieState) -> String {
    let mut pairs: Vec<String> = Vec::new();
    pairs.push(format!("_uuid={}", state.uuid));
    pairs.push(format!("b_lsid={}", state.b_lsid));
    pairs.push(format!("b_nut={}", state.b_nut));
    pairs.push(format!("buvid_fp={}", state.buvid_fp));
    if !state.buvid3.is_empty() {
        pairs.push(format!("buvid3={}", state.buvid3));
    }
    if !state.buvid4.is_empty() {
        pairs.push(format!("buvid4={}", state.buvid4));
    }
    if !state.bili_ticket.is_empty() {
        pairs.push(format!("bili_ticket={}", state.bili_ticket));
        pairs.push(format!("bili_ticket_expires={}", state.bili_ticket_expires));
    }
    pairs.push("CURRENT_FNVAL=4048".to_string());
    pairs.push("CURRENT_QUALITY=0".to_string());
    pairs.join("; ")
}

fn seed_cookie_header(state: &AnonymousCookieState) -> String {
    build_cookie_header(state)
}

fn write_netscape_for_anonymous(state: &AnonymousCookieState) -> std::io::Result<()> {
    let path = crate::cookies::account_path_for_consumer("bilibili.com", Some(ANONYMOUS_SLUG))
        .unwrap_or_else(|| {
            let root = crate::core::paths::app_data_dir().unwrap_or_default();
            root.join("cookies")
                .join("bilibili.com")
                .join(format!("{}.txt", ANONYMOUS_SLUG))
        });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let session_expires = now_secs() + 86400;
    let expires = if state.bili_ticket_expires > 0 {
        state.bili_ticket_expires
    } else {
        session_expires
    };
    let mut content = String::from("# Netscape HTTP Cookie File\n");
    let pairs: Vec<(&str, String)> = vec![
        ("_uuid", state.uuid.clone()),
        ("b_lsid", state.b_lsid.clone()),
        ("b_nut", state.b_nut.clone()),
        ("buvid_fp", state.buvid_fp.clone()),
        ("buvid3", state.buvid3.clone()),
        ("buvid4", state.buvid4.clone()),
        ("bili_ticket", state.bili_ticket.clone()),
        ("bili_ticket_expires", state.bili_ticket_expires.to_string()),
        ("CURRENT_FNVAL", "4048".to_string()),
        ("CURRENT_QUALITY", "0".to_string()),
    ];
    for (name, value) in pairs {
        if value.is_empty() {
            continue;
        }
        content.push_str(&format!(
            ".bilibili.com\tTRUE\t/\tTRUE\t{}\t{}\t{}\n",
            expires, name, value
        ));
    }
    std::fs::write(path, content)
}

fn gen_uuid_b_lsid() -> String {
    let mut rng = rand::rng();
    let s: String = (0..32)
        .map(|i| {
            if i == 8 || i == 12 || i == 16 || i == 20 {
                '-'
            } else {
                let n: u32 = rng.random_range(0..16);
                std::char::from_digit(n, 16).unwrap_or('0')
            }
        })
        .collect();
    format!("{}{}infoc", s, now_ms() % 100000)
}

fn gen_b_lsid() -> String {
    let mut rng = rand::rng();
    let head: String = (0..8)
        .map(|_| {
            let n: u32 = rng.random_range(0..16);
            std::char::from_digit(n, 16).unwrap_or('0')
        })
        .collect::<String>()
        .to_uppercase();
    format!("{}_{:X}", head, now_ms())
}

fn gen_buvid_fp(ua: &str) -> String {
    let mut cursor = Cursor::new(ua.as_bytes());
    let hash = murmur3::murmur3_x64_128(&mut cursor, 31).unwrap_or(0);
    let low = (hash & 0xFFFFFFFFFFFFFFFF) as u64;
    let high = (hash >> 64) as u64;
    format!("{:016x}{:016x}", low, high)
}

fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    let mut mac = match HmacSha256::new_from_slice(key) {
        Ok(m) => m,
        Err(_) => return String::new(),
    };
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

fn exclimb_payload(ua: &str, uuid: &str) -> Value {
    use serde_json::Map;
    let mut device = Map::new();
    device.insert("2673".into(), Value::from(0));
    device.insert("5766".into(), Value::from(24));
    device.insert("6527".into(), Value::from(0));
    device.insert("7003".into(), Value::from(1));
    device.insert("807e".into(), Value::from(1));
    device.insert("b8ce".into(), Value::from(ua));
    device.insert("641c".into(), Value::from(0));
    device.insert("07a4".into(), Value::from("en-US"));
    device.insert("1c57".into(), Value::from(8));
    device.insert("0bd0".into(), Value::from(24));
    device.insert("748e".into(), Value::from(vec![1080, 1920]));
    device.insert("d61f".into(), Value::from(vec![1032, 1920]));
    device.insert("fc9d".into(), Value::from(-180));
    device.insert("6aa9".into(), Value::from("Asia/Shanghai"));
    device.insert("75b8".into(), Value::from(1));
    device.insert("3b21".into(), Value::from(1));
    device.insert("8a1c".into(), Value::from(0));
    device.insert("d52f".into(), Value::from("not available"));
    device.insert("adca".into(), Value::from("MacIntel"));
    device.insert("80c9".into(), Value::Array(vec![]));
    device.insert("13ab".into(), Value::from(""));
    device.insert("bfe9".into(), Value::from(""));
    device.insert("a3c1".into(), Value::Array(vec![]));
    device.insert("6bc5".into(), Value::from(""));
    device.insert("ed31".into(), Value::from(0));
    device.insert("72bd".into(), Value::from(0));
    device.insert("097b".into(), Value::from(0));
    device.insert("52cd".into(), Value::from(vec![0, 0, 0]));
    device.insert("a658".into(), Value::Array(vec![]));
    device.insert("d4a0".into(), Value::from(false));
    device.insert("159f".into(), Value::from(false));
    device.insert("5ea0".into(), Value::from(false));
    device.insert("8eb3".into(), Value::from(false));

    let mut payload = Map::new();
    payload.insert("3064".into(), Value::from(1));
    payload.insert("5062".into(), Value::from(now_ms().to_string()));
    payload.insert("03bf".into(), Value::from("https://www.bilibili.com/"));
    payload.insert("39c8".into(), Value::from("333.999.fp.risk"));
    payload.insert("34f1".into(), Value::from(""));
    payload.insert("d402".into(), Value::from(""));
    payload.insert("654a".into(), Value::from(""));
    payload.insert("6e7c".into(), Value::from("839x959"));
    payload.insert("3c43".into(), Value::Object(device));
    payload.insert("8b94".into(), Value::from(""));
    payload.insert("df35".into(), Value::from(uuid));
    payload.insert("07a4".into(), Value::from("en-US"));
    payload.insert("5f45".into(), Value::Null);
    payload.insert("db46".into(), Value::from(0));

    let mut root = Map::new();
    root.insert("payload".into(), Value::Object(payload));
    Value::Object(root)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
