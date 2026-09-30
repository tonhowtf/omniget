//! 2D image to 3D (estudo 06 §3): standee, papercraft extrusion and relief
//! from a depth map. The depth model (Depth Anything V2 Small, Apache-2.0)
//! sits behind [`DepthEstimator`]; [`relief_from_depth`] takes a depth map
//! that was already computed.

use crate::meshopt;
use crate::model::*;
use crate::types::AssetClass;
use crate::Error;

/// Extrusion depth of the papercraft cut-out, in metres.
pub const PAPERCRAFT_DEPTH_M: f32 = 0.04;
/// Longer image side the geometry is built from.
const WORK_PX: u32 = 512;

/// Relative inverse depth, row-major, `1.0` = nearest.
#[derive(Clone, Debug)]
pub struct DepthMap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

impl DepthMap {
    pub fn at_uv(&self, u: f32, v: f32) -> f32 {
        let x = ((u.clamp(0.0, 1.0) * (self.width - 1) as f32).round() as usize)
            .min(self.width as usize - 1);
        let y = ((v.clamp(0.0, 1.0) * (self.height - 1) as f32).round() as usize)
            .min(self.height as usize - 1);
        self.data
            .get(y * self.width as usize + x)
            .copied()
            .unwrap_or(0.0)
    }
}

/// Anything that turns an image into a depth map.
pub trait DepthEstimator: Send + Sync {
    fn estimate(&self, rgba: &image::RgbaImage) -> Result<DepthMap, String>;
}

/// Runs one f32 NCHW tensor through an ONNX model and returns the first
/// output with its shape. The app implements it on its managed ONNX Runtime.
pub trait OnnxRunner: Send + Sync {
    fn run_f32(&self, input: &[f32], shape: [usize; 4]) -> Result<(Vec<f32>, Vec<usize>), String>;
}

/// Depth Anything V2 Small (Apache-2.0; Base and Large are CC-BY-NC and must
/// not be used). ONNX from `onnx-community/depth-anything-v2-small`.
///
/// TODO(city3d): wire an [`OnnxRunner`] over
/// `omniget-core/src/core/onnxrt.rs` (the managed ONNX Runtime, `ort` with
/// `load-dynamic`): download [`DepthAnythingV2Small::MODEL_URL`] into
/// `<app_data>/city3d/models/`, build an `ort::Session` from it and run the
/// tensor this type prepares. Nothing here touches the runtime directly.
pub struct DepthAnythingV2Small<R: OnnxRunner> {
    pub runner: R,
}

impl<R: OnnxRunner> DepthAnythingV2Small<R> {
    pub const MODEL_URL: &'static str =
        "https://huggingface.co/onnx-community/depth-anything-v2-small/resolve/main/onnx/model.onnx";
    pub const INPUT: u32 = 518;
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];

    /// NCHW tensor, 518×518, ImageNet normalization.
    pub fn preprocess(rgba: &image::RgbaImage) -> Vec<f32> {
        let s = Self::INPUT;
        let img = image::imageops::resize(rgba, s, s, image::imageops::FilterType::CatmullRom);
        let plane = (s * s) as usize;
        let mut out = vec![0f32; plane * 3];
        for (i, p) in img.pixels().enumerate() {
            for c in 0..3 {
                out[c * plane + i] = (p.0[c] as f32 / 255.0 - Self::MEAN[c]) / Self::STD[c];
            }
        }
        out
    }
}

impl<R: OnnxRunner> DepthEstimator for DepthAnythingV2Small<R> {
    fn estimate(&self, rgba: &image::RgbaImage) -> Result<DepthMap, String> {
        let s = Self::INPUT as usize;
        let input = Self::preprocess(rgba);
        let (out, shape) = self.runner.run_f32(&input, [1, 3, s, s])?;
        let (h, w) = match shape.as_slice() {
            [_, h, w] => (*h, *w),
            [_, _, h, w] => (*h, *w),
            [h, w] => (*h, *w),
            _ => return Err(format!("unexpected depth output shape {shape:?}")),
        };
        if out.len() < h * w || h == 0 || w == 0 {
            return Err("depth output is too small".into());
        }
        let (mn, mx) = out[..h * w]
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |a, &v| {
                (a.0.min(v), a.1.max(v))
            });
        let range = (mx - mn).max(1e-6);
        let data = out[..h * w].iter().map(|v| (v - mn) / range).collect();
        Ok(DepthMap {
            width: w as u32,
            height: h as u32,
            data,
        })
    }
}

