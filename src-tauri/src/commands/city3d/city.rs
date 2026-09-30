//! The 3D city's store, founding and editing commands (agent G2). The work
//! lives in `omniget_core::core::city3d`; this file picks the model (the
//! vault and the roster are the app's), runs the founding job, emits the
//! events and connects to a server city with the OmniDisc session token.
//!
//! Events: `city3d:founding` `{job, stage, message, violations, done, error?}`
//! and `city3d:changed` `{region, revision}`. Every change goes through the
//! core's change hook, so an MCP agent's `city_apply` reaches the window too.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine;
use omniget_citygen::program::Proposal;
use omniget_core::core::city3d::{self as c3, api, founding, llm::Llm, roles, server, store};
use omniget_core::core::llm::agent::RuntimeKind;
use omniget_core::core::llm::types::{ModelRef, ProviderId};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use crate::llm_manager::{provider_available, LlmManager};
use crate::AppState;

const ERR: &str = c3::ERR;
const LOCAL: &[&str] = &["ollama", "lmstudio", "llama-server"];

static HOOKED: OnceLock<()> = OnceLock::new();
static JOBS: Mutex<Option<HashMap<String, CancellationToken>>> = Mutex::new(None);
static SCHEDULE: Mutex<Option<CancellationToken>> = Mutex::new(None);

/// Wires the core's change hook to `city3d:changed` and resumes the council
/// schedule saved for the active city. Idempotent; every command calls it,
/// and the app's setup may call it so MCP changes reach the window before
/// the city route was ever opened.
pub fn install(app: &AppHandle) {
    if HOOKED.set(()).is_err() {
        return;
    }
    let emitter = app.clone();
    api::set_change_hook(Arc::new(move |ev| {
        let _ = emitter.emit(c3::EVENT_CHANGED, &ev);
    }));
    if let Some(id) = store::active_id() {
        if let Some(min) = store::load_settings(&id).council_every_min {
            start_schedule(app.clone(), min);
        }
    }
}

// ── Model choice ───────────────────────────────────────────────────────

