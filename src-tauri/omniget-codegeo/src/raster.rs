//! A tiny software rasteriser for previews and visual QA: orthographic view
//! from the south-east, flat Lambert shading with the palette colour of each
//! slot, a depth outline, 2× supersampling. No GPU, no dependencies beyond
//! the PNG encoder.

use crate::canon::{CanonMesh, Q};
use crate::palette::{rgb, slot_name};
use std::collections::BTreeMap;

pub struct View {
    /// Azimuth of the camera, degrees clockwise from north (135 = south-east).
    pub yaw_deg: f64,
    /// Elevation, degrees above the horizon.
    pub pitch_deg: f64,
}

impl Default for View {
    fn default() -> Self {
        View {
            yaw_deg: 135.0,
            pitch_deg: 32.0,
        }
    }
}

fn norm(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Renders to RGBA8, `w × h`.
pub fn render_rgba(
    mesh: &CanonMesh,
    palette: &[(&str, &str)],
    w: u32,
    h: u32,
    view: &View,
) -> Vec<u8> {
    let ss = 2usize;
    let (sw, sh) = (w as usize * ss, h as usize * ss);
    let colours: BTreeMap<&str, [u8; 3]> = palette.iter().map(|(k, v)| (*k, rgb(v))).collect();
    // Camera basis: `to_cam` points from the scene to the camera.
    let (ya, pa) = (view.yaw_deg.to_radians(), view.pitch_deg.to_radians());
    let to_cam = norm([ya.sin() * pa.cos(), pa.sin(), -ya.cos() * pa.cos()]);
    let right = norm([to_cam[2], 0.0, -to_cam[0]]);
    let up = norm([
        to_cam[1] * right[2] - to_cam[2] * right[1],
        to_cam[2] * right[0] - to_cam[0] * right[2],
        to_cam[0] * right[1] - to_cam[1] * right[0],
    ]);
    let up = if up[1] < 0.0 {
        [-up[0], -up[1], -up[2]]
    } else {
        up
    };
    let light = norm([0.45, 0.8, 0.35]);

    let proj = |p: [i32; 3]| -> [f64; 3] {
        let v = [p[0] as f64 / Q, p[1] as f64 / Q, p[2] as f64 / Q];
        [dot(v, right), dot(v, up), dot(v, to_cam)]
    };
    let mut lo = [f64::MAX; 2];
    let mut hi = [f64::MIN; 2];
    for t in &mesh.tris {
        for v in &t.v {
            let p = proj(*v);
            lo[0] = lo[0].min(p[0]);
            lo[1] = lo[1].min(p[1]);
            hi[0] = hi[0].max(p[0]);
            hi[1] = hi[1].max(p[1]);
        }
    }
    let bg = [236u8, 232, 222];
    let mut col = vec![0u8; sw * sh * 3];
    for px in col.chunks_mut(3) {
        px.copy_from_slice(&bg);
    }
    let mut depth = vec![f64::MIN; sw * sh];
    if !mesh.tris.is_empty() {
        let margin = 0.06;
        let span =
            ((hi[0] - lo[0]) / sw as f64).max((hi[1] - lo[1]) / sh as f64) * (1.0 + 2.0 * margin);
        let span = if span <= 0.0 { 1.0 } else { span };
        let cx = (lo[0] + hi[0]) / 2.0;
        let cy = (lo[1] + hi[1]) / 2.0;
        let to_px = |p: [f64; 3]| -> [f64; 3] {
            [
                (p[0] - cx) / span + sw as f64 / 2.0,
                sh as f64 / 2.0 - (p[1] - cy) / span,
                p[2],
            ]
        };
        for t in &mesh.tris {
            let n = t.normal();
            if dot(n, to_cam) <= 0.0 {
                continue;
            }
            let base = colours
                .get(slot_name(t.slot))
                .copied()
                .unwrap_or([128, 128, 128]);
            let lambert = dot(n, light).max(0.0);
            let shade = 0.5 + 0.5 * lambert + 0.08 * n[1].max(0.0);
            let c = [
                (base[0] as f64 * shade).min(255.0) as u8,
                (base[1] as f64 * shade).min(255.0) as u8,
                (base[2] as f64 * shade).min(255.0) as u8,
            ];
            let p: Vec<[f64; 3]> = t.v.iter().map(|v| to_px(proj(*v))).collect();
            let minx = p
                .iter()
                .map(|q| q[0])
                .fold(f64::MAX, f64::min)
                .floor()
                .max(0.0) as usize;
            let maxx = p
                .iter()
                .map(|q| q[0])
                .fold(f64::MIN, f64::max)
                .ceil()
                .min(sw as f64 - 1.0) as usize;
            let miny = p
                .iter()
                .map(|q| q[1])
                .fold(f64::MAX, f64::min)
                .floor()
                .max(0.0) as usize;
            let maxy = p
                .iter()
                .map(|q| q[1])
                .fold(f64::MIN, f64::max)
                .ceil()
                .min(sh as f64 - 1.0) as usize;
            let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            if area.abs() < 1e-12 {
                continue;
            }
            for y in miny..=maxy {
                for x in minx..=maxx {
                    let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                    let w0 =
                        ((p[1][0] - fx) * (p[2][1] - fy) - (p[2][0] - fx) * (p[1][1] - fy)) / area;
                    let w1 =
                        ((p[2][0] - fx) * (p[0][1] - fy) - (p[0][0] - fx) * (p[2][1] - fy)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                        continue;
                    }
                    let z = w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2];
                    let i = y * sw + x;
                    if z > depth[i] {
                        depth[i] = z;
                        col[i * 3..i * 3 + 3].copy_from_slice(&c);
                    }
                }
            }
        }
        // Outline where depth jumps (silhouettes and overhangs).
        let jump = 0.25;
        let mut out = col.clone();
        for y in 1..sh - 1 {
            for x in 1..sw - 1 {
                let i = y * sw + x;
                let d = depth[i];
                let mut edge = false;
                for j in [i - 1, i + 1, i - sw, i + sw] {
                    let e = depth[j];
                    if (d == f64::MIN) != (e == f64::MIN)
                        || (d != f64::MIN && e != f64::MIN && (d - e) > jump)
                    {
                        edge = d != f64::MIN || e != f64::MIN;
                    }
                }
                if edge && d != f64::MIN {
                    for k in 0..3 {
                        out[i * 3 + k] = (out[i * 3 + k] as f64 * 0.45) as u8;
                    }
                }
            }
        }
        col = out;
    }
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut acc = [0u32; 3];
            for dy in 0..ss {
                for dx in 0..ss {
                    let i = ((y * ss + dy) * sw + x * ss + dx) * 3;
                    for k in 0..3 {
                        acc[k] += col[i + k] as u32;
                    }
                }
            }
            let o = (y * w as usize + x) * 4;
            for k in 0..3 {
                rgba[o + k] = (acc[k] / (ss * ss) as u32) as u8;
            }
            rgba[o + 3] = 255;
        }
    }
    rgba
}

pub fn png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().expect("png header");
        wr.write_image_data(rgba).expect("png data");
    }
    out
}

pub fn render_png(
    mesh: &CanonMesh,
    palette: &[(&str, &str)],
    w: u32,
    h: u32,
    view: &View,
) -> Vec<u8> {
    png(&render_rgba(mesh, palette, w, h, view), w, h)
}

/// Lays modules out in a grid (`spacing_m` apart) into one mesh, in the
/// order given, for a contact sheet.
pub fn sheet(meshes: &[CanonMesh], cols: usize, spacing_m: f64) -> CanonMesh {
    let step = (spacing_m * Q) as i32;
    let mut tris = vec![];
    for (i, m) in meshes.iter().enumerate() {
        let (c, r) = ((i % cols) as i32, (i / cols) as i32);
        tris.extend(m.placed(0, c * step, 0, r * step).tris);
    }
    tris.sort();
    CanonMesh { tris }
}
