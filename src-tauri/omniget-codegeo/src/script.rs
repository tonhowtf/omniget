//! The Rhai sandbox: `Engine::new_raw()` + `StandardPackage` (no I/O, no
//! print, no time), hard limits, and the kit functions. Styles are enforced
//! here: sizes snap to the 5 cm grid, materials must be palette slots and
//! every primitive gets a chamfer unless it asks for none.

use crate::geom::{self, snap, v3, Edges};
use crate::palette::slot_index;
use crate::{AnchorPoint, ScriptError, ACTIONS};
use manifold_rust::manifold::Manifold;
use rhai::packages::{Package, StandardPackage};
use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, Position, Scope, AST, INT};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_operations: u64,
    pub max_call_levels: usize,
    pub max_array: usize,
    pub max_string: usize,
    pub max_prims: usize,
    pub max_tris: usize,
    pub time: Duration,
}

impl Limits {
    pub fn user(max_tris: usize) -> Self {
        Limits {
            max_operations: 200_000,
            max_call_levels: 16,
            max_array: 2_000,
            max_string: 256,
            max_prims: 400,
            max_tris,
            time: Duration::from_millis(250),
        }
    }

    pub fn kit() -> Self {
        Limits {
            max_operations: 400_000,
            max_call_levels: 32,
            max_array: 2_000,
            max_string: 256,
            max_prims: 400,
            max_tris: 4_000,
            time: Duration::from_secs(5),
        }
    }
}

/// A solid of the kit. Materials travel with Manifold's original ids.
#[derive(Clone)]
pub struct Solid(pub Manifold);

pub struct Ctx {
    pub slot_of: HashMap<u32, u8>,
    pub anchors: Vec<AnchorPoint>,
    pub prims: usize,
    pub kind: Option<String>,
    pub seed: u64,
    /// Level of detail asked by the kit builder (0 = full, 1 = far).
    pub lod: u8,
    pub deadline: Instant,
    pub limits: Limits,
}

type Res<T> = Result<T, Box<EvalAltResult>>;
type Shared = Rc<RefCell<Ctx>>;

fn err<T>(msg: impl Into<String>) -> Res<T> {
    Err(EvalAltResult::ErrorRuntime(Dynamic::from(msg.into()), Position::NONE).into())
}

fn num(v: &Dynamic, what: &str) -> Res<f64> {
    if let Ok(f) = v.as_float() {
        return Ok(f);
    }
    if let Ok(i) = v.as_int() {
        return Ok(i as f64);
    }
    err(format!("{what} must be a number, got {}", v.type_name()))
}

fn vec3(v: &Dynamic, what: &str) -> Res<[f64; 3]> {
    let Some(a) = v.read_lock::<Array>() else {
        return err(format!("{what} must be an array [x, y, z]"));
    };
    if a.len() != 3 {
        return err(format!(
            "{what} must have 3 numbers [x, y, z], got {}",
            a.len()
        ));
    }
    Ok([num(&a[0], what)?, num(&a[1], what)?, num(&a[2], what)?])
}

fn snap3(p: [f64; 3]) -> [f64; 3] {
    [snap(p[0]), snap(p[1]), snap(p[2])]
}

/// Checked view over an options map: unknown keys are an error so a typo
/// never silently falls back to a default.
struct Opts<'a> {
    f: &'static str,
    m: &'a Map,
}

impl<'a> Opts<'a> {
    fn new(f: &'static str, m: &'a Map, allowed: &[&str]) -> Res<Self> {
        for k in m.keys() {
            if !allowed.contains(&k.as_str()) {
                return err(format!(
                    "{f}: unknown option '{k}' (allowed: {})",
                    allowed.join(", ")
                ));
            }
        }
        Ok(Opts { f, m })
    }
    fn has(&self, k: &str) -> bool {
        self.m.contains_key(k)
    }
    fn num(&self, k: &str, d: f64) -> Res<f64> {
        match self.m.get(k) {
            Some(v) => num(v, &format!("{}.{k}", self.f)),
            None => Ok(d),
        }
    }
    fn req(&self, k: &str) -> Res<f64> {
        match self.m.get(k) {
            Some(v) => num(v, &format!("{}.{k}", self.f)),
            None => err(format!("{}: missing option '{k}'", self.f)),
        }
    }
    /// Size-like value: snapped to the grid and range-checked.
    fn size(&self, k: &str, d: f64) -> Res<f64> {
        let v = snap(self.num(k, d)?);
        size_ok(self.f, k, v)
    }
    fn req_size(&self, k: &str) -> Res<f64> {
        let v = snap(self.req(k)?);
        size_ok(self.f, k, v)
    }
    fn at(&self) -> Res<[f64; 3]> {
        match self.m.get("at") {
            Some(v) => Ok(snap3(vec3(v, &format!("{}.at", self.f))?)),
            None => Ok([0.0; 3]),
        }
    }
    fn string(&self, k: &str) -> Option<String> {
        self.m.get(k).map(|v| v.to_string())
    }
    fn boolean(&self, k: &str, d: bool) -> Res<bool> {
        match self.m.get(k) {
            Some(v) => v
                .as_bool()
                .or_else(|_| err(format!("{}.{k} must be true or false", self.f))),
            None => Ok(d),
        }
    }
    fn mat(&self, k: &str, d: &str) -> Res<String> {
        let m = self.string(k).unwrap_or_else(|| d.to_string());
        check_mat(&m)?;
        Ok(m)
    }
    fn rot(&self) -> Res<f64> {
        Ok(self.num("rot", 0.0)?.round())
    }
    fn face(&self) -> Res<f64> {
        match self.m.get("face") {
            None => Ok(0.0),
            Some(v) => face_deg(v, self.f),
        }
    }
    fn seg(&self, d: i32, lo: i32, hi: i32) -> Res<i32> {
        let s = self.num("seg", d as f64)?.round() as i32;
        if s < lo || s > hi {
            return err(format!("{}.seg must be between {lo} and {hi}", self.f));
        }
        Ok(s)
    }
}

