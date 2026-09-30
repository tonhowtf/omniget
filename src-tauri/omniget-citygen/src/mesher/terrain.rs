//! Terrain triangles on the same dual grid as the buildings.
//!
//! - A tile whose material does not mix with its neighbours is 2 triangles
//!   with the height of its 4 corners.
//! - A mixed tile is cut in 4 quadrants (one per corner), each split along the
//!   line between its two edge midpoints: the outer triangle (touching the tile
//!   centre) keeps the tile's material; the inner one (touching the corner)
//!   takes the material the dual cell of that corner assigns to it. That cell
//!   reads the 4 tiles around the corner and falls in one of the 15 set
//!   partitions of 4 colours (BorisTheBrave, "2D marching cubes with multiple
//!   colors"): the rule table is [`inner_colours`]. 8 triangles per mixed tile.
//! - Heights: corners are the blueprint's; edge midpoints are the integer mean
//!   of their 2 corners and centres the mean of 4, so a 2-triangle tile and a
//!   cut neighbour share the same edge line (no cracks).
//! - Water is a flat surface at the water level, drawn over a bed: corners
//!   whose 4 tiles are all water sink to [`BED_MM`] below the level (or below
//!   their own height, if lower); the shore keeps its height.
//!   The level is the lowest shore corner (the blueprint does not carry the
//!   charter's sea level; see [`terrain_mesh_with_level`]).
//!
//! Budget: at most 8 × 4096 = 32 768 triangles plus the water surface; above
//! [`MAX_TRIANGLES`] the mesh is rebuilt without cutting tiles.

use crate::types::*;
use std::collections::HashMap;

pub const BED_MM: i32 = 800;
pub const MAX_TRIANGLES: usize = 40_000;

pub fn slot_index(name: &str) -> u8 {
    PALETTE_SLOTS.iter().position(|s| *s == name).unwrap_or(0) as u8
}

/// Palette slot of a ground surface.
pub fn surface_slot(s: Surface) -> &'static str {
    match s {
        Surface::Grass | Surface::Lot | Surface::Yard => "grass",
        Surface::Forest => "leaf",
        Surface::Sand | Surface::Field => "ground",
        Surface::Rock => "stone",
        Surface::Water | Surface::Bridge => "water",
        Surface::Street => "street",
        Surface::Sidewalk | Surface::Plaza => "plaza",
    }
}

/// Which colour wins the ambiguous saddle (two diagonal pairs): roads and
/// water stay connected.
fn priority(slot: u8) -> u8 {
    match PALETTE_SLOTS[slot as usize] {
        "street" => 9,
        "water" => 8,
        "plaza" => 7,
        "stone" => 6,
        "ground" => 4,
        "leaf" => 3,
        "grass" => 2,
        _ => 1,
    }
}

/// The 15 partitions of 4 corner colours (NW, NE, SE, SW) → the colour of
/// each quadrant's inner triangle:
/// - 1 class (`AAAA`): own colour;
/// - 3 + 1 (`AAAB` ×4): the lone corner shrinks to its outer triangle, its
///   inner triangle takes the majority colour (rounded corner);
/// - 2 + 2 adjacent (`AABB` ×2): own colour, a straight border;
/// - 2 + 2 diagonal (`ABAB`): saddle, all inner triangles take the higher
///   [`priority`] colour;
/// - 2 diagonal + 1 + 1 (`ABAC` ×2): the diagonal pair joins through the centre;
/// - 2 adjacent + 1 + 1 (`AABC` ×4) and 1+1+1+1 (`ABCD`): own colour.
pub fn inner_colours(c: [u8; 4]) -> [u8; 4] {
    let count = |x: u8| c.iter().filter(|y| **y == x).count();
    // 3 + 1
    for i in 0..4 {
        if count(c[i]) == 1 {
            let j = (i + 1) % 4;
            if count(c[j]) == 3 {
                let mut out = c;
                out[i] = c[j];
                return out;
            }
        }
    }
    let diag_a = c[0] == c[2];
    let diag_b = c[1] == c[3];
    if diag_a && diag_b && c[0] != c[1] {
        let w = if priority(c[0]) >= priority(c[1]) {
            c[0]
        } else {
            c[1]
        };
        return [w; 4];
    }
    if diag_a && c[0] != c[1] && c[0] != c[3] && c[1] != c[3] {
        return [c[0]; 4];
    }
    if diag_b && c[1] != c[0] && c[1] != c[2] && c[0] != c[2] {
        return [c[1]; 4];
    }
    c
}

