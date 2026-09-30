//! Building code R1..R18 (estudo 09 §4). Owner: agent B.
//!
//! Every rule measures in integer tiles and levels (1 tile = 2 m, 1 level =
//! 3 m) and answers `[code, id, measure]` plus one short sentence an LLM can
//! act on. Errors block publication; warnings do not. The same function serves
//! LLMs, people and imports.
//!
//! Codes follow the study, not the prototype: the prototype's R4 (door) is R5
//! here, its R5 (network) is R6, R6 (stairs) is R7, R7 (support) is R8, R8
//! (height) is R9, R9 (bridge) is R10 and its R10? (gap) is R11.

use crate::grammar::{self, Frame};
use crate::program::CityProgram;
use crate::types::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const N: i32 = REGION_TILES;
/// R5: the door reaches a street within this many steps.
pub const MAX_DOOR_STEPS: i32 = 12;
/// R13: longest alley, tiles.
pub const MAX_BECO_LEN: i32 = 24;
/// R18 budget per 16×16 chunk.
pub const CHUNK_MAX_INSTANCES: u32 = 1200;
pub const CHUNK_MAX_TRIANGLES: u32 = 60_000;

/// Keys the kit of a catalog provides (R16).
pub struct Catalog {
    pub key: &'static str,
    pub uses: &'static [&'static str],
    pub shapes: &'static [&'static str],
    pub roofs: &'static [&'static str],
    pub palettes: &'static [&'static str],
    pub styles: &'static [&'static str],
    pub presets: &'static [&'static str],
    pub zone_kinds: &'static [&'static str],
    pub street_classes: &'static [&'static str],
    pub landmark_kinds: &'static [&'static str],
}

pub const KIT_VILA_V1: Catalog = Catalog {
    key: "kit-vila-v1",
    uses: grammar::USES,
    shapes: grammar::SHAPES,
    roofs: grammar::ROOFS,
    palettes: &[
        "cal-e-terracota",
        "azul-anil",
        "ocre-e-verde",
        "rosa-colonial",
        "branco-e-azul",
        "pedra-e-madeira",
        "verde-praia",
    ],
    styles: &["vila-colonial", "vila-praiana"],
    presets: &[
        "oak-estilizada",
        "pine-estilizada",
        "palmeira",
        "arbusto",
        "banco",
        "poste",
        "canteiro",
        "fonte",
        "mesa",
        "cerca",
    ],
    zone_kinds: &["civico", "residencial", "comercial", "rural", "parque"],
    street_classes: &["avenida", "rua", "beco", "trilha"],
    landmark_kinds: &["praca", "parque", "mercado", "porto", "templo"],
};

fn inside(x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && x < N && y < N
}

fn in_rect(x: i32, y: i32, r: [i32; 4]) -> bool {
    x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]
}

fn is_street(s: Surface) -> bool {
    matches!(s, Surface::Street | Surface::Sidewalk | Surface::Bridge)
}

struct Out(Vec<Violation>);

impl Out {
    fn err(&mut self, code: &str, id: &str, measure: String, message: String) {
        self.0.push(Violation {
            code: code.into(),
            id: id.into(),
            measure,
            message,
            severity: Severity::Error,
        });
    }
    fn warn(&mut self, code: &str, id: &str, measure: String, message: String) {
        self.0.push(Violation {
            code: code.into(),
            id: id.into(),
            measure,
            message,
            severity: Severity::Warning,
        });
    }
}

