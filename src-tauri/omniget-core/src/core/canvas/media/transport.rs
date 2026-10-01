use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::schema::{self, GenRequest, JobStatus, ModelSchema, ModelSummary, Spend};
use crate::core::assist::media_provider::Cli;

#[async_trait]
pub trait MediaTransport: Send + Sync {
    async fn models(&self, kind: &str) -> Result<Vec<ModelSummary>, String>;
    async fn schema(&self, model: &str) -> Result<ModelSchema, String>;
    async fn cost(&self, schema: &ModelSchema, req: &GenRequest) -> Result<f64, String>;
    async fn upload(&self, file: &Path) -> Result<String, String>;
    async fn submit(
        &self,
        schema: &ModelSchema,
        req: &GenRequest,
    ) -> Result<Option<String>, String>;
    async fn status(&self, id: &str) -> Result<JobStatus, String>;
    async fn recent(&self, size: u32) -> Result<Vec<JobStatus>, String>;
    async fn balance(&self) -> Result<(f64, String), String>;
    async fn spends(&self) -> Result<Vec<Spend>, String>;
}

pub struct CliTransport {
    cli: Cli,
}

fn tail(args: &[&str]) -> Vec<String> {
    args.iter()
        .map(|s| s.to_string())
        .chain(["--json".to_string(), "--no-color".to_string()])
        .collect()
}

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.is_absolute() {
                out.push(dir.join("higgsfield"));
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".local").join("bin").join("higgsfield"));
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin"] {
        out.push(PathBuf::from(d).join("higgsfield"));
    }
    out
}

impl CliTransport {
    pub fn locate() -> Result<Self, String> {
        if let Ok(db) = crate::core::assist::db::global() {
            if let Ok(cli) = crate::core::assist::media_tools::configured_cli(&db) {
                return Ok(Self { cli });
            }
        }
        for c in candidates() {
            if !c.is_file() {
                continue;
            }
            let Ok(real) = c.canonicalize() else {
                continue;
            };
            let Ok(interpreter) =
                crate::core::assist::media_tools::resolve_interpreter(&real, c.parent())
            else {
                continue;
            };
            if let Ok(cli) = Cli::installed_with(&real, interpreter) {
                return Ok(Self { cli });
            }
        }
        Err("PROVIDER_CLI_UNAVAILABLE: install the Higgsfield CLI (higgsfield) and run `higgsfield auth login`".into())
    }

    async fn run(&self, args: Vec<String>) -> Result<Value, String> {
        self.cli.run(args).await
    }
}

#[async_trait]
impl MediaTransport for CliTransport {
    async fn models(&self, kind: &str) -> Result<Vec<ModelSummary>, String> {
        let flag = format!("--{kind}");
        let raw = self.run(tail(&["model", "list", &flag])).await?;
        schema::parse_model_list(&raw, kind)
    }

    async fn schema(&self, model: &str) -> Result<ModelSchema, String> {
        if !schema::identifier(model) {
            return Err("PROVIDER_INVALID_MODEL".into());
        }
        let raw = self.run(tail(&["model", "get", model])).await?;
        schema::parse_model(&raw, model)
    }

    async fn cost(&self, s: &ModelSchema, req: &GenRequest) -> Result<f64, String> {
        let raw = self.run(schema::args("cost", s, req)?).await?;
        schema::credits(&raw)
    }

    async fn upload(&self, file: &Path) -> Result<String, String> {
        if !file.is_absolute() || !file.is_file() {
            return Err("PROVIDER_INVALID_REFERENCE".into());
        }
        let path = file.to_str().ok_or("PROVIDER_INVALID_REFERENCE")?;
        let raw = self
            .run(vec![
                "upload".into(),
                "create".into(),
                path.into(),
                "--json".into(),
                "--no-color".into(),
            ])
            .await?;
        schema::parse_upload(&raw)
    }

    async fn submit(&self, s: &ModelSchema, req: &GenRequest) -> Result<Option<String>, String> {
        let raw = self.run(schema::args("create", s, req)?).await?;
        Ok(schema::submitted_id(&raw))
    }

    async fn status(&self, id: &str) -> Result<JobStatus, String> {
        if !schema::identifier(id) {
            return Err("PROVIDER_INVALID_ID".into());
        }
        let raw = self.run(tail(&["generate", "get", id])).await?;
        let job = schema::parse_job(&raw).ok_or("PROVIDER_INVALID_RESPONSE")?;
        if job.id != id {
            return Err("PROVIDER_ID_MISMATCH".into());
        }
        Ok(job)
    }

    async fn recent(&self, size: u32) -> Result<Vec<JobStatus>, String> {
        let size = size.clamp(1, 100).to_string();
        let raw = self
            .run(tail(&["generate", "list", "--image", "--size", &size]))
            .await?;
        Ok(schema::parse_jobs(&raw))
    }

    async fn balance(&self) -> Result<(f64, String), String> {
        let raw = self.run(tail(&["account", "status"])).await?;
        schema::parse_balance(&raw)
    }

    async fn spends(&self) -> Result<Vec<Spend>, String> {
        let raw = self
            .run(tail(&["account", "transactions", "--size", "30"]))
            .await?;
        Ok(schema::parse_spends(&raw))
    }
}