fn size_ok(f: &str, k: &str, v: f64) -> Res<f64> {
    let min = if geom::snap_on() { 0.05 } else { 0.005 };
    if !(min..=40.0).contains(&v) {
        return err(format!(
            "{f}.{k} = {v} is out of range: sizes go from 0.05 to 40 m (5 cm grid)"
        ));
    }
    Ok(v)
}

fn check_mat(m: &str) -> Res<()> {
    if slot_index(m).is_none() {
        return err(format!(
            "material '{m}' is not a palette slot; use one of: {}",
            omniget_citygen::types::PALETTE_SLOTS.join(", ")
        ));
    }
    Ok(())
}

fn face_deg(v: &Dynamic, f: &str) -> Res<f64> {
    if let Some(s) = v.read_lock::<rhai::ImmutableString>() {
        return Ok(match s.as_str() {
            "n" | "north" => 0.0,
            "e" | "east" => 90.0,
            "s" | "south" => 180.0,
            "w" | "west" => 270.0,
            other => {
                return err(format!(
                    "{f}.face '{other}' must be \"n\", \"e\", \"s\", \"w\" or degrees"
                ))
            }
        });
    }
    Ok(num(v, &format!("{f}.face"))?.round().rem_euclid(360.0))
}

/// Default chamfer: 5 cm on big parts, thinner on small ones, none on tiny.
fn auto_bevel(min_side: f64) -> f64 {
    if min_side >= 0.3 {
        0.05
    } else if min_side >= 0.1 {
        0.02
    } else {
        0.0
    }
}

fn clamp_bevel(b: f64, min_side: f64) -> f64 {
    b.max(0.0).min(min_side / 3.0)
}

fn place(m: Manifold, at: [f64; 3], rot: f64) -> Manifold {
    geom::turn(&m, rot).translate(v3(at[0], at[1], at[2]))
}

fn solids(parts: &Array, f: &str) -> Res<Vec<Manifold>> {
    let mut out = Vec::with_capacity(parts.len());
    for p in parts {
        if p.is_unit() {
            continue;
        }
        match p.clone().try_cast::<Solid>() {
            Some(s) => out.push(s.0),
            None => {
                if let Some(inner) = p.read_lock::<Array>() {
                    out.extend(solids(&inner, f)?);
                } else {
                    return err(format!("{f}: expected solids, got {}", p.type_name()));
                }
            }
        }
    }
    Ok(out)
}

fn checked(m: Manifold, f: &str) -> Res<Solid> {
    if !geom::ok(&m) {
        return err(format!(
            "{f}: geometry failed ({:?}); check sizes and overlaps",
            m.status()
        ));
    }
    Ok(Solid(m))
}

/// Everything the registered closures need from the run.
#[derive(Clone)]
struct K(Shared);

impl K {
    fn tick(&self) -> Res<()> {
        let c = self.0.borrow();
        if Instant::now() > c.deadline {
            return err(format!(
                "time limit of {} ms exceeded",
                c.limits.time.as_millis()
            ));
        }
        Ok(())
    }
    /// Registers a fresh primitive with its palette slot.
    fn tag(&self, m: Manifold, mat: &str, f: &str) -> Res<Solid> {
        self.tick()?;
        if !geom::ok(&m) || m.is_empty() {
            return err(format!("{f}: could not build this shape; check its sizes"));
        }
        let slot = slot_index(mat).expect("checked");
        let mut c = self.0.borrow_mut();
        c.prims += 1;
        if c.prims > c.limits.max_prims {
            return err(format!(
                "primitive limit ({}) exceeded: use repeat() with fewer copies or merge parts",
                c.limits.max_prims
            ));
        }
        let o = m.as_original();
        c.slot_of.insert(o.original_id() as u32, slot);
        Ok(Solid(o))
    }
    fn anchor(&self, action: &str, pos: [f64; 3], yaw: f64, capacity: u8) {
        self.0.borrow_mut().anchors.push(AnchorPoint {
            action: action.to_string(),
            pos,
            yaw_deg: yaw.rem_euclid(360.0),
            capacity,
        });
    }
}

fn local_to_world(p: [f64; 3], at: [f64; 3], yaw: f64) -> [f64; 3] {
    let r = geom::turn_point(p, yaw);
    [r[0] + at[0], r[1] + at[1], r[2] + at[2]]
}

