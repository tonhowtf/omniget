//! Decoration (`enfeite/*`) chosen by the WFC: never structural, always
//! equivalent for the building code (estudo 09 §5.3). One WFC pass per kind
//! of decoration, each over the region grid in 16×16 windows:
//! - curb (`meio-fio*`) on walkable tiles along a street, one piece per edge
//!   that touches the street: no two ramps, drains or lamp posts side by side;
//! - pavement (`calcada`, 3 patterns) on sidewalk tiles: waves never touch the
//!   checkerboard directly;
//! - lot fences (`cerca`, `muro`, `sebe`, `portao`) on the border of house and
//!   shed lots: one family per run, a gate forced in front of the door;
//! - flower beds and planters on plazas, sidewalks and yards, spread out;
//! - ridge tiles (`telha-capa`, `chamine`, `cata-vento`) along ridges, with
//!   no two chimneys or vanes together;
//! - bridges (`ponte` decks and `ponte-guarda` rails) are placed directly.

use super::keys;
use super::{Ctx, Ridge};
use crate::types::*;
use crate::wfc::{self, Tileset};

const DIRS: [Dir; 4] = [Dir::N, Dir::E, Dir::S, Dir::W];

pub(crate) fn decorate(ctx: &Ctx, ridges: &[Ridge], out: &mut Vec<Instance>) {
    curbs(ctx, out);
    pavement(ctx, out);
    fences(ctx, out);
    beds(ctx, out);
    ridge_tiles(ctx, ridges, out);
    bridges(ctx, out);
}

fn grid() -> (i32, i32) {
    (REGION_TILES, REGION_TILES)
}

fn is_street(s: Surface) -> bool {
    matches!(s, Surface::Street)
}

#[allow(clippy::too_many_arguments)]
fn push_enfeite(
    ctx: &Ctx,
    out: &mut Vec<Instance>,
    piece: &str,
    variant: Option<u32>,
    pos: (i32, i32, i32),
    rot: u8,
    tile: (i32, i32),
    owner: Option<String>,
) {
    let style = ctx.tile_style(tile.0, tile.1);
    let v = variant.unwrap_or_else(|| ctx.variant("enfeite", piece, 0, pos.0 / 1000, pos.2 / 1000));
    let pal = ctx.tile_palette(tile.0, tile.1);
    out.push(ctx.inst("enfeite", piece, style, v, pos, rot, &pal, owner));
}

fn curbs(ctx: &Ctx, out: &mut Vec<Instance>) {
    let ts = Tileset::symmetric(
        &[
            "nada",
            "meio-fio",
            "meio-fio-rampa",
            "meio-fio-bueiro",
            "meio-fio-poste",
        ],
        &[1, 60, 6, 8, 10],
        &[
            ("meio-fio-rampa", "meio-fio-rampa"),
            ("meio-fio-bueiro", "meio-fio-bueiro"),
            ("meio-fio-poste", "meio-fio-poste"),
            ("meio-fio-poste", "meio-fio-rampa"),
            ("meio-fio-bueiro", "meio-fio-rampa"),
        ],
        "meio-fio",
    );
    let (w, h) = grid();
    let street_edges = |x: i32, y: i32| -> Vec<Dir> {
        let s = ctx.surface(x, y);
        if is_street(s)
            || matches!(s, Surface::Bridge | Surface::Water)
            || !s.walkable()
            || ctx.solid(0, x, y)
        {
            return Vec::new();
        }
        DIRS.iter()
            .copied()
            .filter(|d| {
                let (dx, dy) = d.delta();
                Ctx::in_region(x + dx, y + dy) && is_street(ctx.surface(x + dx, y + dy))
            })
            .collect()
    };
    let mut dom = vec![ts.bit("nada"); (w * h) as usize];
    let mut edges = vec![Vec::new(); (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let e = street_edges(x, y);
            if !e.is_empty() {
                dom[tile_index(x, y)] = ts.full() & !ts.bit("nada");
                edges[tile_index(x, y)] = e;
            }
        }
    }
    let r = wfc::solve(&ts, w, h, &dom, &ctx.seed, "meio-fio");
    for y in 0..h {
        for x in 0..w {
            let i = tile_index(x, y);
            for (n, d) in edges[i].iter().enumerate() {
                let name = ts.names[r.tiles[i] as usize];
                let piece = if n == 0 && name != "nada" {
                    name
                } else {
                    "meio-fio"
                };
                let pos = ctx.edge(x, y, *d);
                push_enfeite(ctx, out, piece, None, pos, d.quarter(), (x, y), None);
            }
        }
    }
}

