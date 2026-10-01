use crate::limits_strip::{
    http, iso_ms, json_or_error, net_err, now_ms, num, LimitWindow, ReadError, Reading, Secret,
    UsageProvider, DAY_MS, HOUR_MS,
};
use std::path::PathBuf;

const ENDPOINT: &str = "https://api.kimi.com/coding/v1/usages";

pub struct Kimi;

pub fn credential_path() -> Option<PathBuf> {
    let base = std::env::var_os("KIMI_CODE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::limits_strip::home().map(|h| h.join(".kimi-code")))?;
    Some(base.join("credentials").join("kimi-code.json"))
}

#[derive(Debug, PartialEq)]
pub struct Credential {
    token: Secret,
    pub expires_at: i64,
}

pub fn parse_credential(v: &serde_json::Value) -> Option<Credential> {
    Some(Credential {
        token: Secret::new(v.get("access_token")?.as_str()?)?,
        expires_at: num(v.get("expires_at")).unwrap_or(0.0) as i64 * 1000,
    })
}

fn counted(id: &str, label: &str, span: i64, detail: &serde_json::Value) -> Option<LimitWindow> {
    let used = num(detail.get("used"))
        .or_else(|| Some(num(detail.get("limit"))? - num(detail.get("remaining"))?))?;
    let limit = num(detail.get("limit")).unwrap_or(0.0);
    Some(
        LimitWindow::ratio(id, label, used, limit, "requests")
            .resets(iso_ms(detail.get("resetTime")))
            .span(span),
    )
}

pub fn parse_usage(v: &serde_json::Value) -> (Vec<LimitWindow>, Option<String>) {
    let mut out = Vec::new();
    for l in v
        .get("limits")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        let unit = l
            .pointer("/window/timeUnit")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let duration = num(l.pointer("/window/duration")).unwrap_or(0.0) as i64;
        let Some(detail) = l.get("detail") else {
            continue;
        };
        let w = match (unit, duration) {
            ("TIME_UNIT_MINUTE", 300) | ("TIME_UNIT_HOUR", 5) => {
                counted("rolling", "5h limit", 5 * HOUR_MS, detail)
            }
            ("TIME_UNIT_WEEK", 1) | ("TIME_UNIT_DAY", 7) => {
                counted("weekly", "Weekly limit", 7 * DAY_MS, detail)
            }
            _ => None,
        };
        out.extend(w);
    }
    if !out.iter().any(|w| w.id == "weekly") {
        out.extend(
            v.get("usage")
                .and_then(|u| counted("weekly", "Weekly limit", 7 * DAY_MS, u)),
        );
    }
    let plan = v
        .pointer("/user/membership/level")
        .and_then(|x| x.as_str())
        .map(|l| {
            let l = l.trim_start_matches("LEVEL_").to_lowercase();
            let mut c = l.chars();
            c.next()
                .map(|f| format!("{}{}", f.to_uppercase(), c.as_str()))
                .unwrap_or_default()
        });
    (out, plan)
}

#[async_trait::async_trait]
impl UsageProvider for Kimi {
    fn id(&self) -> &str {
        "kimi"
    }
    fn label(&self) -> &'static str {
        "Kimi"
    }

    async fn detect(&self) -> bool {
        credential_path().map(|p| p.is_file()).unwrap_or(false)
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let cred = credential_path()
            .as_deref()
            .and_then(crate::limits_strip::read_json)
            .as_ref()
            .and_then(parse_credential)
            .ok_or_else(|| ReadError::NeedsAuth("no Kimi Code login".into()))?;
        if cred.expires_at <= now_ms() {
            return Err(ReadError::NeedsAuth(
                "session expired: run kimi once to renew it".into(),
            ));
        }
        let resp = http()
            .get(ENDPOINT)
            .bearer_auth(cred.token.expose())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(net_err)?;
        if resp.status().as_u16() == 404 {
            return Ok(Reading {
                note: Some("no Kimi Code plan on this account".into()),
                ..Default::default()
            });
        }
        let (windows, plan) = parse_usage(&json_or_error(resp).await?);
        Ok(Reading {
            windows,
            plan,
            ..Default::default()
        })
    }
}
