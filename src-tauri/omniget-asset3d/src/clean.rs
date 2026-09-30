//! Passo 3: weld, dedup, prune, indices, normals; and flattening of static
//! assets (node transforms baked into one mesh, skinned meshes frozen in the
//! rest pose).

use crate::meshopt;
use crate::model::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Welds identical vertices (all attributes equal), drops degenerate
/// triangles and orders vertices by first use.
pub fn weld(p: &mut Prim) {
    let n = p.pos.len();
    if n == 0 {
        p.idx.clear();
        return;
    }
    let mut stride = 12;
    if p.nrm.is_some() {
        stride += 12;
    }
    if p.uv.is_some() {
        stride += 8;
    }
    if p.col.is_some() {
        stride += 16;
    }
    if p.joints.is_some() {
        stride += 8 + 16;
    }
    stride += p
        .targets
        .iter()
        .map(|t| 12 + if t.nrm.is_some() { 12 } else { 0 })
        .sum::<usize>();
    let mut packed = Vec::with_capacity(n * stride);
    let put = |b: &mut Vec<u8>, v: &[f32]| {
        for x in v {
            // -0.0 and 0.0 must weld together.
            let x = if *x == 0.0 { 0.0f32 } else { *x };
            b.extend_from_slice(&x.to_le_bytes());
        }
    };
    for i in 0..n {
        put(&mut packed, &p.pos[i]);
        if let Some(v) = &p.nrm {
            put(&mut packed, &v[i]);
        }
        if let Some(v) = &p.uv {
            put(&mut packed, &v[i]);
        }
        if let Some(v) = &p.col {
            put(&mut packed, &v[i]);
        }
        if let (Some(j), Some(w)) = (&p.joints, &p.weights) {
            for k in j[i] {
                packed.extend_from_slice(&k.to_le_bytes());
            }
            put(&mut packed, &w[i]);
        }
        for t in &p.targets {
            put(&mut packed, &t.pos[i]);
            if let Some(v) = &t.nrm {
                put(&mut packed, &v[i]);
            }
        }
    }
    // meshopt takes at most 256 bytes per vertex; many morph targets go
    // through a plain hash map instead.
    let (remap, unique) = if stride <= 256 {
        meshopt::generate_vertex_remap(Some(&p.idx), &packed, stride)
    } else {
        let mut map: HashMap<&[u8], u32> = HashMap::new();
        let mut remap = vec![u32::MAX; n];
        let mut used = vec![false; n];
        for &i in &p.idx {
            used[i as usize] = true;
        }
        for i in 0..n {
            if used[i] {
                let next = map.len() as u32;
                remap[i] = *map
                    .entry(&packed[i * stride..(i + 1) * stride])
                    .or_insert(next);
            }
        }
        (remap, map.len())
    };
    p.apply_remap(&remap, unique);
    drop_degenerate(p);
    compact(p);
}

pub fn drop_degenerate(p: &mut Prim) {
    let mut out = Vec::with_capacity(p.idx.len());
    for t in p.idx.chunks_exact(3) {
        if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
            continue;
        }
        let (a, b, c) = (
            p.pos[t[0] as usize],
            p.pos[t[1] as usize],
            p.pos[t[2] as usize],
        );
        let area = len3(cross(sub(b, a), sub(c, a)));
        // NaN counts as degenerate too.
        if area.is_nan() || area <= 1e-14 {
            continue;
        }
        out.extend_from_slice(t);
    }
    p.idx = out;
}

/// Vertex cache order, then drop unused vertices in fetch order.
pub fn compact(p: &mut Prim) {
    if p.idx.is_empty() {
        *p = Prim {
            material: p.material,
            ..Default::default()
        };
        return;
    }
    meshopt::optimize_vertex_cache(&mut p.idx, p.pos.len());
    let (remap, unique) = meshopt::optimize_vertex_fetch_remap(&p.idx, p.pos.len());
    p.apply_remap(&remap, unique);
}