pub fn engine(k: Shared) -> Engine {
    let lim = k.borrow().limits.clone();
    let mut e = Engine::new_raw();
    e.register_global_module(StandardPackage::new().as_shared_module());
    e.set_max_operations(lim.max_operations)
        .set_max_call_levels(lim.max_call_levels)
        .set_max_array_size(lim.max_array)
        .set_max_map_size(64)
        .set_max_string_size(lim.max_string)
        .set_max_expr_depths(64, 32);
    {
        let k = k.clone();
        e.on_progress(move |ops| {
            if ops % 128 == 0 && Instant::now() > k.borrow().deadline {
                Some(Dynamic::from("time limit exceeded"))
            } else {
                None
            }
        });
    }
    e.register_type_with_name::<Solid>("Solid");
    let k = K(k);

    // ── primitives ──
    let kk = k.clone();
    let box_fn = move |size: Array, o: Map| -> Res<Solid> {
        let op = Opts::new("box", &o, &["at", "mat", "bevel", "edges", "rot"])?;
        if size.len() != 3 {
            return err("box: size must be [w, h, d]");
        }
        let mut s = [0.0; 3];
        for (i, n) in ["w", "h", "d"].iter().enumerate() {
            s[i] = size_ok("box", n, snap(num(&size[i], "box size")?))?;
        }
        let min = s[0].min(s[1]).min(s[2]);
        let b = clamp_bevel(op.num("bevel", auto_bevel(min))?, min);
        let edges = match op.string("edges") {
            Some(x) => match Edges::parse(&x) {
                Some(e) => e,
                None => return err("box.edges must be \"all\", \"x\", \"y\", \"z\" or \"none\""),
            },
            None => Edges::All,
        };
        let m = geom::bevel_box(s[0], s[1], s[2], b, edges).translate(v3(0.0, s[1] / 2.0, 0.0));
        let mat = op.mat("mat", "wall")?;
        kk.tag(place(m, op.at()?, op.rot()?), &mat, "box")
    };
    let b2 = box_fn.clone();
    e.register_fn("box", box_fn);
    e.register_fn("box", move |size: Array| b2(size, Map::new()));

    let kk = k.clone();
    e.register_fn("cyl", move |o: Map| -> Res<Solid> {
        let op = Opts::new("cyl", &o, &["r", "r2", "h", "at", "mat", "seg", "bevel"])?;
        let r = op.req_size("r")?;
        let r2 = if op.has("r2") {
            snap(op.req("r2")?).max(0.0)
        } else {
            r
        };
        let h = op.req_size("h")?;
        let seg = op.seg(12, 3, 48)?;
        let min = (2.0 * r.min(if r2 > 0.0 { r2 } else { r })).min(h);
        let b = clamp_bevel(op.num("bevel", auto_bevel(min))?, min);
        let m = geom::cylinder(r, r2, h, seg, b);
        let mat = op.mat("mat", "wall")?;
        kk.tag(place(m, op.at()?, 0.0), &mat, "cyl")
    });

    let kk = k.clone();
    e.register_fn("cone", move |o: Map| -> Res<Solid> {
        let op = Opts::new("cone", &o, &["r", "h", "at", "mat", "seg"])?;
        let (r, h) = (op.req_size("r")?, op.req_size("h")?);
        let m = geom::cylinder(r, 0.0, h, op.seg(12, 3, 48)?, 0.0);
        let mat = op.mat("mat", "roof")?;
        kk.tag(place(m, op.at()?, 0.0), &mat, "cone")
    });

    let kk = k.clone();
    e.register_fn("wedge", move |size: Array, o: Map| -> Res<Solid> {
        let op = Opts::new("wedge", &o, &["at", "mat", "rot"])?;
        if size.len() != 3 {
            return err("wedge: size must be [w, h, d]");
        }
        let w = size_ok("wedge", "w", snap(num(&size[0], "wedge size")?))?;
        let h = size_ok("wedge", "h", snap(num(&size[1], "wedge size")?))?;
        let d = size_ok("wedge", "d", snap(num(&size[2], "wedge size")?))?;
        let mat = op.mat("mat", "wall")?;
        kk.tag(
            place(geom::wedge(w, h, d), op.at()?, op.rot()?),
            &mat,
            "wedge",
        )
    });

    let kk = k.clone();
    e.register_fn("gable", move |o: Map| -> Res<Solid> {
        let op = Opts::new(
            "gable",
            &o,
            &["w", "d", "h", "eave", "thick", "at", "mat", "wall", "rot"],
        )?;
        let (w, d, h) = (op.req_size("w")?, op.req_size("d")?, op.req_size("h")?);
        let eave = snap(op.num("eave", 0.3)?).clamp(0.0, 2.0);
        let t = op.size("thick", 0.15)?;
        let Some((roof, fill)) = geom::gable(w, d, h, eave, t) else {
            return err("gable: could not build this roof; check w, d and h");
        };
        let (at, rot) = (op.at()?, op.rot()?);
        let r = kk.tag(place(roof, at, rot), &op.mat("mat", "roof")?, "gable")?;
        let f = kk.tag(place(fill, at, rot), &op.mat("wall", "wall")?, "gable")?;
        checked(geom::union(&[r.0, f.0]), "gable")
    });

    let kk = k.clone();
    e.register_fn("hip", move |o: Map| -> Res<Solid> {
        let op = Opts::new(
            "hip",
            &o,
            &["w", "d", "h", "eave", "thick", "at", "mat", "rot"],
        )?;
        let (w, d, h) = (op.req_size("w")?, op.req_size("d")?, op.req_size("h")?);
        let eave = snap(op.num("eave", 0.3)?).clamp(0.0, 2.0);
        let t = op.size("thick", 0.15)?;
        let mat = op.mat("mat", "roof")?;
        kk.tag(
            place(geom::hip(w, d, h, eave, t), op.at()?, op.rot()?),
            &mat,
            "hip",
        )
    });

    let kk = k.clone();
    e.register_fn("cone_roof", move |o: Map| -> Res<Solid> {
        let op = Opts::new(
            "cone_roof",
            &o,
            &["r", "h", "eave", "thick", "at", "mat", "seg"],
        )?;
        let (r, h) = (op.req_size("r")?, op.req_size("h")?);
        let eave = snap(op.num("eave", 0.2)?).clamp(0.0, 2.0);
        let t = op.size("thick", 0.1)?;
        let mat = op.mat("mat", "roof")?;
        let m = geom::cone_roof(r, h, eave, t, op.seg(12, 3, 32)?);
        kk.tag(place(m, op.at()?, 0.0), &mat, "cone_roof")
    });

    let kk = k.clone();
    e.register_fn("arch", move |o: Map| -> Res<Solid> {
        let op = Opts::new("arch", &o, &["w", "h", "d", "at", "mat", "seg", "rot"])?;
        let (w, h, d) = (op.req_size("w")?, op.req_size("h")?, op.req_size("d")?);
        let mat = op.mat("mat", "wall")?;
        let m = geom::arch(w, h, d, op.seg(12, 4, 32)?);
        kk.tag(place(m, op.at()?, op.rot()?), &mat, "arch")
    });

    let kk = k.clone();
    e.register_fn("extrude", move |profile: Array, o: Map| -> Res<Solid> {
        let op = Opts::new("extrude", &o, &["h", "axis", "at", "mat", "rot"])?;
        if profile.len() < 3 || profile.len() > 64 {
            return err("extrude: the profile needs 3 to 64 points [[x, y], ...]");
        }
        let mut pts = Vec::with_capacity(profile.len());
        for p in &profile {
            let Some(a) = p.read_lock::<Array>() else {
                return err("extrude: each profile point must be [x, y]");
            };
            if a.len() != 2 {
                return err("extrude: each profile point must be [x, y]");
            }
            pts.push([
                snap(num(&a[0], "extrude point")?),
                snap(num(&a[1], "extrude point")?),
            ]);
        }
        let h = op.req_size("h")?;
        let axis = match op.string("axis").as_deref() {
            None | Some("y") => 'y',
            Some("x") => 'x',
            Some("z") => 'z',
            Some(o) => return err(format!("extrude.axis '{o}' must be \"x\", \"y\" or \"z\"")),
        };
        let Some(m) = geom::extrude(&pts, h, axis) else {
            return err("extrude: the profile has no area (points in a line or self-crossing)");
        };
        let mat = op.mat("mat", "wall")?;
        kk.tag(place(m, op.at()?, op.rot()?), &mat, "extrude")
    });

    let kk = k.clone();
    e.register_fn("blob", move |o: Map| -> Res<Solid> {
        let op = Opts::new("blob", &o, &["r", "at", "mat", "seg", "squash", "jitter"])?;
        let r = op.req_size("r")?;
        let squash = op.num("squash", 1.0)?.clamp(0.3, 2.0);
        let jitter = op.num("jitter", 0.0)?.clamp(0.0, 0.3);
        let seg = op.seg(8, 4, 24)?;
        let mut m = geom::blob(r, seg, squash);
        if jitter > 0.0 {
            let seed = kk.0.borrow().seed;
            let amp = jitter * r;
            m = m.warp(move |p| {
                let key = (
                    (p.x * 1e4).round() as i64,
                    (p.y * 1e4).round() as i64,
                    (p.z * 1e4).round() as i64,
                );
                let h = crate::rng::hash3(seed, key);
                let s = 1.0 + amp / r * (crate::rng::unit(h) * 2.0 - 1.0);
                p.x *= s;
                p.y *= s;
                p.z *= s;
            });
        }
        let mat = op.mat("mat", "leaf")?;
        kk.tag(place(m, op.at()?, 0.0), &mat, "blob")
    });

    // ── transforms and booleans ──
    e.register_fn("translate", |s: Solid, v: Array| -> Res<Solid> {
        let p = snap3(vec3(&Dynamic::from(v), "translate")?);
        Ok(Solid(s.0.translate(v3(p[0], p[1], p[2]))))
    });
    e.register_fn("turn", |s: Solid, deg: Dynamic| -> Res<Solid> {
        Ok(Solid(geom::turn(&s.0, face_deg(&deg, "turn")?)))
    });
    e.register_fn("mirror", |s: Solid, axis: &str| -> Res<Solid> {
        let n = match axis {
            "x" => v3(1.0, 0.0, 0.0),
            "y" => v3(0.0, 1.0, 0.0),
            "z" => v3(0.0, 0.0, 1.0),
            // Swaps x and z (reflection across the x = z diagonal).
            "xz" => v3(1.0, 0.0, -1.0),
            _ => return err("mirror: axis must be \"x\", \"y\", \"z\" or \"xz\""),
        };
        checked(s.0.mirror(n), "mirror")
    });
    let kk = k.clone();
    e.register_fn(
        "repeat",
        move |s: Solid, n: INT, step: Array| -> Res<Solid> {
            kk.tick()?;
            if !(1..=64).contains(&n) {
                return err("repeat: n must be between 1 and 64");
            }
            let d = snap3(vec3(&Dynamic::from(step), "repeat step")?);
            let parts: Vec<Manifold> = (0..n)
                .map(|i| {
                    s.0.translate(v3(d[0] * i as f64, d[1] * i as f64, d[2] * i as f64))
                })
                .collect();
            checked(geom::union(&parts), "repeat")
        },
    );
    let kk = k.clone();
    e.register_fn("union", move |parts: Array| -> Res<Solid> {
        kk.tick()?;
        checked(geom::union(&solids(&parts, "union")?), "union")
    });
    let kk = k.clone();
    e.register_fn("group", move |parts: Array| -> Res<Solid> {
        kk.tick()?;
        checked(Manifold::compose(&solids(&parts, "group")?), "group")
    });
    let kk = k.clone();
    e.register_fn("subtract", move |a: Solid, cuts: Array| -> Res<Solid> {
        kk.tick()?;
        checked(
            geom::subtract(&a.0, &solids(&cuts, "subtract")?),
            "subtract",
        )
    });
    let kk = k.clone();
    e.register_fn("subtract", move |a: Solid, b: Solid| -> Res<Solid> {
        kk.tick()?;
        checked(geom::subtract(&a.0, &[b.0]), "subtract")
    });
    let kk = k.clone();
    e.register_fn("intersect", move |a: Solid, b: Solid| -> Res<Solid> {
        kk.tick()?;
        checked(geom::intersect(&a.0, &b.0), "intersect")
    });
    let kk = k.clone();
    e.register_fn("hull", move |parts: Array, o: Map| -> Res<Solid> {
        let op = Opts::new("hull", &o, &["mat"])?;
        let ms = solids(&parts, "hull")?;
        let mat = op.mat("mat", "wall")?;
        kk.tag(Manifold::hull_manifolds(&ms), &mat, "hull")
    });
    e.register_fn("tris", |s: Solid| -> INT { s.0.num_tri() as INT });
    e.register_fn("bounds", |s: Solid| -> Map {
        let b = s.0.bounding_box();
        let mut m = Map::new();
        let arr = |x: f64, y: f64, z: f64| -> Dynamic {
            Dynamic::from(vec![Dynamic::from(x), Dynamic::from(y), Dynamic::from(z)])
        };
        m.insert("min".into(), arr(b.min.x, b.min.y, b.min.z));
        m.insert("max".into(), arr(b.max.x, b.max.y, b.max.z));
        m
    });

    // ── helpers ──
    e.register_fn("snap", |v: Dynamic| -> Res<f64> {
        Ok(snap(num(&v, "snap")?))
    });
    let kk = k.clone();
    e.register_fn("seed", move |n: INT| {
        kk.0.borrow_mut().seed = n as u64;
    });
    let kk = k.clone();
    e.register_fn("lod", move || -> INT { kk.0.borrow().lod as INT });
    let kk = k.clone();
    e.register_fn("rand", move |i: INT| -> f64 {
        let seed = kk.0.borrow().seed;
        crate::rng::unit(crate::rng::hash3(seed, (i, 0x5eed, 0)))
    });
    let kk = k.clone();
    e.register_fn("kind", move |s: &str| -> Res<()> {
        if s.len() > 32 {
            return err("kind: name too long");
        }
        kk.0.borrow_mut().kind = Some(s.to_ascii_lowercase());
        Ok(())
    });
    let kk = k.clone();
    e.register_fn("anchor", move |action: &str, o: Map| -> Res<()> {
        let op = Opts::new("anchor", &o, &["at", "face", "capacity"])?;
        if !ACTIONS.contains(&action) {
            return err(format!(
                "anchor: action '{action}' must be one of: {}",
                ACTIONS.join(", ")
            ));
        }
        let cap = op.num("capacity", 1.0)?.round().clamp(1.0, 16.0) as u8;
        kk.anchor(action, op.at()?, op.face()?, cap);
        Ok(())
    });

    // ── semantic pieces (authored facing north, then turned to `face`) ──
    let kk = k.clone();
    e.register_fn("door", move |body: Solid, o: Map| -> Res<Solid> {
        let op = Opts::new(
            "door",
            &o,
            &["at", "face", "w", "h", "mat", "frame", "arch", "step"],
        )?;
        let (at, yaw) = (op.at()?, op.face()?);
        let w = op.size("w", 0.9)?.clamp(0.7, 3.0);
        let h = op.size("h", 2.0)?.clamp(1.9, 4.0);
        let arch = op.boolean("arch", false)?;
        let leaf_mat = op.mat("mat", "door")?;
        let p = |m: Manifold| place(m, at, yaw);
        let cutter = if arch {
            geom::arch(w, h, 0.8, 12)
        } else {
            Manifold::cube(v3(w, h, 0.8), true).translate(v3(0.0, h / 2.0, 0.0))
        };
        let cutter = kk.tag(p(cutter), "trim", "door")?;
        let leaf = if arch {
            geom::arch(w - 0.1, h - 0.05, 0.1, 12)
        } else {
            geom::bevel_box(w - 0.1, h - 0.05, 0.1, 0.02, Edges::Y).translate(v3(
                0.0,
                (h - 0.05) / 2.0,
                0.0,
            ))
        };
        let mut parts = vec![geom::subtract(&body.0, &[cutter.0])];
        parts.push(
            kk.tag(p(leaf.translate(v3(0.0, 0.0, 0.25))), &leaf_mat, "door")?
                .0,
        );
        if op.boolean("frame", true)? {
            for sx in [-1.0, 1.0] {
                let j = geom::bevel_box(0.12, h + 0.06, 0.1, 0.02, Edges::Y).translate(v3(
                    sx * (w / 2.0 + 0.06),
                    (h + 0.06) / 2.0,
                    -0.03,
                ));
                parts.push(kk.tag(p(j), "trim", "door")?.0);
            }
            if !arch {
                let l = geom::bevel_box(w + 0.36, 0.14, 0.1, 0.02, Edges::X).translate(v3(
                    0.0,
                    h + 0.07,
                    -0.03,
                ));
                parts.push(kk.tag(p(l), "trim", "door")?.0);
            }
        }
        if op.boolean("step", true)? {
            let s =
                geom::bevel_box(w + 0.4, 0.15, 0.4, 0.03, Edges::X).translate(v3(0.0, 0.075, -0.2));
            parts.push(kk.tag(p(s), "stone", "door")?.0);
        }
        kk.anchor(
            "enter",
            local_to_world([0.0, 0.0, -0.7], at, yaw),
            yaw + 180.0,
            1,
        );
        checked(geom::union(&parts), "door")
    });

    let kk = k.clone();
    e.register_fn("window", move |body: Solid, o: Map| -> Res<Solid> {
        let op = Opts::new(
            "window",
            &o,
            &[
                "at", "face", "w", "h", "depth", "mat", "sill", "lintel", "shutters", "arch",
            ],
        )?;
        let (at, yaw) = (op.at()?, op.face()?);
        let w = op.size("w", 0.8)?.min(4.0);
        let h = op.size("h", 1.0)?.min(4.0);
        let depth = op.size("depth", 0.1)?.min(0.5);
        let p = |m: Manifold| place(m, at, yaw);
        let cutter = if op.boolean("arch", false)? {
            geom::arch(w, h, depth * 2.0, 10)
        } else {
            Manifold::cube(v3(w, h, depth * 2.0), true).translate(v3(0.0, h / 2.0, 0.0))
        };
        let cutter = kk.tag(p(cutter), &op.mat("mat", "glass")?, "window")?;
        let mut parts = vec![geom::subtract(&body.0, &[cutter.0])];
        if op.boolean("sill", true)? {
            let s = geom::bevel_box(w + 0.2, 0.1, 0.15, 0.02, Edges::X)
                .translate(v3(0.0, -0.05, -0.05));
            parts.push(kk.tag(p(s), "trim", "window")?.0);
        }
        if op.boolean("lintel", false)? {
            let l = geom::bevel_box(w + 0.2, 0.12, 0.1, 0.02, Edges::X).translate(v3(
                0.0,
                h + 0.06,
                -0.025,
            ));
            parts.push(kk.tag(p(l), "trim", "window")?.0);
        }
        if op.boolean("shutters", false)? {
            for sx in [-1.0, 1.0] {
                let s = Manifold::cube(v3(w / 2.0, h, 0.05), true).translate(v3(
                    sx * (w * 0.75 + 0.05),
                    h / 2.0,
                    -0.025,
                ));
                parts.push(kk.tag(p(s), "wood", "window")?.0);
            }
        }
        checked(geom::union(&parts), "window")
    });

    let kk = k.clone();
    e.register_fn("seat", move |o: Map| -> Res<Solid> {
        let op = Opts::new(
            "seat",
            &o,
            &["at", "face", "w", "d", "h", "mat", "back", "capacity"],
        )?;
        let (at, yaw) = (op.at()?, op.face()?);
        let w = op.size("w", 1.2)?.min(6.0);
        let d = op.size("d", 0.45)?.clamp(0.3, 1.2);
        let h = op.size("h", 0.45)?.clamp(0.35, 0.6);
        let mat = op.mat("mat", "wood")?;
        let p = |m: Manifold| place(m, at, yaw);
        let mut parts = vec![];
        let top = geom::bevel_box(w, 0.08, d, 0.02, Edges::X).translate(v3(0.0, h - 0.04, 0.0));
        parts.push(kk.tag(p(top), &mat, "seat")?.0);
        for sx in [-1.0, 1.0] {
            let leg = Manifold::cube(v3(0.1, h - 0.08, d - 0.1), true).translate(v3(
                sx * (w / 2.0 - 0.15),
                (h - 0.08) / 2.0,
                0.0,
            ));
            parts.push(kk.tag(p(leg), &mat, "seat")?.0);
        }
        if op.boolean("back", true)? {
            let b = geom::bevel_box(w, 0.4, 0.06, 0.02, Edges::X).translate(v3(
                0.0,
                h + 0.25,
                d / 2.0 - 0.03,
            ));
            parts.push(kk.tag(p(b), &mat, "seat")?.0);
        }
        let cap = op
            .num("capacity", (w / 0.6).floor().max(1.0))?
            .round()
            .clamp(1.0, 8.0) as u8;
        kk.anchor("sit", local_to_world([0.0, h, 0.0], at, yaw), yaw, cap);
        checked(geom::union(&parts), "seat")
    });

    let kk = k.clone();
    e.register_fn("bed", move |o: Map| -> Res<Solid> {
        let op = Opts::new("bed", &o, &["at", "face", "w", "l", "mat", "cloth"])?;
        let (at, yaw) = (op.at()?, op.face()?);
        let w = op.size("w", 1.0)?.clamp(0.8, 2.0);
        let l = op.size("l", 2.0)?.clamp(1.6, 2.4);
        let (mat, cloth) = (op.mat("mat", "wood")?, op.mat("cloth", "cloth")?);
        let p = |m: Manifold| place(m, at, yaw);
        let parts = vec![
            kk.tag(
                p(geom::bevel_box(w, 0.3, l, 0.03, Edges::All).translate(v3(0.0, 0.15, 0.0))),
                &mat,
                "bed",
            )?
            .0,
            kk.tag(
                p(geom::bevel_box(w - 0.1, 0.2, l - 0.1, 0.05, Edges::All)
                    .translate(v3(0.0, 0.4, 0.0))),
                &cloth,
                "bed",
            )?
            .0,
            kk.tag(
                p(
                    geom::bevel_box(w - 0.3, 0.12, 0.35, 0.04, Edges::All).translate(v3(
                        0.0,
                        0.56,
                        l / 2.0 - 0.3,
                    )),
                ),
                &cloth,
                "bed",
            )?
            .0,
            kk.tag(
                p(geom::bevel_box(w, 0.8, 0.08, 0.02, Edges::Y).translate(v3(
                    0.0,
                    0.4,
                    l / 2.0 + 0.04,
                ))),
                &mat,
                "bed",
            )?
            .0,
        ];
        kk.anchor("sleep", local_to_world([0.0, 0.5, 0.0], at, yaw), yaw, 1);
        checked(geom::union(&parts), "bed")
    });

    let kk = k.clone();
    e.register_fn("counter", move |o: Map| -> Res<Solid> {
        let op = Opts::new("counter", &o, &["at", "face", "w", "d", "h", "mat", "top"])?;
        let (at, yaw) = (op.at()?, op.face()?);
        let w = op.size("w", 1.6)?.min(6.0);
        let d = op.size("d", 0.6)?.clamp(0.4, 1.2);
        let h = op.size("h", 1.0)?.clamp(0.8, 1.2);
        let p = |m: Manifold| place(m, at, yaw);
        let body = geom::bevel_box(w, h - 0.06, d, 0.03, Edges::Y).translate(v3(
            0.0,
            (h - 0.06) / 2.0,
            0.0,
        ));
        let top = geom::bevel_box(w + 0.1, 0.06, d + 0.1, 0.02, Edges::All).translate(v3(
            0.0,
            h - 0.03,
            0.0,
        ));
        let parts = vec![
            kk.tag(p(body), &op.mat("mat", "wood")?, "counter")?.0,
            kk.tag(p(top), &op.mat("top", "trim")?, "counter")?.0,
        ];
        kk.anchor(
            "work",
            local_to_world([0.0, 0.0, d / 2.0 + 0.45], at, yaw),
            yaw,
            1,
        );
        checked(geom::union(&parts), "counter")
    });

    let kk = k.clone();
    e.register_fn("bench_top", move |o: Map| -> Res<Solid> {
        let op = Opts::new("bench_top", &o, &["at", "face", "w", "d", "h", "mat"])?;
        let (at, yaw) = (op.at()?, op.face()?);
        let w = op.size("w", 1.4)?.min(4.0);
        let d = op.size("d", 0.6)?.clamp(0.4, 1.2);
        let h = op.size("h", 0.9)?.clamp(0.7, 1.1);
        let mat = op.mat("mat", "wood")?;
        let p = |m: Manifold| place(m, at, yaw);
        let mut parts = vec![
            kk.tag(
                p(geom::bevel_box(w, 0.08, d, 0.02, Edges::All).translate(v3(0.0, h - 0.04, 0.0))),
                &mat,
                "bench_top",
            )?
            .0,
        ];
        for sx in [-1.0, 1.0] {
            let leg = Manifold::cube(v3(0.08, h - 0.08, d - 0.1), true).translate(v3(
                sx * (w / 2.0 - 0.1),
                (h - 0.08) / 2.0,
                0.0,
            ));
            parts.push(kk.tag(p(leg), &mat, "bench_top")?.0);
        }
        kk.anchor(
            "work",
            local_to_world([0.0, 0.0, -(d / 2.0 + 0.45)], at, yaw),
            yaw + 180.0,
            1,
        );
        checked(geom::union(&parts), "bench_top")
    });

    let kk = k.clone();
    e.register_fn("planter", move |o: Map| -> Res<Solid> {
        let op = Opts::new(
            "planter",
            &o,
            &["at", "face", "w", "d", "h", "mat", "plants"],
        )?;
        let (at, yaw) = (op.at()?, op.face()?);
        let w = op.size("w", 1.2)?.min(6.0);
        let d = op.size("d", 0.5)?.clamp(0.3, 3.0);
        let h = op.size("h", 0.45)?.clamp(0.2, 1.2);
        let p = |m: Manifold| place(m, at, yaw);
        let mut parts = vec![
            kk.tag(
                p(geom::bevel_box(w, h, d, 0.04, Edges::All).translate(v3(0.0, h / 2.0, 0.0))),
                &op.mat("mat", "stone")?,
                "planter",
            )?
            .0,
        ];
        if op.boolean("plants", true)? {
            let n = ((w / 0.45).floor() as i32).clamp(1, 8);
            let r = (d * 0.45).min(0.3);
            for i in 0..n {
                let x = -w / 2.0 + w * (i as f64 + 0.5) / n as f64;
                let b = geom::blob(r, 8, 0.8).translate(v3(x, h + r * 0.4, 0.0));
                parts.push(kk.tag(p(b), "leaf", "planter")?.0);
            }
        }
        kk.anchor(
            "plant",
            local_to_world([0.0, 0.0, -(d / 2.0 + 0.4)], at, yaw),
            yaw + 180.0,
            1,
        );
        checked(geom::union(&parts), "planter")
    });

    e
}

