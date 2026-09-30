//! Software raster of a region so an LLM can see what it built. Owner: agent C.
//!
//! No GPU, no three.js: triangles with flat colour and a depth buffer, drawn
//! from the blueprint (terrain from the mesher, buildings as one box per level
//! and tile with the same roof heights the kit uses, bays as small panels) and
//! the instance list (trees, props, fences, bridges, scaffolding). Views: from
//! above (north up, tile ticks every 8 tiles and chunk lines) and oblique
//! from each side. Building ids are written with a 5×7 bitmap font. Sized to
//! read at 768 px.

pub mod font;

use crate::charter::builtin_palettes;
use crate::mesher::{self, roof, Ctx};
use crate::types::*;
use std::collections::HashMap;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Top,
    /// Oblique view from the given side.
    Oblique(Dir),
}

#[derive(Clone, Debug)]
pub struct PreviewOptions {
    /// Write building ids.
    pub labels: bool,
    /// Tile ticks, chunk lines and caption.
    pub grid: bool,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        PreviewOptions {
            labels: true,
            grid: true,
        }
    }
}

/// Default colour of each palette slot (terrain and fallback for unknown
/// palettes), in [`PALETTE_SLOTS`] order.
pub const DEFAULT_SLOT_COLORS: [(&str, [u8; 3]); 16] = [
    ("wall", [0xF1, 0xE7, 0xD0]),
    ("roof", [0xB9, 0x65, 0x4C]),
    ("trim", [0xE4, 0xD6, 0xB6]),
    ("wood", [0x7A, 0x52, 0x36]),
    ("stone", [0xB8, 0xAE, 0x9C]),
    ("glass", [0x5E, 0x86, 0x9A]),
    ("door", [0x43, 0x6D, 0x59]),
    ("ground", [0xCB, 0xA5, 0x68]),
    ("grass", [0x8F, 0xB0, 0x78]),
    ("water", [0x5F, 0x9F, 0xB4]),
    ("street", [0x8C, 0x85, 0x77]),
    ("plaza", [0xD9, 0xC9, 0xA8]),
    ("leaf", [0x5E, 0x8A, 0x55]),
    ("trunk", [0x6B, 0x4A, 0x33]),
    ("metal", [0x26, 0x3C, 0x37]),
    ("cloth", [0xB9, 0x65, 0x4C]),
];

pub fn default_slot_color(slot: &str) -> [u8; 3] {
    DEFAULT_SLOT_COLORS
        .iter()
        .find(|c| c.0 == slot)
        .map(|c| c.1)
        .unwrap_or([200, 0, 200])
}

/// RGB image with a put/blend API used by the font.
pub struct Image {
    pub w: i32,
    pub h: i32,
    pub rgb: Vec<u8>,
}

impl Image {
    fn new(w: i32, h: i32, bg: [u8; 3]) -> Image {
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..w * h {
            rgb.extend_from_slice(&bg);
        }
        Image { w, h, rgb }
    }

    pub fn put(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = ((y * self.w + x) * 3) as usize;
        self.rgb[i..i + 3].copy_from_slice(&c);
    }

    fn get(&self, x: i32, y: i32) -> [u8; 3] {
        let i = ((y * self.w + x) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    fn blend(&mut self, x: i32, y: i32, c: [u8; 3], alpha: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let o = self.get(x, y);
        let m = |a: u8, b: u8| ((a as u32 * (256 - alpha) + b as u32 * alpha) >> 8) as u8;
        self.put(x, y, [m(o[0], c[0]), m(o[1], c[1]), m(o[2], c[2])]);
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 3], alpha: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend(x, y, c, alpha);
            }
        }
    }

    fn png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.w as u32, self.h as u32);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().expect("png header");
            wr.write_image_data(&self.rgb).expect("png data");
        }
        out
    }
}

type V3 = [f64; 3];

struct Tri {
    p: [V3; 3],
    rgb: [u8; 3],
    id: u32,
}