pub fn check(bp: &Blueprint, p: &CityProgram) -> Vec<Violation> {
    let mut o = Out(Vec::new());
    if bp.surface.len() != (N * N) as usize
        || bp.corner_height_mm.len() != ((N + 1) * (N + 1)) as usize
    {
        o.err(
            "P0",
            "blueprint",
            format!("{} tiles", bp.surface.len()),
            "blueprint grids have the wrong size; solve again".into(),
        );
        return o.0;
    }
    let surf = &bp.surface;
    let lots: BTreeMap<&str, &Lot> = bp.lots.iter().map(|l| (l.id.as_str(), l)).collect();
    let zone_of = |lot: &Lot| p.zones.iter().find(|z| z.id == lot.zone);
    let fixed_door = |lot: &str| {
        p.lots
            .explicit
            .iter()
            .find(|e| e.id == lot)
            .and_then(|e| e.door)
    };

    // Voxel owners per level; R1 overlap on the way.
    let levels = bp
        .buildings
        .iter()
        .map(|b| b.levels.len())
        .max()
        .unwrap_or(0);
    let mut occ = vec![vec![-1i32; (N * N) as usize]; levels.max(1)];
    let mut overlaps: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut first_overlap: BTreeMap<(usize, usize), (usize, [i32; 2])> = BTreeMap::new();
    for (bi, b) in bp.buildings.iter().enumerate() {
        for (k, cells) in b.levels.iter().enumerate() {
            for c in cells {
                if !inside(c[0], c[1]) {
                    continue;
                }
                let i = tile_index(c[0], c[1]);
                let prev = occ[k][i];
                if prev >= 0 && prev as usize != bi {
                    let key = (prev as usize, bi);
                    if overlaps.insert(key) {
                        first_overlap.insert(key, (k, *c));
                    }
                } else {
                    occ[k][i] = bi as i32;
                }
            }
        }
    }
    for (a, b) in &overlaps {
        let (k, c) = first_overlap[&(*a, *b)];
        let (ia, ib) = (&bp.buildings[*a].id, &bp.buildings[*b].id);
        o.err(
            "R1",
            ib,
            format!("level {k} at ({},{})", c[0], c[1]),
            format!(
                "{ib} overlaps {ia} at ({},{}) on level {k}; move one of them to another lot",
                c[0], c[1]
            ),
        );
    }

    // Walk distance to the nearest street tile over free walkable ground.
    let blocked = |i: usize| occ[0][i] >= 0 || !surf[i].walkable();
    let mut dist = vec![-1i32; (N * N) as usize];
    let mut q = VecDeque::new();
    for i in 0..(N * N) as usize {
        if is_street(surf[i]) && !blocked(i) {
            dist[i] = 0;
            q.push_back(i);
        }
    }
    while let Some(i) = q.pop_front() {
        let (x, y) = (i as i32 % N, i as i32 / N);
        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if inside(nx, ny) {
                let ni = tile_index(nx, ny);
                if dist[ni] < 0 && !blocked(ni) {
                    dist[ni] = dist[i] + 1;
                    q.push_back(ni);
                }
            }
        }
    }

    for (bi, b) in bp.buildings.iter().enumerate() {
        let id = b.id.as_str();
        let Some(lot) = lots.get(b.lot.as_str()) else {
            o.err(
                "R1",
                id,
                format!("lot {}", b.lot),
                format!(
                    "{id} stands on lot {}, which is not in the blueprint",
                    b.lot
                ),
            );
            continue;
        };
        let zone = zone_of(lot);
        let ground = b.levels.first().cloned().unwrap_or_default();

        // R1 inside the own lot.
        let outside: Vec<[i32; 2]> = b
            .levels
            .iter()
            .flatten()
            .filter(|c| !in_rect(c[0], c[1], lot.rect))
            .copied()
            .collect();
        if let Some(c) = outside.first() {
            o.err("R1", id, format!("{} voxels outside lot", outside.len()), format!("{id} has {} voxels outside its lot {} (first at ({},{})); keep the building inside the lot", outside.len(), lot.id, c[0], c[1]));
        }

        // R2 nothing on public ground or water.
        let bad: Vec<&[i32; 2]> = ground
            .iter()
            .filter(|c| {
                inside(c[0], c[1])
                    && (surf[tile_index(c[0], c[1])].public()
                        || surf[tile_index(c[0], c[1])] == Surface::Water)
            })
            .collect();
        if let Some(c) = bad.first() {
            let s = surf[tile_index(c[0], c[1])];
            o.err("R2", id, format!("{} tiles", bad.len()), format!("ground floor of {id} covers {} public or water tiles (first a {s:?} at ({},{})); buildings go on lots only", bad.len(), c[0], c[1]));
        }

        // R3 minimum buildable area after setbacks.
        let fd = fixed_door(&lot.id);
        let sb = grammar::setback_for(lot, zone, fd);
        let frame = Frame::new(lot.rect, lot.front);
        let [_, _, du, dv] = grammar::buildable_uv(&frame, &sb);
        let (bw, bh) = match lot.front {
            Dir::N | Dir::S => (du, dv),
            _ => (dv, du),
        };
        if du < 3 || dv < 3 {
            o.err("R3", id, format!("{}x{}", bw.max(0), bh.max(0)), format!("buildable area of {id} on lot {} is {}x{} tiles after setbacks; it needs 3x3 (6x6 m). Pick a bigger lot", lot.id, bw.max(0), bh.max(0)));
        }

        // R4 setbacks, measured on the ground floor.
        if !ground.is_empty() {
            let (_, lot_v) = frame.dims();
            let vs: Vec<i32> = ground.iter().map(|c| frame.to_uv(c[0], c[1]).1).collect();
            let front_gap = *vs.iter().min().unwrap();
            let back_gap = lot_v - 1 - *vs.iter().max().unwrap();
            let need_front = zone
                .map(|z| grammar::min_front_setback(&z.kind))
                .unwrap_or(0);
            if fd.is_none() && front_gap < need_front {
                o.err("R4", id, format!("front {front_gap} < {need_front}"), format!("{id} keeps {front_gap} tiles of front yard; zone {} needs {need_front} (2 m)", lot.zone));
            }
            if back_gap < 1 {
                o.err(
                    "R4",
                    id,
                    format!("back {back_gap} < 1"),
                    format!(
                        "{id} touches the back of lot {}; leave 1 tile (2 m) behind it",
                        lot.id
                    ),
                );
            }
        }

        // R5 the door reaches the street.
        let d = &b.door;
        let (dx, dy) = d.face.delta();
        if !ground.contains(&d.cell) {
            o.err(
                "R5",
                id,
                format!("door ({},{})", d.cell[0], d.cell[1]),
                format!(
                    "door of {id} at ({},{}) is not on its ground floor",
                    d.cell[0], d.cell[1]
                ),
            );
        } else if d.outside != [d.cell[0] + dx, d.cell[1] + dy] {
            o.err(
                "R5",
                id,
                format!("door ({},{})", d.cell[0], d.cell[1]),
                format!("door of {id} does not open onto the tile in front of it"),
            );
        } else if !inside(d.outside[0], d.outside[1]) {
            o.err(
                "R5",
                id,
                format!("({},{})", d.outside[0], d.outside[1]),
                format!(
                    "door of {id} opens outside the region at ({},{})",
                    d.outside[0], d.outside[1]
                ),
            );
        } else {
            let oi = tile_index(d.outside[0], d.outside[1]);
            if blocked(oi) {
                let what = if occ[0][oi] >= 0 {
                    format!("building {}", bp.buildings[occ[0][oi] as usize].id)
                } else {
                    format!("{:?}", surf[oi])
                };
                o.err("R5", id, format!("({},{}) blocked", d.outside[0], d.outside[1]), format!("door of {id} opens onto ({},{}), blocked by {what}; turn the building or clear the tile", d.outside[0], d.outside[1]));
            } else if dist[oi] < 0 {
                o.err("R5", id, "unreachable".into(), format!("door of {id} opens onto ({},{}) but no walkable path leads to a street; move the building or add a street", d.outside[0], d.outside[1]));
            } else if dist[oi] > MAX_DOOR_STEPS {
                o.err("R5", id, format!("{} tiles", dist[oi]), format!("door of {id} is {} tiles ({} m) from the nearest street; max {MAX_DOOR_STEPS}. Add a street closer", dist[oi], 2 * dist[oi]));
            }
        }

        // R7 stairs link every level.
        if b.floors > 1 {
            if b.stairs.is_empty() {
                o.err(
                    "R7",
                    id,
                    format!("{} levels, no stair", b.floors),
                    format!("{id} has {} levels and no stair core", b.floors),
                );
            }
            'st: for s in &b.stairs {
                for k in 0..b.floors as usize {
                    if !b.levels.get(k).map(|l| l.contains(s)).unwrap_or(false) {
                        o.err(
                            "R7",
                            id,
                            format!("level {k}"),
                            format!("stair of {id} at ({},{}) is cut at level {k}", s[0], s[1]),
                        );
                        break 'st;
                    }
                }
            }
        }

        // R8 support.
        for k in 1..b.levels.len() {
            if let Some(c) = b.levels[k].iter().find(|c| !b.levels[k - 1].contains(c)) {
                o.err(
                    "R8",
                    id,
                    format!("level {k} at ({},{})", c[0], c[1]),
                    format!(
                        "{id} has a voxel at ({},{}) on level {k} with nothing under it",
                        c[0], c[1]
                    ),
                );
                break;
            }
        }

        // R9 height by zone, tower limits.
        if let Some(z) = zone {
            if b.floors > z.floors[1] || b.floors < z.floors[0] {
                o.err(
                    "R9",
                    id,
                    format!(
                        "{} floors not in {}..{}",
                        b.floors, z.floors[0], z.floors[1]
                    ),
                    format!(
                        "{id} has {} floors; zone {} allows {} to {}",
                        b.floors, z.id, z.floors[0], z.floors[1]
                    ),
                );
            }
        }
        let total = b.floors as usize + b.tower.as_ref().map(|t| t.levels as usize).unwrap_or(0);
        if b.levels.len() > total {
            o.err(
                "R9",
                id,
                format!("{} levels", b.levels.len()),
                format!("{id} has more levels than floors plus tower"),
            );
        }
        if let Some(t) = &b.tower {
            let (x0, x1) = (
                t.cells.iter().map(|c| c[0]).min().unwrap_or(0),
                t.cells.iter().map(|c| c[0]).max().unwrap_or(0),
            );
            let (y0, y1) = (
                t.cells.iter().map(|c| c[1]).min().unwrap_or(0),
                t.cells.iter().map(|c| c[1]).max().unwrap_or(0),
            );
            if t.levels > 1 || x1 - x0 > 1 || y1 - y0 > 1 {
                o.err(
                    "R9",
                    id,
                    format!("tower {}x{} +{}", x1 - x0 + 1, y1 - y0 + 1, t.levels),
                    format!("tower of {id} may add 1 level on at most 2x2 tiles"),
                );
            }
        }

        // R12 frontage.
        let r = lot.rect;
        let pubt = |x: i32, y: i32| inside(x, y) && surf[tile_index(x, y)].public();
        let contact = match lot.front {
            Dir::N => (0..r[2]).filter(|i| pubt(r[0] + i, r[1] - 1)).count(),
            Dir::S => (0..r[2]).filter(|i| pubt(r[0] + i, r[1] + r[3])).count(),
            Dir::W => (0..r[3]).filter(|j| pubt(r[0] - 1, r[1] + j)).count(),
            Dir::E => (0..r[3]).filter(|j| pubt(r[0] + r[2], r[1] + j)).count(),
        } as i32;
        if contact < 3 && fixed_door(&lot.id).is_some() {
            // Lots migrated from the 2D city keep their pinned door; they are
            // grandfathered as long as the door reaches the street (R5).
            o.warn("R12", id, format!("frontage {contact}"), format!("lot {} of {id} is a migrated lot with {contact} tiles of frontage; allowed because its door is pinned", lot.id));
        } else if contact < 3 {
            o.err("R12", id, format!("frontage {contact}"), format!("lot {} of {id} touches a street or plaza on {contact} tiles; it needs 3 (6 m). Pick a lot on a street", lot.id));
        }

        // R14 level floor.
        let mut hs = BTreeSet::new();
        for c in &ground {
            for (cx, cy) in [
                (c[0], c[1]),
                (c[0] + 1, c[1]),
                (c[0], c[1] + 1),
                (c[0] + 1, c[1] + 1),
            ] {
                if cx >= 0 && cy >= 0 && cx <= N && cy <= N {
                    hs.insert(bp.corner_height_mm[corner_index(cx, cy)]);
                }
            }
        }
        if let (Some(lo), Some(hi)) = (hs.first(), hs.last()) {
            if hi - lo > 0 {
                o.err("R14", id, format!("{} mm", hi - lo), format!("ground under {id} is not level ({} mm between corners); the lot must be terraced", hi - lo));
            }
        }
        let _ = bi;
    }

    // R6 one public network.
    let public = |i: usize| surf[i].public();
    let mut comp = vec![-1i32; (N * N) as usize];
    let mut sizes: Vec<usize> = Vec::new();
    for s in 0..(N * N) as usize {
        if !public(s) || comp[s] >= 0 {
            continue;
        }
        let c = sizes.len() as i32;
        let mut stack = vec![s];
        comp[s] = c;
        let mut n = 0;
        while let Some(i) = stack.pop() {
            n += 1;
            let (x, y) = (i as i32 % N, i as i32 / N);
            for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if inside(nx, ny) {
                    let ni = tile_index(nx, ny);
                    if public(ni) && comp[ni] < 0 {
                        comp[ni] = c;
                        stack.push(ni);
                    }
                }
            }
        }
        sizes.push(n);
    }
    if sizes.len() > 1 {
        let main = (0..sizes.len())
            .max_by_key(|&c| (sizes[c], std::cmp::Reverse(c)))
            .unwrap() as i32;
        let cut: usize = sizes
            .iter()
            .enumerate()
            .filter(|(c, _)| *c as i32 != main)
            .map(|(_, n)| n)
            .sum();
        let mut names: Vec<String> = Vec::new();
        for s in &bp.streets {
            if s.tiles
                .iter()
                .all(|t| inside(t[0], t[1]) && comp[tile_index(t[0], t[1])] != main)
            {
                names.push(s.id.clone());
            }
        }
        for pl in &bp.places {
            let mut any = false;
            let mut in_main = false;
            for y in pl.rect[1]..pl.rect[1] + pl.rect[3] {
                for x in pl.rect[0]..pl.rect[0] + pl.rect[2] {
                    if inside(x, y) && surf[tile_index(x, y)] == Surface::Plaza {
                        any = true;
                        in_main |= comp[tile_index(x, y)] == main;
                    }
                }
            }
            if any && !in_main {
                names.push(pl.id.clone());
            }
        }
        let first = (0..(N * N) as usize)
            .find(|&i| comp[i] >= 0 && comp[i] != main)
            .unwrap_or(0);
        let who = if names.is_empty() {
            String::new()
        } else {
            format!(" ({})", names.join(", "))
        };
        o.err(
            "R6",
            "network",
            format!("{cut} tiles disconnected"),
            format!("{cut} public tiles{who} are cut off from the street network (first at ({},{})); make them touch a street", first as i32 % N, first as i32 / N),
        );
    }
    for s in &bp.streets {
        if let Some(t) = s
            .tiles
            .iter()
            .find(|t| t[0] == 0 || t[1] == 0 || t[0] == N - 1 || t[1] == N - 1)
        {
            o.warn("R6", &s.id, format!("edge ({},{})", t[0], t[1]), format!("street {} reaches the region edge at ({},{}); the neighbour region must continue it", s.id, t[0], t[1]));
        }
    }

    // R10 bridges, R13 widths.
    for (si, s) in bp.streets.iter().enumerate() {
        let wet: Vec<&[i32; 2]> = s
            .tiles
            .iter()
            .filter(|t| inside(t[0], t[1]) && surf[tile_index(t[0], t[1])] == Surface::Water)
            .collect();
        if let Some(t) = wet.first() {
            o.err("R10", &s.id, format!("{} water tiles", wet.len()), format!("street {} crosses water at ({},{}) without a bridge; list it in the water's bridges", s.id, t[0], t[1]));
        }
        if s.width < 2
            && s.tiles
                .iter()
                .any(|t| inside(t[0], t[1]) && surf[tile_index(t[0], t[1])] == Surface::Bridge)
        {
            o.err(
                "R10",
                &s.id,
                format!("width {}", s.width),
                format!(
                    "bridge of {} is {} tile wide; bridges need 2 (4 m)",
                    s.id, s.width
                ),
            );
        }
        let need = match s.class.as_str() {
            "avenida" => 3,
            "rua" => 2,
            _ => 1,
        };
        if s.width < need {
            o.err(
                "R13",
                &s.id,
                format!("width {} < {need}", s.width),
                format!(
                    "{} {} is {} tiles wide; a {} needs {need} ({} m)",
                    s.class,
                    s.id,
                    s.width,
                    s.class,
                    2 * need
                ),
            );
        }
        if s.class == "beco" {
            if let Some(ps) = p.streets.iter().find(|x| x.id == s.id) {
                let len: i32 = ps
                    .path
                    .windows(2)
                    .map(|w| (w[1][0] - w[0][0]).abs().max((w[1][1] - w[0][1]).abs()))
                    .sum::<i32>()
                    + 1;
                if len > MAX_BECO_LEN {
                    o.err("R13", &s.id, format!("length {len} > {MAX_BECO_LEN}"), format!("alley {} is {len} tiles long; alleys stop at {MAX_BECO_LEN} (48 m). Make it a rua", s.id));
                }
            }
        }
        let _ = si;
    }

    // R11 dead gaps of one tile between two buildings (warning).
    let mut gaps: BTreeMap<(usize, usize), (i32, [i32; 2])> = BTreeMap::new();
    for y in 0..N {
        for x in 0..N {
            let i = tile_index(x, y);
            if occ[0][i] >= 0 || surf[i].public() || !surf[i].walkable() || dist[i] >= 0 {
                continue;
            }
            for (ax, ay, bx, by) in [(x - 1, y, x + 1, y), (x, y - 1, x, y + 1)] {
                if inside(ax, ay) && inside(bx, by) {
                    let (a, b) = (occ[0][tile_index(ax, ay)], occ[0][tile_index(bx, by)]);
                    if a >= 0 && b >= 0 && a != b {
                        let key = ((a.min(b)) as usize, (a.max(b)) as usize);
                        let e = gaps.entry(key).or_insert((0, [x, y]));
                        e.0 += 1;
                    }
                }
            }
        }
    }
    for ((a, b), (n, c)) in gaps {
        let (ia, ib) = (&bp.buildings[a].id, &bp.buildings[b].id);
        o.warn(
            "R11",
            ia,
            format!("{n} gap tiles"),
            format!(
                "{ia} and {ib} leave a dead 1-tile gap at ({},{}); join them or leave 2 tiles",
                c[0], c[1]
            ),
        );
    }

    // R14 street ramps (warning).
    let mut steep = 0;
    let mut worst = (0, [0, 0]);
    for y in 0..N {
        for x in 0..N {
            if !is_street(surf[tile_index(x, y)]) {
                continue;
            }
            let c = |cx: i32, cy: i32| bp.corner_height_mm[corner_index(cx, cy)];
            for (a, b) in [
                ((x, y), (x + 1, y)),
                ((x, y), (x, y + 1)),
                ((x + 1, y), (x + 1, y + 1)),
                ((x, y + 1), (x + 1, y + 1)),
            ] {
                let d = (c(a.0, a.1) - c(b.0, b.1)).abs();
                if d > crate::solver::STREET_STEP_MM {
                    steep += 1;
                    if d > worst.0 {
                        worst = (d, [x, y]);
                    }
                }
            }
        }
    }
    if steep > 0 {
        o.warn(
            "R14",
            "streets",
            format!("{} mm per tile", worst.0),
            format!(
                "{steep} street edges are steeper than 1:8; the worst is {} mm per tile at ({},{})",
                worst.0, worst.1[0], worst.1[1]
            ),
        );
    }

    // R15 landmarks keep open ground.
    for pl in &bp.places {
        let open = (pl.rect[1]..pl.rect[1] + pl.rect[3])
            .flat_map(|y| (pl.rect[0]..pl.rect[0] + pl.rect[2]).map(move |x| (x, y)))
            .any(|(x, y)| inside(x, y) && surf[tile_index(x, y)] == Surface::Plaza);
        if !open {
            o.err(
                "R15",
                &pl.id,
                "0 open tiles".into(),
                format!(
                    "landmark {} has no open tile left (streets or water cover it); move it",
                    pl.id
                ),
            );
        }
    }

    check_catalog(&mut o, p);

    // R17 owned lots survive and keep their door.
    for b in &p.buildings {
        if let Some(own) = &b.owner_ref {
            if !bp
                .lots
                .iter()
                .any(|l| l.owner_ref.as_ref() == Some(own) && l.building.as_ref() == Some(&b.id))
            {
                o.err(
                    "R17",
                    &b.id,
                    own.clone(),
                    format!(
                        "owned building {} ({own}) did not get its lot; owned lots never move",
                        b.id
                    ),
                );
            }
        }
    }
    for e in &p.lots.explicit {
        if let (Some(door), Some(bl)) = (e.door, bp.lots.iter().find(|l| l.id == e.id)) {
            if let Some(b) = bl
                .building
                .as_ref()
                .and_then(|id| bp.buildings.iter().find(|x| &x.id == id))
            {
                if b.door.cell != door && b.door.outside != door {
                    o.err(
                        "R17",
                        &b.id,
                        format!("door ({},{})", b.door.outside[0], b.door.outside[1]),
                        format!(
                            "door of {} moved from ({},{}); migrated doors are fixed",
                            b.id, door[0], door[1]
                        ),
                    );
                }
            }
        }
    }

    budget(&mut o, bp, &occ);
    o.0
}

