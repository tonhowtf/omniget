//! The city's operations, local or on a server. The Tauri commands, the
//! `city_*` MCP tools and the council round all call these, so a person, an
//! agent over MCP and a role of the city go through the same pipeline:
//! `program::apply` on a copy → solver → checker → publish policy.

use std::sync::{Arc, RwLock};

use omniget_citygen::preview::View;
use omniget_citygen::program::{self, CityProgram, PatchOp, Proposal, ProposalResult};
use omniget_citygen::{charter as charter_mod, preview, Charter, Dir, Violation};
use serde::Serialize;
use serde_json::{json, Value};

use super::server::{self, ServerConn};
use super::store::{self, Author, LogEntry, RegionBuild};
use super::ERR;

// ── Change notifications ───────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
pub struct ChangeEvent {
    pub region: [i32; 2],
    pub revision: String,
}

type Hook = Arc<dyn Fn(ChangeEvent) + Send + Sync>;
static HOOK: RwLock<Option<Hook>> = RwLock::new(None);

/// The app installs this once to turn changes into `city3d:changed`; a change
/// made by an MCP tool reaches the window the same way.
pub fn set_change_hook(hook: Hook) {
    if let Ok(mut g) = HOOK.write() {
        *g = Some(hook);
    }
}

pub fn has_change_hook() -> bool {
    HOOK.read().map(|g| g.is_some()).unwrap_or(false)
}

fn notify(region: [i32; 2], revision: &str) {
    let hook = HOOK.read().ok().and_then(|g| g.clone());
    if let Some(hook) = hook {
        hook(ChangeEvent {
            region,
            revision: revision.to_string(),
        });
    }
}

pub fn mode() -> &'static str {
    if server::current().is_some() {
        "server"
    } else {
        "local"
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("{ERR}_TASK: {e}"))?
}

/// The active local city and its charter.
pub fn require_local() -> Result<(String, Charter), String> {
    let id = store::active_id()
        .ok_or_else(|| format!("{ERR}_NOT_FOUNDED: this city was not founded yet"))?;
    let charter = store::load_charter(&id)?;
    Ok((id, charter))
}

pub fn parse_view(view: &str) -> View {
    match view.trim().to_ascii_lowercase().as_str() {
        "n" | "north" | "oblique_n" => View::Oblique(Dir::N),
        "e" | "east" | "oblique_e" => View::Oblique(Dir::E),
        "s" | "south" | "oblique_s" | "oblique" => View::Oblique(Dir::S),
        "w" | "west" | "oblique_w" => View::Oblique(Dir::W),
        _ => View::Top,
    }
}

// ── Status and founding ────────────────────────────────────────────────

pub async fn status() -> Result<Value, String> {
    if let Some(conn) = server::current() {
        let cities = server::cities_of(conn.get("cities").await?);
        let mut city = cities
            .into_iter()
            .find(|c| c["city_id"] == conn.city.as_str())
            .unwrap_or(Value::Null);
        if let Some(o) = city.as_object_mut() {
            o.insert("server".into(), json!(conn.base));
        }
        if !city.is_null() {
            if let Ok(c) = conn.get(&conn.city_path("charter")).await {
                city["charter"] = c["charter"].clone();
            }
            if let Ok(l) = log(500).await {
                city["pending_review"] = json!(l["proposals"]
                    .as_array()
                    .map(|p| p.iter().filter(|x| x["state"] == "pending_review").count())
                    .unwrap_or(0));
                city["works"] = json!(l["works"].as_array().map(Vec::len).unwrap_or(0));
            }
        }
        return Ok(json!({
            "mode": "server",
            "server": { "base": conn.base, "city": conn.city },
            "city": city,
        }));
    }
    blocking(|| {
        let Some(id) = store::active_id() else {
            return Ok(json!({ "mode": "local", "city": null }));
        };
        let charter = store::load_charter(&id)?;
        let log = store::read_log(&id);
        let pending = log.iter().filter(|e| e.state == "pending_review").count();
        let regions = store::regions(&id);
        let mut works = 0usize;
        for r in &regions {
            if let Ok(p) = store::load_program(&id, *r) {
                works += p.buildings.iter().filter(|b| b.progress < 100).count();
            }
        }
        let settings = store::load_settings(&id);
        Ok(json!({
            "mode": "local",
            "city": {
                "city_id": id,
                "name": charter.name,
                "publish_policy": charter.publish_policy,
                "charter": charter,
                "regions": regions,
                "pending_review": pending,
                "works": works,
                "settings": settings,
                "budgets": store::load_budgets(&id),
            }
        }))
    })
    .await
}

