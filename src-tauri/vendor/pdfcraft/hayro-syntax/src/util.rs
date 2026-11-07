use crate::sync::OnceLock;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ops::Sub;
use smallvec::{Array, SmallVec};

pub(crate) trait OptionLog {
    fn error_none(self, f: &str) -> Self;
}

impl<T> OptionLog for Option<T> {
    fn error_none(self, _f: &str) -> Self {
        self.or_else(|| {
            error!("{_f}");

            None
        })
    }
}

const SCALAR_NEARLY_ZERO: f32 = 1.0 / (1 << 12) as f32;

/// A number of useful methods for f32 numbers.
pub(crate) trait FloatExt: Sized + Sub<f32, Output = f32> + Copy {
    /// Whether the number is approximately 0.
    fn is_nearly_zero(&self) -> bool {
        self.is_nearly_zero_within_tolerance(SCALAR_NEARLY_ZERO)
    }

    /// Whether the number is approximately 0, with a given tolerance.
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool;
}

impl FloatExt for f32 {
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool {
        debug_assert!(tolerance >= 0.0, "tolerance must be non-negative");

        self.abs() <= tolerance
    }
}

pub(crate) trait ByteBuf: Default {
    fn with_capacity(capacity: usize) -> Self;
    fn push(&mut self, byte: u8);
}

impl ByteBuf for Vec<u8> {
    fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    fn push(&mut self, byte: u8) {
        Self::push(self, byte);
    }
}

impl<A> ByteBuf for SmallVec<A>
where
    A: Array<Item = u8>,
{
    fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    fn push(&mut self, byte: u8) {
        Self::push(self, byte);
    }
}

#[cfg(feature = "unsafe")]
pub(crate) fn find_needle(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::find(haystack, needle)
}

#[cfg(feature = "unsafe")]
pub(crate) fn findr_needle(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::rfind(haystack, needle)
}

#[cfg(not(feature = "unsafe"))]
pub(crate) fn find_needle(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }

    let first = needle[0];
    let mut i = 0;

    while i + needle.len() <= haystack.len() {
        while haystack.get(i).copied()? != first {
            i += 1;
            if i + needle.len() > haystack.len() {
                return None;
            }
        }

        if haystack.get(i..)?.starts_with(needle) {
            return Some(i);
        }

        i += 1;
    }

    None
}

#[cfg(not(feature = "unsafe"))]
pub(crate) fn findr_needle(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(haystack.len());
    }

    if needle.len() > haystack.len() {
        return None;
    }

    let first = needle[0];
    let mut i = haystack.len() - needle.len();

    loop {
        while haystack.get(i).copied()? != first {
            i = i.checked_sub(1)?;
        }

        if haystack.get(i..)?.starts_with(needle) {
            return Some(i);
        }

        i = i.checked_sub(1)?;
    }
}

/// Allows to store elements at an index with an `&self` reference.
///
/// This is similar to a thread-safe arena data structure, but with less moving
/// parts and no unsafe code. It's based on the segment list presented in
/// <https://matklad.github.io/2023/04/23/data-oriented-parallel-value-interner.html>
///
/// Indices should be used in order. Usage of higher indices implies more memory
/// usage (even if lower indices are not in use).
///
/// The capacity is limited at 2^C - 1. Out-of-range indices return `None` (PdfCraft patch:
/// upstream panics; #307 reaches this with object-stream slot numbers from the file).
pub(crate) struct SegmentList<T, const C: usize>([OnceLock<Box<[OnceLock<T>]>>; C]);

impl<T, const C: usize> SegmentList<T, C> {
    pub(crate) fn new() -> Self {
        Self(core::array::from_fn(|_| OnceLock::new()))
    }

    pub(crate) fn get(&self, i: usize) -> Option<&T> {
        let (s, k) = self.locate(i)?;
        let segment = self.0.get(s as usize)?.get()?;
        segment.get(k)?.get()
    }

    #[track_caller]
    pub(crate) fn get_or_init(&self, i: usize, f: impl FnOnce() -> T) -> Option<&T> {
        let (s, k) = self.locate(i)?;
        let segment = self.0.get(s as usize)?.get_or_init(|| {
            (0..2_usize.pow(s))
                .map(|_| OnceLock::new())
                .collect::<Vec<_>>()
                .into_boxed_slice()
        });
        Some(segment.get(k)?.get_or_init(f))
    }

    fn locate(&self, i: usize) -> Option<(u32, usize)> {
        let power = i.checked_add(2)?.checked_next_power_of_two()? / 2;
        let s = power.trailing_zeros();
        let k = i - ((1 << s) - 1);
        Some((s, k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_list() {
        let e = SegmentList::<String, 10>::new();

        // Ensure that it works.
        for i in 0..500 {
            e.get_or_init(i, || format!("{i}"));
        }
        for i in 0..500 {
            assert_eq!(e.get(i), Some(&format!("{i}")));
        }

        // Ensure that all slots in the first 8 segments are actually in use,
        // i.e. we didn't overallocate.
        for s in 0..8 {
            assert!(e.0[s].get().unwrap().iter().all(|s| s.get().is_some()));
        }
    }

    #[test]
    fn segment_list_rejects_out_of_range_indices() {
        let list = SegmentList::<u8, 2>::new();
        assert_eq!(list.get_or_init(2, || 7), Some(&7));
        for index in [3, 20, usize::MAX - 1, usize::MAX] {
            assert_eq!(list.get(index), None);
            assert_eq!(list.get_or_init(index, || 9), None);
        }
    }

    #[test]
    fn test_findr_needle() {
        assert_eq!(findr_needle(b"abc foo def foo xyz", b"foo"), Some(12));
        assert_eq!(findr_needle(b"abc", b""), Some(3));
        assert_eq!(findr_needle(b"abc", b"abcd"), None);
        assert_eq!(findr_needle(b"abc", b"z"), None);
    }
}