/// Native roster models that can answer now; hosted first for `big`
/// (founding), local first otherwise (council rounds are frequent).
fn candidates(llm: &LlmManager, big: bool) -> Vec<ModelRef> {
    let mut out: Vec<ModelRef> = Vec::new();
    for agent in llm.roster() {
        if !matches!(agent.runtime, RuntimeKind::Native) {
            continue;
        }
        let m = llm.model_of(&agent);
        let p = m.provider.as_str();
        if p == "fake" || m.model.is_empty() || !provider_available(p) {
            continue;
        }
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out.sort_by_key(|m| LOCAL.contains(&m.provider.as_str()) == big);
    out
}

/// The saved model of an account when the caller named only the provider.
fn default_model_of(llm: &LlmManager, provider: &str) -> Option<String> {
    if let Some(m) = candidates(llm, true)
        .into_iter()
        .find(|m| m.provider.as_str() == provider)
    {
        return Some(m.model);
    }
    omniget_core::core::tools::ai_keys::list()
        .into_iter()
        .find(|k| (k.id == provider || k.kind == provider) && k.has_key && !k.model.is_empty())
        .map(|k| k.model)
}

fn pick_llm(
    llm: &LlmManager,
    provider: Option<String>,
    model: Option<String>,
    big: bool,
) -> Result<Llm, String> {
    let provider = provider
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    let model = model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    let chosen = match provider {
        Some(p) => {
            if !provider_available(&p) {
                return Err(format!(
                    "{ERR}_NO_KEY: no saved key for `{p}`: add it in AI connections (/llm/accounts) or pick another provider"
                ));
            }
            let model = model.or_else(|| default_model_of(llm, &p)).ok_or_else(|| {
                format!("{ERR}_NO_MODEL: pick a model for `{p}`")
            })?;
            ModelRef {
                provider: ProviderId::new(p),
                model,
            }
        }
        None => candidates(llm, big).into_iter().next().ok_or_else(|| {
            format!("{ERR}_NO_PROVIDER: no AI connection can answer: add a key or start Ollama in AI connections")
        })?,
    };
    let provider = llm.provider_for(&chosen.provider).ok_or_else(|| {
        format!(
            "{ERR}_NO_KEY: `{}` has no usable key",
            chosen.provider.as_str()
        )
    })?;
    Ok(Llm::new(provider, chosen))
}

/// Models the founding panel can offer: `{provider, model, local}`.
#[tauri::command]
pub async fn city3d_providers(state: State<'_, AppState>) -> Result<Value, String> {
    let list: Vec<Value> = candidates(&state.llm, true)
        .into_iter()
        .map(|m| {
            json!({
                "provider": m.provider.as_str(),
                "model": m.model,
                "local": LOCAL.contains(&m.provider.as_str()),
            })
        })
        .collect();
    Ok(json!(list))
}

// ── Status and founding ────────────────────────────────────────────────

#[tauri::command]
pub async fn city3d_status(app: AppHandle) -> Result<Value, String> {
    install(&app);
    api::status().await
}

/// Found without an LLM: the example charter and the starter program.
#[tauri::command]
pub async fn city3d_found_example(
    app: AppHandle,
    city_id: Option<String>,
    seed: Option<String>,
    name: Option<String>,
) -> Result<Value, String> {
    install(&app);
    let city_id = city_id
        .filter(|s| store::safe_id(s))
        .unwrap_or_else(|| "omni".into());
    let seed = seed
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("omni-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let (mut charter, program) = tokio::task::spawn_blocking(move || {
        let (c, p, _) = api::example_founding(&city_id, &seed);
        (c, p)
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
        charter.name = n.trim().to_string();
    }
    let r = api::found(charter, vec![program], store::Author::person(), None).await?;
    serde_json::to_value(r).map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn city3d_found_start(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: Option<String>,
    model: Option<String>,
    brief: String,
    city_id: Option<String>,
    seed: Option<String>,
    attempts: Option<u32>,
) -> Result<Value, String> {
    install(&app);
    let llm = pick_llm(&state.llm, provider, model, true)?;
    let job = format!("found-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let cancel = CancellationToken::new();
    JOBS.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(job.clone(), cancel.clone());
    let req = founding::FoundingRequest {
        job: job.clone(),
        brief,
        city_id,
        seed,
        max_attempts: attempts.unwrap_or(founding::DEFAULT_ATTEMPTS),
    };
    let emitter = app.clone();
    let emit: founding::Emit = Arc::new(move |ev| {
        let _ = emitter.emit(c3::EVENT_FOUNDING, &ev);
    });
    let model_label = llm.label();
    let job_id = job.clone();
    tauri::async_runtime::spawn(async move {
        let out = founding::run(llm, req, emit.clone(), cancel).await;
        if let Err(e) = out {
            emit(founding::FoundingEvent {
                job: job_id.clone(),
                stage: "failed".into(),
                message: e.clone(),
                done: true,
                error: Some(e),
                ..Default::default()
            });
        }
        if let Some(m) = JOBS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            m.remove(&job_id);
        }
    });
    Ok(json!({ "job": job, "model": model_label }))
}

#[tauri::command]
pub async fn city3d_found_cancel(job: String) -> Result<bool, String> {
    let token = JOBS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
        .and_then(|m| m.remove(&job));
    Ok(token.map(|t| t.cancel()).is_some())
}

// ── Regions and proposals ──────────────────────────────────────────────

#[tauri::command]
pub async fn city3d_region(app: AppHandle, region: [i32; 2]) -> Result<Value, String> {
    install(&app);
    api::region(region).await
}

#[tauri::command]
pub async fn city3d_region_mesh(app: AppHandle, region: [i32; 2]) -> Result<Value, String> {
    install(&app);
    api::region_mesh(region).await
}

#[tauri::command]
pub async fn city3d_propose(
    app: AppHandle,
    proposal: Proposal,
    reason: Option<String>,
) -> Result<Value, String> {
    install(&app);
    let out = api::propose(
        proposal,
        store::Author::person(),
        reason.unwrap_or_default(),
    )
    .await?;
    serde_json::to_value(out).map_err(|e| e.to_string())
}

/// Publishes a person's proposal; `city3d:changed` goes out through the hook.
#[tauri::command]
pub async fn city3d_apply(
    app: AppHandle,
    proposal: Proposal,
    reason: Option<String>,
) -> Result<Value, String> {
    install(&app);
    let out = api::apply(
        proposal,
        store::Author::person(),
        reason.unwrap_or_default(),
        None,
        None,
    )
    .await?;
    serde_json::to_value(out).map_err(|e| e.to_string())
}

/// Approve or refuse a `pending_review` proposal (or a draft).
#[tauri::command]
pub async fn city3d_review(
    app: AppHandle,
    id: String,
    approve: bool,
    reason: Option<String>,
) -> Result<Value, String> {
    install(&app);
    if approve {
        let out = api::approve(id).await?;
        serde_json::to_value(out).map_err(|e| e.to_string())
    } else {
        api::refuse(id, reason.unwrap_or_default()).await
    }
}

#[tauri::command]
pub async fn city3d_preview(
    app: AppHandle,
    region: [i32; 2],
    view: Option<String>,
    px: Option<u32>,
) -> Result<Value, String> {
    install(&app);
    let png = api::preview_png(region, view.as_deref().unwrap_or("top"), px.unwrap_or(512)).await?;
    Ok(json!({
        "png_base64": base64::engine::general_purpose::STANDARD.encode(png),
    }))
}

#[tauri::command]
pub async fn city3d_log(app: AppHandle, limit: Option<usize>) -> Result<Value, String> {
    install(&app);
    api::log(limit.unwrap_or(100)).await
}

/// `auto` or `review` for LLM works of the active local city.
#[tauri::command]
pub async fn city3d_policy_set(app: AppHandle, policy: String) -> Result<Value, String> {
    install(&app);
    if policy != "auto" && policy != "review" {
        return Err(format!("{ERR}_BAD_POLICY: auto or review"));
    }
    tokio::task::spawn_blocking(move || {
        let (_, mut charter) = api::require_local()?;
        let _g = store::lock();
        charter.publish_policy = policy.clone();
        store::save_charter(&charter)?;
        Ok(json!({ "publish_policy": policy }))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Roles, budgets, construction ───────────────────────────────────────

#[tauri::command]
pub async fn city3d_council_tick(
    app: AppHandle,
    state: State<'_, AppState>,
    region: Option<[i32; 2]>,
    roles: Option<Vec<String>>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<Value, String> {
    install(&app);
    let llm = pick_llm(&state.llm, provider, model, false)?;
    roles::council_tick(
        llm,
        region.unwrap_or([0, 0]),
        roles.unwrap_or_default(),
        CancellationToken::new(),
    )
    .await
}

fn start_schedule(app: AppHandle, minutes: u32) {
    let cancel = CancellationToken::new();
    if let Some(old) = SCHEDULE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(cancel.clone())
    {
        old.cancel();
    }
    let every = std::time::Duration::from_secs(minutes.max(1) as u64 * 60);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(every) => {}
            }
            if server::current().is_some() || store::active_id().is_none() {
                continue;
            }
            let llm = {
                use tauri::Manager;
                let state = app.state::<AppState>();
                pick_llm(&state.llm, None, None, false)
            };
            let Ok(llm) = llm else { continue };
            let regions = store::active_id()
                .map(|id| store::regions(&id))
                .unwrap_or_default();
            let n = regions.len().max(1);
            let pick = (store::now_ms() / every.as_millis().max(1) as u64) as usize % n;
            let region = regions.get(pick).copied().unwrap_or([0, 0]);
            let report = roles::council_tick(llm, region, Vec::new(), cancel.clone()).await;
            let _ = app.emit(
                "city3d:council",
                &report.unwrap_or_else(|e| json!({ "error": e })),
            );
        }
    });
}

/// Council rounds every `minutes` while the app runs (off with `null`).
#[tauri::command]
pub async fn city3d_council_schedule(
    app: AppHandle,
    minutes: Option<u32>,
) -> Result<Value, String> {
    install(&app);
    let (id, _) = api::require_local()?;
    let mut s = store::load_settings(&id);
    s.council_every_min = minutes.filter(|m| *m > 0);
    store::save_settings(&id, &s)?;
    match s.council_every_min {
        Some(m) => start_schedule(app, m),
        None => {
            if let Some(old) = SCHEDULE.lock().unwrap_or_else(|e| e.into_inner()).take() {
                old.cancel();
            }
        }
    }
    Ok(json!({ "council_every_min": s.council_every_min }))
}

#[tauri::command]
pub async fn city3d_budget_set(
    app: AppHandle,
    role: String,
    limit_tokens: u64,
) -> Result<Value, String> {
    install(&app);
    if !roles::ROLES.contains(&role.as_str()) {
        return Err(format!("{ERR}_BAD_ROLE: {role}"));
    }
    let (id, _) = api::require_local()?;
    roles::set_limit(&id, &role, limit_tokens)?;
    Ok(json!(store::load_budgets(&id)))
}

/// The UI calls this every few seconds while the city is open.
#[tauri::command]
pub async fn city3d_works_tick(app: AppHandle) -> Result<Value, String> {
    install(&app);
    api::works_tick().await
}

// ── Server mode ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn city3d_server_connect(
    app: AppHandle,
    server: Option<String>,
    city: String,
) -> Result<Value, String> {
    install(&app);
    if !store::safe_id(&city) {
        return Err(format!("{ERR}_BAD_ID: city `{city}`"));
    }
    let base = crate::commands::world::city::resolve_base(server, &app)?;
    let token = crate::commands::omnidisc::store::load_token(&base)?
        .ok_or_else(|| format!("{ERR}_NO_SESSION: sign in to OmniDisc at {base} first"))?;
    let conn = server::ServerConn { base, city, token };
    let cities = server::cities_of(conn.get("cities").await?);
    server::set(Some(conn.clone()));
    Ok(json!({ "connected": true, "base": conn.base, "city": conn.city, "cities": cities }))
}

#[tauri::command]
pub async fn city3d_server_disconnect() -> Result<Value, String> {
    server::set(None);
    Ok(json!({ "connected": false }))
}

#[tauri::command]
pub async fn city3d_server_status() -> Result<Value, String> {
    Ok(match server::current() {
        Some(c) => json!({ "connected": true, "base": c.base, "city": c.city }),
        None => json!({ "connected": false }),
    })
}

// ── MCP host half ──────────────────────────────────────────────────────

/// Body of the `city_*` tools that need the app (`city_codegeo`).
pub async fn host_tool(_app: &AppHandle, name: &str, a: Value) -> Result<Value, String> {
    match name {
        "city_codegeo" => codegeo(a).await,
        other => Err(format!("unknown city tool: {other}")),
    }
}

async fn codegeo(a: Value) -> Result<Value, String> {
    let script = a["script"].as_str().unwrap_or("").to_string();
    if script.trim().is_empty() {
        return Err("argument \"script\" is required (string)".into());
    }
    let class = a["class"].as_str().unwrap_or("building").to_string();
    tokio::task::spawn_blocking(move || {
        Ok(match omniget_codegeo::run_script(&script, &class) {
            Ok(out) => json!({
                "glb_base64": base64::engine::general_purpose::STANDARD.encode(&out.glb),
                "typology": out.typology,
                "anchors": out.anchors,
                "triangles": out.triangles,
                "bounds_mm": out.bounds_mm,
                "warnings": out.warnings,
                "errors": [],
            }),
            Err(errors) => json!({
                "glb_base64": "",
                "anchors": [],
                "triangles": 0,
                "errors": errors.iter().map(|e| json!({ "kind": e.kind, "line": e.line, "message": e.to_string() })).collect::<Vec<_>>(),
            }),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
