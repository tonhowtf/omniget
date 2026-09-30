//! The city's LLM roles (M3 §3): the mayor (plans streets, zones and civic
//! works), the council (votes on large works) and the residents (ask for
//! small works). Each role has its own system prompt and a token budget per
//! period kept in the store. A round reads the state, proposes ops, validates
//! them, lets the council vote when the work is large, and applies or leaves
//! it for review according to the city's `publish_policy`. A provider that
//! drops mid-round only ends the round: nothing is written before `apply`.

use omniget_citygen::program::{CityProgram, PatchOp, Proposal};
use omniget_citygen::{Charter, Violation};
use serde::Serialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::api;
use super::llm::{self, Llm};
use super::store::{self, Author, RoleBudget, DAY_MS};
use super::{docs, ERR};

pub const ROLES: &[&str] = &["prefeitura", "conselho", "morador"];
/// A role with less than this left in the period skips its turn.
const MIN_TURN_TOKENS: u64 = 3000;
const MAX_ANSWER_TOKENS: u32 = 3000;

pub const PREFEITURA: &str = "You are the mayor's office (prefeitura) of a stylised 3D city. You keep the master plan: zones, streets, landmarks, civic buildings and trees. Each round you may propose at most 4 patch ops for ONE region that make the city better to live in (a missing street link, a plaza, a school, a market, trees along an avenue), or none when the region is fine. Never touch lots that have an owner_ref. Prefer small, connected steps; the building code rejects anything that breaks a door, a street or a gap. Every new item needs a new semantic id. Example answer: {\"reason\": \"shade along the avenue\", \"ops\": [{\"op\": \"add_props\", \"kind\": \"arvore\", \"preset\": \"oak-estilizada\", \"scatter\": {\"zone\": \"<zone id>\", \"on\": \"ground\", \"spacing\": 6}}, {\"op\": \"add_building\", \"id\": \"escola\", \"lot\": \"auto:zone:<zone id>\", \"intent\": {\"use\": \"civico\", \"floors\": 2, \"shape\": \"L\", \"roof\": \"quatro-aguas\", \"label\": \"Escola\"}}]}";

pub const CONSELHO: &str = "You are the city council (conselho). You vote on large works proposed by the mayor: approve when the work serves the residents, keeps the city's character (charter story, style) and leaves the street network whole; refuse when it demolishes without reason, crowds the region or ignores what residents asked. You see the proposal, its validation summary and the state. Answer ONE JSON object {\"approve\": true|false, \"reason\": \"one sentence\"}.";

pub const MORADOR: &str = "You are a resident (morador) of a stylised 3D city. You ask for ONE small work that your daily life needs: a house or a small shop (loja-sobrado) in a free lot, a bench or trees near your street, or a small change (set_intent) on a building you use. Use only free lots listed in the options. At most 2 ops. Example answer: {\"reason\": \"I need a home near the market\", \"ops\": [{\"op\": \"add_building\", \"id\": \"casa-da-ana\", \"lot\": \"auto:zone:<zone id>\", \"intent\": {\"use\": \"casa\", \"floors\": 1, \"shape\": \"rect\", \"roof\": \"duas-aguas\"}}]}";

pub fn prompt(role: &str) -> &'static str {
    match role {
        "prefeitura" => PREFEITURA,
        "conselho" => CONSELHO,
        _ => MORADOR,
    }
}

// ── Budget ─────────────────────────────────────────────────────────────

fn fresh(role: &str, now: u64) -> RoleBudget {
    RoleBudget {
        limit_tokens: store::default_limit(role),
        used_tokens: 0,
        period_start_ms: now,
        period_ms: DAY_MS,
    }
}

/// Adds spent tokens to a role, rolling the period over when it ended.
pub fn charge(city_id: &str, role: &str, tokens: u64) {
    let _g = store::lock();
    let mut all = store::load_budgets(city_id);
    let now = store::now_ms();
    let b = all
        .entry(role.to_string())
        .or_insert_with(|| fresh(role, now));
    if now >= b.period_start_ms + b.period_ms {
        b.period_start_ms = now;
        b.used_tokens = 0;
    }
    b.used_tokens += tokens;
    let _ = store::save_budgets(city_id, &all);
}

