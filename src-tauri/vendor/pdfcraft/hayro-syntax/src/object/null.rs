//! The null object.

use crate::object::Object;
use crate::object::macros::object;
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, Skippable};

/// The null object.
#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct Null;

object!(Null, Null);

impl Skippable for Null {
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        r.forward_tag(b"null")
    }
}

impl Readable<'_> for Null {
    fn read(r: &mut Reader<'_>, ctx: &ReaderContext<'_>) -> Option<Self> {
        Self::skip(r, ctx.in_content_stream())?;

        Some(Self)
    }
}

#[cfg(test)]
mod tests {
    use crate::object::Null;
    use crate::reader::Reader;
    use crate::reader::ReaderExt;

    #[test]
    fn null() {
        assert_eq!(
            Reader::new("null".as_bytes())
                .read_without_context::<Null>()
                .unwrap(),
            Null
        );
    }

    #[test]
    fn null_trailing() {
        assert_eq!(
            Reader::new("nullabs".as_bytes())
                .read_without_context::<Null>()
                .unwrap(),
            Null
        );
    }
}
