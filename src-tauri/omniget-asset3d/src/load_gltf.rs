//! GLB / glTF / VRM reader for hostile input.
//!
//! The `gltf` crate validates the document (every index points somewhere);
//! the bytes are read here with checked arithmetic, because a lying accessor
//! (`count` past the end of its view, a stride smaller than the element) must
//! fail cleanly instead of panicking. External URIs are refused unless the
//! caller passes a resolver for sibling files; data URIs have a size cap.

use crate::meshopt;
use crate::model::*;
use crate::types::Limits;
use crate::Error;
use base64::Engine;
use gltf::accessor::{DataType, Dimensions};
use gltf::mesh::Mode;
use gltf::Semantic;
use serde_json::Value;

/// Reads a file next to a `.gltf` (buffers and images). Gets the URI already
/// percent-decoded; must refuse anything that is not a plain sibling name.
pub type Resolver<'a> = &'a dyn Fn(&str) -> Result<Vec<u8>, Error>;

const SUPPORTED_REQUIRED: &[&str] = &[
    "KHR_mesh_quantization",
    "EXT_meshopt_compression",
    "KHR_meshopt_compression",
    "EXT_texture_webp",
    "KHR_texture_transform",
    "KHR_materials_unlit",
];

/// Images are kept at most this big after decoding.
pub const WORKING_TEXTURE_PX: u32 = 1024;

struct Budget {
    left: usize,
}

impl Budget {
    fn take(&mut self, n: usize, what: &str) -> Result<(), Error> {
        if n > self.left {
            return Err(Error::Limit(format!(
                "{what} needs more memory than the import allows"
            )));
        }
        self.left -= n;
        Ok(())
    }
}

pub fn split_glb(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), Error> {
    let bad = |m: &str| Error::Malformed(format!("GLB: {m}"));
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err(bad("missing header"));
    }
    let rd = |o: usize| {
        u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as usize
    };
    if rd(4) != 2 {
        return Err(bad("only glTF 2.0 is supported"));
    }
    let total = rd(8).min(bytes.len());
    let mut off = 12;
    let mut json = None;
    let mut bin = None;
    while off + 8 <= total {
        let len = rd(off);
        let kind = rd(off + 4);
        let start = off + 8;
        let end = start
            .checked_add(len)
            .ok_or_else(|| bad("chunk length overflow"))?;
        if end > total {
            return Err(bad("chunk runs past the end of the file"));
        }
        match kind {
            0x4E4F_534A if json.is_none() => json = Some(&bytes[start..end]),
            0x004E_4942 if bin.is_none() => bin = Some(&bytes[start..end]),
            _ => {}
        }
        off = end + ((4 - end % 4) % 4);
    }
    Ok((json.ok_or_else(|| bad("no JSON chunk"))?, bin))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn load_uri(uri: &str, limits: &Limits, resolver: Option<Resolver>) -> Result<Vec<u8>, Error> {
    if let Some(rest) = uri.strip_prefix("data:") {
        if uri.len() > limits.max_data_uri_bytes {
            return Err(Error::Limit(
                "data URI is larger than the import allows".into(),
            ));
        }
        let Some((head, body)) = rest.split_once(',') else {
            return Err(Error::Malformed("data URI without payload".into()));
        };
        if !head.ends_with(";base64") {
            return Err(Error::Malformed(
                "only base64 data URIs are supported".into(),
            ));
        }
        return base64::engine::general_purpose::STANDARD
            .decode(body.trim())
            .map_err(|_| Error::Malformed("bad base64 in data URI".into()));
    }
    let name = percent_decode(uri);
    let plain = !name.is_empty()
        && !name.contains("..")
        && !name.contains(':')
        && !name.starts_with('/')
        && !name.starts_with('\\');
    match resolver {
        Some(r) if plain => r(&name),
        _ => Err(Error::Refused(format!(
            "external reference \"{}\" is not allowed; use a .glb or embed the data",
            uri.chars().take(80).collect::<String>()
        ))),
    }
}

pub struct Loaded {
    pub model: Model,
    pub warnings: Vec<String>,
    pub extensions_used: Vec<String>,
}

