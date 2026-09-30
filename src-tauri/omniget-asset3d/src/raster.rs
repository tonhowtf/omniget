//! Tiny software renderer: the asset's thumbnail and the four views the
//! vision LLM gets (front, side, top, 3/4). Orthographic, flat shaded, one
//! colour per triangle.

use crate::model::*;
use crate::texture::{linear_to_srgb, sample_material};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// From +z looking north.
    Front,
    /// From +x looking west.
    Side,
    Top,
    ThreeQuarter,
}

pub struct ColoredTri {
    pub p: [[f32; 3]; 3],
    pub color: [f32; 4],
}

pub fn colored_triangles(model: &Model) -> Vec<ColoredTri> {
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
                let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                let uv = p.uv.as_ref().map(|u| {
                    [
                        (u[a][0] + u[b][0] + u[c][0]) / 3.0,
                        (u[a][1] + u[b][1] + u[c][1]) / 3.0,
                    ]
                });
                let mut color = sample_material(model, p.material, uv);
                if let Some(col) = &p.col {
                    for k in 0..4 {
                        color[k] *= (col[a][k] + col[b][k] + col[c][k]) / 3.0;
                    }
                }
                out.push(ColoredTri {
                    p: [wp[a], wp[b], wp[c]],
                    color,
                });
            }
        }
    }
    out
}

fn view_basis(v: View) -> ([f32; 3], [f32; 3], [f32; 3]) {
    // (right, up, toward the viewer)
    match v {
        View::Front => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        View::Side => ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
        View::Top => ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        View::ThreeQuarter => {
            let fwd = normalize3([0.6124, 0.5, 0.6124]);
            let right = normalize3(cross([0.0, 1.0, 0.0], fwd));
            let up = cross(fwd, right);
            (right, up, fwd)
        }
    }
}

/// Renders to an RGBA image; `background` alpha 0 gives a transparent PNG.
pub fn render(tris: &[ColoredTri], view: View, px: u32, background: [u8; 4]) -> image::RgbaImage {
    let px = px.clamp(16, 2048);
    let (r, u, f) = view_basis(view);
    let proj: Vec<[[f32; 3]; 3]> = tris
        .iter()
        .map(|t| t.p.map(|p| [dot(p, r), dot(p, u), dot(p, f)]))
        .collect();
    let mut bb = Aabb::empty();
    for t in &proj {
        for p in t {
            bb.add(*p);
        }
    }
    let mut img = image::RgbaImage::from_pixel(px, px, image::Rgba(background));
    if !bb.is_valid() {
        return img;
    }
    let s = bb.size();
    let extent = s[0].max(s[1]).max(1e-6) * 1.08;
    let scale = px as f32 / extent;
    let cx = (bb.min[0] + bb.max[0]) * 0.5;
    let cy = (bb.min[1] + bb.max[1]) * 0.5;
    let to_screen = |p: [f32; 3]| -> [f32; 3] {
        [
            (p[0] - cx) * scale + px as f32 * 0.5,
            px as f32 * 0.5 - (p[1] - cy) * scale,
            p[2],
        ]
    };
    let mut depth = vec![f32::NEG_INFINITY; (px * px) as usize];
    let light = normalize3([0.4, 0.8, 0.45]);
    for (t, src) in proj.iter().zip(tris) {
        let n = normalize3(cross(sub(src.p[1], src.p[0]), sub(src.p[2], src.p[0])));
        let lambert = dot(n, light).abs() * 0.7 + 0.3;
        let c = src.color;
        let rgb = [
            (linear_to_srgb(c[0] * lambert) * 255.0) as u8,
            (linear_to_srgb(c[1] * lambert) * 255.0) as u8,
            (linear_to_srgb(c[2] * lambert) * 255.0) as u8,
        ];
        if c[3] < 0.1 {
            continue;
        }
        let a = to_screen(t[0]);
        let b = to_screen(t[1]);
        let d = to_screen(t[2]);
        let minx = a[0].min(b[0]).min(d[0]).floor().max(0.0) as i32;
        let maxx = a[0].max(b[0]).max(d[0]).ceil().min(px as f32 - 1.0) as i32;
        let miny = a[1].min(b[1]).min(d[1]).floor().max(0.0) as i32;
        let maxy = a[1].max(b[1]).max(d[1]).ceil().min(px as f32 - 1.0) as i32;
        let area = (b[0] - a[0]) * (d[1] - a[1]) - (b[1] - a[1]) * (d[0] - a[0]);
        if area.abs() < 1e-9 {
            continue;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b[0] - fx) * (d[1] - fy) - (b[1] - fy) * (d[0] - fx)) / area;
                let w1 = ((d[0] - fx) * (a[1] - fy) - (d[1] - fy) * (a[0] - fx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * a[2] + w1 * b[2] + w2 * d[2];
                let i = (y as u32 * px + x as u32) as usize;
                if z > depth[i] {
                    depth[i] = z;
                    img.put_pixel(
                        x as u32,
                        y as u32,
                        image::Rgba([rgb[0], rgb[1], rgb[2], 255]),
                    );
                }
            }
        }
    }
    img
}

pub fn png(img: &image::RgbaImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut out, image::ImageFormat::Png);
    out.into_inner()
}

pub fn thumbnail(model: &Model, px: u32) -> Vec<u8> {
    png(&render(
        &colored_triangles(model),
        View::ThreeQuarter,
        px,
        [0, 0, 0, 0],
    ))
}

/// The four PNG views for [`crate::anchors::anchor_prompt`].
pub fn four_views(model: &Model, px: u32) -> Vec<Vec<u8>> {
    let tris = colored_triangles(model);
    [View::Front, View::Side, View::Top, View::ThreeQuarter]
        .iter()
        .map(|v| png(&render(&tris, *v, px, [235, 235, 235, 255])))
        .collect()
}
