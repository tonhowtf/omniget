//! Dual-grid mesher: blueprint → kit instances and terrain triangles.
//! Owner: agent C.
//!
//! For each level k and each dual vertex (the corner shared by 4 tiles) the
//! 4-bit mask of occupied tiles becomes one of 6 canonical cases plus a
//! rotation ([`cases`]). Layers:
//! - `terreo` (k = 0) and `parede` (k > 0) from the combined mask; the style is
//!   the owner's, the owner being the building with the lowest index among
//!   the 4 tiles; where two buildings meet, a `junta` sits on the seam;
//! - `telhado` from the mask "occupied at k and empty at k+1", piece picked by
//!   `roof.kind` and `ridge_axis` ([`roof`]);
//! - `torre` for tower levels, capped with `conico` roof pieces;
//! - `porta` and `bay` (janela, vitrine, sacada) as facade overlays on the
//!   door / bay tile face (`edge` anchor);
//! - `obra/andaime/any` and `obra/fundacao/any` for works below 100 %: only
//!   `floors · progress / 100` levels are drawn, no roof, no tower;
//! - `prop/<kind>/any` and `arvore/<preset>/any` for props and forest tiles;
//! - `enfeite` from the decoration WFC ([`enfeite`]): curbs, pavements, lot
//!   fences, flower beds, ridge tiles, bridges.
//!
//! Positions are mm inside the region (origin at its north-west corner).
//! Buildings stand on their base: the lowest corner height under their
//! ground floor (the solver flattens lots, so this only matters on slopes).
//! Tile-anchored pieces use the integer mean of the tile's 4 corners, edge
//! pieces the mean of the edge's 2 corners.
//!
//! Variants are `hash(seed, "variant", chunk, k, x, y)` with the seed taken
//! from the blueprint's `program_hash`. The list is sorted and hashed with
//! `canonical_hash`, so the same blueprint always gives the same hash.

pub mod cases;
pub mod enfeite;
pub mod keys;
pub mod roof;
pub mod sample;
pub mod terrain;

pub use keys::{kit_catalog, kit_keys, kit_keys_all, STYLES};
pub use terrain::terrain_mesh_with_level;

use crate::hash::{canonical_hash, pick};
use crate::types::*;
use cases::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn mesh_region(bp: &Blueprint) -> InstanceList {
    let ctx = Ctx::new(bp);
    let mut out: Vec<Instance> = Vec::new();
    let mut ridges: Vec<Ridge> = Vec::new();
    walls(&ctx, &mut out);
    roofs(&ctx, &mut out, &mut ridges);
    towers(&ctx, &mut out);
    doors_and_bays(&ctx, &mut out);
    works(&ctx, &mut out);
    props_and_trees(&ctx, &mut out);
    enfeite::decorate(&ctx, &ridges, &mut out);
    out.sort_by(|a, b| {
        (&a.key, a.z_mm, a.x_mm, a.y_mm, a.rot, &a.owner, &a.palette)
            .cmp(&(&b.key, b.z_mm, b.x_mm, b.y_mm, b.rot, &b.owner, &b.palette))
    });
    let hash = canonical_hash(&out);
    InstanceList {
        region: bp.region,
        blueprint_hash: bp.hash.clone(),
        instances: out,
        hash,
    }
}

pub fn terrain_mesh(bp: &Blueprint) -> TerrainMesh {
    terrain::terrain_mesh(bp)
}

/// Instance count per layer (first key segment).
pub fn layer_counts(list: &InstanceList) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for i in &list.instances {
        let layer = i.key.split('/').next().unwrap_or("").to_string();
        *m.entry(layer).or_insert(0) += 1;
    }
    m
}

/// A ridge vertex, input of the ridge-tile WFC.
pub(crate) struct Ridge {
    pub vx: i32,
    pub vy: i32,
    pub y: i32,
    pub rot: u8,
    pub building: usize,
}

/// Occupancy and per-building facts shared by every pass.
pub(crate) struct Ctx<'a> {
    pub bp: &'a Blueprint,
    pub seed: String,
    /// `main[k][tile]` = building index or -1 (visible levels only).
    pub main: Vec<Vec<i16>>,
    /// Tower voxels above the main mass.
    pub tower: Vec<Vec<i16>>,
    pub base: Vec<i32>,
    /// Main levels drawn (progress applied).
    pub vis: Vec<usize>,
    /// Main levels of the finished building.
    pub floors: Vec<usize>,
    pub style: Vec<&'static str>,
    pub default_style: &'static str,
    pub default_palette: String,
    /// Lot index per tile, -1 outside lots.
    pub lot_of: Vec<i16>,
    pub lot_building: Vec<Option<usize>>,
}