/// What a finished script produced before canonicalisation.
pub struct Raw {
    pub solid: Option<Manifold>,
    pub ctx: Ctx,
}

pub struct RunSpec<'a> {
    pub limits: Limits,
    pub lib: Option<&'a AST>,
    pub vars: Vec<(&'static str, Dynamic)>,
    pub seed: u64,
    /// Level of detail the script may read with `lod()` (0 = full).
    pub lod: u8,
    pub allow_empty: bool,
    /// Snap sizes and positions to the 5 cm grid (off only for the kit).
    pub grid: bool,
}

pub fn compile(src: &str) -> Result<AST, ScriptError> {
    // A throwaway engine with the same registrations so unknown functions are
    // still only a runtime error, like in Rhai, but syntax errors surface here.
    let ctx = Rc::new(RefCell::new(new_ctx(Limits::user(0), 0)));
    let e = engine(ctx);
    e.compile(src).map_err(|p| ScriptError {
        line: p.1.line().map(|l| l as u32),
        column: p.1.position().map(|c| c as u32),
        message: p.0.to_string(),
        kind: "syntax".into(),
    })
}

fn new_ctx(limits: Limits, seed: u64) -> Ctx {
    Ctx {
        slot_of: HashMap::new(),
        anchors: vec![],
        prims: 0,
        kind: None,
        seed,
        lod: 0,
        deadline: Instant::now() + limits.time,
        limits,
    }
}