pub fn load(bytes: &[u8], limits: &Limits, resolver: Option<Resolver>) -> Result<Loaded, Error> {
    let (json_bytes, bin) = if bytes.starts_with(b"glTF") {
        split_glb(bytes)?
    } else {
        (bytes, None)
    };
    let mut root: Value = serde_json::from_slice(json_bytes)
        .map_err(|e| Error::Malformed(format!("glTF JSON: {e}")))?;
    let mut warnings = vec![];

    // Required extensions we cannot honour are refused with their name.
    let required: Vec<String> = root
        .get("extensionsRequired")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    for r in &required {
        if !SUPPORTED_REQUIRED.contains(&r.as_str()) {
            return Err(Error::Unsupported(format!(
                "the file requires {r}; export it again without that compression"
            )));
        }
    }
    let extensions_used: Vec<String> = root
        .get("extensionsUsed")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Let the typed parser accept what we handle ourselves.
    if let Some(obj) = root.as_object_mut() {
        obj.remove("extensionsRequired");
    }
    if let Some(texs) = root.get_mut("textures").and_then(Value::as_array_mut) {
        for t in texs {
            let alt = t
                .pointer("/extensions/EXT_texture_webp/source")
                .cloned()
                .or_else(|| t.pointer("/extensions/KHR_texture_basisu/source").cloned());
            if t.get("source").is_none() {
                if let (Some(a), Some(o)) = (alt, t.as_object_mut()) {
                    o.insert("source".into(), a);
                }
            }
        }
    }

    // Structural caps before the typed parse allocates anything big.
    let count = |k: &str| {
        root.get(k)
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0)
    };
    if count("nodes") > limits.max_nodes {
        return Err(Error::Limit(format!(
            "{} nodes (max {})",
            count("nodes"),
            limits.max_nodes
        )));
    }
    if count("images") > limits.max_textures {
        return Err(Error::Limit(format!(
            "{} images (max {})",
            count("images"),
            limits.max_textures
        )));
    }

    let cleaned = serde_json::to_vec(&root).map_err(|e| Error::Malformed(e.to_string()))?;
    let gltf =
        gltf::Gltf::from_slice(&cleaned).map_err(|e| Error::Malformed(format!("glTF: {e}")))?;
    let doc = &gltf.document;

    let mut budget = Budget {
        left: limits.max_decoded_bytes,
    };

    // ── Buffers ──
    let mut buffers: Vec<Option<Vec<u8>>> = vec![];
    for (i, b) in root
        .get("buffers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let declared = b.get("byteLength").and_then(Value::as_u64).unwrap_or(0) as usize;
        let data = match b.get("uri").and_then(Value::as_str) {
            Some(uri) => {
                let d = load_uri(uri, limits, resolver)?;
                budget.take(d.len(), "buffer")?;
                Some(d)
            }
            None if i == 0 && bin.is_some() => Some(bin.unwrap_or_default().to_vec()),
            None => None, // meshopt fallback buffer
        };
        if let Some(d) = &data {
            if d.len() < declared {
                return Err(Error::Malformed(format!(
                    "buffer {i} is shorter than its byteLength"
                )));
            }
        }
        buffers.push(data);
    }

    // ── Buffer views (decompressing meshopt streams) ──
    let views_json = root
        .get("bufferViews")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut views: Vec<Option<Vec<u8>>> = Vec::with_capacity(views_json.len());
    for (i, v) in views_json.iter().enumerate() {
        let ext = v
            .pointer("/extensions/EXT_meshopt_compression")
            .or_else(|| v.pointer("/extensions/KHR_meshopt_compression"));
        let u = |o: &Value, k: &str| o.get(k).and_then(Value::as_u64).map(|x| x as usize);
        if let Some(e) = ext {
            let buf = u(e, "buffer").unwrap_or(usize::MAX);
            let off = u(e, "byteOffset").unwrap_or(0);
            let len = u(e, "byteLength").unwrap_or(0);
            let stride = u(e, "byteStride").unwrap_or(0);
            let cnt = u(e, "count").unwrap_or(0);
            let mode = e.get("mode").and_then(Value::as_str).unwrap_or("");
            let filter = e.get("filter").and_then(Value::as_str).unwrap_or("NONE");
            let src = buffers
                .get(buf)
                .and_then(|b| b.as_ref())
                .and_then(|b| b.get(off..off.checked_add(len)?))
                .ok_or_else(|| Error::Malformed(format!("meshopt view {i} is out of bounds")))?;
            let out_len = cnt
                .checked_mul(stride)
                .ok_or_else(|| Error::Limit("meshopt view too big".into()))?;
            budget.take(out_len, "meshopt stream")?;
            let decoded = match mode {
                "ATTRIBUTES" => {
                    if stride == 0 || stride % 4 != 0 || stride > 256 {
                        return Err(Error::Malformed(format!("meshopt view {i}: bad stride")));
                    }
                    let mut d =
                        meshopt::decode_vertex_buffer(cnt, stride, src).ok_or_else(|| {
                            Error::Malformed(format!("meshopt view {i} does not decode"))
                        })?;
                    if !meshopt::decode_filter(filter, &mut d, cnt, stride) {
                        return Err(Error::Malformed(format!("meshopt view {i}: bad filter")));
                    }
                    d
                }
                "TRIANGLES" => meshopt::decode_index_buffer(cnt, stride, src)
                    .ok_or_else(|| Error::Malformed(format!("meshopt view {i} does not decode")))?,
                "INDICES" => meshopt::decode_index_sequence(cnt, stride, src)
                    .ok_or_else(|| Error::Malformed(format!("meshopt view {i} does not decode")))?,
                _ => return Err(Error::Malformed(format!("meshopt view {i}: unknown mode"))),
            };
            views.push(Some(decoded));
            continue;
        }
        let buf = u(v, "buffer").unwrap_or(usize::MAX);
        let off = u(v, "byteOffset").unwrap_or(0);
        let len = u(v, "byteLength").unwrap_or(0);
        let data = buffers
            .get(buf)
            .and_then(|b| b.as_ref())
            .map(|b| b.get(off..off.checked_add(len)?).map(|s| s.to_vec()));
        match data {
            Some(Some(d)) => views.push(Some(d)),
            Some(None) => {
                return Err(Error::Malformed(format!(
                    "bufferView {i} runs past its buffer"
                )))
            }
            None => views.push(None),
        }
    }

    let mut model = Model::default();
    let mut total_tris = 0usize;

    // ── Meshes ──
    for mesh in doc.meshes() {
        let mut m = Mesh {
            name: mesh.name().map(String::from),
            weights: mesh.weights().map(|w| w.to_vec()),
            extras: root
                .pointer(&format!("/meshes/{}/extras", mesh.index()))
                .cloned(),
            prims: vec![],
        };
        for prim in mesh.primitives() {
            let Some(pos_acc) = prim.get(&Semantic::Positions) else {
                continue;
            };
            let mode = prim.mode();
            if !matches!(
                mode,
                Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan
            ) {
                warnings.push("points and lines were dropped".into());
                continue;
            }
            let pos = read_vec3(&pos_acc, &views, &mut budget)?;
            let n = pos.len();
            let raw_idx = match prim.indices() {
                Some(a) => read_uints(&a, &views, &mut budget)?,
                None => (0..n as u32).collect(),
            };
            if raw_idx.iter().any(|&i| i as usize >= n) {
                return Err(Error::Malformed(
                    "an index points past the vertex list".into(),
                ));
            }
            let idx = triangulate(mode, &raw_idx);
            total_tris += idx.len() / 3;
            if total_tris > limits.max_triangles {
                return Err(Error::Limit(format!(
                    "more than {} triangles",
                    limits.max_triangles
                )));
            }
            let nrm = match prim.get(&Semantic::Normals) {
                Some(a) => Some(read_vec3(&a, &views, &mut budget)?).filter(|v| v.len() == n),
                None => None,
            };
            let uv = match prim.get(&Semantic::TexCoords(0)) {
                Some(a) => Some(read_vec2(&a, &views, &mut budget)?).filter(|v| v.len() == n),
                None => None,
            };
            let col = match prim.get(&Semantic::Colors(0)) {
                Some(a) => Some(read_color(&a, &views, &mut budget)?).filter(|v| v.len() == n),
                None => None,
            };
            let joints = match prim.get(&Semantic::Joints(0)) {
                Some(a) => Some(read_joints(&a, &views, &mut budget)?).filter(|v| v.len() == n),
                None => None,
            };
            let weights = match prim.get(&Semantic::Weights(0)) {
                Some(a) => Some(read_vec4(&a, &views, &mut budget)?).filter(|v| v.len() == n),
                None => None,
            };
            let mut targets = vec![];
            for t in prim.morph_targets() {
                let tp = match t.positions() {
                    Some(a) => read_vec3(&a, &views, &mut budget)?,
                    None => vec![[0.0; 3]; n],
                };
                let tn = match t.normals() {
                    Some(a) => Some(read_vec3(&a, &views, &mut budget)?).filter(|v| v.len() == n),
                    None => None,
                };
                if tp.len() != n {
                    return Err(Error::Malformed(
                        "morph target size differs from the mesh".into(),
                    ));
                }
                targets.push(MorphTarget { pos: tp, nrm: tn });
            }
            let (joints, weights) = match (joints, weights) {
                (Some(j), Some(w)) => (Some(j), Some(w)),
                _ => (None, None),
            };
            m.prims.push(Prim {
                pos,
                nrm,
                uv,
                col,
                joints,
                weights,
                targets,
                idx,
                material: prim.material().index(),
            });
        }
        model.meshes.push(m);
    }

    // ── Nodes ──
    for node in doc.nodes() {
        let (t, r, s) = node.transform().decomposed();
        model.nodes.push(Node {
            name: node.name().map(String::from),
            t,
            r,
            s,
            mesh: node.mesh().map(|m| m.index()),
            skin: node.skin().map(|s| s.index()),
            children: node.children().map(|c| c.index()).collect(),
            weights: node.weights().map(|w| w.to_vec()),
            extensions: root
                .pointer(&format!("/nodes/{}/extensions", node.index()))
                .cloned(),
        });
    }
    check_tree(&model.nodes)?;
    let scene = doc.default_scene().or_else(|| doc.scenes().next());
    if doc.scenes().count() > 1 {
        warnings.push(format!(
            "the file has {} scenes; only the default one was imported",
            doc.scenes().count()
        ));
    }
    model.roots = match scene {
        Some(s) => s.nodes().map(|n| n.index()).collect(),
        None => {
            // No scene: every node without a parent is a root.
            let mut has_parent = vec![false; model.nodes.len()];
            for n in &model.nodes {
                for &c in &n.children {
                    has_parent[c] = true;
                }
            }
            (0..model.nodes.len()).filter(|&i| !has_parent[i]).collect()
        }
    };

    // ── Skins ──
    for skin in doc.skins() {
        if skin.joints().count() > limits.max_bones {
            return Err(Error::Limit(format!(
                "skin with more than {} bones",
                limits.max_bones
            )));
        }
        let ibm = match skin.inverse_bind_matrices() {
            Some(a) => {
                let (f, comps) = read_floats(&a, &views, &mut budget)?;
                if comps != 16 {
                    return Err(Error::Malformed(
                        "inverse bind matrices are not MAT4".into(),
                    ));
                }
                Some(
                    f.chunks_exact(16)
                        .map(|c| c.try_into().unwrap_or(IDENTITY))
                        .collect(),
                )
            }
            None => None,
        };
        model.skins.push(Skin {
            name: skin.name().map(String::from),
            joints: skin.joints().map(|j| j.index()).collect(),
            ibm,
            skeleton: skin.skeleton().map(|s| s.index()),
        });
    }

    // ── Animations ──
    for anim in doc.animations() {
        let mut a = Animation {
            name: anim.name().map(String::from),
            ..Default::default()
        };
        for s in anim.samplers() {
            let (input, ic) = read_floats(&s.input(), &views, &mut budget)?;
            let (output, oc) = read_floats(&s.output(), &views, &mut budget)?;
            if ic != 1 || input.is_empty() {
                return Err(Error::Malformed(
                    "animation input is not a scalar list".into(),
                ));
            }
            a.samplers.push(AnimSampler {
                input,
                output,
                comps: oc,
                interpolation: match s.interpolation() {
                    gltf::animation::Interpolation::Linear => "LINEAR",
                    gltf::animation::Interpolation::Step => "STEP",
                    gltf::animation::Interpolation::CubicSpline => "CUBICSPLINE",
                }
                .into(),
            });
        }
        for c in anim.channels() {
            a.channels.push(AnimChannel {
                sampler: c.sampler().index(),
                node: c.target().node().index(),
                path: match c.target().property() {
                    gltf::animation::Property::Translation => "translation",
                    gltf::animation::Property::Rotation => "rotation",
                    gltf::animation::Property::Scale => "scale",
                    gltf::animation::Property::MorphTargetWeights => "weights",
                }
                .into(),
            });
        }
        model.animations.push(a);
    }

    // ── Images, textures, materials (raw JSON) ──
    let images_json = root
        .get("images")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (i, im) in images_json.iter().enumerate() {
        let bytes = if let Some(v) = im.get("bufferView").and_then(Value::as_u64) {
            views
                .get(v as usize)
                .and_then(|x| x.clone())
                .ok_or_else(|| Error::Malformed(format!("image {i} has no data")))?
        } else if let Some(uri) = im.get("uri").and_then(Value::as_str) {
            load_uri(uri, limits, resolver)?
        } else {
            return Err(Error::Malformed(format!("image {i} has no source")));
        };
        let full = decode_image(&bytes, limits, &mut budget)
            .map_err(|e| Error::Malformed(format!("image {i}: {e}")))?;
        // Nothing downstream uses more than 1024 px: shrink now and give the
        // memory back to the budget.
        let (w, h) = full.dimensions();
        let rgba = if w.max(h) > WORKING_TEXTURE_PX {
            let k = WORKING_TEXTURE_PX as f32 / w.max(h) as f32;
            let small = image::imageops::resize(
                &full,
                ((w as f32 * k) as u32).max(1),
                ((h as f32 * k) as u32).max(1),
                image::imageops::FilterType::Triangle,
            );
            budget.left += (w as usize * h as usize * 8).saturating_sub(small.len() * 2);
            small
        } else {
            full
        };
        model.images.push(ImageData {
            name: im.get("name").and_then(Value::as_str).map(String::from),
            rgba,
            encoded: None,
            mime: "image/webp".into(),
        });
    }
    let samplers = root
        .get("samplers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for t in root
        .get("textures")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let src = t.get("source").and_then(Value::as_u64).map(|x| x as usize);
        model.textures.push(Texture {
            name: t.get("name").and_then(Value::as_str).map(String::from),
            image: src.filter(|&s| s < model.images.len()),
            sampler: t
                .get("sampler")
                .and_then(Value::as_u64)
                .and_then(|s| samplers.get(s as usize).cloned()),
        });
    }
    for m in root
        .get("materials")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        model
            .materials
            .push(material_from_json(m, model.textures.len()));
    }

    if let Some(ext) = root.get("extensions").and_then(Value::as_object) {
        for (k, v) in ext {
            if k == "VRM" || k.starts_with("VRMC_") {
                model.root_extensions.insert(k.clone(), v.clone());
            }
        }
    }

    Ok(Loaded {
        model,
        warnings,
        extensions_used,
    })
}

