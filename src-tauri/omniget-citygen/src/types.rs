//! Shared data of the procedural city. Owned by the orchestrator: every other
//! module of this crate reads these types, so change them only through it.
//!
//! Conventions (estudo 99, arquivo 09 §0):
//! - 1 tile = 2 m horizontally, 1 level (floor) = 3 m. `Fixed` = tile × 256.
//! - A region is 64×64 tiles; a chunk is 16×16. Tile coordinates inside a
//!   region go from 0 to 63; `x` grows east, `y` grows south.
//! - Heights are integer millimetres at tile **corners** (65×65 per region),
//!   so terrain and modules share the dual grid.
//! - Everything that enters a hash or a check is an integer. Floats only
//!   exist in the client, after hashing.
//! - 3D output (instances) uses the three.js convention: `x` = east,
//!   `y` = up, `z` = south, in millimetres.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const REGION_TILES: i32 = 64;
pub const CHUNK_TILES: i32 = 16;
pub const TILE_MM: i32 = 2000;
pub const LEVEL_MM: i32 = 3000;
pub const FIXED_PER_TILE: i32 = 256;
pub const GENERATOR_VERSION: &str = "citygen-1";

/// Row-major index of a tile inside a region (`y * 64 + x`).
pub fn tile_index(x: i32, y: i32) -> usize {
    (y * REGION_TILES + x) as usize
}

/// Row-major index of a corner inside a region (`y * 65 + x`).
pub fn corner_index(x: i32, y: i32) -> usize {
    (y * (REGION_TILES + 1) + x) as usize
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    N,
    E,
    S,
    W,
}

impl Dir {
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::N => (0, -1),
            Dir::E => (1, 0),
            Dir::S => (0, 1),
            Dir::W => (-1, 0),
        }
    }
    /// Quarter turns clockwise from north, as used by `Instance::rot`.
    pub fn quarter(self) -> u8 {
        match self {
            Dir::N => 0,
            Dir::E => 1,
            Dir::S => 2,
            Dir::W => 3,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    X,
    Y,
}

/// What covers a tile at ground level.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Grass,
    Forest,
    Sand,
    Rock,
    Water,
    Street,
    Sidewalk,
    Plaza,
    Bridge,
    Lot,
    Yard,
    Field,
}

impl Surface {
    pub fn walkable(self) -> bool {
        !matches!(self, Surface::Water | Surface::Rock)
    }
    pub fn public(self) -> bool {
        matches!(
            self,
            Surface::Street | Surface::Sidewalk | Surface::Plaza | Surface::Bridge
        )
    }
}

// ── Charter ────────────────────────────────────────────────────────────────

/// The city's founding document, written by an LLM (M3) and executed by the
/// generator. Nothing in here is geometry.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Charter {
    /// `omni.city.charter/v1`
    pub schema: String,
    pub city_id: String,
    pub name: String,
    pub seed: String,
    #[serde(default = "default_generator")]
    pub generator_version: String,
    /// Free text shown to people and fed back to LLMs; never read by the solver.
    #[serde(default)]
    pub story: String,
    pub climate: Climate,
    #[serde(default)]
    pub culture: String,
    /// Default architectural style (a kit style key, e.g. `vila-colonial`).
    pub style: String,
    /// Default palette key (see [`Palette`]).
    pub palette: String,
    #[serde(default)]
    pub palettes: Vec<Palette>,
    pub relief: Relief,
    #[serde(default)]
    pub proximity: Vec<ProximityRule>,
    #[serde(default)]
    pub districts: Vec<District>,
    /// Publication policy of LLM works: `auto` (zero violations publishes)
    /// or `review` (a person approves).
    #[serde(default = "default_policy")]
    pub publish_policy: String,
}

fn default_generator() -> String {
    GENERATOR_VERSION.to_string()
}
fn default_policy() -> String {
    "auto".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Climate {
    Temperate,
    Tropical,
    Arid,
    Cold,
    Coastal,
}

/// Terrain shape knobs, all integers. Heights in millimetres.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Relief {
    /// Sea level in mm.
    pub sea_level_mm: i32,
    /// Typical height of hills above sea level, mm.
    pub hill_mm: i32,
    /// Mountain amplitude, mm (0 = none).
    pub mountain_mm: i32,
    /// 0..=1000: how much the erosion carves valleys.
    pub erosion: u16,
    /// 0..=1000: river density.
    pub rivers: u16,
    /// 0..=1000: forest cover outside the city.
    pub forest: u16,
    /// Maximum slope the city flattens under lots, per mille (100 = 10 %).
    pub city_flatten_slope: u16,
}

/// "A site of kind A must be within / beyond N tiles of B" (Veloren's
/// `ProximityRequirements`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProximityRule {
    pub kind: String,
    pub near: Option<String>,
    pub min_tiles: Option<i32>,
    pub max_tiles: Option<i32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct District {
    pub id: String,
    pub kind: String,
    /// Regions the district covers.
    pub regions: Vec<[i32; 2]>,
    #[serde(default)]
    pub style: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub key: String,
    /// slot → `#rrggbb`. Slots: see [`PALETTE_SLOTS`].
    pub slots: BTreeMap<String, String>,
}

