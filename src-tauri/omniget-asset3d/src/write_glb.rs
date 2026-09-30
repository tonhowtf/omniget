//! Passo 9: GLB with `EXT_meshopt_compression` (+ `KHR_mesh_quantization`
//! and `EXT_texture_webp`), OmniGet metadata in `asset.extras`.
//!
//! Layout: buffer 0 is the GLB BIN chunk with the compressed streams and the
//! images; buffer 1 is the meshopt fallback (no data, only its length).
//! Vertex codec version 0, the one `EXT_meshopt_compression` specifies.

use crate::meshopt;
use crate::model::*;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};

pub struct WriteOptions {
    /// Quantize static positions to u16 through the node transform.
    pub quantize_positions: bool,
    /// meshopt streams and quantized normals. Off gives a plain GLB (float
    /// attributes, raw views) for tools that read nothing else.
    pub compress: bool,
    pub asset_extras: Value,
    pub copyright: Option<String>,
}

type FloatKey = ([u8; 32], usize, Option<u32>, bool);

/// Streams of the same kind (mode, stride, filter, target) are packed into
/// one buffer view, like gltfpack does; accessors point into it with a byte
/// offset. That keeps the JSON small when there are hundreds of animation
/// channels.
#[derive(Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    mode: &'static str,
    stride: usize,
    filter: Option<&'static str>,
    target: Option<u32>,
}

struct Group {
    key: GroupKey,
    data: Vec<u8>,
    max_index: u32,
}

#[derive(Clone, Copy)]
enum Slot {
    Group(usize, usize),
    Raw(usize),
}

struct W {
    compress: bool,
    float_cache: HashMap<FloatKey, usize>,
    groups: Vec<Group>,
    group_of: HashMap<GroupKey, usize>,
    raws: Vec<Vec<u8>>,
    slots: Vec<Slot>,
    acc_slot: Vec<usize>,
    bin: Vec<u8>,
    fallback_len: usize,
    views: Vec<Value>,
    accessors: Vec<Value>,
    used: BTreeSet<String>,
    required: BTreeSet<String>,
}

fn align(v: &mut Vec<u8>, a: usize) {
    while !v.len().is_multiple_of(a) {
        v.push(0);
    }
}

fn mode_str(mode: &str) -> &'static str {
    match mode {
        "TRIANGLES" => "TRIANGLES",
        "INDICES" => "INDICES",
        _ => "ATTRIBUTES",
    }
}

impl W {
    fn push_group(&mut self, key: GroupKey, raw: &[u8]) -> usize {
        let gi = match self.group_of.get(&key) {
            Some(&g) => g,
            None => {
                self.groups.push(Group {
                    key: key.clone(),
                    data: vec![],
                    max_index: 0,
                });
                self.group_of.insert(key.clone(), self.groups.len() - 1);
                self.groups.len() - 1
            }
        };
        let g = &mut self.groups[gi];
        if key.mode == "TRIANGLES" {
            let m = if key.stride == 2 {
                raw.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
                    .max()
            } else {
                raw.chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .max()
            };
            g.max_index = g.max_index.max(m.unwrap_or(0));
        }
        let off = g.data.len();
        g.data.extend_from_slice(raw);
        self.slots.push(Slot::Group(gi, off));
        self.slots.len() - 1
    }

    fn meshopt_view(
        &mut self,
        raw: &[u8],
        stride: usize,
        _count: usize,
        mode: &str,
        target: Option<u32>,
        _vertex_count: usize,
    ) -> usize {
        self.push_group(
            GroupKey {
                mode: mode_str(mode),
                stride,
                filter: None,
                target,
            },
            raw,
        )
    }

    /// Attribute stream already in a meshopt filter's format (`raw` is what
    /// the decoder hands back before the filter runs).
    fn filtered_view(
        &mut self,
        raw: &[u8],
        stride: usize,
        _count: usize,
        filter: &'static str,
    ) -> usize {
        self.push_group(
            GroupKey {
                mode: "ATTRIBUTES",
                stride,
                filter: Some(filter),
                target: None,
            },
            raw,
        )
    }

    fn raw_view(&mut self, bytes: &[u8]) -> usize {
        self.raws.push(bytes.to_vec());
        self.slots.push(Slot::Raw(self.raws.len() - 1));
        self.slots.len() - 1
    }

