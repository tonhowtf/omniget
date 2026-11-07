use crate::object::r#ref::MaybeRef;
use crate::object::{Object, ObjectLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt};
use core::fmt::Debug;
// Note that tuples don't correspond to any specific PDF object. Instead, they simply
// represent a number of PDF objects that are only separated by whitespaces, i.e.
// in an array. We only have those implementations so that it is easier to iterate
// over tuples of items in a PDF array, which happens quite often.

impl<'a, T, U> Readable<'a> for (T, U)
where
    T: ObjectLike<'a>,
    U: ObjectLike<'a>,
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        r.skip_white_spaces_and_comments();
        let t = r.read::<MaybeRef<T>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let u = r.read::<MaybeRef<U>>(ctx)?.resolve(ctx)?;
        Some((t, u))
    }
}

impl<'a, T, U> TryFrom<Object<'a>> for (T, U)
where
    T: ObjectLike<'a>,
    U: ObjectLike<'a>,
{
    type Error = ();

    fn try_from(_: Object<'a>) -> Result<Self, Self::Error> {
        Err(())
    }
}

impl<'a, T, U> ObjectLike<'a> for (T, U)
where
    T: ObjectLike<'a> + Debug + Clone,
    U: ObjectLike<'a> + Debug + Clone,
{
    const GROUP: bool = true;
}

impl<'a, T, U, V> Readable<'a> for (T, U, V)
where
    T: ObjectLike<'a>,
    U: ObjectLike<'a>,
    V: ObjectLike<'a>,
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        r.skip_white_spaces_and_comments();
        let t = r.read::<MaybeRef<T>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let u = r.read::<MaybeRef<U>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let v = r.read::<MaybeRef<V>>(ctx)?.resolve(ctx)?;

        Some((t, u, v))
    }
}

impl<'a, T, U, V> TryFrom<Object<'a>> for (T, U, V)
where
    T: ObjectLike<'a>,
    U: ObjectLike<'a>,
    V: ObjectLike<'a>,
{
    type Error = ();

    fn try_from(_: Object<'a>) -> Result<Self, Self::Error> {
        Err(())
    }
}

impl<'a, T, U, V> ObjectLike<'a> for (T, U, V)
where
    T: ObjectLike<'a> + Debug + Clone,
    U: ObjectLike<'a> + Debug + Clone,
    V: ObjectLike<'a> + Debug + Clone,
{
    const GROUP: bool = true;
}

impl<'a, T, U, V, W> Readable<'a> for (T, U, V, W)
where
    T: ObjectLike<'a>,
    U: ObjectLike<'a>,
    V: ObjectLike<'a>,
    W: ObjectLike<'a>,
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        r.skip_white_spaces_and_comments();
        let t = r.read::<MaybeRef<T>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let u = r.read::<MaybeRef<U>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let v = r.read::<MaybeRef<V>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let w = r.read::<MaybeRef<W>>(ctx)?.resolve(ctx)?;

        Some((t, u, v, w))
    }
}

impl<'a, T, U, V, W> TryFrom<Object<'a>> for (T, U, V, W)
where
    T: ObjectLike<'a>,
    U: ObjectLike<'a>,
    V: ObjectLike<'a>,
    W: ObjectLike<'a>,
{
    type Error = ();

    fn try_from(_: Object<'a>) -> Result<Self, Self::Error> {
        Err(())
    }
}

impl<'a, T, U, V, W> ObjectLike<'a> for (T, U, V, W)
where
    T: ObjectLike<'a> + Debug + Clone,
    U: ObjectLike<'a> + Debug + Clone,
    V: ObjectLike<'a> + Debug + Clone,
    W: ObjectLike<'a> + Debug + Clone,
{
    const GROUP: bool = true;
}

impl<'a, T, U, V, W, X> Readable<'a> for (T, U, V, W, X)
where
    T: ObjectLike<'a>,
    U: ObjectLike<'a>,
    V: ObjectLike<'a>,
    W: ObjectLike<'a>,
    X: ObjectLike<'a>,
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        r.skip_white_spaces_and_comments();
        let t = r.read::<MaybeRef<T>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let u = r.read::<MaybeRef<U>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let v = r.read::<MaybeRef<V>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let w = r.read::<MaybeRef<W>>(ctx)?.resolve(ctx)?;
        r.skip_white_spaces_and_comments();
        let x = r.read::<MaybeRef<X>>(ctx)?.resolve(ctx)?;

        Some((t, u, v, w, x))
    }
}

impl<'a, T, U, V, W, X> TryFrom<Object<'a>> for (T, U, V, W, X)
where
    T: ObjectLike<'a>,
    U: ObjectLike<'a>,
    V: ObjectLike<'a>,
    W: ObjectLike<'a>,
    X: ObjectLike<'a>,
{
    type Error = ();

    fn try_from(_: Object<'a>) -> Result<Self, Self::Error> {
        Err(())
    }
}

impl<'a, T, U, V, W, X> ObjectLike<'a> for (T, U, V, W, X)
where
    T: ObjectLike<'a> + Debug + Clone,
    U: ObjectLike<'a> + Debug + Clone,
    V: ObjectLike<'a> + Debug + Clone,
    W: ObjectLike<'a> + Debug + Clone,
    X: ObjectLike<'a> + Debug + Clone,
{
    const GROUP: bool = true;
}