pub fn material_from_json(m: Value, texture_count: usize) -> Material {
    let f = |p: &str| m.pointer(p).and_then(Value::as_f64).map(|x| x as f32);
    let arr = |p: &str, n: usize| -> Option<Vec<f32>> {
        let a = m.pointer(p)?.as_array()?;
        (a.len() == n).then(|| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
    };
    let bc = arr("/pbrMetallicRoughness/baseColorFactor", 4).unwrap_or(vec![1.0; 4]);
    let em = arr("/emissiveFactor", 3).unwrap_or(vec![0.0; 3]);
    Material {
        name: m.get("name").and_then(Value::as_str).map(String::from),
        base_color: [bc[0], bc[1], bc[2], bc[3]],
        base_tex: m
            .pointer("/pbrMetallicRoughness/baseColorTexture/index")
            .and_then(Value::as_u64)
            .map(|x| x as usize)
            .filter(|&x| x < texture_count),
        alpha_mode: m
            .get("alphaMode")
            .and_then(Value::as_str)
            .unwrap_or("OPAQUE")
            .to_string(),
        alpha_cutoff: f("/alphaCutoff").unwrap_or(0.5),
        double_sided: m
            .get("doubleSided")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        emissive: [em[0], em[1], em[2]],
        unlit: m.pointer("/extensions/KHR_materials_unlit").is_some(),
        raw: m,
    }
}

fn check_tree(nodes: &[Node]) -> Result<(), Error> {
    // Each node has at most one parent and there are no cycles.
    let mut parent = vec![usize::MAX; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for &c in &n.children {
            if c >= nodes.len() || parent[c] != usize::MAX || c == i {
                return Err(Error::Malformed("node hierarchy is not a tree".into()));
            }
            parent[c] = i;
        }
    }
    for start in 0..nodes.len() {
        let mut cur = start;
        let mut steps = 0;
        while parent[cur] != usize::MAX {
            cur = parent[cur];
            steps += 1;
            if steps > nodes.len() {
                return Err(Error::Malformed("node hierarchy has a cycle".into()));
            }
        }
    }
    Ok(())
}

pub fn decode_image(
    bytes: &[u8],
    limits: &Limits,
    budget_left: &mut impl BudgetLike,
) -> Result<image::RgbaImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut lim = image::Limits::default();
    lim.max_image_width = Some(limits.max_texture_px);
    lim.max_image_height = Some(limits.max_texture_px);
    lim.max_alloc = Some((limits.max_texture_px as u64).pow(2) * 4 + (64 << 20));
    reader.limits(lim);
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    budget_left.take_bytes(w as usize * h as usize * 8)?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut lim = image::Limits::default();
    lim.max_image_width = Some(limits.max_texture_px);
    lim.max_image_height = Some(limits.max_texture_px);
    reader.limits(lim);
    let img = reader.decode().map_err(|e| e.to_string())?;
    Ok(img.to_rgba8())
}