/// Founding answer: `founded` only with zero errors in the charter and in
/// every program.
#[derive(Serialize, Clone, Debug)]
pub struct FoundResult {
    pub founded: bool,
    pub city_id: String,
    pub violations: Vec<Violation>,
    pub revisions: Vec<(Vec<i32>, String)>,
}

/// Validate a charter and its programs without writing anything.
pub fn check_founding(charter: &Charter, programs: &[CityProgram]) -> Vec<Violation> {
    let mut v = charter_mod::validate(charter);
    if programs.is_empty() {
        v.push(store::violation(
            "P-PROGRAM",
            &charter.city_id,
            "0 programs",
            "a founding needs at least the program of region [0,0]",
        ));
    }
    for p in programs {
        let mut found = program::validate(p);
        found.extend(store::build_uncached(charter, p).violations);
        for mut x in found {
            if !x.id.contains('@') {
                x.id = format!("{}@{},{}", x.id, p.region[0], p.region[1]);
            }
            v.push(x);
        }
    }
    v
}

/// Normalise what an LLM or a person wrote into the ids the store uses.
pub fn normalise_founding(charter: &mut Charter, programs: &mut [CityProgram]) {
    if charter.schema.is_empty() {
        charter.schema = "omni.city.charter/v1".into();
    }
    if charter.generator_version.is_empty() {
        charter.generator_version = omniget_citygen::GENERATOR_VERSION.into();
    }
    if charter.publish_policy != "review" {
        charter.publish_policy = "auto".into();
    }
    for p in programs.iter_mut() {
        p.city_id = charter.city_id.clone();
        if p.schema.is_empty() {
            p.schema = "omni.city.program/v1".into();
        }
        if p.catalog.is_empty() {
            p.catalog = "kit-vila-v1".into();
        }
    }
}

pub async fn found(
    mut charter: Charter,
    mut programs: Vec<CityProgram>,
    author: Author,
    tokens: Option<u64>,
) -> Result<FoundResult, String> {
    if !store::safe_id(&charter.city_id) {
        return Err(format!(
            "{ERR}_BAD_ID: city_id must be 1-64 letters, digits, - or _"
        ));
    }
    normalise_founding(&mut charter, &mut programs);
    if let Some(conn) = server::current() {
        let body = json!({ "charter": charter, "programs": programs });
        // 201 founded; 422 carries the building code's answer.
        let (status, v) = conn.post_status("cities", &body).await?;
        if status != 201 && status != 200 && status != 422 {
            return Err(server::error_of(status, &v));
        }
        let violations: Vec<Violation> =
            serde_json::from_value(v["violations"].clone()).unwrap_or_default();
        let founded = v["founded"].as_bool().unwrap_or(status != 422);
        let revisions: Vec<(Vec<i32>, String)> = v["regions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|r| {
                let region: Vec<i32> = serde_json::from_value(r["region"].clone()).ok()?;
                Some((region, r["revision"].as_str()?.to_string()))
            })
            .collect();
        if founded {
            // The connection follows the city it just founded.
            server::set(Some(ServerConn {
                city: charter.city_id.clone(),
                ..conn
            }));
            for (r, rev) in &revisions {
                if r.len() == 2 {
                    notify([r[0], r[1]], rev);
                }
            }
        }
        return Ok(FoundResult {
            founded,
            city_id: charter.city_id,
            violations,
            revisions,
        });
    }
    blocking(move || {
        let violations = check_founding(&charter, &programs);
        if store::has_errors(&violations) {
            return Ok(FoundResult {
                founded: false,
                city_id: charter.city_id,
                violations,
                revisions: Vec::new(),
            });
        }
        let _g = store::lock();
        let dir = store::city_dir(&charter.city_id)?;
        // Re-founding the same id starts the regions over; the log stays.
        let _ = std::fs::remove_dir_all(dir.join("programs"));
        let _ = std::fs::remove_dir_all(dir.join("revisions"));
        store::save_charter(&charter)?;
        let log_id = store::new_log_id();
        let mut revisions = Vec::new();
        for p in &programs {
            let rev = store::save_program(&charter.city_id, p, None, Some(&log_id), &author.name)?;
            revisions.push((p.region.to_vec(), rev));
        }
        store::append_log(
            &charter.city_id,
            &LogEntry {
                id: log_id,
                at_ms: store::now_ms(),
                region: programs.first().map(|p| p.region).unwrap_or([0, 0]),
                author: author.clone(),
                reason: format!("founded {}", charter.name),
                ops: Vec::new(),
                base_revision: String::new(),
                revision: revisions.first().map(|r| r.1.clone()),
                violations: violations.clone(),
                state: "founded".into(),
                summary: json!({ "regions": programs.len() }),
                admin: true,
                idempotency_key: None,
                tokens,
                resolution: None,
            },
        )?;
        store::set_active(&charter.city_id)?;
        let mut s = store::load_settings(&charter.city_id);
        s.works_last_ms = store::now_ms();
        store::save_settings(&charter.city_id, &s)?;
        drop(_g);
        for (r, rev) in &revisions {
            notify([r[0], r[1]], rev);
        }
        Ok(FoundResult {
            founded: true,
            city_id: charter.city_id,
            violations,
            revisions,
        })
    })
    .await
}

