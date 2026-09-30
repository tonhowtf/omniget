//! Public types of the normalizer: options in, asset and report out.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Tile side in mm (estudo 09: 2 m grid, 3 m levels).
pub const TILE_MM: i32 = 2000;
pub const LEVEL_MM: i32 = 3000;

/// Same slots as `omniget-citygen::types::PALETTE_SLOTS` (kept as a copy so
/// this crate builds alone, also on the server).
pub const PALETTE_SLOTS: &[&str] = &[
    "wall", "roof", "trim", "wood", "stone", "glass", "door", "ground", "grass", "water", "street",
    "plaza", "leaf", "trunk", "metal", "cloth",
];

/// Reference colour of each slot, used by "fit to style" when the caller has
/// no district palette at hand.
pub const DEFAULT_PALETTE: &[(&str, &str)] = &[
    ("wall", "#e8dcc4"),
    ("roof", "#b5543c"),
    ("trim", "#f4f1ea"),
    ("wood", "#8a5a36"),
    ("stone", "#8f8a82"),
    ("glass", "#8fb8c9"),
    ("door", "#5c3b24"),
    ("ground", "#9b8465"),
    ("grass", "#7aa24f"),
    ("water", "#3f7fae"),
    ("street", "#5b5a58"),
    ("plaza", "#c9b89a"),
    ("leaf", "#4f8a3a"),
    ("trunk", "#6b4a2e"),
    ("metal", "#6f7780"),
    ("cloth", "#c25b6e"),
];

pub const NORMALIZER_VERSION: &str = "omni.asset3d/1";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    /// Fits in one tile (bench, lamp, barrel).
    Prop,
    /// Footprint in whole tiles, 3 m per level.
    Building,
    /// ~1.7 m tall; skin and bones are preserved.
    Character,
    /// Small wall or ground ornament.
    Decoration,
}

