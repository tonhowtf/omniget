//! The kit baked into isometric sprites for the city's lite mode (estudo 99,
//! arquivo 09 §5.5): the 2D renderer draws the same instances with one sprite
//! per module instead of a mesh.
//!
//! Projection: orthographic, camera from the south-east at 30° of pitch,
//! which is exactly the 2D renderer's 2:1 diamond (a 2 m tile is 64 × 32 px).
//! One metre east is `(+16, +8)` px, one metre south `(−16, +8)` px and one
//! metre up `−19.6` px. The module origin (the dual cell centre on the floor)
//! is each frame's pivot, so a sprite lands where the 2D camera projects the
//! instance.
//!
//! Colour: every module is split into one frame per palette slot. A frame is
//! grey (the fixed light's shade, with the depth outline) with the slot's
//! coverage in alpha, so the renderer's per-sprite tint paints it with any
//! palette. Layers of one module never overlap (each subsample belongs to the
//! nearest triangle), so drawing them in a row rebuilds the module.
//!
//! Rotations: a module turned by `rot` under a camera turned by `c` is the
//! module turned by `rot + c`, so 4 frames per key cover every view.
//!
//! Output: the atlas format `src/lib/world/render/atlas.ts` reads (version 1,
//! `pages`, `frames` with `pivot`), plus a `city3d` block mapping each key and
//! rotation to its slot layers.

use crate::canon::{CTri, CanonMesh, Q};
use crate::kit::{self, Kit};
use crate::palette::slot_index;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Bumped whenever the baked pixels change.
pub const BAKE_VERSION: &str = "sprites-1/ortho30-ss3";

/// Pixels per metre along the ground diagonal: a 2 m tile is 64 px wide.
pub const PX_PER_M: f64 = 16.0 * std::f64::consts::SQRT_2;
/// Screen pixels one metre of height takes (cos 30° of `PX_PER_M`).
pub const Z_PX_PER_M: f64 = 19.595_917_942_265_423;
/// Side of an atlas page.
pub const PAGE: u32 = 2048;
/// Supersampling per axis.
const SS: usize = 3;
/// Depth jump (metres) that draws an outline on the nearer side.
const OUTLINE_JUMP: f64 = 0.3;
const OUTLINE_SHADE: f64 = 0.55;

/// Frame id of the ground tile (pivot at the tile's north corner).
pub const TILE_FRAME: &str = "lite/tile";
/// How far the ground tile's sides reach down, metres.
pub const TILE_SKIRT_M: f64 = 4.0;

/// Camera direction (towards the viewer) and light, in world axes
/// (x = east, y = up, z = south).
fn to_cam() -> [f64; 3] {
    let c = 30f64.to_radians().cos() / std::f64::consts::SQRT_2;
    [c, 0.5, c]
}

