use std::collections::VecDeque;

use image::imageops::{self, FilterType};
use image::{GrayImage, Luma, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

pub const EDGE_LIMIT: f32 = 20.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    #[serde(default)]
    pub feather: f32,
    #[serde(default)]
    pub shift: f32,
}

#[derive(Debug, Clone)]
pub enum Fill {
    Transparent,
    Color([u8; 3]),
    Image(RgbaImage),
}

pub fn edge_pixels(per_mille: f32, w: u32, h: u32) -> f32 {
    if !per_mille.is_finite() {
        return 0.0;
    }
    per_mille.clamp(-EDGE_LIMIT, EDGE_LIMIT) * w.max(h) as f32 / 1000.0
}

pub fn from_values(values: &[f32], w: u32, h: u32) -> Option<GrayImage> {
    let n = (w as usize).checked_mul(h as usize)?;
    let values = values.get(..n)?;
    let (lo, hi) = values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    let span = hi - lo;
    let data = values
        .iter()
        .map(|v| {
            if !v.is_finite() || span <= f32::EPSILON {
                0
            } else {
                (((v - lo) / span) * 255.0).round().clamp(0.0, 255.0) as u8
            }
        })
        .collect();
    GrayImage::from_raw(w, h, data)
}

pub fn fit(mask: &GrayImage, w: u32, h: u32) -> GrayImage {
    if mask.dimensions() == (w, h) {
        return mask.clone();
    }
    imageops::resize(mask, w, h, FilterType::Triangle)
}

#[derive(Clone, Copy)]
enum Op {
    Grow,
    Shrink,
}

impl Op {
    fn wins(self, a: u8, b: u8) -> bool {
        match self {
            Op::Grow => a >= b,
            Op::Shrink => a <= b,
        }
    }
}

fn sweep(src: &[u8], dst: &mut [u8], r: usize, op: Op, queue: &mut VecDeque<usize>) {
    queue.clear();
    let n = src.len();
    for i in 0..n + r {
        if i < n {
            while queue.back().is_some_and(|&b| op.wins(src[i], src[b])) {
                queue.pop_back();
            }
            queue.push_back(i);
        }
        if i < r {
            continue;
        }
        let j = i - r;
        while queue.front().is_some_and(|&f| f + r < j) {
            queue.pop_front();
        }
        if let (Some(&f), Some(d)) = (queue.front(), dst.get_mut(j)) {
            *d = src[f];
        }
    }
}

fn morph(mask: &GrayImage, r: u32, op: Op) -> GrayImage {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let r = r as usize;
    let mut rows = vec![0u8; w * h];
    let mut queue = VecDeque::new();
    let src = mask.as_raw();
    for y in 0..h {
        let line = y * w..(y + 1) * w;
        sweep(&src[line.clone()], &mut rows[line], r, op, &mut queue);
    }
    let mut out = vec![0u8; w * h];
    let mut column = vec![0u8; h];
    let mut result = vec![0u8; h];
    for x in 0..w {
        for (y, c) in column.iter_mut().enumerate() {
            *c = rows[y * w + x];
        }
        sweep(&column, &mut result, r, op, &mut queue);
        for (y, v) in result.iter().enumerate() {
            out[y * w + x] = *v;
        }
    }
    GrayImage::from_raw(w as u32, h as u32, out).unwrap_or_else(|| mask.clone())
}

pub fn refine(mask: &GrayImage, edge: Edge) -> GrayImage {
    let (w, h) = mask.dimensions();
    let shift = edge_pixels(edge.shift, w, h).round() as i64;
    let mut out = match shift {
        s if s > 0 => morph(mask, s as u32, Op::Grow),
        s if s < 0 => morph(mask, s.unsigned_abs() as u32, Op::Shrink),
        _ => mask.clone(),
    };
    let sigma = edge_pixels(edge.feather.max(0.0), w, h);
    if sigma >= 0.5 {
        out = imageops::fast_blur(&out, sigma);
    }
    out
}

pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    let digits: Vec<u8> = match hex.len() {
        3 => hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| (d * 17) as u8))
            .collect::<Option<_>>()?,
        6 => (0..3)
            .map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok())
            .collect::<Option<_>>()?,
        _ => return None,
    };
    Some([digits[0], digits[1], digits[2]])
}

pub fn cover(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    let dynamic = image::DynamicImage::ImageRgba8(img.clone());
    dynamic
        .resize_to_fill(w, h, FilterType::Triangle)
        .to_rgba8()
}