/// Area-weighted smooth normals for primitives that have none.
pub fn ensure_normals(p: &mut Prim) {
    if p.nrm.is_some() {
        return;
    }
    let mut acc = vec![[0f32; 3]; p.pos.len()];
    for t in p.idx.chunks_exact(3) {
        let (a, b, c) = (
            p.pos[t[0] as usize],
            p.pos[t[1] as usize],
            p.pos[t[2] as usize],
        );
        let n = cross(sub(b, a), sub(c, a));
        for &v in t {
            for k in 0..3 {
                acc[v as usize][k] += n[k];
            }
        }
    }
    p.nrm = Some(acc.into_iter().map(normalize3).collect());
}

/// Bakes every drawn node into one mesh in world space, one primitive per
/// material. Skins, animations and morph targets are dropped.
pub fn flatten(model: &Model) -> Model {
    let world = model.world_matrices();
    let reach = model.reachable();
    let mut by_mat: HashMap<Option<usize>, Prim> = HashMap::new();
    let mut order: Vec<Option<usize>> = vec![];
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
        let m = world[ni];
        let flip = determinant3(&m) < 0.0;
        for p in &mesh.prims {
            let dst = by_mat.entry(p.material).or_insert_with(|| {
                order.push(p.material);
                Prim {
                    material: p.material,
                    ..Default::default()
                }
            });
            let base = dst.pos.len() as u32;
            let n = p.pos.len();
            let skinned = matches!((&skin, &p.joints, &p.weights), (Some(_), Some(_), Some(_)));
            for i in 0..n {
                let (wp, wn) = if skinned {
                    let sm = skin.as_ref().unwrap();
                    let j = p.joints.as_ref().unwrap()[i];
                    let w = p.weights.as_ref().unwrap()[i];
                    (
                        skin_point(sm, j, w, p.pos[i]),
                        p.nrm.as_ref().map(|nr| skin_dir(sm, j, w, nr[i])),
                    )
                } else {
                    (
                        transform_point(&m, p.pos[i]),
                        p.nrm.as_ref().map(|nr| transform_normal(&m, nr[i])),
                    )
                };
                dst.pos.push(wp);
                // Streams are either present on every merged primitive or
                // filled with a neutral value.
                push_opt(&mut dst.nrm, wn, base as usize, [0.0, 1.0, 0.0]);
                push_opt(
                    &mut dst.uv,
                    p.uv.as_ref().map(|v| v[i]),
                    base as usize,
                    [0.0, 0.0],
                );
                push_opt(
                    &mut dst.col,
                    p.col.as_ref().map(|v| v[i]),
                    base as usize,
                    [1.0; 4],
                );
            }
            for t in p.idx.chunks_exact(3) {
                if flip && !skinned {
                    dst.idx
                        .extend_from_slice(&[base + t[0], base + t[2], base + t[1]]);
                } else {
                    dst.idx
                        .extend_from_slice(&[base + t[0], base + t[1], base + t[2]]);
                }
            }
            // Pad streams the new primitive lacked.
            let total = dst.pos.len();
            pad(&mut dst.nrm, total, [0.0, 1.0, 0.0]);
            pad(&mut dst.uv, total, [0.0, 0.0]);
            pad(&mut dst.col, total, [1.0; 4]);
        }
    }
    let prims = order
        .into_iter()
        .filter_map(|k| by_mat.remove(&k))
        .collect();
    Model {
        nodes: vec![Node {
            name: Some("asset".into()),
            mesh: Some(0),
            ..Default::default()
        }],
        roots: vec![0],
        meshes: vec![Mesh {
            name: Some("asset".into()),
            prims,
            weights: None,
            extras: None,
        }],
        materials: model.materials.clone(),
        textures: model.textures.clone(),
        images: model.images.clone(),
        skins: vec![],
        animations: vec![],
        root_extensions: Default::default(),
    }
}

fn push_opt<T: Copy>(dst: &mut Option<Vec<T>>, v: Option<T>, base: usize, neutral: T) {
    match (dst.as_mut(), v) {
        (Some(d), Some(x)) => d.push(x),
        (Some(d), None) => d.push(neutral),
        (None, Some(x)) => {
            let mut d = vec![neutral; base];
            d.push(x);
            *dst = Some(d);
        }
        (None, None) => {}
    }
}

fn pad<T: Copy>(dst: &mut Option<Vec<T>>, total: usize, neutral: T) {
    if let Some(d) = dst {
        d.resize(total, neutral);
    }
}

