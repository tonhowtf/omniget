//! Local city store: `<app_data>/city3d/`.
//!
//! ```text
//! city3d/active.json                    {"city_id": "..."}  (one active city)
//! city3d/<city_id>/charter.json
//! city3d/<city_id>/programs/<rx>_<ry>.json     current program of a region
//! city3d/<city_id>/revisions/<rx>_<ry>.jsonl   {revision, parent, at_ms, log_id, author}
//! city3d/<city_id>/revisions/<rx>_<ry>/<hex>.json   every program ever published
//! city3d/<city_id>/log.jsonl            proposals, one line per state change
//! city3d/<city_id>/budget.json          token budget per role and period
//! city3d/<city_id>/settings.json        council schedule, construction pace
//! ```
//!
//! Writes go through one lock and land with write-then-rename, so a crash or
//! a provider dropping mid-round never leaves a region half-written. Solving
//! and meshing are cached per (charter, revision) in memory.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};

use omniget_citygen::program::{CityProgram, PatchOp};
use omniget_citygen::{
    checker, hash, mesher, solver, Blueprint, Charter, InstanceList, Severity, TerrainMesh,
    Violation,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ERR;

// ── Paths ──────────────────────────────────────────────────────────────

static ROOT_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);
static WRITE: Mutex<()> = Mutex::new(());

/// Tests and tools point the store somewhere else.
pub fn set_root(dir: Option<PathBuf>) {
    if let Ok(mut g) = ROOT_OVERRIDE.write() {
        *g = dir;
    }
    clear_cache();
}

pub fn root() -> Result<PathBuf, String> {
    if let Some(dir) = ROOT_OVERRIDE.read().ok().and_then(|g| g.clone()) {
        return Ok(dir);
    }
    crate::core::paths::app_data_dir()
        .map(|d| d.join("city3d"))
        .ok_or_else(|| format!("{ERR}_NO_DATA_DIR"))
}

/// One writer at a time for the whole store.
pub fn lock() -> MutexGuard<'static, ()> {
    WRITE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn city_dir(city_id: &str) -> Result<PathBuf, String> {
    if !safe_id(city_id) {
        return Err(format!("{ERR}_BAD_ID: city id `{city_id}`"));
    }
    Ok(root()?.join(city_id))
}

fn region_key(region: [i32; 2]) -> String {
    format!("{}_{}", region[0], region[1])
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{ERR}_IO: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{ERR}_IO: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{ERR}_IO: {e}"))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    write_atomic(path, &text)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = std::fs::read(path).ok()?;
    serde_json::from_slice(&text).ok()
}

fn append_line<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{ERR}_IO: {e}"))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{ERR}_IO: {e}"))?;
    let mut line = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    line.push(b'\n');
    f.write_all(&line).map_err(|e| format!("{ERR}_IO: {e}"))?;
    f.sync_data().map_err(|e| format!("{ERR}_IO: {e}"))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Active city, charter, programs ─────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct Active {
    city_id: Option<String>,
}

pub fn active_id() -> Option<String> {
    let root = root().ok()?;
    let a: Active = read_json(&root.join("active.json"))?;
    let id = a.city_id?;
    root.join(&id).join("charter.json").exists().then_some(id)
}

pub fn set_active(city_id: &str) -> Result<(), String> {
    write_json(
        &root()?.join("active.json"),
        &Active {
            city_id: Some(city_id.to_string()),
        },
    )
}

pub fn load_charter(city_id: &str) -> Result<Charter, String> {
    read_json(&city_dir(city_id)?.join("charter.json"))
        .ok_or_else(|| format!("{ERR}_NOT_FOUNDED: no charter for `{city_id}`"))
}

pub fn save_charter(charter: &Charter) -> Result<(), String> {
    write_json(&city_dir(&charter.city_id)?.join("charter.json"), charter)?;
    clear_cache();
    Ok(())
}

/// The region's current program; an unwritten region is an empty program.
pub fn load_program(city_id: &str, region: [i32; 2]) -> Result<CityProgram, String> {
    let path = city_dir(city_id)?
        .join("programs")
        .join(format!("{}.json", region_key(region)));
    Ok(read_json(&path).unwrap_or_else(|| CityProgram::empty(city_id, region)))
}