impl<'a> Ctx<'a> {
    pub fn new(bp: &'a Blueprint) -> Ctx<'a> {
        let seed = if bp.program_hash.is_empty() {
            bp.hash.clone()
        } else {
            bp.program_hash.clone()
        };
        let nb = bp.buildings.len();
        let mut floors = vec![0usize; nb];
        let mut vis = vec![0usize; nb];
        let mut top = 1usize;
        for (i, b) in bp.buildings.iter().enumerate() {
            let f = (b.floors as usize).min(b.levels.len());
            floors[i] = f;
            vis[i] = if b.progress >= 100 {
                f
            } else {
                f * b.progress as usize / 100
            };
            top = top.max(f + 1);
            if let Some(t) = &b.tower {
                top = top.max(t.from_level as usize + t.levels as usize + 1);
            }
        }
        let cells = (REGION_TILES * REGION_TILES) as usize;
        let mut main = vec![vec![-1i16; cells]; top];
        let mut tower = vec![vec![-1i16; cells]; top];
        let inside =
            |c: &[i32; 2]| c[0] >= 0 && c[1] >= 0 && c[0] < REGION_TILES && c[1] < REGION_TILES;
        for (i, b) in bp.buildings.iter().enumerate() {
            #[allow(clippy::needless_range_loop)]
            for k in 0..vis[i] {
                for c in b.levels[k].iter().filter(|c| inside(c)) {
                    let t = &mut main[k][tile_index(c[0], c[1])];
                    if *t < 0 {
                        *t = i as i16;
                    }
                }
            }
            if let (Some(t), true) = (&b.tower, b.progress >= 100) {
                for k in t.from_level as usize..(t.from_level as usize + t.levels as usize) {
                    for c in t.cells.iter().filter(|c| inside(c)) {
                        let idx = tile_index(c[0], c[1]);
                        if main[k][idx] != i as i16 && tower[k][idx] < 0 {
                            tower[k][idx] = i as i16;
                        }
                    }
                }
            }
        }
        let base: Vec<i32> = bp
            .buildings
            .iter()
            .map(|b| {
                let mut m = i32::MAX;
                for c in b.levels.first().map(|l| l.as_slice()).unwrap_or(&[]) {
                    for (dx, dy) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
                        m = m.min(terrain::corner_h(bp, c[0] + dx, c[1] + dy));
                    }
                }
                if m == i32::MAX {
                    0
                } else {
                    m
                }
            })
            .collect();
        let style: Vec<&'static str> = bp
            .buildings
            .iter()
            .map(|b| keys::norm_style(&b.style))
            .collect();
        let most = |vals: Vec<String>, dflt: &str| -> String {
            let mut count: BTreeMap<String, usize> = BTreeMap::new();
            for v in vals {
                *count.entry(v).or_insert(0) += 1;
            }
            let mut best: Option<(usize, String)> = None;
            for (k, n) in count {
                if best.as_ref().map(|b| n > b.0).unwrap_or(true) {
                    best = Some((n, k));
                }
            }
            best.map(|b| b.1).unwrap_or_else(|| dflt.to_string())
        };
        let ds = most(style.iter().map(|s| s.to_string()).collect(), STYLES[0]);
        let default_style = keys::norm_style(&ds);
        let default_palette = most(
            bp.buildings.iter().map(|b| b.palette.clone()).collect(),
            "cal-e-terracota",
        );
        let mut lot_of = vec![-1i16; cells];
        for (li, l) in bp.lots.iter().enumerate() {
            let [x, y, w, h] = l.rect;
            for ty in y.max(0)..(y + h).min(REGION_TILES) {
                for tx in x.max(0)..(x + w).min(REGION_TILES) {
                    lot_of[tile_index(tx, ty)] = li as i16;
                }
            }
        }
        let lot_building = bp
            .lots
            .iter()
            .map(|l| {
                bp.buildings
                    .iter()
                    .position(|b| b.lot == l.id || l.building.as_deref() == Some(b.id.as_str()))
            })
            .collect();
        Ctx {
            bp,
            seed,
            main,
            tower,
            base,
            vis,
            floors,
            style,
            default_style,
            default_palette,
            lot_of,
            lot_building,
        }
    }

    pub fn levels(&self) -> usize {
        self.main.len()
    }

