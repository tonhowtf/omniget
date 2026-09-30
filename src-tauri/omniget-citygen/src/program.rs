//! The city program: what an LLM (or a person) writes. Declarative intent,
//! never geometry (estudo 99, arquivo 09 §1). Types are a contract shared by
//! the solver, the server and the MCP tools: the data shapes here are owned by
//! the orchestrator; agent B owns `apply` and the helpers below the types.

use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One region's program (`omni.city.program/v1`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct CityProgram {
    pub schema: String,
    pub city_id: String,
    pub region: [i32; 2],
    #[serde(default)]
    pub catalog: String,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub streets: Vec<Street>,
    #[serde(default)]
    pub water: Vec<WaterBody>,
    #[serde(default)]
    pub landmarks: Vec<Landmark>,
    #[serde(default)]
    pub lots: LotRules,
    #[serde(default)]
    pub buildings: Vec<BuildingSpec>,
    #[serde(default)]
    pub fill: Vec<FillRule>,
    #[serde(default)]
    pub props: Vec<PropSpec>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Zone {
    pub id: String,
    /// civico | residencial | comercial | rural | parque
    pub kind: String,
    /// x, y, w, h in region tiles.
    pub rect: [i32; 4],
    pub style: String,
    /// [min, max] floors.
    pub floors: [u8; 2],
    /// 0..=1000.
    #[serde(default = "d500")]
    pub density: u16,
    #[serde(default)]
    pub setback: Setback,
    #[serde(default)]
    pub palette: Option<String>,
}

