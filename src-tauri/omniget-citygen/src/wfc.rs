//! Decoration-only WFC in 16×16 windows with fixed borders. Owner: agent C.
//!
//! Estudo 99, arquivos 07 §3 and 09 §5.3: the WFC never decides structure
//! (streets, lots, masses, doors come from the solver); it only picks between
//! choices the building code sees as equivalent (curb pieces, fences, flower
//! beds, ridge tiles). So:
//! - the grid is solved in 16×16 windows, in a fixed order (x, then y); the
//!   cells of windows already solved are fixed borders for the next ones;
//! - inside a window the next cell is the one with the fewest options (ties:
//!   lowest row-major index), and the tile is a weighted pick from
//!   `sha256(seed|wfc|salt:x:y:n)`: integers only, no float, no global RNG;
//! - a contradiction backtracks (bounded by [`MAX_BACKTRACKS`] per window);
//!   past that the window falls back to the tileset's default tile, silently,
//!   like Townscaper does.

use crate::hash::pick;
use std::collections::VecDeque;

pub const WINDOW: i32 = 16;
pub const MAX_BACKTRACKS: u32 = 64;
/// At most 32 tiles per tileset (domains are `u32` bitsets).
pub const MAX_TILES: usize = 32;

/// Tiles, weights and adjacency. `east[a]` is the set of tiles allowed
/// directly east of `a`; `south[a]` the set allowed directly south of `a`.
#[derive(Clone, Debug)]
pub struct Tileset {
    pub names: Vec<&'static str>,
    pub weights: Vec<u32>,
    pub east: Vec<u32>,
    pub south: Vec<u32>,
    west: Vec<u32>,
    north: Vec<u32>,
    /// Tile used when a window cannot be solved.
    pub default: u8,
}