pub fn decode(bytes: &[u8], max_px: u32) -> Result<image::RgbaImage, Error> {
    let limits = crate::types::Limits {
        max_texture_px: max_px,
        ..Default::default()
    };
    let img = crate::load_gltf::decode_image(bytes, &limits, &mut crate::load_gltf::NoBudget)
        .map_err(Error::Malformed)?;
    let (w, h) = img.dimensions();
    let long = w.max(h);
    Ok(if long > WORK_PX {
        image::imageops::resize(
            &img,
            (w * WORK_PX / long).max(1),
            (h * WORK_PX / long).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    })
}

/// Size (width, height) in metres the image is built at so the class fit
/// keeps it (scale 1) and the extrusion depth stays in centimetres.
fn base_size(class: AssetClass, aspect: f32) -> (f32, f32) {
    match class {
        AssetClass::Building => (8.0, 8.0 / aspect),
        AssetClass::Character => (1.7 * aspect, 1.7),
        AssetClass::Decoration => {
            let long = 0.5;
            if aspect >= 1.0 {
                (long, long / aspect)
            } else {
                (long * aspect, long)
            }
        }
        AssetClass::Prop => {
            let long = 1.0;
            if aspect >= 1.0 {
                (long, long / aspect)
            } else {
                (long * aspect, long)
            }
        }
    }
}

fn model_with(prims: Vec<Prim>, rgba: image::RgbaImage, double_sided: bool) -> Model {
    let has_alpha = rgba.pixels().any(|p| p.0[3] < 250);
    let mut m = Model::default();
    m.images.push(ImageData {
        name: Some("image".into()),
        rgba,
        encoded: None,
        mime: "image/webp".into(),
    });
    m.textures.push(Texture {
        name: None,
        image: Some(0),
        sampler: None,
    });
    m.materials.push(Material {
        name: Some("image".into()),
        base_tex: Some(0),
        alpha_mode: if has_alpha {
            "MASK".into()
        } else {
            "OPAQUE".into()
        },
        alpha_cutoff: 0.5,
        double_sided,
        ..Default::default()
    });
    let prims = prims.into_iter().map(|mut p| {
        p.material = Some(0);
        p
    });
    m.meshes.push(Mesh {
        name: Some("image".into()),
        prims: prims.collect(),
        weights: None,
        extras: None,
    });
    m.nodes.push(Node {
        name: Some("image".into()),
        mesh: Some(0),
        ..Default::default()
    });
    m.roots = vec![0];
    m
}

/// Two triangles facing +z with alpha test, double-sided.
pub fn standee(rgba: image::RgbaImage, class: AssetClass) -> Model {
    let (w, h) = rgba.dimensions();
    let (sw, sh) = base_size(class, w as f32 / h.max(1) as f32);
    let x = sw / 2.0;
    let p = Prim {
        pos: vec![[-x, 0.0, 0.0], [x, 0.0, 0.0], [x, sh, 0.0], [-x, sh, 0.0]],
        nrm: Some(vec![[0.0, 0.0, 1.0]; 4]),
        uv: Some(vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
        idx: vec![0, 1, 2, 0, 2, 3],
        ..Default::default()
    };
    model_with(vec![p], rgba, true)
}

/// Outer contours of the alpha mask (threshold 128), in pixel coordinates,
/// one per component with at least `min_px` pixels.
pub fn contours(rgba: &image::RgbaImage, min_px: usize) -> Vec<Vec<[f32; 2]>> {
    let (w, h) = (rgba.width() as i32, rgba.height() as i32);
    let opaque_all = rgba.pixels().all(|p| p.0[3] >= 128);
    let solid = |x: i32, y: i32| -> bool {
        x >= 0
            && y >= 0
            && x < w
            && y < h
            && (opaque_all || rgba.get_pixel(x as u32, y as u32).0[3] >= 128)
    };
    let mut label = vec![0u32; (w * h) as usize];
    let mut out = vec![];
    let mut next = 0u32;
    for y in 0..h {
        for x in 0..w {
            if !solid(x, y) || label[(y * w + x) as usize] != 0 {
                continue;
            }
            next += 1;
            // Flood fill the component.
            let mut stack = vec![(x, y)];
            label[(y * w + x) as usize] = next;
            let mut count = 0usize;
            while let Some((cx, cy)) = stack.pop() {
                count += 1;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if solid(nx, ny) && label[(ny * w + nx) as usize] == 0 {
                        label[(ny * w + nx) as usize] = next;
                        stack.push((nx, ny));
                    }
                }
            }
            if count < min_px {
                continue;
            }
            // (x, y) is the component's first pixel in scan order: its top-left.
            let lab = next;
            let inside = |px: i32, py: i32| {
                px >= 0 && py >= 0 && px < w && py < h && label[(py * w + px) as usize] == lab
            };
            out.push(trace_boundary(x, y, &inside));
        }
    }
    out
}

/// Walks the pixel-edge boundary of the component (square tracing on
/// vertices), counter-clockwise in image space → clockwise with y up.
fn trace_boundary(sx: i32, sy: i32, inside: &dyn Fn(i32, i32) -> bool) -> Vec<[f32; 2]> {
    // Vertices are pixel corners. Start at the top-left corner of (sx, sy)
    // heading east along its top edge (outside is up).
    // The component stays on the right-hand side of the walk.
    let mut pts = vec![];
    let (mut x, mut y) = (sx, sy);
    let mut dir = 0; // 0 east, 1 south, 2 west, 3 north (image coords)
    let limit = 4 * 1024 * 1024;
    for _ in 0..limit {
        // Pixels left and right of the edge leaving (x, y) along `dir`.
        let (l, r) = match dir {
            0 => ((x, y - 1), (x, y)),
            1 => ((x, y), (x - 1, y)),
            2 => ((x - 1, y), (x - 1, y - 1)),
            _ => ((x - 1, y - 1), (x, y - 1)),
        };
        let (li, ri) = (inside(l.0, l.1), inside(r.0, r.1));
        let next = if li {
            (dir + 3) % 4
        } else if ri {
            dir
        } else {
            (dir + 1) % 4
        };
        if !pts.is_empty() && (x, y) == (sx, sy) && next == 0 {
            break;
        }
        if next == dir || pts.is_empty() {
            // Straight on: only corners are kept.
            if pts.is_empty() {
                pts.push([x as f32, y as f32]);
            }
        } else {
            pts.push([x as f32, y as f32]);
        }
        dir = next;
        match dir {
            0 => x += 1,
            1 => y += 1,
            2 => x -= 1,
            _ => y -= 1,
        }
    }
    pts
}

/// Douglas–Peucker on a closed polygon.
pub fn simplify_polygon(pts: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if pts.len() < 4 {
        return pts.to_vec();
    }
    // Split at the point farthest from the first one.
    let far = (1..pts.len())
        .max_by(|&a, &b| {
            let da = (pts[a][0] - pts[0][0]).powi(2) + (pts[a][1] - pts[0][1]).powi(2);
            let db = (pts[b][0] - pts[0][0]).powi(2) + (pts[b][1] - pts[0][1]).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(1);
    let mut a = dp(&pts[..=far], eps);
    let mut tail: Vec<[f32; 2]> = pts[far..].to_vec();
    tail.push(pts[0]);
    let b = dp(&tail, eps);
    a.pop();
    a.extend_from_slice(&b[..b.len() - 1]);
    a
}

fn dp(pts: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    let mut best = (0f32, 0usize);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len;
        if d > best.0 {
            best = (d, i);
        }
    }
    if best.0 > eps {
        let mut l = dp(&pts[..=best.1], eps);
        let r = dp(&pts[best.1..], eps);
        l.pop();
        l.extend(r);
        l
    } else {
        vec![a, b]
    }
}

fn signed_area(p: &[[f32; 2]]) -> f32 {
    let mut s = 0.0;
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s * 0.5
}

/// Ear clipping of a simple polygon (counter-clockwise). Returns triangles
/// as indices into `p`.
pub fn triangulate(p: &[[f32; 2]]) -> Vec<u32> {
    let n = p.len();
    if n < 3 {
        return vec![];
    }
    let mut v: Vec<usize> = (0..n).collect();
    let mut out = vec![];
    let cross2 = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut guard = 0;
    while v.len() > 3 && guard < n * n {
        guard += 1;
        let m = v.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (v[(i + m - 1) % m], v[i], v[(i + 1) % m]);
            let (a, b, c) = (p[ia], p[ib], p[ic]);
            if cross2(a, b, c) <= 1e-9 {
                continue;
            }
            let ear = v.iter().all(|&j| {
                if j == ia || j == ib || j == ic {
                    return true;
                }
                let q = p[j];
                !(cross2(a, b, q) >= 0.0 && cross2(b, c, q) >= 0.0 && cross2(c, a, q) >= 0.0)
            });
            if ear {
                out.extend_from_slice(&[ia as u32, ib as u32, ic as u32]);
                v.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            // Self-touching outline: fan the rest rather than fail.
            for i in 1..v.len() - 1 {
                out.extend_from_slice(&[v[0] as u32, v[i] as u32, v[i + 1] as u32]);
            }
            return out;
        }
    }
    if v.len() == 3 {
        out.extend_from_slice(&[v[0] as u32, v[1] as u32, v[2] as u32]);
    }
    out
}

/// Cut-out: alpha contour simplified (Douglas–Peucker, `eps_px` on a 512 px
/// image), extruded [`PAPERCRAFT_DEPTH_M`], image on the front and back, the
/// border colour on the sides.
pub fn papercraft(rgba: image::RgbaImage, class: AssetClass, eps_px: f32) -> Result<Model, Error> {
    let (w, h) = rgba.dimensions();
    let min_px = ((w * h) as usize / 200).max(16);
    let outlines = contours(&rgba, min_px);
    if outlines.is_empty() {
        return Err(Error::Malformed(
            "the image has no opaque shape to cut out".into(),
        ));
    }
    let (sw, sh) = base_size(class, w as f32 / h.max(1) as f32);
    let (kx, ky) = (sw / w as f32, sh / h as f32);
    let d = PAPERCRAFT_DEPTH_M / 2.0;
    let mut prim = Prim::default();
    let mut nrm = vec![];
    let mut uv = vec![];
    for outline in outlines {
        let mut poly = simplify_polygon(&outline, eps_px);
        if poly.len() < 3 {
            continue;
        }
        // Image y grows down; flip to y up and make the polygon CCW.
        let mut world: Vec<[f32; 2]> = poly
            .iter()
            .map(|q| [q[0] * kx - sw / 2.0, sh - q[1] * ky])
            .collect();
        if signed_area(&world) < 0.0 {
            world.reverse();
            poly.reverse();
        }
        let uvs: Vec<[f32; 2]> = poly
            .iter()
            .map(|q| [q[0] / w as f32, q[1] / h as f32])
            .collect();
        let tris = triangulate(&world);
        let n = world.len() as u32;
        // Front (+z) and back (−z).
        for (z, nz) in [(d, 1.0f32), (-d, -1.0)] {
            let base = prim.pos.len() as u32;
            for (i, q) in world.iter().enumerate() {
                prim.pos.push([q[0], q[1], z]);
                nrm.push([0.0, 0.0, nz]);
                uv.push(uvs[i]);
            }
            for t in tris.chunks_exact(3) {
                if nz > 0.0 {
                    prim.idx
                        .extend_from_slice(&[base + t[0], base + t[1], base + t[2]]);
                } else {
                    prim.idx
                        .extend_from_slice(&[base + t[0], base + t[2], base + t[1]]);
                }
            }
        }
        // Sides.
        for i in 0..n as usize {
            let j = (i + 1) % n as usize;
            let (a, b) = (world[i], world[j]);
            let out = normalize3([b[1] - a[1], -(b[0] - a[0]), 0.0]);
            let base = prim.pos.len() as u32;
            for (q, z, t) in [
                (a, d, uvs[i]),
                (b, d, uvs[j]),
                (b, -d, uvs[j]),
                (a, -d, uvs[i]),
            ] {
                prim.pos.push([q[0], q[1], z]);
                nrm.push(out);
                uv.push(t);
            }
            prim.idx
                .extend_from_slice(&[base, base + 3, base + 2, base, base + 2, base + 1]);
        }
    }
    prim.nrm = Some(nrm);
    prim.uv = Some(uv);
    // Pull UVs a hair inside the shape so the edge texels are opaque.
    Ok(model_with(vec![prim], rgba, false))
}

/// Relief: a 128×128 grid masked by alpha, pushed forward by the depth map
/// (up to a quarter of the height), then simplified to `target_tris`.
pub fn relief_from_depth(
    rgba: image::RgbaImage,
    depth: &DepthMap,
    class: AssetClass,
    target_tris: usize,
) -> Result<Model, Error> {
    let (w, h) = rgba.dimensions();
    let (sw, sh) = base_size(class, w as f32 / h.max(1) as f32);
    const N: usize = 128;
    let mut pos = Vec::with_capacity(N * N);
    let mut uv = Vec::with_capacity(N * N);
    let mut solid = vec![false; N * N];
    for y in 0..N {
        for x in 0..N {
            let u = x as f32 / (N - 1) as f32;
            let v = y as f32 / (N - 1) as f32;
            let px = ((u * (w - 1) as f32) as u32).min(w - 1);
            let py = ((v * (h - 1) as f32) as u32).min(h - 1);
            solid[y * N + x] = rgba.get_pixel(px, py).0[3] >= 128;
            let z = depth.at_uv(u, v) * sh * 0.25;
            pos.push([u * sw - sw / 2.0, sh - v * sh, z]);
            uv.push([u, v]);
        }
    }
    let mut idx = vec![];
    for y in 0..N - 1 {
        for x in 0..N - 1 {
            let i = (y * N + x) as u32;
            let n = N as u32;
            if !(solid[i as usize]
                || solid[(i + 1) as usize]
                || solid[(i + n) as usize]
                || solid[(i + n + 1) as usize])
            {
                continue;
            }
            idx.extend_from_slice(&[i, i + n, i + 1, i + 1, i + n, i + n + 1]);
        }
    }
    if idx.is_empty() {
        return Err(Error::Malformed("the image has no opaque area".into()));
    }
    let mut prim = Prim {
        pos,
        uv: Some(uv),
        idx,
        ..Default::default()
    };
    crate::clean::compact(&mut prim);
    crate::clean::ensure_normals(&mut prim);
    if prim.tris() > target_tris {
        let (attrs, weights): (Vec<f32>, Vec<f32>) = (
            prim.uv
                .as_ref()
                .unwrap()
                .iter()
                .flatten()
                .copied()
                .collect(),
            vec![1.0, 1.0],
        );
        let (i2, _) = meshopt::simplify_with_attributes(
            &prim.idx,
            &prim.pos,
            &attrs,
            &weights,
            &[],
            target_tris * 3,
            0.01,
            meshopt::SIMPLIFY_LOCK_BORDER,
        );
        prim.idx = i2;
        crate::clean::compact(&mut prim);
    }
    Ok(model_with(vec![prim], rgba, true))
}

/// Relief through an estimator (runs the model, then [`relief_from_depth`]).
pub fn relief(
    rgba: image::RgbaImage,
    estimator: &dyn DepthEstimator,
    class: AssetClass,
    target_tris: usize,
) -> Result<Model, Error> {
    let depth = estimator.estimate(&rgba).map_err(Error::Provider)?;
    relief_from_depth(rgba, &depth, class, target_tris)
}