fn d500() -> u16 {
    500
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Setback {
    #[serde(default)]
    pub front: i32,
    #[serde(default)]
    pub side: i32,
    #[serde(default)]
    pub back: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Street {
    pub id: String,
    /// avenida | rua | beco | trilha
    pub class: String,
    pub width: i32,
    /// Polyline of tile points (axis-aligned or diagonal steps).
    pub path: Vec<[i32; 2]>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WaterBody {
    pub id: String,
    pub rect: [i32; 4],
    #[serde(default)]
    pub bridges: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Landmark {
    pub id: String,
    /// praca | parque | mercado | porto | templo
    pub kind: String,
    pub rect: [i32; 4],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LotRules {
    #[serde(default = "yes")]
    pub auto: bool,
    #[serde(default = "d4")]
    pub min_front: i32,
    #[serde(default = "d5")]
    pub min_depth: i32,
    #[serde(default = "d60")]
    pub max_area: i32,
    #[serde(default = "d5")]
    pub min_buildable: i32,
    /// Lots fixed by hand or migrated from the 2D city (M8): kept verbatim,
    /// never re-split.
    #[serde(default)]
    pub explicit: Vec<ExplicitLot>,
}

impl Default for LotRules {
    fn default() -> Self {
        LotRules {
            auto: true,
            min_front: 4,
            min_depth: 5,
            max_area: 60,
            min_buildable: 5,
            explicit: Vec::new(),
        }
    }
}

fn yes() -> bool {
    true
}
fn d4() -> i32 {
    4
}
fn d5() -> i32 {
    5
}
fn d60() -> i32 {
    60
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExplicitLot {
    pub id: String,
    pub rect: [i32; 4],
    pub front: Dir,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub owner_ref: Option<String>,
    /// Door tile fixed by migration (must not move).
    #[serde(default)]
    pub door: Option<[i32; 2]>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BuildingSpec {
    pub id: String,
    /// Exact lot id, `auto:near:<landmark>` or `auto:zone:<zone>`. Frozen to
    /// the resolved id on publish.
    pub lot: String,
    #[serde(default)]
    pub owner_ref: Option<String>,
    pub intent: Intent,
    /// Script typology from M5 (`codegeo:<sha256>`), replaces the grammar's
    /// massing but not the building code.
    #[serde(default)]
    pub typology: Option<String>,
    /// Construction progress 0..=100; new works start at 0 and the
    /// simulation advances them.
    #[serde(default = "d100")]
    pub progress: u8,
}

fn d100() -> u8 {
    100
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Intent {
    /// casa | loja-sobrado | civico | templo | galpao
    #[serde(rename = "use", default)]
    pub use_: String,
    #[serde(default)]
    pub floors: Option<u8>,
    /// rect | L | U | patio
    #[serde(default)]
    pub shape: Option<String>,
    /// duas-aguas | quatro-aguas | plano | conico
    #[serde(default)]
    pub roof: Option<String>,
    #[serde(default)]
    pub palette: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub balcony: bool,
    #[serde(default)]
    pub awning: bool,
    #[serde(default)]
    pub tower: bool,
    #[serde(default)]
    pub chimney: bool,
    /// Free text (name, vibe): signs and labels only, never read by the solver.
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FillRule {
    pub zone: String,
    #[serde(rename = "use")]
    pub use_: String,
    #[serde(default)]
    pub shape: Vec<String>,
    #[serde(default)]
    pub roof: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PropSpec {
    pub kind: String,
    #[serde(default)]
    pub preset: String,
    /// Either explicit cells or a scatter rule.
    #[serde(default)]
    pub cells: Vec<[i32; 2]>,
    #[serde(default)]
    pub scatter: Option<Scatter>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Scatter {
    pub zone: String,
    /// ground | street | plaza
    #[serde(default)]
    pub on: String,
    pub spacing: i32,
}

/// Patch operations (arquivo 09 §1.1).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchOp {
    AddZone(Zone),
    AddStreet(Street),
    AddLandmark(Landmark),
    AddWater(WaterBody),
    AddBuilding(BuildingSpec),
    AddProps(PropSpec),
    AddFill(FillRule),
    /// Merge fields into a building's intent.
    SetIntent {
        id: String,
        patch: serde_json::Value,
    },
    /// Change a building's `lot` or a street's `path`.
    Move {
        id: String,
        #[serde(default)]
        lot: Option<String>,
        #[serde(default)]
        path: Option<Vec<[i32; 2]>>,
    },
    /// Set construction progress of a building (the simulation does this).
    SetProgress {
        id: String,
        progress: u8,
    },
    Remove {
        id: String,
    },
}

/// A proposal as sent by an LLM or a person.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Proposal {
    pub region: [i32; 2],
    pub base_revision: String,
    pub ops: Vec<PatchOp>,
    /// Lets a patch touch lots with `owner_ref` (owner or delegate only).
    #[serde(default)]
    pub admin: bool,
}

/// What `propose`/`apply` answer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProposalResult {
    pub revision: String,
    pub violations: Vec<Violation>,
    /// Counts and lot diff, free-form for LLM consumption.
    pub summary: BTreeMap<String, serde_json::Value>,
}

impl CityProgram {
    pub fn empty(city_id: &str, region: [i32; 2]) -> Self {
        CityProgram {
            schema: "omni.city.program/v1".into(),
            city_id: city_id.into(),
            region,
            catalog: "kit-vila-v1".into(),
            ..Default::default()
        }
    }

    /// `sha256:<hex>` of the canonical program: the revision id.
    pub fn revision(&self) -> String {
        crate::hash::canonical_hash(self)
    }
}

/// Apply ops on a copy. Structural errors come back as P-codes; the building
/// code is the checker's job. Owner: agent B.
///
/// P-codes: P1 duplicate id, P2 unknown id, P3 rect or point outside the
/// region, P4 empty path or bad width, P5 unknown zone, P6 unknown landmark,
/// P7 owned lot touched without `admin`, P8 invalid value.
pub fn apply(p: &CityProgram, ops: &[PatchOp], admin: bool) -> Result<CityProgram, Vec<Violation>> {
    let mut next = p.clone();
    let mut errs = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        if let Err(v) = apply_one(&mut next, op, admin) {
            errs.push(Violation {
                message: format!("op {i} ({}): {}", op_name(op), v.message),
                ..v
            });
        }
    }
    if errs.is_empty() {
        errs = validate(&next);
    }
    if errs.is_empty() {
        Ok(next)
    } else {
        Err(errs)
    }
}

/// Structural checks of a whole program (P-codes). Empty = the solver can run.
pub fn validate(p: &CityProgram) -> Vec<Violation> {
    let mut v = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for id in all_ids(p) {
        if !seen.insert(id.clone()) {
            v.push(perr("P1", &id, "duplicate", format!("id `{id}` is used twice; ids are shared by zones, streets, water, landmarks and buildings")));
        }
    }
    for z in &p.zones {
        check_rect(&mut v, &z.id, z.rect);
        if z.floors[0] == 0 || z.floors[0] > z.floors[1] {
            v.push(perr(
                "P8",
                &z.id,
                &format!("floors {:?}", z.floors),
                format!(
                    "zone `{}` floors must be [min, max] with 1 <= min <= max",
                    z.id
                ),
            ));
        }
        if z.density > 1000 {
            v.push(perr(
                "P8",
                &z.id,
                &format!("density {}", z.density),
                format!("zone `{}` density is per mille (0..=1000)", z.id),
            ));
        }
    }
    for w in &p.water {
        check_rect(&mut v, &w.id, w.rect);
        for b in &w.bridges {
            if !p.streets.iter().any(|s| &s.id == b) {
                v.push(perr(
                    "P2",
                    &w.id,
                    b,
                    format!(
                        "water `{}` lists bridge `{b}` but there is no street with that id",
                        w.id
                    ),
                ));
            }
        }
    }
    for l in &p.landmarks {
        check_rect(&mut v, &l.id, l.rect);
    }
    for s in &p.streets {
        check_street(&mut v, s);
    }
    for l in &p.lots.explicit {
        check_rect(&mut v, &l.id, l.rect);
        if let Some(z) = &l.zone {
            if !p.zones.iter().any(|x| &x.id == z) {
                v.push(perr(
                    "P5",
                    &l.id,
                    z,
                    format!("lot `{}` names zone `{z}`, which does not exist", l.id),
                ));
            }
        }
    }
    for b in &p.buildings {
        check_lot_ref(&mut v, p, &b.id, &b.lot);
        if b.progress > 100 {
            v.push(perr(
                "P8",
                &b.id,
                &format!("progress {}", b.progress),
                format!("building `{}` progress must be 0..=100", b.id),
            ));
        }
    }
    for f in &p.fill {
        if !p.zones.iter().any(|z| z.id == f.zone) {
            v.push(perr(
                "P5",
                &f.zone,
                "fill",
                format!("fill rule names zone `{}`, which does not exist", f.zone),
            ));
        }
    }
    for pr in &p.props {
        if let Some(sc) = &pr.scatter {
            if !p.zones.iter().any(|z| z.id == sc.zone) {
                v.push(perr(
                    "P5",
                    &sc.zone,
                    "props",
                    format!(
                        "props `{}` scatter in zone `{}`, which does not exist",
                        pr.kind, sc.zone
                    ),
                ));
            }
            if sc.spacing < 1 {
                v.push(perr(
                    "P8",
                    &pr.kind,
                    &format!("spacing {}", sc.spacing),
                    "scatter spacing must be at least 1 tile".into(),
                ));
            }
        }
        for c in &pr.cells {
            if !in_region(*c) {
                v.push(perr(
                    "P3",
                    &pr.kind,
                    &format!("{:?}", c),
                    format!(
                        "prop cell ({},{}) is outside the region (0..63)",
                        c[0], c[1]
                    ),
                ));
            }
        }
    }
    let r = &p.lots;
    if r.min_front < 1 || r.min_depth < 1 || r.max_area < 1 || r.min_buildable < 0 {
        v.push(perr(
            "P8",
            "lots",
            "rules",
            "lot rules need min_front, min_depth, max_area >= 1 and min_buildable >= 0".into(),
        ));
    }
    v
}

fn perr(code: &str, id: &str, measure: &str, message: String) -> Violation {
    Violation {
        code: code.into(),
        id: id.into(),
        measure: measure.into(),
        message,
        severity: Severity::Error,
    }
}

fn in_region(c: [i32; 2]) -> bool {
    c[0] >= 0 && c[1] >= 0 && c[0] < REGION_TILES && c[1] < REGION_TILES
}

fn check_rect(v: &mut Vec<Violation>, id: &str, r: [i32; 4]) {
    if r[2] < 1
        || r[3] < 1
        || r[0] < 0
        || r[1] < 0
        || r[0] + r[2] > REGION_TILES
        || r[1] + r[3] > REGION_TILES
    {
        v.push(perr("P3", id, &format!("rect {:?}", r), format!("`{id}` rect [x, y, w, h] = {:?} must have w, h >= 1 and stay inside the 64x64 region", r)));
    }
}

fn check_street(v: &mut Vec<Violation>, s: &Street) {
    if s.path.is_empty() {
        v.push(perr(
            "P4",
            &s.id,
            "path []",
            format!(
                "street `{}` has an empty path; give at least one [x, y] point",
                s.id
            ),
        ));
    }
    if s.width < 1 || s.width > 8 {
        v.push(perr(
            "P4",
            &s.id,
            &format!("width {}", s.width),
            format!("street `{}` width must be 1..=8 tiles", s.id),
        ));
    }
    for c in &s.path {
        if !in_region(*c) {
            v.push(perr(
                "P3",
                &s.id,
                &format!("{:?}", c),
                format!(
                    "street `{}` point ({},{}) is outside the region (0..63)",
                    s.id, c[0], c[1]
                ),
            ));
        }
    }
}

fn check_lot_ref(v: &mut Vec<Violation>, p: &CityProgram, id: &str, lot: &str) {
    if let Some(rest) = lot.strip_prefix("auto:") {
        if let Some(z) = rest.strip_prefix("zone:") {
            if !p.zones.iter().any(|x| x.id == z) {
                v.push(perr(
                    "P5",
                    id,
                    lot,
                    format!("building `{id}` asks for `{lot}` but zone `{z}` does not exist"),
                ));
            }
        } else if let Some(l) = rest.strip_prefix("near:") {
            if !p.landmarks.iter().any(|x| x.id == l) {
                v.push(perr(
                    "P6",
                    id,
                    lot,
                    format!("building `{id}` asks for `{lot}` but landmark `{l}` does not exist"),
                ));
            }
        } else {
            v.push(perr("P8", id, lot, format!("building `{id}` lot `{lot}` must be a lot id, `auto:near:<landmark>` or `auto:zone:<zone>`")));
        }
    } else if lot.is_empty() {
        v.push(perr(
            "P8",
            id,
            "lot \"\"",
            format!("building `{id}` has no lot"),
        ));
    }
}

fn all_ids(p: &CityProgram) -> Vec<String> {
    let mut ids = Vec::new();
    ids.extend(p.zones.iter().map(|x| x.id.clone()));
    ids.extend(p.streets.iter().map(|x| x.id.clone()));
    ids.extend(p.water.iter().map(|x| x.id.clone()));
    ids.extend(p.landmarks.iter().map(|x| x.id.clone()));
    ids.extend(p.buildings.iter().map(|x| x.id.clone()));
    ids
}

fn has_id(p: &CityProgram, id: &str) -> bool {
    all_ids(p).iter().any(|x| x == id)
}

/// The ownership reference that guards a building: its own, or its explicit
/// lot's.
fn owner_of(p: &CityProgram, b: &BuildingSpec) -> Option<String> {
    b.owner_ref.clone().or_else(|| {
        p.lots
            .explicit
            .iter()
            .find(|l| l.id == b.lot)
            .and_then(|l| l.owner_ref.clone())
    })
}

fn guard_owner(p: &CityProgram, id: &str, what: &str, admin: bool) -> Result<(), Violation> {
    if admin {
        return Ok(());
    }
    if let Some(b) = p.buildings.iter().find(|b| b.id == id) {
        if let Some(o) = owner_of(p, b) {
            return Err(perr("P7", id, &o, format!("building `{id}` sits on an owned lot ({o}); {what} needs the owner (admin flag)")));
        }
    }
    Ok(())
}

fn apply_one(p: &mut CityProgram, op: &PatchOp, admin: bool) -> Result<(), Violation> {
    let dup = |p: &CityProgram, id: &str| -> Result<(), Violation> {
        if has_id(p, id) {
            Err(perr(
                "P1",
                id,
                "duplicate",
                format!("id `{id}` already exists; pick a new id or use set_intent / move"),
            ))
        } else {
            Ok(())
        }
    };
    let single = |mut v: Vec<Violation>| -> Result<(), Violation> {
        if v.is_empty() {
            Ok(())
        } else {
            Err(v.remove(0))
        }
    };
    match op {
        PatchOp::AddZone(z) => {
            dup(p, &z.id)?;
            let mut v = Vec::new();
            check_rect(&mut v, &z.id, z.rect);
            single(v)?;
            p.zones.push(z.clone());
        }
        PatchOp::AddStreet(s) => {
            dup(p, &s.id)?;
            let mut v = Vec::new();
            check_street(&mut v, s);
            single(v)?;
            p.streets.push(s.clone());
        }
        PatchOp::AddLandmark(l) => {
            dup(p, &l.id)?;
            let mut v = Vec::new();
            check_rect(&mut v, &l.id, l.rect);
            single(v)?;
            p.landmarks.push(l.clone());
        }
        PatchOp::AddWater(w) => {
            dup(p, &w.id)?;
            let mut v = Vec::new();
            check_rect(&mut v, &w.id, w.rect);
            single(v)?;
            p.water.push(w.clone());
        }
        PatchOp::AddBuilding(b) => {
            dup(p, &b.id)?;
            if b.owner_ref.is_some() && !admin {
                return Err(perr("P7", &b.id, "owner_ref", format!("building `{}` carries an owner_ref; only the owner (admin flag) can assign one", b.id)));
            }
            if let Some(l) = p.lots.explicit.iter().find(|l| l.id == b.lot) {
                if let (Some(o), false) = (&l.owner_ref, admin) {
                    return Err(perr(
                        "P7",
                        &b.id,
                        o,
                        format!(
                            "lot `{}` is owned ({o}); building on it needs the owner (admin flag)",
                            l.id
                        ),
                    ));
                }
            }
            let mut v = Vec::new();
            check_lot_ref(&mut v, p, &b.id, &b.lot);
            single(v)?;
            p.buildings.push(b.clone());
        }
        PatchOp::AddProps(pr) => {
            if let Some(sc) = &pr.scatter {
                if !p.zones.iter().any(|z| z.id == sc.zone) {
                    return Err(perr(
                        "P5",
                        &sc.zone,
                        "props",
                        format!("zone `{}` does not exist", sc.zone),
                    ));
                }
            }
            p.props.push(pr.clone());
        }
        PatchOp::AddFill(f) => {
            if !p.zones.iter().any(|z| z.id == f.zone) {
                return Err(perr(
                    "P5",
                    &f.zone,
                    "fill",
                    format!("zone `{}` does not exist", f.zone),
                ));
            }
            p.fill.push(f.clone());
        }
        PatchOp::SetIntent { id, patch } => {
            guard_owner(p, id, "changing it", admin)?;
            let Some(b) = p.buildings.iter_mut().find(|b| &b.id == id) else {
                return Err(unknown(id, "building"));
            };
            b.intent = merge_intent(&b.intent, patch).map_err(|m| perr("P8", id, "patch", m))?;
        }
        PatchOp::Move { id, lot, path } => {
            guard_owner(p, id, "moving it", admin)?;
            if let Some(b) = p.buildings.iter_mut().find(|b| &b.id == id) {
                let Some(l) = lot else {
                    return Err(perr(
                        "P8",
                        id,
                        "move",
                        format!("move of building `{id}` needs `lot`"),
                    ));
                };
                b.lot = l.clone();
                let mut v = Vec::new();
                check_lot_ref(&mut v, p, id, l);
                single(v)?;
            } else if let Some(s) = p.streets.iter_mut().find(|s| &s.id == id) {
                let Some(np) = path else {
                    return Err(perr(
                        "P8",
                        id,
                        "move",
                        format!("move of street `{id}` needs `path`"),
                    ));
                };
                s.path = np.clone();
                let s = s.clone();
                let mut v = Vec::new();
                check_street(&mut v, &s);
                single(v)?;
            } else {
                return Err(unknown(id, "building or street"));
            }
        }
        PatchOp::SetProgress { id, progress } => {
            if *progress > 100 {
                return Err(perr(
                    "P8",
                    id,
                    &format!("progress {progress}"),
                    "progress must be 0..=100".into(),
                ));
            }
            let Some(b) = p.buildings.iter_mut().find(|b| &b.id == id) else {
                return Err(unknown(id, "building"));
            };
            b.progress = *progress;
        }
        PatchOp::Remove { id } => {
            guard_owner(p, id, "removing it", admin)?;
            let before = all_ids(p).len();
            p.zones.retain(|x| &x.id != id);
            p.streets.retain(|x| &x.id != id);
            p.water.retain(|x| &x.id != id);
            p.landmarks.retain(|x| &x.id != id);
            p.buildings.retain(|x| &x.id != id);
            if all_ids(p).len() == before {
                return Err(unknown(id, "element"));
            }
            for w in &mut p.water {
                w.bridges.retain(|b| b != id);
            }
        }
    }
    Ok(())
}

fn unknown(id: &str, what: &str) -> Violation {
    perr(
        "P2",
        id,
        "not found",
        format!("there is no {what} with id `{id}`"),
    )
}

/// JSON merge of `patch` into an intent: keys replace fields, `null` resets a
/// field to its default. Unknown keys are refused so typos do not vanish.
pub fn merge_intent(base: &Intent, patch: &serde_json::Value) -> Result<Intent, String> {
    let serde_json::Value::Object(patch) = patch else {
        return Err("set_intent patch must be a JSON object".into());
    };
    let mut cur = serde_json::to_value(base).map_err(|e| e.to_string())?;
    let obj = cur.as_object_mut().expect("intent is an object");
    for (k, val) in patch {
        if !obj.contains_key(k) {
            let keys: Vec<&String> = obj.keys().collect();
            return Err(format!(
                "unknown intent field `{k}`; valid fields: {keys:?}"
            ));
        }
        if val.is_null() {
            obj.remove(k);
        } else {
            obj.insert(k.clone(), val.clone());
        }
    }
    serde_json::from_value(cur).map_err(|e| format!("intent patch does not fit: {e}"))
}

fn op_name(op: &PatchOp) -> &'static str {
    match op {
        PatchOp::AddZone(_) => "add_zone",
        PatchOp::AddStreet(_) => "add_street",
        PatchOp::AddLandmark(_) => "add_landmark",
        PatchOp::AddWater(_) => "add_water",
        PatchOp::AddBuilding(_) => "add_building",
        PatchOp::AddProps(_) => "add_props",
        PatchOp::AddFill(_) => "add_fill",
        PatchOp::SetIntent { .. } => "set_intent",
        PatchOp::Move { .. } => "move",
        PatchOp::SetProgress { .. } => "set_progress",
        PatchOp::Remove { .. } => "remove",
    }
}
