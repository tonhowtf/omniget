//! Basin hydrology: a finite 1024×1024-tile square (257×257 macro nodes of
//! 4 tiles) eroded with the implicit stream-power law, then drained into
//! rivers, lakes and sea.
//!
//! Portions derived from Veloren (gitlab.com/veloren/veloren),
//! GPL-3.0-or-later, © Veloren contributors: the implicit stream-power update
//! (`world/src/sim/erosion.rs`, `erode`, following Braun & Willett 2013 and
//! Cordonnier et al. 2016), drainage accumulation in reverse height order
//! (`get_drainage`) and the lake/river read-out (`get_lakes`, `get_rivers`),
//! at commit b68cd6ea. Rewritten here in integer fixed point with m = 0.5,
//! n = 1, no sediment, a Barnes priority-flood instead of Planchon-Darboux and
//! an explicit hillslope diffusion instead of ADI.
//!
//! The border ring is pinned to the pure relief function, so two basins that
//! share an edge see exactly the same heights on it.

use super::field::{self, Params, TILES_PER_NODE};
use super::noise::ONE;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};

/// Macro cells per basin side; nodes per side = CELLS + 1.
pub const CELLS: i32 = 256;
pub const NODES: i32 = CELLS + 1;
pub const BASIN_TILES: i32 = CELLS * TILES_PER_NODE;

pub const RIVER: u8 = 1;
pub const LAKE: u8 = 2;
pub const SEA: u8 = 4;
pub const COAST: u8 = 8;

pub const NO_WATER: i32 = i32::MIN;
const NONE: u32 = u32::MAX;

