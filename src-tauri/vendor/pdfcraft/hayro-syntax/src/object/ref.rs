//! Object references.

use crate::object::ObjectIdentifier;
use crate::object::ObjectLike;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use core::fmt::{Debug, Formatter};

/// A reference to an object.
#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub struct ObjRef {
    /// The object number.
    pub obj_number: i32,
    /// The generation number.
    pub gen_number: i32,
}

impl ObjRef {
    /// Create a new object reference.
    pub fn new(obj_number: i32, gen_number: i32) -> Self {
        Self {
            obj_number,
            gen_number,
        }
    }
}

impl From<ObjRef> for ObjectIdentifier {
    fn from(value: ObjRef) -> Self {
        Self::new(value.obj_number, value.gen_number)
    }
}

impl From<ObjectIdentifier> for ObjRef {
    fn from(value: ObjectIdentifier) -> Self {
        Self::new(value.obj_number, value.gen_number)
    }
}

impl Skippable for ObjRef {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        // PdfCraft patch: (hayro-syntax (33)) a comment counts as white space, here too.
        r.skip::<i32>(false)?;
        r.skip_white_spaces_and_comments();
        r.skip::<i32>(false)?;
        r.skip_white_spaces_and_comments();
        r.forward_tag(b"R")?;

        Some(())
    }
}

impl Readable<'_> for ObjRef {
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        let obj_ref = r.read_without_context::<i32>()?;
        r.skip_white_spaces_and_comments();
        let gen_num = r.read_without_context::<i32>()?;
        r.skip_white_spaces_and_comments();
        r.forward_tag(b"R")?;

        Some(Self::new(obj_ref, gen_num))
    }
}

/// A struct that is either an object or a reference to an object.
#[derive(PartialEq, Eq)]
pub enum MaybeRef<T> {
    /// A reference to an object.
    Ref(ObjRef),
    /// An object.
    NotRef(T),
}

impl<T> MaybeRef<T> {
    /// If the object is an object reference, return it.
    pub fn as_obj_ref(&self) -> Option<ObjRef> {
        match self {
            Self::Ref(r) => Some(*r),
            Self::NotRef(_) => None,
        }
    }
}

#[allow(private_bounds)]
impl<'a, T> MaybeRef<T>
where
    T: ObjectLike<'a>,
{
    /// Resolve the `MaybeRef` object with the given xref table.
    pub(crate) fn resolve(self, ctx: &ReaderContext<'a>) -> Option<T> {
        match self {
            Self::Ref(r) => ctx.xref().get_with::<T>(r.into(), ctx),
            Self::NotRef(t) => Some(t),
        }
    }
}

impl<T> TryFrom<MaybeRef<T>> for ObjRef {
    type Error = ();

    fn try_from(value: MaybeRef<T>) -> Result<Self, Self::Error> {
        match value {
            MaybeRef::Ref(r) => Ok(r),
            MaybeRef::NotRef(_) => Err(()),
        }
    }
}

impl<T> Debug for MaybeRef<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ref(r) => write!(f, "{r:?}"),
            Self::NotRef(nr) => write!(f, "{nr:?}"),
        }
    }
}

impl<T> Skippable for MaybeRef<T>
where
    T: Skippable,
{
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        r.skip::<ObjRef>(is_content_stream)
            .or_else(|| r.skip::<T>(is_content_stream))
            .map(|_| {})
    }
}

impl<'a, T> Readable<'a> for MaybeRef<T>
where
    T: Readable<'a>,
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        if let Some(obj) = r.read::<ObjRef>(ctx) {
            Some(Self::Ref(obj))
        } else {
            Some(Self::NotRef(r.read::<T>(ctx)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::object::ObjRef;
    use crate::reader::Reader;
    use crate::reader::ReaderExt;

    #[test]
    fn ref_1() {
        assert_eq!(
            Reader::new("34 1 R".as_bytes())
                .read_without_context::<ObjRef>()
                .unwrap(),
            ObjRef::new(34, 1)
        );
    }

    #[test]
    fn ref_trailing() {
        assert_eq!(
            Reader::new("256 0 R (hi)".as_bytes())
                .read_without_context::<ObjRef>()
                .unwrap(),
            ObjRef::new(256, 0)
        );
    }

    #[test]
    fn ref_invalid_1() {
        assert!(
            Reader::new("256 R".as_bytes())
                .read_without_context::<ObjRef>()
                .is_none()
        );
    }

    #[test]
    fn ref_invalid_2() {
        assert!(
            Reader::new("256 257".as_bytes())
                .read_without_context::<ObjRef>()
                .is_none()
        );
    }
}
