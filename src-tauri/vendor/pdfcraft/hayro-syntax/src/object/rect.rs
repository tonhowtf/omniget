//! Rectangles.

use crate::object::Array;
use crate::object::{Object, ObjectLike};
use crate::reader::{Readable, ReaderContext, ReaderExt};

use crate::reader::Reader;

/// A rectangle.
#[derive(Copy, Clone, Debug)]
pub struct Rect {
    /// The minimum x coordinate.
    pub x0: f64,
    /// The minimum y coordinate.
    pub y0: f64,
    /// The maximum x coordinate.
    pub x1: f64,
    /// The maximum y coordinate.
    pub y1: f64,
}

impl Rect {
    /// The empty rectangle at the origin.
    pub const ZERO: Self = Self::new(0., 0., 0., 0.);

    /// A new rectangle from minimum and maximum coordinates.
    #[inline(always)]
    pub const fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// The intersection of two rectangles.
    #[inline]
    pub fn intersect(&self, other: Self) -> Self {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        let x1 = self.x1.min(other.x1);
        let y1 = self.y1.min(other.y1);
        Self::new(x0, y0, x1.max(x0), y1.max(y0))
    }

    /// The width of the rectangle.
    #[inline]
    pub const fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    /// The height of the rectangle.
    #[inline]
    pub const fn height(&self) -> f64 {
        self.y1 - self.y0
    }
}

impl<'a> Readable<'a> for Rect {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let arr = r.read::<Array<'_>>(ctx)?;
        from_arr(&arr)
    }
}

fn from_arr(array: &Array<'_>) -> Option<Rect> {
    let mut iter = array.iter::<f32>();
    let x0 = iter.next()? as f64;
    let y0 = iter.next()? as f64;
    let x1 = iter.next()? as f64;
    let y1 = iter.next()? as f64;

    Some(Rect::new(x0.min(x1), y0.min(y1), x1.max(x0), y1.max(y0)))
}

impl TryFrom<Object<'_>> for Rect {
    type Error = ();

    fn try_from(value: Object<'_>) -> Result<Self, Self::Error> {
        match value {
            Object::Array(arr) => from_arr(&arr).ok_or(()),
            _ => Err(()),
        }
    }
}

impl ObjectLike<'_> for Rect {}
