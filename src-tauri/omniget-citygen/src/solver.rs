//! Program + charter → blueprint. Offers only valid options. Owner: agent B.
//!
//! Pipeline (estudo 09 §1 and §6, port of `prototipos/cidade-programa/city.mjs`
//! with the site idea of Veloren: lots are born against a street and face it):
//! 1. natural surface and corner heights from [`crate::terrain`];
//! 2. program water, streets rasterized with width (sidewalks on the outer rows
//!    of streets 3+ wide), bridges where a street crosses listed water,
//!    landmarks as plazas, explicit lots verbatim;
//! 3. blocks = 4-connected ground of one zone; full rectangular blocks are
//!    split recursively (40..60 % by sub-seed) into lots whose id is the split
//!    path (`q0_13/bbb`); each lot faces the side with most street contact;
//! 4. buildings: explicit specs first (`auto:near`, `auto:zone`, exact id),
//!    then `fill` by sub-seed per lot;
//! 5. the grammar gives the massing; terrain is graded under streets and
//!    flattened under lots; props are scattered on a jittered grid.
//!
//! `lots.min_buildable = 0` turns off the offer filters (landmark zone, lot
//! side, frontage, relief). It reproduces the prototype's first solver and
//! exists for fixtures and diagnosis only.

use crate::grammar::{self, Frame};
use crate::hash::{canonical_hash, pick, pick_u64};
use crate::program::*;
use crate::terrain;
use crate::types::*;
use std::collections::BTreeSet;

const N: i32 = REGION_TILES;
/// Street grade: at most 1 level (3 m) per 8 tiles, per tile edge.
pub const STREET_STEP_MM: i32 = LEVEL_MM / 8;
/// Lots whose natural relief exceeds this are not offered (R14).
pub const MAX_LOT_RELIEF_MM: i32 = 2 * LEVEL_MM;

fn inside(x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && x < N && y < N
}

fn in_rect(x: i32, y: i32, r: [i32; 4]) -> bool {
    x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]
}

fn is_ground(s: Surface) -> bool {
    matches!(
        s,
        Surface::Grass | Surface::Forest | Surface::Sand | Surface::Field
    )
}

/// First zone whose rect holds the tile.
pub fn zone_at(p: &CityProgram, x: i32, y: i32) -> Option<&Zone> {
    p.zones.iter().find(|z| in_rect(x, y, z.rect))
}

fn rect_center(r: [i32; 4]) -> [i32; 2] {
    [r[0] + r[2] / 2, r[1] + r[3] / 2]
}

#[derive(Clone, Debug)]
pub(crate) struct LotInfo {
    pub lot: Lot,
    pub has_front: bool,
    /// Tiles of street/plaza contact on the front side.
    pub frontage: i32,
    pub explicit: bool,
    pub fixed_door: Option<[i32; 2]>,
    /// Natural height range under the lot, mm.
    pub relief_mm: i32,
}

pub(crate) struct Layout {
    pub seed: String,
    pub surface: Vec<Surface>,
    pub heights: Vec<i32>,
    pub streets: Vec<StreetRef>,
    pub lots: Vec<LotInfo>,
    pub blocks: usize,
}

/// Tiles of a street polyline, with a flag for the outer (sidewalk) rows.
pub fn rasterize_street(s: &Street) -> Vec<([i32; 2], bool)> {
    let lo = -((s.width.max(1) - 1) >> 1);
    let hi = lo + s.width.max(1) - 1;
    let sidewalks = s.width >= 3;
    let mut out = Vec::new();
    let mut put = |x: i32, y: i32, across_x: bool| {
        for o in lo..=hi {
            let (tx, ty) = if across_x { (x + o, y) } else { (x, y + o) };
            out.push(([tx, ty], sidewalks && (o == lo || o == hi)));
        }
    };
    if s.path.len() == 1 {
        put(s.path[0][0], s.path[0][1], false);
    }
    for seg in s.path.windows(2) {
        let [x0, y0] = seg[0];
        let [x1, y1] = seg[1];
        let (ax, ay) = ((x1 - x0).abs(), (y1 - y0).abs());
        let (sx, sy) = ((x1 - x0).signum(), (y1 - y0).signum());
        // The width spreads across the dominant direction.
        let across_x = ay > ax;
        // Bresenham, 4-connected: a diagonal step also paints the corner tile.
        let (mut x, mut y) = (x0, y0);
        let mut err = ax - ay;
        put(x, y, across_x);
        while (x, y) != (x1, y1) {
            let e2 = 2 * err;
            let (mut nx, mut ny) = (x, y);
            if e2 > -ay {
                err -= ay;
                nx += sx;
            }
            if e2 < ax {
                err += ax;
                ny += sy;
            }
            if nx != x && ny != y {
                put(nx, y, across_x);
            }
            x = nx;
            y = ny;
            put(x, y, across_x);
        }
    }
    out
}

