use omniget_core::core::roadmap::{self, RoadmapError};
use serde_json::{json, Value};

fn edge_error(e: RoadmapError) -> String {
    let next_step = match &e {
        RoadmapError::UnknownStatus(_) => "call again without status to see the columns",
        _ => "try again later, or open https://getomniget.com/contact in a browser",
    };
    json!({
        "code": e.code(),
        "message": e.to_string(),
        "retryable": e.retryable(),
        "next_step": next_step,
    })
    .to_string()
}

#[tauri::command]
pub async fn roadmap_get(status: Option<String>) -> Result<Value, String> {
    let all = roadmap::fetch().await.map_err(edge_error)?;
    let shown = match status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => roadmap::only_status(all, s).map_err(edge_error)?,
        None => all,
    };
    serde_json::to_value(shown).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_error_carries_code_retry_and_next_step() {
        let v: Value =
            serde_json::from_str(&edge_error(RoadmapError::UnknownStatus("soon".into()))).unwrap();
        assert_eq!(v["code"], "ROADMAP_UNKNOWN_STATUS");
        assert_eq!(v["retryable"], false);
        assert!(v["next_step"].as_str().unwrap().contains("without status"));
        let n: Value = serde_json::from_str(&edge_error(RoadmapError::Status(503))).unwrap();
        assert_eq!(n["retryable"], true);
    }
}
