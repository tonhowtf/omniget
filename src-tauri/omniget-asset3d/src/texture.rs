//! Passo 6: textures to 256/512 px WebP, and "fit to style": colours snapped
//! to the district palette (static assets lose their textures and are painted
//! per triangle with the nearest slot, through COLOR_0 and a `slot:<slot>`
//! material the client can recolour; characters keep their maps, posterized
//! to the palette).

use crate::model::*;
use crate::types::{DEFAULT_PALETTE, PALETTE_SLOTS};
use image::imageops::FilterType;
use std::collections::BTreeMap;

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// OKLab from linear sRGB.
pub fn oklab(c: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_46 * c[0] + 0.536_332_55 * c[1] + 0.051_445_995 * c[2];
    let m = 0.211_903_5 * c[0] + 0.680_699_5 * c[1] + 0.107_396_96 * c[2];
    let s = 0.088_302_46 * c[0] + 0.281_718_85 * c[1] + 0.629_978_7 * c[2];
    let (l, m, s) = (l.max(0.0).cbrt(), m.max(0.0).cbrt(), s.max(0.0).cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn parse_hex(h: &str) -> Option<[f32; 3]> {
    let h = h.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([
        ((v >> 16) & 255) as f32 / 255.0,
        ((v >> 8) & 255) as f32 / 255.0,
        (v & 255) as f32 / 255.0,
    ])
}

/// Palette in slot order: `(slot, linear rgb, oklab)`.
pub struct Palette {
    pub entries: Vec<(String, [f32; 3], [f32; 3])>,
}

impl Palette {
    pub fn new(custom: Option<&BTreeMap<String, String>>) -> Self {
        let mut entries = vec![];
        for slot in PALETTE_SLOTS {
            let hex = custom
                .and_then(|c| c.get(*slot))
                .map(String::as_str)
                .or_else(|| {
                    DEFAULT_PALETTE
                        .iter()
                        .find(|(s, _)| s == slot)
                        .map(|(_, h)| *h)
                })
                .unwrap_or("#808080");
            let srgb = parse_hex(hex).unwrap_or([0.5; 3]);
            let lin = [
                srgb_to_linear(srgb[0]),
                srgb_to_linear(srgb[1]),
                srgb_to_linear(srgb[2]),
            ];
            entries.push((slot.to_string(), lin, oklab(lin)));
        }
        Palette { entries }
    }

    pub fn nearest(&self, linear: [f32; 3]) -> usize {
        let q = oklab(linear);
        let mut best = (f32::INFINITY, 0);
        for (i, (_, _, lab)) in self.entries.iter().enumerate() {
            let d = sub(q, *lab);
            let d2 = dot(d, d);
            if d2 < best.0 {
                best = (d2, i);
            }
        }
        best.1
    }
}

/// Linear RGBA of a material at `uv` (nearest texel, repeat wrap).
pub fn sample_material(model: &Model, mat: Option<usize>, uv: Option<[f32; 2]>) -> [f32; 4] {
    let Some(m) = mat.and_then(|m| model.materials.get(m)) else {
        return [0.8, 0.8, 0.8, 1.0];
    };
    let mut c = m.base_color;
    let img = m
        .base_tex
        .and_then(|t| model.textures.get(t))
        .and_then(|t| t.image)
        .and_then(|i| model.images.get(i));
    if let (Some(img), Some(uv)) = (img, uv) {
        let t = sample(&img.rgba, uv);
        for k in 0..3 {
            c[k] *= srgb_to_linear(t[k]);
        }
        c[3] *= t[3];
    }
    c
}

pub fn sample(img: &image::RgbaImage, uv: [f32; 2]) -> [f32; 4] {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let fu = uv[0] - uv[0].floor();
    let fv = uv[1] - uv[1].floor();
    let x = ((fu * w as f32) as u32).min(w - 1);
    let y = ((fv * h as f32) as u32).min(h - 1);
    let p = img.get_pixel(x, y).0;
    [
        p[0] as f32 / 255.0,
        p[1] as f32 / 255.0,
        p[2] as f32 / 255.0,
        p[3] as f32 / 255.0,
    ]
}

/// Resizes to at most `px` on the longer side and encodes lossy WebP.
pub fn encode_images(model: &mut Model, px: u32, quality: f32) {
    for img in &mut model.images {
        let (w, h) = img.rgba.dimensions();
        let long = w.max(h).max(1);
        if long > px {
            let nw = ((w as u64 * px as u64) / long as u64).max(1) as u32;
            let nh = ((h as u64 * px as u64) / long as u64).max(1) as u32;
            img.rgba = image::imageops::resize(&img.rgba, nw, nh, FilterType::Triangle);
        }
        img.encoded = Some(encode_webp(&img.rgba, quality));
        img.mime = "image/webp".into();
    }
}

pub fn encode_webp(rgba: &image::RgbaImage, quality: f32) -> Vec<u8> {
    let (w, h) = rgba.dimensions();
    let opaque = rgba.pixels().all(|p| p.0[3] == 255);
    if opaque {
        let rgb: Vec<u8> = rgba
            .pixels()
            .flat_map(|p| [p.0[0], p.0[1], p.0[2]])
            .collect();
        webp::Encoder::from_rgb(&rgb, w, h).encode(quality).to_vec()
    } else {
        webp::Encoder::from_rgba(rgba.as_raw(), w, h)
            .encode(quality)
            .to_vec()
    }
}

/// Static assets: one primitive per palette slot, COLOR_0 = slot colour,
/// material `slot:<slot>`; textures are dropped.
pub fn fit_style_static(model: &mut Model, palette: &Palette) {
    let mut per_slot: Vec<Prim> = (0..palette.entries.len())
        .map(|_| Prim::default())
        .collect();
    let mut remaps: Vec<std::collections::HashMap<(usize, usize, u32), u32>> =
        (0..palette.entries.len())
            .map(|_| Default::default())
            .collect();
    for (mi, mesh) in model.meshes.iter().enumerate() {
        for (pi, p) in mesh.prims.iter().enumerate() {
            let vcol: Vec<[f32; 4]> = (0..p.pos.len())
                .map(|v| crate::budget::vertex_color(model, p, v))
                .collect();
            for t in p.idx.chunks_exact(3) {
                // Average of the three corners and the centroid sample.
                let mut c = [0f32; 3];
                for &v in t {
                    for k in 0..3 {
                        c[k] += vcol[v as usize][k] / 4.0;
                    }
                }
                if let Some(uv) = &p.uv {
                    let (a, b, d) = (uv[t[0] as usize], uv[t[1] as usize], uv[t[2] as usize]);
                    let cuv = [(a[0] + b[0] + d[0]) / 3.0, (a[1] + b[1] + d[1]) / 3.0];
                    let s = sample_material(model, p.material, Some(cuv));
                    for k in 0..3 {
                        c[k] += s[k] / 4.0;
                    }
                } else {
                    for k in 0..3 {
                        c[k] += vcol[t[0] as usize][k] / 4.0;
                    }
                }
                let slot = palette.nearest(c);
                let dst = &mut per_slot[slot];
                let map = &mut remaps[slot];
                for &v in t {
                    let key = (mi, pi, v);
                    let ni = *map.entry(key).or_insert_with(|| {
                        dst.pos.push(p.pos[v as usize]);
                        if let Some(n) = &p.nrm {
                            dst.nrm.get_or_insert_with(Vec::new).push(n[v as usize]);
                        }
                        (dst.pos.len() - 1) as u32
                    });
                    dst.idx.push(ni);
                }
            }
        }
    }
    let mut materials = vec![];
    let mut prims = vec![];
    for (slot, mut p) in per_slot.into_iter().enumerate() {
        if p.idx.is_empty() {
            continue;
        }
        let (name, lin, _) = &palette.entries[slot];
        if p.nrm.as_ref().map(|n| n.len()) != Some(p.pos.len()) {
            p.nrm = None;
            crate::clean::ensure_normals(&mut p);
        }
        p.col = Some(vec![[lin[0], lin[1], lin[2], 1.0]; p.pos.len()]);
        materials.push(Material {
            name: Some(format!("slot:{name}")),
            base_color: [1.0; 4],
            ..Default::default()
        });
        p.material = Some(materials.len() - 1);
        crate::clean::compact(&mut p);
        prims.push(p);
    }
    model.meshes = vec![Mesh {
        name: Some("asset".into()),
        prims,
        weights: None,
        extras: None,
    }];
    model.materials = materials;
    model.textures.clear();
    model.images.clear();
    for n in &mut model.nodes {
        n.mesh = n.mesh.map(|_| 0);
    }
}

/// Characters: posterize every texture to the palette (keeps UV and skin).
pub fn fit_style_textures(model: &mut Model, palette: &Palette) {
    let srgb: Vec<[u8; 3]> = palette
        .entries
        .iter()
        .map(|(_, l, _)| {
            [
                (linear_to_srgb(l[0]) * 255.0).round() as u8,
                (linear_to_srgb(l[1]) * 255.0).round() as u8,
                (linear_to_srgb(l[2]) * 255.0).round() as u8,
            ]
        })
        .collect();
    for img in &mut model.images {
        let mut cache: std::collections::HashMap<[u8; 3], usize> = Default::default();
        for p in img.rgba.pixels_mut() {
            let key = [p.0[0] & 0xF8, p.0[1] & 0xF8, p.0[2] & 0xF8];
            let i = *cache.entry(key).or_insert_with(|| {
                let lin = [
                    srgb_to_linear(key[0] as f32 / 255.0),
                    srgb_to_linear(key[1] as f32 / 255.0),
                    srgb_to_linear(key[2] as f32 / 255.0),
                ];
                palette.nearest(lin)
            });
            p.0[0] = srgb[i][0];
            p.0[1] = srgb[i][1];
            p.0[2] = srgb[i][2];
        }
    }
}
