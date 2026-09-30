//! Passo 7: footprint on the ground (tiles), collision shape (never the
//! visual mesh), and which side is the front.

use crate::model::*;
use crate::types::{AssetClass, Collision, Footprint, LEVEL_MM, TILE_MM};

fn mm(v: f32) -> i32 {
    (v * 1000.0).round() as i32
}

/// Triangles of the final model in asset space (pivot at base centre).
pub fn world_triangles(model: &Model) -> Vec<[[f32; 3]; 3]> {
    let world = model.world_matrices();
    let reach = model.reachable();
    let mut out = vec![];
    for (ni, node) in model.nodes.iter().enumerate() {
        if !reach[ni] {
            continue;
        }
        let Some(mesh) = node.mesh.and_then(|m| model.meshes.get(m)) else {
            continue;
        };
        let skin = node
            .skin
            .and_then(|s| model.skins.get(s))
            .map(|s| skin_matrices(s, &world));
        for p in &mesh.prims {
            let wp: Vec<[f32; 3]> = (0..p.pos.len())
                .map(|i| match (&skin, &p.joints, &p.weights) {
                    (Some(sm), Some(j), Some(w)) => skin_point(sm, j[i], w[i], p.pos[i]),
                    _ => transform_point(&world[ni], p.pos[i]),
                })
                .collect();
            for t in p.idx.chunks_exact(3) {
                out.push([wp[t[0] as usize], wp[t[1] as usize], wp[t[2] as usize]]);
            }
        }
    }
    out
}

/// Tiles covered by the top-down projection, sampled on a 0.5 m raster.
pub fn footprint(tris: &[[[f32; 3]; 3]], bb: &Aabb) -> Footprint {
    let size = bb.size();
    let tile = TILE_MM as f32 / 1000.0;
    let w_tiles = ((size[0] / tile).ceil() as i32).max(1);
    let d_tiles = ((size[2] / tile).ceil() as i32).max(1);
    // Tiles are laid out centred on the pivot.
    let x0 = -(w_tiles as f32) * tile * 0.5;
    let z0 = -(d_tiles as f32) * tile * 0.5;
    let sub = 4; // samples per tile side
    let step = tile / sub as f32;
    let gw = (w_tiles * sub) as usize;
    let gd = (d_tiles * sub) as usize;
    let mut hit = vec![false; gw * gd];
    for t in tris {
        let (a, b, c) = ([t[0][0], t[0][2]], [t[1][0], t[1][2]], [t[2][0], t[2][2]]);
        let minx = a[0].min(b[0]).min(c[0]);
        let maxx = a[0].max(b[0]).max(c[0]);
        let minz = a[1].min(b[1]).min(c[1]);
        let maxz = a[1].max(b[1]).max(c[1]);
        let ix0 = (((minx - x0) / step).floor() as i64).clamp(0, gw as i64 - 1) as usize;
        let ix1 = (((maxx - x0) / step).floor() as i64).clamp(0, gw as i64 - 1) as usize;
        let iz0 = (((minz - z0) / step).floor() as i64).clamp(0, gd as i64 - 1) as usize;
        let iz1 = (((maxz - z0) / step).floor() as i64).clamp(0, gd as i64 - 1) as usize;
        for iz in iz0..=iz1 {
            for ix in ix0..=ix1 {
                if hit[iz * gw + ix] {
                    continue;
                }
                let cell = [x0 + (ix as f32 + 0.5) * step, z0 + (iz as f32 + 0.5) * step];
                // Degenerate projections (vertical walls) mark the cells their
                // bounding box touches when thin.
                let thin = (maxx - minx) < step || (maxz - minz) < step;
                if thin || inside2(cell, a, b, c) {
                    hit[iz * gw + ix] = true;
                }
            }
        }
    }
    let mut cells = vec![];
    for tz in 0..d_tiles {
        for tx in 0..w_tiles {
            let mut any = false;
            for sz in 0..sub {
                for sx in 0..sub {
                    let ix = (tx * sub + sx) as usize;
                    let iz = (tz * sub + sz) as usize;
                    any |= hit[iz * gw + ix];
                }
            }
            if any {
                cells.push([tx, tz]);
            }
        }
    }
    if cells.is_empty() {
        cells.push([0, 0]);
    }
    Footprint {
        w_tiles,
        d_tiles,
        cells,
        size_mm: [mm(size[0]), mm(size[1]), mm(size[2])],
    }
}

