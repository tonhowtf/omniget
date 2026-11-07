use ::image::{DynamicImage, Rgb, Rgba};
use serde::{Deserialize, Serialize};

const MAX_EV: f32 = 2.0;
const MAX_WARMTH_EV: f32 = 0.35;
const MIN_CROP: f64 = 0.01;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Edits {
    pub exposure: i16,
    pub contrast: i16,
    pub saturation: i16,
    pub temperature: i16,
    pub rotate: u16,
    pub crop: Option<Crop>,
}

impl Edits {
    pub fn normalized(&self) -> Edits {
        let clamp = |v: i16| v.clamp(-100, 100);
        Edits {
            exposure: clamp(self.exposure),
            contrast: clamp(self.contrast),
            saturation: clamp(self.saturation),
            temperature: clamp(self.temperature),
            rotate: (self.rotate / 90 % 4) * 90,
            crop: self.crop.and_then(Crop::normalized),
        }
    }

    pub fn is_identity(&self) -> bool {
        let n = self.normalized();
        n.exposure == 0
            && n.contrast == 0
            && n.saturation == 0
            && n.temperature == 0
            && n.rotate == 0
            && n.crop.is_none()
    }

    fn has_tone(&self) -> bool {
        self.exposure != 0 || self.contrast != 0 || self.saturation != 0 || self.temperature != 0
    }
}

impl Crop {
    pub fn normalized(self) -> Option<Crop> {
        if ![self.x, self.y, self.w, self.h]
            .iter()
            .all(|v| v.is_finite())
        {
            return None;
        }
        let x = self.x.clamp(0.0, 1.0);
        let y = self.y.clamp(0.0, 1.0);
        let w = self.w.min(1.0 - x);
        let h = self.h.min(1.0 - y);
        if w < MIN_CROP || h < MIN_CROP {
            return None;
        }
        if x <= 0.0 && y <= 0.0 && w >= 1.0 && h >= 1.0 {
            return None;
        }
        Some(Crop { x, y, w, h })
    }

    pub fn pixels(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let fw = f64::from(width);
        let fh = f64::from(height);
        let x = (self.x * fw).round().min(fw - 1.0).max(0.0) as u32;
        let y = (self.y * fh).round().min(fh - 1.0).max(0.0) as u32;
        let w = ((self.w * fw).round() as u32).clamp(1, width - x);
        let h = ((self.h * fh).round() as u32).clamp(1, height - y);
        (x, y, w, h)
    }
}

pub fn apply(img: DynamicImage, edits: &Edits) -> DynamicImage {
    let e = edits.normalized();
    let img = match e.rotate {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img,
    };
    let img = match e.crop {
        Some(c) if img.width() > 0 && img.height() > 0 => {
            let (x, y, w, h) = c.pixels(img.width(), img.height());
            img.crop_imm(x, y, w, h)
        }
        _ => img,
    };
    if !e.has_tone() {
        return img;
    }
    let tone = Tone::new(&e);
    if img.color().has_alpha() {
        let mut rgba = img.into_rgba8();
        for p in rgba.pixels_mut() {
            let [r, g, b] = tone.pixel([p[0], p[1], p[2]]);
            *p = Rgba([r, g, b, p[3]]);
        }
        DynamicImage::ImageRgba8(rgba)
    } else {
        let mut rgb = img.into_rgb8();
        for p in rgb.pixels_mut() {
            *p = Rgb(tone.pixel(p.0));
        }
        DynamicImage::ImageRgb8(rgb)
    }
}

struct Tone {
    curves: [[u8; 256]; 3],
    saturation: f32,
}

impl Tone {
    fn new(e: &Edits) -> Tone {
        let ev = f32::from(e.exposure) / 100.0 * MAX_EV;
        let warmth = f32::from(e.temperature) / 100.0 * MAX_WARMTH_EV;
        let gains = [(ev + warmth).exp2(), ev.exp2(), (ev - warmth).exp2()];
        let contrast = f32::from(e.contrast) / 100.0;
        let mut curves = [[0u8; 256]; 3];
        for (channel, curve) in curves.iter_mut().enumerate() {
            for (i, out) in curve.iter_mut().enumerate() {
                let linear = srgb_to_linear(i as f32 / 255.0) * gains[channel];
                let v = s_curve(linear_to_srgb(linear.min(1.0)), contrast);
                *out = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        Tone {
            curves,
            saturation: 1.0 + f32::from(e.saturation) / 100.0,
        }
    }

    fn pixel(&self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        let c = [
            self.curves[0][r as usize],
            self.curves[1][g as usize],
            self.curves[2][b as usize],
        ];
        if (self.saturation - 1.0).abs() < f32::EPSILON {
            return c;
        }
        let f = c.map(f32::from);
        let luma = 0.2126 * f[0] + 0.7152 * f[1] + 0.0722 * f[2];
        f.map(|v| {
            (luma + (v - luma) * self.saturation)
                .round()
                .clamp(0.0, 255.0) as u8
        })
    }
}

fn s_curve(x: f32, contrast: f32) -> f32 {
    if contrast == 0.0 {
        return x;
    }
    let p = if contrast > 0.0 {
        1.0 + 2.0 * contrast
    } else {
        1.0 / (1.0 - 2.0 * contrast)
    };
    if x < 0.5 {
        0.5 * (2.0 * x).powf(p)
    } else {
        1.0 - 0.5 * (2.0 * (1.0 - x)).powf(p)
    }
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
