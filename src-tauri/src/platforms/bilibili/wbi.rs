use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde_json::Value;
use tokio::sync::RwLock;

use super::api::{check_api_response, ApiClient, BilibiliError, Result};

const NAV_URL: &str = "https://api.bilibili.com/x/web-interface/nav";
const KEYS_TTL_SECS: u64 = 30 * 60;

const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

const RESERVED_FILTER: &[char] = &['!', '\'', '(', ')', '*'];

#[derive(Debug, Clone)]
pub struct WbiKeys {
    pub img_key: String,
    pub sub_key: String,
    pub fetched_at_secs: u64,
}

impl WbiKeys {
    pub fn mixin_key(&self) -> String {
        let combined = format!("{}{}", self.img_key, self.sub_key);
        let bytes = combined.as_bytes();
        let mut mixin = String::with_capacity(32);
        for &idx in MIXIN_KEY_ENC_TAB.iter().take(32) {
            if idx < bytes.len() {
                mixin.push(bytes[idx] as char);
            }
        }
        mixin
    }

    pub fn is_fresh(&self) -> bool {
        now_secs().saturating_sub(self.fetched_at_secs) < KEYS_TTL_SECS
    }
}

static KEYS_CACHE: RwLock<Option<WbiKeys>> = RwLock::const_new(None);

pub async fn keys(client: &ApiClient) -> Result<WbiKeys> {
    {
        let guard = KEYS_CACHE.read().await;
        if let Some(k) = guard.as_ref() {
            if k.is_fresh() {
                return Ok(k.clone());
            }
        }
    }
    let fresh = fetch_keys(client).await?;
    {
        let mut guard = KEYS_CACHE.write().await;
        *guard = Some(fresh.clone());
    }
    Ok(fresh)
}

pub async fn invalidate_cache() {
    let mut guard = KEYS_CACHE.write().await;
    *guard = None;
}

async fn fetch_keys(client: &ApiClient) -> Result<WbiKeys> {
    let raw = client.get_json(NAV_URL).await?;
    let data = match check_api_response(&raw) {
        Ok(d) => d,
        Err(BilibiliError::NotLoggedIn) => {
            raw.get("data").ok_or(BilibiliError::ContentUnavailable)?
        }
        Err(e) => return Err(e),
    };
    let wbi_img = data
        .get("wbi_img")
        .ok_or(BilibiliError::ContentUnavailable)?;
    let img_url = wbi_img
        .get("img_url")
        .and_then(Value::as_str)
        .ok_or(BilibiliError::ContentUnavailable)?;
    let sub_url = wbi_img
        .get("sub_url")
        .and_then(Value::as_str)
        .ok_or(BilibiliError::ContentUnavailable)?;
    Ok(WbiKeys {
        img_key: extract_key_from_url(img_url),
        sub_key: extract_key_from_url(sub_url),
        fetched_at_secs: now_secs(),
    })
}

fn extract_key_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .map(|seg| seg.split('.').next().unwrap_or(seg).to_string())
        .unwrap_or_default()
}

pub fn sign(params: &[(&str, String)], keys: &WbiKeys) -> String {
    let wts = now_secs().to_string();
    let mut all: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| ((*k).to_string(), filter_value(v)))
        .collect();
    all.push(("wts".to_string(), wts.clone()));
    all.sort_by(|a, b| a.0.cmp(&b.0));

    let query = encode_pairs(&all);
    let to_hash = format!("{}{}", query, keys.mixin_key());
    let mut hasher = Md5::new();
    hasher.update(to_hash.as_bytes());
    let w_rid = hex::encode(hasher.finalize());

    format!("{}&w_rid={}", query, w_rid)
}

pub async fn signed_query(client: &ApiClient, params: &[(&str, String)]) -> Result<String> {
    let k = keys(client).await?;
    Ok(sign(params, &k))
}

fn filter_value(v: &str) -> String {
    v.chars().filter(|c| !RESERVED_FILTER.contains(c)).collect()
}

fn encode_pairs(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixin_key_extracts_32_chars() {
        let k = WbiKeys {
            img_key: "7cd084941338484aae1ad9425b84077c".to_string(),
            sub_key: "4932caff0ff746eab6f01bf08b70ac45".to_string(),
            fetched_at_secs: 0,
        };
        let mix = k.mixin_key();
        assert_eq!(mix.len(), 32);
    }

    #[test]
    fn extract_key_handles_typical_url() {
        let url = "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png";
        assert_eq!(
            extract_key_from_url(url),
            "7cd084941338484aae1ad9425b84077c"
        );
    }

    #[test]
    fn sign_produces_w_rid() {
        let k = WbiKeys {
            img_key: "7cd084941338484aae1ad9425b84077c".to_string(),
            sub_key: "4932caff0ff746eab6f01bf08b70ac45".to_string(),
            fetched_at_secs: 0,
        };
        let params: Vec<(&str, String)> =
            vec![("foo", "114".to_string()), ("bar", "514".to_string())];
        let query = sign(&params, &k);
        assert!(query.contains("w_rid="));
        assert!(query.contains("wts="));
    }
}
