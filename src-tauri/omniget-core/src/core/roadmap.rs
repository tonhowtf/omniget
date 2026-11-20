use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};

pub const ROADMAP_URL: &str = "https://getomniget.com/roadmap.json";
const MAX_BYTES: usize = 512 * 1024;
const MAX_ITEMS: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum RoadmapError {
    #[error("could not reach the roadmap: {0}")]
    Network(String),
    #[error("the roadmap answered HTTP {0}")]
    Status(u16),
    #[error("the roadmap is larger than {MAX_BYTES} bytes")]
    TooLarge,
    #[error("the roadmap could not be read: {0}")]
    Parse(String),
    #[error("unknown status \"{0}\"")]
    UnknownStatus(String),
}

impl RoadmapError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Network(_) => "ROADMAP_NETWORK",
            Self::Status(_) => "ROADMAP_HTTP",
            Self::TooLarge => "ROADMAP_TOO_LARGE",
            Self::Parse(_) => "ROADMAP_PARSE",
            Self::UnknownStatus(_) => "ROADMAP_UNKNOWN_STATUS",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Status(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub key: String,
    pub title: String,
    pub status: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub priority: String,
    pub area: String,
    pub created: String,
    pub updated: String,
    pub url: String,
    pub issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roadmap {
    pub updated: String,
    pub next_version: String,
    pub board: String,
    pub columns: Vec<String>,
    pub items: Vec<Ticket>,
}

fn is_site_link(url: &str) -> bool {
    url.starts_with("https://getomniget.com/") || url.starts_with("https://github.com/")
}

pub fn parse(body: &str) -> Result<Roadmap, RoadmapError> {
    let mut roadmap: Roadmap =
        serde_json::from_str(body).map_err(|e| RoadmapError::Parse(e.to_string()))?;
    if !is_site_link(&roadmap.board) {
        return Err(RoadmapError::Parse("board link is not ours".into()));
    }
    roadmap.items.truncate(MAX_ITEMS);
    roadmap
        .items
        .retain(|t| roadmap.columns.contains(&t.status) && is_site_link(&t.url));
    for ticket in &mut roadmap.items {
        if ticket.issue.as_deref().is_some_and(|u| !is_site_link(u)) {
            ticket.issue = None;
        }
    }
    Ok(roadmap)
}

pub fn only_status(mut roadmap: Roadmap, status: &str) -> Result<Roadmap, RoadmapError> {
    if !roadmap.columns.iter().any(|c| c == status) {
        return Err(RoadmapError::UnknownStatus(status.to_string()));
    }
    roadmap.items.retain(|t| t.status == status);
    Ok(roadmap)
}

pub async fn fetch() -> Result<Roadmap, RoadmapError> {
    let builder = reqwest::Client::builder()
        .user_agent(concat!("OmniGet/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15));
    let client = crate::core::http_client::apply_global_proxy(builder)
        .build()
        .map_err(|e| RoadmapError::Network(e.to_string()))?;
    let resp = client
        .get(ROADMAP_URL)
        .send()
        .await
        .map_err(|e| RoadmapError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(RoadmapError::Status(resp.status().as_u16()));
    }
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| RoadmapError::Network(e.to_string()))?;
        if body.len() + chunk.len() > MAX_BYTES {
            return Err(RoadmapError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(body).map_err(|e| RoadmapError::Parse(e.to_string()))?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "updated": "2026-11-19",
      "next_version": "1.0.1",
      "board": "https://getomniget.com/contact#roadmap",
      "columns": ["backlog", "todo", "progress", "done", "wontfix"],
      "items": [
        {"key": "OMNI-45", "title": "Radio goes silent", "status": "progress", "type": "bug",
         "priority": "high", "area": "all", "created": "2026-11-01", "updated": "2026-11-19",
         "url": "https://getomniget.com/contact#t-radio_silent",
         "issue": "https://github.com/tonhowtf/omniget/issues/429"},
        {"key": "OMNI-50", "title": "Use it from a chat app", "status": "done", "type": "feature",
         "priority": "low", "area": "all", "created": "2026-11-05", "updated": "2026-11-05",
         "url": "https://getomniget.com/contact#t-chat", "issue": null}
      ]
    }"#;

    #[test]
    fn parse_reads_tickets_with_and_without_an_issue() {
        let r = parse(FIXTURE).unwrap();
        assert_eq!(r.next_version, "1.0.1");
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.items[0].kind, "bug");
        assert_eq!(
            r.items[0].issue.as_deref(),
            Some("https://github.com/tonhowtf/omniget/issues/429")
        );
        assert_eq!(r.items[1].issue, None);
    }

    #[test]
    fn parse_rejects_text_that_is_not_the_roadmap() {
        assert!(matches!(
            parse("<html>maintenance</html>"),
            Err(RoadmapError::Parse(_))
        ));
        assert!(matches!(parse("{}"), Err(RoadmapError::Parse(_))));
    }

    #[test]
    fn parse_drops_links_that_leave_the_site_and_the_repository() {
        let hostile = FIXTURE
            .replace(
                "https://getomniget.com/contact#t-chat",
                "javascript:alert(1)",
            )
            .replace(
                "https://github.com/tonhowtf/omniget/issues/429",
                "https://evil.example/issues/429",
            );
        let r = parse(&hostile).unwrap();
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].key, "OMNI-45");
        assert_eq!(r.items[0].issue, None);
    }

    #[test]
    fn parse_rejects_a_board_link_that_is_not_ours() {
        let hostile = FIXTURE.replace(
            "https://getomniget.com/contact#roadmap",
            "https://evil.example/board",
        );
        assert!(matches!(parse(&hostile), Err(RoadmapError::Parse(_))));
    }

    #[test]
    fn parse_drops_a_ticket_in_a_column_the_board_does_not_have() {
        let odd = FIXTURE.replace("\"status\": \"done\"", "\"status\": \"secret\"");
        assert_eq!(parse(&odd).unwrap().items.len(), 1);
    }

    #[test]
    fn only_status_keeps_one_column() {
        let r = only_status(parse(FIXTURE).unwrap(), "progress").unwrap();
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].key, "OMNI-45");
    }

    #[test]
    fn only_status_refuses_a_column_that_does_not_exist() {
        let e = only_status(parse(FIXTURE).unwrap(), "soon").unwrap_err();
        assert_eq!(e.code(), "ROADMAP_UNKNOWN_STATUS");
        assert!(!e.retryable());
    }
}