pub fn run(src_ast: &AST, spec: RunSpec) -> Result<Raw, ScriptError> {
    geom::set_snap(spec.grid);
    let r = run_inner(src_ast, spec);
    geom::set_snap(true);
    r
}

fn run_inner(src_ast: &AST, spec: RunSpec) -> Result<Raw, ScriptError> {
    let mut c = new_ctx(spec.limits.clone(), spec.seed);
    c.lod = spec.lod;
    let ctx = Rc::new(RefCell::new(c));
    let e = engine(ctx.clone());
    let merged;
    let ast = match spec.lib {
        Some(lib) => {
            merged = lib.merge(src_ast);
            &merged
        }
        None => src_ast,
    };
    let mut scope = Scope::new();
    for (k, v) in spec.vars {
        scope.push_constant(k, v);
    }
    let out = e
        .eval_ast_with_scope::<Dynamic>(&mut scope, ast)
        .map_err(|x| runtime_error(&x))?;
    drop(e);
    let ctx = Rc::try_unwrap(ctx)
        .ok()
        .expect("engine dropped")
        .into_inner();
    let solid = if out.is_unit() {
        None
    } else if let Some(s) = out.clone().try_cast::<Solid>() {
        Some(s.0)
    } else if let Some(a) = out.read_lock::<Array>() {
        let parts = solids(&a, "result").map_err(|x| runtime_error(&x))?;
        Some(geom::union(&parts))
    } else {
        return Err(ScriptError {
            line: None,
            column: None,
            message: format!(
                "the script must end with a solid (or an array of solids), got {}",
                out.type_name()
            ),
            kind: "result".into(),
        });
    };
    if solid.is_none() && !spec.allow_empty {
        return Err(ScriptError {
            line: None,
            column: None,
            message: "the script produced nothing: end it with the solid to build".into(),
            kind: "result".into(),
        });
    }
    if let Some(s) = &solid {
        if !geom::ok(s) {
            return Err(ScriptError {
                line: None,
                column: None,
                message: format!("the final solid is invalid ({:?})", s.status()),
                kind: "geometry".into(),
            });
        }
        if s.num_tri() > spec.limits.max_tris {
            return Err(ScriptError {
                line: None,
                column: None,
                message: format!(
                    "{} triangles is over the limit of {} for this class: use fewer segments or parts",
                    s.num_tri(),
                    spec.limits.max_tris
                ),
                kind: "limit".into(),
            });
        }
    }
    Ok(Raw { solid, ctx })
}

