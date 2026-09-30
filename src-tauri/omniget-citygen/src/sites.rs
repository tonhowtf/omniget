//! Site placement on a jittered grid (Minecraft `random_spread`) and trails
//! between sites (Veloren civ). Owner: agent A.
//!
//! Portions derived from Veloren (gitlab.com/veloren/veloren),
//! GPL-3.0-or-later, © Veloren contributors: the trail cost of
//! `world/src/civ/mod.rs` (`walk_in_all_dirs`, `find_path`: 1 + slope² +
//! water, a bridge as a cardinal jump over water, bounded A*) and the
//! `ProximityRequirements` idea, at commit b68cd6ea. Rewritten with integer
//! costs on our 4-tile macro nodes.

use crate::charter::TERRAIN_FEATURES;
use crate::hash::hash2;
use crate::terrain::basin::{LAKE, RIVER, SEA};
use crate::terrain::{Sampler, WaterKind};
use crate::types::*;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Site {
    pub id: String,
    /// town | village | farm | landmark
    pub kind: String,
    /// Global tile coordinates of the site center.
    pub center: [i32; 2],
    pub radius_tiles: i32,
}

/// Grid cell side, tiles (3 regions).
pub const SPACING: i32 = 192;
/// Minimum gap between candidates of neighbouring cells, tiles.
pub const SEPARATION: i32 = 64;
pub const SALT: u32 = 0x0051_74E5;
/// Placement attempts per cell before giving up.
const ATTEMPTS: u32 = 6;
/// Flat radii tried, largest first.
const RADII: [i32; 6] = [36, 28, 20, 14, 10, 6];

const KINDS: [&str; 4] = ["town", "village", "farm", "landmark"];

fn kind_for(radius: i32) -> usize {
    if radius >= 28 {
        0
    } else if radius >= 20 {
        1
    } else if radius >= 10 {
        2
    } else {
        3
    }
}

/// Largest flat radius around a tile, or 0 when the centre is unusable.
fn flat_radius(s: &mut Sampler, cx: i32, cy: i32, slope_pm: i32) -> i32 {
    let t = s.tile(cx, cy);
    if t.water != WaterKind::Dry || t.range > TILE_MM {
        return 0;
    }
    'r: for r in RADII {
        let step = (r / 4).max(2);
        let (mut lo, mut hi, mut wet, mut total) = (i32::MAX, i32::MIN, 0, 0);
        // Rise allowed across the diameter at the charter's flatten slope,
        // plus the fine detail noise.
        let limit = 2 * r * TILE_MM / 1000 * slope_pm.max(1) + 600;
        let mut y = -r;
        while y <= r {
            let mut x = -r;
            while x <= r {
                let h = s.corner(cx + x, cy + y);
                lo = lo.min(h);
                hi = hi.max(h);
                if hi - lo > limit {
                    continue 'r;
                }
                total += 1;
                if h < s.sea() || s.tile(cx + x, cy + y).water != WaterKind::Dry {
                    wet += 1;
                }
                x += step;
            }
            y += step;
        }
        if wet * 10 <= total {
            return r;
        }
    }
    0
}

/// The candidate of one grid cell before proximity rules. Pure in the cell.
fn candidate(s: &mut Sampler, c: &Charter, cx: i32, cy: i32) -> Option<(Site, usize)> {
    let span = (SPACING - SEPARATION) as u32;
    for n in 0..ATTEMPTS {
        let salt = SALT.wrapping_add(n * 7919);
        let ox = (hash2(s.p.seed, cx, cy, salt) % span) as i32;
        let oy = (hash2(s.p.seed, cx, cy, salt ^ 0xFFFF) % span) as i32;
        let (gx, gy) = (cx * SPACING + ox, cy * SPACING + oy);
        let r = flat_radius(s, gx, gy, c.relief.city_flatten_slope as i32);
        if r > 0 {
            let k = kind_for(r);
            let site = Site {
                id: format!("site:{}:{}", cx, cy),
                kind: KINDS[k].into(),
                center: [gx, gy],
                radius_tiles: r,
            };
            return Some((site, k));
        }
    }
    None
}

fn dist2(a: [i32; 2], b: [i32; 2]) -> i64 {
    let (dx, dy) = ((a[0] - b[0]) as i64, (a[1] - b[1]) as i64);
    dx * dx + dy * dy
}

