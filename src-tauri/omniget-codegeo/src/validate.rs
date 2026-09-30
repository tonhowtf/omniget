//! Semantic and budget checks on a finished script: a house needs a way in,
//! a bench a place to sit, and every class has a triangle ceiling. Also turns
//! float anchors into the city's grid anchors (tile, level, yaw).

use crate::canon::{CanonMesh, Q};
use crate::{AnchorPoint, Footprint, ScriptError};
use omniget_citygen::types::Anchor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassKind {
    Prop,
    Building,
    Module,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    pub kind: ClassKind,
    /// `prop:bench`, `building:house`, `module:telhado`…
    pub sub: Option<String>,
}

impl Class {
    pub fn parse(s: &str) -> Result<Class, ScriptError> {
        let s = s.trim().to_ascii_lowercase();
        let (k, sub) = match s.split_once(':') {
            Some((k, sub)) => (
                k.to_string(),
                Some(sub.to_string()).filter(|x| !x.is_empty()),
            ),
            None => (s.clone(), None),
        };
        let kind = match k.as_str() {
            "prop" | "objeto" => ClassKind::Prop,
            "building" | "predio" | "prédio" | "typology" => ClassKind::Building,
            "module" | "modulo" | "módulo" => ClassKind::Module,
            _ => {
                return Err(ScriptError::plain(
                    "class",
                    format!("unknown class '{s}': use prop, building or module (optionally prop:bench, building:house, module:telhado)"),
                ))
            }
        };
        Ok(Class { kind, sub })
    }

    pub fn max_tris(&self) -> usize {
        match self.kind {
            ClassKind::Prop => 1_500,
            ClassKind::Building => 5_000,
            ClassKind::Module => 400,
        }
    }
}

/// Soft triangle budget per kit layer (VISUAL.md), `(min, max)`. Only the
/// ceiling is checked; the floor is the authoring guideline.
pub fn layer_budget(layer: &str) -> Option<(usize, usize)> {
    Some(match layer {
        "terreo" => (8, 80),
        "parede" => (8, 64),
        "telhado" => (24, 56),
        "porta" => (8, 280),
        "bay" => (8, 320),
        "junta" => (8, 64),
        "torre" => (8, 96),
        "enfeite" => (8, 200),
        "arvore" => (120, 600),
        "prop" => (12, 900),
        "obra" => (24, 400),
        _ => return None,
    })
}

/// Which anchor a kind of thing must expose.
fn required_action(kind: &str) -> Option<(&'static str, &'static str)> {
    Some(match kind {
        "bench" | "banco" | "chair" | "cadeira" | "seat" | "sofa" => ("sit", "seat(#{...})"),
        "bed" | "cama" => ("sleep", "bed(#{...})"),
        "planter" | "canteiro" | "garden" | "horta" => ("plant", "planter(#{...})"),
        "counter" | "balcao" | "balcão" | "stall" | "banca" | "kiosk" | "quiosque" => {
            ("work", "counter(#{...})")
        }
        "workbench" | "bancada" => ("work", "bench_top(#{...})"),
        "well" | "poco" | "poço" => ("water", "anchor(\"water\", #{...})"),
        _ => return None,
    })
}

pub fn footprint(mesh: &CanonMesh) -> Footprint {
    let (lo, _) = mesh.bounds();
    let ground = lo[1] + (0.5 * Q) as i32;
    let mut min = [i32::MAX; 2];
    let mut max = [i32::MIN; 2];
    for t in &mesh.tris {
        for v in &t.v {
            if v[1] <= ground {
                min[0] = min[0].min(v[0]);
                min[1] = min[1].min(v[2]);
                max[0] = max[0].max(v[0]);
                max[1] = max[1].max(v[2]);
            }
        }
    }
    if min[0] == i32::MAX {
        return Footprint {
            w: 1,
            d: 1,
            origin_mm: [-1000, -1000],
        };
    }
    let tiles = |a: i32, b: i32| -> i32 {
        let ext = (b - a) as f64 / Q;
        (((ext - 0.1) / 2.0).ceil() as i32).max(1)
    };
    let (w, d) = (tiles(min[0], max[0]), tiles(min[1], max[1]));
    let cx = (min[0] + max[0]) as f64 / 2.0 / Q;
    let cz = (min[1] + max[1]) as f64 / 2.0 / Q;
    Footprint {
        w,
        d,
        origin_mm: [
            ((cx - w as f64) * 1000.0).round() as i32,
            ((cz - d as f64) * 1000.0).round() as i32,
        ],
    }
}

pub fn grid_anchor(a: &AnchorPoint, fp: &Footprint) -> Anchor {
    let x = a.pos[0] * 1000.0 - fp.origin_mm[0] as f64;
    let z = a.pos[2] * 1000.0 - fp.origin_mm[1] as f64;
    Anchor {
        action: a.action.clone(),
        cell: [(x / 2000.0).floor() as i32, (z / 2000.0).floor() as i32],
        level: ((a.pos[1] + 0.25) / 3.0).floor().clamp(0.0, 255.0) as u8,
        yaw: ((a.yaw_deg / 360.0 * 256.0).round() as i64).rem_euclid(256) as u8,
        capacity: a.capacity,
    }
}

