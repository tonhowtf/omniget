//! Geometry primitives of the kit, on top of Manifold. Everything here is in
//! metres with the three.js axes (x = east, y = up, z = south) and knows
//! nothing about Rhai: `script.rs` validates options and tags materials.

use manifold_rust::cross_section::CrossSection;
use manifold_rust::linalg::{Vec2, Vec3};
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{Error as MfError, OpType};

/// Every position and size the kit accepts snaps to this grid (5 cm).
pub const GRID: f64 = 0.05;

thread_local! {
    static SNAP_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Turns grid snapping on or off for this thread (the built-in kit scripts
/// are trusted and author finer curves; user scripts always snap).
pub fn set_snap(on: bool) {
    SNAP_ON.with(|c| c.set(on));
}

pub fn snap_on() -> bool {
    SNAP_ON.with(|c| c.get())
}

pub fn snap(v: f64) -> f64 {
    if SNAP_ON.with(|c| c.get()) {
        (v / GRID).round() * GRID
    } else {
        (v * 10_000.0).round() / 10_000.0
    }
}

pub fn v3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

pub fn ok(m: &Manifold) -> bool {
    m.status() == MfError::NoError
}

/// Which box edges get the chamfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edges {
    All,
    X,
    Y,
    Z,
    None,
}

impl Edges {
    pub fn parse(s: &str) -> Option<Edges> {
        Some(match s {
            "all" => Edges::All,
            "x" => Edges::X,
            "y" => Edges::Y,
            "z" => Edges::Z,
            "none" => Edges::None,
            _ => return None,
        })
    }
}

/// Box centred at the origin, chamfered by `b` on the chosen edges. The
/// chamfer is a convex hull of inset boxes: one segment, cheap and manifold.
pub fn bevel_box(w: f64, h: f64, d: f64, b: f64, edges: Edges) -> Manifold {
    let cube = |x: f64, y: f64, z: f64| Manifold::cube(v3(x, y, z), true);
    if b <= 0.0 || edges == Edges::None {
        return cube(w, h, d);
    }
    let b2 = 2.0 * b;
    match edges {
        Edges::All => Manifold::hull_manifolds(&[
            cube(w, h - b2, d - b2),
            cube(w - b2, h, d - b2),
            cube(w - b2, h - b2, d),
        ]),
        Edges::X => Manifold::hull_manifolds(&[cube(w, h - b2, d), cube(w, h, d - b2)]),
        Edges::Y => Manifold::hull_manifolds(&[cube(w - b2, h, d), cube(w, h, d - b2)]),
        Edges::Z => Manifold::hull_manifolds(&[cube(w, h - b2, d), cube(w - b2, h, d)]),
        Edges::None => cube(w, h, d),
    }
}

/// Vertical cylinder (or truncated cone) standing on y = 0. A chamfer rounds
/// both rims when `b > 0`.
pub fn cylinder(r: f64, r2: f64, h: f64, seg: i32, b: f64) -> Manifold {
    let up = |m: Manifold| m.rotate(-90.0, 0.0, 0.0);
    if b <= 0.0 || r2 <= 0.0 {
        return up(Manifold::cylinder(h, r, r2.max(0.0), seg));
    }
    let inner = Manifold::cylinder(h, (r - b).max(0.01), (r2 - b).max(0.01), seg);
    let outer = Manifold::cylinder(h - 2.0 * b, r, r2, seg).translate(v3(0.0, 0.0, b));
    up(Manifold::hull_manifolds(&[inner, outer]))
}

fn signed_area(p: &[Vec2]) -> f64 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (p0, p1) = (p[i], p[(i + 1) % p.len()]);
        a += p0.x * p1.y - p1.x * p0.y;
    }
    a * 0.5
}

