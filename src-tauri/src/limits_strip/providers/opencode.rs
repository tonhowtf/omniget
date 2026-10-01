use crate::limits_strip::{
    http, iso_ms, json_or_error, net_err, num, LimitWindow, ReadError, Reading, Secret,
    UsageProvider, DAY_MS, HOUR_MS,
};
use std::path::PathBuf;

const ENDPOINT: &str = "https://opencode.ai/zen/go/v1/usage";
pub const KEY_FIELDS: [&str; 7] = [
    "key",
    "apiKey",
    "api_key",
    "token",
    "accessToken",
    "access",
    "auth_token",
];

pub struct OpenCode;

pub fn auth_path() -> Option<PathBuf> {
    super::xdg_data_home().map(|d| d.join("opencode").join("auth.json"))
}

pub fn key_from(v: &serde_json::Value, id: &str) -> Option<Secret> {
    Secret::new(&super::string_or_field(v.get(id)?, &KEY_FIELDS)?)
}

pub fn parse_usage(v: &serde_json::Value) -> Vec<LimitWindow> {
    let mut out = Vec::new();
    for (id, label, span) in [
        ("rolling", "5h limit", 5 * HOUR_MS),
        ("weekly", "Weekly limit", 7 * DAY_MS),
        ("monthly", "Monthly limit", 30 * DAY_MS),
    ] {
        let Some(w) = v.pointer(&format!("/usage/{id}")) else {
            continue;
        };
        if let Some(pct) = num(w.get("percent")) {
            out.push(
                LimitWindow::percent(id, label, pct)
                    .resets(iso_ms(w.get("resetsAt")))
                    .span(span),
            );
        }
    }
    out
}

#[async_trait::async_trait]
impl UsageProvider for OpenCode {
    fn id(&self) -> &str {
        "opencode"
    }
    fn label(&self) -> &'static str {
        "OpenCode"
    }

    async fn detect(&self) -> bool {
        auth_path().map(|p| p.is_file()).unwrap_or(false)
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let key = auth_path()
            .as_deref()
            .and_then(crate::limits_strip::read_json)
            .and_then(|v| key_from(&v, "opencode-go"))
            .ok_or_else(|| ReadError::NeedsAuth("no OpenCode Go key".into()))?;
        let resp = http()
            .get(ENDPOINT)
            .bearer_auth(key.expose())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(net_err)?;
        if resp.status().as_u16() == 403 {
            return Ok(Reading {
                note: Some("no OpenCode Go subscription on this key".into()),
                ..Default::default()
            });
        }
        Ok(Reading {
            windows: parse_usage(&json_or_error(resp).await?),
            plan: Some("Go".into()),
            ..Default::default()
        })
    }
}