/// Nearest distance (tiles, squared) from `p` to a node with any of `flags`,
/// searched up to `max` tiles. `None` = not found.
fn nearest_feature(s: &mut Sampler, p: [i32; 2], flags: u8, coast: bool, max: i32) -> Option<i64> {
    let (nx, ny) = (p[0].div_euclid(4), p[1].div_euclid(4));
    let rn = (max / 4).clamp(1, 256);
    let mut best: Option<i64> = None;
    for dy in -rn..=rn {
        for dx in -rn..=rn {
            let d = ((dx * 4) as i64).pow(2) + ((dy * 4) as i64).pow(2);
            if d > (max as i64).pow(2) || best.is_some_and(|b| d >= b) {
                continue;
            }
            let n = s.node(nx + dx, ny + dy);
            let hit = if coast {
                n.flags & crate::terrain::basin::COAST != 0
            } else {
                n.flags & flags != 0
            };
            if hit {
                best = Some(d);
            }
        }
    }
    best
}

fn feature_flags(near: &str) -> (u8, bool) {
    match near {
        "river" => (RIVER, false),
        "lake" => (LAKE, false),
        "sea" => (SEA, false),
        "coast" => (SEA, true),
        _ => (RIVER | LAKE | SEA, false),
    }
}

/// Does `site` (as kind `kind`) satisfy every proximity rule for that kind?
fn rules_ok(
    s: &mut Sampler,
    c: &Charter,
    site: &Site,
    kind: &str,
    others: &[(Site, usize)],
) -> bool {
    for rule in c.proximity.iter().filter(|r| r.kind == kind) {
        let Some(near) = rule.near.as_deref() else {
            continue;
        };
        let lo = rule.min_tiles.unwrap_or(0).max(0) as i64;
        let hi = rule.max_tiles.map(|m| m.clamp(0, 4096));
        let d: Option<i64> = if TERRAIN_FEATURES.contains(&near) {
            let (flags, coast) = feature_flags(near);
            let reach = hi.unwrap_or(0).max(lo as i32).max(4);
            nearest_feature(s, site.center, flags, coast, reach)
        } else {
            others
                .iter()
                .filter(|(o, k)| o.id != site.id && KINDS[*k] == near)
                .map(|(o, _)| dist2(o.center, site.center))
                .min()
        };
        match (d, hi) {
            (Some(d), Some(h)) if d > (h as i64).pow(2) => return false,
            (None, Some(_)) => return false,
            (Some(d), _) if d < lo * lo => return false,
            _ => {}
        }
    }
    true
}

fn cells_for(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for cy in y0.div_euclid(SPACING)..=(y1 - 1).div_euclid(SPACING) {
        for cx in x0.div_euclid(SPACING)..=(x1 - 1).div_euclid(SPACING) {
            v.push((cx, cy));
        }
    }
    v
}

/// Sites whose grid cell overlaps the square of `radius` regions around `region`.
pub fn sites_near(c: &Charter, region: [i32; 2], radius: i32) -> Vec<Site> {
    let radius = radius.clamp(0, 16);
    let mut s = Sampler::new(c, false);
    let x0 = (region[0] - radius) * REGION_TILES;
    let y0 = (region[1] - radius) * REGION_TILES;
    let x1 = (region[0] + radius + 1) * REGION_TILES;
    let y1 = (region[1] + radius + 1) * REGION_TILES;
    let cells = cells_for(x0, y0, x1, y1);
    // Rules between sites need the neighbours' candidates, out to the
    // largest max_tiles/min_tiles of a site-to-site rule.
    let reach = c
        .proximity
        .iter()
        .filter(|r| r.near.as_deref().is_some_and(|n| KINDS.contains(&n)))
        .map(|r| {
            r.max_tiles
                .unwrap_or(0)
                .max(r.min_tiles.unwrap_or(0))
                .clamp(0, 512)
        })
        .max()
        .unwrap_or(0);
    let mut cand: Vec<((i32, i32), (Site, usize))> = Vec::new();
    for (cx, cy) in cells_for(x0 - reach, y0 - reach, x1 + reach, y1 + reach) {
        if let Some(x) = candidate(&mut s, c, cx, cy) {
            cand.push(((cx, cy), x));
        }
    }
    let all: Vec<(Site, usize)> = cand.iter().map(|x| x.1.clone()).collect();
    let mut out = Vec::new();
    for (cell, (site, k0)) in cand.iter() {
        if !cells.contains(cell) {
            continue;
        }
        // Demote town → village → farm → landmark until the rules hold.
        #[allow(clippy::needless_range_loop)]
        for k in *k0..KINDS.len() {
            if rules_ok(&mut s, c, site, KINDS[k], &all) {
                let mut site = site.clone();
                site.kind = KINDS[k].into();
                out.push(site);
                break;
            }
        }
    }
    out
}