/// The example charter and the solver's default program, on the first seed
/// (`seed`, `seed-1`, … up to 24 tries) whose land lets the program pass the
/// building code: founding without an LLM must not fail on a hilly seed.
pub fn example_founding(city_id: &str, seed: &str) -> (Charter, CityProgram, Vec<Violation>) {
    let mut first = None;
    for n in 0..24 {
        let s = if n == 0 {
            seed.to_string()
        } else {
            format!("{seed}-{n}")
        };
        let charter = charter_mod::example(city_id, &s);
        let program = default_program(&charter);
        let v = check_founding(&charter, std::slice::from_ref(&program));
        if !store::has_errors(&v) {
            return (charter, program, v);
        }
        if first.is_none() {
            first = Some((charter, program, v));
        }
    }
    first.expect("at least one try")
}

/// A starting program for region [0,0] when no LLM is involved.
pub fn default_program(charter: &Charter) -> CityProgram {
    super::docs::starter_program(charter)
}

// ── Regions ────────────────────────────────────────────────────────────

pub async fn region(region: [i32; 2]) -> Result<Value, String> {
    if let Some(conn) = server::current() {
        let p = conn
            .get(&conn.city_path(&format!("program/{}/{}", region[0], region[1])))
            .await?;
        let program = server::unwrap_field(p.clone(), "program");
        let revision = p["revision"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| {
                serde_json::from_value::<CityProgram>(program.clone())
                    .map(|p| p.revision())
                    .unwrap_or_default()
            });
        let b = conn
            .get(&conn.city_path(&format!("blueprint/{}/{}", region[0], region[1])))
            .await?;
        return Ok(json!({
            "program": program,
            "revision": revision,
            "blueprint": server::unwrap_field(b.clone(), "blueprint"),
            "violations": b.get("violations").cloned().unwrap_or(json!([])),
        }));
    }
    blocking(move || {
        let (id, charter) = require_local()?;
        let program = store::load_program(&id, region)?;
        let built = store::build(&charter, &program);
        Ok(json!({
            "program": program,
            "revision": built.revision,
            "blueprint": built.blueprint,
            "violations": built.violations,
        }))
    })
    .await
}

/// The local build of a region (server mode: from the server's blueprint).
pub async fn region_build(region: [i32; 2]) -> Result<Arc<RegionBuild>, String> {
    if server::current().is_some() {
        let v = self::region(region).await?;
        let blueprint = serde_json::from_value(v["blueprint"].clone()).ok();
        let violations = serde_json::from_value(v["violations"].clone()).unwrap_or_default();
        let revision = v["revision"].as_str().unwrap_or_default().to_string();
        return Ok(Arc::new(RegionBuild::from_parts(
            revision, blueprint, violations,
        )));
    }
    blocking(move || {
        let (id, charter) = require_local()?;
        let program = store::load_program(&id, region)?;
        Ok(store::build(&charter, &program))
    })
    .await
}