fn light() -> [f64; 3] {
    let l = [0.3, 0.85, 0.6];
    let n: f64 = l[0] * l[0] + l[1] * l[1] + l[2] * l[2];
    let n = n.sqrt();
    [l[0] / n, l[1] / n, l[2] / n]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Shade of a face: tops come out at 1.0, the south face a bit darker, the
/// east face darker still.
fn shade_of(n: [f64; 3]) -> f64 {
    let l = light();
    let lam = dot(n, l).max(0.0);
    let top = 0.55 + 0.45 * l[1];
    ((0.55 + 0.45 * lam) / top).min(1.0)
}

/// Screen position (px, zoom 1, origin at the module origin) and depth
/// (metres towards the camera) of a quantised point.
fn project(p: [i32; 3]) -> [f64; 3] {
    let (x, y, z) = (p[0] as f64 / Q, p[1] as f64 / Q, p[2] as f64 / Q);
    [
        (x - z) * PX_PER_M / std::f64::consts::SQRT_2,
        (x + z) * PX_PER_M / (2.0 * std::f64::consts::SQRT_2) - y * Z_PX_PER_M,
        dot([x, y, z], to_cam()),
    ]
}

/// One slot layer of a rasterised mesh: grey shade in RGB, coverage in alpha.
pub struct Layer {
    pub slot: u8,
    pub w: u32,
    pub h: u32,
    /// Pivot: the module origin, px from the layer's top-left.
    pub pivot: [i32; 2],
    pub rgba: Vec<u8>,
}

/// Rasterises a mesh (already turned) into its slot layers, each trimmed to
/// its own pixels. Empty meshes give no layers.
pub fn rasterize(mesh: &CanonMesh) -> Vec<Layer> {
    let cam = to_cam();
    let tris: Vec<(&CTri, [[f64; 3]; 3], f64)> = mesh
        .tris
        .iter()
        .filter_map(|t| {
            let n = t.normal();
            if dot(n, cam) <= 1e-6 {
                return None;
            }
            Some((
                t,
                [project(t.v[0]), project(t.v[1]), project(t.v[2])],
                shade_of(n),
            ))
        })
        .collect();
    if tris.is_empty() {
        return vec![];
    }
    let mut lo = [f64::MAX; 2];
    let mut hi = [f64::MIN; 2];
    for (_, p, _) in &tris {
        for q in p {
            lo[0] = lo[0].min(q[0]);
            lo[1] = lo[1].min(q[1]);
            hi[0] = hi[0].max(q[0]);
            hi[1] = hi[1].max(q[1]);
        }
    }
    // Integer pixel box around the projection; the origin sits on a pixel corner.
    let x0 = lo[0].floor() as i32 - 1;
    let y0 = lo[1].floor() as i32 - 1;
    let w = (hi[0].ceil() as i32 + 1 - x0).max(1) as usize;
    let h = (hi[1].ceil() as i32 + 1 - y0).max(1) as usize;
    let (sw, sh) = (w * SS, h * SS);
    let mut depth = vec![f64::MIN; sw * sh];
    let mut slot = vec![u8::MAX; sw * sh];
    let mut shade = vec![0f32; sw * sh];
    let ss = SS as f64;
    for (t, p, s) in &tris {
        let q: Vec<[f64; 3]> = p
            .iter()
            .map(|v| [(v[0] - x0 as f64) * ss, (v[1] - y0 as f64) * ss, v[2]])
            .collect();
        let area =
            (q[1][0] - q[0][0]) * (q[2][1] - q[0][1]) - (q[2][0] - q[0][0]) * (q[1][1] - q[0][1]);
        if area.abs() < 1e-12 {
            continue;
        }
        let minx = q
            .iter()
            .map(|v| v[0])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.0) as usize;
        let maxx = (q.iter().map(|v| v[0]).fold(f64::MIN, f64::max).ceil() as usize).min(sw - 1);
        let miny = q
            .iter()
            .map(|v| v[1])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.0) as usize;
        let maxy = (q.iter().map(|v| v[1]).fold(f64::MIN, f64::max).ceil() as usize).min(sh - 1);
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((q[1][0] - fx) * (q[2][1] - fy) - (q[2][0] - fx) * (q[1][1] - fy)) / area;
                let w1 = ((q[2][0] - fx) * (q[0][1] - fy) - (q[0][0] - fx) * (q[2][1] - fy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                    continue;
                }
                let d = w0 * q[0][2] + w1 * q[1][2] + w2 * q[2][2];
                let i = y * sw + x;
                if d > depth[i] {
                    depth[i] = d;
                    slot[i] = t.slot;
                    shade[i] = *s as f32;
                }
            }
        }
    }
    // Outline on the nearer side of a depth jump (overhangs, eaves, frames).
    let mut out = shade.clone();
    for y in 1..sh.saturating_sub(1) {
        for x in 1..sw.saturating_sub(1) {
            let i = y * sw + x;
            let d = depth[i];
            if d == f64::MIN {
                continue;
            }
            let edge = [i - 1, i + 1, i - sw, i + sw]
                .iter()
                .any(|&j| depth[j] != f64::MIN && d - depth[j] > OUTLINE_JUMP);
            if edge {
                out[i] = (shade[i] as f64 * OUTLINE_SHADE) as f32;
            }
        }
    }
    let shade = out;

    let mut slots: Vec<u8> = slot.iter().copied().filter(|s| *s != u8::MAX).collect();
    slots.sort();
    slots.dedup();
    let n = (SS * SS) as f64;
    let mut layers = vec![];
    for s in slots {
        let mut rgba = vec![0u8; w * h * 4];
        let (mut lx0, mut ly0, mut lx1, mut ly1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for py in 0..h {
            for px in 0..w {
                let (mut cov, mut acc) = (0usize, 0f64);
                for dy in 0..SS {
                    for dx in 0..SS {
                        let i = (py * SS + dy) * sw + px * SS + dx;
                        if slot[i] == s {
                            cov += 1;
                            acc += shade[i] as f64;
                        }
                    }
                }
                if cov == 0 {
                    continue;
                }
                let g = ((acc / cov as f64) * 255.0).round().clamp(0.0, 255.0) as u8;
                let o = (py * w + px) * 4;
                rgba[o] = g;
                rgba[o + 1] = g;
                rgba[o + 2] = g;
                rgba[o + 3] = ((cov as f64 / n) * 255.0).round() as u8;
                lx0 = lx0.min(px);
                ly0 = ly0.min(py);
                lx1 = lx1.max(px);
                ly1 = ly1.max(py);
            }
        }
        if lx0 == usize::MAX {
            continue;
        }
        let (tw, th) = (lx1 - lx0 + 1, ly1 - ly0 + 1);
        let mut trimmed = vec![0u8; tw * th * 4];
        for y in 0..th {
            let src = ((ly0 + y) * w + lx0) * 4;
            trimmed[y * tw * 4..(y + 1) * tw * 4].copy_from_slice(&rgba[src..src + tw * 4]);
        }
        layers.push(Layer {
            slot: s,
            w: tw as u32,
            h: th as u32,
            pivot: [-x0 - lx0 as i32, -y0 - ly0 as i32],
            rgba: trimmed,
        });
    }
    layers
}