fn inside2(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let s = |p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]| {
        (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])
    };
    let d1 = s(p, a, b);
    let d2 = s(p, b, c);
    let d3 = s(p, c, a);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

pub fn collision(class: AssetClass, tris: &[[[f32; 3]; 3]], bb: &Aabb) -> Collision {
    let bmin = [mm(bb.min[0]), mm(bb.min[1]), mm(bb.min[2])];
    let bmax = [mm(bb.max[0]), mm(bb.max[1]), mm(bb.max[2])];
    match class {
        AssetClass::Character => {
            let s = bb.size();
            let radius = (s[0].max(s[2]) * 0.5).clamp(0.15, 0.6);
            Collision::Capsule {
                radius_mm: mm(radius),
                height_mm: mm(s[1]),
            }
        }
        AssetClass::Building => {
            let levels = ((bmax[1] - bmin[1]) as f32 / LEVEL_MM as f32)
                .ceil()
                .max(1.0) as i32;
            let mut boxes = vec![];
            for l in 0..levels {
                let y0 = (l * LEVEL_MM) as f32 / 1000.0 + bb.min[1];
                let y1 = y0 + LEVEL_MM as f32 / 1000.0;
                let mut lb = Aabb::empty();
                for t in tris {
                    // Clip each triangle to the slab by its vertices plus the
                    // edge crossings.
                    for i in 0..3 {
                        let (p, q) = (t[i], t[(i + 1) % 3]);
                        if p[1] >= y0 && p[1] <= y1 {
                            lb.add(p);
                        }
                        for y in [y0, y1] {
                            if (p[1] - y) * (q[1] - y) < 0.0 {
                                let f = (y - p[1]) / (q[1] - p[1]);
                                lb.add([p[0] + (q[0] - p[0]) * f, y, p[2] + (q[2] - p[2]) * f]);
                            }
                        }
                    }
                }
                if lb.is_valid() {
                    boxes.push([
                        [mm(lb.min[0]), mm(lb.min[1]), mm(lb.min[2])],
                        [mm(lb.max[0]), mm(lb.max[1]), mm(lb.max[2])],
                    ]);
                }
            }
            if boxes.is_empty() {
                boxes.push([bmin, bmax]);
            }
            Collision::Boxes { boxes }
        }
        AssetClass::Prop | AssetClass::Decoration => Collision::Box {
            min_mm: bmin,
            max_mm: bmax,
        },
    }
}

/// Side with the most outward vertical face area. glTF convention (and a
/// tie) says the front looks at +z, which is south in the city.
pub fn front(tris: &[[[f32; 3]; 3]]) -> String {
    let mut area = [0f32; 4]; // n e s w
    for t in tris {
        let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        let a = len3(n) * 0.5;
        if a <= 0.0 {
            continue;
        }
        let n = normalize3(n);
        if n[1].abs() > 0.5 {
            continue;
        }
        if n[2] < -0.7 {
            area[0] += a;
        } else if n[0] > 0.7 {
            area[1] += a;
        } else if n[2] > 0.7 {
            area[2] += a;
        } else if n[0] < -0.7 {
            area[3] += a;
        }
    }
    let s = area[2];
    let (mut best, mut bi) = (s * 1.15, 2);
    for (i, a) in area.iter().enumerate() {
        if *a > best {
            best = *a;
            bi = i;
        }
    }
    ["n", "e", "s", "w"][bi].to_string()
}