/// Regions that have a written program, sorted.
pub fn regions(city_id: &str) -> Vec<[i32; 2]> {
    let Ok(dir) = city_dir(city_id).map(|d| d.join("programs")) else {
        return Vec::new();
    };
    let mut out: Vec<[i32; 2]> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let stem = name.strip_suffix(".json")?;
            let (x, y) = stem.split_once('_')?;
            Some([x.parse().ok()?, y.parse().ok()?])
        })
        .collect();
    out.sort();
    out
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RevisionRecord {
    pub revision: String,
    pub parent: Option<String>,
    pub at_ms: u64,
    pub log_id: Option<String>,
    pub author: String,
}

/// Publishes a region's program: the snapshot, the history line, then the
/// current file (the last write is the switch).
pub fn save_program(
    city_id: &str,
    program: &CityProgram,
    parent: Option<String>,
    log_id: Option<&str>,
    author: &str,
) -> Result<String, String> {
    let dir = city_dir(city_id)?;
    let key = region_key(program.region);
    let revision = program.revision();
    let hex = revision.trim_start_matches("sha256:");
    write_json(
        &dir.join("revisions").join(&key).join(format!("{hex}.json")),
        program,
    )?;
    append_line(
        &dir.join("revisions").join(format!("{key}.jsonl")),
        &RevisionRecord {
            revision: revision.clone(),
            parent,
            at_ms: now_ms(),
            log_id: log_id.map(str::to_string),
            author: author.to_string(),
        },
    )?;
    write_json(&dir.join("programs").join(format!("{key}.json")), program)?;
    Ok(revision)
}

pub fn history(city_id: &str, region: [i32; 2]) -> Vec<RevisionRecord> {
    let Ok(dir) = city_dir(city_id) else {
        return Vec::new();
    };
    let path = dir
        .join("revisions")
        .join(format!("{}.jsonl", region_key(region)));
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// A published program by revision (for undo and diffs).
pub fn program_at(city_id: &str, region: [i32; 2], revision: &str) -> Option<CityProgram> {
    let hex = revision.trim_start_matches("sha256:");
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    read_json(
        &city_dir(city_id)
            .ok()?
            .join("revisions")
            .join(region_key(region))
            .join(format!("{hex}.json")),
    )
}

// ── Log ────────────────────────────────────────────────────────────────

/// Who asked for a change. Never a chain of thought: `reason` is one short
/// sentence the author gave.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Author {
    /// person | llm | mcp | system
    pub kind: String,
    pub name: String,
    /// prefeitura | conselho | morador | fundador (LLM roles)
    #[serde(default)]
    pub role: Option<String>,
}

impl Author {
    pub fn person() -> Self {
        Author {
            kind: "person".into(),
            name: "owner".into(),
            role: None,
        }
    }
    pub fn system(name: &str) -> Self {
        Author {
            kind: "system".into(),
            name: name.into(),
            role: None,
        }
    }
    /// LLM authors never get `admin` and go through the publish policy.
    pub fn is_machine(&self) -> bool {
        matches!(self.kind.as_str(), "llm" | "mcp")
    }
}

/// One proposal. `log.jsonl` keeps one line per state change; the last line
/// of an id wins.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LogEntry {
    pub id: String,
    pub at_ms: u64,
    pub region: [i32; 2],
    pub author: Author,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub ops: Vec<PatchOp>,
    #[serde(default)]
    pub base_revision: String,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub violations: Vec<Violation>,
    /// draft | applied | rejected | pending_review | refused | stale |
    /// founded | work_done
    pub state: String,
    #[serde(default)]
    pub summary: Value,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub tokens: Option<u64>,
    /// Why a person approved or refused it.
    #[serde(default)]
    pub resolution: Option<String>,
}

