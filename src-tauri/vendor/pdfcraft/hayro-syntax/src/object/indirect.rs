use crate::object::{ObjectIdentifier, ObjectLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};

#[derive(Debug, Clone)]
pub(crate) struct IndirectObject<T> {
    id: ObjectIdentifier,
    inner: T,
}

impl<T> IndirectObject<T> {
    pub(crate) fn get(self) -> T {
        self.inner
    }

    pub(crate) fn id(&self) -> &ObjectIdentifier {
        &self.id
    }
}

impl<'a, T> Readable<'a> for IndirectObject<T>
where
    T: ObjectLike<'a>,
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let mut ctx = ctx.clone();
        let id = r.read_without_context::<ObjectIdentifier>()?;

        if ctx.parent_chain_contains(&id) {
            warn!("cycle detected in indirect object: {id:?}");

            return None;
        }

        ctx.set_obj_number(id);
        ctx.parent_chain_push(id);
        r.skip_white_spaces_and_comments();
        let inner = r.read_with_context::<T>(&ctx)?;
        r.skip_white_spaces_and_comments();
        // We are lenient and don't require it.
        r.forward_tag(b"endobj");

        Some(Self { id, inner })
    }
}

impl<T> Skippable for IndirectObject<T>
where
    T: Skippable,
{
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        r.skip::<ObjectIdentifier>(false)?;
        r.skip_white_spaces_and_comments();
        r.skip::<T>(false)?;
        r.skip_white_spaces_and_comments();
        // We are lenient and don't require it.
        r.forward_tag(b"endobj");

        Some(())
    }
}