fn build_layout(c: &Charter, p: &CityProgram) -> Layout {
    let cells = (N * N) as usize;
    let mut surface = terrain::region_natural_surface(c, p.region);
    if surface.len() != cells {
        surface = vec![Surface::Grass; cells];
    }
    let mut heights = terrain::region_corner_heights(c, p.region);
    if heights.len() != ((N + 1) * (N + 1)) as usize {
        heights = vec![0; ((N + 1) * (N + 1)) as usize];
    }
    let natural_heights = heights.clone();

    // Water of the program.
    let mut water_of = vec![-1i32; cells];
    for (wi, w) in p.water.iter().enumerate() {
        for y in w.rect[1]..w.rect[1] + w.rect[3] {
            for x in w.rect[0]..w.rect[0] + w.rect[2] {
                if inside(x, y) {
                    surface[tile_index(x, y)] = Surface::Water;
                    water_of[tile_index(x, y)] = wi as i32;
                }
            }
        }
    }

    // Streets, sidewalks and bridges.
    let mut street_of = vec![-1i32; cells];
    let mut streets = Vec::new();
    for (si, s) in p.streets.iter().enumerate() {
        let mut tiles = BTreeSet::new();
        for ([x, y], edge) in rasterize_street(s) {
            if !inside(x, y) {
                continue;
            }
            let i = tile_index(x, y);
            let cur = surface[i];
            surface[i] = match cur {
                Surface::Water => {
                    let listed = water_of[i] < 0
                        || p.water[water_of[i] as usize]
                            .bridges
                            .iter()
                            .any(|b| b == &s.id);
                    if listed {
                        Surface::Bridge
                    } else {
                        Surface::Water
                    }
                }
                Surface::Bridge => Surface::Bridge,
                Surface::Street => Surface::Street,
                _ if edge => Surface::Sidewalk,
                _ => Surface::Street,
            };
            street_of[i] = si as i32;
            tiles.insert((y, x));
        }
        streets.push(StreetRef {
            id: s.id.clone(),
            class: s.class.clone(),
            width: s.width,
            tiles: tiles.into_iter().map(|(y, x)| [x, y]).collect(),
        });
    }

    // Landmarks become plazas where the ground is free.
    for l in &p.landmarks {
        for y in l.rect[1]..l.rect[1] + l.rect[3] {
            for x in l.rect[0]..l.rect[0] + l.rect[2] {
                if inside(x, y) {
                    let i = tile_index(x, y);
                    if is_ground(surface[i]) || surface[i] == Surface::Rock {
                        surface[i] = Surface::Plaza;
                    }
                }
            }
        }
    }

    let relief = |r: [i32; 4]| {
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for y in r[1]..=r[1] + r[3] {
            for x in r[0]..=r[0] + r[2] {
                if x >= 0 && y >= 0 && x <= N && y <= N {
                    let h = natural_heights[corner_index(x, y)];
                    lo = lo.min(h);
                    hi = hi.max(h);
                }
            }
        }
        if lo > hi {
            0
        } else {
            hi - lo
        }
    };

    // Explicit lots, verbatim.
    let mut reserved = vec![false; cells];
    let mut lots: Vec<LotInfo> = Vec::new();
    for el in &p.lots.explicit {
        for y in el.rect[1]..el.rect[1] + el.rect[3] {
            for x in el.rect[0]..el.rect[0] + el.rect[2] {
                if inside(x, y) {
                    reserved[tile_index(x, y)] = true;
                }
            }
        }
        let zone = el.zone.clone().or_else(|| {
            let [cx, cy] = rect_center(el.rect);
            zone_at(p, cx, cy).map(|z| z.id.clone())
        });
        lots.push(LotInfo {
            lot: Lot {
                id: el.id.clone(),
                rect: el.rect,
                zone: zone.unwrap_or_default(),
                front: el.front,
                building: None,
                owner_ref: el.owner_ref.clone(),
            },
            has_front: true,
            frontage: 0,
            explicit: true,
            fixed_door: el.door,
            relief_mm: relief(el.rect),
        });
    }

    // Blocks: 4-connected ground of a single zone.
    let zone_idx = |x: i32, y: i32| p.zones.iter().position(|z| in_rect(x, y, z.rect));
    let mut seen = vec![false; cells];
    #[allow(clippy::type_complexity)]
    let mut blocks: Vec<([i32; 4], Vec<(i32, i32)>, String)> = Vec::new();
    for y in 0..N {
        for x in 0..N {
            let i = tile_index(x, y);
            if seen[i] || reserved[i] || !is_ground(surface[i]) {
                continue;
            }
            let Some(z) = zone_idx(x, y) else { continue };
            let mut stack = vec![(x, y)];
            seen[i] = true;
            let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
            let mut members = Vec::new();
            while let Some((cx, cy)) = stack.pop() {
                members.push((cx, cy));
                x0 = x0.min(cx);
                y0 = y0.min(cy);
                x1 = x1.max(cx);
                y1 = y1.max(cy);
                for (nx, ny) in [(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)] {
                    if !inside(nx, ny) {
                        continue;
                    }
                    let ni = tile_index(nx, ny);
                    if !seen[ni]
                        && !reserved[ni]
                        && is_ground(surface[ni])
                        && zone_idx(nx, ny) == Some(z)
                    {
                        seen[ni] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            let rect = [x0, y0, x1 - x0 + 1, y1 - y0 + 1];
            blocks.push((rect, members, p.zones[z].id.clone()));
        }
    }

    // Lots: recursive split along the longer axis.
    let seed = c.seed.clone();
    let rules = &p.lots;
    let min_part = rules.min_front.min(rules.min_depth).max(1);
    let mut auto_lots: Vec<(String, [i32; 4], String)> = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn split(
        r: [i32; 4],
        bid: &str,
        path: &str,
        zone: &str,
        seed: &str,
        min_part: i32,
        max_area: i32,
        out: &mut Vec<(String, [i32; 4], String)>,
    ) {
        let [x, y, w, h] = r;
        let axis_x = w >= h;
        let len = if axis_x { w } else { h };
        if w * h <= max_area || len < 2 * min_part {
            let p = if path.is_empty() { "r" } else { path };
            out.push((format!("{bid}/{p}"), r, zone.to_string()));
            return;
        }
        let pct = 40 + pick(seed, "lot", &format!("{bid}/{path}"), 0, 21) as i32;
        let cut = (len * pct / 100).clamp(min_part, len - min_part);
        let (a, b) = if axis_x {
            ([x, y, cut, h], [x + cut, y, w - cut, h])
        } else {
            ([x, y, w, cut], [x, y + cut, w, h - cut])
        };
        split(
            a,
            bid,
            &format!("{path}a"),
            zone,
            seed,
            min_part,
            max_area,
            out,
        );
        split(
            b,
            bid,
            &format!("{path}b"),
            zone,
            seed,
            min_part,
            max_area,
            out,
        );
    }
    if rules.auto {
        for (rect, members, zone) in &blocks {
            let bid = format!("q{}_{}", rect[0], rect[1]);
            if members.len() as i32 == rect[2] * rect[3] {
                split(
                    *rect,
                    &bid,
                    "",
                    zone,
                    &seed,
                    min_part,
                    rules.max_area.max(1),
                    &mut auto_lots,
                );
            } else if strict(p) {
                // Irregular block (around a plaza, a bend): cover it with
                // greedy maximal rectangles and split each, so the ground
                // along its streets is not wasted. The prototype skipped them.
                for (n, r) in cover_rects(*rect, members).into_iter().enumerate() {
                    if r[2].min(r[3]) >= min_part {
                        split(
                            r,
                            &format!("{bid}r{n}"),
                            "",
                            zone,
                            &seed,
                            min_part,
                            rules.max_area.max(1),
                            &mut auto_lots,
                        );
                    }
                }
            }
        }
    }

    let public_at = |x: i32, y: i32| inside(x, y) && surface[tile_index(x, y)].public();
    let contact = |r: [i32; 4], d: Dir| -> i32 {
        let [x, y, w, h] = r;
        match d {
            Dir::N => (0..w).filter(|i| public_at(x + i, y - 1)).count() as i32,
            Dir::S => (0..w).filter(|i| public_at(x + i, y + h)).count() as i32,
            Dir::W => (0..h).filter(|j| public_at(x - 1, y + j)).count() as i32,
            Dir::E => (0..h).filter(|j| public_at(x + w, y + j)).count() as i32,
        }
    };
    for li in lots.iter_mut() {
        li.frontage = contact(li.lot.rect, li.lot.front);
    }
    for (id, rect, zone) in auto_lots {
        // Ties broken E, N, S, W (the prototype's order).
        let mut best = (Dir::N, 0);
        for d in [Dir::E, Dir::N, Dir::S, Dir::W] {
            let n = contact(rect, d);
            if n > best.1 {
                best = (d, n);
            }
        }
        lots.push(LotInfo {
            lot: Lot {
                id,
                rect,
                zone,
                front: best.0,
                building: None,
                owner_ref: None,
            },
            has_front: best.1 > 0,
            frontage: best.1,
            explicit: false,
            fixed_door: None,
            relief_mm: relief(rect),
        });
    }

    // Surface of lots and yards.
    for li in &lots {
        let r = li.lot.rect;
        for y in r[1]..r[1] + r[3] {
            for x in r[0]..r[0] + r[2] {
                if inside(x, y) {
                    let i = tile_index(x, y);
                    if is_ground(surface[i]) || surface[i] == Surface::Rock {
                        surface[i] = if li.has_front {
                            Surface::Lot
                        } else {
                            Surface::Yard
                        };
                    }
                }
            }
        }
    }

    Layout {
        seed,
        surface,
        heights,
        streets,
        lots,
        blocks: blocks.len(),
    }
}

/// Greedy cover of an irregular block by rectangles in scan order: the first
/// free cell grows right, then down while the whole row fits.
fn cover_rects(bbox: [i32; 4], members: &[(i32, i32)]) -> Vec<[i32; 4]> {
    let [bx, by, bw, bh] = bbox;
    let at = |x: i32, y: i32| ((y - by) * bw + (x - bx)) as usize;
    let mut free = vec![false; (bw * bh) as usize];
    for &(x, y) in members {
        free[at(x, y)] = true;
    }
    let mut out = Vec::new();
    for y in by..by + bh {
        for x in bx..bx + bw {
            if !free[at(x, y)] {
                continue;
            }
            let mut x1 = x;
            while x1 + 1 < bx + bw && free[at(x1 + 1, y)] {
                x1 += 1;
            }
            let mut y1 = y;
            while y1 + 1 < by + bh && (x..=x1).all(|xx| free[at(xx, y1 + 1)]) {
                y1 += 1;
            }
            for yy in y..=y1 {
                for xx in x..=x1 {
                    free[at(xx, yy)] = false;
                }
            }
            out.push([x, y, x1 - x + 1, y1 - y + 1]);
        }
    }
    out
}

fn strict(p: &CityProgram) -> bool {
    p.lots.min_buildable > 0
}

/// Would the solver offer this lot to a new building?
fn offerable(li: &LotInfo, p: &CityProgram) -> bool {
    if !li.has_front {
        return false;
    }
    if li.explicit && li.lot.owner_ref.is_some() {
        return false;
    }
    if !strict(p) {
        return true;
    }
    let r = li.lot.rect;
    let zone = p.zones.iter().find(|z| z.id == li.lot.zone);
    let b = grammar::buildable_rect(
        &Frame::new(r, li.lot.front),
        &grammar::effective_setback(zone),
    );
    li.frontage >= 3
        && r[2].min(r[3]) >= p.lots.min_buildable
        && li.relief_mm <= MAX_LOT_RELIEF_MM
        && b[2] >= 3
        && b[3] >= 3
}

/// One building bound to a lot.
struct Assigned {
    spec: BuildingSpec,
    lot: usize,
    toward: Option<[i32; 2]>,
}

struct Plan {
    layout: Layout,
    assigned: Vec<Assigned>,
    errors: Vec<Violation>,
}

fn err(code: &str, id: &str, measure: &str, message: String) -> Violation {
    Violation {
        code: code.into(),
        id: id.into(),
        measure: measure.into(),
        message,
        severity: Severity::Error,
    }
}

fn dist2(r: [i32; 4], t: [i32; 4]) -> i64 {
    let (cx, cy) = (2 * r[0] + r[2], 2 * r[1] + r[3]);
    let (tx, ty) = (2 * t[0] + t[2], 2 * t[1] + t[3]);
    let (dx, dy) = ((cx - tx) as i64, (cy - ty) as i64);
    dx * dx + dy * dy
}

fn plan(c: &Charter, p: &CityProgram, with_fill: bool) -> Plan {
    let layout = build_layout(c, p);
    let seed = layout.seed.clone();
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut assigned = Vec::new();
    let mut errors = Vec::new();
    let st = strict(p);

    for b in &p.buildings {
        let free = |i: &usize| !used.contains(i);
        let mut toward = None;
        let found: Result<usize, Violation> = if let Some(l) = b.lot.strip_prefix("auto:near:") {
            match p.landmarks.iter().find(|x| x.id == l) {
                None => Err(err(
                    "P6",
                    &b.id,
                    &b.lot,
                    format!("landmark `{l}` does not exist"),
                )),
                Some(lm) => {
                    toward = Some(rect_center(lm.rect));
                    let [cx, cy] = rect_center(lm.rect);
                    let lz = zone_at(p, cx, cy).map(|z| z.id.clone());
                    let mut cand: Vec<usize> = (0..layout.lots.len())
                        .filter(|i| free(i) && offerable(&layout.lots[*i], p))
                        .filter(|i| !st || lz.as_deref() == Some(layout.lots[*i].lot.zone.as_str()))
                        .collect();
                    cand.sort_by(|&a, &bb| {
                        let (la, lb) = (&layout.lots[a].lot, &layout.lots[bb].lot);
                        dist2(la.rect, lm.rect)
                            .cmp(&dist2(lb.rect, lm.rect))
                            .then((lb.rect[2] * lb.rect[3]).cmp(&(la.rect[2] * la.rect[3])))
                            .then(la.id.cmp(&lb.id))
                    });
                    cand.first().copied().ok_or_else(|| {
                        err("P9", &b.id, &b.lot, format!("no free buildable lot near `{l}` in zone `{}`; add a street next to the landmark or free a lot", lz.unwrap_or_default()))
                    })
                }
            }
        } else if let Some(z) = b.lot.strip_prefix("auto:zone:") {
            let cand: Vec<usize> = (0..layout.lots.len())
                .filter(|i| {
                    free(i) && offerable(&layout.lots[*i], p) && layout.lots[*i].lot.zone == z
                })
                .collect();
            if cand.is_empty() {
                Err(err("P9", &b.id, &b.lot, format!("no free buildable lot left in zone `{z}`; add a street through the zone or use another zone")))
            } else {
                Ok(cand[pick(&seed, "lotpick", &b.id, 0, cand.len() as u64) as usize])
            }
        } else {
            match layout.lots.iter().position(|l| l.lot.id == b.lot) {
                None => Err(err("P2", &b.id, &b.lot, format!("lot `{}` does not exist in this region; ask for the free lots or use auto:zone", b.lot))),
                Some(i) if used.contains(&i) => Err(err("P9", &b.id, &b.lot, format!("lot `{}` is already taken by another building", b.lot))),
                Some(i) if !layout.lots[i].has_front => Err(err("P9", &b.id, &b.lot, format!("lot `{}` has no street frontage (it is a yard)", b.lot))),
                Some(i) => Ok(i),
            }
        };
        match found {
            Ok(i) => {
                used.insert(i);
                assigned.push(Assigned {
                    spec: b.clone(),
                    lot: i,
                    toward,
                });
            }
            Err(v) => errors.push(v),
        }
    }

    if with_fill {
        for (i, li) in layout.lots.iter().enumerate() {
            if used.contains(&i) || li.explicit || !offerable(li, p) {
                continue;
            }
            let Some(rule) = p.fill.iter().find(|f| f.zone == li.lot.zone) else {
                continue;
            };
            let Some(z) = p.zones.iter().find(|z| z.id == li.lot.zone) else {
                continue;
            };
            let id = &li.lot.id;
            if pick(&seed, "fill", id, 0, 1000) >= z.density as u64 {
                continue;
            }
            let (lo, hi) = (z.floors[0].max(1), z.floors[1].max(z.floors[0]).max(1));
            let floors = lo + pick(&seed, "floors", id, 0, (hi - lo + 1) as u64) as u8;
            let choose = |list: &Vec<String>, dom: &str, dflt: &str| {
                if list.is_empty() {
                    dflt.to_string()
                } else {
                    list[pick(&seed, dom, id, 0, list.len() as u64) as usize].clone()
                }
            };
            let intent = Intent {
                use_: rule.use_.clone(),
                floors: Some(floors),
                shape: Some(choose(&rule.shape, "shape", "rect")),
                roof: Some(choose(&rule.roof, "roof", "duas-aguas")),
                ..Default::default()
            };
            used.insert(i);
            assigned.push(Assigned {
                spec: BuildingSpec {
                    id: format!("b:{id}"),
                    lot: id.clone(),
                    owner_ref: None,
                    intent,
                    typology: None,
                    progress: 100,
                },
                lot: i,
                toward: None,
            });
        }
    }
    Plan {
        layout,
        assigned,
        errors,
    }
}

/// Program + charter → blueprint. `Err` holds structural problems (P-codes)
/// that keep the solver from running; the building code runs afterwards in
/// [`crate::checker::check`].
pub fn solve(c: &Charter, p: &CityProgram) -> Result<Blueprint, Vec<Violation>> {
    let structural = validate(p);
    if !structural.is_empty() {
        return Err(structural);
    }
    let Plan {
        layout,
        assigned,
        errors,
    } = plan(c, p, true);
    if !errors.is_empty() {
        return Err(errors);
    }
    let Layout {
        seed,
        mut surface,
        mut heights,
        streets,
        mut lots,
        ..
    } = layout;

    // Massing.
    let mut buildings: Vec<Building> = Vec::new();
    let mut balcony: Vec<bool> = Vec::new();
    for a in &assigned {
        let li = &lots[a.lot];
        let zone = p.zones.iter().find(|z| z.id == li.lot.zone);
        let it = &a.spec.intent;
        let use_ = grammar::normalize_key(if it.use_.is_empty() { "casa" } else { &it.use_ });
        let floors = it
            .floors
            .unwrap_or_else(|| zone.map(|z| z.floors[0]).unwrap_or(1))
            .max(1);
        let palette = it
            .palette
            .clone()
            .or_else(|| zone.and_then(|z| z.palette.clone()))
            .unwrap_or_else(|| c.palette.clone());
        let style = it
            .style
            .clone()
            .or_else(|| zone.map(|z| z.style.clone()))
            .unwrap_or_else(|| c.style.clone());
        let mut lot = li.lot.clone();
        lot.owner_ref = lot.owner_ref.clone().or_else(|| a.spec.owner_ref.clone());
        let b = grammar::build(&grammar::Spec {
            id: &a.spec.id,
            lot: &lot,
            zone,
            use_,
            floors,
            shape: grammar::normalize_key(it.shape.as_deref().unwrap_or("rect")),
            roof: grammar::normalize_key(it.roof.as_deref().unwrap_or("duas-aguas")),
            style,
            palette,
            tower: it.tower,
            fixed_door: li.fixed_door,
            tower_toward: a.toward,
            typology: a.spec.typology.clone(),
            progress: a.spec.progress,
            owner_ref: lot.owner_ref.clone(),
        });
        lots[a.lot].lot.building = Some(b.id.clone());
        lots[a.lot].lot.owner_ref = lot.owner_ref;
        buildings.push(b);
        balcony.push(it.balcony);
    }

    // Voxel owner per level for blind walls.
    let max_levels = buildings.iter().map(|b| b.levels.len()).max().unwrap_or(0);
    let mut occ = vec![vec![-1i32; (N * N) as usize]; max_levels];
    for (bi, b) in buildings.iter().enumerate() {
        for (k, cells) in b.levels.iter().enumerate() {
            for c in cells {
                if inside(c[0], c[1]) && occ[k][tile_index(c[0], c[1])] < 0 {
                    occ[k][tile_index(c[0], c[1])] = bi as i32;
                }
            }
        }
    }
    for bi in 0..buildings.len() {
        let other = |k: usize, x: i32, y: i32| {
            inside(x, y)
                && k < occ.len()
                && occ[k][tile_index(x, y)] >= 0
                && occ[k][tile_index(x, y)] != bi as i32
        };
        let public = |x: i32, y: i32| inside(x, y) && surface[tile_index(x, y)].public();
        let bays = grammar::facades(&buildings[bi], balcony[bi], &other, &public);
        buildings[bi].bays = bays;
    }

    // Terrain: plazas flat, streets graded, lots flattened.
    for l in &p.landmarks {
        flatten_tiles(&mut heights, &surface, l.rect, Surface::Plaza);
    }
    grade_streets(&mut heights, &surface);
    let mut claimed = vec![false; ((N + 1) * (N + 1)) as usize];
    let built: Vec<usize> = (0..lots.len())
        .filter(|i| lots[*i].lot.building.is_some() || lots[*i].explicit)
        .collect();
    for i in built {
        let li = &lots[i];
        let fp: Vec<[i32; 2]> = buildings
            .iter()
            .find(|b| Some(&b.id) == li.lot.building.as_ref())
            .map(|b| b.levels.first().cloned().unwrap_or_default())
            .unwrap_or_default();
        flatten_lot(&mut heights, &mut claimed, li.lot.rect, li.lot.front, &fp);
    }
    // Buildings standing on water or streets keep the surface: the checker reports it.
    let _ = &mut surface;

    // Props.
    let door_out: BTreeSet<(i32, i32)> = buildings
        .iter()
        .map(|b| (b.door.outside[0], b.door.outside[1]))
        .collect();
    let mut props = Vec::new();
    for (pi, ps) in p.props.iter().enumerate() {
        let mk = |x: i32, y: i32| Prop {
            kind: ps.kind.clone(),
            preset: ps.preset.clone(),
            cell: [x, y],
            seed: pick_u64(&seed, "prop", &format!("{pi}:{x}:{y}"), 0) as u32,
            anchors: prop_anchors(&ps.kind, &ps.preset, [x, y]),
        };
        for c in &ps.cells {
            props.push(mk(c[0], c[1]));
        }
        if let Some(sc) = &ps.scatter {
            let Some(z) = p.zones.iter().find(|z| z.id == sc.zone) else {
                continue;
            };
            let s = sc.spacing.max(1);
            let half = (s / 2).max(1);
            let accept = |x: i32, y: i32| -> bool {
                if !inside(x, y) || !in_rect(x, y, z.rect) || door_out.contains(&(x, y)) {
                    return false;
                }
                let i = tile_index(x, y);
                if !occ.is_empty() && occ[0][i] >= 0 {
                    return false;
                }
                match sc.on.as_str() {
                    "street" => surface[i] == Surface::Sidewalk,
                    "plaza" => surface[i] == Surface::Plaza,
                    _ => is_ground(surface[i]) || surface[i] == Surface::Yard,
                }
            };
            let mut gy = 0;
            while z.rect[1] + gy * s < z.rect[1] + z.rect[3] {
                let mut gx = 0;
                while z.rect[0] + gx * s < z.rect[0] + z.rect[2] {
                    let key = format!("{}:{}:{gx}:{gy}", ps.kind, ps.preset);
                    let x = z.rect[0]
                        + gx * s
                        + s / 4
                        + pick(&seed, "tree", &key, 0, half as u64) as i32;
                    let y = z.rect[1]
                        + gy * s
                        + s / 4
                        + pick(&seed, "tree", &key, 1, half as u64) as i32;
                    if accept(x, y) {
                        props.push(mk(x, y));
                    }
                    gx += 1;
                }
                gy += 1;
            }
        }
    }

    let places = p
        .landmarks
        .iter()
        .map(|l| Place {
            id: l.id.clone(),
            kind: l.kind.clone(),
            rect: l.rect,
        })
        .collect();
    let mut bp = Blueprint {
        schema: "omni.city.blueprint/v1".into(),
        city_id: p.city_id.clone(),
        region: p.region,
        generator_version: c.generator_version.clone(),
        program_hash: p.revision(),
        surface,
        corner_height_mm: heights,
        streets,
        lots: lots
            .into_iter()
            .filter(|l| l.has_front)
            .map(|l| l.lot)
            .collect(),
        buildings,
        props,
        places,
        hash: String::new(),
    };
    bp.hash = blueprint_hash(&bp);
    Ok(bp)
}

/// `sha256:<hex>` of the canonical blueprint without its own `hash` field.
pub fn blueprint_hash(bp: &Blueprint) -> String {
    let mut v = serde_json::to_value(bp).expect("serializable");
    if let Some(o) = v.as_object_mut() {
        o.remove("hash");
    }
    canonical_hash(&v)
}

fn prop_anchors(kind: &str, preset: &str, cell: [i32; 2]) -> Vec<Anchor> {
    let a = |action: &str, cap: u8| {
        vec![Anchor {
            action: action.into(),
            cell,
            level: 0,
            yaw: 0,
            capacity: cap,
        }]
    };
    match (kind, preset) {
        (_, "banco") | ("banco", _) | (_, "bench") => a("sit", 2),
        (_, "mesa") | ("mesa", _) => a("sit", 4),
        (_, "canteiro") | ("canteiro", _) => a("plant", 1),
        (_, "fonte") | ("fonte", _) | (_, "poco") => a("water", 1),
        _ => Vec::new(),
    }
}

/// Mean height of the given tiles' corners, then set them all to it.
fn flatten_tiles(h: &mut [i32], surface: &[Surface], r: [i32; 4], only: Surface) {
    let mut corners = BTreeSet::new();
    for y in r[1]..r[1] + r[3] {
        for x in r[0]..r[0] + r[2] {
            if inside(x, y) && surface[tile_index(x, y)] == only {
                for (cx, cy) in [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)] {
                    corners.insert(corner_index(cx, cy));
                }
            }
        }
    }
    if corners.is_empty() {
        return;
    }
    let sum: i64 = corners.iter().map(|&i| h[i] as i64).sum();
    let level = sum.div_euclid(corners.len() as i64) as i32;
    for i in corners {
        h[i] = level;
    }
}

/// Lower street corners until neighbours along streets differ by at most
/// [`STREET_STEP_MM`] (a cut, never a fill: the Lipschitz lower envelope).
fn grade_streets(h: &mut [i32], surface: &[Surface]) {
    let m = (N + 1) as usize;
    let mut on = vec![false; m * m];
    for y in 0..N {
        for x in 0..N {
            if matches!(
                surface[tile_index(x, y)],
                Surface::Street | Surface::Sidewalk | Surface::Bridge
            ) {
                for (cx, cy) in [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)] {
                    on[corner_index(cx, cy)] = true;
                }
            }
        }
    }
    for _ in 0..(4 * m) {
        let mut changed = false;
        let mut relax = |i: usize, j: usize, h: &mut [i32]| {
            if on[i] && on[j] && h[i] > h[j] + STREET_STEP_MM {
                h[i] = h[j] + STREET_STEP_MM;
                changed = true;
            }
        };
        for y in 0..=N {
            for x in 0..=N {
                let i = corner_index(x, y);
                if x > 0 {
                    relax(i, corner_index(x - 1, y), h);
                }
                if y > 0 {
                    relax(i, corner_index(x, y - 1), h);
                }
            }
        }
        for y in (0..=N).rev() {
            for x in (0..=N).rev() {
                let i = corner_index(x, y);
                if x < N {
                    relax(i, corner_index(x + 1, y), h);
                }
                if y < N {
                    relax(i, corner_index(x, y + 1), h);
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Flatten a lot at the height of its street edge (Veloren's `hard_alt`), so
/// the door meets the sidewalk. Corners already claimed by a neighbour's
/// footprint keep theirs, and a geminated footprint inherits it.
fn flatten_lot(
    h: &mut [i32],
    claimed: &mut [bool],
    r: [i32; 4],
    front: Dir,
    footprint: &[[i32; 2]],
) {
    let [x0, y0, w, hh] = r;
    let edge: Vec<(i32, i32)> = match front {
        Dir::N => (0..=w).map(|i| (x0 + i, y0)).collect(),
        Dir::S => (0..=w).map(|i| (x0 + i, y0 + hh)).collect(),
        Dir::W => (0..=hh).map(|j| (x0, y0 + j)).collect(),
        Dir::E => (0..=hh).map(|j| (x0 + w, y0 + j)).collect(),
    };
    let ok = |x: i32, y: i32| x >= 0 && y >= 0 && x <= N && y <= N;
    let mut fp_corners = Vec::new();
    for c in footprint {
        for (cx, cy) in [
            (c[0], c[1]),
            (c[0] + 1, c[1]),
            (c[0], c[1] + 1),
            (c[0] + 1, c[1] + 1),
        ] {
            if ok(cx, cy) {
                fp_corners.push(corner_index(cx, cy));
            }
        }
    }
    let inherited = fp_corners.iter().find(|&&i| claimed[i]).map(|&i| h[i]);
    let level = inherited.unwrap_or_else(|| {
        let e: Vec<i64> = edge
            .iter()
            .filter(|(x, y)| ok(*x, *y))
            .map(|(x, y)| h[corner_index(*x, *y)] as i64)
            .collect();
        if e.is_empty() {
            0
        } else {
            (e.iter().sum::<i64>().div_euclid(e.len() as i64)) as i32
        }
    });
    for y in y0..=y0 + hh {
        for x in x0..=x0 + w {
            if ok(x, y) && !claimed[corner_index(x, y)] {
                h[corner_index(x, y)] = level;
            }
        }
    }
    for i in fp_corners {
        claimed[i] = true;
    }
}

/// Lots that a new building could use right now (free, buildable, in zone).
/// `building` carries the fill building that would stand there today, if any;
/// explicit buildings claim lots before fill does.
pub fn free_lots(c: &Charter, p: &CityProgram, zone: Option<&str>) -> Vec<Lot> {
    let pl = plan(c, p, true);
    let explicit: BTreeSet<usize> = pl
        .assigned
        .iter()
        .filter(|a| !a.spec.id.starts_with("b:"))
        .map(|a| a.lot)
        .collect();
    pl.layout
        .lots
        .iter()
        .enumerate()
        .filter(|(i, li)| {
            !explicit.contains(i)
                && offerable(li, p)
                && zone.map(|z| li.lot.zone == z).unwrap_or(true)
        })
        .map(|(i, li)| {
            let mut l = li.lot.clone();
            l.building = pl
                .assigned
                .iter()
                .find(|a| a.lot == i)
                .map(|a| a.spec.id.clone());
            l
        })
        .collect()
}

/// Everything an LLM may name in a proposal for this region: zones,
/// landmarks, streets, free lots per zone and the valid enums.
pub fn options(c: &Charter, p: &CityProgram) -> serde_json::Value {
    use serde_json::json;
    let free = free_lots(c, p, None);
    let pl = plan(c, p, false);
    let zones: Vec<serde_json::Value> = p
        .zones
        .iter()
        .map(|z| {
            let lots: Vec<serde_json::Value> = free
                .iter()
                .filter(|l| l.zone == z.id)
                .map(|l| {
                    let fr = pl.layout.lots.iter().find(|x| x.lot.id == l.id).map(|x| x.frontage).unwrap_or(0);
                    json!({ "id": l.id, "rect": l.rect, "front": l.front, "frontage": fr, "fill_building": l.building })
                })
                .collect();
            json!({
                "id": z.id, "kind": z.kind, "rect": z.rect, "style": z.style, "floors": z.floors,
                "density": z.density, "setback": grammar::effective_setback(Some(z)).front_side_back(),
                "free_lots": lots,
            })
        })
        .collect();
    let mut palettes: Vec<String> = crate::checker::KIT_VILA_V1
        .palettes
        .iter()
        .map(|s| s.to_string())
        .collect();
    for pal in &c.palettes {
        if !palettes.contains(&pal.key) {
            palettes.push(pal.key.clone());
        }
    }
    let cat = &crate::checker::KIT_VILA_V1;
    json!({
        "region": p.region,
        "revision": p.revision(),
        "blocks": pl.layout.blocks,
        "zones": zones,
        "landmarks": p.landmarks.iter().map(|l| json!({ "id": l.id, "kind": l.kind, "rect": l.rect,
            "zone": zone_at(p, rect_center(l.rect)[0], rect_center(l.rect)[1]).map(|z| z.id.clone()) })).collect::<Vec<_>>(),
        "streets": p.streets.iter().map(|s| json!({ "id": s.id, "class": s.class, "width": s.width, "path": s.path })).collect::<Vec<_>>(),
        "buildings": p.buildings.iter().map(|b| json!({ "id": b.id, "lot": b.lot, "owned": b.owner_ref.is_some() })).collect::<Vec<_>>(),
        "lot_rules": p.lots,
        "enums": {
            "use": grammar::USES, "shape": grammar::SHAPES, "roof": grammar::ROOFS, "palette": palettes,
            "style": cat.styles, "zone_kind": cat.zone_kinds, "street_class": cat.street_classes,
            "landmark_kind": cat.landmark_kinds, "prop_preset": cat.presets,
            "flags": ["balcony", "awning", "tower", "chimney"],
            "lot": ["<lot id>", "auto:near:<landmark>", "auto:zone:<zone>"],
            "ops": ["add_zone", "add_street", "add_landmark", "add_water", "add_building", "add_props", "add_fill", "set_intent", "move", "set_progress", "remove"],
        },
        "units": { "tile_m": 2, "floor_m": 3, "region_tiles": REGION_TILES },
    })
}

trait SetbackJson {
    fn front_side_back(&self) -> [i32; 3];
}
impl SetbackJson for Setback {
    fn front_side_back(&self) -> [i32; 3] {
        [self.front, self.side, self.back]
    }
}

/// Ready program for founding without an LLM (the prototype's v2 region with
/// setbacks, a market, trees and benches). It passes the building code.
pub fn default_program(city_id: &str, region: [i32; 2]) -> CityProgram {
    let mut p: CityProgram = serde_json::from_str(include_str!("default_program.json"))
        .expect("default_program.json is a valid program");
    p.city_id = city_id.to_string();
    p.region = region;
    p
}

/// Text map of a blueprint for logs and LLM feedback: `.` ground, `=` street,
/// `-` sidewalk, `#` plaza, `~` water, `H` bridge, `,` lot, `:` yard, digit =
/// levels, `D` door.
pub fn ascii(bp: &Blueprint) -> String {
    let mut top = vec![0usize; (N * N) as usize];
    for b in &bp.buildings {
        for (k, cells) in b.levels.iter().enumerate() {
            for c in cells {
                if inside(c[0], c[1]) {
                    let i = tile_index(c[0], c[1]);
                    top[i] = top[i].max(k + 1);
                }
            }
        }
    }
    let doors: BTreeSet<(i32, i32)> = bp
        .buildings
        .iter()
        .map(|b| (b.door.cell[0], b.door.cell[1]))
        .collect();
    let mut s = String::with_capacity((N * (N + 1)) as usize);
    for y in 0..N {
        for x in 0..N {
            let i = tile_index(x, y);
            let ch = if doors.contains(&(x, y)) {
                'D'
            } else if top[i] > 0 {
                char::from_digit(top[i].min(9) as u32, 10).unwrap()
            } else {
                match bp.surface[i] {
                    Surface::Street => '=',
                    Surface::Sidewalk => '-',
                    Surface::Plaza => '#',
                    Surface::Water => '~',
                    Surface::Bridge => 'H',
                    Surface::Lot => ',',
                    Surface::Yard => ':',
                    Surface::Rock => '^',
                    Surface::Forest => 'f',
                    _ => '.',
                }
            };
            s.push(ch);
        }
        s.push('\n');
    }
    s
}