fn check_catalog(o: &mut Out, p: &CityProgram) {
    let cat = &KIT_VILA_V1;
    if !p.catalog.is_empty() && p.catalog != cat.key {
        o.warn(
            "R16",
            "catalog",
            p.catalog.clone(),
            format!(
                "catalog {} is unknown; only {} is checked",
                p.catalog, cat.key
            ),
        );
        return;
    }
    let bad = |list: &[&str], v: &str| !list.contains(&grammar::normalize_key(v).as_str());
    for b in &p.buildings {
        let it = &b.intent;
        if !it.use_.is_empty() && bad(cat.uses, &it.use_) {
            o.err(
                "R16",
                &b.id,
                format!("use {}", it.use_),
                format!(
                    "use `{}` of {} is not in the catalog: {:?}",
                    it.use_, b.id, cat.uses
                ),
            );
        }
        if let Some(s) = &it.shape {
            if bad(cat.shapes, s) {
                o.err(
                    "R16",
                    &b.id,
                    format!("shape {s}"),
                    format!(
                        "shape `{s}` of {} is not in the catalog: {:?}",
                        b.id, cat.shapes
                    ),
                );
            }
        }
        if let Some(r) = &it.roof {
            if bad(cat.roofs, r) {
                o.err(
                    "R16",
                    &b.id,
                    format!("roof {r}"),
                    format!(
                        "roof `{r}` of {} is not in the catalog: {:?}",
                        b.id, cat.roofs
                    ),
                );
            }
        }
        if let Some(pal) = &it.palette {
            if bad(cat.palettes, pal) {
                o.warn(
                    "R16",
                    &b.id,
                    format!("palette {pal}"),
                    format!(
                        "palette `{pal}` of {} is not in the catalog; the default colours are used",
                        b.id
                    ),
                );
            }
        }
        if let Some(st) = &it.style {
            if bad(cat.styles, st) {
                o.warn(
                    "R16",
                    &b.id,
                    format!("style {st}"),
                    format!("style `{st}` of {} has no kit yet: {:?}", b.id, cat.styles),
                );
            }
        }
        if let Some(t) = &b.typology {
            if !t.starts_with("codegeo:") {
                o.err(
                    "R16",
                    &b.id,
                    format!("typology {t}"),
                    format!("typology of {} must be `codegeo:<sha256>`", b.id),
                );
            }
        }
    }
    for f in &p.fill {
        if bad(cat.uses, &f.use_) {
            o.err(
                "R16",
                &f.zone,
                format!("fill use {}", f.use_),
                format!(
                    "fill of zone {} uses `{}`, not in {:?}",
                    f.zone, f.use_, cat.uses
                ),
            );
        }
        for s in f.shape.iter().filter(|s| bad(cat.shapes, s)) {
            o.err(
                "R16",
                &f.zone,
                format!("fill shape {s}"),
                format!(
                    "fill of zone {} lists shape `{s}`, not in {:?}",
                    f.zone, cat.shapes
                ),
            );
        }
        for r in f.roof.iter().filter(|r| bad(cat.roofs, r)) {
            o.err(
                "R16",
                &f.zone,
                format!("fill roof {r}"),
                format!(
                    "fill of zone {} lists roof `{r}`, not in {:?}",
                    f.zone, cat.roofs
                ),
            );
        }
    }
    for z in &p.zones {
        if bad(cat.zone_kinds, &z.kind) {
            o.err(
                "R16",
                &z.id,
                format!("kind {}", z.kind),
                format!("zone kind `{}` is not one of {:?}", z.kind, cat.zone_kinds),
            );
        }
        if bad(cat.styles, &z.style) {
            o.warn(
                "R16",
                &z.id,
                format!("style {}", z.style),
                format!(
                    "style `{}` of zone {} has no kit yet: {:?}",
                    z.style, z.id, cat.styles
                ),
            );
        }
        if let Some(pal) = &z.palette {
            if bad(cat.palettes, pal) {
                o.warn(
                    "R16",
                    &z.id,
                    format!("palette {pal}"),
                    format!("palette `{pal}` of zone {} is not in the catalog", z.id),
                );
            }
        }
    }
    for s in &p.streets {
        if bad(cat.street_classes, &s.class) {
            o.err(
                "R16",
                &s.id,
                format!("class {}", s.class),
                format!(
                    "street class `{}` is not one of {:?}",
                    s.class, cat.street_classes
                ),
            );
        }
    }
    for l in &p.landmarks {
        if bad(cat.landmark_kinds, &l.kind) {
            o.err(
                "R16",
                &l.id,
                format!("kind {}", l.kind),
                format!(
                    "landmark kind `{}` is not one of {:?}",
                    l.kind, cat.landmark_kinds
                ),
            );
        }
    }
    for pr in &p.props {
        let key = if pr.preset.is_empty() {
            &pr.kind
        } else {
            &pr.preset
        };
        if bad(cat.presets, key) {
            o.warn(
                "R16",
                &pr.kind,
                format!("preset {key}"),
                format!("prop preset `{key}` is not in the kit: {:?}", cat.presets),
            );
        }
    }
}