fn pavement(ctx: &Ctx, out: &mut Vec<Instance>) {
    let ts = Tileset::symmetric(
        &["nada", "lisa", "ondas", "xadrez"],
        &[1, 50, 25, 25],
        &[("ondas", "xadrez")],
        "lisa",
    );
    let (w, h) = grid();
    let cell = |x: i32, y: i32| ctx.surface(x, y) == Surface::Sidewalk && !ctx.solid(0, x, y);
    let dom: Vec<u32> = (0..w * h)
        .map(|i| {
            if cell(i % w, i / w) {
                ts.bit("lisa") | ts.bit("ondas") | ts.bit("xadrez")
            } else {
                ts.bit("nada")
            }
        })
        .collect();
    let r = wfc::solve(&ts, w, h, &dom, &ctx.seed, "calcada");
    for y in 0..h {
        for x in 0..w {
            if !cell(x, y) {
                continue;
            }
            let t = r.tiles[tile_index(x, y)] as u32;
            let pattern = t
                .saturating_sub(1)
                .min(keys::variants("enfeite", "calcada") - 1);
            let pos = (
                x * TILE_MM + TILE_MM / 2,
                ctx.tile_h(x, y),
                y * TILE_MM + TILE_MM / 2,
            );
            push_enfeite(ctx, out, "calcada", Some(pattern), pos, 0, (x, y), None);
        }
    }
}

fn fenced(ctx: &Ctx, lot: usize) -> Option<usize> {
    let b = ctx.lot_building[lot]?;
    matches!(ctx.bp.buildings[b].use_.as_str(), "casa" | "galpao").then_some(b)
}