pub async fn region_mesh(region: [i32; 2]) -> Result<Value, String> {
    let built = region_build(region).await?;
    blocking(move || match built.mesh() {
        Some((instances, terrain)) => Ok(json!({
            "revision": built.revision,
            "instances": instances,
            "terrain": terrain,
        })),
        None => Err(format!(
            "{ERR}_NO_BLUEPRINT: the region does not solve ({} violations)",
            built.violations.len()
        )),
    })
    .await
}

pub async fn preview_png(region: [i32; 2], view: &str, px: u32) -> Result<Vec<u8>, String> {
    let px = px.clamp(64, 1024);
    if let Some(conn) = server::current() {
        return conn
            .get_png(&conn.city_path(&format!(
                "preview/{}/{}?view={}&px={}",
                region[0],
                region[1],
                server::view_param(view),
                px
            )))
            .await;
    }
    let built = region_build(region).await?;
    let view = parse_view(view);
    blocking(move || render(&built, view, px)).await
}

fn render(built: &RegionBuild, view: View, px: u32) -> Result<Vec<u8>, String> {
    let bp = built
        .blueprint
        .as_ref()
        .ok_or_else(|| format!("{ERR}_NO_BLUEPRINT: nothing to render"))?;
    let (instances, _) = built
        .mesh()
        .ok_or_else(|| format!("{ERR}_NO_BLUEPRINT: nothing to render"))?;
    let png = preview::render_png(bp, instances, view, px);
    if png.is_empty() {
        return Err(format!(
            "{ERR}_RENDER: the preview renderer returned nothing"
        ));
    }
    Ok(png)
}

// ── Proposals ──────────────────────────────────────────────────────────

/// New buildings start as construction sites.
fn prepare_ops(ops: &[PatchOp]) -> Vec<PatchOp> {
    ops.iter()
        .cloned()
        .map(|op| match op {
            PatchOp::AddBuilding(mut spec) => {
                spec.progress = 0;
                PatchOp::AddBuilding(spec)
            }
            other => other,
        })
        .collect()
}

struct Evaluation {
    current: CityProgram,
    next: Option<CityProgram>,
    build: Option<RegionBuild>,
    result: ProposalResult,
    admin: bool,
}

fn evaluate(
    city_id: &str,
    charter: &Charter,
    proposal: &Proposal,
    author: &Author,
) -> Result<Evaluation, String> {
    let current = store::load_program(city_id, proposal.region)?;
    let current_rev = current.revision();
    let admin = proposal.admin && !author.is_machine();
    let fail = |current: CityProgram, v: Vec<Violation>| Evaluation {
        result: ProposalResult {
            revision: current.revision(),
            violations: v,
            summary: Default::default(),
        },
        current,
        next: None,
        build: None,
        admin,
    };
    if proposal.base_revision.is_empty() && author.is_machine() {
        return Ok(fail(
            current,
            vec![store::violation(
                "P-REVISION",
                "base_revision",
                &current_rev,
                "base_revision is required: read the region (city_inspect) and pass its revision",
            )],
        ));
    }
    if !proposal.base_revision.is_empty() && proposal.base_revision != current_rev {
        return Ok(fail(
            current,
            vec![store::violation(
                "P-REVISION",
                "base_revision",
                &format!("{} != {}", proposal.base_revision, current_rev),
                "the region changed since you read it: read it again and redo the ops on the current revision",
            )],
        ));
    }
    if proposal.ops.is_empty() {
        return Ok(fail(
            current,
            vec![store::violation("P-EMPTY", "ops", "0", "no operations")],
        ));
    }
    let ops = prepare_ops(&proposal.ops);
    let next = match program::apply(&current, &ops, admin) {
        Ok(next) => next,
        Err(v) => return Ok(fail(current, v)),
    };
    let before = store::build(charter, &current);
    let after = store::build_uncached(charter, &next);
    let summary = store::summary(Some(&before), &after);
    Ok(Evaluation {
        result: ProposalResult {
            revision: after.revision.clone(),
            violations: after.violations.clone(),
            summary,
        },
        current,
        next: Some(next),
        build: Some(after),
        admin,
    })
}

