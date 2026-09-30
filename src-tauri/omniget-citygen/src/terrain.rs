//! Terrain: basins with fixed-point erosion, rivers, natural surface.
//! Owner: agent A.
//!
//! Three scales (estudo 99, arquivo 10 §6): a pure relief function of
//! (seed, position) in [`field`]; a basin of 1024×1024 tiles (257×257 macro
//! nodes of 4 tiles) eroded and drained in [`basin`]; and regions of 64×64
//! tiles sampled from the basin by integer bilinear interpolation in global
//! coordinates, so neighbouring regions match exactly on their shared edge.

pub mod basin;
pub mod field;
pub mod noise;

use crate::types::*;
use basin::{Basin, BASIN_TILES, CELLS, LAKE, NO_WATER, RIVER, SEA};
use field::{Params, TILES_PER_NODE};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What kind of water a tile holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaterKind {
    Dry,
    River,
    Lake,
    Sea,
}

/// One tile, as seen by the natural-surface rules.
#[derive(Clone, Copy, Debug)]
pub struct TileInfo {
    pub corners: [i32; 4],
    pub avg: i32,
    pub range: i32,
    /// Bilinear river strength at the tile centre (0..=512).
    pub river: i32,
    pub water: WaterKind,
    /// Water surface, mm, or `i32::MIN` when dry.
    pub water_level: i32,
}

/// Macro node, as seen by trails and sites.
#[derive(Clone, Copy, Debug)]
pub struct NodeInfo {
    pub alt: i32,
    pub flags: u8,
}

impl NodeInfo {
    pub fn wet(&self) -> bool {
        self.flags & (RIVER | LAKE | SEA) != 0
    }
}

/// Terrain sampler in global tile coordinates, holding the basins it touched.
/// `fast` = the valleys preview (no basin, no erosion).
pub struct Sampler {
    pub p: Params,
    pub fast: bool,
    basins: BTreeMap<(i32, i32), Arc<Basin>>,
}

impl Sampler {
    pub fn new(c: &Charter, fast: bool) -> Sampler {
        Sampler {
            p: Params::from_charter(c),
            fast,
            basins: BTreeMap::new(),
        }
    }

    pub fn sea(&self) -> i32 {
        self.p.sea as i32
    }

    fn basin(&mut self, bx: i32, by: i32) -> Arc<Basin> {
        if let Some(b) = self.basins.get(&(bx, by)) {
            return b.clone();
        }
        let b = basin::get(&self.p, bx, by);
        self.basins.insert((bx, by), b.clone());
        b
    }

    /// Pick the basin holding global corner `g` on one axis: on a shared
    /// border either basin is exact (the border is pinned), so reuse one we
    /// already have instead of generating a neighbour.
    fn basin_axis(&self, g: i32, other: i32, x_axis: bool) -> i32 {
        let b = g.div_euclid(BASIN_TILES);
        if g - b * BASIN_TILES == 0 {
            let key = if x_axis {
                (b - 1, other)
            } else {
                (other, b - 1)
            };
            if self.basins.contains_key(&key) {
                return b - 1;
            }
        }
        b
    }

    /// Height (mm) at a global corner.
    pub fn corner(&mut self, gx: i32, gy: i32) -> i32 {
        let d = field::detail(&self.p, gx, gy);
        if self.fast {
            return field::sample(&self.p, field::corner_pos(gx), field::corner_pos(gy)).h + d;
        }
        let by0 = gy.div_euclid(BASIN_TILES);
        let bx = self.basin_axis(gx, by0, true);
        let by = self.basin_axis(gy, bx, false);
        let b = self.basin(bx, by);
        let lx = gx - bx * BASIN_TILES;
        let ly = gy - by * BASIN_TILES;
        let i = (lx / TILES_PER_NODE).min(CELLS - 1);
        let j = (ly / TILES_PER_NODE).min(CELLS - 1);
        let fx = (lx - i * TILES_PER_NODE) as i64;
        let fy = (ly - j * TILES_PER_NODE) as i64;
        let t = TILES_PER_NODE as i64;
        let a = b.alt[Basin::idx(i, j)] as i64;
        let bb = b.alt[Basin::idx(i + 1, j)] as i64;
        let cc = b.alt[Basin::idx(i, j + 1)] as i64;
        let dd = b.alt[Basin::idx(i + 1, j + 1)] as i64;
        let v = (t - fx) * (t - fy) * a + fx * (t - fy) * bb + (t - fx) * fy * cc + fx * fy * dd;
        v.div_euclid(t * t) as i32 + d
    }

