//! Roof pieces: which kit module sits on a roof dual vertex, turned how, and
//! how high above the wall top.
//!
//! A pitched roof is built from pieces placed at `d` steps above the eave,
//! where `d` is the distance (in tiles) from the vertex to the nearest eave
//! along the slope, measured on the building's own roof tiles (per column /
//! row run, so eaves always sit at `d = 0`). Each step rises
//! [`RISE_MM`] (slope 1:2) or [`RISE_CONICO_MM`] for conical tower caps.
//! Every piece is modelled as a height field `y(x, z)` in its canonical frame
//! (see `keys::kit_catalog`), which is what makes neighbours meet.

use super::cases::*;
use crate::types::{Axis, Dir};

pub const RISE_MM: i32 = 1000;
pub const RISE_CONICO_MM: i32 = 4000;

/// Distances from a vertex to the eave on each side, in tiles: `[N, E, S, W]`
/// (`i32::MAX` = no roof tile on that axis around the vertex).
pub fn eave_distances(is_roof: &dyn Fn(i32, i32) -> bool, vx: i32, vy: i32) -> [i32; 4] {
    let mut d = [i32::MAX; 4];
    for c in [vx - 1, vx] {
        let t = if is_roof(c, vy - 1) {
            vy - 1
        } else if is_roof(c, vy) {
            vy
        } else {
            continue;
        };
        let (mut s, mut e) = (t, t);
        while s > -1 && is_roof(c, s - 1) {
            s -= 1;
        }
        while e < 64 && is_roof(c, e + 1) {
            e += 1;
        }
        d[0] = d[0].min(vy - s);
        d[2] = d[2].min(e + 1 - vy);
    }
    for r in [vy - 1, vy] {
        let t = if is_roof(vx - 1, r) {
            vx - 1
        } else if is_roof(vx, r) {
            vx
        } else {
            continue;
        };
        let (mut s, mut e) = (t, t);
        while s > -1 && is_roof(s - 1, r) {
            s -= 1;
        }
        while e < 64 && is_roof(e + 1, r) {
            e += 1;
        }
        d[3] = d[3].min(vx - s);
        d[1] = d[1].min(e + 1 - vx);
    }
    d
}

/// Roof family of a `roof.kind` (unknown kinds are flat).
pub fn family(kind: &str) -> &'static str {
    match kind {
        "duas-aguas" => "duas-aguas",
        "quatro-aguas" => "quatro-aguas",
        "conico" => "conico",
        _ => "plano",
    }
}

pub fn rise_of(family: &str) -> i32 {
    match family {
        "plano" => 0,
        "conico" => RISE_CONICO_MM,
        _ => RISE_MM,
    }
}

/// The directions the roof slopes toward (eaves) for a family and ridge axis.
fn slope_dirs(family: &str, axis: Axis) -> &'static [Dir] {
    match (family, axis) {
        ("duas-aguas", Axis::X) => &[Dir::N, Dir::S],
        ("duas-aguas", Axis::Y) => &[Dir::E, Dir::W],
        _ => &[Dir::N, Dir::E, Dir::S, Dir::W],
    }
}

/// Steps above the eave at a vertex (what the preview uses for heights).
pub fn steps(family: &str, axis: Axis, dist: &[i32; 4]) -> i32 {
    if family == "plano" {
        return 0;
    }
    let m = slope_dirs(family, axis)
        .iter()
        .map(|d| dist[d.quarter() as usize])
        .min()
        .unwrap_or(0);
    if m == i32::MAX {
        0
    } else {
        m
    }
}

/// A roof piece: the case part of the key (`duas-aguas-rampa`, …), its
/// rotation and its height above the wall top in mm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub name: String,
    pub rot: u8,
    pub lift_mm: i32,
}