pub fn new_log_id() -> String {
    format!("p{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

pub fn append_log(city_id: &str, entry: &LogEntry) -> Result<(), String> {
    append_line(&city_dir(city_id)?.join("log.jsonl"), entry)
}

/// Every proposal in first-seen order with its latest state.
pub fn read_log(city_id: &str) -> Vec<LogEntry> {
    let Ok(dir) = city_dir(city_id) else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(dir.join("log.jsonl")).unwrap_or_default();
    let mut order: Vec<String> = Vec::new();
    let mut last: HashMap<String, LogEntry> = HashMap::new();
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<LogEntry>(line) else {
            continue;
        };
        if !last.contains_key(&e.id) {
            order.push(e.id.clone());
        }
        last.insert(e.id.clone(), e);
    }
    order
        .into_iter()
        .filter_map(|id| last.remove(&id))
        .collect()
}

pub fn log_entry(city_id: &str, id: &str) -> Option<LogEntry> {
    read_log(city_id).into_iter().find(|e| e.id == id)
}

// ── Budget and settings ────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoleBudget {
    pub limit_tokens: u64,
    pub used_tokens: u64,
    pub period_start_ms: u64,
    pub period_ms: u64,
}

pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;

pub fn default_limit(role: &str) -> u64 {
    match role {
        "prefeitura" => 200_000,
        "conselho" => 60_000,
        "morador" => 100_000,
        _ => 300_000,
    }
}

pub fn load_budgets(city_id: &str) -> BTreeMap<String, RoleBudget> {
    city_dir(city_id)
        .ok()
        .and_then(|d| read_json(&d.join("budget.json")))
        .unwrap_or_default()
}

pub fn save_budgets(city_id: &str, b: &BTreeMap<String, RoleBudget>) -> Result<(), String> {
    write_json(&city_dir(city_id)?.join("budget.json"), b)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// Council round every N minutes while the app runs; `None` = manual.
    pub council_every_min: Option<u32>,
    /// Construction progress points per minute (100 = one building).
    pub works_rate_per_min: u32,
    pub works_last_ms: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            council_every_min: None,
            works_rate_per_min: 20,
            works_last_ms: 0,
        }
    }
}

pub fn load_settings(city_id: &str) -> Settings {
    city_dir(city_id)
        .ok()
        .and_then(|d| read_json(&d.join("settings.json")))
        .unwrap_or_default()
}

pub fn save_settings(city_id: &str, s: &Settings) -> Result<(), String> {
    write_json(&city_dir(city_id)?.join("settings.json"), s)
}

// ── Solve / check / mesh cache ─────────────────────────────────────────

/// A region as the generator sees it at one revision.
pub struct RegionBuild {
    pub revision: String,
    pub blueprint: Option<Blueprint>,
    pub violations: Vec<Violation>,
    mesh: OnceLock<Option<(InstanceList, TerrainMesh)>>,
}

impl RegionBuild {
    pub fn from_parts(
        revision: String,
        blueprint: Option<Blueprint>,
        violations: Vec<Violation>,
    ) -> Self {
        RegionBuild {
            revision,
            blueprint,
            violations,
            mesh: OnceLock::new(),
        }
    }

    /// Instances and terrain, computed on first use.
    pub fn mesh(&self) -> Option<&(InstanceList, TerrainMesh)> {
        self.mesh
            .get_or_init(|| {
                self.blueprint
                    .as_ref()
                    .map(|bp| (mesher::mesh_region(bp), mesher::terrain_mesh(bp)))
            })
            .as_ref()
    }

    pub fn errors(&self) -> usize {
        self.violations
            .iter()
            .filter(|v| v.severity == Severity::Error)
            .count()
    }
}

static CACHE: Mutex<Option<HashMap<String, Arc<RegionBuild>>>> = Mutex::new(None);
const CACHE_MAX: usize = 48;

pub fn clear_cache() {
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
}

/// Solve + check, no cache (used for proposals that may never be published).
pub fn build_uncached(charter: &Charter, program: &CityProgram) -> RegionBuild {
    let revision = program.revision();
    let (blueprint, violations) = match solver::solve(charter, program) {
        Ok(bp) => {
            let v = checker::check(&bp, program);
            (Some(bp), v)
        }
        Err(v) if v.is_empty() => (
            None,
            vec![violation(
                "P-SOLVE",
                &format!("{:?}", program.region),
                "",
                "the solver could not build this region",
            )],
        ),
        Err(v) => (None, v),
    };
    RegionBuild {
        revision,
        blueprint,
        violations,
        mesh: OnceLock::new(),
    }
}

