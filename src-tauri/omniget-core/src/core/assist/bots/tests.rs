//! Bot identity, skills and capability tests. The turn tests drive the real
//! `Coordinator` and `ToolBroker` with a scripted runtime that behaves like a
//! model which calls tools: it reads the tool name from the system message,
//! reads the file list from the tool result, and answers with what the last
//! tool result contained. No real model is involved.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::augment::{self, BotsAugment};
use super::skills::{self, SkillProjection};
use super::BotEnv;
use crate::core::assist::db::AssistDb;
use crate::core::llm::agent::{AgentDef, AgentRole, Budget, GrantMode, ModelPolicy, RuntimeKind};
use crate::core::llm::broker::{ToolBroker, ToolExecutor};
use crate::core::llm::coordinator::TurnAugment;
use crate::core::llm::error::LlmError;
use crate::core::llm::types::{ModelRef, ProviderId, ToolSpec};
use crate::core::omni::bus::Bus;
use crate::core::skills::install;
use crate::core::skills::scan::ScanStatus;

// ── Fixture ────────────────────────────────────────────────────────────────

struct Fx {
    dir: PathBuf,
    root: PathBuf,
    db: Arc<AssistDb>,
    env: BotEnv,
    proj: Arc<SkillProjection>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Runs whatever internal tool it is asked; stands in for the app's table.
struct Echo;

#[async_trait]
impl ToolExecutor for Echo {
    async fn execute(&self, name: &str, input: Value) -> Result<String, LlmError> {
        Ok(format!("{name} ran with {input}"))
    }
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: format!("{name} (test)"),
        input_schema: json!({ "type": "object" }),
    }
}

fn fx() -> Fx {
    let dir = std::env::temp_dir().join(format!("omniget-bots-{}", uuid::Uuid::new_v4()));
    let root = dir.join("skills");
    std::fs::create_dir_all(&root).unwrap();
    let db = Arc::new(AssistDb::open_in_memory().unwrap());
    let broker = Arc::new(ToolBroker::new(
        vec![spec("shell_exec"), spec("fake_lookup")],
        Arc::new(Echo),
        Arc::new(Bus::new()),
    ));
    let env = BotEnv::new(db.clone(), root.clone(), Some(broker.clone()));
    let proj = SkillProjection::new(env.clone());
    Fx {
        dir,
        root,
        db,
        env,
        proj,
    }
}

/// Writes a skill folder under `<fx>/src/<name>` and returns its path.
fn write_skill(
    fx: &Fx,
    name: &str,
    front_extra: &str,
    body: &str,
    files: &[(&str, &str)],
) -> PathBuf {
    let src = fx.dir.join("src").join(name);
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Picks the next book for the reader.\n{front_extra}---\n\n{body}\n"),
    )
    .unwrap();
    for (rel, content) in files {
        let p = src.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    src
}

/// Installs through the same code path as the app (copy, sidecar, record,
/// re-register the broker source).
fn install(fx: &Fx, src: &Path) -> skills::ProjectionSummary {
    let outcome =
        install::install_from_dir_in_with(&fx.root, src, &|_| ScanStatus::NotScanned).unwrap();
    assert!(!outcome.is_pending());
    skills::skills_changed(&fx.proj, Some(&outcome.manifest.name)).unwrap()
}

fn agent(id: &str, skills: &[&str]) -> AgentDef {
    AgentDef {
        id: id.into(),
        name: "Reading companion".into(),
        role: AgentRole::Worker,
        system_prompt: String::new(),
        model: ModelPolicy::Fixed {
            model: ModelRef {
                provider: ProviderId::new("fake"),
                model: "m".into(),
            },
        },
        tools: vec![],
        skills: skills.iter().map(|s| s.to_string()).collect(),
        budget: Budget::default(),
        runtime: RuntimeKind::Native,
        skin: None,
    }
}

fn run_augment(fx: &Fx, a: &AgentDef, conv: &str) -> (AgentDef, String) {
    let hook = BotsAugment::new(fx.env.clone(), fx.proj.clone());
    let mut eff = a.clone();
    let text = hook.augment(&mut eff, conv, "hello").unwrap_or_default();
    (eff, text)
}

fn granted(a: &AgentDef) -> Vec<String> {
    a.tools
        .iter()
        .filter(|g| !matches!(g.mode, GrantMode::Deny))
        .map(|g| crate::core::llm::broker::grant_key(&g.source))
        .collect()
}

fn skill_tool(a: &AgentDef) -> Option<String> {
    granted(a).into_iter().find(|t| t.starts_with("skill__"))
}

// ── A scripted "model" that calls tools ────────────────────────────────────

// ── Reproduction (Etapa 0) ─────────────────────────────────────────────────

// ── A01 ────────────────────────────────────────────────────────────────────

// ── A02 ────────────────────────────────────────────────────────────────────

// ── A03 ────────────────────────────────────────────────────────────────────

// ── A20 ────────────────────────────────────────────────────────────────────

// ── B07 ────────────────────────────────────────────────────────────────────

// ── Capabilities: ticked ≠ available ───────────────────────────────────────

#[test]
fn legacy_roster_skills_become_bindings_once() {
    let fx = fx();
    install(&fx, &write_skill(&fx, "curator", "", "Body.", &[]));
    let a = agent("old-bot", &["curator", "gone"]);
    let (eff, _) = run_augment(&fx, &a, "c");
    assert!(skill_tool(&eff).is_some());
    let b = skills::bindings(&fx.db, "old-bot").unwrap();
    assert_eq!(b.len(), 2);
    assert!(b.iter().any(|b| b.skill == "gone" && b.hash.is_empty()));
    // The table is the truth from now on.
    skills::unbind(&fx.db, "old-bot", "curator").unwrap();
    let (eff, _) = run_augment(&fx, &a, "c");
    assert!(skill_tool(&eff).is_none());
    let _ = augment::ensure_profile(&fx.env, &a).unwrap();
}