fn over(fg: [u8; 4], coverage: u8, bg: [u8; 4]) -> [u8; 4] {
    let a = coverage as f32 / 255.0 * fg[3] as f32 / 255.0;
    let ba = bg[3] as f32 / 255.0;
    let out_a = a + ba * (1.0 - a);
    if out_a <= 0.0 {
        return [0, 0, 0, 0];
    }
    let mix = |c: u8, b: u8| {
        ((c as f32 * a + b as f32 * ba * (1.0 - a)) / out_a)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [
        mix(fg[0], bg[0]),
        mix(fg[1], bg[1]),
        mix(fg[2], bg[2]),
        (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

pub fn compose(src: &RgbaImage, mask: &GrayImage, fill: &Fill) -> RgbaImage {
    let (w, h) = src.dimensions();
    let mask = fit(mask, w, h);
    let backdrop = match fill {
        Fill::Image(img) => Some(cover(img, w, h)),
        _ => None,
    };
    let mut out = RgbaImage::new(w, h);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let fg = src.get_pixel(x, y).0;
        let Luma([m]) = *mask.get_pixel(x, y);
        let value = match (fill, backdrop.as_ref()) {
            (Fill::Color([r, g, b]), _) => over(fg, m, [*r, *g, *b, 255]),
            (Fill::Image(_), Some(bg)) => over(fg, m, bg.get_pixel(x, y).0),
            _ => {
                let a = (m as u32 * fg[3] as u32 + 127) / 255;
                [fg[0], fg[1], fg[2], a as u8]
            }
        };
        *px = Rgba(value);
    }
    out
}

pub fn render(src: &RgbaImage, raw: &GrayImage, edge: Edge, fill: &Fill) -> RgbaImage {
    let (w, h) = src.dimensions();
    let mask = refine(&fit(raw, w, h), edge);
    compose(src, &mask, fill)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> GrayImage {
        GrayImage::from_fn(w, h, |x, y| {
            Luma([if x >= x0 && x < x1 && y >= y0 && y < y1 {
                255
            } else {
                0
            }])
        })
    }

    fn area(mask: &GrayImage) -> usize {
        mask.pixels().filter(|p| p.0[0] >= 128).count()
    }

    #[test]
    fn values_are_stretched_to_the_full_range() {
        let m = from_values(&[0.25, 0.5, 0.75, 0.25], 2, 2).unwrap();
        assert_eq!(m.as_raw(), &vec![0, 128, 255, 0]);
        let flat = from_values(&[0.5; 4], 2, 2).unwrap();
        assert!(flat.pixels().all(|p| p.0[0] == 0));
        assert!(from_values(&[0.0; 3], 2, 2).is_none());
    }

    #[test]
    fn grow_and_shrink_move_the_edge_by_the_radius() {
        let m = square(1000, 1000, 400, 400, 600, 600);
        let grown = refine(
            &m,
            Edge {
                feather: 0.0,
                shift: 10.0,
            },
        );
        assert_eq!(area(&grown), 220 * 220);
        let shrunk = refine(
            &m,
            Edge {
                feather: 0.0,
                shift: -10.0,
            },
        );
        assert_eq!(area(&shrunk), 180 * 180);
        assert_eq!(grown.get_pixel(390, 390).0[0], 255);
        assert_eq!(shrunk.get_pixel(405, 405).0[0], 0);
    }

    #[test]
    fn edge_scales_with_the_picture() {
        let small = square(100, 50, 40, 10, 60, 40);
        let grown = refine(
            &small,
            Edge {
                feather: 0.0,
                shift: 20.0,
            },
        );
        assert_eq!(area(&grown), 24 * 34);
        assert_eq!(edge_pixels(f32::NAN, 100, 100), 0.0);
        assert_eq!(edge_pixels(500.0, 1000, 10), 20.0);
    }

    #[test]
    fn feather_softens_without_moving_the_middle() {
        let m = square(400, 400, 100, 100, 300, 300);
        let soft = refine(
            &m,
            Edge {
                feather: 10.0,
                shift: 0.0,
            },
        );
        assert_eq!(soft.get_pixel(200, 200).0[0], 255);
        assert_eq!(soft.get_pixel(10, 10).0[0], 0);
        let rim = soft.get_pixel(100, 200).0[0];
        assert!(rim > 60 && rim < 200, "rim was {rim}");
    }

    #[test]
    fn colors_parse_in_short_and_long_form() {
        assert_eq!(parse_color("#ff8000"), Some([255, 128, 0]));
        assert_eq!(parse_color("0f0"), Some([0, 255, 0]));
        assert_eq!(parse_color(" #FFF "), Some([255, 255, 255]));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#zzzzzz"), None);
    }

    #[test]
    fn compose_cuts_out_and_fills() {
        let src = RgbaImage::from_pixel(4, 4, Rgba([200, 10, 10, 255]));
        let mask = square(4, 4, 0, 0, 2, 4);
        let cut = compose(&src, &mask, &Fill::Transparent);
        assert_eq!(cut.get_pixel(0, 0).0, [200, 10, 10, 255]);
        assert_eq!(cut.get_pixel(3, 0).0[3], 0);

        let blue = compose(&src, &mask, &Fill::Color([0, 0, 255]));
        assert_eq!(blue.get_pixel(0, 0).0, [200, 10, 10, 255]);
        assert_eq!(blue.get_pixel(3, 3).0, [0, 0, 255, 255]);

        let backdrop = RgbaImage::from_pixel(2, 2, Rgba([0, 255, 0, 255]));
        let swapped = compose(&src, &mask, &Fill::Image(backdrop));
        assert_eq!(swapped.get_pixel(3, 3).0, [0, 255, 0, 255]);

        let half = GrayImage::from_pixel(4, 4, Luma([128]));
        let mixed = compose(&src, &half, &Fill::Color([0, 0, 0]));
        assert_eq!(mixed.get_pixel(1, 1).0, [100, 5, 5, 255]);
    }

    #[test]
    fn render_fits_a_low_res_mask_to_the_picture() {
        let src = RgbaImage::from_pixel(64, 32, Rgba([1, 2, 3, 255]));
        let raw = square(8, 8, 0, 0, 4, 8);
        let out = render(&src, &raw, Edge::default(), &Fill::Transparent);
        assert_eq!(out.dimensions(), (64, 32));
        assert_eq!(out.get_pixel(2, 16).0[3], 255);
        assert_eq!(out.get_pixel(60, 16).0[3], 0);
    }
}
