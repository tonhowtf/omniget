//! Genome v1: the 32 bytes of appearance that travel with a resident
//! (estudo 02 §5). Layout, byte by byte:
//!
//! - 0: format version (high 4 bits) and body base (low 4 bits, 0 = Anny)
//! - 1..=6: phenotypes u8 (gender, age, muscle, weight, height, proportions)
//! - 7..=14: four local face changes, (target index u8, value i8); 255 = none
//! - 15, 16: skin tone, skin variation
//! - 17, 18: hair style, hair color
//! - 19: eye color; 20: brows (high 4 bits) and beard (low 4 bits)
//! - 21..=24: top, bottom, shoes, accessory (0 = none)
//! - 25..=27: cloth colors (primary, secondary, accent)
//! - 28: gait (high 4) and idle (low 4); 29: mood (high 4) and blink rate (low 4)
//! - 30, 31: u16 LE seed for deterministic micro variation

use crate::catalog::{Catalog, Piece};
use crate::rng::Rng;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const GENOME_LEN: usize = 32;
pub const GENOME_VERSION: u8 = 1;
pub const NO_FACE: u8 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Genome {
    pub version: u8,
    pub body: u8,
    pub phenotypes: [u8; 6],
    pub face: [(u8, i8); 4],
    pub skin: u8,
    pub skin_var: u8,
    pub hair: u8,
    pub hair_color: u8,
    pub eyes: u8,
    pub brows: u8,
    pub beard: u8,
    pub top: u8,
    pub bottom: u8,
    pub shoes: u8,
    pub accessory: u8,
    pub colors: [u8; 3],
    pub gait: u8,
    pub idle: u8,
    pub mood: u8,
    pub blink: u8,
    pub seed: u16,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GenomeError {
    #[error("genome must be {GENOME_LEN} bytes, got {0}")]
    Length(usize),
    #[error("unsupported genome version {0}")]
    Version(u8),
    #[error("genome hex is invalid")]
    Hex,
}

pub mod pheno {
    pub const GENDER: usize = 0;
    pub const AGE: usize = 1;
    pub const MUSCLE: usize = 2;
    pub const WEIGHT: usize = 3;
    pub const HEIGHT: usize = 4;
    pub const PROPORTIONS: usize = 5;
}

pub fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub fn uq(v: u8) -> f32 {
    v as f32 / 255.0
}

impl Genome {
    pub fn encode(&self) -> [u8; GENOME_LEN] {
        let mut b = [0u8; GENOME_LEN];
        b[0] = (self.version << 4) | (self.body & 0x0f);
        b[1..7].copy_from_slice(&self.phenotypes);
        for (i, (t, v)) in self.face.iter().enumerate() {
            b[7 + i * 2] = *t;
            b[8 + i * 2] = *v as u8;
        }
        b[15] = self.skin;
        b[16] = self.skin_var;
        b[17] = self.hair;
        b[18] = self.hair_color;
        b[19] = self.eyes;
        b[20] = (self.brows << 4) | (self.beard & 0x0f);
        b[21] = self.top;
        b[22] = self.bottom;
        b[23] = self.shoes;
        b[24] = self.accessory;
        b[25..28].copy_from_slice(&self.colors);
        b[28] = (self.gait << 4) | (self.idle & 0x0f);
        b[29] = (self.mood << 4) | (self.blink & 0x0f);
        b[30..32].copy_from_slice(&self.seed.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Result<Genome, GenomeError> {
        if b.len() != GENOME_LEN {
            return Err(GenomeError::Length(b.len()));
        }
        let version = b[0] >> 4;
        if version != GENOME_VERSION {
            return Err(GenomeError::Version(version));
        }
        let mut phenotypes = [0u8; 6];
        phenotypes.copy_from_slice(&b[1..7]);
        let mut face = [(NO_FACE, 0i8); 4];
        for (i, f) in face.iter_mut().enumerate() {
            *f = (b[7 + i * 2], b[8 + i * 2] as i8);
        }
        Ok(Genome {
            version,
            body: b[0] & 0x0f,
            phenotypes,
            face,
            skin: b[15],
            skin_var: b[16],
            hair: b[17],
            hair_color: b[18],
            eyes: b[19],
            brows: b[20] >> 4,
            beard: b[20] & 0x0f,
            top: b[21],
            bottom: b[22],
            shoes: b[23],
            accessory: b[24],
            colors: [b[25], b[26], b[27]],
            gait: b[28] >> 4,
            idle: b[28] & 0x0f,
            mood: b[29] >> 4,
            blink: b[29] & 0x0f,
            seed: u16::from_le_bytes([b[30], b[31]]),
        })
    }

    pub fn to_hex(&self) -> String {
        self.encode().iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Result<Genome, GenomeError> {
        let s = s.trim();
        if s.len() != GENOME_LEN * 2 || !s.is_ascii() {
            return Err(GenomeError::Hex);
        }
        let mut b = [0u8; GENOME_LEN];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| GenomeError::Hex)?;
        }
        Genome::decode(&b)
    }

    pub fn phenotype(&self, i: usize) -> f32 {
        uq(self.phenotypes[i])
    }

    /// Everything that breaks the catalog, as readable messages. Empty = valid.
    pub fn validate(&self, cat: &Catalog) -> Vec<String> {
        let mut e = Vec::new();
        let mut range = |what: &str, v: usize, n: usize| {
            if v >= n {
                e.push(format!("{what} index {v} is outside the catalog (0..{n})"));
            }
        };
        if self.version != GENOME_VERSION {
            range(
                "version",
                self.version as usize,
                GENOME_VERSION as usize + 1,
            );
        }
        range("body", self.body as usize, 1);
        for (t, _) in self.face {
            if t != NO_FACE {
                range("face target", t as usize, cat.face_targets.len());
            }
        }
        range("skin", self.skin as usize, cat.skin_tones.len());
        range("hair style", self.hair as usize, cat.hair_styles.len());
        range(
            "hair color",
            self.hair_color as usize,
            cat.hair_colors.len(),
        );
        range("eye color", self.eyes as usize, cat.eye_colors.len());
        range("brows", self.brows as usize, cat.brows.len());
        range("beard", self.beard as usize, cat.beards.len());
        range("top", self.top as usize, cat.tops.len());
        range("bottom", self.bottom as usize, cat.bottoms.len());
        range("shoes", self.shoes as usize, cat.shoes.len());
        range("accessory", self.accessory as usize, cat.accessories.len());
        for c in self.colors {
            range("cloth color", c as usize, cat.cloth_colors.len());
        }
        range("gait", self.gait as usize, cat.gaits.len());
        range("idle", self.idle as usize, cat.idles.len());
        range("mood", self.mood as usize, cat.moods.len());
        if e.is_empty() {
            let top = &cat.tops[self.top as usize];
            if top.full_body && self.bottom != 0 {
                e.push(format!(
                    "top '{}' covers the legs, bottom must be none",
                    top.id
                ));
            }
            if self.top == 0 {
                e.push("a top is required".into());
            }
            if self.bottom == 0 && !top.full_body {
                e.push("a bottom is required".into());
            }
        }
        e
    }

    /// Makes the genome satisfy the piece restrictions without touching the
    /// rest (full-body tops drop the bottom, missing base clothing is added).
    pub fn fix_restrictions(&mut self, cat: &Catalog) {
        let top_full = |g: &Genome| {
            cat.tops
                .get(g.top as usize)
                .map(|t| t.full_body)
                .unwrap_or(false)
        };
        if self.top == 0 {
            self.top = index_or(&cat.tops, "t-shirt", 1);
        }
        if top_full(self) {
            self.bottom = 0;
        } else if self.bottom == 0 {
            self.bottom = index_or(&cat.bottoms, "pants", 1);
        }
    }

    /// A plausible random resident. Same seed, same genome.
    pub fn random(seed: u64) -> Genome {
        random_with(crate::catalog::catalog(), seed)
    }
}

fn index_or(pieces: &[Piece], id: &str, fallback: u8) -> u8 {
    pieces
        .iter()
        .position(|p| p.id == id)
        .map(|i| i as u8)
        .unwrap_or(fallback)
}

pub fn random_with(cat: &Catalog, seed: u64) -> Genome {
    let mut r = Rng::new(seed ^ 0x6765_6e6f_6d65_7631);
    let mut g = Genome {
        version: GENOME_VERSION,
        ..Default::default()
    };
    // gender: mostly near the two ends, some in between
    let gender = match r.below(20) {
        0..=8 => r.range(0.0, 0.22),
        9..=17 => r.range(0.78, 1.0),
        _ => r.range(0.3, 0.7),
    };
    // age: mostly adults, some children and elders
    let age = match r.below(20) {
        0..=2 => r.range(0.25, 0.55),
        3..=15 => r.range(0.62, 0.86),
        _ => r.range(0.88, 1.0),
    };
    let muscle = r.bell(0.5, 0.18);
    let weight = r.bell(0.45, 0.2);
    let height = r.bell(0.38, 0.12);
    let proportions = r.range(0.0, 0.35);
    g.phenotypes = [
        q(gender),
        q(age),
        q(muscle),
        q(weight),
        q(height),
        q(proportions),
    ];

    let n_face = cat.face_targets.len() as u32;
    let mut used = Vec::new();
    for f in g.face.iter_mut() {
        let mut t = r.below(n_face) as u8;
        while used.contains(&t) {
            t = r.below(n_face) as u8;
        }
        used.push(t);
        *f = (t, (r.range(-0.8, 0.8) * 127.0).round() as i8);
    }
    g.skin = r.below(cat.skin_tones.len() as u32) as u8;
    g.skin_var = r.below(256) as u8;

    let feminine = gender > 0.6;
    let elder = age > 0.88;
    let child = age < 0.56;
    let styles: Vec<&str> = if feminine {
        vec![
            "long",
            "long-wavy",
            "ponytail",
            "bun",
            "bob",
            "shoulder",
            "braids",
            "curly",
            "afro",
            "pixie",
            "locs",
            "short",
        ]
    } else {
        vec![
            "short",
            "buzz",
            "side-part",
            "curly",
            "afro",
            "locs",
            "bald",
            "short",
            "buzz",
            "mohawk",
            "shoulder",
            "ponytail",
        ]
    };
    let mut style = styles[r.below(styles.len() as u32) as usize];
    if elder && !feminine && r.below(3) == 0 {
        style = "bald";
    }
    g.hair = cat
        .hair_styles
        .iter()
        .position(|h| h.id == style)
        .unwrap_or(2) as u8;
    let natural = [
        "black",
        "dark-brown",
        "brown",
        "chestnut",
        "auburn",
        "copper",
        "ginger",
        "light-brown",
        "dark-blonde",
        "blonde",
    ];
    let color = if elder {
        ["gray", "silver", "white"][r.below(3) as usize]
    } else if r.below(25) == 0 {
        ["blue", "pink", "green", "purple", "platinum"][r.below(5) as usize]
    } else {
        natural[r.below(natural.len() as u32) as usize]
    };
    g.hair_color = cat
        .hair_colors
        .iter()
        .position(|h| h.id == color)
        .unwrap_or(0) as u8;
    g.eyes = r.below(cat.eye_colors.len() as u32) as u8;
    g.brows = 1 + r.below(cat.brows.len() as u32 - 1) as u8;
    g.beard = if !feminine && !child && r.below(5) < 2 {
        1 + r.below(cat.beards.len() as u32 - 1) as u8
    } else {
        0
    };

    // clothing from a palette
    g.top = 1 + r.below(cat.tops.len() as u32 - 1) as u8;
    if cat.tops[g.top as usize].id == "dress" && !feminine && r.below(3) != 0 {
        g.top = index_or(&cat.tops, "shirt", 4);
    }
    g.bottom = 2 + r.below(cat.bottoms.len() as u32 - 2) as u8;
    if !feminine && cat.bottoms[g.bottom as usize].id.contains("skirt") && r.below(3) != 0 {
        g.bottom = index_or(&cat.bottoms, "pants", 4);
    }
    g.shoes = 1 + r.below(cat.shoes.len() as u32 - 1) as u8;
    g.accessory = if r.below(3) == 0 {
        1 + r.below(cat.accessories.len() as u32 - 1) as u8
    } else {
        0
    };
    let pal = &cat.palettes[r.below(cat.palettes.len() as u32) as usize];
    for c in g.colors.iter_mut() {
        *c = pal.colors[r.below(pal.colors.len() as u32) as usize];
    }
    g.gait = r.below(cat.gaits.len() as u32) as u8;
    g.idle = r.below(cat.idles.len() as u32) as u8;
    g.mood = r.below(cat.moods.len() as u32) as u8;
    g.blink = 3 + r.below(10) as u8;
    g.seed = r.below(65536) as u16;
    g.fix_restrictions(cat);
    g
}

impl Serialize for Genome {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Genome {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Genome::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

/// Anny body height (meters, T-pose, head to sole) for a phenotype, from the
/// baked LOD: linear in `height`, tabulated over age, per gender end.
pub fn body_height_m(gender: f32, age: f32, height: f32) -> f32 {
    let (lo, hi) = height_span(gender, age);
    lo + (hi - lo) * height.clamp(0.0, 1.0)
}

/// The `height` phenotype that gives `meters` at this gender and age.
pub fn height_for(gender: f32, age: f32, meters: f32) -> f32 {
    let (lo, hi) = height_span(gender, age);
    ((meters - lo) / (hi - lo)).clamp(0.0, 1.0)
}

// Measured with bake data (see tools/bake_anny.py): rows of
// (age, male h=0, male h=1, female h=0, female h=1).
const HEIGHTS: [(f32, f32, f32, f32, f32); 9] = [
    (0.0, 0.436, 0.891, 0.436, 0.891),
    (0.1, 0.601, 1.203, 0.573, 1.174),
    (0.2, 0.767, 1.515, 0.710, 1.458),
    (0.33, 0.982, 1.921, 0.888, 1.827),
    (0.5, 1.175, 2.190, 1.057, 2.072),
    (0.6, 1.287, 2.345, 1.155, 2.214),
    (0.67, 1.362, 2.448, 1.221, 2.308),
    (0.8, 1.350, 2.437, 1.215, 2.306),
    (1.0, 1.333, 2.420, 1.207, 2.302),
];

fn height_span(gender: f32, age: f32) -> (f32, f32) {
    let a = age.clamp(0.0, 1.0);
    let mut i = 1;
    while i < HEIGHTS.len() - 1 && HEIGHTS[i].0 < a {
        i += 1;
    }
    let (a0, a1) = (HEIGHTS[i - 1], HEIGHTS[i]);
    let t = ((a - a0.0) / (a1.0 - a0.0)).clamp(0.0, 1.0);
    let l = |x: f32, y: f32| x + (y - x) * t;
    let g = gender.clamp(0.0, 1.0);
    let lo = l(a0.1, a1.1) * (1.0 - g) + l(a0.3, a1.3) * g;
    let hi = l(a0.2, a1.2) * (1.0 - g) + l(a0.4, a1.4) * g;
    (lo, hi)
}

/// Anny's age phenotype for a number of years (anchors: newborn, 1, 11, 25, 90;
/// phenotype 0 = 1 year, 1/3 = 11, 2/3 = 25, 1 = 90).
pub fn age_for_years(years: f32) -> f32 {
    let y = years.max(0.0);
    if y <= 1.0 {
        0.0
    } else if y <= 11.0 {
        (y - 1.0) / 10.0 / 3.0
    } else if y <= 25.0 {
        1.0 / 3.0 + (y - 11.0) / 14.0 / 3.0
    } else {
        (2.0 / 3.0 + (y - 25.0) / 65.0 / 3.0).min(1.0)
    }
}

pub fn years_for_age(age: f32) -> f32 {
    let a = age.clamp(0.0, 1.0);
    if a <= 1.0 / 3.0 {
        1.0 + a * 3.0 * 10.0
    } else if a <= 2.0 / 3.0 {
        11.0 + (a - 1.0 / 3.0) * 3.0 * 14.0
    } else {
        25.0 + (a - 2.0 / 3.0) * 3.0 * 65.0
    }
}
