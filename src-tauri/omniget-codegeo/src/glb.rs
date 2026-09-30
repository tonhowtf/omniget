//! Minimal glTF 2.0 binary writer: positions, flat normals, indices, one
//! material per palette slot (`slot:<slot>`) and named nodes. Anchors become
//! child nodes `anchor:<action>` with their data in `extras`.

use crate::canon::{CanonMesh, Q};
use crate::palette::{rgb, slot_name};
use crate::AnchorPoint;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct GlbNode {
    pub name: String,
    /// Index into the mesh list, `None` for an empty module.
    pub mesh: Option<usize>,
    pub anchors: Vec<AnchorPoint>,
    pub extras: Option<Value>,
}

fn srgb_to_linear(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    let l = if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    };
    (l * 10_000.0).round() / 10_000.0
}

struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn push(&mut self, bytes: &[u8], target: u32) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let off = self.data.len();
        self.data.extend_from_slice(bytes);
        self.views.push(
            json!({"buffer": 0, "byteOffset": off, "byteLength": bytes.len(), "target": target}),
        );
        self.views.len() - 1
    }
    fn accessor(&mut self, a: Value) -> usize {
        self.accessors.push(a);
        self.accessors.len() - 1
    }
}

/// Writes the GLB. `palette` gives each slot's preview colour.
pub fn write(
    meshes: &[CanonMesh],
    nodes: &[GlbNode],
    palette: &[(&str, &str)],
    generator: &str,
) -> Vec<u8> {
    let colours: BTreeMap<&str, &str> = palette.iter().copied().collect();
    let mut used: Vec<u8> = meshes.iter().flat_map(|m| m.slots()).collect();
    used.sort();
    used.dedup();
    let mat_index: BTreeMap<u8, usize> = used.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let materials: Vec<Value> = used
        .iter()
        .map(|s| {
            let name = slot_name(*s);
            let c = rgb(colours.get(name).copied().unwrap_or("#808080"));
            json!({
                "name": format!("slot:{name}"),
                "pbrMetallicRoughness": {
                    "baseColorFactor": [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2]), 1.0],
                    "metallicFactor": 0.0,
                    "roughnessFactor": 1.0
                }
            })
        })
        .collect();

    let mut bin = Bin {
        data: vec![],
        views: vec![],
        accessors: vec![],
    };
    let mut gl_meshes = vec![];
    for m in meshes {
        let mut prims = vec![];
        for slot in m.slots() {
            // Weld identical (position, normal) pairs inside the slot.
            let mut index: BTreeMap<([i32; 3], [i16; 3]), u32> = BTreeMap::new();
            let mut verts: Vec<([i32; 3], [i16; 3])> = vec![];
            let mut idx: Vec<u32> = vec![];
            for t in m.tris.iter().filter(|t| t.slot == slot) {
                let n = t.normal();
                let qn = [
                    (n[0] * 32767.0).round() as i16,
                    (n[1] * 32767.0).round() as i16,
                    (n[2] * 32767.0).round() as i16,
                ];
                for v in &t.v {
                    let key = (*v, qn);
                    let i = *index.entry(key).or_insert_with(|| {
                        verts.push(key);
                        (verts.len() - 1) as u32
                    });
                    idx.push(i);
                }
            }
            let mut pos = Vec::with_capacity(verts.len() * 12);
            let mut nor = Vec::with_capacity(verts.len() * 12);
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for (p, n) in &verts {
                for k in 0..3 {
                    let f = (p[k] as f64 / Q) as f32;
                    lo[k] = lo[k].min(f);
                    hi[k] = hi[k].max(f);
                    pos.extend_from_slice(&f.to_le_bytes());
                    nor.extend_from_slice(&((n[k] as f32) / 32767.0).to_le_bytes());
                }
            }
            let pv = bin.push(&pos, 34962);
            let pa = bin.accessor(json!({
                "bufferView": pv, "componentType": 5126, "count": verts.len(), "type": "VEC3",
                "min": [lo[0], lo[1], lo[2]], "max": [hi[0], hi[1], hi[2]]
            }));
            let nv = bin.push(&nor, 34962);
            let na = bin.accessor(json!({"bufferView": nv, "componentType": 5126, "count": verts.len(), "type": "VEC3"}));
            let (ibytes, ctype) = if verts.len() <= u16::MAX as usize {
                (
                    idx.iter()
                        .flat_map(|i| (*i as u16).to_le_bytes())
                        .collect::<Vec<u8>>(),
                    5123,
                )
            } else {
                (
                    idx.iter()
                        .flat_map(|i| i.to_le_bytes())
                        .collect::<Vec<u8>>(),
                    5125,
                )
            };
            let iv = bin.push(&ibytes, 34963);
            let ia = bin.accessor(json!({"bufferView": iv, "componentType": ctype, "count": idx.len(), "type": "SCALAR"}));
            prims.push(json!({
                "attributes": {"POSITION": pa, "NORMAL": na},
                "indices": ia,
                "material": mat_index[&slot],
                "mode": 4
            }));
        }
        gl_meshes.push(json!({ "primitives": prims }));
    }

    let mut gl_nodes: Vec<Value> = vec![];
    let mut roots = vec![];
    for n in nodes {
        let mut children = vec![];
        for a in &n.anchors {
            let half = (-a.yaw_deg).to_radians() / 2.0;
            gl_nodes.push(json!({
                "name": format!("anchor:{}", a.action),
                "translation": [a.pos[0] as f32, a.pos[1] as f32, a.pos[2] as f32],
                "rotation": [0.0, half.sin() as f32, 0.0, half.cos() as f32],
                "extras": {"action": a.action, "yaw_deg": a.yaw_deg, "capacity": a.capacity}
            }));
            children.push(gl_nodes.len() - 1);
        }
        let mut node = json!({ "name": n.name });
        if let Some(m) = n.mesh {
            node["mesh"] = json!(m);
        }
        if !children.is_empty() {
            node["children"] = json!(children);
        }
        if let Some(x) = &n.extras {
            node["extras"] = x.clone();
        }
        gl_nodes.push(node);
        roots.push(gl_nodes.len() - 1);
    }

    while !bin.data.len().is_multiple_of(4) {
        bin.data.push(0);
    }
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": generator},
        "scene": 0,
        "scenes": [{"nodes": roots}],
        "nodes": gl_nodes,
    });
    if !gl_meshes.is_empty() {
        doc["meshes"] = json!(gl_meshes);
        doc["materials"] = json!(materials);
        doc["accessors"] = json!(bin.accessors);
        doc["bufferViews"] = json!(bin.views);
        doc["buffers"] = json!([{"byteLength": bin.data.len()}]);
    }
    let mut js = serde_json::to_vec(&doc).expect("json");
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let has_bin = !gl_meshes.is_empty();
    let total = 12 + 8 + js.len() + if has_bin { 8 + bin.data.len() } else { 0 };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    if has_bin {
        out.extend_from_slice(&(bin.data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin.data);
    }
    out
}
