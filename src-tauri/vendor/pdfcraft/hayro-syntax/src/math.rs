// Note that these polyfills can be very imprecise, but hopefully good enough
// for the vast majority of cases.

#[inline(always)]
pub(crate) fn trunc_f64(x: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        x.trunc()
    }
    #[cfg(not(feature = "std"))]
    {
        x as i64 as f64
    }
}

#[inline(always)]
pub(crate) fn powi_f64(x: f64, n: u32) -> f64 {
    #[cfg(feature = "std")]
    {
        x.powi(n as i32)
    }
    #[cfg(not(feature = "std"))]
    {
        let mut result = 1.0;
        let mut base = x;
        let mut exp = n;
        while exp > 0 {
            if exp & 1 == 1 {
                result *= base;
            }
            base *= base;
            exp >>= 1;
        }
        result
    }
}
