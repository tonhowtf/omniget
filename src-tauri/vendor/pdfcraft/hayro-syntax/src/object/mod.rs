//! Parsing and reading from PDF objects.

pub use crate::object::array::Array;
pub use crate::object::date::DateTime;
pub use crate::object::dict::Dict;
pub use crate::object::name::Name;
use crate::object::name::skip_name_like;
pub use crate::object::null::Null;
pub use crate::object::number::Number;
pub use crate::object::rect::Rect;
pub use crate::object::r#ref::{MaybeRef, ObjRef};
pub use crate::object::stream::Stream;
pub use crate::object::string::String;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use core::fmt::Debug;

mod bool;
mod date;
mod null;
mod number;
mod rect;
mod r#ref;
mod string;
mod tuple;

pub(crate) mod indirect;
pub(crate) mod name;

pub mod array;
pub mod dict;
pub mod stream;

/// A trait for PDF objects.
pub(crate) trait ObjectLike<'a>: TryFrom<Object<'a>> + Readable<'a> + Debug + Clone {
    /// PdfCraft patch (32): whether this is several items of an array, read one by one (a tuple).
    /// Such a thing can't itself be given by reference, so one at its start is its first item's.
    const GROUP: bool = false;
}

/// A trait for PDF objects that can be borrowed directly from an [`Object`].
pub(crate) trait ObjectRefLike<'a>: Sized {
    fn cast_ref<'b>(obj: &'b Object<'a>) -> Option<&'b Self>;
}

/// A primitive PDF object.
#[derive(Debug, Clone, PartialEq)]
pub enum Object<'a> {
    /// A null object.
    Null(Null),
    /// A boolean object.
    Boolean(bool),
    /// A number object.
    Number(Number),
    /// A string object.
    String(String<'a>),
    /// A name object.
    Name(Name<'a>),
    /// A dict object.
    Dict(Dict<'a>),
    /// An array object.
    Array(Array<'a>),
    /// A stream object.
    // Can only be an indirect object in theory and thus comes with some caveats,
    // but we just treat it the same.
    Stream(Stream<'a>),
}

impl<'a> Object<'a> {
    /// Try casting the object to a specific subtype.
    pub(crate) fn cast<T>(self) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        self.try_into().ok()
    }

    /// Try casting the object to a reference of the specific type.
    pub(crate) fn cast_ref<T>(&self) -> Option<&T>
    where
        T: ObjectRefLike<'a>,
    {
        T::cast_ref(self)
    }

    /// Try casting the object to a dict.
    #[inline(always)]
    pub fn into_dict(self) -> Option<Dict<'a>> {
        self.cast()
    }

    /// Try casting the object to a name.
    #[inline(always)]
    pub fn into_name(self) -> Option<Name<'a>> {
        self.cast()
    }

    /// Try casting the object to the null object.
    #[inline(always)]
    pub fn into_null(self) -> Option<Null> {
        self.cast()
    }

    /// Try casting the object to a bool.
    #[inline(always)]
    pub fn into_bool(self) -> Option<bool> {
        self.cast()
    }

    /// Try casting the object to a string.
    #[inline(always)]
    pub fn into_string(self) -> Option<String<'a>> {
        self.cast()
    }

    /// Try casting the object to a stream.
    #[inline(always)]
    pub fn into_stream(self) -> Option<Stream<'a>> {
        self.cast()
    }

    /// Try casting the object to an array.
    #[inline(always)]
    pub fn into_array(self) -> Option<Array<'a>> {
        self.cast()
    }

    /// Try casting the object to a u8.
    #[inline(always)]
    pub fn into_u8(self) -> Option<u8> {
        self.cast()
    }

    /// Try casting the object to a u16.
    #[inline(always)]
    pub fn into_u16(self) -> Option<u16> {
        self.cast()
    }

    /// Try casting the object to a f32.
    #[inline(always)]
    pub fn into_f32(self) -> Option<f32> {
        self.cast()
    }

    /// Try casting the object to a i32.
    #[inline(always)]
    pub fn into_i32(self) -> Option<i32> {
        self.cast()
    }

    /// Try casting the object to a number.
    #[inline(always)]
    pub fn into_number(self) -> Option<Number> {
        self.cast()
    }
}