/// Dry run: nothing is logged or written.
pub async fn validate(proposal: Proposal, author: Author) -> Result<ProposalResult, String> {
    if let Some(conn) = server::current() {
        // The server has no dry run: `propose` evaluates and keeps a draft.
        return server_propose(&conn, &proposal, &author, "").await;
    }
    blocking(move || {
        let (id, charter) = require_local()?;
        Ok(evaluate(&id, &charter, &proposal, &author)?.result)
    })
    .await
}

/// `POST {city}/propose`: the result, with an old base reported as the
/// local store does (a `P-REVISION` error).
async fn server_propose(
    conn: &ServerConn,
    proposal: &Proposal,
    author: &Author,
    reason: &str,
) -> Result<ProposalResult, String> {
    let body = server::proposal_body(proposal, author, reason, None);
    let v = conn.post(&conn.city_path("propose"), &body).await?;
    let mut result: ProposalResult =
        serde_json::from_value(v).map_err(|e| format!("{ERR}_SERVER_BODY: {e}"))?;
    if result.summary.get("stale").and_then(Value::as_bool) == Some(true) {
        let current = result
            .summary
            .get("current_revision")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        result.violations.insert(
            0,
            server::stale_violation(&proposal.base_revision, &current),
        );
    }
    Ok(result)
}

/// The server's `{proposal_id, state, result}` (apply and review) as an
/// [`Outcome`]; a 409 `stale_revision` becomes the `stale` state.
fn server_outcome(proposal: Option<&Proposal>, status: u16, v: Value) -> Result<Outcome, String> {
    if status == 409 && v["code"] == "stale_revision" {
        let current = v["message"].as_str().unwrap_or_default().to_string();
        let base = proposal.map(|p| p.base_revision.as_str()).unwrap_or("");
        return Ok(Outcome {
            result: ProposalResult {
                revision: current.clone(),
                violations: vec![server::stale_violation(base, &current)],
                summary: Default::default(),
            },
            log_id: None,
            state: "stale".into(),
        });
    }
    if !(200..300).contains(&status) {
        return Err(server::error_of(status, &v));
    }
    let result: ProposalResult = serde_json::from_value(v["result"].clone())
        .map_err(|e| format!("{ERR}_SERVER_BODY: {e}"))?;
    Ok(Outcome {
        result,
        log_id: v["proposal_id"].as_str().map(str::to_string),
        state: v["state"].as_str().unwrap_or("rejected").to_string(),
    })
}

/// Outcome of propose/apply for the log and the UI.
#[derive(Serialize, Clone, Debug)]
pub struct Outcome {
    #[serde(flatten)]
    pub result: ProposalResult,
    pub log_id: Option<String>,
    /// draft | applied | rejected | pending_review | stale
    pub state: String,
}

#[allow(clippy::too_many_arguments)]
fn entry(
    id: String,
    proposal: &Proposal,
    author: &Author,
    reason: &str,
    result: &ProposalResult,
    state: &str,
    admin: bool,
    idempotency_key: Option<String>,
    tokens: Option<u64>,
) -> LogEntry {
    LogEntry {
        id,
        at_ms: store::now_ms(),
        region: proposal.region,
        author: author.clone(),
        reason: reason.chars().take(280).collect(),
        ops: proposal.ops.clone(),
        base_revision: proposal.base_revision.clone(),
        revision: Some(result.revision.clone()),
        violations: result.violations.clone(),
        state: state.into(),
        summary: serde_json::to_value(&result.summary).unwrap_or(Value::Null),
        admin,
        idempotency_key,
        tokens,
        resolution: None,
    }
}