fn fences(ctx: &Ctx, out: &mut Vec<Instance>) {
    let ts = Tileset::symmetric(
        &["nada", "aberto", "cerca", "muro", "sebe", "portao"],
        &[1, 4, 30, 20, 20, 0],
        &[
            ("cerca", "muro"),
            ("cerca", "sebe"),
            ("muro", "sebe"),
            ("portao", "portao"),
            ("portao", "sebe"),
        ],
        "aberto",
    );
    let (w, h) = grid();
    let mut dom = vec![ts.bit("nada"); (w * h) as usize];
    // Border cells and the edges each one draws.
    let mut edges: Vec<Vec<Dir>> = vec![Vec::new(); (w * h) as usize];
    let mut gate: Vec<Option<Dir>> = vec![None; (w * h) as usize];
    for (li, lot) in ctx.bp.lots.iter().enumerate() {
        let Some(bi) = fenced(ctx, li) else { continue };
        let [lx, ly, lw, lh] = lot.rect;
        let inside = |x: i32, y: i32| x >= lx && y >= ly && x < lx + lw && y < ly + lh;
        let b = &ctx.bp.buildings[bi];
        // The gate: walk from the door's outside tile toward the street.
        let mut gate_cell = None;
        let d = &b.door;
        let (gx, gy) = (d.outside[0], d.outside[1]);
        if inside(gx, gy) {
            let (dx, dy) = d.face.delta();
            let (mut x, mut y) = (gx, gy);
            while inside(x + dx, y + dy) {
                x += dx;
                y += dy;
            }
            gate_cell = Some((x, y));
        }
        for y in ly.max(0)..(ly + lh).min(h) {
            for x in lx.max(0)..(lx + lw).min(w) {
                if ctx.solid(0, x, y) {
                    continue;
                }
                let mut e = Vec::new();
                for dir in DIRS {
                    let (dx, dy) = dir.delta();
                    let (nx, ny) = (x + dx, y + dy);
                    if inside(nx, ny) {
                        continue;
                    }
                    if ctx.solid(0, nx, ny) {
                        continue;
                    }
                    // Between two fenced lots only the lower index draws.
                    if Ctx::in_region(nx, ny) {
                        let other = ctx.lot_of[tile_index(nx, ny)];
                        if other >= 0
                            && (other as usize) < li
                            && fenced(ctx, other as usize).is_some()
                        {
                            continue;
                        }
                    }
                    e.push(dir);
                }
                if e.is_empty() {
                    continue;
                }
                let i = tile_index(x, y);
                if gate_cell == Some((x, y)) && e.contains(&d.face) {
                    dom[i] = ts.bit("portao");
                    gate[i] = Some(d.face);
                } else {
                    dom[i] = ts.bit("aberto") | ts.bit("cerca") | ts.bit("muro") | ts.bit("sebe");
                }
                edges[i] = e;
            }
        }
    }
    let r = wfc::solve(&ts, w, h, &dom, &ctx.seed, "cerca");
    for y in 0..h {
        for x in 0..w {
            let i = tile_index(x, y);
            let name = ts.names[r.tiles[i] as usize];
            if matches!(name, "nada" | "aberto") {
                continue;
            }
            let owner = ctx.lot_of[i];
            let owner = (owner >= 0).then(|| ctx.bp.lots[owner as usize].id.clone());
            for d in &edges[i] {
                let piece = if name == "portao" {
                    if gate[i] == Some(*d) {
                        "portao"
                    } else {
                        "cerca"
                    }
                } else {
                    name
                };
                let pos = ctx.edge(x, y, *d);
                push_enfeite(
                    ctx,
                    out,
                    piece,
                    None,
                    pos,
                    d.quarter(),
                    (x, y),
                    owner.clone(),
                );
            }
        }
    }
}

fn beds(ctx: &Ctx, out: &mut Vec<Instance>) {
    let ts = Tileset::symmetric(
        &["nada", "livre", "canteiro", "vaso"],
        &[1, 70, 12, 6],
        &[("vaso", "vaso"), ("canteiro", "vaso")],
        "livre",
    );
    let (w, h) = grid();
    let mut blocked = vec![false; (w * h) as usize];
    for p in &ctx.bp.props {
        if Ctx::in_region(p.cell[0], p.cell[1]) {
            blocked[tile_index(p.cell[0], p.cell[1])] = true;
        }
    }
    for b in &ctx.bp.buildings {
        let o = b.door.outside;
        if Ctx::in_region(o[0], o[1]) {
            blocked[tile_index(o[0], o[1])] = true;
        }
    }
    let cell = |x: i32, y: i32| {
        matches!(
            ctx.surface(x, y),
            Surface::Plaza | Surface::Sidewalk | Surface::Yard
        ) && !ctx.solid(0, x, y)
            && !blocked[tile_index(x, y)]
    };
    let dom: Vec<u32> = (0..w * h)
        .map(|i| {
            if cell(i % w, i / w) {
                ts.bit("livre") | ts.bit("canteiro") | ts.bit("vaso")
            } else {
                ts.bit("nada")
            }
        })
        .collect();
    let r = wfc::solve(&ts, w, h, &dom, &ctx.seed, "canteiro");
    for y in 0..h {
        for x in 0..w {
            let name = ts.names[r.tiles[tile_index(x, y)] as usize];
            if !matches!(name, "canteiro" | "vaso") || !cell(x, y) {
                continue;
            }
            let pos = (
                x * TILE_MM + TILE_MM / 2,
                ctx.tile_h(x, y),
                y * TILE_MM + TILE_MM / 2,
            );
            push_enfeite(ctx, out, name, None, pos, 0, (x, y), None);
        }
    }
}