/// R18: estimated module instances and triangles per 16×16 chunk, with the
/// dual-grid case table of the prototype mesher.
fn budget(o: &mut Out, bp: &Blueprint, occ: &[Vec<i32>]) {
    // mask → canonical case (0 empty, 1 convex, 2 wall, 3 diagonal, 4 concave, 5 full)
    const CASE: [usize; 16] = [0, 1, 1, 2, 1, 3, 2, 4, 1, 2, 3, 4, 2, 4, 4, 5];
    const TRI_GROUND: [u32; 6] = [0, 60, 44, 80, 52, 8];
    const TRI_WALL: [u32; 6] = [0, 48, 36, 64, 40, 8];
    const TRI_ROOF: [u32; 6] = [0, 40, 32, 56, 36, 24];
    let chunks = (N / CHUNK_TILES) as usize;
    let mut inst = vec![0u32; chunks * chunks];
    let mut tri = vec![0u32; chunks * chunks];
    let get =
        |k: usize, x: i32, y: i32| k < occ.len() && inside(x, y) && occ[k][tile_index(x, y)] >= 0;
    for k in 0..occ.len() {
        for y in 0..=N {
            for x in 0..=N {
                let m = |k: usize| {
                    (get(k, x - 1, y - 1) as usize)
                        | (get(k, x, y - 1) as usize) << 1
                        | (get(k, x, y) as usize) << 2
                        | (get(k, x - 1, y) as usize) << 3
                };
                let here = m(k);
                if here == 0 {
                    continue;
                }
                let ch = ((y.min(N - 1) / CHUNK_TILES) as usize) * chunks
                    + (x.min(N - 1) / CHUNK_TILES) as usize;
                let case = CASE[here];
                inst[ch] += 1;
                tri[ch] += if k == 0 {
                    TRI_GROUND[case]
                } else {
                    TRI_WALL[case]
                };
                let roof = here & !m(k + 1);
                if roof != 0 {
                    inst[ch] += 1;
                    tri[ch] += TRI_ROOF[CASE[roof]];
                }
            }
        }
    }
    for pr in &bp.props {
        if inside(pr.cell[0], pr.cell[1]) {
            let ch = ((pr.cell[1] / CHUNK_TILES) as usize) * chunks
                + (pr.cell[0] / CHUNK_TILES) as usize;
            inst[ch] += 1;
            tri[ch] += 500;
        }
    }
    for ch in 0..chunks * chunks {
        if inst[ch] > CHUNK_MAX_INSTANCES || tri[ch] > CHUNK_MAX_TRIANGLES {
            let (cx, cy) = (ch % chunks, ch / chunks);
            o.warn("R18", &format!("chunk {cx},{cy}"), format!("{} instances, {} triangles", inst[ch], tri[ch]), format!("chunk ({cx},{cy}) needs about {} modules and {} triangles; the budget is {CHUNK_MAX_INSTANCES} and {CHUNK_MAX_TRIANGLES}. Use fewer floors or fewer trees there", inst[ch], tri[ch]));
        }
    }
}

