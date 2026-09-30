//! The 16 → 6 table of the dual grid (estudo 09 §5.2, `CANON` in the
//! prototype `city.mjs`), with the kit's canonical orientation.
//!
//! A dual vertex is the corner shared by 4 tiles. Its mask reads them in the
//! order NW, NE, SE, SW: `NW | NE<<1 | SE<<2 | SW<<3`. The canonical mask of
//! each case has the "outside" facing north (−z), as the kit contract says:
//!
//! | case       | mask | occupied quadrants | outside faces |
//! |------------|------|--------------------|---------------|
//! | `vazio`    | 0    | none               | –             |
//! | `convexo`  | 4    | SE                 | N and W       |
//! | `parede`   | 12   | SE + SW            | N             |
//! | `diagonal` | 5    | NW + SE            | NE and SW     |
//! | `concavo`  | 14   | all but NW         | N and W (inner corner) |
//! | `cheio`    | 15   | all                | –             |
//!
//! `rot` quarter turns clockwise seen from above map the canonical mask onto
//! the real one: a clockwise quarter turn moves NW→NE→SE→SW→NW, which is a
//! left rotation of the 4 bits.

use crate::types::Dir;

pub const VAZIO: u8 = 0;
pub const CONVEXO: u8 = 1;
pub const PAREDE: u8 = 2;
pub const DIAGONAL: u8 = 3;
pub const CONCAVO: u8 = 4;
pub const CHEIO: u8 = 5;

pub const CASE_NAMES: [&str; 6] = ["vazio", "convexo", "parede", "diagonal", "concavo", "cheio"];
pub const CANON_MASK: [u8; 6] = [0, 4, 12, 5, 14, 15];

pub const NW: u8 = 1;
pub const NE: u8 = 2;
pub const SE: u8 = 4;
pub const SW: u8 = 8;

/// One clockwise quarter turn of a quadrant mask.
pub fn rot_cw(m: u8) -> u8 {
    ((m << 1) | (m >> 3)) & 15
}

/// `mask` (0..16) → `(case, rot)`: `rot_cw^rot(CANON_MASK[case]) == mask`.
pub fn classify(mask: u8) -> (u8, u8) {
    let mask = mask & 15;
    for (case, canon) in CANON_MASK.iter().enumerate() {
        let mut m = *canon;
        for r in 0..4u8 {
            if m == mask {
                return (case as u8, r);
            }
            m = rot_cw(m);
        }
    }
    unreachable!("every 4-bit mask is a rotation of a canonical case")
}

pub fn dir_from_quarter(q: u8) -> Dir {
    match q & 3 {
        0 => Dir::N,
        1 => Dir::E,
        2 => Dir::S,
        _ => Dir::W,
    }
}

/// `d` turned `rot` quarter turns clockwise.
pub fn rotate(d: Dir, rot: u8) -> Dir {
    dir_from_quarter(d.quarter() + rot)
}

pub fn opposite(d: Dir) -> Dir {
    rotate(d, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_covers_all_masks() {
        for m in 0..16u8 {
            let (c, r) = classify(m);
            let mut x = CANON_MASK[c as usize];
            for _ in 0..r {
                x = rot_cw(x);
            }
            assert_eq!(x, m);
        }
        // Wall with the building to the west faces east: rot 1.
        assert_eq!(classify(NW | SW), (PAREDE, 1));
        assert_eq!(rotate(Dir::N, 1), Dir::E);
    }
}