/// Extrudes a 2D profile. `axis` picks the plane of the profile:
/// - `y`: profile in plan `(x, z)`, extruded up from y = 0 to `len`;
/// - `x`: profile in `(z, y)`, extruded along x, centred;
/// - `z`: profile in `(x, y)`, extruded along z, centred.
///
/// Returns `None` when the profile has no area.
pub fn extrude(profile: &[[f64; 2]], len: f64, axis: char) -> Option<Manifold> {
    let uv: Vec<Vec2> = profile
        .iter()
        .map(|p| match axis {
            'y' => Vec2::new(p[0], -p[1]),
            'x' => Vec2::new(-p[0], p[1]),
            _ => Vec2::new(p[0], p[1]),
        })
        .collect();
    let cs = CrossSection::from_polygons_fill(vec![uv]);
    let mut polys = cs.to_polygons();
    polys.retain(|p| p.len() >= 3 && signed_area(p).abs() > 1e-9);
    if polys.is_empty() {
        return None;
    }
    for p in polys.iter_mut() {
        // Clipper returns outer contours CCW already; keep the guard for
        // single contours so a CW input still extrudes outward.
        if polys_len_one_cw(p) {
            p.reverse();
        }
    }
    let m = Manifold::extrude(&polys, len, 0, 0.0, Vec2::new(1.0, 1.0));
    if !ok(&m) || m.is_empty() {
        return None;
    }
    Some(match axis {
        'y' => m.rotate(-90.0, 0.0, 0.0),
        'x' => m.rotate(0.0, 90.0, 0.0).translate(v3(-len / 2.0, 0.0, 0.0)),
        _ => m.translate(v3(0.0, 0.0, -len / 2.0)),
    })
}

fn polys_len_one_cw(p: &[Vec2]) -> bool {
    signed_area(p) < 0.0
}

/// Two-pitch roof: a chevron slab with the ridge along x, plus the gable
/// fill under it (returned separately so it can take the wall slot).
/// `w` = length along the ridge, `d` = depth of the building under it,
/// `h` = ridge height above y = 0, `eave` = overhang, `t` = slab thickness.
pub fn gable(w: f64, d: f64, h: f64, eave: f64, t: f64) -> Option<(Manifold, Manifold)> {
    let half = d / 2.0;
    let k = h / half;
    let dd = half + eave;
    let low = -eave * k;
    let chevron = [
        [-dd, low],
        [-dd, low + t],
        [0.0, h + t],
        [dd, low + t],
        [dd, low],
        [0.0, h],
    ];
    let roof = extrude(&chevron, w + 2.0 * eave * 0.5, 'x')?;
    let fill = [
        [-half, 0.0],
        [half, 0.0],
        [half, t * 0.5],
        [0.0, h + t * 0.5],
        [-half, t * 0.5],
    ];
    let fill = extrude(&fill, w, 'x')?;
    Some((roof, fill))
}

/// Four-pitch roof as a convex hull: eaves on all sides, ridge along the
/// longer axis, flat soffit under the overhang.
pub fn hip(w: f64, d: f64, h: f64, eave: f64, t: f64) -> Manifold {
    let k = h / (w.min(d) / 2.0);
    let (ew, ed) = (w / 2.0 + eave, d / 2.0 + eave);
    let low = -eave * k;
    let mut pts = Vec::with_capacity(10);
    for (x, z) in [(-ew, -ed), (ew, -ed), (ew, ed), (-ew, ed)] {
        pts.push(v3(x, low, z));
        pts.push(v3(x, low + t, z));
    }
    let r = ((w - d).abs() / 2.0).max(0.0);
    if w >= d {
        pts.push(v3(-r, h + t, 0.0));
        pts.push(v3(r, h + t, 0.0));
    } else {
        pts.push(v3(0.0, h + t, -r));
        pts.push(v3(0.0, h + t, r));
    }
    Manifold::hull(&pts)
}

/// Conical roof: a cone with a thick eave ring.
pub fn cone_roof(r: f64, h: f64, eave: f64, t: f64, seg: i32) -> Manifold {
    let rr = r + eave;
    let low = -eave * (h / r);
    let mut pts = Vec::with_capacity(seg as usize * 2 + 1);
    for i in 0..seg {
        let a = (i as f64) * 360.0 / seg as f64;
        let (s, c) = (sind(a), cosd(a));
        pts.push(v3(rr * c, low, rr * s));
        pts.push(v3(rr * c, low + t, rr * s));
    }
    pts.push(v3(0.0, h + t, 0.0));
    Manifold::hull(&pts)
}

/// Ramp: full height `h` at the north edge (−z), zero at the south edge.
pub fn wedge(w: f64, h: f64, d: f64) -> Manifold {
    let (x, z) = (w / 2.0, d / 2.0);
    Manifold::hull(&[
        v3(-x, 0.0, -z),
        v3(x, 0.0, -z),
        v3(-x, 0.0, z),
        v3(x, 0.0, z),
        v3(-x, h, -z),
        v3(x, h, -z),
    ])
}