/// Pick the piece for a roof vertex with quadrant case `(case, crot)`.
pub fn piece(family: &str, axis: Axis, case: u8, crot: u8, dist: &[i32; 4]) -> Piece {
    let d = steps(family, axis, dist);
    let lift_mm = d * rise_of(family);
    let slopes = slope_dirs(family, axis);
    let is_slope = |x: Dir| slopes.contains(&x);
    let (name, rot) = match family {
        "plano" => (format!("plano-{}", CASE_NAMES[case as usize]), crot),
        "duas-aguas" => match case {
            CHEIO => {
                let sides: Vec<Dir> = slopes
                    .iter()
                    .copied()
                    .filter(|x| dist[x.quarter() as usize] == d)
                    .collect();
                if sides.len() >= 2 {
                    (
                        "duas-aguas-cumeeira".into(),
                        if axis == Axis::X { 0 } else { 1 },
                    )
                } else {
                    let s = sides.first().copied().unwrap_or(slopes[0]);
                    ("duas-aguas-rampa".into(), s.quarter())
                }
            }
            PAREDE => {
                let o = rotate(Dir::N, crot);
                if is_slope(o) {
                    ("duas-aguas-beiral".into(), crot)
                } else {
                    // Gable end: which way does the roof line rise along it?
                    let (a, b) = (slopes[0], slopes[1]);
                    let (da, db) = (dist[a.quarter() as usize], dist[b.quarter() as usize]);
                    let name = if da == db {
                        "duas-aguas-oitao-topo"
                    } else {
                        let rising = if da < db { b } else { a };
                        if rising == rotate(Dir::E, crot) {
                            "duas-aguas-oitao-leste"
                        } else {
                            "duas-aguas-oitao-oeste"
                        }
                    };
                    (name.into(), crot)
                }
            }
            CONVEXO | CONCAVO => {
                let base = if case == CONVEXO {
                    "duas-aguas-canto"
                } else {
                    "duas-aguas-concavo"
                };
                let eave_n = is_slope(rotate(Dir::N, crot));
                (
                    format!("{base}-beiral-{}", if eave_n { "n" } else { "o" }),
                    crot,
                )
            }
            _ => ("duas-aguas-diagonal".into(), crot),
        },
        _ => {
            // quatro-aguas and conico share the hip logic.
            let pre = if family == "conico" {
                "conico"
            } else {
                "quatro-aguas"
            };
            let (part, rot) = match case {
                CHEIO => hip_interior(dist, d),
                CONVEXO => ("canto", crot),
                PAREDE => ("beiral", crot),
                CONCAVO => ("concavo", crot),
                _ => ("diagonal", crot),
            };
            let part = if family == "conico" {
                match part {
                    "beiral" => "beiral",
                    "canto" | "concavo" | "diagonal" => "canto",
                    _ => "pico",
                }
            } else {
                part
            };
            let rot = if family == "conico" && part == "pico" {
                0
            } else {
                rot
            };
            (format!("{pre}-{part}"), rot)
        }
    };
    Piece { name, rot, lift_mm }
}

/// Interior hip piece from the sides at the minimum distance.
fn hip_interior(dist: &[i32; 4], d: i32) -> (&'static str, u8) {
    let at: Vec<u8> = (0..4u8).filter(|q| dist[*q as usize] == d).collect();
    match at.len() {
        0 | 4 => ("pico", 0),
        1 => ("rampa", at[0]),
        2 => {
            let (a, b) = (at[0], at[1]);
            if (a + 2) % 4 == b {
                ("cumeeira", if a == 0 { 0 } else { 1 })
            } else {
                // Canonical espigao is low toward N and W: rot r covers {r, r+3}.
                let r = if (a + 3) % 4 == b { a } else { b };
                ("espigao", r)
            }
        }
        _ => {
            let missing = (0..4u8).find(|q| !at.contains(q)).unwrap_or(1);
            ("topo", (missing + 3) % 4)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hip_pieces_turn_right() {
        // Low toward N and W → espigao rot 0; toward N and E → rot 1.
        assert_eq!(hip_interior(&[1, 5, 5, 1], 1), ("espigao", 0));
        assert_eq!(hip_interior(&[1, 1, 5, 5], 1), ("espigao", 1));
        assert_eq!(hip_interior(&[5, 1, 1, 5], 1), ("espigao", 2));
        assert_eq!(hip_interior(&[5, 5, 1, 1], 1), ("espigao", 3));
        // Missing E → topo rot 0.
        assert_eq!(hip_interior(&[1, 5, 1, 1], 1), ("topo", 0));
    }
}
