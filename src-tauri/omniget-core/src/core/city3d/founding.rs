//! Founding by LLM (estudo 10 §7, 09 §6): the model reads the land of the
//! seed and the format, writes the charter and the first program, the
//! generator runs, and the violations (and a picture, when the model has
//! vision) go back until the checker has zero errors or the attempts run out.
//! Nothing is written before a clean attempt: a failed founding leaves no
//! city behind.

use std::sync::Arc;

use omniget_citygen::program::CityProgram;
use omniget_citygen::{charter as charter_mod, sites, terrain, Charter, Violation};
use serde::Serialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::api::{self, FoundResult};
use super::llm::{self, Llm};
use super::store::{self, Author};
use super::{docs, ERR};

pub const DEFAULT_ATTEMPTS: u32 = 5;
const MAX_ANSWER_TOKENS: u32 = 8000;

/// What `city3d:founding` carries.
#[derive(Serialize, Clone, Debug, Default)]
pub struct FoundingEvent {
    pub job: String,
    /// terrain | asking | checking | retry | founded | failed
    pub stage: String,
    pub message: String,
    pub violations: Vec<Violation>,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub attempt: u32,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city_id: Option<String>,
}

pub type Emit = Arc<dyn Fn(FoundingEvent) + Send + Sync>;

#[derive(Clone, Debug, Default)]
pub struct FoundingRequest {
    pub job: String,
    pub brief: String,
    pub city_id: Option<String>,
    pub seed: Option<String>,
    pub max_attempts: u32,
}

pub const SYSTEM: &str = "You are the founding urban planner of a stylised 3D city. You never write geometry: you write a charter (the city's identity and terrain knobs) and the declarative program of its first region, and a deterministic generator with a building code executes them. Keep the first region modest and connected: 3 to 6 streets forming one network, 1 to 3 zones, 1 or 2 landmarks that touch a street, 1 to 4 named buildings and fill rules for the rest. Answer with ONE JSON object and nothing else.";

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(40).collect();
    if out.is_empty() {
        "cidade".into()
    } else {
        out
    }
}

fn land(charter: &Charter) -> Value {
    json!({
        "terrain": terrain::summary(charter, [0, 0], 2),
        "sites": sites::sites_near(charter, [0, 0], 2),
    })
}

/// Parse `{charter, program}` (or `programs: [..]`) out of an answer.
pub fn parse_answer(
    v: &Value,
    city_id: &str,
    seed: &str,
) -> Result<(Charter, CityProgram), Vec<Violation>> {
    let bad = |what: &str, e: String| {
        vec![store::violation(
            "P-JSON",
            what,
            "",
            &format!("{what} does not match the format: {e}"),
        )]
    };
    let mut c = v["charter"].clone();
    if let Some(o) = c.as_object_mut() {
        o.insert("city_id".into(), json!(city_id));
        let s = o.get("seed").and_then(|s| s.as_str()).unwrap_or("");
        if s.trim().is_empty() {
            o.insert("seed".into(), json!(seed));
        }
        o.entry("schema").or_insert(json!("omni.city.charter/v1"));
    }
    let charter: Charter = serde_json::from_value(c).map_err(|e| bad("charter", e.to_string()))?;
    let mut p = if v["program"].is_object() {
        v["program"].clone()
    } else {
        v["programs"][0].clone()
    };
    if let Some(o) = p.as_object_mut() {
        o.insert("city_id".into(), json!(city_id));
        o.insert("region".into(), json!([0, 0]));
        o.entry("schema").or_insert(json!("omni.city.program/v1"));
    }
    let program: CityProgram =
        serde_json::from_value(p).map_err(|e| bad("program", e.to_string()))?;
    Ok((charter, program))
}