/// Arch-shaped solid (a box topped by a half cylinder), opening along z,
/// standing on y = 0 with total height `h`.
pub fn arch(w: f64, h: f64, d: f64, seg: i32) -> Manifold {
    let r = w / 2.0;
    let body_h = (h - r).max(0.05);
    let body = Manifold::cube(v3(w, body_h, d), true).translate(v3(0.0, body_h / 2.0, 0.0));
    let top = Manifold::cylinder(d, r, r, seg)
        .translate(v3(0.0, 0.0, -d / 2.0))
        .translate(v3(0.0, body_h, 0.0));
    Manifold::batch_boolean(&[body, top], OpType::Add)
}

/// Low-poly sphere, optionally squashed on y.
pub fn blob(r: f64, seg: i32, squash: f64) -> Manifold {
    let m = Manifold::sphere(r, seg);
    if (squash - 1.0).abs() > 1e-9 {
        m.scale(v3(1.0, squash, 1.0))
    } else {
        m
    }
}

/// Exact sine/cosine for multiples of 90°, the platform libm otherwise.
pub fn sind(deg: f64) -> f64 {
    let r = deg.rem_euclid(360.0);
    if r == 0.0 || r == 180.0 {
        0.0
    } else if r == 90.0 {
        1.0
    } else if r == 270.0 {
        -1.0
    } else {
        r.to_radians().sin()
    }
}

pub fn cosd(deg: f64) -> f64 {
    sind(deg + 90.0)
}

/// Yaw clockwise seen from above, in degrees (north → east for 90°).
pub fn turn(m: &Manifold, deg: f64) -> Manifold {
    if deg.rem_euclid(360.0) == 0.0 {
        return m.clone();
    }
    m.rotate(0.0, -deg, 0.0)
}

/// Rotates a point the same way as [`turn`].
pub fn turn_point(p: [f64; 3], deg: f64) -> [f64; 3] {
    let (s, c) = (sind(deg), cosd(deg));
    // CW from above: north (0,0,-1) → east (1,0,0).
    [p[0] * c - p[2] * s, p[1], p[0] * s + p[2] * c]
}

pub fn union(parts: &[Manifold]) -> Manifold {
    match parts.len() {
        0 => Manifold::empty(),
        1 => parts[0].clone(),
        _ => Manifold::batch_boolean(parts, OpType::Add),
    }
}

pub fn subtract(base: &Manifold, cuts: &[Manifold]) -> Manifold {
    if cuts.is_empty() {
        return base.clone();
    }
    let mut all = Vec::with_capacity(cuts.len() + 1);
    all.push(base.clone());
    all.extend(cuts.iter().cloned());
    Manifold::batch_boolean(&all, OpType::Subtract)
}

pub fn intersect(a: &Manifold, b: &Manifold) -> Manifold {
    a.boolean(b, OpType::Intersect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_is_clockwise_from_above() {
        let p = turn_point([0.0, 0.0, -1.0], 90.0);
        assert!((p[0] - 1.0).abs() < 1e-12 && p[2].abs() < 1e-12);
        let m = Manifold::cube(v3(0.2, 0.2, 0.2), true).translate(v3(0.0, 0.0, -1.0));
        let bb = turn(&m, 90.0).bounding_box();
        assert!(bb.min.x > 0.8 && bb.max.z.abs() < 0.2);
    }

    #[test]
    fn extrude_axes() {
        let sq = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let y = extrude(&sq, 2.0, 'y').unwrap().bounding_box();
        assert!(
            (y.max.y - 2.0).abs() < 1e-9 && (y.max.z - 1.0).abs() < 1e-9 && y.min.z.abs() < 1e-9
        );
        let x = extrude(&sq, 2.0, 'x').unwrap().bounding_box();
        assert!(
            (x.max.x - 1.0).abs() < 1e-9
                && (x.max.z - 1.0).abs() < 1e-9
                && (x.max.y - 1.0).abs() < 1e-9
        );
        let cw: Vec<[f64; 2]> = sq.iter().rev().copied().collect();
        let m = extrude(&cw, 1.0, 'z').unwrap();
        assert!(m.volume() > 0.99);
    }
}