pub struct Verdict {
    pub errors: Vec<ScriptError>,
    pub warnings: Vec<String>,
}

pub fn check(
    class: &Class,
    kind: Option<&str>,
    mesh: &CanonMesh,
    anchors: &[AnchorPoint],
    fp: &Footprint,
) -> Verdict {
    let mut errors = vec![];
    let mut warnings = vec![];
    let mut fail = |kind: &str, msg: String| errors.push(ScriptError::plain(kind, msg));
    let tris = mesh.len();
    let (lo, hi) = mesh.bounds();
    let m = |v: i32| v as f64 / Q;
    let size = [m(hi[0] - lo[0]), m(hi[1] - lo[1]), m(hi[2] - lo[2])];

    if tris == 0 && class.kind != ClassKind::Module {
        fail("validation", "the result has no surface".into());
    }
    if tris > class.max_tris() {
        fail(
            "limit",
            format!(
                "{tris} triangles is over the limit of {} for this class",
                class.max_tris()
            ),
        );
    }
    for a in anchors {
        let out = (0..3).any(|k| a.pos[k] < m(lo[k]) - 1.5 || a.pos[k] > m(hi[k]) + 1.5);
        if out && tris > 0 {
            fail(
                "anchor",
                format!(
                    "anchor '{}' at [{:.2}, {:.2}, {:.2}] is far from the object; keep anchors within 1.5 m of it",
                    a.action, a.pos[0], a.pos[1], a.pos[2]
                ),
            );
        }
    }

    match class.kind {
        ClassKind::Building => {
            let doors: Vec<&AnchorPoint> = anchors.iter().filter(|a| a.action == "enter").collect();
            if doors.is_empty() {
                fail(
                    "semantic",
                    "a building needs an entrance: cut one with door(body, #{ at: [x, 0, z], face: \"s\" })".into(),
                );
            }
            for d in &doors {
                if d.pos[1] > 1.0 {
                    fail(
                        "semantic",
                        format!(
                            "the door at y = {:.2} m must be at ground level (y = 0)",
                            d.pos[1]
                        ),
                    );
                }
                let (x0, z0) = (
                    fp.origin_mm[0] as f64 / 1000.0,
                    fp.origin_mm[1] as f64 / 1000.0,
                );
                let (x1, z1) = (x0 + fp.w as f64 * 2.0, z0 + fp.d as f64 * 2.0);
                let dx = (x0 - d.pos[0]).max(d.pos[0] - x1).max(0.0);
                let dz = (z0 - d.pos[2]).max(d.pos[2] - z1).max(0.0);
                let edge = (d.pos[0] - x0)
                    .min(x1 - d.pos[0])
                    .min(d.pos[2] - z0)
                    .min(z1 - d.pos[2]);
                if dx.max(dz) > 1.5 || edge > 0.5 {
                    fail(
                        "semantic",
                        "the door must open on an outer wall of the building (put it on the facade plane)".into(),
                    );
                }
            }
            if fp.w > 16 || fp.d > 16 {
                fail(
                    "size",
                    format!("footprint {}×{} tiles is over 16×16 (32 m)", fp.w, fp.d),
                );
            }
            if size[1] > 45.0 {
                fail("size", format!("height {:.1} m is over 45 m", size[1]));
            }
        }
        ClassKind::Prop => {
            if fp.w > 5 || fp.d > 5 {
                fail(
                    "size",
                    format!(
                        "a prop fits in 5×5 tiles (10 m); this one is {}×{}",
                        fp.w, fp.d
                    ),
                );
            }
            if size[1] > 15.0 {
                fail(
                    "size",
                    format!("a prop is at most 15 m tall; this one is {:.1} m", size[1]),
                );
            }
        }
        ClassKind::Module => {
            let inside = m(lo[0]) >= -1.3 && m(hi[0]) <= 1.3 && m(lo[2]) >= -1.3 && m(hi[2]) <= 1.3;
            // Roof pieces hang their gable walls down to the eave line.
            let floor = if class.sub.as_deref() == Some("telhado") {
                -4.6
            } else {
                -0.6
            };
            let tall = m(lo[1]) >= floor && m(hi[1]) <= 3.6;
            if tris > 0 && !(inside && tall) {
                fail(
                    "size",
                    format!("a module lives in the 2×2×3 m cell: x and z within ±1.3 m, y from {floor} to 3.6 m"),
                );
            }
            if let Some((lo_b, hi_b)) = class.sub.as_deref().and_then(layer_budget) {
                if tris > hi_b {
                    warnings.push(format!(
                        "{tris} triangles is over the {lo_b}–{hi_b} budget of this layer"
                    ));
                }
            }
        }
    }

    let kind = kind.or(class.sub.as_deref());
    if let Some(k) = kind {
        if let Some((action, how)) = required_action(k) {
            if !anchors.iter().any(|a| a.action == action) {
                fail(
                    "semantic",
                    format!("a {k} needs a '{action}' anchor: add {how}"),
                );
            }
        }
    }
    Verdict { errors, warnings }
}