impl<'a> ObjectLike<'a> for Object<'a> {}
impl<'a> ObjectRefLike<'a> for Object<'a> {
    fn cast_ref(obj: &Self) -> Option<&Self> {
        Some(obj)
    }
}

impl Skippable for Object<'_> {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        match r.peek_byte()? {
            b'n' => Null::skip(r, is_content_stream),
            b't' | b'f' => bool::skip(r, is_content_stream),
            b'/' => Name::skip(r, is_content_stream),
            b'<' => match r.peek_bytes(2)? {
                // A stream can never appear in a dict/array, so it should never be skipped.
                b"<<" => Dict::skip(r, is_content_stream),
                _ => String::skip(r, is_content_stream),
            },
            b'(' => String::skip(r, is_content_stream),
            b'.' | b'+' | b'-' | b'0'..=b'9' => Number::skip(r, is_content_stream),
            b'[' => Array::skip(r, is_content_stream),
            // See test case operator-in-TJ-array-0: Be lenient and skip content operators in
            // array
            _ => skip_name_like(r, false),
        }
    }
}

impl<'a> Readable<'a> for Object<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let object = match r.peek_byte()? {
            b'n' => Self::Null(Null::read(r, ctx)?),
            b't' | b'f' => Self::Boolean(bool::read(r, ctx)?),
            b'/' => Self::Name(Name::read(r, ctx)?),
            b'<' => match r.peek_bytes(2)? {
                b"<<" => {
                    let mut cloned = r.clone();
                    let dict = Dict::read(&mut cloned, ctx);
                    cloned.skip_white_spaces_and_comments();
                    let is_stream = cloned.forward_tag(b"stream").is_some();
                    // PdfCraft patch: what the clone looked at past the end counts for `r` too.
                    r.absorb_past_end(&cloned);
                    let dict = dict?;

                    if is_stream {
                        Object::Stream(Stream::read(r, ctx)?)
                    } else {
                        r.jump(cloned.offset());

                        Object::Dict(dict)
                    }
                }
                _ => Self::String(String::read(r, ctx)?),
            },
            b'(' => Self::String(String::read(r, ctx)?),
            b'.' | b'+' | b'-' | b'0'..=b'9' => Self::Number(Number::read(r, ctx)?),
            b'[' => Self::Array(Array::read(r, ctx)?),
            // See the comment in `skip`.
            _ => {
                skip_name_like(r, false)?;
                Self::Null(Null)
            }
        };

        Some(object)
    }
}

/// A trait for objects that can be parsed from a simple byte stream.
pub trait FromBytes<'a>: Sized {
    /// Try to read the object from the given bytes.
    fn from_bytes(b: &'a [u8]) -> Option<Self>;
}

impl<'a, T: Readable<'a>> FromBytes<'a> for T {
    fn from_bytes(b: &'a [u8]) -> Option<Self> {
        Self::from_bytes_impl(b)
    }
}

/// An identifier for a PDF object.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ObjectIdentifier {
    /// The object number.
    pub obj_number: i32,
    /// The generation number.
    pub gen_number: i32,
}

impl ObjectIdentifier {
    /// Create a new `ObjectIdentifier`.
    pub fn new(obj_number: i32, gen_number: i32) -> Self {
        Self {
            obj_number,
            gen_number,
        }
    }
}

impl Readable<'_> for ObjectIdentifier {
    fn read(r: &mut Reader<'_>, _: &ReaderContext<'_>) -> Option<Self> {
        let obj_number = r.read_without_context::<i32>()?;
        r.skip_white_spaces_and_comments();
        let gen_number = r.read_without_context::<i32>()?;
        r.skip_white_spaces_and_comments();
        r.forward_tag(b"obj")?;

        Some(Self {
            obj_number,
            gen_number,
        })
    }
}