pub trait BudgetLike {
    fn take_bytes(&mut self, n: usize) -> Result<(), String>;
}

impl BudgetLike for Budget {
    fn take_bytes(&mut self, n: usize) -> Result<(), String> {
        self.take(n, "image").map_err(|e| e.to_string())
    }
}

/// Unlimited budget for callers that already capped their input.
pub struct NoBudget;

impl BudgetLike for NoBudget {
    fn take_bytes(&mut self, _n: usize) -> Result<(), String> {
        Ok(())
    }
}

fn triangulate(mode: Mode, idx: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(idx.len());
    match mode {
        Mode::Triangles => {
            out.extend_from_slice(&idx[..idx.len() / 3 * 3]);
        }
        Mode::TriangleStrip => {
            for i in 2..idx.len() {
                if i % 2 == 0 {
                    out.extend_from_slice(&[idx[i - 2], idx[i - 1], idx[i]]);
                } else {
                    out.extend_from_slice(&[idx[i - 1], idx[i - 2], idx[i]]);
                }
            }
        }
        Mode::TriangleFan => {
            for i in 2..idx.len() {
                out.extend_from_slice(&[idx[0], idx[i - 1], idx[i]]);
            }
        }
        _ => {}
    }
    out
}

// ── Accessor reading ───────────────────────────────────────────────────────