// ── simple solids for the frames the kit does not carry ─────────────────────

fn m(v: f64) -> i32 {
    (v * Q).round() as i32
}

/// A quad, wound so its normal points along `out`.
fn quad(tris: &mut Vec<CTri>, slot: u8, c: [[f64; 3]; 4], out: [f64; 3]) {
    let v = c.map(|p| [m(p[0]), m(p[1]), m(p[2])]);
    for (a, b, d) in [(0, 1, 2), (0, 2, 3)] {
        let mut t = [v[a], v[b], v[d]];
        if let Some(ct) = CTri::new(slot, t) {
            if dot(ct.normal(), out) < 0.0 {
                t.swap(1, 2);
            }
        }
        if let Some(ct) = CTri::new(slot, t) {
            tris.push(ct);
        }
    }
}

fn cuboid(tris: &mut Vec<CTri>, slot: u8, lo: [f64; 3], hi: [f64; 3]) {
    let [x0, y0, z0] = lo;
    let [x1, y1, z1] = hi;
    quad(
        tris,
        slot,
        [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
        [0.0, 1.0, 0.0],
    );
    quad(
        tris,
        slot,
        [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        [1.0, 0.0, 0.0],
    );
    quad(
        tris,
        slot,
        [[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]],
        [-1.0, 0.0, 0.0],
    );
    quad(
        tris,
        slot,
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        [0.0, 0.0, 1.0],
    );
    quad(
        tris,
        slot,
        [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]],
        [0.0, 0.0, -1.0],
    );
}

/// The ground tile: a 2 × 2 m block from the origin (its north corner on
/// screen) to the south-east, top at y = 0, in the `ground` slot. Its sides
/// run [`TILE_SKIRT_M`] down, so a step in the terrain shows a bank instead
/// of a hole, and it overlaps its neighbours by a few centimetres so the
/// baked chunks have no seams.
pub fn tile_mesh() -> CanonMesh {
    let mut tris = vec![];
    let s = slot_index("ground").unwrap_or(7);
    let e = 0.04;
    cuboid(
        &mut tris,
        s,
        [-e, -TILE_SKIRT_M, -e],
        [2.0 + e, 0.0, 2.0 + e],
    );
    tris.sort();
    CanonMesh { tris }
}

/// A person marker: body in `cloth`, head in `trim` (the renderer tints both
/// per person), about 1.7 m tall, centred on the origin.
pub fn person_mesh() -> CanonMesh {
    let mut tris = vec![];
    let cloth = slot_index("cloth").unwrap_or(15);
    let head = slot_index("trim").unwrap_or(2);
    cuboid(&mut tris, cloth, [-0.22, 0.0, -0.16], [0.22, 1.3, 0.16]);
    cuboid(&mut tris, head, [-0.15, 1.33, -0.15], [0.15, 1.68, 0.15]);
    tris.sort();
    CanonMesh { tris }
}

// ── atlas ────────────────────────────────────────────────────────────────

/// A baked atlas: the JSON manifest and its PNG pages, in `pages` order.
pub struct BakedAtlas {
    pub manifest: Value,
    pub pages: Vec<Vec<u8>>,
}

struct Placed {
    id: String,
    layer: Layer,
    page: usize,
    x: u32,
    y: u32,
}

/// Shelf packing, tallest first, 1 px gutter. Deterministic: ties by id.
fn pack(mut items: Vec<(String, Layer)>) -> (Vec<Placed>, Vec<(u32, u32)>) {
    items.sort_by(|a, b| {
        b.1.h
            .cmp(&a.1.h)
            .then(b.1.w.cmp(&a.1.w))
            .then(a.0.cmp(&b.0))
    });
    let mut placed = vec![];
    let mut pages: Vec<(u32, u32)> = vec![];
    let (mut page, mut x, mut y, mut shelf) = (0usize, 1u32, 1u32, 0u32);
    let mut used_h = 0u32;
    for (id, layer) in items {
        let (w, h) = (layer.w.min(PAGE - 2), layer.h.min(PAGE - 2));
        if x + w + 1 > PAGE {
            x = 1;
            y += shelf + 1;
            shelf = 0;
        }
        if y + h + 1 > PAGE {
            pages.push((PAGE, used_h.max(1)));
            page += 1;
            x = 1;
            y = 1;
            shelf = 0;
            used_h = 0;
        }
        placed.push(Placed {
            id,
            layer,
            page,
            x,
            y,
        });
        x += w + 1;
        shelf = shelf.max(h);
        used_h = used_h.max(y + h + 1);
    }
    pages.push((PAGE, used_h.max(1)));
    // Pages are cut to their used height, rounded up to a multiple of 64.
    let pages = pages
        .into_iter()
        .map(|(w, h)| (w, (h.div_ceil(64) * 64).min(PAGE)))
        .collect();
    (placed, pages)
}

fn frame_id(key: &str, rot: u8, slot: u8) -> String {
    format!("{key}@{rot}#{slot}")
}

/// Bakes every module of a built kit (full detail, 4 rotations) plus the
/// ground tile and the person marker. Aliased keys point at the frames of
/// the key that serves them. Far variants (`#lod1`) are not baked: a sprite
/// costs the same whatever its triangle count, so the renderer maps them to
/// the full module.
pub fn bake_kit(kit: &Kit) -> BakedAtlas {
    let mut items: Vec<(String, Layer)> = vec![];
    let mut modules: BTreeMap<String, Vec<Vec<(String, u8)>>> = BTreeMap::new();
    for module in &kit.modules {
        let layer = module.key.split('/').next().unwrap_or("");
        let overlay = matches!(layer, "porta" | "bay");
        let mut rots = vec![];
        for rot in 0..4u8 {
            // Facade overlays face north at rot 0: turned away from the camera
            // (north, west) they are hidden behind their own building.
            if overlay && (rot == 0 || rot == 3) {
                rots.push(vec![]);
                continue;
            }
            let mesh = module.mesh.placed(rot, 0, 0, 0);
            let mut frames = vec![];
            for l in rasterize(&mesh) {
                let id = frame_id(&module.key, rot, l.slot);
                frames.push((id.clone(), l.slot));
                items.push((id, l));
            }
            rots.push(frames);
        }
        modules.insert(module.key.clone(), rots);
    }
    let mut aliases = Map::new();
    for (asked, served) in &kit.aliases {
        aliases.insert(asked.clone(), Value::String(served.clone()));
    }
    let tile = rasterize(&tile_mesh());
    for l in tile {
        items.push((TILE_FRAME.to_string(), l));
    }
    let mut person = vec![];
    for l in rasterize(&person_mesh()) {
        let id = format!("lite/person#{}", l.slot);
        person.push(json!([id, l.slot]));
        items.push((id, l));
    }

    let (placed, sizes) = pack(items);
    let mut canvases: Vec<Vec<u8>> = sizes
        .iter()
        .map(|(w, h)| vec![0u8; (*w * *h * 4) as usize])
        .collect();
    let mut frames = Map::new();
    for p in &placed {
        let (pw, ph) = sizes[p.page];
        let (w, h) = (p.layer.w.min(pw - p.x), p.layer.h.min(ph - p.y));
        let buf = &mut canvases[p.page];
        for row in 0..h {
            let src = (row * p.layer.w * 4) as usize;
            let dst = (((p.y + row) * pw + p.x) * 4) as usize;
            buf[dst..dst + (w * 4) as usize]
                .copy_from_slice(&p.layer.rgba[src..src + (w * 4) as usize]);
        }
        frames.insert(
            p.id.clone(),
            json!({
                "page": p.page,
                "x": p.x,
                "y": p.y,
                "w": w,
                "h": h,
                "pivot": p.layer.pivot,
                "z_base": 0,
            }),
        );
    }
    let pages: Vec<Vec<u8>> = canvases
        .iter()
        .zip(&sizes)
        .map(|(c, (w, h))| crate::raster::png(c, *w, *h))
        .collect();
    let page_names: Vec<String> = (0..pages.len())
        .map(|i| format!("{}-{i}.png", kit.style))
        .collect();
    let modules: Map<String, Value> = modules
        .into_iter()
        .map(|(k, rots)| {
            (
                k,
                Value::Array(
                    rots.into_iter()
                        .map(|fs| {
                            Value::Array(fs.into_iter().map(|(id, s)| json!([id, s])).collect())
                        })
                        .collect(),
                ),
            )
        })
        .collect();
    let manifest = json!({
        "version": 1,
        "pages": page_names,
        "frames": frames,
        "city3d": {
            "style": kit.style,
            "kit_hash": kit.hash,
            "bake": BAKE_VERSION,
            "px_per_m": PX_PER_M,
            "z_px_per_m": Z_PX_PER_M,
            "tile": TILE_FRAME,
            "person": person,
            "modules": modules,
            "aliases": aliases,
            "missing": kit.missing,
        },
    });
    BakedAtlas { manifest, pages }
}

/// Builds the style's kit and bakes it.
pub fn bake_style(style: &str) -> Result<BakedAtlas, String> {
    let kit = kit::build_kit(style)?;
    Ok(bake_kit(&kit))
}

/// Cache name of a style's atlas: the kit hash plus the bake version.
pub fn atlas_hash(style: &str) -> String {
    let mut h = Sha256::new();
    h.update(kit::kit_hash(style).as_bytes());
    h.update(BAKE_VERSION.as_bytes());
    crate::canon::hex(&h.finalize())
}

/// The style's atlas from `dir` (`atlas-<style>-<hash>.json` and its pages),
/// baked on a miss.
pub fn atlas_cached(dir: &Path, style: &str) -> Result<BakedAtlas, String> {
    let stem = format!("atlas-{style}-{}", &atlas_hash(style)[..16]);
    let json_path = dir.join(format!("{stem}.json"));
    if let Ok(text) = std::fs::read_to_string(&json_path) {
        if let Ok(manifest) = serde_json::from_str::<Value>(&text) {
            let n = manifest["pages"].as_array().map(|a| a.len()).unwrap_or(0);
            let pages: Vec<Vec<u8>> = (0..n)
                .filter_map(|i| std::fs::read(dir.join(format!("{stem}-{i}.png"))).ok())
                .collect();
            if n > 0 && pages.len() == n {
                return Ok(BakedAtlas { manifest, pages });
            }
        }
    }
    let baked = bake_style(style)?;
    if std::fs::create_dir_all(dir).is_ok() {
        let mut ok = true;
        for (i, p) in baked.pages.iter().enumerate() {
            ok &= std::fs::write(dir.join(format!("{stem}-{i}.png")), p).is_ok();
        }
        // The manifest goes last: its presence means the pages are complete.
        if ok {
            let tmp = json_path.with_extension("json.tmp");
            if std::fs::write(&tmp, baked.manifest.to_string()).is_ok() {
                let _ = std::fs::rename(&tmp, &json_path);
            }
        }
    }
    Ok(baked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_is_the_2d_diamond() {
        let l = rasterize(&tile_mesh());
        assert_eq!(l.len(), 1);
        let t = &l[0];
        // A 64 px diamond (plus the overlap) with its skirt below, pivot on
        // the north corner (top middle).
        assert!((64..=70).contains(&t.w), "w {}", t.w);
        let skirt = (TILE_SKIRT_M * Z_PX_PER_M) as u32;
        assert!((32 + skirt..=38 + skirt).contains(&t.h), "h {}", t.h);
        assert!(
            (t.pivot[0] - t.w as i32 / 2).abs() <= 1,
            "pivot {:?}",
            t.pivot
        );
        assert!((0..=3).contains(&t.pivot[1]), "pivot {:?}", t.pivot);
    }

    #[test]
    fn person_layers_are_deterministic() {
        let a = rasterize(&person_mesh());
        let b = rasterize(&person_mesh());
        assert_eq!(a.len(), 2);
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.rgba, y.rgba);
            assert_eq!(x.pivot, y.pivot);
        }
    }
}