/// R17 between two revisions: every owned lot of `prev` keeps its id, rect
/// and door in `next`, unless the change was made by the owner.
pub fn check_revision(prev: &Blueprint, next: &Blueprint, admin: bool) -> Vec<Violation> {
    let mut o = Out(Vec::new());
    if admin {
        return o.0;
    }
    for l in prev.lots.iter().filter(|l| l.owner_ref.is_some()) {
        let own = l.owner_ref.clone().unwrap_or_default();
        match next.lots.iter().find(|n| n.id == l.id) {
            None => o.err(
                "R17",
                &l.id,
                own.clone(),
                format!(
                    "owned lot {} ({own}) disappeared; owned lots cannot be removed",
                    l.id
                ),
            ),
            Some(n) if n.rect != l.rect || n.owner_ref != l.owner_ref => o.err(
                "R17",
                &l.id,
                format!("rect {:?} -> {:?}", l.rect, n.rect),
                format!("owned lot {} ({own}) changed; owned lots cannot move", l.id),
            ),
            Some(_) => {
                let door = |bp: &Blueprint| {
                    l.building
                        .as_ref()
                        .and_then(|b| bp.buildings.iter().find(|x| &x.id == b))
                        .map(|b| b.door.outside)
                };
                if let (Some(a), Some(b)) = (door(prev), door(next)) {
                    if a != b {
                        o.err(
                            "R17",
                            &l.id,
                            format!("door ({},{}) -> ({},{})", a[0], a[1], b[0], b[1]),
                            format!(
                                "the door of owned lot {} moved; the address must stay the same",
                                l.id
                            ),
                        );
                    }
                }
            }
        }
    }
    o.0
}

/// Only the blocking ones.
pub fn errors(v: &[Violation]) -> Vec<&Violation> {
    v.iter().filter(|x| x.severity == Severity::Error).collect()
}
