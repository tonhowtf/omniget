//! Arrays.

use crate::object::macros::object;
use crate::object::r#ref::MaybeRef;
use crate::object::{FromBytes, Object, ObjectLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use alloc::vec::Vec;
use core::fmt::{Debug, Formatter};
use core::marker::PhantomData;
use smallvec::SmallVec;

/// An array of PDF objects.
#[derive(Clone)]
pub struct Array<'a> {
    data: &'a [u8],
    ctx: ReaderContext<'a>,
}

// Note that this is not structural equality, i.e. two arrays with the same
// items are still considered different if they have different whitespaces.
impl PartialEq for Array<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl<'a> Array<'a> {
    /// Returns an iterator over the objects of the array.
    pub fn raw_iter(&self) -> ArrayIter<'a> {
        ArrayIter::new(self.data, &self.ctx)
    }

    /// Returns an iterator over the resolved objects of the array.
    #[allow(
        private_bounds,
        reason = "users shouldn't be able to implement `ObjectLike` for custom objects."
    )]
    pub fn iter<T>(&self) -> ResolvedArrayIter<'a, T>
    where
        T: ObjectLike<'a>,
    {
        ResolvedArrayIter::new(self.data, &self.ctx)
    }

    /// Return a flex iterator over the items in the array.
    pub fn flex_iter(&self) -> FlexArrayIter<'a> {
        FlexArrayIter::new(self.data, &self.ctx)
    }

    /// Return the raw data of the array.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }
}

impl Debug for Array<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let mut debug_list = f.debug_list();

        self.raw_iter().for_each(|i| {
            debug_list.entry(&i);
        });

        Ok(())
    }
}

object!(Array<'a>, Array);

impl Skippable for Array<'_> {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        r.forward_tag(b"[")?;
        r.enter_container()?;

        // PdfCraft patch: leave even when an element's skip fails through `?`.
        let result = (|| loop {
            r.skip_white_spaces_and_comments();

            if let Some(()) = r.forward_tag(b"]") {
                return Some(());
            } else if is_content_stream {
                r.skip::<Object<'_>>(true)?;
            } else {
                r.skip::<MaybeRef<Object<'_>>>(false)?;
            }
        })();
        r.leave_container();
        result
    }
}

impl Default for Array<'_> {
    fn default() -> Self {
        Self::from_bytes(b"[]").unwrap()
    }
}

impl<'a> Readable<'a> for Array<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let bytes = r.skip::<Array<'_>>(ctx.in_content_stream())?;

        Some(Self {
            data: &bytes[1..bytes.len() - 1],
            ctx: ctx.clone(),
        })
    }
}

/// An iterator over the items of an array.
pub struct ArrayIter<'a> {
    reader: Reader<'a>,
    ctx: ReaderContext<'a>,
}

impl<'a> ArrayIter<'a> {
    fn new(data: &'a [u8], ctx: &ReaderContext<'a>) -> Self {
        Self {
            reader: Reader::new(data),
            ctx: ctx.clone(),
        }
    }
}

impl<'a> Iterator for ArrayIter<'a> {
    type Item = MaybeRef<Object<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.skip_white_spaces_and_comments();

        if !self.reader.at_end() {
            // Objects are already guaranteed to be valid.
            let item = self
                .reader
                .read_with_context::<MaybeRef<Object<'_>>>(&self.ctx)
                .unwrap();
            return Some(item);
        }

        None
    }
}

/// An iterator over the array that resolves objects of a specific type.
pub struct ResolvedArrayIter<'a, T> {
    flex_iter: FlexArrayIter<'a>,
    phantom_data: PhantomData<T>,
}

impl<'a, T> ResolvedArrayIter<'a, T> {
    fn new(data: &'a [u8], ctx: &ReaderContext<'a>) -> Self {
        Self {
            flex_iter: FlexArrayIter::new(data, ctx),
            phantom_data: PhantomData,
        }
    }
}

impl<'a, T> Iterator for ResolvedArrayIter<'a, T>
where
    T: ObjectLike<'a>,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.flex_iter.next::<T>()
    }
}

