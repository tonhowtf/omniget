use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CHAT_BASE: &str = "https://getomniget.com/api/chat";
const TICKET_MIN_AGE: Duration = Duration::from_millis(3200);
const MAX_TEXT: usize = 600;
const MAX_TITLE: usize = 90;
const MAX_POW_BITS: u32 = 24;

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        omniget_core::core::http_client::apply_global_proxy(
            reqwest::Client::builder()
                .user_agent(concat!("OmniGet/", env!("CARGO_PKG_VERSION")))
                .cookie_store(false)
                .timeout(Duration::from_secs(40))
                .connect_timeout(Duration::from_secs(10)),
        )
        .build()
        .unwrap_or_default()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    pub id: u64,
    pub t: String,
    pub name: String,
    pub text: String,
    #[serde(default)]
    pub author: bool,
    #[serde(default)]
    pub idea: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatPage {
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub last: u64,
    #[serde(default)]
    pub frozen: bool,
    #[serde(default)]
    pub gone: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatAnswer {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub id: Option<u64>,
}

#[derive(Deserialize)]
struct Ticket {
    ticket: String,
    bits: u32,
}

fn leading_zero_bits(digest: &[u8]) -> u32 {
    let mut n = 0;
    for byte in digest {
        if *byte == 0 {
            n += 8;
            continue;
        }
        return n + byte.leading_zeros();
    }
    n
}

pub fn solve_puzzle(ticket: &str, bits: u32) -> Option<u64> {
    if bits > MAX_POW_BITS {
        return None;
    }
    (0..u64::MAX).find(|nonce| {
        let digest = Sha256::digest(format!("{ticket}:{nonce}").as_bytes());
        leading_zero_bits(&digest) >= bits
    })
}

fn chat_lang(lang: &str) -> &'static str {
    match lang {
        l if l.starts_with("pt") => "pt",
        l if l.starts_with("zh") => "zh",
        _ => "en",
    }
}

fn clip(text: &str, limit: usize) -> String {
    text.trim().chars().take(limit).collect()
}

#[tauri::command]
pub async fn community_chat_messages(after: u64, wait: bool) -> Result<ChatPage, String> {
    let mut url = format!("{CHAT_BASE}/messages?after={after}");
    if !wait {
        url.push_str("&now=1");
    }
    let res = client().get(&url).send().await.map_err(|e| e.to_string())?;
    if res.status().as_u16() == 503 {
        return Err("busy".into());
    }
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    res.json::<ChatPage>().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn community_chat_send(
    text: String,
    lang: String,
    title: Option<String>,
) -> Result<ChatAnswer, String> {
    let text = clip(&text, MAX_TEXT);
    let title = title
        .map(|t| clip(&t.replace('\n', " "), MAX_TITLE))
        .filter(|t| !t.is_empty());
    if text.is_empty() && title.is_none() {
        return Ok(ChatAnswer {
            ok: false,
            state: "error".into(),
            id: None,
        });
    }
    let got = Instant::now();
    let ticket: Ticket = client()
        .get(format!("{CHAT_BASE}/ticket"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let (value, bits) = (ticket.ticket.clone(), ticket.bits);
    let nonce = tokio::task::spawn_blocking(move || solve_puzzle(&value, bits))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "puzzle".to_string())?;
    if let Some(left) = TICKET_MIN_AGE.checked_sub(got.elapsed()) {
        tokio::time::sleep(left).await;
    }
    let nonce = nonce.to_string();
    let mut form: Vec<(&str, &str)> = vec![
        ("ticket", ticket.ticket.as_str()),
        ("nonce", nonce.as_str()),
        ("lang", chat_lang(&lang)),
        ("text", text.as_str()),
    ];
    if let Some(title) = title.as_deref() {
        form.push(("kind", "idea"));
        form.push(("title", title));
    }
    let res = client()
        .post(format!("{CHAT_BASE}/send"))
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    res.json::<ChatAnswer>().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn community_chat_report(id: u64) -> Result<ChatAnswer, String> {
    let id = id.to_string();
    let res = client()
        .post(format!("{CHAT_BASE}/report"))
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[("id", id.as_str()), ("lang", "en")])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    res.json::<ChatAnswer>().await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puzzle_meets_the_requested_bits() {
        let ticket = "1761998400.0123456789abcdef.12.0123456789abcdef0123456789abcdef";
        let nonce = solve_puzzle(ticket, 12).expect("nonce");
        let digest = Sha256::digest(format!("{ticket}:{nonce}").as_bytes());
        assert!(leading_zero_bits(&digest) >= 12);
    }

    #[test]
    fn refuses_an_unreasonable_puzzle() {
        assert_eq!(solve_puzzle("x", 40), None);
    }

    #[test]
    fn leading_zero_bits_counts_across_bytes() {
        assert_eq!(leading_zero_bits(&[0, 0b0001_0000]), 11);
        assert_eq!(leading_zero_bits(&[0xff]), 0);
        assert_eq!(leading_zero_bits(&[0, 0]), 16);
    }

    #[test]
    fn language_maps_to_the_three_rooms() {
        assert_eq!(chat_lang("pt-BR"), "pt");
        assert_eq!(chat_lang("zh-CN"), "zh");
        assert_eq!(chat_lang("ru"), "en");
    }

    #[test]
    fn clip_keeps_whole_characters() {
        assert_eq!(clip("  héllo  ", 3), "hél");
    }
}