    pub fn in_region(x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < REGION_TILES && y < REGION_TILES
    }

    pub fn main_at(&self, k: usize, x: i32, y: i32) -> i16 {
        if k >= self.main.len() || !Self::in_region(x, y) {
            return -1;
        }
        self.main[k][tile_index(x, y)]
    }

    pub fn tower_at(&self, k: usize, x: i32, y: i32) -> i16 {
        if k >= self.tower.len() || !Self::in_region(x, y) {
            return -1;
        }
        self.tower[k][tile_index(x, y)]
    }

    /// Anything built at level k on the tile.
    pub fn solid(&self, k: usize, x: i32, y: i32) -> bool {
        self.main_at(k, x, y) >= 0 || self.tower_at(k, x, y) >= 0
    }

    pub fn surface(&self, x: i32, y: i32) -> Surface {
        terrain::surface_at(self.bp, x, y)
    }

    pub fn corner(&self, vx: i32, vy: i32) -> i32 {
        terrain::corner_h(self.bp, vx, vy)
    }

    pub fn tile_h(&self, x: i32, y: i32) -> i32 {
        (self.corner(x, y)
            + self.corner(x + 1, y)
            + self.corner(x + 1, y + 1)
            + self.corner(x, y + 1))
        .div_euclid(4)
    }

    /// Middle of the tile edge on side `d` (mm) and its height.
    pub fn edge(&self, x: i32, y: i32, d: Dir) -> (i32, i32, i32) {
        let (ax, ay, bx, by) = match d {
            Dir::N => (x, y, x + 1, y),
            Dir::E => (x + 1, y, x + 1, y + 1),
            Dir::S => (x, y + 1, x + 1, y + 1),
            Dir::W => (x, y, x, y + 1),
        };
        let h = (self.corner(ax, ay) + self.corner(bx, by)).div_euclid(2);
        ((ax + bx) * TILE_MM / 2, h, (ay + by) * TILE_MM / 2)
    }

    pub fn tile_style(&self, x: i32, y: i32) -> &'static str {
        self.tile_building(x, y)
            .map(|b| self.style[b])
            .unwrap_or(self.default_style)
    }

    pub fn tile_palette(&self, x: i32, y: i32) -> String {
        self.tile_building(x, y)
            .map(|b| self.bp.buildings[b].palette.clone())
            .unwrap_or_else(|| self.default_palette.clone())
    }

    pub fn tile_building(&self, x: i32, y: i32) -> Option<usize> {
        if !Self::in_region(x, y) {
            return None;
        }
        let l = self.lot_of[tile_index(x, y)];
        if l < 0 {
            None
        } else {
            self.lot_building[l as usize]
        }
    }

    pub fn variant(&self, layer: &str, case: &str, k: usize, x: i32, y: i32) -> u32 {
        let n = keys::variants(layer, case);
        if n <= 1 {
            return 0;
        }
        let (cx, cy) = (
            x.clamp(0, REGION_TILES - 1) / CHUNK_TILES,
            y.clamp(0, REGION_TILES - 1) / CHUNK_TILES,
        );
        pick(
            &self.seed,
            "variant",
            &format!("{cx},{cy}|{k}|{x},{y}|{layer}/{case}"),
            0,
            n as u64,
        ) as u32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn inst(
        &self,
        layer: &str,
        case: &str,
        style: &str,
        variant: u32,
        pos: (i32, i32, i32),
        rot: u8,
        palette: &str,
        owner: Option<String>,
    ) -> Instance {
        Instance {
            key: format!("{layer}/{case}/{style}/{variant}"),
            x_mm: pos.0,
            y_mm: pos.1,
            z_mm: pos.2,
            rot: rot & 3,
            palette: palette.to_string(),
            owner,
        }
    }
}

fn mask_of(f: impl Fn(i32, i32) -> bool, vx: i32, vy: i32) -> u8 {
    (f(vx - 1, vy - 1) as u8)
        | ((f(vx, vy - 1) as u8) << 1)
        | ((f(vx, vy) as u8) << 2)
        | ((f(vx - 1, vy) as u8) << 3)
}

fn around(vx: i32, vy: i32) -> [(i32, i32); 4] {
    [(vx - 1, vy - 1), (vx, vy - 1), (vx, vy), (vx - 1, vy)]
}

