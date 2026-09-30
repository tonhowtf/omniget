//! Fixed-point value noise (Q16) on top of `hash::hash2`: lattice values,
//! quintic smoothstep, fBm, ridged fBm and piecewise-linear tables. Pure
//! integer arithmetic, so the same bytes come out on every platform.

use crate::hash::hash2;

pub const ONE: i64 = 1 << 16;
pub const HALF: i64 = ONE / 2;

/// Lattice value in `0..ONE`.
#[inline]
fn lattice(seed: u64, x: i32, y: i32, salt: u32) -> i64 {
    (hash2(seed, x, y, salt) >> 16) as i64
}

/// `6t^5 - 15t^4 + 10t^3` for `t` in `0..=ONE`.
#[inline]
pub fn smooth(t: i64) -> i64 {
    let a = 6 * t - 15 * ONE;
    let b = ((t * a) >> 16) + 10 * ONE;
    let t3 = (((t * t) >> 16) * t) >> 16;
    (t3 * b) >> 16
}

/// Value noise at a Q16 lattice position, result in `0..ONE`.
pub fn value(seed: u64, salt: u32, px: i64, py: i64) -> i64 {
    let x0 = (px >> 16) as i32;
    let y0 = (py >> 16) as i32;
    let fx = smooth(px & (ONE - 1));
    let fy = smooth(py & (ONE - 1));
    let a = lattice(seed, x0, y0, salt);
    let b = lattice(seed, x0.wrapping_add(1), y0, salt);
    let c = lattice(seed, x0, y0.wrapping_add(1), salt);
    let d = lattice(seed, x0.wrapping_add(1), y0.wrapping_add(1), salt);
    let ab = a + (((b - a) * fx) >> 16);
    let cd = c + (((d - c) * fx) >> 16);
    ab + (((cd - ab) * fy) >> 16)
}

/// fBm of `octaves` value-noise octaves. `px`, `py` are Q16 positions in
/// arbitrary units and `period` is the size of the first octave's lattice in
/// those units (Q16 as well, so fractional periods work). Result in `0..ONE`.
pub fn fbm(seed: u64, salt: u32, px: i64, py: i64, period_q16: i64, octaves: u32) -> i64 {
    let period = period_q16.max(1);
    // Position in lattice units, Q16 (floor division keeps negatives seamless).
    let mut x = ((px as i128) << 16).div_euclid(period as i128);
    let mut y = ((py as i128) << 16).div_euclid(period as i128);
    let (mut sum, mut amp, mut norm) = (0i64, ONE, 0i64);
    for o in 0..octaves {
        // Per-octave offset breaks the alignment of lattices at the origin.
        let ox = (hash2(seed, o as i32, 17, salt ^ 0xA5A5) & 0x00FF_FFFF) as i128;
        let oy = (hash2(seed, o as i32, 29, salt ^ 0x5A5A) & 0x00FF_FFFF) as i128;
        let vx = clamp_i64(x + ox);
        let vy = clamp_i64(y + oy);
        sum += (value(seed, salt.wrapping_add(o * 0x9E37), vx, vy) * amp) >> 16;
        norm += amp;
        amp >>= 1;
        x *= 2;
        y *= 2;
    }
    if norm == 0 {
        return HALF;
    }
    (sum * ONE) / norm
}

/// Ridged fBm: `1 - |2n - 1|`, squared, in `0..ONE`.
pub fn ridged(seed: u64, salt: u32, px: i64, py: i64, period_q16: i64, octaves: u32) -> i64 {
    let n = fbm(seed, salt, px, py, period_q16, octaves);
    let r = ONE - (2 * n - ONE).abs();
    (r * r) >> 16
}

/// fBm stretched around 0.5 by `gain_per_mille` and clamped to `0..=ONE`
/// (value-noise fBm clusters around the middle; this spreads it out).
pub fn stretch(n: i64, gain_per_mille: i64) -> i64 {
    (HALF + (n - HALF) * gain_per_mille / 1000).clamp(0, ONE)
}

/// Piecewise-linear table lookup: `points` sorted by x (Q16), y in any unit.
pub fn table(points: &[(i64, i64)], x: i64) -> i64 {
    if points.is_empty() {
        return 0;
    }
    if x <= points[0].0 {
        return points[0].1;
    }
    for w in points.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if x <= x1 {
            if x1 == x0 {
                return y1;
            }
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    points[points.len() - 1].1
}

#[inline]
pub fn div_floor(a: i64, b: i64) -> i64 {
    a.div_euclid(b)
}

#[inline]
fn clamp_i64(v: i128) -> i64 {
    v.clamp(i64::MIN as i128 / 4, i64::MAX as i128 / 4) as i64
}

/// Integer square root.
pub fn isqrt(v: u64) -> u64 {
    v.isqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fbm_in_range_and_stable() {
        let mut acc = 0i64;
        for i in -50..50 {
            let n = fbm(42, 1, i * 12345, -i * 777, 40 * ONE, 5);
            assert!((0..=ONE).contains(&n));
            acc = acc.wrapping_mul(31).wrapping_add(n);
        }
        let mut acc2 = 0i64;
        for i in -50..50 {
            acc2 = acc2
                .wrapping_mul(31)
                .wrapping_add(fbm(42, 1, i * 12345, -i * 777, 40 * ONE, 5));
        }
        assert_eq!(acc, acc2);
    }

    #[test]
    fn smooth_endpoints() {
        assert_eq!(smooth(0), 0);
        assert_eq!(smooth(ONE), ONE);
    }
}