/// Validate and keep it as a draft (or a rejection) in the log.
pub async fn propose(
    proposal: Proposal,
    author: Author,
    reason: String,
) -> Result<Outcome, String> {
    if let Some(conn) = server::current() {
        let result = server_propose(&conn, &proposal, &author, &reason).await?;
        let log_id = result
            .summary
            .get("proposal_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let state = if store::has_errors(&result.violations) {
            "rejected"
        } else {
            "draft"
        };
        return Ok(Outcome {
            result,
            log_id,
            state: state.into(),
        });
    }
    blocking(move || {
        let (id, charter) = require_local()?;
        let ev = evaluate(&id, &charter, &proposal, &author)?;
        let state = if store::has_errors(&ev.result.violations) {
            "rejected"
        } else {
            "draft"
        };
        let log_id = store::new_log_id();
        let _g = store::lock();
        store::append_log(
            &id,
            &entry(
                log_id.clone(),
                &proposal,
                &author,
                &reason,
                &ev.result,
                state,
                ev.admin,
                None,
                None,
            ),
        )?;
        Ok(Outcome {
            result: ev.result,
            log_id: Some(log_id),
            state: state.into(),
        })
    })
    .await
}

/// Validate and publish, or queue for a person under the `review` policy.
pub async fn apply(
    proposal: Proposal,
    author: Author,
    reason: String,
    idempotency_key: Option<String>,
    tokens: Option<u64>,
) -> Result<Outcome, String> {
    if let Some(conn) = server::current() {
        // The server requires the key; a retry of this call reuses it.
        let key = idempotency_key.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let body = server::proposal_body(&proposal, &author, &reason, Some(&key));
        let (status, v) = conn.post_status(&conn.city_path("apply"), &body).await?;
        let outcome = server_outcome(Some(&proposal), status, v)?;
        if outcome.state == "applied" {
            notify(proposal.region, &outcome.result.revision);
        }
        return Ok(outcome);
    }
    let (outcome, changed) = blocking(move || {
        let (id, charter) = require_local()?;
        let _g = store::lock();
        if let Some(key) = idempotency_key.as_deref() {
            if let Some(prev) = store::read_log(&id)
                .into_iter()
                .find(|e| e.idempotency_key.as_deref() == Some(key) && e.author.name == author.name)
            {
                let result = ProposalResult {
                    revision: prev.revision.clone().unwrap_or_default(),
                    violations: prev.violations.clone(),
                    summary: serde_json::from_value(prev.summary.clone()).unwrap_or_default(),
                };
                return Ok((
                    Outcome {
                        result,
                        log_id: Some(prev.id),
                        state: prev.state,
                    },
                    None,
                ));
            }
        }
        let ev = evaluate(&id, &charter, &proposal, &author)?;
        let log_id = store::new_log_id();
        let errors = store::has_errors(&ev.result.violations);
        let state = if errors {
            "rejected"
        } else if author.is_machine() && charter.publish_policy == "review" {
            "pending_review"
        } else {
            "applied"
        };
        let mut changed = None;
        if state == "applied" {
            if let Some(next) = &ev.next {
                let rev = store::save_program(
                    &id,
                    next,
                    Some(ev.current.revision()),
                    Some(&log_id),
                    &author.name,
                )?;
                changed = Some((proposal.region, rev));
            }
        }
        store::append_log(
            &id,
            &entry(
                log_id.clone(),
                &proposal,
                &author,
                &reason,
                &ev.result,
                state,
                ev.admin,
                idempotency_key.clone(),
                tokens,
            ),
        )?;
        drop(ev.build);
        Ok((
            Outcome {
                result: ev.result,
                log_id: Some(log_id),
                state: state.into(),
            },
            changed,
        ))
    })
    .await?;
    if let Some((region, rev)) = changed {
        notify(region, &rev);
    }
    Ok(outcome)
}

