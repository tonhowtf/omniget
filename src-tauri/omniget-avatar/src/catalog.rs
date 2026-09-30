//! The versioned appearance catalog. One JSON file (`catalog/catalog_v1.json`)
//! is the single source: Rust embeds it, the bake script reads the curated face
//! targets from it and the frontend imports it. Client and server agree on a
//! catalog by its hash.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub const CATALOG_V1: &str = include_str!("../catalog/catalog_v1.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Named {
    pub id: String,
    #[serde(default)]
    pub hex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceTarget {
    pub id: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    pub id: String,
    #[serde(default)]
    pub cover: Vec<String>,
    #[serde(default)]
    pub full_body: bool,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub shape: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    pub id: String,
    pub colors: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: String,
    pub version: u32,
    pub phenotypes: Vec<String>,
    pub face_targets: Vec<FaceTarget>,
    pub skin_tones: Vec<Named>,
    pub hair_styles: Vec<Named>,
    pub hair_colors: Vec<Named>,
    pub eye_colors: Vec<Named>,
    pub brows: Vec<String>,
    pub beards: Vec<String>,
    pub tops: Vec<Piece>,
    pub bottoms: Vec<Piece>,
    pub shoes: Vec<Piece>,
    pub accessories: Vec<Piece>,
    pub cloth_colors: Vec<String>,
    pub palettes: Vec<Palette>,
    pub gaits: Vec<String>,
    pub idles: Vec<String>,
    pub moods: Vec<String>,
    pub words: serde_json::Map<String, serde_json::Value>,
}

impl Catalog {
    pub fn index_of<'a>(ids: impl IntoIterator<Item = &'a str>, id: &str) -> Option<usize> {
        ids.into_iter().position(|x| x.eq_ignore_ascii_case(id))
    }

    pub fn piece_ids(pieces: &[Piece]) -> Vec<&str> {
        pieces.iter().map(|p| p.id.as_str()).collect()
    }

    pub fn named_ids(items: &[Named]) -> Vec<&str> {
        items.iter().map(|p| p.id.as_str()).collect()
    }

    /// Phenotype targets of a descriptive word ("idoso", "tall"), if known.
    pub fn word(&self, w: &str) -> Option<Vec<(usize, f32)>> {
        let v = self.words.get(&w.trim().to_lowercase())?;
        let obj = v.as_object()?;
        let mut out = Vec::new();
        for (k, val) in obj {
            let i = self.phenotypes.iter().position(|p| p == k)?;
            out.push((i, val.as_f64()? as f32));
        }
        Some(out)
    }
}

/// FNV-1a 64 of the catalog bytes, as 16 hex chars. Stable across platforms.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn catalog_hash() -> String {
    format!("{:016x}", fnv1a64(CATALOG_V1.as_bytes()))
}

pub fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(|| serde_json::from_str(CATALOG_V1).expect("catalog_v1.json is valid"))
}

pub fn catalog_json() -> serde_json::Value {
    serde_json::from_str(CATALOG_V1).expect("catalog_v1.json is valid")
}

pub fn hex_to_rgb(hex: &str) -> [u8; 3] {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    [p(0), p(2), p(4)]
}

/// Index of the catalog color nearest to `hex` (squared RGB distance).
pub fn nearest_color<'a>(hexes: impl IntoIterator<Item = &'a str>, hex: &str) -> Option<usize> {
    let t = hex_to_rgb(hex);
    hexes
        .into_iter()
        .enumerate()
        .map(|(i, h)| {
            let c = hex_to_rgb(h);
            let d: i32 = (0..3).map(|k| (c[k] as i32 - t[k] as i32).pow(2)).sum();
            (i, d)
        })
        .min_by_key(|(_, d)| *d)
        .map(|(i, _)| i)
}
