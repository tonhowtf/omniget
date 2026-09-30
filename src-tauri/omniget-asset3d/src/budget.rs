//! Passo 5: triangle budget per class.
//!
//! Good meshes go through `simplifyWithAttributes` (UV and normals in the
//! metric). Meshes the simplifier cannot reduce (hard edges, triangle soup)
//! fall back to the permissive mode and then to voxel `remesh` + simplify,
//! which loses UV: the colour is sampled back into COLOR_0 from the nearest
//! original vertex. Characters only get the non-destructive path: indices
//! change, vertices (and so joints, weights and morph targets) never move.

use crate::clean;
use crate::meshopt;
use crate::model::*;
use crate::texture::sample_material;
use crate::types::AssetClass;

pub struct BudgetResult {
    pub before: usize,
    pub after: usize,
    pub remeshed: usize,
}

pub fn apply(
    model: &mut Model,
    class: AssetClass,
    flattened: bool,
    warnings: &mut Vec<String>,
) -> BudgetResult {
    let before = model.triangles();
    let budget = class.triangle_budget();
    let mut remeshed = 0;
    if before <= budget {
        return BudgetResult {
            before,
            after: before,
            remeshed,
        };
    }
    let ratio = budget as f64 / before as f64;
    let mut new_materials: Vec<Material> = vec![];
    let base_mat_count = model.materials.len();
    for mi in 0..model.meshes.len() {
        for pi in 0..model.meshes[mi].prims.len() {
            let tris = model.meshes[mi].prims[pi].tris();
            let target_tris = ((tris as f64 * ratio).ceil() as usize).max(4);
            if tris <= target_tris {
                continue;
            }
            let target = target_tris * 3;
            let p = &model.meshes[mi].prims[pi];
            let (attrs, weights) = attributes_of(p);
            let (mut idx, _) = meshopt::simplify_with_attributes(
                &p.idx,
                &p.pos,
                &attrs,
                &weights,
                &[],
                target,
                0.02,
                meshopt::SIMPLIFY_ERROR_CLAMPED,
            );
            if !flattened {
                // Character: nothing more aggressive than this.
                if idx.len() > target * 2 {
                    let (i2, _) = meshopt::simplify_with_attributes(
                        &p.idx,
                        &p.pos,
                        &attrs,
                        &weights,
                        &[],
                        target,
                        0.05,
                        meshopt::SIMPLIFY_ERROR_CLAMPED,
                    );
                    idx = i2;
                }
                let p = &mut model.meshes[mi].prims[pi];
                p.idx = idx;
                clean::compact(p);
                continue;
            }
            if idx.len() > target * 3 / 2 {
                let (i2, _) = meshopt::simplify_with_attributes(
                    &p.idx,
                    &p.pos,
                    &attrs,
                    &weights,
                    &[],
                    target,
                    0.05,
                    meshopt::SIMPLIFY_PERMISSIVE | meshopt::SIMPLIFY_ERROR_CLAMPED,
                );
                if i2.len() < idx.len() {
                    idx = i2;
                }
            }
            if idx.len() > target * 2 {
                // Remesh path.
                let colors: Vec<[f32; 4]> = (0..p.pos.len())
                    .map(|v| vertex_color(model, p, v))
                    .collect();
                let p = &model.meshes[mi].prims[pi];
                if let Some(np) = remesh_to(p, &colors, target_tris) {
                    remeshed += 1;
                    let mut mat = p
                        .material
                        .and_then(|m| model.materials.get(m).cloned())
                        .unwrap_or_default();
                    mat.base_tex = None;
                    mat.base_color = [1.0, 1.0, 1.0, mat.base_color[3]];
                    mat.name = Some(format!(
                        "{}-baked",
                        mat.name.clone().unwrap_or_else(|| "material".into())
                    ));
                    new_materials.push(mat);
                    let mut np = np;
                    np.material = Some(base_mat_count + new_materials.len() - 1);
                    model.meshes[mi].prims[pi] = np;
                    continue;
                }
            }
            let p = &mut model.meshes[mi].prims[pi];
            p.idx = idx;
            clean::compact(p);
        }
    }
    model.materials.extend(new_materials);
    let after = model.triangles();
    if after > budget * 2 {
        warnings.push(format!(
            "{after} triangles left after simplify (budget {budget})"
        ));
    }
    BudgetResult {
        before,
        after,
        remeshed,
    }
}