/// A person approves a `pending_review` proposal. It is re-validated on the
/// current revision first: a region that moved on marks it `stale`.
pub async fn approve(log_id: String) -> Result<Outcome, String> {
    if let Some(conn) = server::current() {
        let (status, v) = conn
            .post_status(
                &conn.city_path(&format!("review/{log_id}")),
                &json!({ "approve": true }),
            )
            .await?;
        let mut outcome = server_outcome(None, status, v)?;
        // Approved on a region that moved on and no longer fits.
        if outcome.state == "rejected" {
            outcome.state = "stale".into();
        }
        if outcome.state == "applied" {
            let region = log(500).await.ok().and_then(|l| {
                l["proposals"]
                    .as_array()?
                    .iter()
                    .find(|p| p["id"] == log_id.as_str())
                    .and_then(|p| serde_json::from_value::<[i32; 2]>(p["region"].clone()).ok())
            });
            if let Some(region) = region {
                notify(region, &outcome.result.revision);
            }
        }
        outcome.log_id = Some(log_id);
        return Ok(outcome);
    }
    let (outcome, changed) = blocking(move || {
        let (id, charter) = require_local()?;
        let _g = store::lock();
        let mut e =
            store::log_entry(&id, &log_id).ok_or_else(|| format!("{ERR}_NO_PROPOSAL: {log_id}"))?;
        if e.state != "pending_review" {
            return Err(format!("{ERR}_NOT_PENDING: {log_id} is {}", e.state));
        }
        let current = store::load_program(&id, e.region)?;
        let proposal = Proposal {
            region: e.region,
            base_revision: current.revision(),
            ops: e.ops.clone(),
            admin: false,
        };
        // Approving is the person's act, but the ops stay the machine's.
        let ev = evaluate(&id, &charter, &proposal, &Author::person())?;
        let mut changed = None;
        if store::has_errors(&ev.result.violations) {
            e.state = "stale".into();
        } else if let Some(next) = &ev.next {
            let rev = store::save_program(
                &id,
                next,
                Some(current.revision()),
                Some(&log_id),
                &e.author.name,
            )?;
            changed = Some((e.region, rev));
            e.state = "applied".into();
        }
        e.at_ms = store::now_ms();
        e.revision = Some(ev.result.revision.clone());
        e.violations = ev.result.violations.clone();
        e.resolution = Some("approved by the owner".into());
        store::append_log(&id, &e)?;
        Ok((
            Outcome {
                result: ev.result,
                log_id: Some(log_id),
                state: e.state,
            },
            changed,
        ))
    })
    .await?;
    if let Some((region, rev)) = changed {
        notify(region, &rev);
    }
    Ok(outcome)
}

pub async fn refuse(log_id: String, reason: String) -> Result<Value, String> {
    if let Some(conn) = server::current() {
        let mut body = json!({ "approve": false });
        if !reason.trim().is_empty() {
            body["reason"] = json!(reason.chars().take(280).collect::<String>());
        }
        let v = conn
            .post(&conn.city_path(&format!("review/{log_id}")), &body)
            .await?;
        return Ok(json!({ "log_id": log_id, "state": "refused", "server_state": v["state"] }));
    }
    blocking(move || {
        let (id, _) = require_local()?;
        let _g = store::lock();
        let mut e =
            store::log_entry(&id, &log_id).ok_or_else(|| format!("{ERR}_NO_PROPOSAL: {log_id}"))?;
        if e.state != "pending_review" && e.state != "draft" {
            return Err(format!("{ERR}_NOT_PENDING: {log_id} is {}", e.state));
        }
        e.state = "refused".into();
        e.at_ms = store::now_ms();
        e.resolution = Some(if reason.trim().is_empty() {
            "refused by the owner".into()
        } else {
            reason.chars().take(280).collect()
        });
        store::append_log(&id, &e)?;
        Ok(json!({ "log_id": log_id, "state": "refused" }))
    })
    .await
}

/// Proposals (newest first) and the construction sites.
pub async fn log(limit: usize) -> Result<Value, String> {
    if let Some(conn) = server::current() {
        let v = conn
            .get(&conn.city_path(&format!("log?limit={}", limit.clamp(1, 500))))
            .await?;
        return Ok(server::log_of(v));
    }
    blocking(move || {
        let Some(id) = store::active_id() else {
            return Ok(json!({ "proposals": [], "works": [] }));
        };
        let mut proposals = store::read_log(&id);
        proposals.reverse();
        proposals.truncate(limit.clamp(1, 500));
        Ok(json!({ "proposals": proposals, "works": works(&id) }))
    })
    .await
}

/// Buildings still under construction, in every written region.
pub fn works(city_id: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for r in store::regions(city_id) {
        let Ok(p) = store::load_program(city_id, r) else {
            continue;
        };
        for b in p.buildings.iter().filter(|b| b.progress < 100) {
            out.push(json!({
                "region": r,
                "id": b.id,
                "lot": b.lot,
                "use": b.intent.use_,
                "label": b.intent.label,
                "progress": b.progress,
            }));
        }
    }
    out
}