    #[allow(clippy::too_many_arguments)]
    fn accessor(
        &mut self,
        slot: usize,
        ctype: u32,
        count: usize,
        ty: &str,
        normalized: bool,
        min: Option<Value>,
        max: Option<Value>,
    ) -> usize {
        let mut a = json!({"bufferView": 0, "componentType": ctype, "count": count, "type": ty});
        if normalized {
            a["normalized"] = json!(true);
        }
        if let (Some(mn), Some(mx)) = (min, max) {
            a["min"] = mn;
            a["max"] = mx;
        }
        self.accessors.push(a);
        self.acc_slot.push(slot);
        self.accessors.len() - 1
    }

    /// Encodes the groups and raw blobs into buffer views; returns
    /// `slot → (view, byteOffset)`.
    fn finalize(&mut self) -> Vec<(usize, usize)> {
        let mut group_view = vec![0usize; self.groups.len()];
        let groups = std::mem::take(&mut self.groups);
        for (gi, g) in groups.iter().enumerate() {
            let k = &g.key;
            let count = g.data.len() / k.stride.max(1);
            let mut v = if self.compress {
                let compressed = match k.mode {
                    "TRIANGLES" => {
                        let idx: Vec<u32> = if k.stride == 2 {
                            g.data
                                .chunks_exact(2)
                                .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
                                .collect()
                        } else {
                            g.data
                                .chunks_exact(4)
                                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                .collect()
                        };
                        meshopt::encode_index_buffer(&idx, g.max_index as usize + 1)
                    }
                    _ => meshopt::encode_vertex_buffer(&g.data, k.stride),
                };
                align(&mut self.bin, 4);
                let c_off = self.bin.len();
                self.bin.extend_from_slice(&compressed);
                self.fallback_len = self.fallback_len.div_ceil(16) * 16;
                let f_off = self.fallback_len;
                self.fallback_len += g.data.len();
                let mut ext = json!({
                    "buffer": 0, "byteOffset": c_off, "byteLength": compressed.len(),
                    "byteStride": k.stride, "count": count, "mode": k.mode
                });
                if let Some(f) = k.filter {
                    ext["filter"] = json!(f);
                }
                json!({"buffer": 1, "byteOffset": f_off, "byteLength": g.data.len(),
                       "extensions": {"EXT_meshopt_compression": ext}})
            } else {
                align(&mut self.bin, 4);
                let off = self.bin.len();
                self.bin.extend_from_slice(&g.data);
                json!({"buffer": 0, "byteOffset": off, "byteLength": g.data.len()})
            };
            if let Some(t) = k.target {
                v["target"] = json!(t);
                if t == 34962 {
                    v["byteStride"] = json!(k.stride);
                }
            }
            self.views.push(v);
            group_view[gi] = self.views.len() - 1;
        }
        let mut raw_view = vec![0usize; self.raws.len()];
        let raws = std::mem::take(&mut self.raws);
        for (ri, r) in raws.iter().enumerate() {
            align(&mut self.bin, 4);
            let off = self.bin.len();
            self.bin.extend_from_slice(r);
            self.views
                .push(json!({"buffer": 0, "byteOffset": off, "byteLength": r.len()}));
            raw_view[ri] = self.views.len() - 1;
        }
        let map: Vec<(usize, usize)> = self
            .slots
            .iter()
            .map(|s| match *s {
                Slot::Group(g, off) => (group_view[g], off),
                Slot::Raw(r) => (raw_view[r], 0),
            })
            .collect();
        for (ai, a) in self.accessors.iter_mut().enumerate() {
            let (view, off) = map[self.acc_slot[ai]];
            a["bufferView"] = json!(view);
            if off > 0 {
                a["byteOffset"] = json!(off);
            }
        }
        map
    }

    fn float_accessor(
        &mut self,
        data: &[f32],
        comps: usize,
        ty: &str,
        target: Option<u32>,
        minmax: bool,
    ) -> usize {
        let count = data.len() / comps;
        let raw: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        // Identical streams (animation times above all) share one accessor.
        let key = (Sha256::digest(&raw).into(), comps, target, minmax);
        if let Some(&a) = self.float_cache.get(&key) {
            return a;
        }
        let a = self.float_accessor_new(&raw, data, comps, ty, target, minmax, count);
        self.float_cache.insert(key, a);
        a
    }