/// Solve + check, cached by (charter, city, region, revision).
pub fn build(charter: &Charter, program: &CityProgram) -> Arc<RegionBuild> {
    let key = format!(
        "{}|{}|{:?}|{}",
        hash::canonical_hash(charter),
        charter.city_id,
        program.region,
        program.revision()
    );
    if let Some(hit) = CACHE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(&key).cloned()))
    {
        return hit;
    }
    let built = Arc::new(build_uncached(charter, program));
    if let Ok(mut g) = CACHE.lock() {
        let map = g.get_or_insert_with(HashMap::new);
        if map.len() >= CACHE_MAX {
            map.clear();
        }
        map.insert(key, built.clone());
    }
    built
}

pub fn violation(code: &str, id: &str, measure: &str, message: &str) -> Violation {
    Violation {
        code: code.into(),
        id: id.into(),
        measure: measure.into(),
        message: message.into(),
        severity: Severity::Error,
    }
}

pub fn has_errors(v: &[Violation]) -> bool {
    v.iter().any(|x| x.severity == Severity::Error)
}

/// Counts and lot diff between two builds, for the log and the LLM.
pub fn summary(before: Option<&RegionBuild>, after: &RegionBuild) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let errors = after.errors();
    out.insert("errors".into(), json!(errors));
    out.insert("warnings".into(), json!(after.violations.len() - errors));
    if let Some(bp) = &after.blueprint {
        out.insert("buildings".into(), json!(bp.buildings.len()));
        out.insert("lots".into(), json!(bp.lots.len()));
        out.insert("streets".into(), json!(bp.streets.len()));
        out.insert(
            "works".into(),
            json!(bp.buildings.iter().filter(|b| b.progress < 100).count()),
        );
        if let Some(prev) = before.and_then(|b| b.blueprint.as_ref()) {
            let old: std::collections::BTreeSet<&str> =
                prev.lots.iter().map(|l| l.id.as_str()).collect();
            let new: std::collections::BTreeSet<&str> =
                bp.lots.iter().map(|l| l.id.as_str()).collect();
            let added: Vec<&str> = new.difference(&old).copied().take(30).collect();
            let removed: Vec<&str> = old.difference(&new).copied().take(30).collect();
            out.insert("lots_added".into(), json!(added));
            out.insert("lots_removed".into(), json!(removed));
            let oldb: std::collections::BTreeSet<&str> =
                prev.buildings.iter().map(|b| b.id.as_str()).collect();
            let newb: std::collections::BTreeSet<&str> =
                bp.buildings.iter().map(|b| b.id.as_str()).collect();
            out.insert(
                "buildings_added".into(),
                json!(newb.difference(&oldb).copied().take(30).collect::<Vec<_>>()),
            );
            out.insert(
                "buildings_removed".into(),
                json!(oldb.difference(&newb).copied().take(30).collect::<Vec<_>>()),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_keeps_the_latest_state_in_first_seen_order() {
        let dir = std::env::temp_dir().join(format!("city3d-store-{}", uuid::Uuid::new_v4()));
        set_root(Some(dir.clone()));
        let mk = |id: &str, state: &str| LogEntry {
            id: id.into(),
            at_ms: 1,
            region: [0, 0],
            author: Author::person(),
            reason: String::new(),
            ops: vec![],
            base_revision: String::new(),
            revision: None,
            violations: vec![],
            state: state.into(),
            summary: Value::Null,
            admin: false,
            idempotency_key: None,
            tokens: None,
            resolution: None,
        };
        append_log("c1", &mk("a", "draft")).unwrap();
        append_log("c1", &mk("b", "pending_review")).unwrap();
        append_log("c1", &mk("a", "applied")).unwrap();
        let log = read_log("c1");
        assert_eq!(log.len(), 2);
        assert_eq!(
            (log[0].id.as_str(), log[0].state.as_str()),
            ("a", "applied")
        );
        assert_eq!(log[1].state, "pending_review");
        set_root(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ids_cannot_escape_the_store() {
        assert!(safe_id("omni-2026_a"));
        assert!(!safe_id("../x"));
        assert!(!safe_id(""));
    }
}