/// Material slots every kit module uses. The client colours each slot from the
/// building's palette; there is no colour texture.
pub const PALETTE_SLOTS: &[&str] = &[
    "wall", "roof", "trim", "wood", "stone", "glass", "door", "ground", "grass", "water", "street",
    "plaza", "leaf", "trunk", "metal", "cloth",
];

// ── Blueprint (authoritative output of the solver) ─────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Blueprint {
    /// `omni.city.blueprint/v1`
    pub schema: String,
    pub city_id: String,
    pub region: [i32; 2],
    pub generator_version: String,
    /// `sha256:<hex>` of the canonical program that produced this.
    pub program_hash: String,
    /// 64×64, [`tile_index`].
    pub surface: Vec<Surface>,
    /// 65×65 corner heights in mm, [`corner_index`].
    pub corner_height_mm: Vec<i32>,
    pub streets: Vec<StreetRef>,
    pub lots: Vec<Lot>,
    pub buildings: Vec<Building>,
    pub props: Vec<Prop>,
    /// Named public places (plazas, parks) for the checker and the residents.
    pub places: Vec<Place>,
    /// `sha256:<hex>` of everything above, canonical JSON.
    pub hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StreetRef {
    pub id: String,
    pub class: String,
    pub width: i32,
    pub tiles: Vec<[i32; 2]>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Lot {
    pub id: String,
    /// x, y, w, h in region tiles.
    pub rect: [i32; 4],
    pub zone: String,
    /// The side that faces the street.
    pub front: Dir,
    pub building: Option<String>,
    /// Opaque reference to the ownership ledger (never a user id).
    pub owner_ref: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Building {
    pub id: String,
    pub lot: String,
    pub style: String,
    pub palette: String,
    #[serde(rename = "use")]
    pub use_: String,
    pub floors: u8,
    /// Occupied tiles per level, `levels[k]` for k in 0..floors (+ tower).
    pub levels: Vec<Vec<[i32; 2]>>,
    pub door: Door,
    pub stairs: Vec<[i32; 2]>,
    pub roof: Roof,
    pub bays: Vec<Bay>,
    pub tower: Option<Tower>,
    /// Script-defined typology (M5), if any: `codegeo:<sha256>`.
    #[serde(default)]
    pub typology: Option<String>,
    /// Construction stage 0..=100 (100 = finished). The mesher draws partial
    /// levels and scaffolding below 100.
    #[serde(default = "full")]
    pub progress: u8,
    pub owner_ref: Option<String>,
}

fn full() -> u8 {
    100
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Door {
    pub cell: [i32; 2],
    /// The public tile the door opens onto.
    pub outside: [i32; 2],
    pub face: Dir,
    pub level: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Roof {
    /// duas-aguas | quatro-aguas | plano | conico
    pub kind: String,
    pub ridge_axis: Axis,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Bay {
    pub level: u8,
    pub cell: [i32; 2],
    pub face: Dir,
    /// janela | vitrine | porta | sacada | cega
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tower {
    pub cells: Vec<[i32; 2]>,
    pub from_level: u8,
    pub levels: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Prop {
    pub kind: String,
    pub preset: String,
    pub cell: [i32; 2],
    pub seed: u32,
    /// Usable prop (bench, seat, planter…) exposes anchors to the simulation.
    #[serde(default)]
    pub anchors: Vec<Anchor>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Place {
    pub id: String,
    pub kind: String,
    pub rect: [i32; 4],
}

/// Where a character stands/sits to use something (MUNDO-VIVO-PROPOSTA with
/// the 3D adjustment: level + yaw).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    /// sit | sleep | work | plant | water | harvest | talk | stand | enter
    pub action: String,
    pub cell: [i32; 2],
    pub level: u8,
    /// 0..=255 → 0..360°, clockwise from north.
    pub yaw: u8,
    pub capacity: u8,
}

// ── Checker ────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// `[code, id, measure]` plus a sentence an LLM can act on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// R1..R18 (estudo 09 §4), or P* for program/patch errors.
    pub code: String,
    pub id: String,
    pub measure: String,
    pub message: String,
    pub severity: Severity,
}

// ── Mesher output ──────────────────────────────────────────────────────────

/// One kit module placed in the world. The client groups instances by `key`
/// into one `InstancedMesh` each.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    /// `layer/case/style/variant`, e.g. `parede/concavo/vila-colonial/2`.
    pub key: String,
    pub x_mm: i32,
    pub y_mm: i32,
    pub z_mm: i32,
    /// Quarter turns clockwise seen from above (0..=3).
    pub rot: u8,
    /// Palette key used to colour the module's slots.
    pub palette: String,
    /// The building (or prop) this belongs to, for picking and inspection.
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InstanceList {
    pub region: [i32; 2],
    pub blueprint_hash: String,
    pub instances: Vec<Instance>,
    /// `sha256:<hex>` over the canonical instance list.
    pub hash: String,
}

/// Terrain as plain triangles (mm, three.js axes) with a material slot per
/// vertex. Built by the mesher from the blueprint's surface and heights.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerrainMesh {
    pub region: [i32; 2],
    /// xyz triples, mm.
    pub positions: Vec<i32>,
    /// One palette slot index per vertex into [`PALETTE_SLOTS`].
    pub slots: Vec<u8>,
    pub indices: Vec<u32>,
}
