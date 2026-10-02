use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use omniget_core::core::canvas::layout::free_spot_in;
use omniget_core::core::canvas::media::{
    self, schema, Actor, CliTransport, GenRequest, Intent, JobStatus, MediaTransport, ModelSchema,
    ModelSummary, Reserve,
};
use omniget_core::core::canvas::model::{
    default_size, new_id, CanvasNode, Connection, ConnectionKind, NodeKind, Point,
};
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::store::CanvasStore;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

pub const EVENT_PROGRESS: &str = "canvas://generate-progress";
pub const VIA: &str = "generate";
const MAX_REFERENCE: u64 = 25 * 1024 * 1024;
const POLL_EVERY: Duration = Duration::from_secs(3);
const POLL_FOR: Duration = Duration::from_secs(20 * 60);
const CACHE_FOR: Duration = Duration::from_secs(10 * 60);
pub const STATES: &[&str] = &[
    "draft",
    "quoted",
    "awaiting_approval",
    "running",
    "done",
    "uncertain",
    "failed",
];

static APP: OnceLock<AppHandle> = OnceLock::new();
type Cached<T> = LazyLock<Mutex<HashMap<String, (Instant, T)>>>;
static SCHEMAS: Cached<ModelSchema> = LazyLock::new(|| Mutex::new(HashMap::new()));
static MODELS: Cached<Vec<ModelSummary>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static BY: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static POLLING: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(Default::default()));

fn store() -> Arc<CanvasStore> {
    crate::canvas::doc::host().store().clone()
}

fn transport() -> Result<Arc<dyn MediaTransport>, String> {
    Ok(Arc::new(CliTransport::locate()?))
}

pub fn start(app: &AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(8)).await;
        resume_open().await;
    });
}

fn emit(ws: &str, node: &str, body: Value) {
    let Some(app) = APP.get() else { return };
    let mut v = json!({ "workspaceId": ws, "nodeId": node });
    if let (Some(o), Some(extra)) = (v.as_object_mut(), body.as_object()) {
        for (k, x) in extra {
            o.insert(k.clone(), x.clone());
        }
    }
    let _ = app.emit(EVENT_PROGRESS, v);
}

pub fn requested_by(node_id: &str, origin: &str) {
    let origin = origin.trim();
    if node_id.is_empty() || origin.is_empty() || origin.contains('#') {
        return;
    }
    let mut by = BY.lock().unwrap_or_else(|e| e.into_inner());
    if by.len() > 1024 && !by.contains_key(node_id) {
        by.clear();
    }
    by.insert(node_id.to_string(), origin.chars().take(80).collect());
}

fn origin_for(node_id: &str, step: &str, detail: Option<&str>) -> String {
    let by = BY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(node_id)
        .cloned()
        .unwrap_or_else(|| "system".into());
    let mut o = format!("{by}#{VIA}:{step}");
    if let Some(d) = detail {
        o.push(':');
        o.extend(d.chars().filter(|c| *c != '#').take(32));
    }
    o
}

fn step_of(data: &Value) -> (&'static str, Option<String>) {
    match data["state"].as_str() {
        Some("quoted") | Some("awaiting_approval") => (
            "quoted",
            data["quote"]["credits"]
                .as_f64()
                .map(|c| format!("{c} credits")),
        ),
        Some("running") if data.get("intentId").is_some() => ("submitted", None),
        Some("running") => ("progress", None),
        Some("done") => ("generated", None),
        Some("failed") => ("failed", None),
        Some("uncertain") => ("uncertain", None),
        Some(_) => ("edited", None),
        None if data["error"].is_string() => ("failed", None),
        None => ("progress", None),
    }
}

fn patch(ws: &str, floor: &str, id: &str, data: Value) -> Result<(), String> {
    let (step, detail) = step_of(&data);
    store()
        .apply(
            ws,
            floor,
            vec![Op::update_node(id, json!({ "data": data }))],
            &origin_for(id, step, detail.as_deref()),
            None,
        )
        .map(|_| ())
}

fn get_cached<T: Clone>(cache: &Cached<T>, key: &str) -> Option<T> {
    let g = cache.lock().unwrap_or_else(|e| e.into_inner());
    g.get(key)
        .filter(|(at, _)| at.elapsed() < CACHE_FOR)
        .map(|(_, v)| v.clone())
}

fn put_cached<T>(cache: &Cached<T>, key: &str, v: T) {
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), (Instant::now(), v));
}

pub async fn models() -> Result<Vec<ModelSummary>, String> {
    if let Some(m) = get_cached(&MODELS, "image") {
        return Ok(m);
    }
    let m = transport()?.models("image").await?;
    put_cached(&MODELS, "image", m.clone());
    Ok(m)
}

pub async fn schema_of(model: &str) -> Result<ModelSchema, String> {
    if let Some(s) = get_cached(&SCHEMAS, model) {
        return Ok(s);
    }
    let s = transport()?.schema(model).await?;
    if s.kind != "image" {
        return Err(format!(
            "PROVIDER_MODEL_UNSUPPORTED: {model} makes {}; only image models for now",
            s.kind
        ));
    }
    put_cached(&SCHEMAS, model, s.clone());
    Ok(s)
}