fn comp_size(dt: DataType) -> usize {
    match dt {
        DataType::I8 | DataType::U8 => 1,
        DataType::I16 | DataType::U16 => 2,
        DataType::U32 | DataType::F32 => 4,
    }
}

fn comps_of(d: Dimensions) -> usize {
    match d {
        Dimensions::Scalar => 1,
        Dimensions::Vec2 => 2,
        Dimensions::Vec3 => 3,
        Dimensions::Vec4 => 4,
        Dimensions::Mat2 => 4,
        Dimensions::Mat3 => 9,
        Dimensions::Mat4 => 16,
    }
}

fn read_comp(b: &[u8], dt: DataType, normalized: bool) -> f32 {
    match dt {
        DataType::I8 => {
            let v = b[0] as i8 as f32;
            if normalized {
                (v / 127.0).max(-1.0)
            } else {
                v
            }
        }
        DataType::U8 => {
            let v = b[0] as f32;
            if normalized {
                v / 255.0
            } else {
                v
            }
        }
        DataType::I16 => {
            let v = i16::from_le_bytes([b[0], b[1]]) as f32;
            if normalized {
                (v / 32767.0).max(-1.0)
            } else {
                v
            }
        }
        DataType::U16 => {
            let v = u16::from_le_bytes([b[0], b[1]]) as f32;
            if normalized {
                v / 65535.0
            } else {
                v
            }
        }
        DataType::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
        DataType::F32 => {
            let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if v.is_finite() {
                v
            } else {
                0.0
            }
        }
    }
}