/// An iterator over the array that allows reading a different object each time.
pub struct FlexArrayIter<'a> {
    reader: Reader<'a>,
    ctx: ReaderContext<'a>,
}

impl<'a> FlexArrayIter<'a> {
    fn new(data: &'a [u8], ctx: &ReaderContext<'a>) -> Self {
        Self {
            reader: Reader::new(data),
            ctx: ctx.clone(),
        }
    }

    #[allow(
        private_bounds,
        reason = "users shouldn't be able to implement `ObjectLike` for custom objects."
    )]
    #[allow(clippy::should_implement_trait)]
    /// Try reading the next item as a specific object from the array.
    pub fn next<T: ObjectLike<'a>>(&mut self) -> Option<T> {
        self.reader.skip_white_spaces_and_comments();

        if !self.reader.at_end() {
            // PdfCraft patch (32): its items each follow a reference. Read as one that might be, a
            // reference at the start was taken for the whole group, which it can't be, and lost.
            if T::GROUP {
                return self.reader.read_with_context::<T>(&self.ctx);
            }

            return match self.reader.read_with_context::<MaybeRef<T>>(&self.ctx)? {
                MaybeRef::Ref(r) => self.ctx.xref().get_with::<T>(r.into(), &self.ctx),
                MaybeRef::NotRef(i) => Some(i),
            };
        }

        None
    }
}

impl<'a, T: ObjectLike<'a> + Copy + Default, const C: usize> TryFrom<Array<'a>> for [T; C] {
    type Error = ();

    fn try_from(value: Array<'a>) -> Result<Self, Self::Error> {
        let mut iter = value.iter::<T>();

        let mut val = [T::default(); C];

        for i in 0..C {
            val[i] = iter.next().ok_or(())?;
        }

        if iter.next().is_some() {
            warn!("found excess elements in array");

            return Err(());
        }

        Ok(val)
    }
}

impl<'a, T: ObjectLike<'a> + Copy + Default, const C: usize> TryFrom<Object<'a>> for [T; C]
where
    [T; C]: TryFrom<Array<'a>, Error = ()>,
{
    type Error = ();

    fn try_from(value: Object<'a>) -> Result<Self, Self::Error> {
        match value {
            Object::Array(a) => a.try_into(),
            _ => Err(()),
        }
    }
}

impl<'a, T: ObjectLike<'a> + Copy + Default, const C: usize> Readable<'a> for [T; C] {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let array = Array::read(r, ctx)?;
        array.try_into().ok()
    }
}

impl<'a, T: ObjectLike<'a> + Copy + Default, const C: usize> ObjectLike<'a> for [T; C] {}

impl<'a, T: ObjectLike<'a>> TryFrom<Array<'a>> for Vec<T> {
    type Error = ();

    fn try_from(value: Array<'a>) -> Result<Self, Self::Error> {
        Ok(value.iter::<T>().collect())
    }
}

impl<'a, T: ObjectLike<'a>> TryFrom<Object<'a>> for Vec<T> {
    type Error = ();

    fn try_from(value: Object<'a>) -> Result<Self, Self::Error> {
        match value {
            Object::Array(a) => a.try_into(),
            _ => Err(()),
        }
    }
}

impl<'a, T: ObjectLike<'a>> Readable<'a> for Vec<T> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let array = Array::read(r, ctx)?;
        array.try_into().ok()
    }
}

impl<'a, T: ObjectLike<'a>> ObjectLike<'a> for Vec<T> {}

impl<'a, U: ObjectLike<'a>, T: ObjectLike<'a> + smallvec::Array<Item = U>> TryFrom<Array<'a>>
    for SmallVec<T>
{
    type Error = ();

    fn try_from(value: Array<'a>) -> Result<Self, Self::Error> {
        Ok(value.iter::<U>().collect())
    }
}

impl<'a, U: ObjectLike<'a>, T: ObjectLike<'a> + smallvec::Array<Item = U>> TryFrom<Object<'a>>
    for SmallVec<T>
{
    type Error = ();

    fn try_from(value: Object<'a>) -> Result<Self, Self::Error> {
        match value {
            Object::Array(a) => a.try_into(),
            _ => Err(()),
        }
    }
}