pub fn remaining(city_id: &str, role: &str) -> u64 {
    let now = store::now_ms();
    let all = store::load_budgets(city_id);
    match all.get(role) {
        Some(b) if now < b.period_start_ms + b.period_ms => {
            b.limit_tokens.saturating_sub(b.used_tokens)
        }
        Some(b) => b.limit_tokens,
        None => store::default_limit(role),
    }
}

pub fn set_limit(city_id: &str, role: &str, limit_tokens: u64) -> Result<(), String> {
    let _g = store::lock();
    let mut all = store::load_budgets(city_id);
    let now = store::now_ms();
    all.entry(role.to_string())
        .or_insert_with(|| fresh(role, now))
        .limit_tokens = limit_tokens;
    store::save_budgets(city_id, &all)
}

// ── The round ──────────────────────────────────────────────────────────

#[derive(Serialize, Clone, Debug, Default)]
pub struct RoleStep {
    pub role: String,
    /// applied | pending_review | rejected | refused | nothing | skipped | error
    pub state: String,
    pub reason: String,
    pub log_id: Option<String>,
    pub violations: Vec<Violation>,
    pub tokens: u64,
    pub message: String,
}

struct State {
    city_id: String,
    revision: String,
    text: String,
}

async fn read_state(region: [i32; 2]) -> Result<State, String> {
    let status = api::status().await?;
    let charter: Charter = serde_json::from_value(status["city"]["charter"].clone())
        .map_err(|_| format!("{ERR}_NOT_FOUNDED: this city was not founded yet"))?;
    let city_id = charter.city_id.clone();
    let r = api::region(region).await?;
    let program: CityProgram = serde_json::from_value(r["program"].clone())
        .unwrap_or_else(|_| CityProgram::empty(&city_id, region));
    let revision = r["revision"].as_str().unwrap_or_default().to_string();
    let codes: Vec<String> = r["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            format!(
                "{} {} {}",
                v["code"].as_str().unwrap_or(""),
                v["id"].as_str().unwrap_or(""),
                v["measure"].as_str().unwrap_or("")
            )
        })
        .take(20)
        .collect();
    let log = api::log(12).await.unwrap_or(json!({}));
    let recent: Vec<Value> = log["proposals"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            json!({
                "role": e["author"]["role"],
                "state": e["state"],
                "reason": e["reason"],
            })
        })
        .collect();
    let options = {
        let (c, p) = (charter.clone(), program.clone());
        tokio::task::spawn_blocking(move || docs::options(&c, &p))
            .await
            .map_err(|e| format!("{ERR}_TASK: {e}"))?
    };
    let text = format!(
        "CITY: {} ({}), style {}, policy {}.\nSTORY: {}\nREGION {:?} at revision {}.\nPROGRAM: {}\nCURRENT CHECKER NOTES: {:?}\nWORKS IN PROGRESS: {}\nRECENT PROPOSALS: {}\nOPTIONS: {}",
        charter.name,
        charter.city_id,
        charter.style,
        charter.publish_policy,
        charter.story,
        region,
        revision,
        serde_json::to_string(&program).unwrap_or_default(),
        codes,
        log["works"],
        serde_json::to_string(&recent).unwrap_or_default(),
        options,
    );
    Ok(State {
        city_id,
        revision,
        text,
    })
}

/// Ops one by one, so the model learns which one was wrong.
fn parse_ops(v: &Value) -> Result<Vec<PatchOp>, String> {
    let Some(list) = v.as_array() else {
        return Err("\"ops\" must be an array".into());
    };
    list.iter()
        .enumerate()
        .map(|(i, op)| {
            serde_json::from_value(op.clone()).map_err(|e| {
                let text: String = op.to_string().chars().take(200).collect();
                format!("ops[{i}] {text}: {e}")
            })
        })
        .collect()
}

fn is_large(role: &str, ops: &[PatchOp]) -> bool {
    role == "prefeitura"
        || ops.len() > 2
        || ops.iter().any(|op| {
            matches!(
                op,
                PatchOp::AddZone(_)
                    | PatchOp::AddStreet(_)
                    | PatchOp::AddLandmark(_)
                    | PatchOp::AddWater(_)
                    | PatchOp::Remove { .. }
            )
        })
}

fn author(llm: &Llm, role: &str) -> Author {
    Author {
        kind: "llm".into(),
        name: llm.label(),
        role: Some(role.into()),
    }
}