    /// A global tile: its corners, water kind and river strength.
    pub fn tile(&mut self, gx: i32, gy: i32) -> TileInfo {
        let corners = [
            self.corner(gx, gy),
            self.corner(gx + 1, gy),
            self.corner(gx, gy + 1),
            self.corner(gx + 1, gy + 1),
        ];
        self.tile_with(gx, gy, corners)
    }

    fn tile_with(&mut self, gx: i32, gy: i32, corners: [i32; 4]) -> TileInfo {
        let sum: i64 = corners.iter().map(|&v| v as i64).sum();
        let avg = sum.div_euclid(4) as i32;
        let range = corners.iter().max().unwrap() - corners.iter().min().unwrap();
        let sea = self.sea();
        let (river, lake_level) = if self.fast {
            let s = field::sample(
                &self.p,
                field::tile_center_pos(gx),
                field::tile_center_pos(gy),
            );
            (s.river, NO_WATER)
        } else {
            let bx = gx.div_euclid(BASIN_TILES);
            let by = gy.div_euclid(BASIN_TILES);
            let b = self.basin(bx, by);
            // Tile centre in eighths of a node.
            let px = 2 * (gx - bx * BASIN_TILES) + 1;
            let py = 2 * (gy - by * BASIN_TILES) + 1;
            let e = 2 * TILES_PER_NODE;
            let (i, j) = (px / e, py / e);
            let (fx, fy) = ((px - i * e) as i64, (py - j * e) as i64);
            let e = e as i64;
            let r = |ii: i32, jj: i32| b.river[Basin::idx(ii, jj)] as i64;
            let v = (e - fx) * (e - fy) * r(i, j)
                + fx * (e - fy) * r(i + 1, j)
                + (e - fx) * fy * r(i, j + 1)
                + fx * fy * r(i + 1, j + 1);
            let river = (v / (e * e)) as i32;
            // Nearest node decides lakes.
            let ni = (px + e as i32 / 2) / e as i32;
            let nj = (py + e as i32 / 2) / e as i32;
            let k = Basin::idx(ni.min(CELLS), nj.min(CELLS));
            let lake = if b.flags[k] & LAKE != 0 {
                b.water[k]
            } else {
                NO_WATER
            };
            (river, lake)
        };
        let (water, water_level) = if avg < sea {
            (WaterKind::Sea, sea)
        } else if lake_level != NO_WATER && avg < lake_level {
            (WaterKind::Lake, lake_level)
        } else if river >= 128 {
            (WaterKind::River, avg + 300)
        } else {
            (WaterKind::Dry, NO_WATER)
        };
        TileInfo {
            corners,
            avg,
            range,
            river,
            water,
            water_level,
        }
    }

    /// A macro node in global node coordinates (1 node = 4 tiles).
    pub fn node(&mut self, nx: i32, ny: i32) -> NodeInfo {
        if self.fast {
            let s = field::sample(&self.p, nx as i64 * noise::ONE, ny as i64 * noise::ONE);
            let mut flags = 0;
            if s.h < self.sea() {
                flags |= SEA;
            } else if s.river >= 256 {
                flags |= RIVER;
            }
            return NodeInfo { alt: s.h, flags };
        }
        let bx = nx.div_euclid(CELLS);
        let by = ny.div_euclid(CELLS);
        let b = self.basin(bx, by);
        let k = Basin::idx(nx - bx * CELLS, ny - by * CELLS);
        NodeInfo {
            alt: b.alt[k],
            flags: b.flags[k],
        }
    }

