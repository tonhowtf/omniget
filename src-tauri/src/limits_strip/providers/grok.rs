use crate::limits_strip::{
    http, iso_ms, json_or_error, net_err, now_ms, num, LimitWindow, ReadError, Reading, Secret,
    UsageProvider, DAY_MS,
};
use std::path::PathBuf;

const ENDPOINT: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const TRUSTED_ISSUER: &str = "https://auth.x.ai";

pub struct Grok;

pub fn auth_path() -> Option<PathBuf> {
    crate::limits_strip::home().map(|h| h.join(".grok").join("auth.json"))
}

#[derive(Debug, PartialEq)]
pub struct Session {
    token: Secret,
    pub email: Option<String>,
    pub expired: bool,
}

pub fn pick_session(v: &serde_json::Value, now: i64) -> Option<Session> {
    let mut best: Option<(i64, Session)> = None;
    for (key, entry) in v.as_object()? {
        let issuer = key.split("::").next().unwrap_or("");
        let trusted = issuer == TRUSTED_ISSUER
            || entry.get("oidc_issuer").and_then(|x| x.as_str()) == Some(TRUSTED_ISSUER);
        let token = entry
            .get("key")
            .and_then(|x| x.as_str())
            .and_then(Secret::new);
        let (true, Some(token)) = (trusted, token) else {
            continue;
        };
        let expires = iso_ms(entry.get("expires_at")).unwrap_or(i64::MAX);
        if best.as_ref().map(|b| expires > b.0).unwrap_or(true) {
            best = Some((
                expires,
                Session {
                    token,
                    email: entry
                        .get("email")
                        .and_then(|x| x.as_str())
                        .map(str::to_string),
                    expired: expires <= now,
                },
            ));
        }
    }
    best.map(|b| b.1)
}

pub fn parse_usage(v: &serde_json::Value) -> Vec<LimitWindow> {
    let Some(config) = v.get("config") else {
        return Vec::new();
    };
    let period = config.get("currentPeriod");
    let resets = iso_ms(period.and_then(|p| p.get("end")));
    let start = iso_ms(period.and_then(|p| p.get("start")));
    let weekly = period
        .and_then(|p| p.get("type"))
        .and_then(|x| x.as_str())
        .map(|t| t.contains("WEEKLY"))
        .unwrap_or(false);
    let span = match (start, resets) {
        (Some(s), Some(e)) if e > s => Some(e - s),
        _ => weekly.then_some(7 * DAY_MS),
    };
    let mut out = Vec::new();
    if let Some(pct) = num(config.get("creditUsagePercent")) {
        let mut w = LimitWindow::percent(
            "credits",
            if weekly { "Weekly credits" } else { "Credits" },
            pct,
        )
        .resets(resets);
        w.span_ms = span;
        out.push(w);
    }
    for product in config
        .get("productUsage")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(pct)) = (
            product.get("product").and_then(|x| x.as_str()),
            num(product.get("usagePercent")),
        ) else {
            continue;
        };
        out.push(
            LimitWindow::percent(&format!("product.{name}"), name, pct)
                .resets(resets)
                .grouped("Products"),
        );
    }
    out
}

#[async_trait::async_trait]
impl UsageProvider for Grok {
    fn id(&self) -> &str {
        "grok"
    }
    fn label(&self) -> &'static str {
        "Grok"
    }

    async fn detect(&self) -> bool {
        auth_path().map(|p| p.is_file()).unwrap_or(false)
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let session = auth_path()
            .as_deref()
            .and_then(crate::limits_strip::read_json)
            .as_ref()
            .and_then(|v| pick_session(v, now_ms()))
            .ok_or_else(|| ReadError::NeedsAuth("no xAI session in the Grok CLI".into()))?;
        if session.expired {
            return Err(ReadError::NeedsAuth(
                "session expired: run grok once to renew it".into(),
            ));
        }
        let resp = http()
            .get(ENDPOINT)
            .bearer_auth(session.token.expose())
            .header("X-XAI-Token-Auth", "xai-grok-cli")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(net_err)?;
        let v = json_or_error(resp).await?;
        Ok(Reading {
            windows: parse_usage(&v),
            account: session.email,
            ..Default::default()
        })
    }
}