impl AssetClass {
    pub fn as_str(self) -> &'static str {
        match self {
            AssetClass::Prop => "prop",
            AssetClass::Building => "building",
            AssetClass::Character => "character",
            AssetClass::Decoration => "decoration",
        }
    }

    /// Triangle budget of the class (estudo 06 §1 passo 5, against the 60k
    /// triangles of a 16×16 chunk).
    pub fn triangle_budget(self) -> usize {
        match self {
            AssetClass::Prop => 1_500,
            AssetClass::Building => 8_000,
            AssetClass::Character => 15_000,
            AssetClass::Decoration => 300,
        }
    }

    pub fn texture_px(self) -> u32 {
        match self {
            AssetClass::Prop | AssetClass::Decoration => 256,
            AssetClass::Building | AssetClass::Character => 512,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LicenseKind {
    Cc0,
    CcBy,
    /// "I made it".
    Own,
    /// Taken from the VRM `meta`.
    Vrm,
}

/// Declared licence; mandatory on import.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct License {
    pub kind: LicenseKind,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl License {
    pub fn validate(&self, has_vrm_meta: bool) -> Result<(), String> {
        match self.kind {
            LicenseKind::CcBy => {
                if self
                    .author
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                {
                    return Err("CC-BY needs the author's name".into());
                }
            }
            LicenseKind::Vrm => {
                if !has_vrm_meta {
                    return Err("the file has no VRM meta to take the licence from".into());
                }
            }
            LicenseKind::Cc0 | LicenseKind::Own => {}
        }
        Ok(())
    }
}

/// How a 2D image becomes 3D (estudo 06 §3).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageMode {
    /// Two triangles with alpha test.
    Standee,
    /// Alpha contour, simplified and extruded 3–5 cm, texture on the front.
    #[default]
    Papercraft,
    /// Relief from a depth map (Depth Anything V2 Small).
    Relief,
}

/// Input limits (estudo 06 §1 passo 1). Anything above is refused before work.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Limits {
    pub max_file_bytes: usize,
    pub max_triangles: usize,
    pub max_textures: usize,
    pub max_texture_px: u32,
    pub max_bones: usize,
    pub max_nodes: usize,
    pub max_data_uri_bytes: usize,
    /// Total bytes the loader may allocate for decoded accessors and images.
    pub max_decoded_bytes: usize,
    pub timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: 64 << 20,
            max_triangles: 2_000_000,
            max_textures: 64,
            max_texture_px: 8192,
            max_bones: 256,
            max_nodes: 20_000,
            max_data_uri_bytes: 32 << 20,
            max_decoded_bytes: 1 << 30,
            timeout_ms: 90_000,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NormalizeOptions {
    pub name: String,
    pub class: AssetClass,
    /// Quantize colours to the palette and drop textures (static classes) or
    /// posterize textures (characters).
    #[serde(default)]
    pub fit_style: bool,
    /// slot → `#rrggbb`; [`DEFAULT_PALETTE`] when absent.
    #[serde(default)]
    pub palette: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub image_mode: ImageMode,
    /// Overrides the class texture size (clamped to 64..=1024).
    #[serde(default)]
    pub texture_px: Option<u32>,
    /// WebP quality 0..=100.
    #[serde(default = "default_quality")]
    pub webp_quality: f32,
    #[serde(default)]
    pub limits: Limits,
    /// Written into `asset.extras.omniget` (and `asset.copyright`).
    #[serde(default)]
    pub license: Option<License>,
    /// Free text: where the file came from (site, generator, task id).
    #[serde(default)]
    pub provenance: Option<String>,
}

fn default_quality() -> f32 {
    80.0
}

impl NormalizeOptions {
    pub fn new(name: &str, class: AssetClass) -> Self {
        NormalizeOptions {
            name: name.to_string(),
            class,
            fit_style: false,
            palette: None,
            image_mode: ImageMode::default(),
            texture_px: None,
            webp_quality: default_quality(),
            limits: Limits::default(),
            license: None,
            provenance: None,
        }
    }
}

/// Occupied tiles of the asset seen from above, relative to its north-west
/// tile; `w_tiles` along +x (east), `d_tiles` along +z (south).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Footprint {
    pub w_tiles: i32,
    pub d_tiles: i32,
    pub cells: Vec<[i32; 2]>,
    /// Bounding box size in mm `[x, y, z]`.
    pub size_mm: [i32; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Collision {
    Box {
        min_mm: [i32; 3],
        max_mm: [i32; 3],
    },
    /// One box per 3 m level (buildings).
    Boxes {
        boxes: Vec<[[i32; 3]; 2]>,
    },
    Capsule {
        radius_mm: i32,
        height_mm: i32,
    },
}

/// Interaction point in the asset's local space (mm, pivot at the base centre).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AssetAnchor {
    /// sit | sleep | work | stand | stack | door | talk | plant | water | harvest
    pub action: String,
    pub pos_mm: [i32; 3],
    /// 0..=255 → 0..360°, clockwise from north, direction the user faces.
    pub yaw: u8,
    pub capacity: u8,
    /// `heuristic` or `llm`.
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub bytes: usize,
    pub triangles: usize,
    pub vertices: usize,
    pub meshes: usize,
    pub materials: usize,
    pub textures: usize,
    pub texture_px_max: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StepTime {
    pub step: String,
    pub ms: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Report {
    pub format: String,
    pub input: Stats,
    pub output: Stats,
    pub steps: Vec<StepTime>,
    pub total_ms: f64,
    pub scale: f32,
    pub warnings: Vec<String>,
}

/// What the VRM `meta` allows (estudo 02 §meta). The most restrictive value
/// is the default for anything missing or unknown.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Permissions {
    /// Someone other than the author may act as this avatar.
    pub use_as_avatar: bool,
    /// The file may be relayed to other players.
    pub redistribute_to_others: bool,
    pub modify: bool,
    /// A modified copy may leave the app (export).
    pub export: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VrmInfo {
    /// `0.x` or `1.0`.
    pub version: String,
    pub meta: serde_json::Value,
    pub permissions: Permissions,
    pub credit_required: bool,
}

/// Output of [`crate::normalize`].
#[derive(Clone, Debug)]
pub struct NormalizedAsset {
    pub glb: Vec<u8>,
    /// SHA-256 hex of `glb` (what circulates).
    pub hash: String,
    /// SHA-256 hex of the original input (to trace reports).
    pub source_hash: String,
    pub name: String,
    pub class: AssetClass,
    pub footprint: Footprint,
    pub collision: Collision,
    pub anchors: Vec<AssetAnchor>,
    /// `n`, `e`, `s` or `w`: side the asset's front faces.
    pub front: String,
    pub vrm: Option<VrmInfo>,
    /// 256 px PNG, 3/4 view.
    pub thumbnail_png: Vec<u8>,
    pub report: Report,
}