    #[allow(clippy::too_many_arguments)]
    fn float_accessor_new(
        &mut self,
        raw: &[u8],
        data: &[f32],
        comps: usize,
        ty: &str,
        target: Option<u32>,
        minmax: bool,
        count: usize,
    ) -> usize {
        let view = self.meshopt_view(raw, comps * 4, count, "ATTRIBUTES", target, 0);
        let (mn, mx) = if minmax {
            let mut mn = vec![f32::INFINITY; comps];
            let mut mx = vec![f32::NEG_INFINITY; comps];
            for e in data.chunks_exact(comps) {
                for k in 0..comps {
                    mn[k] = mn[k].min(e[k]);
                    mx[k] = mx[k].max(e[k]);
                }
            }
            (Some(json!(mn)), Some(json!(mx)))
        } else {
            (None, None)
        };
        self.accessor(view, 5126, count, ty, false, mn, mx)
    }
}

/// Writes the model. With `quantize_positions`, each mesh node's positions
/// become u16 and the node gets the dequantizing transform (only valid for
/// flattened, unskinned models).
pub fn write(model: &Model, opts: &WriteOptions) -> Vec<u8> {
    let mut w = W {
        compress: opts.compress,
        float_cache: HashMap::new(),
        groups: vec![],
        group_of: HashMap::new(),
        raws: vec![],
        slots: vec![],
        acc_slot: vec![],
        bin: vec![],
        fallback_len: 0,
        views: vec![],
        accessors: vec![],
        used: BTreeSet::new(),
        required: BTreeSet::new(),
    };
    if opts.compress {
        w.used.insert("EXT_meshopt_compression".into());
        w.required.insert("EXT_meshopt_compression".into());
    }

    // Per mesh quantization frame (min, step), shared by its primitives.
    let mut qframe: Vec<Option<([f32; 3], f32)>> = vec![None; model.meshes.len()];
    if opts.quantize_positions && opts.compress {
        for (mi, mesh) in model.meshes.iter().enumerate() {
            let mut bb = Aabb::empty();
            for p in &mesh.prims {
                for v in &p.pos {
                    bb.add(*v);
                }
            }
            if bb.is_valid()
                && mesh
                    .prims
                    .iter()
                    .all(|p| p.targets.is_empty() && p.joints.is_none())
            {
                let s = bb.size();
                let step = (s[0].max(s[1]).max(s[2]) / 65535.0).max(1e-7);
                qframe[mi] = Some((bb.min, step));
            }
        }
    }
    // Skinned meshes: one shared frame, folded into the inverse bind
    // matrices (the node transform of a skinned mesh is ignored), as
    // gltfpack does. Off when a skin also drives an unskinned mesh.
    let mut skinned_q = vec![false; model.meshes.len()];
    let mut skin_dequant: Option<Mat4> = None;
    if opts.compress {
        let mut ok = !model.skins.is_empty();
        let mut bb = Aabb::empty();
        let mut used_by_skin = vec![false; model.meshes.len()];
        let mut used_plain = vec![false; model.meshes.len()];
        for n in &model.nodes {
            if let Some(m) = n.mesh {
                if m < model.meshes.len() {
                    if n.skin.is_some() {
                        used_by_skin[m] = true;
                    } else {
                        used_plain[m] = true;
                    }
                }
            }
        }
        for (mi, mesh) in model.meshes.iter().enumerate() {
            if !used_by_skin[mi] {
                continue;
            }
            if used_plain[mi] || !mesh.prims.iter().all(|p| p.joints.is_some()) {
                ok = false;
            }
            for p in &mesh.prims {
                for v in &p.pos {
                    bb.add(*v);
                }
            }
        }
        if ok && bb.is_valid() {
            let s = bb.size();
            let step = (s[0].max(s[1]).max(s[2]) / 65535.0).max(1e-7);
            for mi in 0..model.meshes.len() {
                if used_by_skin[mi] {
                    qframe[mi] = Some((bb.min, step));
                    skinned_q[mi] = true;
                }
            }
            skin_dequant = Some(from_trs(bb.min, [0.0, 0.0, 0.0, 1.0], [step; 3]));
        }
    }

    // ── Meshes ──
    let mut meshes_json = vec![];
    for (mi, mesh) in model.meshes.iter().enumerate() {
        let mut prims_json = vec![];
        for p in &mesh.prims {
            if p.idx.is_empty() {
                continue;
            }
            let n = p.pos.len();
            let mut attrs = Map::new();
            // POSITION
            let pos_acc = if let Some((min, step)) = qframe[mi] {
                w.used.insert("KHR_mesh_quantization".into());
                w.required.insert("KHR_mesh_quantization".into());
                let mut raw = Vec::with_capacity(n * 8);
                let mut qmin = [u16::MAX; 3];
                let mut qmax = [0u16; 3];
                for v in &p.pos {
                    for k in 0..3 {
                        let q = (((v[k] - min[k]) / step).round()).clamp(0.0, 65535.0) as u16;
                        qmin[k] = qmin[k].min(q);
                        qmax[k] = qmax[k].max(q);
                        raw.extend_from_slice(&q.to_le_bytes());
                    }
                    raw.extend_from_slice(&[0, 0]);
                }
                let view = w.meshopt_view(&raw, 8, n, "ATTRIBUTES", Some(34962), 0);
                w.accessor(
                    view,
                    5123,
                    n,
                    "VEC3",
                    false,
                    Some(json!(qmin)),
                    Some(json!(qmax)),
                )
            } else {
                let flat: Vec<f32> = p.pos.iter().flatten().copied().collect();
                w.float_accessor(&flat, 3, "VEC3", Some(34962), true)
            };
            attrs.insert("POSITION".into(), json!(pos_acc));
            if let (Some(nrm), false) = (&p.nrm, opts.compress) {
                let flat: Vec<f32> = nrm.iter().flat_map(|v| normalize3(*v)).collect();
                let a = w.float_accessor(&flat, 3, "VEC3", Some(34962), false);
                attrs.insert("NORMAL".into(), json!(a));
            } else if let Some(nrm) = &p.nrm {
                w.used.insert("KHR_mesh_quantization".into());
                w.required.insert("KHR_mesh_quantization".into());
                let mut raw = Vec::with_capacity(n * 4);
                for v in nrm {
                    let v = normalize3(*v);
                    for x in v {
                        raw.push(((x.clamp(-1.0, 1.0) * 127.0).round() as i8) as u8);
                    }
                    raw.push(0);
                }
                let view = w.meshopt_view(&raw, 4, n, "ATTRIBUTES", Some(34962), 0);
                let a = w.accessor(view, 5120, n, "VEC3", true, None, None);
                attrs.insert("NORMAL".into(), json!(a));
            }
            if let Some(uv) = &p.uv {
                let unit = uv
                    .iter()
                    .all(|t| (0.0..=1.0).contains(&t[0]) && (0.0..=1.0).contains(&t[1]));
                let a = if unit {
                    let mut raw = Vec::with_capacity(n * 4);
                    for t in uv {
                        raw.extend_from_slice(&((t[0] * 65535.0).round() as u16).to_le_bytes());
                        raw.extend_from_slice(&((t[1] * 65535.0).round() as u16).to_le_bytes());
                    }
                    let view = w.meshopt_view(&raw, 4, n, "ATTRIBUTES", Some(34962), 0);
                    w.accessor(view, 5123, n, "VEC2", true, None, None)
                } else {
                    let flat: Vec<f32> = uv.iter().flatten().copied().collect();
                    w.float_accessor(&flat, 2, "VEC2", Some(34962), false)
                };
                attrs.insert("TEXCOORD_0".into(), json!(a));
            }
            if let Some(col) = &p.col {
                let raw: Vec<u8> = col
                    .iter()
                    .flat_map(|c| c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8))
                    .collect();
                let view = w.meshopt_view(&raw, 4, n, "ATTRIBUTES", Some(34962), 0);
                let a = w.accessor(view, 5121, n, "VEC4", true, None, None);
                attrs.insert("COLOR_0".into(), json!(a));
            }
            if let (Some(j), Some(wt)) = (&p.joints, &p.weights) {
                let max = j.iter().flat_map(|x| x.iter()).copied().max().unwrap_or(0);
                let a = if max < 256 {
                    let raw: Vec<u8> = j.iter().flat_map(|x| x.map(|v| v as u8)).collect();
                    let view = w.meshopt_view(&raw, 4, n, "ATTRIBUTES", Some(34962), 0);
                    w.accessor(view, 5121, n, "VEC4", false, None, None)
                } else {
                    let raw: Vec<u8> = j
                        .iter()
                        .flat_map(|x| x.iter().flat_map(|v| v.to_le_bytes()))
                        .collect();
                    let view = w.meshopt_view(&raw, 8, n, "ATTRIBUTES", Some(34962), 0);
                    w.accessor(view, 5123, n, "VEC4", false, None, None)
                };
                attrs.insert("JOINTS_0".into(), json!(a));
                let mut raw = Vec::with_capacity(n * 4);
                for x in wt {
                    raw.extend_from_slice(&quantize_weights(*x));
                }
                let view = w.meshopt_view(&raw, 4, n, "ATTRIBUTES", Some(34962), 0);
                let a = w.accessor(view, 5121, n, "VEC4", true, None, None);
                attrs.insert("WEIGHTS_0".into(), json!(a));
            }
            let mut targets = vec![];
            for t in &p.targets {
                let mut tj = Map::new();
                // Deltas live in the quantized space too.
                let k = qframe[mi].map(|(_, step)| 1.0 / step).unwrap_or(1.0);
                let flat: Vec<f32> = t.pos.iter().flatten().map(|x| x * k).collect();
                let fits = qframe[mi].is_some() && flat.iter().all(|x| x.abs() <= 32767.0);
                let pa = if fits {
                    // Same grid as the base positions: i16, KHR_mesh_quantization.
                    let mut raw = Vec::with_capacity(n * 8);
                    let (mut mn, mut mx) = ([i16::MAX; 3], [i16::MIN; 3]);
                    for v in flat.chunks_exact(3) {
                        for c in 0..3 {
                            let q = v[c].round() as i16;
                            mn[c] = mn[c].min(q);
                            mx[c] = mx[c].max(q);
                            raw.extend_from_slice(&q.to_le_bytes());
                        }
                        raw.extend_from_slice(&[0, 0]);
                    }
                    let view = w.meshopt_view(&raw, 8, n, "ATTRIBUTES", Some(34962), 0);
                    w.accessor(
                        view,
                        5122,
                        n,
                        "VEC3",
                        false,
                        Some(json!(mn)),
                        Some(json!(mx)),
                    )
                } else {
                    w.float_accessor(&flat, 3, "VEC3", Some(34962), true)
                };
                tj.insert("POSITION".into(), json!(pa));
                if let Some(tn) = &t.nrm {
                    let na = if opts.compress {
                        let mut raw = Vec::with_capacity(n * 4);
                        for v in tn {
                            for x in &v[..3] {
                                raw.push(((x.clamp(-1.0, 1.0) * 127.0).round() as i8) as u8);
                            }
                            raw.push(0);
                        }
                        let view = w.meshopt_view(&raw, 4, n, "ATTRIBUTES", Some(34962), 0);
                        w.accessor(view, 5120, n, "VEC3", true, None, None)
                    } else {
                        let flat: Vec<f32> = tn.iter().flatten().copied().collect();
                        w.float_accessor(&flat, 3, "VEC3", Some(34962), false)
                    };
                    tj.insert("NORMAL".into(), json!(na));
                }
                targets.push(Value::Object(tj));
            }
            // Indices
            let small = n <= 65535;
            let raw: Vec<u8> = if small {
                p.idx
                    .iter()
                    .flat_map(|i| (*i as u16).to_le_bytes())
                    .collect()
            } else {
                p.idx.iter().flat_map(|i| i.to_le_bytes()).collect()
            };
            let stride = if small { 2 } else { 4 };
            let view = w.meshopt_view(&raw, stride, p.idx.len(), "TRIANGLES", Some(34963), n);
            let ia = w.accessor(
                view,
                if small { 5123 } else { 5125 },
                p.idx.len(),
                "SCALAR",
                false,
                None,
                None,
            );
            let mut pj = json!({"attributes": attrs, "indices": ia, "mode": 4});
            if let Some(m) = p.material {
                pj["material"] = json!(m);
            }
            if !targets.is_empty() {
                pj["targets"] = json!(targets);
            }
            prims_json.push(pj);
        }
        let mut mj = json!({"primitives": prims_json});
        if let Some(nm) = &mesh.name {
            mj["name"] = json!(nm);
        }
        if let Some(wt) = &mesh.weights {
            mj["weights"] = json!(wt);
        }
        if let Some(ex) = &mesh.extras {
            mj["extras"] = ex.clone();
        }
        meshes_json.push(mj);
    }

    // ── Nodes ──
    let mut nodes_json = vec![];
    for node in &model.nodes {
        let mut nj = Map::new();
        if let Some(n) = &node.name {
            nj.insert("name".into(), json!(n));
        }
        let (mut t, mut s) = (node.t, node.s);
        if let Some(mi) = node.mesh {
            if let (Some(Some((min, step))), false) =
                (qframe.get(mi), skinned_q.get(mi).copied().unwrap_or(false))
            {
                // node · (min + q·step): fold the dequantization into TRS.
                // Only flattened models quantize and their mesh node is the
                // identity, so this is exact.
                t = [
                    t[0] + min[0] * s[0],
                    t[1] + min[1] * s[1],
                    t[2] + min[2] * s[2],
                ];
                s = [s[0] * step, s[1] * step, s[2] * step];
            }
        }
        if t != [0.0; 3] {
            nj.insert("translation".into(), json!(t));
        }
        if node.r != [0.0, 0.0, 0.0, 1.0] {
            nj.insert("rotation".into(), json!(node.r));
        }
        if s != [1.0; 3] {
            nj.insert("scale".into(), json!(s));
        }
        if let Some(m) = node.mesh {
            nj.insert("mesh".into(), json!(m));
        }
        if let Some(sk) = node.skin {
            nj.insert("skin".into(), json!(sk));
        }
        if !node.children.is_empty() {
            nj.insert("children".into(), json!(node.children));
        }
        if let Some(wt) = &node.weights {
            nj.insert("weights".into(), json!(wt));
        }
        if let Some(ext) = &node.extensions {
            collect_ext_names(ext, &mut w.used);
            nj.insert("extensions".into(), ext.clone());
        }
        nodes_json.push(Value::Object(nj));
    }

    // ── Skins ──
    let mut skins_json = vec![];
    for sk in &model.skins {
        let mut sj = json!({"joints": sk.joints});
        let ibm: Option<Vec<Mat4>> = match (&sk.ibm, skin_dequant) {
            (Some(v), Some(d)) => Some(v.iter().map(|m| mul(m, &d)).collect()),
            (None, Some(d)) => Some(vec![d; sk.joints.len()]),
            (v, None) => v.clone(),
        };
        if let Some(ibm) = &ibm {
            let flat: Vec<f32> = ibm.iter().flatten().copied().collect();
            sj["inverseBindMatrices"] = json!(w.float_accessor(&flat, 16, "MAT4", None, false));
        }
        if let Some(s) = sk.skeleton {
            sj["skeleton"] = json!(s);
        }
        if let Some(n) = &sk.name {
            sj["name"] = json!(n);
        }
        skins_json.push(sj);
    }

    // ── Animations ──
    let mut anims_json = vec![];
    for an in &model.animations {
        let mut samplers = vec![];
        for s in &an.samplers {
            let input = w.float_accessor(&s.input, 1, "SCALAR", None, true);
            let ty = match s.comps {
                1 => "SCALAR",
                2 => "VEC2",
                3 => "VEC3",
                _ => "VEC4",
            };
            let comps = s.comps.clamp(1, 4);
            let data = &s.output[..s.output.len() / comps * comps];
            let path = an
                .channels
                .iter()
                .find(|c| std::ptr::eq(&an.samplers[c.sampler.min(an.samplers.len() - 1)], s))
                .map(|c| c.path.as_str());
            let output = if opts.compress
                && comps == 4
                && path == Some("rotation")
                && s.interpolation != "CUBICSPLINE"
            {
                // Unit quaternions: 16-bit QUATERNION filter, SHORT normalized.
                let q: Vec<[f32; 4]> = data
                    .chunks_exact(4)
                    .map(|c| {
                        let l = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2] + c[3] * c[3])
                            .sqrt()
                            .max(1e-9);
                        [c[0] / l, c[1] / l, c[2] / l, c[3] / l]
                    })
                    .collect();
                let raw = meshopt::encode_filter_quat(&q, 16);
                let view = w.filtered_view(&raw, 8, q.len(), "QUATERNION");
                w.accessor(view, 5122, q.len(), "VEC4", true, None, None)
            } else if opts.compress && comps == 3 {
                // Translation/scale: EXPONENTIAL filter, 16-bit mantissa.
                let raw = meshopt::encode_filter_exp(data, 3, 16);
                let view = w.filtered_view(&raw, 12, data.len() / 3, "EXPONENTIAL");
                w.accessor(view, 5126, data.len() / 3, "VEC3", false, None, None)
            } else {
                w.float_accessor(data, comps, ty, None, false)
            };
            samplers
                .push(json!({"input": input, "output": output, "interpolation": s.interpolation}));
        }
        let channels: Vec<Value> = an
            .channels
            .iter()
            .map(|c| json!({"sampler": c.sampler, "target": {"node": c.node, "path": c.path}}))
            .collect();
        let mut aj = json!({"samplers": samplers, "channels": channels});
        if let Some(n) = &an.name {
            aj["name"] = json!(n);
        }
        anims_json.push(aj);
    }

    // ── Images, samplers, textures ──
    let mut images_json = vec![];
    let mut image_slots = vec![];
    for img in &model.images {
        let bytes = img
            .encoded
            .clone()
            .unwrap_or_else(|| crate::texture::encode_webp(&img.rgba, 80.0));
        let slot = w.raw_view(&bytes);
        image_slots.push(slot);
        let mut ij = json!({"bufferView": 0, "mimeType": img.mime});
        if let Some(n) = &img.name {
            ij["name"] = json!(n);
        }
        images_json.push(ij);
    }
    let mut samplers_json: Vec<Value> = vec![];
    let mut sampler_idx: HashMap<String, usize> = HashMap::new();
    let mut textures_json = vec![];
    for t in &model.textures {
        let mut tj = Map::new();
        if let Some(s) = &t.sampler {
            let mut s = s.clone();
            if let Some(o) = s.as_object_mut() {
                o.retain(|k, _| {
                    matches!(k.as_str(), "magFilter" | "minFilter" | "wrapS" | "wrapT")
                });
            }
            let key = s.to_string();
            let i = *sampler_idx.entry(key).or_insert_with(|| {
                samplers_json.push(s);
                samplers_json.len() - 1
            });
            tj.insert("sampler".into(), json!(i));
        }
        if let Some(i) = t.image {
            w.used.insert("EXT_texture_webp".into());
            w.required.insert("EXT_texture_webp".into());
            tj.insert(
                "extensions".into(),
                json!({"EXT_texture_webp": {"source": i}}),
            );
        }
        if let Some(n) = &t.name {
            tj.insert("name".into(), json!(n));
        }
        textures_json.push(Value::Object(tj));
    }

    // ── Materials ──
    let preserve_raw = !model.root_extensions.is_empty() || !model.skins.is_empty();
    let mut materials_json = vec![];
    for m in &model.materials {
        if preserve_raw && m.raw.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            let mut raw = m.raw.clone();
            // Textures and images kept their indices; drop references that
            // no longer exist.
            strip_dangling_textures(&mut raw, model.textures.len());
            collect_ext_names(&raw, &mut w.used);
            if raw.pointer("/extensions/KHR_texture_transform").is_some() {
                w.used.insert("KHR_texture_transform".into());
            }
            materials_json.push(raw);
            continue;
        }
        let mut pbr =
            json!({"baseColorFactor": m.base_color, "metallicFactor": 0.0, "roughnessFactor": 1.0});
        if let Some(t) = m.base_tex {
            pbr["baseColorTexture"] = json!({"index": t});
        }
        let mut mj = json!({"pbrMetallicRoughness": pbr});
        if let Some(n) = &m.name {
            mj["name"] = json!(n);
        }
        if m.alpha_mode != "OPAQUE" {
            mj["alphaMode"] = json!(m.alpha_mode);
            if m.alpha_mode == "MASK" {
                mj["alphaCutoff"] = json!(m.alpha_cutoff);
            }
        }
        if m.double_sided {
            mj["doubleSided"] = json!(true);
        }
        if m.emissive != [0.0; 3] {
            mj["emissiveFactor"] = json!(m.emissive);
        }
        if m.unlit {
            w.used.insert("KHR_materials_unlit".into());
            mj["extensions"] = json!({"KHR_materials_unlit": {}});
        }
        materials_json.push(mj);
    }

    // ── Root ──
    let mut asset = json!({"version": "2.0", "generator": format!("OmniGet {}", crate::types::NORMALIZER_VERSION)});
    if let Some(c) = &opts.copyright {
        asset["copyright"] = json!(c);
    }
    if !opts.asset_extras.is_null() {
        asset["extras"] = opts.asset_extras.clone();
    }
    let mut root = Map::new();
    root.insert("asset".into(), asset);
    root.insert("scene".into(), json!(0));
    root.insert("scenes".into(), json!([{"nodes": model.roots}]));
    root.insert("nodes".into(), json!(nodes_json));
    root.insert("meshes".into(), json!(meshes_json));
    if !materials_json.is_empty() {
        root.insert("materials".into(), json!(materials_json));
    }
    if !textures_json.is_empty() {
        root.insert("textures".into(), json!(textures_json));
    }
    if !samplers_json.is_empty() {
        root.insert("samplers".into(), json!(samplers_json));
    }
    if !skins_json.is_empty() {
        root.insert("skins".into(), json!(skins_json));
    }
    if !anims_json.is_empty() {
        root.insert("animations".into(), json!(anims_json));
    }
    if !model.root_extensions.is_empty() {
        for k in model.root_extensions.keys() {
            w.used.insert(k.clone());
        }
        root.insert(
            "extensions".into(),
            Value::Object(model.root_extensions.clone()),
        );
    }
    let slot_map = w.finalize();
    for (ij, slot) in images_json.iter_mut().zip(&image_slots) {
        ij["bufferView"] = json!(slot_map[*slot].0);
    }
    if !images_json.is_empty() {
        root.insert("images".into(), json!(images_json));
    }
    align(&mut w.bin, 4);
    if opts.compress {
        root.insert(
            "buffers".into(),
            json!([
                {"byteLength": w.bin.len()},
                {"byteLength": w.fallback_len.max(1), "extensions": {"EXT_meshopt_compression": {"fallback": true}}}
            ]),
        );
    } else {
        root.insert("buffers".into(), json!([{"byteLength": w.bin.len()}]));
    }
    root.insert("bufferViews".into(), json!(w.views));
    root.insert("accessors".into(), json!(w.accessors));
    if !w.used.is_empty() {
        root.insert(
            "extensionsUsed".into(),
            json!(w.used.iter().collect::<Vec<_>>()),
        );
    }
    if !w.required.is_empty() {
        root.insert(
            "extensionsRequired".into(),
            json!(w.required.iter().collect::<Vec<_>>()),
        );
    }

    let mut json_bytes = serde_json::to_vec(&Value::Object(root)).unwrap_or_default();
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let total = 12 + 8 + json_bytes.len() + 8 + w.bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(w.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&w.bin);
    out
}

