//! The status item image: Loop's head inside a progress ring, drawn in RGBA
//! at 36×36 px (18 pt at 2×, the menu bar convention).
//!
//! Like Claude-Usage-Tracker's concentric icon
//! (github.com/hamed-elfayome/Claude-Usage-Tracker,
//! `Claude Usage/MenuBar/MenuBarIconRenderer.swift` `createConcentricIcon`,
//! MIT) the ring starts at 12 o'clock and fills clockwise over a faint track,
//! and like CodexBar (`Sources/CodexBar/IconRenderer.swift`, MIT) the image
//! stays a monochrome template while nothing is wrong, so macOS tints it for
//! the light and dark menu bar. Past the warning threshold the image stops
//! being a template and the ring turns orange, then red. Reimplemented here.
//!
//! Loop's glyph comes from `icons/usage-tray/` (generated with Higgsfield from
//! `static/loop.png`): `loop-mask-20.bin` is a 20×20 alpha mask and
//! `loop-color-20.rgba` the same head in his colours.

pub const SIZE: u32 = 36;
const GLYPH: u32 = 20;
const MASK: &[u8] = include_bytes!("../../icons/usage-tray/loop-mask-20.bin");
const COLOR: &[u8] = include_bytes!("../../icons/usage-tray/loop-color-20.rgba");

const ORANGE: [u8; 3] = [255, 149, 0];
const RED: [u8; 3] = [255, 59, 48];
/// Loop's light green (the X on his hood).
const GREEN: [u8; 3] = [76, 175, 80];
const GREY: [u8; 3] = [142, 142, 147];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// 0..1, `None` = no reading yet: only the track is drawn.
    pub used: Option<f64>,
    /// `ok`, `warn`, `critical`.
    pub level: &'static str,
    pub colored: bool,
}

/// The pixels and whether macOS should treat them as a template.
pub fn render(look: Look) -> (Vec<u8>, bool) {
    let alert = match look.level {
        "critical" => Some(RED),
        "warn" => Some(ORANGE),
        _ => None,
    };
    let template = alert.is_none() && !look.colored;
    let (fill, track): ([u8; 3], ([u8; 3], f32)) = match (alert, template) {
        (Some(c), _) => (c, (GREY, 0.45)),
        (None, true) => ([0, 0, 0], ([0, 0, 0], 0.28)),
        (None, false) => (GREEN, (GREY, 0.45)),
    };
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];

    // Ring: centre 18,18, outer radius 17, 3.5 px thick, 4×4 supersampled.
    let (c, r_out, r_in) = (SIZE as f32 / 2.0, 17.0f32, 13.5f32);
    let sweep = look
        .used
        .map(|u| u.clamp(0.0, 1.0) as f32 * std::f32::consts::TAU);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (mut on_track, mut on_fill) = (0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let fx = x as f32 + (sx as f32 + 0.5) / 4.0 - c;
                    let fy = y as f32 + (sy as f32 + 0.5) / 4.0 - c;
                    let d = (fx * fx + fy * fy).sqrt();
                    if d < r_in || d > r_out {
                        continue;
                    }
                    // Clockwise from 12 o'clock, y pointing down.
                    let mut a = fx.atan2(-fy);
                    if a < 0.0 {
                        a += std::f32::consts::TAU;
                    }
                    if sweep.map(|s| a <= s).unwrap_or(false) {
                        on_fill += 1;
                    } else {
                        on_track += 1;
                    }
                }
            }
            let fill_a = on_fill as f32 / 16.0;
            let track_a = on_track as f32 / 16.0 * track.1;
            blend(&mut px, x, y, fill, fill_a);
            blend(&mut px, x, y, track.0, track_a);
        }
    }

    // Loop's head in the middle.
    let off = (SIZE - GLYPH) / 2;
    for gy in 0..GLYPH {
        for gx in 0..GLYPH {
            let i = (gy * GLYPH + gx) as usize;
            let (rgb, a) = if look.colored {
                let p = &COLOR[i * 4..i * 4 + 4];
                ([p[0], p[1], p[2]], p[3] as f32 / 255.0)
            } else {
                (alert.unwrap_or([0, 0, 0]), MASK[i] as f32 / 255.0)
            };
            blend(&mut px, gx + off, gy + off, rgb, a);
        }
    }
    (px, template)
}

/// Source-over onto straight (non-premultiplied) RGBA.
fn blend(px: &mut [u8], x: u32, y: u32, rgb: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let i = ((y * SIZE + x) * 4) as usize;
    let da = px[i + 3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    if oa <= 0.0 {
        return;
    }
    for k in 0..3 {
        let s = rgb[k] as f32;
        let d = px[i + k] as f32;
        px[i + k] = ((s * a + d * da * (1.0 - a)) / oa).round() as u8;
    }
    px[i + 3] = (oa * 255.0).round() as u8;
}

/// "37%" for the status item title; nothing without a reading.
pub fn title(pct: Option<f64>) -> Option<String> {
    pct.map(|p| format!("{}%", p.round().clamp(0.0, 999.0) as i64))
}
