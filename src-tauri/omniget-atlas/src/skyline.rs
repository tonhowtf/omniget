//! Own skyline bin packer (bottom-left, best-fit on the lowest skyline).
//!
//! No packing crate: the atlas format is ours and the packer has to be
//! deterministic byte for byte, which means the placement order is part of the
//! contract, not an implementation detail of somebody else's heuristic.

/// One horizontal run of the skyline: everything at or below `y` in
/// `x..x + w` is occupied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Node {
    x: u32,
    y: u32,
    w: u32,
}

#[derive(Debug, Clone)]
pub struct Skyline {
    width: u32,
    height: u32,
    nodes: Vec<Node>,
    used: u64,
}

impl Skyline {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            nodes: vec![Node {
                x: 0,
                y: 0,
                w: width,
            }],
            used: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixels actually covered by inserted rectangles.
    pub fn used_area(&self) -> u64 {
        self.used
    }

    /// Lowest `y` reached by the skyline; the page can be cropped to it.
    pub fn extent_y(&self) -> u32 {
        self.nodes.iter().map(|n| n.y).max().unwrap_or(0)
    }

    /// Rightmost `x` that carries anything.
    pub fn extent_x(&self) -> u32 {
        self.nodes
            .iter()
            .filter(|n| n.y > 0)
            .map(|n| n.x + n.w)
            .max()
            .unwrap_or(0)
    }

    /// Places a `w x h` rectangle, returning its top-left corner.
    ///
    /// Picks the candidate with the lowest top edge, breaking ties by the
    /// leftmost `x`, so the same input always produces the same page.
    pub fn insert(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w == 0 || h == 0 || w > self.width || h > self.height {
            return None;
        }
        let mut best: Option<(u32, u32, usize)> = None; // (y, x, node index)
        for i in 0..self.nodes.len() {
            if let Some(y) = self.fit(i, w) {
                if y + h > self.height {
                    continue;
                }
                let x = self.nodes[i].x;
                let better = match best {
                    None => true,
                    Some((by, bx, _)) => (y, x) < (by, bx),
                };
                if better {
                    best = Some((y, x, i));
                }
            }
        }
        let (y, x, index) = best?;
        self.add_level(index, x, y, w, h);
        self.used += u64::from(w) * u64::from(h);
        Some((x, y))
    }

    /// Top edge a `w`-wide rectangle would sit at if left-aligned with node `i`,
    /// or `None` when it runs off the right edge.
    fn fit(&self, i: usize, w: u32) -> Option<u32> {
        let x = self.nodes[i].x;
        if x + w > self.width {
            return None;
        }
        let mut remaining = w;
        let mut y = 0;
        let mut j = i;
        while remaining > 0 {
            let node = self.nodes.get(j)?;
            y = y.max(node.y);
            remaining = remaining.saturating_sub(node.w);
            j += 1;
        }
        Some(y)
    }

    fn add_level(&mut self, index: usize, x: u32, y: u32, w: u32, h: u32) {
        self.nodes.insert(index, Node { x, y: y + h, w });

        // Trim every run the new level covers. The index never moves: either the
        // run under it is swallowed whole (and the vector shrinks) or it is
        // clipped and the trim is done.
        let i = index + 1;
        while i < self.nodes.len() {
            let prev_right = self.nodes[i - 1].x + self.nodes[i - 1].w;
            if self.nodes[i].x >= prev_right {
                break;
            }
            let shrink = prev_right - self.nodes[i].x;
            if self.nodes[i].w <= shrink {
                self.nodes.remove(i);
                continue;
            }
            self.nodes[i].x += shrink;
            self.nodes[i].w -= shrink;
            break;
        }

        // Merge neighbours that ended up at the same height.
        let mut i = 0;
        while i + 1 < self.nodes.len() {
            if self.nodes[i].y == self.nodes[i + 1].y {
                self.nodes[i].w += self.nodes[i + 1].w;
                self.nodes.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_insert_lands_at_the_origin() {
        let mut sky = Skyline::new(64, 64);
        assert_eq!(sky.insert(10, 10), Some((0, 0)));
        assert_eq!(sky.used_area(), 100);
        assert_eq!(sky.extent_y(), 10);
    }

    #[test]
    fn rectangles_fill_the_row_before_starting_a_new_one() {
        let mut sky = Skyline::new(32, 32);
        assert_eq!(sky.insert(16, 8), Some((0, 0)));
        assert_eq!(sky.insert(16, 8), Some((16, 0)));
        // The row is full, so the next one has to climb.
        assert_eq!(sky.insert(16, 8), Some((0, 8)));
    }

    #[test]
    fn insert_refuses_what_cannot_fit() {
        let mut sky = Skyline::new(16, 16);
        assert_eq!(sky.insert(17, 4), None);
        assert_eq!(sky.insert(4, 17), None);
        assert_eq!(sky.insert(0, 4), None);
        assert_eq!(sky.insert(16, 16), Some((0, 0)));
        assert_eq!(sky.insert(1, 1), None, "the page is full");
    }

    #[test]
    fn placements_never_overlap() {
        // 60 rectangles of varied size, checked pairwise.
        let mut sky = Skyline::new(256, 256);
        let mut placed: Vec<(u32, u32, u32, u32)> = Vec::new();
        for i in 0..60u32 {
            let w = 3 + (i * 7) % 29;
            let h = 3 + (i * 11) % 23;
            let (x, y) = sky.insert(w, h).expect("fits");
            for &(px, py, pw, ph) in &placed {
                let disjoint = x + w <= px || px + pw <= x || y + h <= py || py + ph <= y;
                assert!(disjoint, "({x},{y},{w},{h}) overlaps ({px},{py},{pw},{ph})");
            }
            assert!(x + w <= 256 && y + h <= 256, "out of the page");
            placed.push((x, y, w, h));
        }
        assert!(sky.extent_x() <= 256 && sky.extent_y() <= 256);
    }

    #[test]
    fn packing_is_deterministic_across_runs() {
        let run = || {
            let mut sky = Skyline::new(128, 128);
            (0..25u32)
                .map(|i| sky.insert(5 + i % 13, 4 + i % 9).expect("fits"))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