fn walls(ctx: &Ctx, out: &mut Vec<Instance>) {
    for k in 0..ctx.levels() {
        let layer = if k == 0 { "terreo" } else { "parede" };
        for vy in 0..=REGION_TILES {
            for vx in 0..=REGION_TILES {
                let q = around(vx, vy).map(|(x, y)| ctx.main_at(k, x, y));
                let mask = mask_of(|x, y| ctx.main_at(k, x, y) >= 0, vx, vy);
                if mask == 0 {
                    continue;
                }
                let owner = q.iter().copied().filter(|t| *t >= 0).min().unwrap() as usize;
                let b = &ctx.bp.buildings[owner];
                let style = ctx.style[owner];
                let (case, rot) = classify(mask);
                let pos = (
                    vx * TILE_MM,
                    ctx.base[owner] + k as i32 * LEVEL_MM,
                    vy * TILE_MM,
                );
                if case != CHEIO {
                    let cname = CASE_NAMES[case as usize];
                    let v = ctx.variant(layer, cname, k, vx, vy);
                    out.push(ctx.inst(
                        layer,
                        cname,
                        style,
                        v,
                        pos,
                        rot,
                        &b.palette,
                        Some(b.id.clone()),
                    ));
                }
                let mut other = 0u8;
                for (i, t) in q.iter().enumerate() {
                    if *t >= 0 && *t as usize != owner {
                        other |= 1 << i;
                    }
                }
                if other != 0 && mask != 15 {
                    let (oc, orot) = classify(other);
                    let cname = CASE_NAMES[oc as usize];
                    let v = ctx.variant("junta", cname, k, vx, vy);
                    out.push(ctx.inst(
                        "junta",
                        cname,
                        style,
                        v,
                        pos,
                        orot,
                        &b.palette,
                        Some(b.id.clone()),
                    ));
                }
            }
        }
    }
}

fn roofs(ctx: &Ctx, out: &mut Vec<Instance>, ridges: &mut Vec<Ridge>) {
    for k in 0..ctx.levels() {
        let is_roof_any = |x: i32, y: i32| ctx.main_at(k, x, y) >= 0 && !ctx.solid(k + 1, x, y);
        for vy in 0..=REGION_TILES {
            for vx in 0..=REGION_TILES {
                let mask = mask_of(is_roof_any, vx, vy);
                if mask == 0 {
                    continue;
                }
                let owner = around(vx, vy)
                    .iter()
                    .filter(|(x, y)| is_roof_any(*x, *y))
                    .map(|(x, y)| ctx.main_at(k, *x, *y))
                    .min()
                    .unwrap() as usize;
                // Only finished buildings get a roof.
                if ctx.vis[owner] < ctx.floors[owner] {
                    continue;
                }
                let b = &ctx.bp.buildings[owner];
                let own = |x: i32, y: i32| {
                    ctx.main_at(k, x, y) == owner as i16 && !ctx.solid(k + 1, x, y)
                };
                let dist = roof::eave_distances(&own, vx, vy);
                let fam = roof::family(&b.roof.kind);
                let (case, crot) = classify(mask);
                let p = roof::piece(fam, b.roof.ridge_axis, case, crot, &dist);
                let y = ctx.base[owner] + (k as i32 + 1) * LEVEL_MM + p.lift_mm;
                let style = ctx.style[owner];
                if p.name.ends_with("cumeeira") {
                    ridges.push(Ridge {
                        vx,
                        vy,
                        y,
                        rot: p.rot,
                        building: owner,
                    });
                }
                let v = ctx.variant("telhado", &p.name, k, vx, vy);
                out.push(ctx.inst(
                    "telhado",
                    &p.name,
                    style,
                    v,
                    (vx * TILE_MM, y, vy * TILE_MM),
                    p.rot,
                    &b.palette,
                    Some(b.id.clone()),
                ));
            }
        }
    }
}

fn vertices_of(cells: &[[i32; 2]]) -> BTreeSet<(i32, i32)> {
    let mut s = BTreeSet::new();
    for c in cells {
        for (dx, dy) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
            s.insert((c[1] + dy, c[0] + dx));
        }
    }
    s
}

