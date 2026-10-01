use anyhow::anyhow;
use serde_json::{json, Value};

use super::transcript::fmt_clock;
use super::Meeting;

fn text(t: &str, extra: Value) -> Value {
    let mut v = json!({ "type": "TextBlock", "text": t, "wrap": true });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in e {
            o.insert(k.clone(), val.clone());
        }
    }
    v
}

fn section(body: &mut Vec<Value>, title: &str, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    body.push(text(
        title,
        json!({ "weight": "Bolder", "spacing": "Medium" }),
    ));
    body.push(text(&lines.join("\n"), json!({ "spacing": "Small" })));
}

pub fn card(m: &Meeting) -> Value {
    let mut body = vec![text(
        &m.title,
        json!({ "size": "Large", "weight": "Bolder" }),
    )];
    let mut meta = Vec::new();
    if let Some(d) = chrono::DateTime::from_timestamp_millis(m.created_at_ms as i64) {
        meta.push(
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
        );
    }
    if m.duration_ms > 0 {
        meta.push(fmt_clock(m.duration_ms));
    }
    if !m.speakers.is_empty() {
        meta.push(m.speakers.join(", "));
    }
    if !meta.is_empty() {
        body.push(text(
            &meta.join(" · "),
            json!({ "isSubtle": true, "spacing": "None" }),
        ));
    }
    if let Some(s) = &m.summary {
        if !s.overview.is_empty() {
            body.push(text(&s.overview, json!({})));
        }
        let bullets = |v: &[String]| v.iter().map(|x| format!("- {x}")).collect::<Vec<_>>();
        section(&mut body, "Decisions", bullets(&s.decisions));
        section(
            &mut body,
            "Action items",
            s.action_items
                .iter()
                .map(|a| {
                    let mut line = format!("- {}", a.task);
                    if let Some(o) = &a.owner {
                        line.push_str(&format!(" — **{o}**"));
                    }
                    if let Some(d) = &a.due {
                        line.push_str(&format!(" ({d})"));
                    }
                    line
                })
                .collect(),
        );
        section(&mut body, "Open questions", bullets(&s.open_questions));
    }
    body.push(text(
        "Notes by OmniGet",
        json!({ "isSubtle": true, "size": "Small", "spacing": "Medium" }),
    ));
    json!({
        "type": "message",
        "attachments": [{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "contentUrl": null,
            "content": {
                "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                "type": "AdaptiveCard",
                "version": "1.4",
                "body": body
            }
        }]
    })
}

pub async fn post_to_teams(m: &Meeting) -> anyhow::Result<()> {
    if m.summary.is_none() {
        return Err(anyhow!("Summarize the meeting before posting it to Teams"));
    }
    let url = super::settings::teams_webhook()
        .ok_or_else(|| anyhow!("Add a Teams webhook URL in Meetings settings first"))?;
    let resp = crate::core::tools::client()?
        .post(&url)
        .json(&card(m))
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Teams answered HTTP {}: {}",
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        ));
    }
    Ok(())
}
