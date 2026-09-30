//! The pure relief function: `(params, position) → height`. It is what the
//! basin's border is pinned to and what the fast "valleys" preview samples
//! directly. Positions are Q16 macro nodes (1 node = 4 tiles = 8 m).

use super::noise::{fbm, ridged, stretch, table, HALF, ONE};
use crate::hash::{hash2, pick_u64};
use crate::types::*;

/// Bumped when the terrain algorithm changes: it enters the cache key and the
/// seed, so an old world is never silently rewritten.
pub const TERRAIN_VERSION: u64 = 1;

pub const TILES_PER_NODE: i32 = 4;

/// Distance (macro nodes) over which the origin land bias fades out.
pub const ORIGIN_FADE: i64 = 200;

/// Everything the terrain reads from a charter, already in integers.
#[derive(Clone, Debug)]
pub struct Params {
    pub seed: u64,
    pub sea: i64,
    pub hill: i64,
    pub mountain: i64,
    pub erosion: i64,
    pub rivers: i64,
    pub forest: i64,
    /// Continental bias, Q16 (positive = more land).
    pub land_bias: i64,
    /// Forest bias per mille added to `forest`.
    pub forest_bias: i64,
    /// Extra continentalness at the world origin, Q16, fading out over
    /// [`ORIGIN_FADE`] nodes: cities are founded around region (0,0), so the
    /// origin should be land for almost every seed.
    pub origin_land: i64,
    pub climate: Climate,
    /// Cache key prefix: canonical hash of everything above + versions.
    pub key: String,
}

impl Params {
    pub fn from_charter(c: &Charter) -> Params {
        let r = &c.relief;
        let (land_bias, forest_bias, origin_land) = match c.climate {
            Climate::Temperate => (ONE / 20, 0, ONE / 4),
            Climate::Tropical => (-ONE / 40, 150, ONE / 6),
            Climate::Arid => (ONE / 12, -350, ONE / 4),
            Climate::Cold => (ONE / 40, -50, ONE / 4),
            Climate::Coastal => (-ONE / 8, 0, ONE / 8),
        };
        let seed = pick_u64(&c.seed, "terrain", &c.generator_version, TERRAIN_VERSION);
        let key = crate::hash::canonical_hash(&(
            &c.seed,
            &c.generator_version,
            TERRAIN_VERSION,
            r,
            &c.climate,
        ));
        Params {
            seed,
            sea: r.sea_level_mm as i64,
            hill: r.hill_mm.max(0) as i64,
            mountain: r.mountain_mm.max(0) as i64,
            erosion: r.erosion.min(1000) as i64,
            rivers: r.rivers.min(1000) as i64,
            forest: r.forest.min(1000) as i64,
            land_bias,
            forest_bias,
            origin_land,
            climate: c.climate.clone(),
            key,
        }
    }
}

/// Height and valley-river strength at a point.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub h: i32,
    /// 0 = dry, 256 = river edge, 512 = river axis (valleys channel).
    pub river: i32,
}

/// Continental height in per mille of `hill_mm`, by stretched continentalness.
const SHAPE: &[(i64, i64)] = &[
    (0, -1400),
    (18350, -500), // 0.28
    (26214, -60),  // 0.40
    (28836, 40),   // 0.44
    (36045, 300),  // 0.55
    (49152, 700),  // 0.75
    (65536, 1100), // 1.0
];

/// The base relief at a Q16 node position.
pub fn sample(p: &Params, px: i64, py: i64) -> Sample {
    let s = p.seed;
    // Domain warp: two low-frequency fields push the lookup around.
    let wx = fbm(s, 11, px, py, 64 * ONE, 3) - HALF;
    let wy = fbm(s, 12, px, py, 64 * ONE, 3) - HALF;
    let qx = px + wx * 40;
    let qy = py + wy * 40;

    let d = ((px / ONE).pow(2) as u64 + (py / ONE).pow(2) as u64).isqrt() as i64;
    let origin = p.origin_land * (ORIGIN_FADE - d).max(0) / ORIGIN_FADE;
    let cont = stretch(fbm(s, 1, qx, qy, 320 * ONE, 5) + p.land_bias + origin, 2200);
    let mut h = p.sea + p.hill * table(SHAPE, cont) / 1000;
    // How much of this is land (0 at the shore, ONE well inland).
    let lf = ((cont - 26214) * 4).clamp(0, ONE);

    let hd = stretch(fbm(s, 2, qx, qy, 36 * ONE, 5), 1800) - HALF;
    h += ((p.hill * 2 * hd) >> 16) * (200 + ((400 * lf) >> 16)) / 1000;

    if p.mountain > 0 {
        let m = stretch(fbm(s, 4, qx, qy, 360 * ONE, 3), 2200);
        let mask = ((m - 36045) * 4).clamp(0, ONE);
        if mask > 0 {
            let r = ridged(s, 3, qx, qy, 110 * ONE, 5);
            h += (((((p.mountain * r) >> 16) * mask) >> 16) * lf) >> 16;
        }
    }

    let mut river = 0;
    if p.rivers > 0 && h > p.sea {
        let n = fbm(s, 5, qx, qy, 150 * ONE, 4);
        let d = (n - HALF).abs();
        let rw = 262 + p.rivers * 1311 / 1000;
        let vw = rw * 6;
        if d < vw {
            let t = d * ONE / vw;
            let prof = ONE - ((t * t) >> 16);
            let floor = p.sea + ((h - p.sea) / 6).max(200);
            if h > floor {
                h -= ((h - floor) * prof) >> 16;
            }
            if d < rw {
                river = (256 + (rw - d) * 256 / rw) as i32;
            }
        }
    }
    Sample {
        h: h.clamp(i32::MIN as i64 / 2, i32::MAX as i64 / 2) as i32,
        river,
    }
}

/// Q16 node position of a global corner (tile boundary) coordinate.
#[inline]
pub fn corner_pos(g: i32) -> i64 {
    (g as i64) * ONE / TILES_PER_NODE as i64
}

/// Q16 node position of a global tile centre.
#[inline]
pub fn tile_center_pos(g: i32) -> i64 {
    (2 * g as i64 + 1) * ONE / (2 * TILES_PER_NODE as i64)
}

/// Fine detail added at every corner (a few decimetres), pure in global
/// corner coordinates so neighbouring regions agree.
pub fn detail(p: &Params, gx: i32, gy: i32) -> i32 {
    let amp = (p.hill / 50).min(300);
    if amp == 0 {
        return 0;
    }
    let n = fbm(p.seed, 21, gx as i64 * ONE, gy as i64 * ONE, 7 * ONE, 2) - HALF;
    ((2 * amp * n) >> 16) as i32
}

/// Forest noise at a global tile, 0..1000 (per mille).
pub fn forest_noise(p: &Params, gx: i32, gy: i32) -> i64 {
    let n = stretch(
        fbm(p.seed, 31, gx as i64 * ONE, gy as i64 * ONE, 26 * ONE, 4),
        2000,
    );
    let jitter = (hash2(p.seed, gx, gy, 32) % 120) as i64;
    ((n * 1000) >> 16) * 9 / 10 + jitter
}