fn towers(ctx: &Ctx, out: &mut Vec<Instance>) {
    for (bi, b) in ctx.bp.buildings.iter().enumerate() {
        let Some(t) = &b.tower else { continue };
        if b.progress < 100 || t.levels == 0 {
            continue;
        }
        let style = ctx.style[bi];
        let verts = vertices_of(&t.cells);
        let from = t.from_level as usize;
        let top = from + t.levels as usize - 1;
        for k in from..=top {
            for &(vy, vx) in &verts {
                let mask = mask_of(|x, y| ctx.tower_at(k, x, y) == bi as i16, vx, vy);
                let (case, rot) = classify(mask);
                if case == VAZIO || case == CHEIO {
                    continue;
                }
                let cname = CASE_NAMES[case as usize];
                let v = ctx.variant("torre", cname, k, vx, vy);
                let pos = (
                    vx * TILE_MM,
                    ctx.base[bi] + k as i32 * LEVEL_MM,
                    vy * TILE_MM,
                );
                out.push(ctx.inst(
                    "torre",
                    cname,
                    style,
                    v,
                    pos,
                    rot,
                    &b.palette,
                    Some(b.id.clone()),
                ));
            }
        }
        let cap =
            |x: i32, y: i32| ctx.tower_at(top, x, y) == bi as i16 && !ctx.solid(top + 1, x, y);
        for &(vy, vx) in &verts {
            let mask = mask_of(cap, vx, vy);
            if mask == 0 {
                continue;
            }
            let dist = roof::eave_distances(&cap, vx, vy);
            let (case, crot) = classify(mask);
            let p = roof::piece("conico", Axis::X, case, crot, &dist);
            let y = ctx.base[bi] + (top as i32 + 1) * LEVEL_MM + p.lift_mm;
            out.push(ctx.inst(
                "telhado",
                &p.name,
                style,
                0,
                (vx * TILE_MM, y, vy * TILE_MM),
                p.rot,
                &b.palette,
                Some(b.id.clone()),
            ));
        }
    }
}

fn door_kind(use_: &str) -> &'static str {
    match use_ {
        "civico" | "templo" | "galpao" => "dupla",
        u if crate::grammar::is_shop(u) => "loja",
        _ => "simples",
    }
}

fn doors_and_bays(ctx: &Ctx, out: &mut Vec<Instance>) {
    for (bi, b) in ctx.bp.buildings.iter().enumerate() {
        let style = ctx.style[bi];
        let owner = Some(b.id.clone());
        let visible = |k: usize| {
            if k < ctx.floors[bi] {
                k < ctx.vis[bi]
            } else {
                b.progress >= 100
            }
        };
        let d = &b.door;
        if visible(d.level as usize) && Ctx::in_region(d.cell[0], d.cell[1]) {
            let (x, _, z) = ctx.edge(d.cell[0], d.cell[1], d.face);
            let y = ctx.base[bi] + d.level as i32 * LEVEL_MM;
            let kind = door_kind(&b.use_);
            let v = ctx.variant("porta", kind, d.level as usize, d.cell[0], d.cell[1]);
            out.push(ctx.inst(
                "porta",
                kind,
                style,
                v,
                (x, y, z),
                d.face.quarter(),
                &b.palette,
                owner.clone(),
            ));
        }
        for bay in &b.bays {
            let k = bay.level as usize;
            if !visible(k) || !Ctx::in_region(bay.cell[0], bay.cell[1]) {
                continue;
            }
            let (dx, dy) = bay.face.delta();
            if ctx.solid(k, bay.cell[0] + dx, bay.cell[1] + dy) {
                continue;
            }
            let (layer, kind) = match bay.kind.as_str() {
                "janela" | "vitrine" | "sacada" => ("bay", bay.kind.as_str()),
                "porta" => {
                    if bay.cell == d.cell && bay.face == d.face && bay.level == d.level {
                        continue;
                    }
                    ("porta", "simples")
                }
                _ => continue,
            };
            let (x, _, z) = ctx.edge(bay.cell[0], bay.cell[1], bay.face);
            let y = ctx.base[bi] + k as i32 * LEVEL_MM;
            let v = ctx.variant(layer, kind, k, bay.cell[0] * 2 + dx, bay.cell[1] * 2 + dy);
            out.push(ctx.inst(
                layer,
                kind,
                style,
                v,
                (x, y, z),
                bay.face.quarter(),
                &b.palette,
                owner.clone(),
            ));
        }
    }
}