/// Lowest height among corners that touch both water and land.
pub fn water_level_mm(bp: &Blueprint) -> Option<i32> {
    let wet = |x: i32, y: i32| -> Option<bool> {
        if x < 0 || y < 0 || x >= REGION_TILES || y >= REGION_TILES {
            return None;
        }
        Some(matches!(
            bp.surface.get(tile_index(x, y)),
            Some(Surface::Water) | Some(Surface::Bridge)
        ))
    };
    let mut shore = i32::MAX;
    let mut any = i32::MAX;
    for vy in 0..=REGION_TILES {
        for vx in 0..=REGION_TILES {
            let around = [
                wet(vx - 1, vy - 1),
                wet(vx, vy - 1),
                wet(vx, vy),
                wet(vx - 1, vy),
            ];
            let w = around.iter().filter(|a| **a == Some(true)).count();
            let l = around.iter().filter(|a| **a == Some(false)).count();
            let h = corner_h(bp, vx, vy);
            if w > 0 {
                any = any.min(h);
                if l > 0 {
                    shore = shore.min(h);
                }
            }
        }
    }
    if shore != i32::MAX {
        Some(shore)
    } else if any != i32::MAX {
        Some(any)
    } else {
        None
    }
}

pub fn corner_h(bp: &Blueprint, vx: i32, vy: i32) -> i32 {
    let (x, y) = (vx.clamp(0, REGION_TILES), vy.clamp(0, REGION_TILES));
    bp.corner_height_mm
        .get(corner_index(x, y))
        .copied()
        .unwrap_or(0)
}

pub fn surface_at(bp: &Blueprint, x: i32, y: i32) -> Surface {
    let (x, y) = (x.clamp(0, REGION_TILES - 1), y.clamp(0, REGION_TILES - 1));
    bp.surface
        .get(tile_index(x, y))
        .copied()
        .unwrap_or(Surface::Grass)
}

pub fn terrain_mesh(bp: &Blueprint) -> TerrainMesh {
    terrain_mesh_with_level(bp, None)
}

/// Same as [`terrain_mesh`] with an explicit water level (the charter's
/// `sea_level_mm`, when the caller has it).
pub fn terrain_mesh_with_level(bp: &Blueprint, water_mm: Option<i32>) -> TerrainMesh {
    let level = water_mm.or_else(|| water_level_mm(bp));
    let m = build(bp, level, true);
    if m.indices.len() / 3 > MAX_TRIANGLES {
        return build(bp, level, false);
    }
    m
}

struct Builder {
    positions: Vec<i32>,
    slots: Vec<u8>,
    indices: Vec<u32>,
    seen: HashMap<(i32, i32, i32, u8), u32>,
}

impl Builder {
    fn vertex(&mut self, p: [i32; 3], slot: u8) -> u32 {
        let key = (p[0], p[1], p[2], slot);
        if let Some(i) = self.seen.get(&key) {
            return *i;
        }
        let i = self.slots.len() as u32;
        self.positions.extend_from_slice(&p);
        self.slots.push(slot);
        self.seen.insert(key, i);
        i
    }

    /// Triangle facing up (+y): counter-clockwise seen from above in three.js.
    fn tri(&mut self, a: [i32; 3], b: [i32; 3], c: [i32; 3], slot: u8) {
        let cross = (b[2] as i64 - a[2] as i64) * (c[0] as i64 - a[0] as i64)
            - (b[0] as i64 - a[0] as i64) * (c[2] as i64 - a[2] as i64);
        let (b, c) = if cross >= 0 { (b, c) } else { (c, b) };
        let ia = self.vertex(a, slot);
        let ib = self.vertex(b, slot);
        let ic = self.vertex(c, slot);
        self.indices.extend_from_slice(&[ia, ib, ic]);
    }
}