/// Dedup identical images, textures and materials, remove unused ones and
/// merge primitives that end up on the same material. Only for flattened
/// models (indices change).
pub fn dedup_and_prune(m: &mut Model) {
    // Images by pixel hash.
    let mut img_map = vec![0usize; m.images.len()];
    let mut seen: HashMap<[u8; 32], usize> = HashMap::new();
    let mut images = vec![];
    for (i, im) in m.images.iter().enumerate() {
        let mut h = Sha256::new();
        h.update(im.rgba.width().to_le_bytes());
        h.update(im.rgba.height().to_le_bytes());
        h.update(im.rgba.as_raw());
        let key: [u8; 32] = h.finalize().into();
        img_map[i] = *seen.entry(key).or_insert_with(|| {
            images.push(im.clone());
            images.len() - 1
        });
    }
    // Textures by image (samplers are rewritten by the writer anyway).
    let mut tex_map = vec![None; m.textures.len()];
    let mut tex_seen: HashMap<usize, usize> = HashMap::new();
    let mut textures: Vec<Texture> = vec![];
    for (i, t) in m.textures.iter().enumerate() {
        if let Some(img) = t.image.map(|x| img_map[x]) {
            tex_map[i] = Some(*tex_seen.entry(img).or_insert_with(|| {
                textures.push(Texture {
                    name: t.name.clone(),
                    image: Some(img),
                    sampler: t.sampler.clone(),
                });
                textures.len() - 1
            }));
        }
    }
    // Materials by the fields the flattened writer keeps.
    let mut mat_map = vec![0usize; m.materials.len()];
    let mut mat_seen: HashMap<String, usize> = HashMap::new();
    let mut materials: Vec<Material> = vec![];
    for (i, mat) in m.materials.iter().enumerate() {
        let mut nm = mat.clone();
        nm.base_tex = mat.base_tex.and_then(|t| tex_map[t]);
        let key = format!(
            "{:?}|{:?}|{}|{}|{}|{:?}|{}",
            nm.base_color,
            nm.base_tex,
            nm.alpha_mode,
            nm.alpha_cutoff,
            nm.double_sided,
            nm.emissive,
            nm.unlit
        );
        mat_map[i] = *mat_seen.entry(key).or_insert_with(|| {
            materials.push(nm);
            materials.len() - 1
        });
    }
    for mesh in &mut m.meshes {
        for p in &mut mesh.prims {
            p.material = p.material.map(|x| mat_map[x]);
        }
    }
    m.images = images;
    m.textures = textures;
    m.materials = materials;
    merge_same_material(m);
    prune(m);
}

pub fn merge_same_material(m: &mut Model) {
    for mesh in &mut m.meshes {
        let mut merged: Vec<Prim> = vec![];
        for p in mesh.prims.drain(..) {
            if let Some(dst) = merged
                .iter_mut()
                .find(|d| d.material == p.material && same_streams(d, &p))
            {
                let base = dst.pos.len() as u32;
                dst.pos.extend_from_slice(&p.pos);
                if let (Some(a), Some(b)) = (&mut dst.nrm, &p.nrm) {
                    a.extend_from_slice(b);
                }
                if let (Some(a), Some(b)) = (&mut dst.uv, &p.uv) {
                    a.extend_from_slice(b);
                }
                if let (Some(a), Some(b)) = (&mut dst.col, &p.col) {
                    a.extend_from_slice(b);
                }
                dst.idx.extend(p.idx.iter().map(|i| i + base));
            } else {
                merged.push(p);
            }
        }
        mesh.prims = merged.into_iter().filter(|p| !p.idx.is_empty()).collect();
    }
}

fn same_streams(a: &Prim, b: &Prim) -> bool {
    a.nrm.is_some() == b.nrm.is_some()
        && a.uv.is_some() == b.uv.is_some()
        && a.col.is_some() == b.col.is_some()
        && a.joints.is_none()
        && b.joints.is_none()
        && a.targets.is_empty()
        && b.targets.is_empty()
}