    /// Everything about one region.
    pub fn region(&mut self, region: [i32; 2]) -> RegionData {
        let (ox, oy) = (region[0] * REGION_TILES, region[1] * REGION_TILES);
        if !self.fast {
            // Seat the region's own basin first so shared borders reuse it.
            self.basin(ox.div_euclid(BASIN_TILES), oy.div_euclid(BASIN_TILES));
        }
        let n = REGION_TILES + 1;
        let mut heights = vec![0i32; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                heights[corner_index(x, y)] = self.corner(ox + x, oy + y);
            }
        }
        let mut tiles = Vec::with_capacity((REGION_TILES * REGION_TILES) as usize);
        for y in 0..REGION_TILES {
            for x in 0..REGION_TILES {
                let corners = [
                    heights[corner_index(x, y)],
                    heights[corner_index(x + 1, y)],
                    heights[corner_index(x, y + 1)],
                    heights[corner_index(x + 1, y + 1)],
                ];
                tiles.push(self.tile_with(ox + x, oy + y, corners));
            }
        }
        let surface = tiles.iter().enumerate().map(|(k, t)| {
            let gx = ox + (k as i32 % REGION_TILES);
            let gy = oy + (k as i32 / REGION_TILES);
            self.surface_of(gx, gy, t)
        });
        let surface: Vec<Surface> = surface.collect();
        RegionData {
            region,
            heights,
            tiles,
            surface,
        }
    }

    /// The natural-surface rules for one tile.
    pub fn surface_of(&self, gx: i32, gy: i32, t: &TileInfo) -> Surface {
        let p = &self.p;
        if t.water != WaterKind::Dry {
            return Surface::Water;
        }
        let sea = p.sea as i32;
        let above = t.avg as i64 - p.sea;
        // Steep: more than 80 % across a tile.
        if t.range > 1600 {
            return Surface::Rock;
        }
        let high = p.hill + p.mountain * 6 / 10;
        if p.mountain > 0 && above > high && t.range > 600 {
            return Surface::Rock;
        }
        if t.river >= 64 || t.avg < sea + 1200 {
            return Surface::Sand;
        }
        let fnoise = field::forest_noise(p, gx, gy);
        if p.climate == Climate::Arid && fnoise > 820 {
            return Surface::Sand;
        }
        let mut limit = p.forest + p.forest_bias;
        if above > high {
            limit -= 300;
        }
        limit -= (t.range as i64 / 4).min(300);
        if fnoise < limit {
            return Surface::Forest;
        }
        Surface::Grass
    }
}

/// A region with everything the public functions need.
pub struct RegionData {
    pub region: [i32; 2],
    pub heights: Vec<i32>,
    pub tiles: Vec<TileInfo>,
    pub surface: Vec<Surface>,
}

/// 65×65 corner heights (mm) of a region, deterministic in (charter, region).
pub fn region_corner_heights(c: &Charter, region: [i32; 2]) -> Vec<i32> {
    Sampler::new(c, false).region(region).heights
}

/// The same, in the fast "valleys" mode: rivers from the zero lines of a
/// noise field carved as valleys, no basin and no erosion. For previews.
pub fn region_corner_heights_fast(c: &Charter, region: [i32; 2]) -> Vec<i32> {
    Sampler::new(c, true).region(region).heights
}

/// Natural surface (grass, forest, sand, rock, water) of a region before the
/// city is laid over it. 64×64.
pub fn region_natural_surface(c: &Charter, region: [i32; 2]) -> Vec<Surface> {
    Sampler::new(c, false).region(region).surface
}

/// The natural surface in the fast mode.
pub fn region_natural_surface_fast(c: &Charter, region: [i32; 2]) -> Vec<Surface> {
    Sampler::new(c, true).region(region).surface
}

/// Water surface per tile (mm, 64×64), `i32::MIN` where dry. Lakes and sea
/// are flat; rivers sit 30 cm above the tile's mean ground.
pub fn region_water_levels(c: &Charter, region: [i32; 2]) -> Vec<i32> {
    Sampler::new(c, false)
        .region(region)
        .tiles
        .iter()
        .map(|t| t.water_level)
        .collect()
}

/// Sea level of the charter, mm (heights are absolute, sea included).
pub fn sea_level_mm(c: &Charter) -> i32 {
    c.relief.sea_level_mm
}

/// Maximum slope (per mille) the city flattens under lots. Metadata for the
/// solver, which does the flattening with [`flatten_rect`].
pub fn city_flatten_slope(c: &Charter) -> u16 {
    c.relief.city_flatten_slope
}

/// Flatten the corners of a lot to `target_mm`. `heights` is a 65×65 region
/// corner grid; `rect` is x, y, w, h in region tiles (clipped to the region).
pub fn flatten_rect(heights: &mut [i32], rect: [i32; 4], target_mm: i32) {
    let n = REGION_TILES + 1;
    if heights.len() < (n * n) as usize {
        return;
    }
    let [x, y, w, h] = rect;
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + w).min(REGION_TILES), (y + h).min(REGION_TILES));
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            heights[corner_index(cx, cy)] = target_mm;
        }
    }
}