#[derive(Default)]
struct Scene {
    tris: Vec<Tri>,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l == 0.0 {
        [0.0, 1.0, 0.0]
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

fn shade(c: [u8; 3], n: V3) -> [u8; 3] {
    let l = norm([-0.5, 0.8, -0.3]);
    let k = 0.55 + 0.45 * dot(norm(n), l).max(0.0);
    [
        (c[0] as f64 * k).min(255.0) as u8,
        (c[1] as f64 * k).min(255.0) as u8,
        (c[2] as f64 * k).min(255.0) as u8,
    ]
}

impl Scene {
    /// Triangle lit by its own normal, turned to face up / outward.
    fn tri(&mut self, a: V3, b: V3, c: V3, rgb: [u8; 3], id: u32, hint: Option<V3>) {
        let mut n = cross(sub(b, a), sub(c, a));
        match hint {
            Some(h) if dot(n, h) < 0.0 => n = [-n[0], -n[1], -n[2]],
            None if n[1] < 0.0 => n = [-n[0], -n[1], -n[2]],
            _ => {}
        }
        self.tris.push(Tri {
            p: [a, b, c],
            rgb: shade(rgb, n),
            id,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn quad(&mut self, a: V3, b: V3, c: V3, d: V3, rgb: [u8; 3], id: u32, hint: Option<V3>) {
        self.tri(a, b, c, rgb, id, hint);
        self.tri(a, c, d, rgb, id, hint);
    }

    /// Axis-aligned box without its bottom face.
    fn cuboid(&mut self, min: V3, max: V3, rgb: [u8; 3], id: u32) {
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        self.quad(
            [x0, y1, z0],
            [x1, y1, z0],
            [x1, y1, z1],
            [x0, y1, z1],
            rgb,
            id,
            Some([0.0, 1.0, 0.0]),
        );
        self.quad(
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y1, z0],
            [x0, y1, z0],
            rgb,
            id,
            Some([0.0, 0.0, -1.0]),
        );
        self.quad(
            [x0, y0, z1],
            [x1, y0, z1],
            [x1, y1, z1],
            [x0, y1, z1],
            rgb,
            id,
            Some([0.0, 0.0, 1.0]),
        );
        self.quad(
            [x0, y0, z0],
            [x0, y0, z1],
            [x0, y1, z1],
            [x0, y1, z0],
            rgb,
            id,
            Some([-1.0, 0.0, 0.0]),
        );
        self.quad(
            [x1, y0, z0],
            [x1, y0, z1],
            [x1, y1, z1],
            [x1, y1, z0],
            rgb,
            id,
            Some([1.0, 0.0, 0.0]),
        );
    }
}

struct Palettes {
    map: HashMap<String, HashMap<String, [u8; 3]>>,
}

impl Palettes {
    fn new() -> Palettes {
        let mut map = HashMap::new();
        for p in builtin_palettes() {
            let slots = p
                .slots
                .iter()
                .filter_map(|(k, v)| parse_hex(v).map(|c| (k.clone(), c)))
                .collect();
            map.insert(p.key.clone(), slots);
        }
        Palettes { map }
    }

    fn get(&self, palette: &str, slot: &str) -> [u8; 3] {
        self.map
            .get(palette)
            .and_then(|p| p.get(slot))
            .copied()
            .unwrap_or_else(|| default_slot_color(slot))
    }
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn tint(c: [u8; 3], id: &str) -> [u8; 3] {
    let h = crate::hash::pick("preview", "tint", id, 0, 25) as i32 - 12;
    let f = |x: u8| (x as i32 + h).clamp(0, 255) as u8;
    [f(c[0]), f(c[1]), f(c[2])]
}

/// PNG bytes.
pub fn render_png(bp: &Blueprint, inst: &InstanceList, view: View, px: u32) -> Vec<u8> {
    render_png_with(bp, inst, view, px, &PreviewOptions::default())
}

pub fn render_png_with(
    bp: &Blueprint,
    inst: &InstanceList,
    view: View,
    px: u32,
    opts: &PreviewOptions,
) -> Vec<u8> {
    render_image(bp, inst, view, px, opts).png()
}

/// The four oblique views plus the top view, as PNGs, in the order top, N, E, S, W.
pub fn render_all(bp: &Blueprint, inst: &InstanceList, px: u32) -> Vec<(String, Vec<u8>)> {
    let mut v = vec![("top".to_string(), render_png(bp, inst, View::Top, px))];
    for (n, d) in [("n", Dir::N), ("e", Dir::E), ("s", Dir::S), ("w", Dir::W)] {
        v.push((n.to_string(), render_png(bp, inst, View::Oblique(d), px)));
    }
    v
}

fn build_scene(bp: &Blueprint, inst: &InstanceList) -> (Scene, Vec<Label>) {
    let pal = Palettes::new();
    let mut sc = Scene::default();
    // Terrain.
    let t = mesher::terrain_mesh(bp);
    for tri in t.indices.chunks(3) {
        let v = |i: u32| {
            let i = i as usize;
            [
                t.positions[i * 3] as f64,
                t.positions[i * 3 + 1] as f64,
                t.positions[i * 3 + 2] as f64,
            ]
        };
        let slot = PALETTE_SLOTS[t.slots[tri[0] as usize] as usize];
        sc.tri(
            v(tri[0]),
            v(tri[1]),
            v(tri[2]),
            default_slot_color(slot),
            0,
            None,
        );
    }
    let ctx = Ctx::new(bp);
    let mut labels = Vec::new();
    for (bi, b) in bp.buildings.iter().enumerate() {
        let id = bi as u32 + 1;
        let wall = tint(pal.get(&b.palette, "wall"), &b.id);
        let roof_c = pal.get(&b.palette, "roof");
        let finished = ctx.vis[bi] >= ctx.floors[bi];
        let fam = roof::family(&b.roof.kind);
        let mut top_y = ctx.base[bi];
        let mut sum = (0i64, 0i64, 0i64);
        for k in 0..ctx.levels() {
            let is_main = |x: i32, y: i32| ctx.main_at(k, x, y) == bi as i16;
            let is_tower = |x: i32, y: i32| ctx.tower_at(k, x, y) == bi as i16;
            let y0 = (ctx.base[bi] + k as i32 * LEVEL_MM) as f64;
            let y1 = y0 + LEVEL_MM as f64;
            for ty in 0..REGION_TILES {
                for tx in 0..REGION_TILES {
                    let tower = is_tower(tx, ty);
                    if !is_main(tx, ty) && !tower {
                        continue;
                    }
                    if k == 0 {
                        sum = (sum.0 + tx as i64, sum.1 + ty as i64, sum.2 + 1);
                    }
                    let exposed = !ctx.solid(k + 1, tx, ty);
                    // Roof height (mm above y1) at a corner.
                    let lift = |vx: i32, vy: i32| -> f64 {
                        if !exposed || !finished {
                            return 0.0;
                        }
                        if tower {
                            let own = |x: i32, y: i32| is_tower(x, y) && !ctx.solid(k + 1, x, y);
                            let d = roof::eave_distances(&own, vx, vy);
                            (roof::steps("conico", Axis::X, &d) * roof::rise_of("conico")) as f64
                        } else {
                            let own = |x: i32, y: i32| is_main(x, y) && !ctx.solid(k + 1, x, y);
                            let d = roof::eave_distances(&own, vx, vy);
                            (roof::steps(fam, b.roof.ridge_axis, &d) * roof::rise_of(fam)) as f64
                        }
                    };
                    let (x0, x1) = ((tx * TILE_MM) as f64, ((tx + 1) * TILE_MM) as f64);
                    let (z0, z1) = ((ty * TILE_MM) as f64, ((ty + 1) * TILE_MM) as f64);
                    let l = [
                        lift(tx, ty),
                        lift(tx + 1, ty),
                        lift(tx + 1, ty + 1),
                        lift(tx, ty + 1),
                    ];
                    let colour = if tower { tint(wall, "tower") } else { wall };
                    // Walls.
                    for d in [Dir::N, Dir::E, Dir::S, Dir::W] {
                        let (dx, dy) = d.delta();
                        if ctx.solid(k, tx + dx, ty + dy) {
                            continue;
                        }
                        let (a, bb, la, lb) = match d {
                            Dir::N => ([x0, z0], [x1, z0], l[0], l[1]),
                            Dir::E => ([x1, z0], [x1, z1], l[1], l[2]),
                            Dir::S => ([x1, z1], [x0, z1], l[2], l[3]),
                            Dir::W => ([x0, z1], [x0, z0], l[3], l[0]),
                        };
                        let bottom = if k == 0 { y0 - 400.0 } else { y0 };
                        let hint = Some([dx as f64, 0.0, dy as f64]);
                        sc.quad(
                            [a[0], bottom, a[1]],
                            [bb[0], bottom, bb[1]],
                            [bb[0], y1 + lb, bb[1]],
                            [a[0], y1 + la, a[1]],
                            colour,
                            id,
                            hint,
                        );
                    }
                    if exposed {
                        let c = if finished {
                            roof_c
                        } else {
                            pal.get(&b.palette, "stone")
                        };
                        let p = [
                            [x0, y1 + l[0], z0],
                            [x1, y1 + l[1], z0],
                            [x1, y1 + l[2], z1],
                            [x0, y1 + l[3], z1],
                        ];
                        if l[0] + l[2] >= l[1] + l[3] {
                            sc.tri(p[0], p[1], p[2], c, id, None);
                            sc.tri(p[0], p[2], p[3], c, id, None);
                        } else {
                            sc.tri(p[0], p[1], p[3], c, id, None);
                            sc.tri(p[1], p[2], p[3], c, id, None);
                        }
                        let peak = y1 as i32 + l.iter().copied().fold(0.0, f64::max) as i32;
                        top_y = top_y.max(peak);
                    }
                }
            }
        }
        // Bays and door as panels just outside the wall.
        let glass = pal.get(&b.palette, "glass");
        let door_c = pal.get(&b.palette, "door");
        let mut panel = |cell: [i32; 2], face: Dir, level: u8, kind: &str| {
            let k = level as usize;
            let (dx, dy) = face.delta();
            if ctx.main_at(k, cell[0], cell[1]) != bi as i16
                && ctx.tower_at(k, cell[0], cell[1]) != bi as i16
            {
                return;
            }
            if ctx.solid(k, cell[0] + dx, cell[1] + dy) {
                return;
            }
            let (u0, u1, v0, v1, c) = match kind {
                "porta" => (0.3, 0.7, 0.0, 2300.0, door_c),
                "vitrine" => (0.12, 0.88, 300.0, 2400.0, glass),
                "sacada" => (0.25, 0.75, 0.0, 2300.0, glass),
                _ => (0.3, 0.7, 900.0, 2100.0, glass),
            };
            let y = (ctx.base[bi] + k as i32 * LEVEL_MM) as f64;
            let (ex, ez) = ((cell[0] * TILE_MM) as f64, (cell[1] * TILE_MM) as f64);
            let t = TILE_MM as f64;
            let off = 40.0;
            let (a, bb) = match face {
                Dir::N => ([ex + u0 * t, ez - off], [ex + u1 * t, ez - off]),
                Dir::S => ([ex + u0 * t, ez + t + off], [ex + u1 * t, ez + t + off]),
                Dir::W => ([ex - off, ez + u0 * t], [ex - off, ez + u1 * t]),
                Dir::E => ([ex + t + off, ez + u0 * t], [ex + t + off, ez + u1 * t]),
            };
            let hint = Some([dx as f64, 0.0, dy as f64]);
            sc.quad(
                [a[0], y + v0, a[1]],
                [bb[0], y + v0, bb[1]],
                [bb[0], y + v1, bb[1]],
                [a[0], y + v1, a[1]],
                c,
                id,
                hint,
            );
            if kind == "sacada" {
                let rail = pal.get(&b.palette, "metal");
                let (ox, oz) = (dx as f64 * 700.0, dy as f64 * 700.0);
                sc.quad(
                    [a[0] + ox, y + 200.0, a[1] + oz],
                    [bb[0] + ox, y + 200.0, bb[1] + oz],
                    [bb[0] + ox, y + 1100.0, bb[1] + oz],
                    [a[0] + ox, y + 1100.0, a[1] + oz],
                    rail,
                    id,
                    hint,
                );
            }
        };
        panel(b.door.cell, b.door.face, b.door.level, "porta");
        for bay in &b.bays {
            if matches!(bay.kind.as_str(), "janela" | "vitrine" | "sacada" | "porta") {
                panel(bay.cell, bay.face, bay.level, &bay.kind);
            }
        }
        if sum.2 > 0 {
            let cx = (sum.0 * 2 + sum.2) as f64 * TILE_MM as f64 / (2.0 * sum.2 as f64);
            let cz = (sum.1 * 2 + sum.2) as f64 * TILE_MM as f64 / (2.0 * sum.2 as f64);
            let mut text = b.id.clone();
            if b.progress < 100 {
                text.push_str(&format!(" ({}%)", b.progress));
            }
            labels.push(Label {
                at: [cx, top_y as f64 + 300.0, cz],
                text,
                weight: sum.2,
            });
        }
    }
    instances(&mut sc, &pal, inst);
    (sc, labels)
}

struct Label {
    at: V3,
    text: String,
    weight: i64,
}

/// Simple stand-ins for the kit modules that matter at preview scale.
fn instances(sc: &mut Scene, pal: &Palettes, inst: &InstanceList) {
    for i in &inst.instances {
        let mut parts = i.key.split('/');
        let layer = parts.next().unwrap_or("");
        let case = parts.next().unwrap_or("");
        let (x, y, z) = (i.x_mm as f64, i.y_mm as f64, i.z_mm as f64);
        let c = |slot: &str| pal.get(&i.palette, slot);
        let along_x = i.rot % 2 == 0;
        let edge_box = |sc: &mut Scene, hgt: f64, half: f64, rgb: [u8; 3]| {
            let (hx, hz) = if along_x {
                (1000.0, half)
            } else {
                (half, 1000.0)
            };
            sc.cuboid([x - hx, y, z - hz], [x + hx, y + hgt, z + hz], rgb, 0);
        };
        match (layer, case) {
            ("arvore", p) => {
                let palm = matches!(p, "palmeira" | "coqueiro");
                let trunk_h = if palm { 3600.0 } else { 1500.0 };
                sc.cuboid(
                    [x - 150.0, y, z - 150.0],
                    [x + 150.0, y + trunk_h, z + 150.0],
                    c("trunk"),
                    0,
                );
                let (r, c0, c1) = if palm {
                    (1300.0, trunk_h - 200.0, trunk_h + 600.0)
                } else if p == "arbusto" {
                    (700.0, 0.0, 1000.0)
                } else if p == "pinheiro-estilizado" {
                    (800.0, 1000.0, 5200.0)
                } else {
                    (1000.0, 1300.0, 3900.0)
                };
                sc.cuboid([x - r, y + c0, z - r], [x + r, y + c1, z + r], c("leaf"), 0);
            }
            ("prop", "poste") => sc.cuboid(
                [x - 100.0, y, z - 100.0],
                [x + 100.0, y + 3500.0, z + 100.0],
                c("metal"),
                0,
            ),
            ("prop", "chafariz") => {
                sc.cuboid(
                    [x - 900.0, y, z - 900.0],
                    [x + 900.0, y + 500.0, z + 900.0],
                    c("stone"),
                    0,
                );
                sc.cuboid(
                    [x - 600.0, y + 500.0, z - 600.0],
                    [x + 600.0, y + 520.0, z + 600.0],
                    c("water"),
                    0,
                );
            }
            ("prop", _) => sc.cuboid(
                [x - 350.0, y, z - 350.0],
                [x + 350.0, y + 700.0, z + 350.0],
                c("wood"),
                0,
            ),
            ("enfeite", "cerca") | ("enfeite", "portao") => edge_box(sc, 900.0, 60.0, c("wood")),
            ("enfeite", "muro") => edge_box(sc, 1200.0, 120.0, c("wall")),
            ("enfeite", "sebe") => edge_box(sc, 1000.0, 250.0, c("leaf")),
            ("enfeite", "ponte-guarda") => edge_box(sc, 1000.0, 80.0, c("stone")),
            ("enfeite", "ponte") => sc.cuboid(
                [x - 1000.0, y - 300.0, z - 1000.0],
                [x + 1000.0, y, z + 1000.0],
                c("stone"),
                0,
            ),
            ("enfeite", "canteiro") => sc.cuboid(
                [x - 650.0, y, z - 650.0],
                [x + 650.0, y + 300.0, z + 650.0],
                c("leaf"),
                0,
            ),
            ("enfeite", "vaso") => sc.cuboid(
                [x - 300.0, y, z - 300.0],
                [x + 300.0, y + 700.0, z + 300.0],
                c("leaf"),
                0,
            ),
            ("enfeite", "chamine") => sc.cuboid(
                [x - 250.0, y - 400.0, z - 250.0],
                [x + 250.0, y + 1200.0, z + 250.0],
                c("stone"),
                0,
            ),
            ("enfeite", "meio-fio-poste") => sc.cuboid(
                [x - 80.0, y, z - 80.0],
                [x + 80.0, y + 3500.0, z + 80.0],
                c("metal"),
                0,
            ),
            ("obra", "andaime") => {
                sc.cuboid(
                    [x - 90.0, y, z - 90.0],
                    [x + 90.0, y + 3000.0, z + 90.0],
                    c("wood"),
                    0,
                );
            }
            ("obra", "fundacao") => sc.cuboid(
                [x - 1000.0, y, z - 1000.0],
                [x + 1000.0, y + 300.0, z + 1000.0],
                c("stone"),
                0,
            ),
            _ => {}
        }
    }
}

struct Cam {
    r: V3,
    u: V3,
    d: V3,
}

fn camera(view: View) -> Cam {
    match view {
        View::Top => Cam {
            r: [1.0, 0.0, 0.0],
            u: [0.0, 0.0, -1.0],
            d: [0.0, -1.0, 0.0],
        },
        View::Oblique(side) => {
            let f: V3 = match side {
                Dir::S => [0.0, 0.0, -1.0],
                Dir::N => [0.0, 0.0, 1.0],
                Dir::E => [-1.0, 0.0, 0.0],
                Dir::W => [1.0, 0.0, 0.0],
            };
            let e = 38f64.to_radians();
            let r = [-f[2], 0.0, f[0]];
            let u = [f[0] * e.sin(), e.cos(), f[2] * e.sin()];
            let d = [f[0] * e.cos(), -e.sin(), f[2] * e.cos()];
            Cam { r, u, d }
        }
    }
}

fn render_image(
    bp: &Blueprint,
    inst: &InstanceList,
    view: View,
    px: u32,
    opts: &PreviewOptions,
) -> Image {
    let px = px.clamp(64, 4096) as i32;
    let top = matches!(view, View::Top);
    let (w, h) = if top { (px, px) } else { (px, px * 3 / 4) };
    let (scene, mut labels) = build_scene(bp, inst);
    let cam = camera(view);
    let proj = |p: V3| (dot(p, cam.r), -dot(p, cam.u), dot(p, cam.d));
    // Fit: the region square for the top view, everything for oblique ones.
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    if top {
        let s = (REGION_TILES * TILE_MM) as f64;
        (minx, miny, maxx, maxy) = (0.0, 0.0, s, s);
    } else {
        for t in &scene.tris {
            for p in t.p {
                let (x, y, _) = proj(p);
                minx = minx.min(x);
                maxx = maxx.max(x);
                miny = miny.min(y);
                maxy = maxy.max(y);
            }
        }
    }
    let margin = if top && opts.grid { 28.0 } else { 10.0 };
    let bottom_extra = if opts.grid { 14.0 } else { 0.0 };
    let scale = ((w as f64 - 2.0 * margin) / (maxx - minx).max(1.0))
        .min((h as f64 - 2.0 * margin - bottom_extra) / (maxy - miny).max(1.0));
    let ox = margin + ((w as f64 - 2.0 * margin) - (maxx - minx) * scale) / 2.0 - minx * scale;
    let oy = margin + ((h as f64 - 2.0 * margin - bottom_extra) - (maxy - miny) * scale) / 2.0
        - miny * scale;
    let to_screen = |p: V3| {
        let (x, y, d) = proj(p);
        (x * scale + ox, y * scale + oy, d)
    };

    let sky = if top {
        [0x2B, 0x33, 0x38]
    } else {
        [0xDD, 0xE8, 0xEE]
    };
    let mut img = Image::new(w, h, sky);
    let mut zbuf = vec![i64::MAX; (w * h) as usize];
    let mut ids = vec![u32::MAX; (w * h) as usize];
    for t in &scene.tris {
        let v: Vec<(i64, i64, i64)> =
            t.p.iter()
                .map(|p| {
                    let (x, y, d) = to_screen(*p);
                    (
                        (x * 256.0).round() as i64,
                        (y * 256.0).round() as i64,
                        d.round() as i64,
                    )
                })
                .collect();
        raster(
            &mut img,
            &mut zbuf,
            &mut ids,
            [v[0], v[1], v[2]],
            t.rgb,
            t.id,
        );
    }
    // Outlines: building silhouettes and depth steps.
    let mut edge = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                let (a, b) = (ids[i], ids[j]);
                if a == u32::MAX && b == u32::MAX {
                    continue;
                }
                let step = (zbuf[i].min(i64::MAX / 4) - zbuf[j].min(i64::MAX / 4)).abs() > 2500;
                let near_obj = if zbuf[i] < zbuf[j] { a } else { b };
                if (a != b && (a > 0 && a != u32::MAX || b > 0 && b != u32::MAX))
                    || (step && near_obj > 0 && near_obj != u32::MAX)
                {
                    if zbuf[i] >= zbuf[j] {
                        edge[i] = true;
                    } else {
                        edge[j] = true;
                    }
                }
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            if edge[(y * w + x) as usize] {
                img.blend(x, y, [0x26, 0x2A, 0x2C], 150);
            }
        }
    }

    let scale_txt = if px >= 600 { 2 } else { 1 };
    if opts.grid {
        let ink = [0xF4, 0xF1, 0xE8];
        if top {
            let region_px = |t: i32| (t * TILE_MM) as f64 * scale;
            for t in (0..=REGION_TILES).step_by(8) {
                let sx = (region_px(t) + ox).round() as i32;
                let sy = (region_px(t) + oy).round() as i32;
                let s = t.to_string();
                let tw = font::width(&s, 1);
                font::draw(
                    &mut img,
                    &s,
                    sx - tw / 2,
                    (margin as i32 - 12).max(1),
                    1,
                    ink,
                );
                font::draw(
                    &mut img,
                    &s,
                    (margin as i32 - 4 - tw).max(1),
                    sy - 3,
                    1,
                    ink,
                );
                if t % CHUNK_TILES == 0 && t > 0 && t < REGION_TILES {
                    let (a, b) = (
                        (oy).round() as i32,
                        (region_px(REGION_TILES) + oy).round() as i32,
                    );
                    for yy in a..b {
                        img.blend(sx, yy, [255, 255, 255], 70);
                    }
                    let (a, b) = (
                        (ox).round() as i32,
                        (region_px(REGION_TILES) + ox).round() as i32,
                    );
                    for xx in a..b {
                        img.blend(xx, sy, [255, 255, 255], 70);
                    }
                }
            }
        }
        let caption = match view {
            View::Top => format!("region {},{} - top, N up", bp.region[0], bp.region[1]),
            View::Oblique(d) => format!("region {},{} - from {:?}", bp.region[0], bp.region[1], d),
        };
        let c = if top { ink } else { [0x26, 0x3C, 0x37] };
        font::draw(&mut img, &caption, 6, h - 7 * scale_txt - 4, scale_txt, c);
    }

    if opts.labels {
        labels.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.text.cmp(&b.text)));
        let mut taken: Vec<[i32; 4]> = Vec::new();
        for l in &labels {
            let (sx, sy, _) = to_screen(l.at);
            let tw = font::width(&l.text, scale_txt);
            let th = 7 * scale_txt;
            // Above the anchor first, then below, then shifted sideways.
            let (ax, ay) = (sx.round() as i32, sy.round() as i32);
            let candidates = [
                (ax - tw / 2 - 3, ay - th - 8),
                (ax - tw / 2 - 3, ay + 4),
                (ax - tw - 6, ay - th - 8),
                (ax, ay - th - 8),
                (ax - tw - 6, ay + 4),
                (ax, ay + 4),
            ];
            let fits = |r: &[i32; 4]| {
                r[0] >= 0
                    && r[1] >= 0
                    && r[2] <= w
                    && r[3] <= h
                    && !taken
                        .iter()
                        .any(|t| r[0] < t[2] && t[0] < r[2] && r[1] < t[3] && t[1] < r[3])
            };
            let Some((x0, y0)) = candidates
                .iter()
                .copied()
                .find(|(x0, y0)| fits(&[*x0, *y0, x0 + tw + 6, y0 + th + 6]))
            else {
                continue;
            };
            let r = [x0, y0, x0 + tw + 6, y0 + th + 6];
            taken.push(r);
            img.fill_rect(r[0], r[1], r[2], r[3], [0xFF, 0xFD, 0xF6], 225);
            for x in r[0]..r[2] {
                img.put(x, r[3] - 1, [0x26, 0x3C, 0x37]);
            }
            let (a, b) = if ay >= r[3] {
                (r[3], ay)
            } else if ay < r[1] {
                (ay, r[1] - 1)
            } else {
                (0, -1)
            };
            for yy in a..=b {
                img.put(ax, yy, [0x26, 0x3C, 0x37]);
            }
            font::draw(
                &mut img,
                &l.text,
                x0 + 3,
                y0 + 3,
                scale_txt,
                [0x26, 0x3C, 0x37],
            );
        }
    }
    img
}

fn edge_fn(a: (i64, i64), b: (i64, i64), p: (i64, i64)) -> i64 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

fn raster(
    img: &mut Image,
    zbuf: &mut [i64],
    ids: &mut [u32],
    v: [(i64, i64, i64); 3],
    rgb: [u8; 3],
    id: u32,
) {
    let (a, b, c) = ((v[0].0, v[0].1), (v[1].0, v[1].1), (v[2].0, v[2].1));
    let area = edge_fn(a, b, c);
    if area == 0 {
        return;
    }
    let (w, h) = (img.w as i64, img.h as i64);
    let minx = (a.0.min(b.0).min(c.0) >> 8).max(0);
    let maxx = ((a.0.max(b.0).max(c.0) >> 8) + 1).min(w - 1);
    let miny = (a.1.min(b.1).min(c.1) >> 8).max(0);
    let maxy = ((a.1.max(b.1).max(c.1) >> 8) + 1).min(h - 1);
    let sign = if area > 0 { 1 } else { -1 };
    let area = area * sign;
    for py in miny..=maxy {
        for pxl in minx..=maxx {
            let p = (pxl * 256 + 128, py * 256 + 128);
            let w0 = edge_fn(b, c, p) * sign;
            let w1 = edge_fn(c, a, p) * sign;
            let w2 = edge_fn(a, b, p) * sign;
            if w0 < 0 || w1 < 0 || w2 < 0 {
                continue;
            }
            let z = ((w0 as i128 * v[0].2 as i128
                + w1 as i128 * v[1].2 as i128
                + w2 as i128 * v[2].2 as i128)
                / area as i128) as i64;
            let i = (py * w + pxl) as usize;
            if z < zbuf[i] {
                zbuf[i] = z;
                ids[i] = id;
                img.put(pxl as i32, py as i32, rgb);
            }
        }
    }
}