impl Tileset {
    /// Every pair is compatible in every direction except `forbidden`
    /// (symmetric: `(a, b)` forbids `b` next to `a` on any side, and vice versa).
    pub fn symmetric(
        names: &[&'static str],
        weights: &[u32],
        forbidden: &[(&str, &str)],
        default: &str,
    ) -> Tileset {
        assert!(names.len() <= MAX_TILES && names.len() == weights.len());
        let n = names.len();
        let idx = |s: &str| {
            names
                .iter()
                .position(|x| *x == s)
                .unwrap_or_else(|| panic!("unknown tile {s}"))
        };
        let all = if n == 32 { u32::MAX } else { (1u32 << n) - 1 };
        let mut compat = vec![all; n];
        for (a, b) in forbidden {
            let (ia, ib) = (idx(a), idx(b));
            compat[ia] &= !(1 << ib);
            compat[ib] &= !(1 << ia);
        }
        Tileset::directional(names, weights, compat.clone(), compat, idx(default) as u8)
    }

    pub fn directional(
        names: &[&'static str],
        weights: &[u32],
        east: Vec<u32>,
        south: Vec<u32>,
        default: u8,
    ) -> Tileset {
        let n = names.len();
        let mut west = vec![0u32; n];
        let mut north = vec![0u32; n];
        for a in 0..n {
            for b in 0..n {
                if east[a] & (1 << b) != 0 {
                    west[b] |= 1 << a;
                }
                if south[a] & (1 << b) != 0 {
                    north[b] |= 1 << a;
                }
            }
        }
        Tileset {
            names: names.to_vec(),
            weights: weights.to_vec(),
            east,
            south,
            west,
            north,
            default,
        }
    }

    pub fn index(&self, name: &str) -> Option<u8> {
        self.names.iter().position(|x| *x == name).map(|i| i as u8)
    }

    pub fn bit(&self, name: &str) -> u32 {
        self.index(name).map(|i| 1u32 << i).unwrap_or(0)
    }

    pub fn full(&self) -> u32 {
        if self.names.len() == 32 {
            u32::MAX
        } else {
            (1u32 << self.names.len()) - 1
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// One tile index per cell, row-major (`y * w + x`).
    pub tiles: Vec<u8>,
    pub windows: u32,
    pub backtracks: u32,
    /// Windows that fell back to the default tile.
    pub fallbacks: u32,
}

/// Solve a `w × h` grid. `domain[i]` is the initial set of allowed tiles of
/// each cell (0 is read as "only the default tile").
pub fn solve(ts: &Tileset, w: i32, h: i32, domain: &[u32], seed: &str, salt: &str) -> Outcome {
    assert_eq!(domain.len(), (w * h) as usize);
    let full = ts.full();
    let mut out = Outcome {
        tiles: vec![ts.default; (w * h) as usize],
        ..Default::default()
    };
    let mut solved = vec![false; (w * h) as usize];
    let mut wy = 0;
    while wy < h {
        let mut wx = 0;
        while wx < w {
            let ww = WINDOW.min(w - wx);
            let wh = WINDOW.min(h - wy);
            solve_window(
                ts,
                full,
                w,
                h,
                [wx, wy, ww, wh],
                domain,
                &mut out,
                &mut solved,
                seed,
                salt,
            );
            out.windows += 1;
            wx += WINDOW;
        }
        wy += WINDOW;
    }
    out
}

fn popcount(v: u32) -> u32 {
    v.count_ones()
}

fn lowest(v: u32) -> u8 {
    v.trailing_zeros() as u8
}

#[allow(clippy::too_many_arguments)]
fn solve_window(
    ts: &Tileset,
    full: u32,
    w: i32,
    h: i32,
    win: [i32; 4],
    domain: &[u32],
    out: &mut Outcome,
    solved: &mut [bool],
    seed: &str,
    salt: &str,
) {
    let [x0, y0, ww, wh] = win;
    let n = (ww * wh) as usize;
    let gidx = |lx: i32, ly: i32| ((y0 + ly) * w + (x0 + lx)) as usize;
    let dflt = 1u32 << ts.default;
    let mut d = vec![0u32; n];
    for ly in 0..wh {
        for lx in 0..ww {
            let (gx, gy) = (x0 + lx, y0 + ly);
            let mut v = domain[gidx(lx, ly)] & full;
            if v == 0 {
                v = dflt;
            }
            // Fixed borders: cells of windows solved earlier.
            let fixed = |x: i32, y: i32| -> Option<u8> {
                if x < 0 || y < 0 || x >= w || y >= h {
                    return None;
                }
                let i = (y * w + x) as usize;
                if solved[i] {
                    Some(out.tiles[i])
                } else {
                    None
                }
            };
            if lx == 0 {
                if let Some(t) = fixed(gx - 1, gy) {
                    v &= ts.east[t as usize];
                }
            }
            if ly == 0 {
                if let Some(t) = fixed(gx, gy - 1) {
                    v &= ts.south[t as usize];
                }
            }
            if lx == ww - 1 {
                if let Some(t) = fixed(gx + 1, gy) {
                    v &= ts.west[t as usize];
                }
            }
            if ly == wh - 1 {
                if let Some(t) = fixed(gx, gy + 1) {
                    v &= ts.north[t as usize];
                }
            }
            if v == 0 {
                v = dflt;
            }
            d[(ly * ww + lx) as usize] = v;
        }
    }
    let initial = d.clone();

    let neighbours = |i: usize| -> [(Option<usize>, u8); 4] {
        let (lx, ly) = ((i as i32) % ww, (i as i32) / ww);
        [
            (if lx + 1 < ww { Some(i + 1) } else { None }, 0),
            (
                if ly + 1 < wh {
                    Some(i + ww as usize)
                } else {
                    None
                },
                1,
            ),
            (if lx > 0 { Some(i - 1) } else { None }, 2),
            (if ly > 0 { Some(i - ww as usize) } else { None }, 3),
        ]
    };
    let propagate = |d: &mut Vec<u32>, start: &[usize]| -> bool {
        let mut q: VecDeque<usize> = start.iter().copied().collect();
        while let Some(i) = q.pop_front() {
            for (nb, dir) in neighbours(i) {
                let Some(j) = nb else { continue };
                let table = match dir {
                    0 => &ts.east,
                    1 => &ts.south,
                    2 => &ts.west,
                    _ => &ts.north,
                };
                let mut allowed = 0u32;
                let mut s = d[i];
                while s != 0 {
                    let t = s.trailing_zeros() as usize;
                    allowed |= table[t];
                    s &= s - 1;
                }
                let nd = d[j] & allowed;
                if nd != d[j] {
                    if nd == 0 {
                        return false;
                    }
                    d[j] = nd;
                    q.push_back(j);
                }
            }
        }
        true
    };

    let fallback = |out: &mut Outcome, solved: &mut [bool]| {
        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            let (lx, ly) = ((i as i32) % ww, (i as i32) / ww);
            let v = initial[i];
            let t = if popcount(v) == 1 {
                lowest(v)
            } else if v & dflt != 0 {
                ts.default
            } else {
                lowest(v)
            };
            let g = gidx(lx, ly);
            out.tiles[g] = t;
            solved[g] = true;
        }
        out.fallbacks += 1;
    };

    let all: Vec<usize> = (0..n).collect();
    if !propagate(&mut d, &all) {
        fallback(out, solved);
        return;
    }
    let mut stack: Vec<(Vec<u32>, usize, u8)> = Vec::new();
    let mut backtracks = 0u32;
    let mut step = 0u64;
    loop {
        // Fewest options first; ties by index (fixed order).
        let mut best: Option<(u32, usize)> = None;
        for (i, v) in d.iter().enumerate() {
            let c = popcount(*v);
            if c > 1 && best.map(|b| c < b.0).unwrap_or(true) {
                best = Some((c, i));
            }
        }
        let Some((_, cell)) = best else { break };
        let (lx, ly) = ((cell as i32) % ww, (cell as i32) / ww);
        let tile = weighted(
            ts,
            d[cell],
            seed,
            &format!("{salt}:{}:{}:{step}", x0 + lx, y0 + ly),
        );
        step += 1;
        stack.push((d.clone(), cell, tile));
        d[cell] = 1 << tile;
        if propagate(&mut d, &[cell]) {
            continue;
        }
        // Contradiction: undo the last choices, banning each one.
        let mut ok = false;
        while let Some((snap, c, t)) = stack.pop() {
            backtracks += 1;
            if backtracks > MAX_BACKTRACKS {
                break;
            }
            d = snap;
            d[c] &= !(1u32 << t);
            if d[c] != 0 && propagate(&mut d, &[c]) {
                ok = true;
                break;
            }
        }
        out.backtracks += backtracks.min(MAX_BACKTRACKS);
        if !ok {
            fallback(out, solved);
            return;
        }
    }
    #[allow(clippy::needless_range_loop)]
    for i in 0..n {
        let (lx, ly) = ((i as i32) % ww, (i as i32) / ww);
        let g = gidx(lx, ly);
        out.tiles[g] = lowest(d[i]);
        solved[g] = true;
    }
}

fn weighted(ts: &Tileset, set: u32, seed: &str, id: &str) -> u8 {
    let mut total: u64 = 0;
    let mut s = set;
    while s != 0 {
        let t = s.trailing_zeros() as usize;
        total += ts.weights[t] as u64;
        s &= s - 1;
    }
    if total == 0 {
        return lowest(set);
    }
    let mut r = pick(seed, "wfc", id, 0, total);
    let mut s = set;
    while s != 0 {
        let t = s.trailing_zeros() as usize;
        let wgt = ts.weights[t] as u64;
        if r < wgt {
            return t as u8;
        }
        r -= wgt;
        s &= s - 1;
    }
    lowest(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wfc_is_deterministic_and_respects_rules() {
        let ts = Tileset::symmetric(
            &["nada", "a", "b", "c"],
            &[1, 5, 3, 2],
            &[("b", "b"), ("c", "c"), ("b", "c")],
            "a",
        );
        let (w, h) = (40, 23);
        let dom: Vec<u32> = (0..w * h)
            .map(|i| {
                if i % 7 == 0 {
                    ts.bit("nada")
                } else {
                    ts.bit("a") | ts.bit("b") | ts.bit("c")
                }
            })
            .collect();
        let r1 = solve(&ts, w, h, &dom, "seed", "t");
        let r2 = solve(&ts, w, h, &dom, "seed", "t");
        assert_eq!(r1, r2);
        let b = ts.index("b").unwrap();
        let c = ts.index("c").unwrap();
        let bad = |t: u8| t == b || t == c;
        for y in 0..h {
            for x in 0..w {
                let t = r1.tiles[(y * w + x) as usize];
                if x + 1 < w && r1.fallbacks == 0 {
                    assert!(!(bad(t) && bad(r1.tiles[(y * w + x + 1) as usize])));
                }
                if y + 1 < h && r1.fallbacks == 0 {
                    assert!(!(bad(t) && bad(r1.tiles[((y + 1) * w + x) as usize])));
                }
            }
        }
    }
}