fn read_comp_u32(b: &[u8], dt: DataType) -> u32 {
    match dt {
        DataType::I8 | DataType::U8 => b[0] as u32,
        DataType::I16 | DataType::U16 => u16::from_le_bytes([b[0], b[1]]) as u32,
        DataType::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        DataType::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]).max(0.0) as u32,
    }
}

/// Byte slices of each element of an accessor, bounds-checked.
fn elements(
    view_bytes: &[u8],
    stride: Option<usize>,
    offset: usize,
    count: usize,
    elem: usize,
) -> Result<Vec<&[u8]>, Error> {
    let stride = stride.unwrap_or(elem);
    if stride < elem {
        return Err(Error::Malformed(
            "accessor stride is smaller than its element".into(),
        ));
    }
    if count == 0 {
        return Ok(vec![]);
    }
    let last = stride
        .checked_mul(count - 1)
        .and_then(|x| x.checked_add(offset))
        .and_then(|x| x.checked_add(elem))
        .ok_or_else(|| Error::Malformed("accessor size overflows".into()))?;
    if last > view_bytes.len() {
        return Err(Error::Malformed(
            "accessor count runs past its buffer view".into(),
        ));
    }
    Ok((0..count)
        .map(|i| &view_bytes[offset + i * stride..offset + i * stride + elem])
        .collect())
}

