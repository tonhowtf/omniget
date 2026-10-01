use anyhow::bail;
use serde_json::Value;

fn path_of(since: &str, action: Option<&str>, limit: usize) -> anyhow::Result<String> {
    if omniget_core::core::llm::audit::parse_since(since).is_none() {
        bail!("--since must look like 1d, 12h, 30m or 2w (got `{since}`)");
    }
    let mut path = format!("/v1/audit?since={since}&limit={}", limit.clamp(1, 5000));
    if let Some(a) = action.map(str::trim).filter(|a| !a.is_empty()) {
        if !a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
        {
            bail!("--action is a prefix like job. or ask.answered");
        }
        path.push_str(&format!("&action={a}"));
    }
    Ok(path)
}

fn line(r: &Value) -> String {
    let when = r["ts_ms"]
        .as_i64()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default();
    let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
    format!(
        "{when}  {:<16} {:<22} {:<8} {}:{}",
        s("action"),
        s("actor"),
        s("channel"),
        s("entity_kind"),
        s("entity_id")
    )
}

pub async fn execute(
    since: String,
    action: Option<String>,
    limit: usize,
    json_out: bool,
) -> anyhow::Result<()> {
    let path = path_of(&since, action.as_deref(), limit)?;
    let bridge = super::agent::Bridge::open()?;
    let rows = bridge.get(&path).await?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    let list = rows.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        println!("no audit lines in the last {since}");
        return Ok(());
    }
    for r in list.iter().rev() {
        println!("{}", line(r));
    }
    Ok(())
}