fn runtime_error(e: &EvalAltResult) -> ScriptError {
    // Walk to the innermost error: that is where the message and the most
    // precise position live.
    let mut cur = e;
    let mut pos = e.position();
    while let EvalAltResult::ErrorInFunctionCall(_, _, inner, p)
    | EvalAltResult::ErrorInModule(_, inner, p) = cur
    {
        if !p.is_none() {
            pos = *p;
        }
        cur = inner;
    }
    if !cur.position().is_none() {
        pos = cur.position();
    }
    let kind = match cur {
        EvalAltResult::ErrorTooManyOperations(..) | EvalAltResult::ErrorTerminated(..) => "limit",
        EvalAltResult::ErrorStackOverflow(..)
        | EvalAltResult::ErrorDataTooLarge(..)
        | EvalAltResult::ErrorTooManyModules(..) => "limit",
        EvalAltResult::ErrorFunctionNotFound(..) => "unknown_function",
        _ => "runtime",
    };
    let message = match cur {
        EvalAltResult::ErrorRuntime(d, _) => d.to_string(),
        EvalAltResult::ErrorTerminated(..) => "time limit exceeded".to_string(),
        EvalAltResult::ErrorTooManyOperations(..) => {
            "too many operations (endless loop?): the kit allows 200 000 steps".to_string()
        }
        other => {
            let s = other.to_string();
            match s.rfind(" (line ") {
                Some(i) => s[..i].to_string(),
                None => s,
            }
        }
    };
    let kind = if kind == "runtime" && message.contains(" limit") {
        "limit"
    } else {
        kind
    };
    ScriptError {
        line: pos.line().map(|l| l as u32),
        column: pos.position().map(|c| c as u32),
        message,
        kind: kind.into(),
    }
}