/// Generic reader: every component as f32 (normalized ints mapped to 0..1 or
/// −1..1). Returns `(values, components_per_element)`.
fn read_floats(
    acc: &gltf::Accessor,
    views: &[Option<Vec<u8>>],
    budget: &mut Budget,
) -> Result<(Vec<f32>, usize), Error> {
    let dt = acc.data_type();
    let comps = comps_of(acc.dimensions());
    let cs = comp_size(dt);
    let count = acc.count();
    budget.take(count.saturating_mul(comps).saturating_mul(4), "accessor")?;
    let mut out = vec![0f32; count * comps];
    if let Some(view) = acc.view() {
        let bytes = views
            .get(view.index())
            .and_then(|v| v.as_deref())
            .ok_or_else(|| Error::Malformed("accessor view has no data".into()))?;
        for (i, e) in elements(bytes, view.stride(), acc.offset(), count, cs * comps)?
            .iter()
            .enumerate()
        {
            for c in 0..comps {
                out[i * comps + c] = read_comp(&e[c * cs..], dt, acc.normalized());
            }
        }
    }
    if let Some(sp) = acc.sparse() {
        let n = sp.count();
        let iv = sp.indices();
        let ibytes = views
            .get(iv.view().index())
            .and_then(|v| v.as_deref())
            .ok_or_else(|| Error::Malformed("sparse indices have no data".into()))?;
        let its = match iv.index_type() {
            gltf::accessor::sparse::IndexType::U8 => DataType::U8,
            gltf::accessor::sparse::IndexType::U16 => DataType::U16,
            gltf::accessor::sparse::IndexType::U32 => DataType::U32,
        };
        let idx = elements(ibytes, None, iv.offset(), n, comp_size(its))?;
        let vv = sp.values();
        let vbytes = views
            .get(vv.view().index())
            .and_then(|v| v.as_deref())
            .ok_or_else(|| Error::Malformed("sparse values have no data".into()))?;
        let vals = elements(vbytes, None, vv.offset(), n, cs * comps)?;
        for (ie, ve) in idx.iter().zip(vals.iter()) {
            let target = read_comp_u32(ie, its) as usize;
            if target >= count {
                return Err(Error::Malformed("sparse index past the accessor".into()));
            }
            for c in 0..comps {
                out[target * comps + c] = read_comp(&ve[c * cs..], dt, acc.normalized());
            }
        }
    }
    Ok((out, comps))
}