fn attributes_of(p: &Prim) -> (Vec<f32>, Vec<f32>) {
    let mut weights = vec![];
    if p.nrm.is_some() {
        weights.extend_from_slice(&[0.5, 0.5, 0.5]);
    }
    if p.uv.is_some() {
        weights.extend_from_slice(&[1.0, 1.0]);
    }
    if p.col.is_some() {
        weights.extend_from_slice(&[0.5, 0.5, 0.5]);
    }
    let ac = weights.len();
    let mut attrs = Vec::with_capacity(p.pos.len() * ac);
    for i in 0..p.pos.len() {
        if let Some(n) = &p.nrm {
            attrs.extend_from_slice(&n[i]);
        }
        if let Some(u) = &p.uv {
            attrs.extend_from_slice(&u[i]);
        }
        if let Some(c) = &p.col {
            attrs.extend_from_slice(&c[i][..3]);
        }
    }
    (attrs, weights)
}

pub fn vertex_color(model: &Model, p: &Prim, v: usize) -> [f32; 4] {
    let mut c = sample_material(model, p.material, p.uv.as_ref().map(|u| u[v]));
    if let Some(col) = &p.col {
        for k in 0..4 {
            c[k] *= col[v][k];
        }
    }
    c
}

/// Voxel remesh + simplify to `target_tris`, colours from the nearest
/// original vertex. `None` when the remesher gives nothing usable.
pub fn remesh_to(p: &Prim, colors: &[[f32; 4]], target_tris: usize) -> Option<Prim> {
    let res = ((target_tris as f32).sqrt() * 1.5).clamp(24.0, 128.0) as i32;
    let soup = meshopt::remesh(&p.idx, &p.pos, res, meshopt::REMESH_SOLVE);
    if soup.len() < 3 {
        return None;
    }
    // Weld the soup by position.
    let bytes: Vec<u8> = soup
        .iter()
        .flat_map(|v| v.iter().flat_map(|x| x.to_le_bytes()))
        .collect();
    let (remap, unique) = meshopt::generate_vertex_remap(None, &bytes, 12);
    let pos = meshopt::remap_vertex_buffer(&soup, unique, &remap);
    let idx = meshopt::remap_index_buffer(None, soup.len(), &remap);
    let (idx, _) = meshopt::simplify(
        &idx,
        &pos,
        target_tris * 3,
        0.02,
        meshopt::SIMPLIFY_PRESERVE_FOLDS,
    );
    if idx.is_empty() {
        return None;
    }
    let grid = Grid::new(&p.pos);
    let col = pos
        .iter()
        .map(|v| {
            grid.nearest(&p.pos, *v)
                .map(|i| colors[i])
                .unwrap_or([1.0; 4])
        })
        .collect();
    let mut np = Prim {
        pos,
        idx,
        col: Some(col),
        ..Default::default()
    };
    clean::drop_degenerate(&mut np);
    clean::compact(&mut np);
    clean::ensure_normals(&mut np);
    Some(np)
}

/// Uniform grid for nearest-vertex lookups.
pub struct Grid {
    min: [f32; 3],
    cell: f32,
    dims: [i32; 3],
    cells: std::collections::HashMap<(i32, i32, i32), Vec<u32>>,
}

impl Grid {
    pub fn new(pts: &[[f32; 3]]) -> Self {
        let bb = Aabb::of(pts);
        let s = bb.size();
        let diag = len3(s).max(1e-6);
        let cell = diag / 48.0;
        let dims = [
            (s[0] / cell).ceil() as i32 + 1,
            (s[1] / cell).ceil() as i32 + 1,
            (s[2] / cell).ceil() as i32 + 1,
        ];
        let mut g = Grid {
            min: bb.min,
            cell,
            dims,
            cells: Default::default(),
        };
        for (i, p) in pts.iter().enumerate() {
            let k = g.key(*p);
            g.cells.entry(k).or_default().push(i as u32);
        }
        g
    }

    fn key(&self, p: [f32; 3]) -> (i32, i32, i32) {
        let f = |a: usize| {
            (((p[a] - self.min[a]) / self.cell).floor() as i32).clamp(0, self.dims[a] - 1)
        };
        (f(0), f(1), f(2))
    }

    pub fn nearest(&self, pts: &[[f32; 3]], q: [f32; 3]) -> Option<usize> {
        let (kx, ky, kz) = self.key(q);
        let mut best: Option<(f32, usize)> = None;
        for ring in 0i32..6 {
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    for dz in -ring..=ring {
                        if dx.abs() != ring && dy.abs() != ring && dz.abs() != ring {
                            continue;
                        }
                        if let Some(list) = self.cells.get(&(kx + dx, ky + dy, kz + dz)) {
                            for &i in list {
                                let d = sub(pts[i as usize], q);
                                let d2 = dot(d, d);
                                if best.map(|b| d2 < b.0).unwrap_or(true) {
                                    best = Some((d2, i as usize));
                                }
                            }
                        }
                    }
                }
            }
            if best.is_some() && ring >= 1 {
                break;
            }
        }
        best.map(|b| b.1)
    }
}