pub struct Loaded {
    pub ws: String,
    pub floor: String,
    pub node: CanvasNode,
}

impl Loaded {
    fn s(&self, k: &str) -> String {
        self.node.data_str(k).unwrap_or("").to_string()
    }
    fn params(&self) -> Map<String, Value> {
        self.node
            .data
            .get("params")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }
    fn versions(&self) -> Vec<Value> {
        self.node
            .data
            .get("versions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }
    fn state(&self) -> String {
        self.node.data_str("state").unwrap_or("draft").to_string()
    }
}

pub fn load(node_id: &str) -> Result<Loaded, String> {
    let s = store();
    let (ws, floor) = s
        .locate_node(node_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: no node '{node_id}'"))?;
    let node = s
        .read(&ws, &floor, |d| d.node(node_id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: no node '{node_id}'"))?;
    if node.kind != NodeKind::Generation {
        return Err(format!(
            "CANVAS_INVALID: '{node_id}' is not a generation node"
        ));
    }
    Ok(Loaded { ws, floor, node })
}

pub struct RefFile {
    pub node_id: String,
    pub path: PathBuf,
    pub hash: String,
}

fn result_ids(versions: &[Value]) -> Vec<String> {
    versions
        .iter()
        .filter_map(|v| v.get("nodeId").and_then(Value::as_str).map(str::to_string))
        .collect()
}

pub fn references(l: &Loaded) -> Result<Vec<RefFile>, String> {
    let s = store();
    let mut results = result_ids(&l.versions());
    if let Some(target) = l
        .node
        .data
        .get("usedIn")
        .and_then(|u| u.get("nodeId"))
        .and_then(Value::as_str)
    {
        results.push(target.to_string());
    }
    let (conns, nodes) = s.read(&l.ws, &l.floor, |d| {
        (d.connections.clone(), d.nodes.clone())
    })?;
    let files_root = s.workspace_dir(&l.ws).join("files");
    let files_root = files_root.canonicalize().ok();
    let mut out: Vec<RefFile> = Vec::new();
    for c in conns.iter().filter(|c| c.touches(&l.node.id)) {
        let Some(other) = c.other_end(&l.node.id) else {
            continue;
        };
        if results.iter().any(|r| r == other) || out.iter().any(|r| r.node_id == other) {
            continue;
        }
        let Some(n) = nodes.iter().find(|n| n.id == other) else {
            continue;
        };
        let is_image = n.kind == NodeKind::Image
            || (n.kind == NodeKind::File
                && n.data.get("isImage").and_then(Value::as_bool) == Some(true));
        if !is_image {
            continue;
        }
        let label = n.title.clone().unwrap_or_else(|| n.id.clone());
        let raw = n.data_str("path").ok_or_else(|| {
            format!("REFERENCE_NOT_LOCAL: '{label}' has no file; only images saved in this workspace can be references")
        })?;
        let real = Path::new(raw)
            .canonicalize()
            .map_err(|_| format!("REFERENCE_MISSING: the file of '{label}' is gone"))?;
        let inside = files_root
            .as_ref()
            .is_some_and(|root| real.starts_with(root));
        if !inside {
            return Err(format!(
                "REFERENCE_OUTSIDE_WORKSPACE: '{label}' is not stored in the workspace files; add it to the canvas as an image first"
            ));
        }
        let meta = std::fs::metadata(&real).map_err(|e| format!("REFERENCE_MISSING: {e}"))?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_REFERENCE {
            return Err(format!(
                "REFERENCE_INVALID: '{label}' must be an image file up to 25 MB"
            ));
        }
        let bytes = std::fs::read(&real).map_err(|e| format!("REFERENCE_MISSING: {e}"))?;
        if media::sniff_image(&bytes).is_none() {
            return Err(format!(
                "REFERENCE_INVALID: '{label}' is not a PNG, JPEG, WebP or GIF image"
            ));
        }
        out.push(RefFile {
            node_id: n.id.clone(),
            path: real,
            hash: hex::encode(Sha256::digest(&bytes)),
        });
    }
    out.sort_by(|a, b| a.hash.cmp(&b.hash));
    Ok(out)
}

struct Prepared {
    schema: ModelSchema,
    model: String,
    prompt: String,
    params: Map<String, Value>,
    refs: Vec<RefFile>,
    fingerprint: String,
}

async fn prepare(l: &Loaded) -> Result<Prepared, String> {
    let model = l.s("model");
    if model.is_empty() {
        return Err("GENERATION_NO_MODEL: pick a model first".into());
    }
    let prompt = l.s("prompt");
    schema::check_prompt(&prompt)?;
    let sc = schema_of(&model).await?;
    let params = schema::clean_params(&sc, &l.params())?;
    let refs = references(l)?;
    if !refs.is_empty() && !sc.references {
        return Err(format!(
            "PROVIDER_REFERENCES_UNSUPPORTED: {} takes no reference images; remove the image cords or pick another model",
            sc.name
        ));
    }
    let hashes: Vec<String> = refs.iter().map(|r| r.hash.clone()).collect();
    let fingerprint = schema::fingerprint(&model, &prompt, &params, &hashes);
    Ok(Prepared {
        schema: sc,
        model,
        prompt,
        params,
        refs,
        fingerprint,
    })
}

async fn upload_refs(
    t: &Arc<dyn MediaTransport>,
    refs: &[RefFile],
    account: &str,
) -> Result<Vec<String>, String> {
    let ledger = media::ledger()?;
    let mut ids = Vec::new();
    for r in refs {
        let now = media::now_ms();
        let id = match ledger.upload_get(&r.hash, account, now)? {
            Some(id) => id,
            None => {
                let id = t.upload(&r.path).await?;
                ledger.upload_put(&r.hash, account, &id, now)?;
                id
            }
        };
        ids.push(id);
    }
    Ok(ids)
}

fn credits(v: i64) -> f64 {
    (schema::from_micro(v) * 100.0).round() / 100.0
}

pub async fn quote(node_id: &str, actor: Actor) -> Result<Value, String> {
    let l = load(node_id)?;
    if l.state() == "running" {
        return Err("GENERATION_BUSY: a generation is running on this node".into());
    }
    let result = quote_inner(&l, actor).await;
    if let Err(e) = &result {
        let _ = patch(
            &l.ws,
            &l.floor,
            &l.node.id,
            json!({ "error": e, "quote": null }),
        );
    }
    result
}

async fn quote_inner(l: &Loaded, actor: Actor) -> Result<Value, String> {
    let p = prepare(l).await?;
    let t = transport()?;
    let (balance, account) = t.balance().await?;
    let uploads = upload_refs(&t, &p.refs, &account).await?;
    let req = GenRequest {
        model: p.model.clone(),
        prompt: p.prompt.clone(),
        params: p.params.clone(),
        references: uploads,
    };
    let cost = schema::micro(t.cost(&p.schema, &req).await?);
    let balance_micro = schema::micro(balance);
    let ledger = media::ledger()?;
    let now = media::now_ms();
    let q = ledger.quote(
        &l.ws,
        &l.node.id,
        &p.fingerprint,
        cost,
        Some(balance_micro),
        Some(&account),
        now,
    )?;
    let a = ledger.assess(&l.ws, cost, Some(balance_micro), now)?;
    let needs_approval = actor == Actor::Agent && a.needs_approval;
    let view = json!({
        "quoteId": q.id,
        "credits": credits(cost),
        "left": credits(a.left.min(balance_micro)),
        "budgetLeft": credits(a.left),
        "balance": credits(balance_micro),
        "expiresAt": q.expires_ms,
        "fitsBudget": a.fits_budget && a.fits_balance,
        "needsApproval": needs_approval,
        "reason": a.reason,
        "references": p.refs.len(),
    });
    let state = if needs_approval {
        "awaiting_approval"
    } else {
        "quoted"
    };
    patch(
        &l.ws,
        &l.floor,
        &l.node.id,
        json!({ "state": state, "quote": view, "error": null, "modelName": p.schema.name }),
    )?;
    Ok(view)
}

fn intent_json(i: &Intent) -> Value {
    json!({
        "intentId": i.id,
        "state": i.state,
        "jobId": i.provider_id,
        "reserved": credits(i.reserved),
        "actual": i.actual.map(credits),
        "actor": i.actor,
        "requestedBy": BY.lock().unwrap_or_else(|e| e.into_inner()).get(&i.node).cloned(),
        "version": i.version,
        "error": i.error,
    })
}

pub async fn submit(
    node_id: &str,
    quote_id: &str,
    key: Option<String>,
    actor: Actor,
) -> Result<Value, String> {
    let l = load(node_id)?;
    if l.state() == "running" {
        return Err("GENERATION_BUSY: a generation is running on this node".into());
    }
    let ledger = media::ledger()?;
    if let Some(open) = ledger.latest(&l.ws, &l.node.id)? {
        if open.state == "unknown" || open.state == "submitted" {
            let same = key.as_deref().is_some_and(|k| k == open.key);
            if !same {
                return Err("GENERATION_UNCERTAIN: the last generation of this node is not settled; search again or release it first".into());
            }
        }
    }
    let p = prepare(&l).await?;
    let t = transport()?;
    let (balance, account) = t.balance().await?;
    let key = key.unwrap_or_else(|| format!("q-{quote_id}"));
    let reserved = ledger.reserve(Reserve {
        workspace: &l.ws,
        node: &l.node.id,
        key: &key,
        quote: quote_id,
        fingerprint: &p.fingerprint,
        actor,
        model: &p.model,
        prompt: &p.prompt,
        balance: schema::micro(balance),
        now: media::now_ms(),
    });
    let (intent, dispatch) = match reserved {
        Ok(v) => v,
        Err(e) => {
            let current = l.state();
            let state = if e.starts_with("PROVIDER_APPROVAL_REQUIRED") {
                "awaiting_approval"
            } else if e.starts_with("PROVIDER_QUOTE_INVALID") {
                "draft"
            } else {
                current.as_str()
            };
            let _ = patch(
                &l.ws,
                &l.floor,
                &l.node.id,
                json!({ "state": state, "error": e }),
            );
            emit(&l.ws, &l.node.id, json!({ "state": state, "error": e }));
            return Err(e);
        }
    };
    if !dispatch {
        return Ok(intent_json(&intent));
    }
    let started = media::now_ms();
    patch(
        &l.ws,
        &l.floor,
        &l.node.id,
        json!({
            "state": "running",
            "intentId": intent.id,
            "jobId": null,
            "error": null,
            "progress": { "status": "submitting", "startedAt": started },
        }),
    )?;
    emit(
        &l.ws,
        &l.node.id,
        json!({ "state": "running", "status": "submitting", "startedAt": started }),
    );
    let uploads = match upload_refs(&t, &p.refs, &account).await {
        Ok(u) => u,
        Err(e) => {
            let i = ledger.release(&intent.id, media::now_ms())?;
            let _ = patch(
                &l.ws,
                &l.floor,
                &l.node.id,
                json!({ "state": "failed", "error": e, "progress": null }),
            );
            emit(&l.ws, &l.node.id, json!({ "state": "failed", "error": e }));
            return Ok(intent_json(&i));
        }
    };
    let req = GenRequest {
        model: p.model.clone(),
        prompt: p.prompt.clone(),
        params: p.params.clone(),
        references: uploads,
    };
    let created = t.submit(&p.schema, &req).await;
    match created {
        Ok(Some(job)) => {
            let i = ledger.submitted(&intent.id, &job, media::now_ms())?;
            let _ = patch(
                &l.ws,
                &l.floor,
                &l.node.id,
                json!({ "jobId": job, "progress": { "status": "queued", "startedAt": started } }),
            );
            spawn_poll(i.id.clone());
            Ok(intent_json(&i))
        }
        _ => {
            spawn_reconcile(intent.id.clone(), 3);
            Ok(intent_json(&ledger.intent(&intent.id)?))
        }
    }
}

fn spawn_poll(intent_id: String) {
    {
        let mut g = POLLING.lock().unwrap_or_else(|e| e.into_inner());
        if !g.insert(intent_id.clone()) {
            return;
        }
    }
    tauri::async_runtime::spawn(async move {
        let r = poll(&intent_id).await;
        POLLING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&intent_id);
        if let Err(e) = r {
            tracing::warn!("[canvas-generate] poll {intent_id}: {e}");
        }
    });
}

fn spawn_reconcile(intent_id: String, tries: u32) {
    tauri::async_runtime::spawn(async move {
        for n in 0..tries {
            tokio::time::sleep(Duration::from_secs(4 + 6 * n as u64)).await;
            match reconcile_intent(&intent_id).await {
                Ok(true) => return,
                Ok(false) => continue,
                Err(e) => {
                    tracing::warn!("[canvas-generate] reconcile {intent_id}: {e}");
                }
            }
        }
        if let Ok(ledger) = media::ledger() {
            if let Ok(i) = ledger.intent(&intent_id) {
                if i.state == "unknown" {
                    mark_uncertain(&i);
                }
            }
        }
    });
}

fn mark_uncertain(i: &Intent) {
    let Some((ws, floor)) = store().locate_node(&i.node) else {
        return;
    };
    let msg = "GENERATION_UNCERTAIN: Higgsfield did not confirm this job; search again, or release the reserved credits";
    let _ = patch(
        &ws,
        &floor,
        &i.node,
        json!({ "state": "uncertain", "error": msg, "progress": null }),
    );
    emit(&ws, &i.node, json!({ "state": "uncertain", "error": msg }));
}

fn created_ms(job: &JobStatus) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(&job.created_at)
        .ok()
        .map(|d| d.timestamp_millis())
}

async fn reconcile_intent(intent_id: &str) -> Result<bool, String> {
    let ledger = media::ledger()?;
    let i = ledger.intent(intent_id)?;
    match i.state.as_str() {
        "submitted" => {
            spawn_poll(i.id.clone());
            return Ok(true);
        }
        "unknown" => {}
        _ => return Ok(true),
    }
    let t = transport()?;
    let recent = t.recent(30).await?;
    let mut hits: Vec<&JobStatus> = Vec::new();
    for j in &recent {
        if j.model != i.model || j.prompt != i.prompt {
            continue;
        }
        if created_ms(j).is_none_or(|c| c + 60_000 < i.created_ms) {
            continue;
        }
        if ledger.provider_claimed(&j.id)? {
            continue;
        }
        hits.push(j);
    }
    hits.sort_by_key(|j| created_ms(j).unwrap_or(i64::MAX));
    let Some(found) = hits.first() else {
        return Ok(false);
    };
    let i = ledger.submitted(&i.id, &found.id, media::now_ms())?;
    if let Some((ws, floor)) = store().locate_node(&i.node) {
        let _ = patch(
            &ws,
            &floor,
            &i.node,
            json!({ "state": "running", "jobId": found.id, "error": null, "progress": { "status": found.status, "startedAt": i.created_ms } }),
        );
    }
    spawn_poll(i.id.clone());
    Ok(true)
}

async fn poll(intent_id: &str) -> Result<(), String> {
    let ledger = media::ledger()?;
    let t = transport()?;
    let deadline = Instant::now() + POLL_FOR;
    let mut last = String::new();
    let mut failures = 0u32;
    loop {
        let i = ledger.intent(intent_id)?;
        if i.state != "submitted" {
            return Ok(());
        }
        let job_id = i.provider_id.clone().ok_or("PROVIDER_OUTCOME_UNKNOWN")?;
        let Some((ws, floor)) = store().locate_node(&i.node) else {
            if Instant::now() > deadline {
                return Ok(());
            }
            tokio::time::sleep(POLL_EVERY * 10).await;
            continue;
        };
        match t.status(&job_id).await {
            Ok(job) => {
                failures = 0;
                let elapsed = media::now_ms() - i.created_ms;
                if job.status != last {
                    last = job.status.clone();
                    let _ = patch(
                        &ws,
                        &floor,
                        &i.node,
                        json!({ "progress": { "status": job.status, "startedAt": i.created_ms } }),
                    );
                }
                emit(
                    &ws,
                    &i.node,
                    json!({ "state": "running", "status": job.status, "elapsedMs": elapsed, "jobId": job_id }),
                );
                match job.status.as_str() {
                    "completed" => return finish(&i, &job, &ws, &floor).await,
                    "failed" => {
                        let msg = format!("GENERATION_FAILED: Higgsfield says {}", job.raw_status);
                        ledger.settle(
                            &i.id,
                            false,
                            None,
                            None,
                            None,
                            Some(&msg),
                            media::now_ms(),
                        )?;
                        let _ = patch(
                            &ws,
                            &floor,
                            &i.node,
                            json!({ "state": "failed", "error": msg, "progress": null }),
                        );
                        emit(&ws, &i.node, json!({ "state": "failed", "error": msg }));
                        return Ok(());
                    }
                    _ => {}
                }
            }
            Err(e) => {
                failures += 1;
                if failures >= 10 {
                    let msg =
                        format!("GENERATION_UNCERTAIN: lost track of the job ({e}); search again");
                    let _ = patch(
                        &ws,
                        &floor,
                        &i.node,
                        json!({ "state": "uncertain", "error": msg, "progress": null }),
                    );
                    emit(&ws, &i.node, json!({ "state": "uncertain", "error": msg }));
                    return Ok(());
                }
            }
        }
        if Instant::now() > deadline {
            let msg =
                "GENERATION_UNCERTAIN: still not finished after 20 minutes; search again later";
            let _ = patch(
                &ws,
                &floor,
                &i.node,
                json!({ "state": "uncertain", "error": msg, "progress": null }),
            );
            emit(&ws, &i.node, json!({ "state": "uncertain", "error": msg }));
            return Ok(());
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

async fn actual_cost(
    t: &Arc<dyn MediaTransport>,
    i: &Intent,
    job: &JobStatus,
) -> (Option<i64>, Option<String>) {
    let Ok(spends) = t.spends().await else {
        return (None, None);
    };
    let ledger = match media::ledger() {
        Ok(l) => l,
        Err(_) => return (None, None),
    };
    let mut best: Option<(i64, String, i64)> = None;
    for s in spends {
        if job.name.is_empty() || !s.name.starts_with(&job.name) {
            continue;
        }
        let Some(at) = chrono::DateTime::parse_from_rfc3339(&s.at)
            .ok()
            .map(|d| d.timestamp_millis())
        else {
            continue;
        };
        if at + 10_000 < i.created_ms {
            continue;
        }
        let r = format!("{}|{}|{}", s.at, s.name, s.credits);
        if ledger.spend_claimed(&r).unwrap_or(true) {
            continue;
        }
        if best.as_ref().is_none_or(|b| at < b.0) {
            best = Some((at, r, schema::micro(s.credits)));
        }
    }
    match best {
        Some((_, r, c)) => (Some(c), Some(r)),
        None => (None, None),
    }
}

fn dims(bytes: &[u8]) -> Option<(f64, f64)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        .map(|(w, h)| (w as f64, h as f64))
}

async fn finish(i: &Intent, job: &JobStatus, ws: &str, floor: &str) -> Result<(), String> {
    let ledger = media::ledger()?;
    let url = job
        .result_url
        .clone()
        .ok_or("GENERATION_NO_RESULT: the job finished without an image")?;
    let fetched = match media::fetch_result(&url).await {
        Ok(f) => f,
        Err(e) => {
            let msg =
                format!("GENERATION_DOWNLOAD_FAILED: {e}; search again to retry the download");
            let _ = patch(
                ws,
                floor,
                &i.node,
                json!({ "state": "uncertain", "error": msg, "progress": null }),
            );
            emit(ws, &i.node, json!({ "state": "uncertain", "error": msg }));
            return Err(msg);
        }
    };
    let s = store();
    let gen = s
        .read(ws, floor, |d| d.node(&i.node).cloned())?
        .ok_or("CANVAS_NOT_FOUND: the generation node is gone")?;
    let mut versions: Vec<Value> = gen
        .data
        .get("versions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let ws_dir = s.workspace_dir(ws);
    let base = gen.title.clone().unwrap_or_else(|| "Generation".into());
    let mut n = versions
        .iter()
        .filter_map(|v| v.get("version").and_then(Value::as_i64))
        .max()
        .unwrap_or(0)
        .max(versions.len() as i64)
        + 1;
    let library = match omniget_core::core::canvas::assets::put_bytes(&ws_dir, &fetched.bytes) {
        Ok(r) => {
            if let Err(e) = omniget_core::core::canvas::assets::remember_generated(
                &ws_dir,
                &r.id,
                &format!("{base} v{n}"),
            ) {
                tracing::warn!("[canvas.generate] naming the asset of {}: {e}", i.node);
            }
            Some(r)
        }
        Err(e) => {
            tracing::warn!("[canvas.generate] {} kept outside the library: {e}", i.node);
            None
        }
    };
    let path = match &library {
        Some(r) => PathBuf::from(&r.abs),
        None => {
            let dir = ws_dir
                .join("files")
                .join("assets")
                .join("generated")
                .join(&i.node);
            std::fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
            let ext = media::ext_for(&fetched.mime);
            loop {
                let p = dir.join(format!("v{n}.{ext}"));
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&p)
                {
                    Ok(mut f) => {
                        use std::io::Write;
                        f.write_all(&fetched.bytes)
                            .map_err(|e| format!("CANVAS_IO: {e}"))?;
                        break p;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                    Err(e) => return Err(format!("CANVAS_IO: {e}")),
                }
            }
        }
    };
    let asset_id = library.as_ref().map(|r| r.id.clone());
    let t = transport()?;
    let (actual, actual_ref) = actual_cost(&t, i, job).await;
    let settled = ledger.settle(
        &i.id,
        true,
        actual,
        actual_ref.as_deref(),
        Some(n),
        None,
        media::now_ms(),
    )?;
    let params = gen.data.get("params").cloned().unwrap_or(json!({}));
    let (nw, nh) = dims(&fetched.bytes)
        .or_else(|| {
            schema::aspect(
                params.as_object().unwrap_or(&Map::new()),
                get_cached(&SCHEMAS, &i.model).as_ref(),
            )
        })
        .unwrap_or((1.0, 1.0));
    let w = 320.0;
    let h = (w * nh / nw).clamp(120.0, 720.0);
    let title = format!("{base} v{n}");
    let path_s = path.to_string_lossy().to_string();
    let image_id = new_id();
    let at = s.read(ws, floor, |d| {
        free_spot_in(
            d,
            w,
            h,
            Some(Point {
                x: gen.x + gen.w + 60.0,
                y: gen.y + (n as f64 - 1.0) * 40.0,
            }),
            &[],
        )
    })?;
    let z = s.read(ws, floor, |d| d.max_z())? + 1.0;
    let image = CanvasNode {
        id: image_id.clone(),
        kind: NodeKind::Image,
        x: (at.x / 20.0).round() * 20.0,
        y: (at.y / 20.0).round() * 20.0,
        w,
        h: (h / 20.0).round() * 20.0,
        z,
        title: Some(title),
        subtitle: None,
        icon: None,
        color: None,
        group_id: None,
        locked: false,
        stack_id: None,
        data: {
            let mut d = match &library {
                Some(r) => omniget_core::core::canvas::assets::node_data(r, None),
                None => Map::new(),
            };
            if library.is_none() {
                d.insert("path".into(), json!(path_s));
            }
            d.insert("naturalW".into(), json!(nw));
            d.insert("naturalH".into(), json!(nh));
            d.insert("generatedBy".into(), json!(i.node));
            d.insert("version".into(), json!(n));
            Value::Object(d)
        },
    };
    let credits_used = credits(settled.actual.unwrap_or(settled.reserved));
    versions.push(json!({
        "version": n,
        "path": path_s,
        "assetId": asset_id,
        "nodeId": image_id,
        "credits": credits_used,
        "creditsKnown": actual.is_some(),
        "prompt": i.prompt,
        "params": params,
        "model": i.model,
        "jobId": job.id,
        "createdAt": media::now_ms(),
    }));
    let ops = vec![
        Op::AddNode { node: image },
        Op::AddConnection {
            connection: Connection {
                id: new_id(),
                from: i.node.clone(),
                to: image_id.clone(),
                kind: ConnectionKind::Spawn,
                perms: vec![],
                condition: None,
                verify: None,
                label: Some(format!("v{n}")),
                points: vec![],
                from_note_id: None,
                to_note_id: None,
                is_active: None,
            },
        },
        Op::update_node(
            &i.node,
            json!({ "data": {
                "state": "done",
                "versions": versions,
                "progress": null,
                "quote": null,
                "error": null,
                "lastResultNodeId": image_id,
            }}),
        ),
    ];
    let detail = format!("v{n}, {credits_used} credits");
    s.apply(
        ws,
        floor,
        ops,
        &origin_for(&i.node, "generated", Some(&detail)),
        None,
    )?;
    emit(
        ws,
        &i.node,
        json!({ "state": "done", "resultNodeId": image_id, "version": n, "credits": credits_used }),
    );
    Ok(())
}

pub async fn reconcile(node_id: &str) -> Result<Value, String> {
    let l = load(node_id)?;
    let ledger = media::ledger()?;
    let i = ledger
        .latest(&l.ws, &l.node.id)?
        .ok_or("GENERATION_NOTHING_PENDING")?;
    match i.state.as_str() {
        "unknown" => {
            if !reconcile_intent(&i.id).await? {
                mark_uncertain(&i);
            }
        }
        "submitted" => {
            let _ = patch(
                &l.ws,
                &l.floor,
                &l.node.id,
                json!({ "state": "running", "error": null }),
            );
            spawn_poll(i.id.clone());
        }
        "completed" if l.state() == "uncertain" => {
            let _ = patch(
                &l.ws,
                &l.floor,
                &l.node.id,
                json!({ "state": "done", "error": null }),
            );
        }
        _ => {}
    }
    status(node_id)
}

pub fn release(node_id: &str) -> Result<Value, String> {
    let l = load(node_id)?;
    let ledger = media::ledger()?;
    let i = ledger
        .latest(&l.ws, &l.node.id)?
        .ok_or("GENERATION_NOTHING_PENDING")?;
    let i = ledger.release(&i.id, media::now_ms())?;
    let next = if l.versions().is_empty() {
        "draft"
    } else {
        "done"
    };
    patch(
        &l.ws,
        &l.floor,
        &l.node.id,
        json!({ "state": next, "error": null, "progress": null, "quote": null }),
    )?;
    emit(&l.ws, &l.node.id, json!({ "state": next }));
    Ok(intent_json(&i))
}

pub fn status(node_id: &str) -> Result<Value, String> {
    let l = load(node_id)?;
    let ledger = media::ledger()?;
    let intent = ledger.latest(&l.ws, &l.node.id)?;
    let versions = l.versions();
    Ok(json!({
        "nodeId": l.node.id,
        "workspaceId": l.ws,
        "state": l.state(),
        "progress": l.node.data.get("progress").cloned().unwrap_or(Value::Null),
        "error": l.node.data.get("error").cloned().unwrap_or(Value::Null),
        "quote": l.node.data.get("quote").cloned().unwrap_or(Value::Null),
        "version": versions.len(),
        "resultNodeId": l.node.data.get("lastResultNodeId").cloned().unwrap_or(Value::Null),
        "assetId": versions.last().and_then(|v| v.get("assetId")).cloned().unwrap_or(Value::Null),
        "intent": intent.as_ref().map(intent_json),
    }))
}

pub fn versions(node_id: &str) -> Result<Value, String> {
    let l = load(node_id)?;
    let list: Vec<Value> = l
        .versions()
        .into_iter()
        .map(|v| {
            json!({
                "version": v.get("version"),
                "assetPath": v.get("path"),
                "assetId": v.get("assetId"),
                "nodeId": v.get("nodeId"),
                "credits": v.get("credits"),
                "prompt": v.get("prompt"),
                "model": v.get("model"),
                "params": v.get("params"),
                "createdAt": v.get("createdAt"),
            })
        })
        .collect();
    Ok(json!({ "nodeId": l.node.id, "versions": list }))
}

pub async fn budget(
    ws: &str,
    ceiling: Option<f64>,
    auto_limit: Option<f64>,
    auto_per_hour: Option<i64>,
    with_balance: bool,
) -> Result<Value, String> {
    store().workspace(ws)?;
    let ledger = media::ledger()?;
    let b = if ceiling.is_some() || auto_limit.is_some() || auto_per_hour.is_some() {
        for v in [ceiling, auto_limit].into_iter().flatten() {
            if !v.is_finite() || v < 0.0 {
                return Err("PROVIDER_INVALID_BUDGET".into());
            }
        }
        ledger.set_budget(
            ws,
            ceiling.map(schema::micro),
            auto_limit.map(schema::micro),
            auto_per_hour,
        )?
    } else {
        ledger.budget(ws)?
    };
    let balance = if with_balance {
        match transport() {
            Ok(t) => t.balance().await.ok().map(|(c, _)| c),
            Err(_) => None,
        }
    } else {
        None
    };
    Ok(json!({
        "workspaceId": ws,
        "ceiling": credits(b.ceiling),
        "reserved": credits(b.reserved),
        "spent": credits(b.spent),
        "left": credits(b.left()),
        "autoLimit": credits(b.auto_per_gen),
        "autoPerHour": b.auto_per_hour,
        "balance": balance,
    }))
}

pub struct CreateSpec {
    pub model: String,
    pub prompt: String,
    pub params: Map<String, Value>,
    pub title: Option<String>,
    pub used_in: Option<String>,
    pub used_in_node: Option<String>,
    pub refs: Vec<String>,
    pub at: Option<Point>,
    pub by: Option<String>,
}

pub async fn create(ws: &str, floor: &str, spec: CreateSpec) -> Result<Value, String> {
    let sc = schema_of(&spec.model).await?;
    schema::check_prompt(&spec.prompt)?;
    let params = schema::clean_params(&sc, &spec.params)?;
    if !spec.refs.is_empty() && !sc.references {
        return Err(format!(
            "PROVIDER_REFERENCES_UNSUPPORTED: {} takes no reference images",
            sc.name
        ));
    }
    let s = store();
    let doc = s.read(ws, floor, |d| d.clone())?;
    for r in &spec.refs {
        let n = doc
            .node(r)
            .ok_or_else(|| format!("CANVAS_NOT_FOUND: no image '{r}' on this floor"))?;
        if n.kind != NodeKind::Image {
            return Err(format!("CANVAS_INVALID: '{r}' is not an image node"));
        }
    }
    if let Some(u) = &spec.used_in_node {
        if doc.node(u).is_none() {
            return Err(format!("CANVAS_NOT_FOUND: no node '{u}' on this floor"));
        }
    }
    let size = default_size(&NodeKind::Generation);
    let near = spec.at.or_else(|| {
        spec.refs.first().and_then(|r| doc.node(r)).map(|n| Point {
            x: n.x + n.w + 60.0,
            y: n.y,
        })
    });
    let at = match spec.at {
        Some(p) => p,
        None => free_spot_in(&doc, size.w, size.h, near, &[]),
    };
    let id = new_id();
    let mut used_in = Map::new();
    if let Some(t) = spec.used_in.as_ref().filter(|t| !t.trim().is_empty()) {
        used_in.insert("text".into(), json!(t.trim()));
    }
    if let Some(u) = &spec.used_in_node {
        used_in.insert("nodeId".into(), json!(u));
        if !used_in.contains_key("text") {
            if let Some(t) = doc.node(u).and_then(|n| n.title.clone()) {
                used_in.insert("text".into(), json!(t));
            }
        }
    }
    let node = CanvasNode {
        id: id.clone(),
        kind: NodeKind::Generation,
        x: (at.x / 20.0).round() * 20.0,
        y: (at.y / 20.0).round() * 20.0,
        w: size.w,
        h: size.h,
        z: doc.max_z() + 1.0,
        title: Some(spec.title.clone().unwrap_or_else(|| "Generation".into())),
        subtitle: None,
        icon: None,
        color: None,
        group_id: None,
        locked: false,
        stack_id: None,
        data: json!({
            "model": sc.model,
            "modelName": sc.name,
            "prompt": spec.prompt,
            "params": params,
            "usedIn": if used_in.is_empty() { Value::Null } else { Value::Object(used_in) },
            "versions": [],
            "state": "draft",
        }),
    };
    let mut ops = vec![Op::AddNode { node }];
    let cord = |from: &str, to: &str, kind: ConnectionKind, label: &str| Op::AddConnection {
        connection: Connection {
            id: new_id(),
            from: from.to_string(),
            to: to.to_string(),
            kind,
            perms: vec![],
            condition: None,
            verify: None,
            label: Some(label.to_string()),
            points: vec![],
            from_note_id: None,
            to_note_id: None,
            is_active: None,
        },
    };
    for r in &spec.refs {
        ops.push(cord(r, &id, ConnectionKind::Message, "reference"));
    }
    if let Some(u) = &spec.used_in_node {
        ops.push(cord(&id, u, ConnectionKind::Note, "used in"));
    }
    if let Some(by) = &spec.by {
        requested_by(&id, by);
    }
    s.apply(ws, floor, ops, &origin_for(&id, "created", None), None)?;
    let n = s.read(ws, floor, |d| d.node(&id).cloned())?;
    Ok(json!({ "nodeId": id, "node": n, "schema": sc }))
}

async fn resume_open() {
    let Ok(ledger) = media::ledger() else {
        return;
    };
    let Ok(open) = ledger.open_intents() else {
        return;
    };
    for i in open {
        match i.state.as_str() {
            "submitted" => spawn_poll(i.id.clone()),
            "unknown" => spawn_reconcile(i.id.clone(), 1),
            _ => {}
        }
    }
}