/// D8 neighbours: N, E, S, W, NE, SE, SW, NW.
pub const D8: [(i32, i32); 8] = [
    (0, -1),
    (1, 0),
    (0, 1),
    (-1, 0),
    (1, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
];

#[derive(Debug)]
pub struct Basin {
    pub bx: i32,
    pub by: i32,
    /// Terrain height per node, mm.
    pub alt: Vec<i32>,
    /// Water surface per node, mm, or [`NO_WATER`].
    pub water: Vec<i32>,
    /// River strength per node: 0 dry, 256 edge, 512 wide river.
    pub river: Vec<u16>,
    pub flags: Vec<u8>,
    /// Drainage area in nodes.
    pub area: Vec<u32>,
    /// D8 index of the receiver, 255 for outlets.
    pub downhill: Vec<u8>,
}

impl Basin {
    #[inline]
    pub fn idx(i: i32, j: i32) -> usize {
        (j * NODES + i) as usize
    }
}

/// Reused buffers of the flood / accumulation passes.
struct Work {
    filled: Vec<i64>,
    seen: Vec<bool>,
    rec: Vec<u32>,
    order: Vec<u32>,
    area: Vec<u32>,
    heap: BinaryHeap<Reverse<(i64, u32)>>,
}

impl Work {
    fn new(n: usize) -> Work {
        Work {
            filled: vec![0; n],
            seen: vec![false; n],
            rec: vec![NONE; n],
            order: Vec::with_capacity(n),
            area: vec![1; n],
            heap: BinaryHeap::with_capacity(4 * NODES as usize),
        }
    }

    /// Priority-flood with epsilon (Barnes et al. 2014): every node gets a
    /// receiver that drains to the border or to the sea; `order` is the pop
    /// order (outlets first), ties broken by index.
    fn flood(&mut self, h: &[i64], sea: i64) {
        let n = NODES;
        self.order.clear();
        self.heap.clear();
        for v in self.seen.iter_mut() {
            *v = false;
        }
        for j in 0..n {
            for i in 0..n {
                let k = Basin::idx(i, j);
                let border = i == 0 || j == 0 || i == n - 1 || j == n - 1;
                if border || h[k] < sea {
                    self.seen[k] = true;
                    self.filled[k] = h[k];
                    self.rec[k] = NONE;
                    self.heap.push(Reverse((h[k], k as u32)));
                }
            }
        }
        while let Some(Reverse((hh, k))) = self.heap.pop() {
            self.order.push(k);
            let (x, y) = ((k as i32) % n, (k as i32) / n);
            for (dx, dy) in D8 {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= n || ny >= n {
                    continue;
                }
                let j = Basin::idx(nx, ny);
                if self.seen[j] {
                    continue;
                }
                self.seen[j] = true;
                self.filled[j] = h[j].max(hh + 1);
                self.rec[j] = k;
                self.heap.push(Reverse((self.filled[j], j as u32)));
            }
        }
    }

    /// Drainage area: each node adds itself to its receiver, highest first.
    fn accumulate(&mut self) {
        for a in self.area.iter_mut() {
            *a = 1;
        }
        for &k in self.order.iter().rev() {
            let r = self.rec[k as usize];
            if r != NONE {
                self.area[r as usize] = self.area[r as usize].saturating_add(self.area[k as usize]);
            }
        }
    }
}

#[inline]
fn is_diag(a: u32, b: u32) -> bool {
    let n = NODES as u32;
    a % n != b % n && a / n != b / n
}

/// Generate one basin. Pure in (params, bx, by).
pub fn generate(p: &Params, bx: i32, by: i32) -> Basin {
    let n = NODES;
    let len = (n * n) as usize;
    let mut h = vec![0i64; len];
    let mut valley = vec![0u16; len];
    for j in 0..n {
        for i in 0..n {
            let gx = (bx * CELLS + i) as i64 * ONE;
            let gy = (by * CELLS + j) as i64 * ONE;
            let s = field::sample(p, gx, gy);
            let k = Basin::idx(i, j);
            h[k] = s.h as i64;
            valley[k] = s.river.clamp(0, 512) as u16;
        }
    }
    let pinned = |k: usize| {
        let (x, y) = ((k as i32) % n, (k as i32) / n);
        x == 0 || y == 0 || x == n - 1 || y == n - 1
    };

    let mut w = Work::new(len);
    let iters = (p.erosion * 40 / 1000) as usize;
    // Erodibility per sqrt(area), Q16.
    let k_eff: i64 = 700 + p.erosion * 6 / 10;
    let mut tmp = vec![0i64; len];
    for it in 0..iters {
        w.flood(&h, p.sea);
        w.accumulate();
        for idx in 0..w.order.len() {
            let k = w.order[idx] as usize;
            let r = w.rec[k];
            if r == NONE || pinned(k) || w.filled[k] > h[k] {
                continue;
            }
            let r = r as usize;
            // Land never erodes below the sea it drains into.
            let target = if w.filled[r] > h[r] {
                w.filled[r]
            } else {
                h[r]
            }
            .max(p.sea);
            let sa = (w.area[k] as u64).isqrt() as i64;
            let mut f = k_eff * sa;
            if is_diag(k as u32, r as u32) {
                f = (f * 46341) >> 16;
            }
            if h[k] > target {
                h[k] = (h[k] * ONE + f * target) / (ONE + f);
            }
        }
        // Hillslope diffusion every 4th step, explicit and gentle.
        if it % 4 == 3 {
            tmp.copy_from_slice(&h);
            for j in 1..n - 1 {
                for i in 1..n - 1 {
                    let k = Basin::idx(i, j);
                    if tmp[k] < p.sea {
                        continue;
                    }
                    let s = tmp[k - 1] + tmp[k + 1] + tmp[k - n as usize] + tmp[k + n as usize];
                    h[k] = tmp[k] + (s - 4 * tmp[k]) / 32;
                }
            }
        }
    }

    // Read-out: final drainage on the eroded surface.
    w.flood(&h, p.sea);
    w.accumulate();

    let mut flags = vec![0u8; len];
    let mut water = vec![NO_WATER; len];
    let mut river = vec![0u16; len];
    let mut downhill = vec![255u8; len];
    let thr: u32 = if p.rivers == 0 {
        u32::MAX
    } else {
        (3000 - p.rivers * 26 / 10).max(200) as u32
    };

    for k in 0..len {
        let r = w.rec[k];
        if r != NONE {
            let (x, y) = ((k as i32) % n, (k as i32) / n);
            let (rx, ry) = ((r as i32) % n, (r as i32) / n);
            downhill[k] = D8
                .iter()
                .position(|&d| d == (rx - x, ry - y))
                .map(|d| d as u8)
                .unwrap_or(255);
        }
        if h[k] < p.sea {
            flags[k] |= SEA;
            water[k] = p.sea as i32;
        } else if w.filled[k] - h[k] >= 400 {
            flags[k] |= LAKE;
            water[k] = w.filled[k] as i32;
        }
    }
    // Rivers by flow accumulation, walked outlets-last so thickening is stable.
    for &k in w.order.iter() {
        let k = k as usize;
        if flags[k] & (SEA | LAKE) != 0 {
            continue;
        }
        let a = w.area[k];
        let mut s = valley[k];
        if a >= thr {
            let extra = ((a / thr) as u64).isqrt().saturating_sub(1).min(4) as u16 * 64;
            s = s.max(256 + extra);
        }
        if s >= 256 {
            river[k] = river[k].max(s);
            // A diagonal step would leave the water touching only at a
            // corner: widen it with one orthogonal node.
            let r = w.rec[k];
            if r != NONE && is_diag(k as u32, r) {
                let y = (k as i32) / n;
                let rx = (r as i32) % n;
                let side = Basin::idx(rx, y);
                if flags[side] & (SEA | LAKE) == 0 {
                    river[side] = river[side].max(s);
                }
            }
        }
    }
    let mut alt = vec![0i32; len];
    for k in 0..len {
        let mut a = h[k];
        if river[k] >= 256 {
            flags[k] |= RIVER;
            let depth = 500 + (river[k] as i64 - 256) * 4;
            if !pinned(k) {
                a = (a - depth).max(p.sea - 300);
            }
            water[k] = (a + depth - 200).max(a) as i32;
        }
        alt[k] = a.clamp(i32::MIN as i64 / 2, i32::MAX as i64 / 2) as i32;
    }
    for j in 0..n {
        for i in 0..n {
            let k = Basin::idx(i, j);
            if flags[k] & SEA != 0 {
                continue;
            }
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (x, y) = (i + dx, j + dy);
                if x >= 0 && y >= 0 && x < n && y < n && flags[Basin::idx(x, y)] & SEA != 0 {
                    flags[k] |= COAST;
                }
            }
        }
    }
    Basin {
        bx,
        by,
        alt,
        water,
        river,
        flags,
        area: w.area,
        downhill,
    }
}

// ── Cache ───────────────────────────────────────────────────────────────────

const CACHE_CAP: usize = 24;
type Entry = (String, i32, i32, Arc<Basin>);
static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// The basin (bx, by) for these params, from a small in-memory LRU.
pub fn get(p: &Params, bx: i32, by: i32) -> Arc<Basin> {
    {
        let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = c
            .iter()
            .position(|e| e.1 == bx && e.2 == by && e.0 == p.key)
        {
            let e = c.remove(pos);
            let b = e.3.clone();
            c.push(e);
            return b;
        }
    }
    let b = Arc::new(generate(p, bx, by));
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if !c.iter().any(|e| e.1 == bx && e.2 == by && e.0 == p.key) {
        if c.len() >= CACHE_CAP {
            c.remove(0);
        }
        c.push((p.key.clone(), bx, by, b.clone()));
    }
    b
}

/// Clear the cache (tests and benchmarks).
pub fn clear_cache() {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).clear();
}