/// u8 weights that sum to exactly 255.
fn quantize_weights(w: [f32; 4]) -> [u8; 4] {
    let sum: f32 = w.iter().map(|x| x.max(0.0)).sum();
    if sum <= 1e-6 {
        return [255, 0, 0, 0];
    }
    let mut q = w.map(|x| ((x.max(0.0) / sum) * 255.0).round() as i32);
    let diff = 255 - q.iter().sum::<i32>();
    let big = (0..4).max_by_key(|&i| q[i]).unwrap_or(0);
    q[big] += diff;
    q.map(|x| x.clamp(0, 255) as u8)
}

fn collect_ext_names(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                if k == "extensions" {
                    if let Some(e) = x.as_object() {
                        for name in e.keys() {
                            out.insert(name.clone());
                        }
                    }
                }
                collect_ext_names(x, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_ext_names(x, out);
            }
        }
        _ => {}
    }
}

fn strip_dangling_textures(v: &mut Value, texture_count: usize) {
    if let Value::Object(o) = v {
        let dangling: Vec<String> = o
            .iter()
            .filter(|(k, x)| {
                k.ends_with("Texture")
                    && x.get("index")
                        .and_then(Value::as_u64)
                        .map(|i| i as usize >= texture_count)
                        .unwrap_or(false)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in dangling {
            o.remove(&k);
        }
        for (_, x) in o.iter_mut() {
            strip_dangling_textures(x, texture_count);
        }
    } else if let Value::Array(a) = v {
        for x in a {
            strip_dangling_textures(x, texture_count);
        }
    }
}