/// Short text + numbers about the land around `center`, for the founding LLM.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct TerrainSummary {
    pub center: [i32; 2],
    pub radius: i32,
    /// Per region in the square: [rx, ry, min_mm, max_mm, water_per_mille, forest_per_mille].
    pub regions: Vec<[i32; 6]>,
    pub notes: Vec<String>,
}

pub fn summary(c: &Charter, center: [i32; 2], radius: i32) -> TerrainSummary {
    let radius = radius.clamp(0, 6);
    let mut s = Sampler::new(c, false);
    let sea = s.sea();
    let mut regions = Vec::new();
    let mut notes = Vec::new();
    // Sea tiles and all tiles per side of the centre region: N, E, S, W.
    let mut side_sea = [0i64; 4];
    let mut side_all = [0i64; 4];
    let mut highest: Option<(i32, [i32; 2])> = None;
    let mut flat: Vec<(i32, i32, [i32; 2])> = Vec::new();
    let (ccx, ccy) = (center[0] * REGION_TILES + 32, center[1] * REGION_TILES + 32);

    for ry in center[1] - radius..=center[1] + radius {
        for rx in center[0] - radius..=center[0] + radius {
            let d = s.region([rx, ry]);
            let mn = *d.heights.iter().min().unwrap();
            let mx = *d.heights.iter().max().unwrap();
            let total = d.tiles.len() as i64;
            let water = d.tiles.iter().filter(|t| t.water != WaterKind::Dry).count() as i64;
            let forest = d.surface.iter().filter(|&&x| x == Surface::Forest).count() as i64;
            let lake = d
                .tiles
                .iter()
                .filter(|t| t.water == WaterKind::Lake)
                .count() as i64;
            let wpm = (water * 1000 / total) as i32;
            let fpm = (forest * 1000 / total) as i32;
            regions.push([rx, ry, mn, mx, wpm, fpm]);

            // Rivers touching the edges.
            let edge = |x: i32, y: i32| d.tiles[tile_index(x, y)].water == WaterKind::River;
            let n = (0..REGION_TILES).any(|x| edge(x, 0));
            let so = (0..REGION_TILES).any(|x| edge(x, REGION_TILES - 1));
            let w = (0..REGION_TILES).any(|y| edge(0, y));
            let e = (0..REGION_TILES).any(|y| edge(REGION_TILES - 1, y));
            if n && so {
                notes.push(format!("river crosses region ({},{}) north-south", rx, ry));
            } else if e && w {
                notes.push(format!("river crosses region ({},{}) east-west", rx, ry));
            } else {
                let sides: Vec<&str> = [(n, "north"), (e, "east"), (so, "south"), (w, "west")]
                    .iter()
                    .filter(|x| x.0)
                    .map(|x| x.1)
                    .collect();
                if sides.len() == 2 {
                    notes.push(format!(
                        "river bends through region ({},{}) from {} to {}",
                        rx, ry, sides[0], sides[1]
                    ));
                }
            }
            if lake * 1000 / total >= 30 {
                notes.push(format!(
                    "lake in region ({},{}), {}% of it",
                    rx,
                    ry,
                    lake * 100 / total
                ));
            }

            for (k, t) in d.tiles.iter().enumerate() {
                let gx = rx * REGION_TILES + (k as i32 % REGION_TILES);
                let gy = ry * REGION_TILES + (k as i32 / REGION_TILES);
                let (dx, dy) = (gx - ccx, gy - ccy);
                if dx == 0 && dy == 0 {
                    continue;
                }
                let side = if dy.abs() >= dx.abs() {
                    if dy < 0 {
                        0
                    } else {
                        2
                    }
                } else if dx > 0 {
                    1
                } else {
                    3
                };
                side_all[side] += 1;
                if t.water == WaterKind::Sea {
                    side_sea[side] += 1;
                }
            }
            if highest.is_none_or(|h| mx > h.0) {
                highest = Some((mx, [rx, ry]));
            }
            if wpm < 100 {
                flat.push((mx - mn, fpm, [rx, ry]));
            }
        }
    }
    let names = ["north", "east", "south", "west"];
    for k in 0..4 {
        if side_all[k] > 0 && side_sea[k] * 4 >= side_all[k] {
            notes.push(format!(
                "coast to the {} (sea covers {}% of that side)",
                names[k],
                side_sea[k] * 100 / side_all[k]
            ));
        }
    }
    if let Some(r) = regions
        .iter()
        .find(|r| r[0] == center[0] && r[1] == center[1])
    {
        if r[4] > 500 {
            notes.push("the center region is mostly water".to_string());
        }
    }
    if let Some((h, r)) = highest {
        notes.push(format!(
            "highest ground in region ({},{}): {} m above sea",
            r[0],
            r[1],
            (h - sea) / 1000
        ));
    }
    flat.sort_by_key(|f| (f.0, f.2[1], f.2[0]));
    for (range, fpm, r) in flat.iter().take(3) {
        notes.push(format!(
            "flat dry land in region ({},{}): relief {} m, forest {}%",
            r[0],
            r[1],
            range / 1000,
            fpm / 10
        ));
    }
    if regions.iter().all(|r| r[4] >= 900) {
        notes.push("no dry land here: pick another center".to_string());
    }
    TerrainSummary {
        center,
        radius,
        regions,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charter::example;
    use crate::hash::canonical_hash;

    #[test]
    fn same_input_same_hash() {
        let c = example("t", "semente-1");
        let a = canonical_hash(&(
            region_corner_heights(&c, [0, 0]),
            region_natural_surface(&c, [0, 0]),
        ));
        basin::clear_cache();
        let b = canonical_hash(&(
            region_corner_heights(&c, [0, 0]),
            region_natural_surface(&c, [0, 0]),
        ));
        assert_eq!(a, b);
        let f1 = canonical_hash(&region_corner_heights_fast(&c, [3, -2]));
        let f2 = canonical_hash(&region_corner_heights_fast(&c, [3, -2]));
        assert_eq!(f1, f2);
    }

    #[test]
    fn neighbour_edges_match() {
        let c = example("t", "semente-2");
        // Inside one basin, and across the basin border at x = 1024 tiles.
        for (a, b) in [([2, 3], [3, 3]), ([15, 0], [16, 0])] {
            let ha = region_corner_heights(&c, a);
            let hb = region_corner_heights(&c, b);
            for y in 0..=REGION_TILES {
                assert_eq!(
                    ha[corner_index(REGION_TILES, y)],
                    hb[corner_index(0, y)],
                    "{:?}|{:?} y={}",
                    a,
                    b,
                    y
                );
            }
        }
        let ha = region_corner_heights(&c, [0, 15]);
        let hb = region_corner_heights(&c, [0, 16]);
        for x in 0..=REGION_TILES {
            assert_eq!(ha[corner_index(x, REGION_TILES)], hb[corner_index(x, 0)]);
        }
        let fa = region_corner_heights_fast(&c, [4, 4]);
        let fb = region_corner_heights_fast(&c, [4, 5]);
        for x in 0..=REGION_TILES {
            assert_eq!(fa[corner_index(x, REGION_TILES)], fb[corner_index(x, 0)]);
        }
    }

    #[test]
    fn flatten_sets_corners() {
        let mut h = vec![5; 65 * 65];
        flatten_rect(&mut h, [2, 2, 3, 2], 100);
        assert_eq!(h[corner_index(2, 2)], 100);
        assert_eq!(h[corner_index(5, 4)], 100);
        assert_eq!(h[corner_index(6, 4)], 5);
    }

    /// `cargo test -p omniget-citygen --release basin_timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn basin_timing() {
        let c = example("t", "bench");
        let p = Params::from_charter(&c);
        let t = std::time::Instant::now();
        let b = basin::generate(&p, 0, 0);
        let el = t.elapsed();
        let rivers = b.flags.iter().filter(|&&f| f & RIVER != 0).count();
        let lakes = b.flags.iter().filter(|&&f| f & LAKE != 0).count();
        let sea = b.flags.iter().filter(|&&f| f & SEA != 0).count();
        println!(
            "basin 257² in {:?}; river nodes {rivers}, lake {lakes}, sea {sea}",
            el
        );
        let s = summary(&c, [8, 8], 2);
        for n in &s.notes {
            println!("{n}");
        }
        assert!(el.as_secs() < 5);
    }
}