/// Drops materials, textures and images nothing uses (flattened models).
pub fn prune(m: &mut Model) {
    let mut used_mat = vec![false; m.materials.len()];
    for p in m.meshes.iter().flat_map(|x| &x.prims) {
        if let Some(i) = p.material {
            used_mat[i] = true;
        }
    }
    let mut mat_map = vec![None; m.materials.len()];
    let mut mats = vec![];
    for (i, mat) in m.materials.iter().enumerate() {
        if used_mat[i] {
            mat_map[i] = Some(mats.len());
            mats.push(mat.clone());
        }
    }
    for p in m.meshes.iter_mut().flat_map(|x| &mut x.prims) {
        p.material = p.material.and_then(|i| mat_map[i]);
    }
    let mut used_tex = vec![false; m.textures.len()];
    for mat in &mats {
        if let Some(t) = mat.base_tex {
            used_tex[t] = true;
        }
    }
    let mut tex_map = vec![None; m.textures.len()];
    let mut texs: Vec<Texture> = vec![];
    for (i, t) in m.textures.iter().enumerate() {
        if used_tex[i] {
            tex_map[i] = Some(texs.len());
            texs.push(t.clone());
        }
    }
    for mat in &mut mats {
        mat.base_tex = mat.base_tex.and_then(|t| tex_map[t]);
    }
    let mut used_img = vec![false; m.images.len()];
    for t in &texs {
        if let Some(i) = t.image {
            used_img[i] = true;
        }
    }
    let mut img_map = vec![None; m.images.len()];
    let mut imgs = vec![];
    for (i, im) in m.images.iter().enumerate() {
        if used_img[i] {
            img_map[i] = Some(imgs.len());
            imgs.push(im.clone());
        }
    }
    for t in &mut texs {
        t.image = t.image.and_then(|i| img_map[i]);
    }
    m.materials = mats;
    m.textures = texs;
    m.images = imgs;
}

/// Drops keyframes a linear track would interpolate anyway (glTF-Transform's
/// `resample`), and collapses constant tracks to one key.
pub fn resample_animations(m: &mut Model) {
    const EPS: f32 = 1e-4;
    for an in &mut m.animations {
        for s in &mut an.samplers {
            let c = s.comps.max(1);
            let n = s.input.len();
            if s.interpolation == "CUBICSPLINE" || n < 3 || s.output.len() != n * c {
                continue;
            }
            let rot = c == 4;
            let val = |i: usize| &s.output[i * c..(i + 1) * c];
            let mut keep = vec![0usize];
            for i in 1..n - 1 {
                let prev = *keep.last().unwrap();
                let (t0, t1, t2) = (s.input[prev], s.input[i], s.input[i + 1]);
                let f = if t2 > t0 { (t1 - t0) / (t2 - t0) } else { 0.0 };
                let (a, b, v) = (val(prev), val(i + 1), val(i));
                let redundant = if s.interpolation == "STEP" {
                    (0..c).all(|k| (a[k] - v[k]).abs() <= EPS)
                } else {
                    // Rotations: normalized lerp with the sign fixed.
                    let sign = if rot && (0..4).map(|k| a[k] * b[k]).sum::<f32>() < 0.0 {
                        -1.0
                    } else {
                        1.0
                    };
                    let mut l: Vec<f32> = (0..c).map(|k| a[k] + (b[k] * sign - a[k]) * f).collect();
                    if rot {
                        let len = l.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
                        l.iter_mut().for_each(|x| *x /= len);
                        let d = (0..4).map(|k| l[k] * v[k]).sum::<f32>().abs();
                        d >= 1.0 - EPS * EPS
                    } else {
                        (0..c).all(|k| (l[k] - v[k]).abs() <= EPS * (1.0 + v[k].abs()))
                    }
                };
                if !redundant {
                    keep.push(i);
                }
            }
            keep.push(n - 1);
            let constant =
                keep.len() == 2 && (0..c).all(|k| (val(0)[k] - val(n - 1)[k]).abs() <= EPS);
            if constant {
                keep.pop();
            }
            if keep.len() < n {
                let input: Vec<f32> = keep.iter().map(|&i| s.input[i]).collect();
                let output: Vec<f32> = keep
                    .iter()
                    .flat_map(|&i| s.output[i * c..(i + 1) * c].to_vec())
                    .collect();
                s.input = input;
                s.output = output;
            }
        }
    }
}
