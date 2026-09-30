//! `.vox` (MagicaVoxel) export: surface voxelization of the asset with one
//! palette colour per voxel, written with `dot_vox`.

use crate::model::*;
use crate::raster::colored_triangles;
use crate::texture::linear_to_srgb;
use std::collections::HashMap;

pub struct VoxResult {
    pub bytes: Vec<u8>,
    /// Voxel size actually used (grown when the model would exceed 256).
    pub voxel_mm: u32,
    pub voxels: usize,
    pub size: [u32; 3],
}

pub fn export(model: &Model, voxel_mm: u32) -> Result<VoxResult, String> {
    let tris = colored_triangles(model);
    let mut bb = Aabb::empty();
    for t in &tris {
        for p in &t.p {
            bb.add(*p);
        }
    }
    if !bb.is_valid() {
        return Err("the asset has no geometry".into());
    }
    let s = bb.size();
    let mut vm = voxel_mm.clamp(10, 2000);
    let longest = s[0].max(s[1]).max(s[2]);
    while longest / (vm as f32 / 1000.0) > 255.0 {
        vm = (vm as f32 * 1.25).ceil() as u32;
    }
    let v = vm as f32 / 1000.0;
    let dims = [
        ((s[0] / v).floor() as u32 + 1).min(256),
        ((s[1] / v).floor() as u32 + 1).min(256),
        ((s[2] / v).floor() as u32 + 1).min(256),
    ];
    let cell = |p: [f32; 3]| -> [u32; 3] {
        [
            (((p[0] - bb.min[0]) / v) as u32).min(dims[0] - 1),
            (((p[1] - bb.min[1]) / v) as u32).min(dims[1] - 1),
            (((p[2] - bb.min[2]) / v) as u32).min(dims[2] - 1),
        ]
    };
    // voxel → srgb colour (last writer wins).
    let mut grid: HashMap<[u32; 3], [u8; 3]> = HashMap::new();
    for t in &tris {
        if t.color[3] < 0.1 {
            continue;
        }
        let c = [
            (linear_to_srgb(t.color[0]) * 255.0) as u8,
            (linear_to_srgb(t.color[1]) * 255.0) as u8,
            (linear_to_srgb(t.color[2]) * 255.0) as u8,
        ];
        let e = [
            len3(sub(t.p[1], t.p[0])),
            len3(sub(t.p[2], t.p[1])),
            len3(sub(t.p[0], t.p[2])),
        ];
        let n = ((e[0].max(e[1]).max(e[2]) / (v * 0.5)).ceil() as usize).clamp(1, 2048);
        for i in 0..=n {
            for j in 0..=(n - i) {
                let (a, b) = (i as f32 / n as f32, j as f32 / n as f32);
                let cw = 1.0 - a - b;
                let p = [
                    t.p[0][0] * cw + t.p[1][0] * a + t.p[2][0] * b,
                    t.p[0][1] * cw + t.p[1][1] * a + t.p[2][1] * b,
                    t.p[0][2] * cw + t.p[1][2] * a + t.p[2][2] * b,
                ];
                grid.insert(cell(p), c);
            }
        }
    }
    // Palette: 4-bit per channel buckets, the 255 most used, the rest mapped
    // to the nearest of those.
    let q = |c: [u8; 3]| [c[0] >> 4, c[1] >> 4, c[2] >> 4];
    let mut freq: HashMap<[u8; 3], usize> = HashMap::new();
    for c in grid.values() {
        *freq.entry(q(*c)).or_default() += 1;
    }
    let mut buckets: Vec<([u8; 3], usize)> = freq.into_iter().collect();
    buckets.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    buckets.truncate(255);
    let colors: Vec<[u8; 3]> = buckets
        .iter()
        .map(|(k, _)| [k[0] * 17, k[1] * 17, k[2] * 17])
        .collect();
    let mut index_of: HashMap<[u8; 3], u8> = HashMap::new();
    let mut nearest = |c: [u8; 3]| -> u8 {
        let k = q(c);
        *index_of.entry(k).or_insert_with(|| {
            let kk = [k[0] as i32 * 17, k[1] as i32 * 17, k[2] as i32 * 17];
            let mut best = (i32::MAX, 0u8);
            for (i, p) in colors.iter().enumerate() {
                let d = (0..3).map(|x| (p[x] as i32 - kk[x]).pow(2)).sum::<i32>();
                if d < best.0 {
                    best = (d, i as u8);
                }
            }
            best.1
        })
    };
    let mut keys: Vec<[u32; 3]> = grid.keys().copied().collect();
    keys.sort();
    let voxels: Vec<dot_vox::Voxel> = keys
        .iter()
        .map(|k| {
            let c = grid[k];
            // glTF (x, y up, z south) → MagicaVoxel (x, y north, z up).
            dot_vox::Voxel {
                x: k[0] as u8,
                y: (dims[2] - 1 - k[2]) as u8,
                z: k[1] as u8,
                i: nearest(c),
            }
        })
        .collect();
    let mut palette: Vec<dot_vox::Color> = colors
        .iter()
        .map(|c| dot_vox::Color {
            r: c[0],
            g: c[1],
            b: c[2],
            a: 255,
        })
        .collect();
    palette.resize(
        256,
        dot_vox::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        },
    );
    let count = voxels.len();
    let data = dot_vox::DotVoxData {
        version: 150,
        index_map: vec![],
        models: vec![dot_vox::Model {
            size: dot_vox::Size {
                x: dims[0],
                y: dims[2],
                z: dims[1],
            },
            voxels,
        }],
        palette,
        materials: vec![],
        scenes: vec![],
        layers: vec![],
    };
    let mut bytes = vec![];
    data.write_vox(&mut bytes).map_err(|e| e.to_string())?;
    Ok(VoxResult {
        bytes,
        voxel_mm: vm,
        voxels: count,
        size: [dims[0], dims[2], dims[1]],
    })
}