impl Skippable for ObjectIdentifier {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        r.skip::<i32>(false)?;
        r.skip_white_spaces_and_comments();
        r.skip::<i32>(false)?;
        r.skip_white_spaces_and_comments();
        r.forward_tag(b"obj")?;

        Some(())
    }
}

/// A convenience function that extracts a dict and a stream from an object.
/// If the object is just a dictionary, it will return `None` for the stream.
/// If the object is a stream, it will return its dictionary as well as the stream
/// itself.
pub fn dict_or_stream<'a, 'b>(
    obj: &'b Object<'a>,
) -> Option<(&'b Dict<'a>, Option<&'b Stream<'a>>)> {
    if let Some(stream) = obj.cast_ref::<Stream<'a>>() {
        Some((stream.dict(), Some(stream)))
    } else {
        obj.cast_ref::<Dict<'a>>().map(|dict| (dict, None))
    }
}

mod macros {
    macro_rules! object {
        ($t:ident $(<$l:lifetime>),*, $s:ident) => {
            impl<'a> TryFrom<Object<'a>> for $t$(<$l>),* {
                type Error = ();

                fn try_from(value: Object<'a>) -> core::result::Result<Self, Self::Error> {
                    match value {
                        Object::$s(b) => Ok(b),
                        _ => Err(()),
                    }
                }
            }

            impl<'a> crate::object::ObjectLike<'a> for $t$(<$l>),* {}

            impl<'a> crate::object::ObjectRefLike<'a> for $t$(<$l>),* {
                fn cast_ref<'b>(obj: &'b crate::object::Object<'a>) -> Option<&'b Self> {
                    match obj {
                        crate::object::Object::$s(b) => Some(b),
                        _ => None,
                    }
                }
            }
        };
    }

    pub(crate) use object;
}

#[cfg(test)]
mod tests {
    use crate::object::{Array, Dict, Name, Null, Number, Object, Stream, String};
    use crate::reader::Reader;
    use crate::reader::{ReaderContext, ReaderExt};

    fn object_impl(data: &[u8]) -> Option<Object<'_>> {
        let mut r = Reader::new(data);
        r.read_with_context::<Object<'_>>(&ReaderContext::dummy())
    }

    #[test]
    fn null() {
        assert!(matches!(object_impl(b"null").unwrap(), Object::Null(_)));
    }

    #[test]
    fn bool() {
        assert!(matches!(object_impl(b"true").unwrap(), Object::Boolean(_)));
    }

    #[test]
    fn number() {
        assert!(matches!(object_impl(b"34.5").unwrap(), Object::Number(_)));
    }

    #[test]
    fn string_1() {
        assert!(matches!(object_impl(b"(Hi)").unwrap(), Object::String(_)));
    }

    #[test]
    fn string_2() {
        assert!(matches!(object_impl(b"<34>").unwrap(), Object::String(_)));
    }

    #[test]
    fn name() {
        assert!(matches!(object_impl(b"/Name").unwrap(), Object::Name(_)));
    }

    #[test]
    fn dict() {
        assert!(matches!(
            object_impl(b"<</Entry 45>>").unwrap(),
            Object::Dict(_)
        ));
    }

    #[test]
    fn array() {
        assert!(matches!(object_impl(b"[45]").unwrap(), Object::Array(_)));
    }

    #[test]
    fn stream() {
        assert!(matches!(
            object_impl(b"<< /Length 3 >> stream\nabc\nendstream").unwrap(),
            Object::Stream(_)
        ));
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn object_sizes() {
        assert_eq!(size_of::<Object<'_>>(), 40);
        assert_eq!(size_of::<Array<'_>>(), 32);
        assert_eq!(size_of::<Dict<'_>>(), 8);
        assert_eq!(size_of::<Name<'_>>(), 32);
        assert_eq!(size_of::<Null>(), 0);
        assert_eq!(size_of::<Number>(), 16);
        assert_eq!(size_of::<Stream<'_>>(), 24);
        assert_eq!(size_of::<String<'_>>(), 32);
    }
}