/// Local construction: every site advances by the time since the last tick
/// (`works_rate_per_min` points a minute). A server city builds on the
/// server's own tick.
pub async fn works_tick() -> Result<Value, String> {
    if server::current().is_some() {
        return Ok(json!({ "mode": "server", "advanced": [] }));
    }
    let (value, changes) = blocking(|| {
        let Some(id) = store::active_id() else {
            return Ok((json!({ "advanced": [] }), Vec::new()));
        };
        let _g = store::lock();
        let mut settings = store::load_settings(&id);
        let now = store::now_ms();
        if settings.works_last_ms == 0 || settings.works_last_ms > now {
            settings.works_last_ms = now;
            store::save_settings(&id, &settings)?;
            return Ok((json!({ "advanced": [] }), Vec::new()));
        }
        let rate = settings.works_rate_per_min.max(1) as u64;
        let elapsed = (now - settings.works_last_ms).min(60 * 60 * 1000);
        let points = elapsed * rate / 60_000;
        if points == 0 {
            return Ok((json!({ "advanced": [] }), Vec::new()));
        }
        let mut advanced = Vec::new();
        let mut changes = Vec::new();
        for r in store::regions(&id) {
            let current = store::load_program(&id, r)?;
            let ops: Vec<PatchOp> = current
                .buildings
                .iter()
                .filter(|b| b.progress < 100)
                .map(|b| PatchOp::SetProgress {
                    id: b.id.clone(),
                    progress: (b.progress as u64 + points).min(100) as u8,
                })
                .collect();
            if ops.is_empty() {
                continue;
            }
            let next = program::apply(&current, &ops, true).map_err(|v| {
                format!(
                    "{ERR}_WORKS: {}",
                    v.first().map(|x| x.message.clone()).unwrap_or_default()
                )
            })?;
            let rev = store::save_program(&id, &next, Some(current.revision()), None, "obras")?;
            for op in &ops {
                if let PatchOp::SetProgress { id: bid, progress } = op {
                    advanced.push(json!({ "region": r, "id": bid, "progress": progress }));
                    if *progress >= 100 {
                        let mut done = LogEntry {
                            id: store::new_log_id(),
                            at_ms: now,
                            region: r,
                            author: Author::system("obras"),
                            reason: format!("{bid} finished"),
                            ops: vec![op.clone()],
                            base_revision: current.revision(),
                            revision: Some(rev.clone()),
                            violations: Vec::new(),
                            state: "work_done".into(),
                            summary: json!({ "building": bid }),
                            admin: true,
                            idempotency_key: None,
                            tokens: None,
                            resolution: None,
                        };
                        done.summary["building"] = json!(bid);
                        store::append_log(&id, &done)?;
                    }
                }
            }
            changes.push((r, rev));
        }
        // Keep the remainder so slow ticks still add up.
        settings.works_last_ms += points * 60_000 / rate;
        store::save_settings(&id, &settings)?;
        Ok((json!({ "advanced": advanced, "points": points }), changes))
    })
    .await?;
    for (r, rev) in changes {
        notify(r, &rev);
    }
    Ok(value)
}

/// The body of `city_render` for a proposal that is not published: apply the
/// ops on a copy and draw that.
pub async fn preview_ops(
    region: [i32; 2],
    ops: Vec<PatchOp>,
    view: &str,
    px: u32,
) -> Result<(Vec<u8>, Vec<Violation>), String> {
    let view = parse_view(view);
    let px = px.clamp(64, 1024);
    blocking(move || {
        let (id, charter) = require_local()?;
        let current = store::load_program(&id, region)?;
        let next = program::apply(&current, &prepare_ops(&ops), false).map_err(|v| {
            format!(
                "{ERR}_OPS: {}",
                serde_json::to_string(&v).unwrap_or_default()
            )
        })?;
        let built = store::build_uncached(&charter, &next);
        let png = render(&built, view, px)?;
        Ok((png, built.violations.clone()))
    })
    .await
}