fn ridge_tiles(ctx: &Ctx, ridges: &[Ridge], out: &mut Vec<Instance>) {
    if ridges.is_empty() {
        return;
    }
    let ts = Tileset::symmetric(
        &["nada", "liso", "telha-capa", "chamine", "cata-vento"],
        &[1, 60, 20, 10, 3],
        &[
            ("chamine", "chamine"),
            ("cata-vento", "cata-vento"),
            ("chamine", "cata-vento"),
        ],
        "liso",
    );
    let (w, h) = (REGION_TILES + 1, REGION_TILES + 1);
    let mut dom = vec![ts.bit("nada"); (w * h) as usize];
    // Several levels can share a vertex; the highest ridge wins.
    let mut at: Vec<Option<usize>> = vec![None; (w * h) as usize];
    for (i, r) in ridges.iter().enumerate() {
        let c = (r.vy * w + r.vx) as usize;
        if at[c].map(|j| ridges[j].y < r.y).unwrap_or(true) {
            at[c] = Some(i);
        }
        dom[c] = ts.full() & !ts.bit("nada");
    }
    let r = wfc::solve(&ts, w, h, &dom, &ctx.seed, "cumeeira");
    for (c, slot) in at.iter().enumerate() {
        let Some(i) = slot else { continue };
        let name = ts.names[r.tiles[c] as usize];
        if matches!(name, "nada" | "liso") {
            continue;
        }
        let rd = &ridges[*i];
        let b = &ctx.bp.buildings[rd.building];
        let style = ctx.style[rd.building];
        let v = ctx.variant("enfeite", name, 0, rd.vx, rd.vy);
        out.push(ctx.inst(
            "enfeite",
            name,
            style,
            v,
            (rd.vx * TILE_MM, rd.y, rd.vy * TILE_MM),
            rd.rot,
            &b.palette,
            Some(b.id.clone()),
        ));
    }
}

fn bridges(ctx: &Ctx, out: &mut Vec<Instance>) {
    let water = super::terrain::water_level_mm(ctx.bp);
    for y in 0..REGION_TILES {
        for x in 0..REGION_TILES {
            if ctx.surface(x, y) != Surface::Bridge {
                continue;
            }
            let deck = |x: i32, y: i32| {
                let top = [
                    ctx.corner(x, y),
                    ctx.corner(x + 1, y),
                    ctx.corner(x + 1, y + 1),
                    ctx.corner(x, y + 1),
                ]
                .into_iter()
                .max()
                .unwrap();
                top.max(water.map(|w| w + 500).unwrap_or(i32::MIN))
            };
            let yy = deck(x, y);
            let along = |d: Dir| {
                let (dx, dy) = d.delta();
                Ctx::in_region(x + dx, y + dy)
                    && matches!(
                        ctx.surface(x + dx, y + dy),
                        Surface::Bridge | Surface::Street
                    )
            };
            let rot = if along(Dir::N) || along(Dir::S) { 0 } else { 1 };
            let pos = (x * TILE_MM + TILE_MM / 2, yy, y * TILE_MM + TILE_MM / 2);
            push_enfeite(ctx, out, "ponte", None, pos, rot, (x, y), None);
            for d in DIRS {
                let (dx, dy) = d.delta();
                if Ctx::in_region(x + dx, y + dy) && ctx.surface(x + dx, y + dy) == Surface::Water {
                    let (ex, _, ez) = ctx.edge(x, y, d);
                    push_enfeite(
                        ctx,
                        out,
                        "ponte-guarda",
                        None,
                        (ex, yy, ez),
                        d.quarter(),
                        (x, y),
                        None,
                    );
                }
            }
        }
    }
}