/// One role proposes, fixes once on violations, and (for large works) the
/// council votes before `apply`.
async fn role_turn(
    llm: &Llm,
    role: &str,
    region: [i32; 2],
    cancel: &CancellationToken,
) -> Result<RoleStep, String> {
    let mut step = RoleStep {
        role: role.into(),
        ..Default::default()
    };
    let st = read_state(region).await?;
    if remaining(&st.city_id, role) < MIN_TURN_TOKENS {
        step.state = "skipped".into();
        step.message = "token budget of this period is spent".into();
        return Ok(step);
    }
    let ask_ops = format!(
        "{}\n\nFORMAT:\n{}\n\nAnswer ONE JSON object {{\"reason\": \"one short sentence for the city log\", \"ops\": [ ...patch ops... ]}}. Use \"ops\": [] when nothing is needed.",
        st.text,
        docs::FORMAT
    );
    let mut messages = vec![llm::system(prompt(role)), llm::user(ask_ops)];
    let mut proposal = None;
    let mut failed: Option<Proposal> = None;
    let mut reason = String::new();
    for attempt in 0..2 {
        let answer = llm::ask(llm, messages.clone(), MAX_ANSWER_TOKENS, cancel.clone()).await?;
        step.tokens += answer.tokens();
        charge(&st.city_id, role, answer.tokens());
        let v = match llm::extract_json(&answer.text) {
            Ok(v) => v,
            Err(e) => {
                step.state = "error".into();
                step.message = format!("unusable answer: {e}");
                return Ok(step);
            }
        };
        reason = v["reason"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(280)
            .collect();
        let ops = match parse_ops(&v["ops"]) {
            Ok(ops) => ops,
            Err(e) if attempt == 0 => {
                step.message = e.clone();
                messages.push(llm::assistant(answer.text));
                messages.push(llm::user(format!(
                    "Your ops did not parse: {e}. Every op has \"op\" plus the fields of its kind exactly as in FORMAT (add_zone, add_street, add_landmark, add_water and add_building need an \"id\"; add_props has kind, preset and cells or scatter). Answer again in the same JSON format."
                )));
                continue;
            }
            Err(e) => {
                step.state = "error".into();
                step.message = e;
                return Ok(step);
            }
        };
        if ops.is_empty() {
            step.state = "nothing".into();
            step.reason = reason;
            return Ok(step);
        }
        let p = Proposal {
            region,
            base_revision: st.revision.clone(),
            ops,
            admin: false,
        };
        let result = api::validate(p.clone(), author(llm, role)).await?;
        if !store::has_errors(&result.violations) {
            proposal = Some(p);
            break;
        }
        step.violations = result.violations.clone();
        failed = Some(p);
        if attempt == 0 {
            messages.push(llm::assistant(answer.text));
            messages.push(llm::user(format!(
                "The building code rejected it: {}. Fix the ops (or answer \"ops\": []) in the same JSON format.",
                serde_json::to_string(&result.violations).unwrap_or_default()
            )));
        }
    }
    let Some(p) = proposal else {
        // Logged so the inspector shows what was tried and why it failed.
        let tried = failed.unwrap_or(Proposal {
            region,
            base_revision: st.revision.clone(),
            ops: Vec::new(),
            admin: false,
        });
        let out = api::propose(tried, author(llm, role), reason.clone())
            .await
            .ok();
        step.state = "rejected".into();
        step.reason = reason;
        step.log_id = out.and_then(|o| o.log_id);
        return Ok(step);
    };

    if is_large(role, &p.ops) && role != "conselho" {
        if remaining(&st.city_id, "conselho") >= MIN_TURN_TOKENS {
            let preview = api::validate(p.clone(), author(llm, role)).await?;
            let ask = format!(
                "{}\n\nPROPOSAL by {role}: reason \"{reason}\", ops {}\nVALIDATION: {} violations (0 errors), summary {}",
                st.text,
                serde_json::to_string(&p.ops).unwrap_or_default(),
                preview.violations.len(),
                serde_json::to_string(&preview.summary).unwrap_or_default(),
            );
            let answer = llm::ask(
                llm,
                vec![llm::system(CONSELHO), llm::user(ask)],
                600,
                cancel.clone(),
            )
            .await?;
            step.tokens += answer.tokens();
            charge(&st.city_id, "conselho", answer.tokens());
            let vote = llm::extract_json(&answer.text).unwrap_or(json!({}));
            if vote["approve"].as_bool() != Some(true) {
                let why = vote["reason"]
                    .as_str()
                    .unwrap_or("the council refused")
                    .to_string();
                let out = api::propose(p, author(llm, role), reason.clone()).await?;
                if let Some(id) = &out.log_id {
                    let _ = api::refuse(id.clone(), format!("council: {why}")).await;
                }
                step.state = "refused".into();
                step.reason = reason;
                step.message = why;
                step.log_id = out.log_id;
                return Ok(step);
            }
            step.message = format!("council: {}", vote["reason"].as_str().unwrap_or("approved"));
        } else {
            step.message = "council budget spent: left for review".into();
        }
    }

    let out = api::apply(
        p,
        author(llm, role),
        reason.clone(),
        None,
        Some(step.tokens),
    )
    .await?;
    step.state = out.state;
    step.reason = reason;
    step.log_id = out.log_id;
    step.violations = out.result.violations;
    Ok(step)
}

/// One council round. `roles` defaults to a resident then the mayor.
pub async fn council_tick(
    llm: Llm,
    region: [i32; 2],
    roles: Vec<String>,
    cancel: CancellationToken,
) -> Result<Value, String> {
    let roles = if roles.is_empty() {
        vec!["morador".to_string(), "prefeitura".to_string()]
    } else {
        roles
    };
    let mut steps = Vec::new();
    for role in roles {
        if !ROLES.contains(&role.as_str()) {
            return Err(format!("{ERR}_BAD_ROLE: {role}"));
        }
        if cancel.is_cancelled() {
            break;
        }
        match role_turn(&llm, &role, region, &cancel).await {
            Ok(step) => steps.push(step),
            Err(e) => steps.push(RoleStep {
                role,
                state: "error".into(),
                message: e,
                ..Default::default()
            }),
        }
    }
    Ok(json!({ "region": region, "model": llm.label(), "steps": steps }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Example city + one council round against a local Ollama, then a
    /// construction tick: `cargo test -p omniget-core live_council -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_council_with_ollama() {
        use crate::core::llm::providers::openai_compat::OpenAiCompat;
        use crate::core::llm::types::{ModelRef, ProviderId};
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("city3d-council-{}", uuid::Uuid::new_v4()));
        store::set_root(Some(dir.clone()));
        let (charter, program, tried) = api::example_founding("vila-teste", "omni-live");
        eprintln!(
            "example seed {}: {:?}",
            charter.seed,
            tried
                .iter()
                .map(|v| format!("{} {} {} {:?}", v.code, v.id, v.measure, v.severity))
                .collect::<Vec<_>>()
        );
        let founded = api::found(charter, vec![program], Author::person(), None)
            .await
            .unwrap();
        eprintln!(
            "founded: {} {:?}",
            founded.founded,
            founded
                .violations
                .iter()
                .map(|v| &v.code)
                .collect::<Vec<_>>()
        );
        let model = std::env::var("CITY3D_LIVE_MODEL").unwrap_or_else(|_| "qwen3:8b".into());
        let provider = OpenAiCompat::new(
            ProviderId::new("ollama"),
            "http://127.0.0.1:11434/v1",
            String::new(),
        )
        .unwrap();
        let llm = Llm::new(
            Arc::new(provider),
            ModelRef {
                provider: ProviderId::new("ollama"),
                model,
            },
        );
        let report = council_tick(llm, [0, 0], Vec::new(), CancellationToken::new())
            .await
            .unwrap();
        eprintln!("{}", serde_json::to_string_pretty(&report).unwrap());
        let mut s = store::load_settings("vila-teste");
        s.works_last_ms = store::now_ms() - 3 * 60_000;
        store::save_settings("vila-teste", &s).unwrap();
        eprintln!("works tick: {}", api::works_tick().await.unwrap());
        eprintln!("log: {}", api::log(10).await.unwrap()["works"]);
        eprintln!("store: {}", dir.display());
    }

    #[test]
    fn a_street_is_a_large_work_and_a_house_is_not() {
        let street: PatchOp = serde_json::from_value(
            json!({"op":"add_street","id":"r","class":"rua","width":2,"path":[[0,0],[5,0]]}),
        )
        .unwrap();
        let house: PatchOp = serde_json::from_value(
            json!({"op":"add_building","id":"h","lot":"auto:zone:z","intent":{"use":"casa"}}),
        )
        .unwrap();
        assert!(is_large("morador", &[street]));
        assert!(!is_large("morador", std::slice::from_ref(&house)));
        assert!(is_large("prefeitura", &[house]));
    }
}