/// Trail between two sites: A* over 4-tile macro nodes with the Veloren cost
/// (integer: 10 per step, 14 diagonal, + slope², water impassable), bridges
/// as cardinal jumps of 2 to 6 nodes over water, at most 200 000 expansions.
/// Returns global tiles (8-connected), empty if no trail was found.
pub fn trail_between(c: &Charter, a: &Site, b: &Site) -> Vec<[i32; 2]> {
    let mut s = Sampler::new(c, false);
    let start = (a.center[0].div_euclid(4), a.center[1].div_euclid(4));
    let goal = (b.center[0].div_euclid(4), b.center[1].div_euclid(4));
    let margin = 64;
    let (minx, maxx) = (start.0.min(goal.0) - margin, start.0.max(goal.0) + margin);
    let (miny, maxy) = (start.1.min(goal.1) - margin, start.1.max(goal.1) + margin);
    let h = |p: (i32, i32)| -> i64 {
        let (dx, dy) = (((p.0 - goal.0) as i64).abs(), ((p.1 - goal.1) as i64).abs());
        10 * dx.max(dy) + 4 * dx.min(dy)
    };
    let mut nodes: HashMap<(i32, i32), crate::terrain::NodeInfo> = HashMap::new();
    let mut info =
        |s: &mut Sampler, p: (i32, i32)| *nodes.entry(p).or_insert_with(|| s.node(p.0, p.1));
    let mut g: HashMap<(i32, i32), i64> = HashMap::new();
    let mut came: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut open = BinaryHeap::new();
    g.insert(start, 0);
    open.push(Reverse((h(start), start.1, start.0)));
    let mut expansions = 0u32;
    let mut found = false;
    while let Some(Reverse((_, y, x))) = open.pop() {
        let cur = (x, y);
        if cur == goal {
            found = true;
            break;
        }
        expansions += 1;
        if expansions > 200_000 {
            break;
        }
        let gc = g[&cur];
        let here = info(&mut s, cur);
        for (k, (dx, dy)) in crate::terrain::basin::D8.iter().enumerate() {
            let nb = (x + dx, y + dy);
            if nb.0 < minx || nb.0 > maxx || nb.1 < miny || nb.1 > maxy {
                continue;
            }
            let ni = info(&mut s, nb);
            let mut step: Option<((i32, i32), i64)> = None;
            if !ni.wet() || nb == goal {
                let dh = ((ni.alt - here.alt) as i64).abs() / 400;
                let base = if k < 4 { 10 } else { 14 };
                step = Some((nb, base + dh * dh));
            } else if k < 4 {
                // Bridge: jump over the water to the first dry node.
                for len in 2..=6 {
                    let far = (x + dx * len, y + dy * len);
                    let fi = info(&mut s, far);
                    if !fi.wet() {
                        step = Some((far, 60 + 20 * len as i64));
                        break;
                    }
                }
            }
            if let Some((to, cost)) = step {
                let ng = gc + cost;
                if g.get(&to).is_none_or(|&old| ng < old) {
                    g.insert(to, ng);
                    came.insert(to, cur);
                    open.push(Reverse((ng + h(to), to.1, to.0)));
                }
            }
        }
    }
    if !found {
        return Vec::new();
    }
    let mut chain = vec![goal];
    let mut cur = goal;
    while let Some(&p) = came.get(&cur) {
        chain.push(p);
        cur = p;
    }
    chain.reverse();
    // Expand node steps (and bridge jumps) into 8-connected tiles.
    let mut tiles: Vec<[i32; 2]> = Vec::new();
    for w in chain.windows(2) {
        let (a, b) = ((w[0].0 * 4, w[0].1 * 4), (w[1].0 * 4, w[1].1 * 4));
        let n = (b.0 - a.0).abs().max((b.1 - a.1).abs());
        for i in 0..n {
            let t = [a.0 + (b.0 - a.0) * i / n, a.1 + (b.1 - a.1) * i / n];
            if tiles.last() != Some(&t) {
                tiles.push(t);
            }
        }
    }
    if let Some(&l) = chain.last() {
        tiles.push([l.0 * 4, l.1 * 4]);
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charter::example;

    #[test]
    fn sites_are_deterministic() {
        let c = example("t", "sites-1");
        let a = sites_near(&c, [0, 0], 2);
        let b = sites_near(&c, [0, 0], 2);
        assert_eq!(a, b);
        assert!(
            !a.is_empty(),
            "the origin is land: expected at least one site"
        );
        // A site found from another query window is the same site.
        let far = sites_near(&c, [1, 1], 3);
        for s in &a {
            if let Some(o) = far.iter().find(|o| o.id == s.id) {
                assert_eq!(o, s);
            }
        }
    }
}
