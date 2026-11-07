pub(crate) const SIMD_WIDTH: usize = 8;

#[cfg(feature = "simd")]
mod inner {
    use super::SIMD_WIDTH;
    use core::ops::{BitAnd, BitOr, BitXor, BitXorAssign};
    use fearless_simd::{Select, SimdBase, SimdInt};

    pub(crate) use fearless_simd::{Level, Simd, dispatch};

    #[derive(Copy, Clone)]
    #[allow(non_camel_case_types)]
    #[repr(C, align(32))]
    pub(crate) struct mask32x8<S: Simd> {
        inner: fearless_simd::mask32x8<S>,
    }

    impl<S: Simd> mask32x8<S> {
        #[inline(always)]
        pub(crate) fn select(self, if_true: u32x8<S>, if_false: u32x8<S>) -> u32x8<S> {
            u32x8 {
                inner: self.inner.select(if_true.inner, if_false.inner),
            }
        }
    }

    #[derive(Copy, Clone)]
    #[allow(non_camel_case_types)]
    #[repr(C, align(32))]
    pub(crate) struct u32x8<S: Simd> {
        inner: fearless_simd::u32x8<S>,
    }

    impl<S: Simd> u32x8<S> {
        #[inline(always)]
        pub(crate) fn from_slice(simd: S, slice: &[u32]) -> Self {
            Self {
                inner: fearless_simd::u32x8::from_slice(simd, &slice[..SIMD_WIDTH]),
            }
        }

        #[inline(always)]
        pub(crate) fn splat(simd: S, val: u32) -> Self {
            Self {
                inner: fearless_simd::u32x8::splat(simd, val),
            }
        }

        #[inline(always)]
        pub(crate) fn store(self, slice: &mut [u32]) {
            self.inner.store_slice(&mut slice[..SIMD_WIDTH]);
        }

        #[inline(always)]
        pub(crate) fn simd_gt(self, other: Self) -> mask32x8<S> {
            mask32x8 {
                inner: self.inner.simd_gt(other.inner),
            }
        }
    }

    impl<S: Simd> BitAnd for u32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn bitand(self, rhs: Self) -> Self {
            Self {
                inner: self.inner & rhs.inner,
            }
        }
    }

    impl<S: Simd> BitOr for u32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn bitor(self, rhs: Self) -> Self {
            Self {
                inner: self.inner | rhs.inner,
            }
        }
    }

    impl<S: Simd> BitXor for u32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn bitxor(self, rhs: Self) -> Self {
            Self {
                inner: self.inner ^ rhs.inner,
            }
        }
    }

    impl<S: Simd> BitXorAssign for u32x8<S> {
        #[inline(always)]
        fn bitxor_assign(&mut self, rhs: Self) {
            self.inner = self.inner ^ rhs.inner;
        }
    }
}

#[cfg(not(feature = "simd"))]
mod inner {
    use super::SIMD_WIDTH;
    use core::marker::PhantomData;
    use core::ops::{BitAnd, BitOr, BitXor, BitXorAssign};

    pub(crate) trait Simd: Copy + Clone {}

    #[derive(Copy, Clone)]
    pub(crate) struct ScalarSimd;
    impl Simd for ScalarSimd {}

    pub(crate) struct Level;
    impl Level {
        #[inline(always)]
        pub(crate) fn new() -> Self {
            Level
        }
    }

    macro_rules! simd_dispatch {
        ($level:expr, $simd:ident => $body:expr) => {{
            let _ = $level;
            let $simd = $crate::simd::ScalarSimd;
            $body
        }};
    }

    pub(crate) use simd_dispatch as dispatch;

    #[derive(Copy, Clone)]
    #[allow(non_camel_case_types)]
    #[repr(C, align(32))]
    pub(crate) struct mask32x8<S: Simd> {
        val: [bool; SIMD_WIDTH],
        _marker: PhantomData<S>,
    }

    impl<S: Simd> mask32x8<S> {
        #[inline(always)]
        pub(crate) fn select(self, if_true: u32x8<S>, if_false: u32x8<S>) -> u32x8<S> {
            let mut result = [0u32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = if self.val[i] {
                    if_true.val[i]
                } else {
                    if_false.val[i]
                };
            }
            u32x8 {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    #[derive(Copy, Clone)]
    #[allow(non_camel_case_types)]
    #[repr(C, align(32))]
    pub(crate) struct u32x8<S: Simd> {
        pub(super) val: [u32; SIMD_WIDTH],
        _marker: PhantomData<S>,
    }

    impl<S: Simd> u32x8<S> {
        #[inline(always)]
        pub(crate) fn from_slice(_simd: S, slice: &[u32]) -> Self {
            let mut val = [0u32; SIMD_WIDTH];
            val.copy_from_slice(&slice[..SIMD_WIDTH]);
            Self {
                val,
                _marker: PhantomData,
            }
        }

        #[inline(always)]
        pub(crate) fn splat(_simd: S, val: u32) -> Self {
            Self {
                val: [val; SIMD_WIDTH],
                _marker: PhantomData,
            }
        }

        #[inline(always)]
        pub(crate) fn store(self, slice: &mut [u32]) {
            slice[..SIMD_WIDTH].copy_from_slice(&self.val);
        }

        #[inline(always)]
        pub(crate) fn simd_gt(self, other: Self) -> mask32x8<S> {
            let mut result = [false; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] > other.val[i];
            }
            mask32x8 {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> BitAnd for u32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn bitand(self, rhs: Self) -> Self {
            let mut result = [0u32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] & rhs.val[i];
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> BitOr for u32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn bitor(self, rhs: Self) -> Self {
            let mut result = [0u32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] | rhs.val[i];
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> BitXor for u32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn bitxor(self, rhs: Self) -> Self {
            let mut result = [0u32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] ^ rhs.val[i];
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> BitXorAssign for u32x8<S> {
        #[inline(always)]
        fn bitxor_assign(&mut self, rhs: Self) {
            for i in 0..SIMD_WIDTH {
                self.val[i] ^= rhs.val[i];
            }
        }
    }
}

pub(crate) use inner::*;
