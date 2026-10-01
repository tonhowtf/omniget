use crate::limits_strip::{json_or_error, LocalModel, ReadError, Reading, UsageProvider};

const ENDPOINT: &str = "http://127.0.0.1:1234/v1/models";

pub struct LmStudio;

pub fn parse_models(v: &serde_json::Value) -> Vec<LocalModel> {
    v.get("data")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id")?.as_str())
        .map(|id| LocalModel {
            name: id.to_string(),
            ..Default::default()
        })
        .collect()
}

#[async_trait::async_trait]
impl UsageProvider for LmStudio {
    fn id(&self) -> &str {
        "lmstudio"
    }
    fn label(&self) -> &'static str {
        "LM Studio"
    }
    fn local(&self) -> bool {
        true
    }
    fn beta(&self) -> bool {
        false
    }

    async fn detect(&self) -> bool {
        super::installed(".lmstudio", "lms")
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let Ok(resp) = super::loopback().get(ENDPOINT).send().await else {
            return Ok(super::not_running());
        };
        let local_models = parse_models(&json_or_error(resp).await?);
        Ok(Reading {
            note: local_models
                .is_empty()
                .then(|| "running, no model available".to_string()),
            local_models,
            ..Default::default()
        })
    }
}
