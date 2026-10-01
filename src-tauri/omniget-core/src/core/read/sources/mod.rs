pub mod declarative;
pub mod googlebooks;
pub mod gutendex;
pub mod mangadex;
pub mod mangaplus;
pub mod openlibrary;
pub mod standardebooks;
pub mod suwayomi;

use serde::{Deserialize, Serialize};

pub const USER_AGENT: &str = concat!(
    "OmniGet/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/tonhowtf/omniget)"
);

pub fn client() -> reqwest::Client {
    crate::core::http_client::apply_global_proxy(
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30)),
    )
    .build()
    .unwrap_or_default()
}

pub async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let r = client.get(url).send().await.map_err(|e| e.to_string())?;
    let status = r.status();
    if !status.is_success() {
        return Err(format!("{status} from {url}"));
    }
    r.json().await.map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub source: String,
    pub key: String,
    pub title: String,
    pub authors: Vec<String>,
    pub cover: Option<String>,
    pub summary: String,
    pub language: String,
    pub year: Option<i32>,
    pub subjects: Vec<String>,
    pub kind: String,
    pub downloads: Vec<Download>,
    pub url: String,
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Download {
    pub url: String,
    pub format: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Page {
    pub hits: Vec<Hit>,
    pub total: Option<u64>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RemoteChapter {
    pub key: String,
    pub title: String,
    pub number: Option<f64>,
    pub volume: Option<f64>,
    pub language: String,
    pub scanlator: String,
    pub published: Option<i64>,
    pub pages: Option<u32>,
    pub external_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RemotePage {
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xor_key: Option<String>,
}