fn build(bp: &Blueprint, level: Option<i32>, cut: bool) -> TerrainMesh {
    let n = REGION_TILES;
    let water = slot_index("water");
    let bed = slot_index("ground");
    let colour = |x: i32, y: i32| slot_index(surface_slot(surface_at(bp, x, y)));
    let wet = |x: i32, y: i32| colour(x, y) == water;
    // Corner heights with the water bed dug in.
    let mut h = vec![0i32; ((n + 1) * (n + 1)) as usize];
    for vy in 0..=n {
        for vx in 0..=n {
            let mut v = corner_h(bp, vx, vy);
            let inside = |x: i32, y: i32| x >= 0 && y >= 0 && x < n && y < n;
            let all_wet = [(vx - 1, vy - 1), (vx, vy - 1), (vx, vy), (vx - 1, vy)]
                .iter()
                .all(|&(x, y)| !inside(x, y) || wet(x, y));
            if all_wet {
                v = v.min(level.unwrap_or(v)) - BED_MM;
            }
            h[corner_index(vx, vy)] = v;
        }
    }
    let hc = |vx: i32, vy: i32| h[corner_index(vx, vy)];
    // Inner colours of every dual cell, indexed by vertex.
    let mut inner = vec![[0u8; 4]; ((n + 1) * (n + 1)) as usize];
    for vy in 0..=n {
        for vx in 0..=n {
            let c = [
                colour(vx - 1, vy - 1),
                colour(vx, vy - 1),
                colour(vx, vy),
                colour(vx - 1, vy),
            ];
            inner[corner_index(vx, vy)] = if cut { inner_colours(c) } else { c };
        }
    }
    let mut b = Builder {
        positions: Vec::new(),
        slots: Vec::new(),
        indices: Vec::new(),
        seen: HashMap::new(),
    };
    let mm = |t: i32| t * TILE_MM;
    let emit = |b: &mut Builder, p: [[i32; 3]; 3], slot: u8| {
        if slot == water {
            b.tri(p[0], p[1], p[2], bed);
            if let Some(l) = level {
                b.tri(
                    [p[0][0], l, p[0][2]],
                    [p[1][0], l, p[1][2]],
                    [p[2][0], l, p[2][2]],
                    water,
                );
            }
        } else {
            b.tri(p[0], p[1], p[2], slot);
        }
    };
    for ty in 0..n {
        for tx in 0..n {
            let own = colour(tx, ty);
            // Corners of the tile: NW, NE, SE, SW; the tile's position in
            // each corner's dual cell is SE, SW, NW, NE.
            let corners = [
                (tx, ty, 2usize),
                (tx + 1, ty, 3),
                (tx + 1, ty + 1, 0),
                (tx, ty + 1, 1),
            ];
            let pure = corners
                .iter()
                .all(|&(vx, vy, pos)| inner[corner_index(vx, vy)][pos] == own);
            let p = |vx: i32, vy: i32| [mm(vx), hc(vx, vy), mm(vy)];
            if pure || !cut {
                let (a, bb, c, d) = (p(tx, ty), p(tx + 1, ty), p(tx + 1, ty + 1), p(tx, ty + 1));
                emit(&mut b, [a, bb, c], own);
                emit(&mut b, [a, c, d], own);
                continue;
            }
            let centre = [
                mm(tx) + TILE_MM / 2,
                (hc(tx, ty) + hc(tx + 1, ty) + hc(tx + 1, ty + 1) + hc(tx, ty + 1)).div_euclid(4),
                mm(ty) + TILE_MM / 2,
            ];
            let mid = |ax: i32, ay: i32, bx: i32, by: i32| {
                [
                    (mm(ax) + mm(bx)) / 2,
                    (hc(ax, ay) + hc(bx, by)).div_euclid(2),
                    (mm(ay) + mm(by)) / 2,
                ]
            };
            let mn = mid(tx, ty, tx + 1, ty);
            let me = mid(tx + 1, ty, tx + 1, ty + 1);
            let ms = mid(tx, ty + 1, tx + 1, ty + 1);
            let mw = mid(tx, ty, tx, ty + 1);
            let quads = [
                (corners[0], mw, mn),
                (corners[1], mn, me),
                (corners[2], me, ms),
                (corners[3], ms, mw),
            ];
            for ((vx, vy, pos), ma, mb) in quads {
                emit(&mut b, [centre, ma, mb], own);
                emit(
                    &mut b,
                    [p(vx, vy), ma, mb],
                    inner[corner_index(vx, vy)][pos],
                );
            }
        }
    }
    TerrainMesh {
        region: bp.region,
        positions: b.positions,
        slots: b.slots,
        indices: b.indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifteen_partitions() {
        let (a, b, c, d) = (1u8, 2u8, 3u8, 4u8);
        assert_eq!(inner_colours([a, a, a, a]), [a; 4]);
        assert_eq!(inner_colours([b, a, a, a]), [a; 4]);
        assert_eq!(inner_colours([a, a, b, b]), [a, a, b, b]);
        assert_eq!(inner_colours([a, b, a, c]), [a; 4]);
        assert_eq!(inner_colours([a, b, c, d]), [a, b, c, d]);
    }
}