fn read_uints(
    acc: &gltf::Accessor,
    views: &[Option<Vec<u8>>],
    budget: &mut Budget,
) -> Result<Vec<u32>, Error> {
    let dt = acc.data_type();
    if comps_of(acc.dimensions()) != 1 || matches!(dt, DataType::F32 | DataType::I8 | DataType::I16)
    {
        return Err(Error::Malformed("indices must be unsigned scalars".into()));
    }
    let cs = comp_size(dt);
    let count = acc.count();
    budget.take(count.saturating_mul(4), "indices")?;
    let view = acc
        .view()
        .ok_or_else(|| Error::Malformed("indices without a buffer view".into()))?;
    let bytes = views
        .get(view.index())
        .and_then(|v| v.as_deref())
        .ok_or_else(|| Error::Malformed("index view has no data".into()))?;
    Ok(elements(bytes, view.stride(), acc.offset(), count, cs)?
        .iter()
        .map(|e| read_comp_u32(e, dt))
        .collect())
}

fn read_vec3(
    acc: &gltf::Accessor,
    views: &[Option<Vec<u8>>],
    budget: &mut Budget,
) -> Result<Vec<[f32; 3]>, Error> {
    let (f, c) = read_floats(acc, views, budget)?;
    if c != 3 {
        return Err(Error::Malformed("expected a VEC3 accessor".into()));
    }
    Ok(f.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect())
}

fn read_vec2(
    acc: &gltf::Accessor,
    views: &[Option<Vec<u8>>],
    budget: &mut Budget,
) -> Result<Vec<[f32; 2]>, Error> {
    let (f, c) = read_floats(acc, views, budget)?;
    if c != 2 {
        return Err(Error::Malformed("expected a VEC2 accessor".into()));
    }
    Ok(f.chunks_exact(2).map(|v| [v[0], v[1]]).collect())
}

fn read_vec4(
    acc: &gltf::Accessor,
    views: &[Option<Vec<u8>>],
    budget: &mut Budget,
) -> Result<Vec<[f32; 4]>, Error> {
    let (f, c) = read_floats(acc, views, budget)?;
    if c != 4 {
        return Err(Error::Malformed("expected a VEC4 accessor".into()));
    }
    Ok(f.chunks_exact(4)
        .map(|v| [v[0], v[1], v[2], v[3]])
        .collect())
}

fn read_color(
    acc: &gltf::Accessor,
    views: &[Option<Vec<u8>>],
    budget: &mut Budget,
) -> Result<Vec<[f32; 4]>, Error> {
    let (f, c) = read_floats(acc, views, budget)?;
    match c {
        3 => Ok(f.chunks_exact(3).map(|v| [v[0], v[1], v[2], 1.0]).collect()),
        4 => Ok(f
            .chunks_exact(4)
            .map(|v| [v[0], v[1], v[2], v[3]])
            .collect()),
        _ => Err(Error::Malformed("COLOR_0 must be VEC3 or VEC4".into())),
    }
}

fn read_joints(
    acc: &gltf::Accessor,
    views: &[Option<Vec<u8>>],
    budget: &mut Budget,
) -> Result<Vec<[u16; 4]>, Error> {
    let dt = acc.data_type();
    if comps_of(acc.dimensions()) != 4 {
        return Err(Error::Malformed("JOINTS_0 must be VEC4".into()));
    }
    let cs = comp_size(dt);
    let count = acc.count();
    budget.take(count.saturating_mul(8), "joints")?;
    let view = acc
        .view()
        .ok_or_else(|| Error::Malformed("joints without a buffer view".into()))?;
    let bytes = views
        .get(view.index())
        .and_then(|v| v.as_deref())
        .ok_or_else(|| Error::Malformed("joint view has no data".into()))?;
    Ok(elements(bytes, view.stride(), acc.offset(), count, cs * 4)?
        .iter()
        .map(|e| {
            let g = |c: usize| read_comp_u32(&e[c * cs..], dt).min(u16::MAX as u32) as u16;
            [g(0), g(1), g(2), g(3)]
        })
        .collect())
}