fn works(ctx: &Ctx, out: &mut Vec<Instance>) {
    for (bi, b) in ctx.bp.buildings.iter().enumerate() {
        if b.progress >= 100 || ctx.floors[bi] == 0 {
            continue;
        }
        let l = ctx.vis[bi];
        if l >= ctx.floors[bi] {
            continue;
        }
        let owner = Some(b.id.clone());
        let cells = &b.levels[l];
        let set: BTreeSet<(i32, i32)> = cells.iter().map(|c| (c[0], c[1])).collect();
        for (vy, vx) in vertices_of(cells) {
            let mask = mask_of(|x, y| set.contains(&(x, y)), vx, vy);
            let (case, rot) = classify(mask);
            if case == VAZIO || case == CHEIO {
                continue;
            }
            let pos = (
                vx * TILE_MM,
                ctx.base[bi] + l as i32 * LEVEL_MM,
                vy * TILE_MM,
            );
            out.push(ctx.inst(
                "obra",
                "andaime",
                keys::ANY,
                0,
                pos,
                rot,
                &b.palette,
                owner.clone(),
            ));
        }
        if l == 0 {
            for c in cells.iter().filter(|c| Ctx::in_region(c[0], c[1])) {
                let pos = (
                    c[0] * TILE_MM + TILE_MM / 2,
                    ctx.base[bi],
                    c[1] * TILE_MM + TILE_MM / 2,
                );
                out.push(ctx.inst(
                    "obra",
                    "fundacao",
                    keys::ANY,
                    0,
                    pos,
                    0,
                    &b.palette,
                    owner.clone(),
                ));
            }
        }
    }
}

fn props_and_trees(ctx: &Ctx, out: &mut Vec<Instance>) {
    let mut taken: BTreeSet<(i32, i32)> = BTreeSet::new();
    for p in &ctx.bp.props {
        let [x, y] = p.cell;
        if !Ctx::in_region(x, y) {
            continue;
        }
        taken.insert((x, y));
        let pos = (
            x * TILE_MM + TILE_MM / 2,
            ctx.tile_h(x, y),
            y * TILE_MM + TILE_MM / 2,
        );
        let rot = ((p.seed / 7) % 4) as u8;
        let owner = Some(format!("prop:{x}_{y}"));
        let pal = ctx.tile_palette(x, y);
        if p.kind == "arvore" {
            let preset = keys::norm_tree(&p.preset);
            let v = p.seed % keys::variants("arvore", preset);
            out.push(ctx.inst("arvore", preset, keys::ANY, v, pos, rot, &pal, owner));
        } else {
            let kind = keys::norm_prop(&p.kind);
            let v = p.seed % keys::variants("prop", kind);
            out.push(ctx.inst("prop", kind, keys::ANY, v, pos, rot, &pal, owner));
        }
    }
    // Forest tiles get trees of their own: 45 % of the tiles, jittered.
    let s = crate::hash::seed_u64(&ctx.seed);
    for y in 0..REGION_TILES {
        for x in 0..REGION_TILES {
            if ctx.surface(x, y) != Surface::Forest || taken.contains(&(x, y)) || ctx.solid(0, x, y)
            {
                continue;
            }
            let h = crate::hash::hash2(s, x, y, 0x7EE0);
            if h % 100 >= 45 {
                continue;
            }
            let h2 = crate::hash::hash2(s, x, y, 0x7EE1);
            let preset = if (h >> 8).is_multiple_of(3) {
                "oak-estilizada"
            } else {
                "pinheiro-estilizado"
            };
            let v = (h >> 12) % keys::variants("arvore", preset);
            let jx = (h2 % 801) as i32 - 400;
            let jz = ((h2 >> 12) % 801) as i32 - 400;
            let pos = (
                x * TILE_MM + TILE_MM / 2 + jx,
                ctx.tile_h(x, y),
                y * TILE_MM + TILE_MM / 2 + jz,
            );
            out.push(ctx.inst(
                "arvore",
                preset,
                keys::ANY,
                v,
                pos,
                ((h >> 20) % 4) as u8,
                &ctx.default_palette,
                None,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_blueprint_same_hash() {
        let bp = sample::sample_blueprint();
        let a = mesh_region(&bp);
        let b = mesh_region(&bp);
        assert_eq!(a.hash, b.hash);
        assert!(!a.instances.is_empty());
        let t1 = terrain_mesh(&bp);
        let t2 = terrain_mesh(&bp);
        assert_eq!(canonical_hash(&t1), canonical_hash(&t2));
        assert!(t1.indices.len() / 3 <= terrain::MAX_TRIANGLES);
    }

    #[test]
    fn every_key_is_in_the_kit() {
        let bp = sample::sample_blueprint();
        let list = mesh_region(&bp);
        let all: BTreeSet<String> = kit_keys_all().into_iter().collect();
        for i in &list.instances {
            assert!(all.contains(&i.key), "key {} missing from kit_keys", i.key);
        }
    }
}