/// Run the loop. The caller picked the model (and knows it has a key).
pub async fn run(
    llm: Llm,
    req: FoundingRequest,
    emit: Emit,
    cancel: CancellationToken,
) -> Result<FoundResult, String> {
    let job = req.job.clone();
    let send =
        |stage: &str, message: String, violations: Vec<Violation>, attempt: u32, tokens: u64| {
            emit(FoundingEvent {
                job: job.clone(),
                stage: stage.into(),
                message,
                violations,
                done: false,
                error: None,
                attempt,
                tokens,
                city_id: None,
            })
        };
    let seed = req
        .seed
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("omni-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let provisional = req
        .city_id
        .clone()
        .filter(|s| store::safe_id(s))
        .unwrap_or_else(|| "nova".into());
    let attempts = if req.max_attempts == 0 {
        DEFAULT_ATTEMPTS
    } else {
        req.max_attempts.min(12)
    };

    send(
        "terrain",
        format!("reading the land of seed {seed}"),
        Vec::new(),
        0,
        0,
    );
    let base = charter_mod::example(&provisional, &seed);
    let (land0, options0, example0) = {
        let base = base.clone();
        tokio::task::spawn_blocking(move || {
            let p = docs::starter_program(&base);
            (
                land(&base),
                docs::options(&base, &p),
                serde_json::to_string(&p).unwrap_or_default(),
            )
        })
        .await
        .map_err(|e| format!("{ERR}_TASK: {e}"))?
    };

    let first = format!(
        "BRIEF from the person founding the city:\n{}\n\nSEED: {seed}\nThe land below was generated with an example charter; your relief knobs reshape it (you will see the result).\nLAND: {}\nOPTIONS (valid keys, computed for the example program): {}\nEXAMPLE CHARTER (adapt freely): {}\nEXAMPLE PROGRAM that passes the building code on flat land (a starting point, not a template to copy): {}\n\nFORMAT:\n{}\n\nAnswer ONE JSON object: {{\"charter\": {{...}}, \"program\": {{...region [0,0]...}}}}.",
        if req.brief.trim().is_empty() { "(no brief: surprise me, keep it charming)" } else { req.brief.trim() },
        land0,
        options0,
        serde_json::to_string(&base).unwrap_or_default(),
        example0,
        docs::FORMAT,
    );
    let system = llm::system(SYSTEM);
    let first_msg = llm::user(first);
    let mut last: Option<(String, crate::core::llm::types::Message)> = None;
    let mut tokens = 0u64;
    let mut last_violations = Vec::new();

    for attempt in 1..=attempts {
        if cancel.is_cancelled() {
            return Err(format!("{ERR}_CANCELLED"));
        }
        send(
            "asking",
            format!("attempt {attempt}/{attempts}: asking {}", llm.label()),
            Vec::new(),
            attempt,
            tokens,
        );
        let mut messages = vec![system.clone(), first_msg.clone()];
        if let Some((answer, feedback)) = &last {
            messages.push(llm::assistant(answer.clone()));
            messages.push(feedback.clone());
        }
        let answer = llm::ask(&llm, messages, MAX_ANSWER_TOKENS, cancel.clone()).await?;
        tokens += answer.tokens();

        let parsed = llm::extract_json(&answer.text)
            .map_err(|e| vec![store::violation("P-JSON", "answer", "", &e)])
            .and_then(|v| {
                let name = v["charter"]["name"].as_str().unwrap_or("").to_string();
                let id = req
                    .city_id
                    .clone()
                    .filter(|s| store::safe_id(s))
                    .unwrap_or_else(|| slug(&name));
                parse_answer(&v, &id, &seed)
            });
        let (mut charter, program) = match parsed {
            Ok(x) => x,
            Err(v) => {
                send(
                    "retry",
                    "the answer did not parse".into(),
                    v.clone(),
                    attempt,
                    tokens,
                );
                last_violations = v.clone();
                last = Some((
                    answer.text.clone(),
                    llm::user(format!(
                        "Your answer was not usable: {}. Answer again with ONE JSON object {{\"charter\":..., \"program\":...}} exactly in the format.",
                        serde_json::to_string(&v).unwrap_or_default()
                    )),
                ));
                continue;
            }
        };

        send(
            "checking",
            format!("checking {}", charter.name),
            Vec::new(),
            attempt,
            tokens,
        );
        let mut programs = vec![program];
        api::normalise_founding(&mut charter, &mut programs);
        let vision = llm.vision;
        let (violations, png, map, land_now, options_now) = {
            let charter = charter.clone();
            let programs = programs.clone();
            tokio::task::spawn_blocking(move || {
                let violations = api::check_founding(&charter, &programs);
                let built = store::build_uncached(&charter, &programs[0]);
                let png = if vision {
                    built
                        .blueprint
                        .as_ref()
                        .zip(built.mesh())
                        .map(|(bp, (inst, _))| {
                            omniget_citygen::preview::render_png(
                                bp,
                                inst,
                                omniget_citygen::preview::View::Top,
                                512,
                            )
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let map = built
                    .blueprint
                    .as_ref()
                    .map(omniget_citygen::solver::ascii)
                    .unwrap_or_default();
                (
                    violations,
                    png,
                    map,
                    land(&charter),
                    docs::options(&charter, &programs[0]),
                )
            })
            .await
            .map_err(|e| format!("{ERR}_TASK: {e}"))?
        };

        if !store::has_errors(&violations) {
            let author = Author {
                kind: "llm".into(),
                name: llm.label(),
                role: Some("fundador".into()),
            };
            let result = api::found(charter.clone(), programs, author, Some(tokens)).await?;
            if result.founded {
                super::roles::charge(&result.city_id, "fundador", tokens);
                emit(FoundingEvent {
                    job: req.job.clone(),
                    stage: "founded".into(),
                    message: format!("{} was founded", charter.name),
                    violations: result.violations.clone(),
                    done: true,
                    error: None,
                    attempt,
                    tokens,
                    city_id: Some(result.city_id.clone()),
                });
                return Ok(result);
            }
            last_violations = result.violations;
        } else {
            last_violations = violations;
        }

        let errors: Vec<&Violation> = last_violations
            .iter()
            .filter(|v| v.severity == omniget_citygen::Severity::Error)
            .collect();
        send(
            "retry",
            format!("{} errors, asking for a fix", errors.len()),
            last_violations.clone(),
            attempt,
            tokens,
        );
        let feedback = format!(
            "The generator rejected this founding with {} errors (warnings do not block). Each is [code, id, measure] + message:\n{}\nThe land with YOUR charter: {}\nValid options now: {}\n{}{}Fix every error and answer the whole JSON again (charter + program).",
            errors.len(),
            serde_json::to_string(&last_violations).unwrap_or_default(),
            land_now,
            options_now,
            if png.is_empty() { "" } else { "The attached image is a top view of what your program produced.\n" },
            if map.is_empty() {
                String::new()
            } else {
                format!("Text map of what it produced (. ground, = street, - sidewalk, # plaza, ~ water, H bridge, , lot, : yard, digit = floors, D door):\n{map}\n")
            },
        );
        last = Some((answer.text.clone(), llm::user_with_png(feedback, &png)));
    }

    Err(format!(
        "{ERR}_FOUNDING_FAILED: {} errors left after {attempts} attempts",
        last_violations
            .iter()
            .filter(|v| v.severity == omniget_citygen::Severity::Error)
            .count()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real founding against a local Ollama (`ollama serve`):
    /// `CITY3D_LIVE_MODEL=qwen3:8b cargo test -p omniget-core live_founding -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_founding_with_ollama() {
        use crate::core::llm::providers::openai_compat::OpenAiCompat;
        use crate::core::llm::types::{ModelRef, ProviderId};
        let dir = std::env::temp_dir().join(format!("city3d-live-{}", uuid::Uuid::new_v4()));
        store::set_root(Some(dir.clone()));
        let model = std::env::var("CITY3D_LIVE_MODEL").unwrap_or_else(|_| "qwen3:8b".into());
        let base = std::env::var("CITY3D_LIVE_BASE")
            .unwrap_or_else(|_| "http://127.0.0.1:11434/v1".into());
        let provider = OpenAiCompat::new(ProviderId::new("ollama"), base, String::new()).unwrap();
        let llm = Llm::new(
            Arc::new(provider),
            ModelRef {
                provider: ProviderId::new("ollama"),
                model,
            },
        );
        let emit: Emit = Arc::new(|ev: FoundingEvent| {
            eprintln!(
                "[{}] #{} {} ({} violations, {} tokens)",
                ev.stage,
                ev.attempt,
                ev.message,
                ev.violations.len(),
                ev.tokens
            );
            for v in ev.violations.iter().take(8) {
                eprintln!("    {} {} {} {}", v.code, v.id, v.measure, v.message);
            }
        });
        let req = FoundingRequest {
            job: "live".into(),
            brief: "A small riverside village with a church square and a market street.".into(),
            max_attempts: std::env::var("CITY3D_LIVE_ATTEMPTS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(4),
            ..Default::default()
        };
        let out = run(llm, req, emit, CancellationToken::new()).await;
        eprintln!("result: {out:?}");
        eprintln!("store: {}", dir.display());
    }

    #[test]
    fn slugs_are_store_safe() {
        assert_eq!(slug("São Lourenço da Mata"), "sao-lourenco-da-mata");
        assert!(store::safe_id(&slug("!!!")));
    }
}