impl<'a, U: ObjectLike<'a>, T: ObjectLike<'a> + smallvec::Array<Item = U>> Readable<'a>
    for SmallVec<T>
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let array = Array::read(r, ctx)?;
        array.try_into().ok()
    }
}

impl<'a, U: ObjectLike<'a>, T: ObjectLike<'a> + smallvec::Array<Item = U>> ObjectLike<'a>
    for SmallVec<T>
where
    U: Clone,
    U: Debug,
{
}

#[cfg(test)]
mod tests {
    use crate::object::Array;
    use crate::object::Object;
    use crate::object::r#ref::{MaybeRef, ObjRef};
    use crate::reader::Reader;
    use crate::reader::{ReaderContext, ReaderExt};
    use crate::xref::XRef;

    fn array_impl(data: &[u8]) -> Option<Vec<Object<'_>>> {
        Reader::new(data)
            .read_with_context::<Array<'_>>(&ReaderContext::new(XRef::dummy(), false))
            .map(|a| a.iter::<Object<'_>>().collect::<Vec<_>>())
    }

    fn array_ref_impl(data: &[u8]) -> Option<Vec<MaybeRef<Object<'_>>>> {
        Reader::new(data)
            .read_with_context::<Array<'_>>(&ReaderContext::new(XRef::dummy(), false))
            .map(|a| a.raw_iter().collect::<Vec<_>>())
    }

    #[test]
    fn empty_array_1() {
        let res = array_impl(b"[]").unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn empty_array_2() {
        let res = array_impl(b"[   \n]").unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn array_1() {
        let res = array_impl(b"[34]").unwrap();
        assert!(matches!(res[0], Object::Number(_)));
    }

    #[test]
    fn array_2() {
        let res = array_impl(b"[true  ]").unwrap();
        assert!(matches!(res[0], Object::Boolean(_)));
    }

    #[test]
    fn array_3() {
        let res = array_impl(b"[true \n false 34.564]").unwrap();
        assert!(matches!(res[0], Object::Boolean(_)));
        assert!(matches!(res[1], Object::Boolean(_)));
        assert!(matches!(res[2], Object::Number(_)));
    }

    #[test]
    fn array_4() {
        let res = array_impl(b"[(A string.) << /Hi 34.35 >>]").unwrap();
        assert!(matches!(res[0], Object::String(_)));
        assert!(matches!(res[1], Object::Dict(_)));
    }

    #[test]
    fn array_5() {
        let res = array_impl(b"[[32]  345.6]").unwrap();
        assert!(matches!(res[0], Object::Array(_)));
        assert!(matches!(res[1], Object::Number(_)));
    }

    #[test]
    fn array_with_ref() {
        let res = array_ref_impl(b"[345 34 5 R 34.0]").unwrap();
        assert!(matches!(res[0], MaybeRef::NotRef(Object::Number(_))));
        assert!(matches!(
            res[1],
            MaybeRef::Ref(ObjRef {
                obj_number: 34,
                gen_number: 5
            })
        ));
        assert!(matches!(res[2], MaybeRef::NotRef(Object::Number(_))));
    }

    #[test]
    fn malformed_ref_is_not_valid_array() {
        assert!(array_ref_impl(b"[8 0R]").is_none());
    }

    #[test]
    fn array_with_single_number_is_valid() {
        assert!(array_ref_impl(b"[345345345]").is_some());
    }

    #[test]
    fn array_with_comment() {
        let res = array_impl(b"[true % A comment \n false]").unwrap();
        assert!(matches!(res[0], Object::Boolean(_)));
        assert!(matches!(res[1], Object::Boolean(_)));
    }

    #[test]
    fn array_with_trailing() {
        let res = array_impl(b"[(Hi) /Test]trialing data").unwrap();
        assert!(matches!(res[0], Object::String(_)));
        assert!(matches!(res[1], Object::Name(_)));
    }
}
