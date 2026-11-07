//! Reading the pages of a PDF document.

use crate::content::{TypedIter, UntypedIter};
use crate::object::Array;
use crate::object::Dict;
use crate::object::Name;
use crate::object::Rect;
use crate::object::Stream;
use crate::object::dict::keys::*;
use crate::object::{Object, ObjectLike};
use crate::reader::ReaderContext;
use crate::sync::OnceLock;
use crate::transform::Transform;
use crate::util::FloatExt;
use crate::xref::XRef;
use crate::filter::{FlateState, MAX_DECODED_STREAM};
use crate::object::stream::Incremental;
use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::collections::{BTreeSet, VecDeque};
use crate::object::ObjectIdentifier;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Deref;

/// Attributes that can be inherited.
#[derive(Debug, Clone)]
struct PagesContext {
    media_box: Option<Rect>,
    crop_box: Option<Rect>,
    rotate: Option<i32>,
}

impl PagesContext {
    fn new() -> Self {
        Self {
            media_box: None,
            crop_box: None,
            rotate: None,
        }
    }
}

/// A structure holding the pages of a PDF document.
pub struct Pages<'a> {
    pages: Vec<Page<'a>>,
    xref: &'a XRef,
}

impl<'a> Pages<'a> {
    /// Create a new `Pages` object.
    pub(crate) fn new(
        pages_dict: &Dict<'a>,
        ctx: &ReaderContext<'a>,
        xref: &'a XRef,
    ) -> Option<Self> {
        let mut pages = vec![];
        let pages_ctx = PagesContext::new();
        // PdfCraft patch: guard against page-tree cycles (a `/Kids` entry pointing back up
        // the tree), which otherwise recurse until the stack overflows.
        let mut visited = BTreeSet::new();
        resolve_pages(
            pages_dict,
            &mut pages,
            pages_ctx,
            Resources::new(Dict::empty(), None, ctx),
            &mut visited,
            0,
        )?;

        Some(Self { pages, xref })
    }

    /// Create a new `Pages` object by bruteforce-searching.
    ///
    /// Of course this could result in the order of pages being messed up, but
    /// this is still better than nothing.
    pub(crate) fn new_brute_force(ctx: &ReaderContext<'a>, xref: &'a XRef) -> Option<Self> {
        let mut pages = vec![];

        for object in xref.objects() {
            if let Some(dict) = object.into_dict()
                && let Some(page) = Page::new(
                    &dict,
                    &PagesContext::new(),
                    Resources::new(Dict::empty(), None, ctx),
                    true,
                )
            {
                pages.push(page);
            }
        }

        if pages.is_empty() {
            return None;
        }

        Some(Self { pages, xref })
    }

    /// Return the xref table (of the document the pages belong to).   
    pub fn xref(&self) -> &'a XRef {
        self.xref
    }
}

impl<'a> Deref for Pages<'a> {
    type Target = [Page<'a>];

    fn deref(&self) -> &Self::Target {
        &self.pages
    }
}

/// PdfCraft patch: maximum page-tree depth (direct dictionaries have no id to track).
const MAX_PAGE_TREE_DEPTH: usize = 256;

/// PdfCraft patch: the largest `/UserUnit` honoured, as in Acrobat (a larger value counts as
/// this one).
const MAX_USER_UNIT: f64 = 75_000.0;

fn resolve_pages<'a>(
    pages_dict: &Dict<'a>,
    entries: &mut Vec<Page<'a>>,
    mut ctx: PagesContext,
    resources: Resources<'a>,
    visited: &mut BTreeSet<ObjectIdentifier>,
    depth: usize,
) -> Option<()> {
    // PdfCraft patch: skip nodes already visited and absurdly deep trees.
    if depth > MAX_PAGE_TREE_DEPTH {
        return None;
    }
    if let Some(id) = pages_dict.obj_id()
        && !visited.insert(id)
    {
        return None;
    }
    if let Some(media_box) = pages_dict.get::<Rect>(MEDIA_BOX) {
        ctx.media_box = Some(media_box);
    }

    if let Some(crop_box) = pages_dict.get::<Rect>(CROP_BOX) {
        ctx.crop_box = Some(crop_box);
    }

    if let Some(rotate) = pages_dict.get::<i32>(ROTATE) {
        ctx.rotate = Some(rotate);
    }

    let resources = Resources::for_page(
        pages_dict.contains_key(RESOURCES).then(|| pages_dict.get::<Dict<'_>>(RESOURCES).unwrap_or_default()),
        resources.clone(),
    );

    let kids = pages_dict.get::<Array<'a>>(KIDS)?;

    for dict in kids.iter::<Dict<'_>>() {
        match dict.get::<Name<'_>>(TYPE).as_deref() {
            Some(PAGES) => {
                resolve_pages(&dict, entries, ctx.clone(), resources.clone(), visited, depth + 1);
            }
            // Let's be lenient and assume it's a `Page` in case it's `None` or something else
            // (see corpus test case 0083781).
            _ => {
                if let Some(page) = Page::new(&dict, &ctx, resources.clone(), false) {
                    entries.push(page);
                }
            }
        }
    }

    Some(())
}

/// The rotation of the page.
#[derive(Debug, Copy, Clone)]
pub enum Rotation {
    /// No rotation.
    None,
    /// A rotation of 90 degrees.
    Horizontal,
    /// A rotation of 180 degrees.
    Flipped,
    /// A rotation of 270 degrees.
    FlippedHorizontal,
}

/// PdfCraft patch: the cut-off for content that expands from its compressed streams. The decoded
/// content of a page may be `MAX_CONTENT_EXPANSION` times the size of those streams, and never less
/// than `MAX_DECODED_STREAM`. Real content expands a few times, and a stream that inflates a
/// thousandfold from a few kilobytes is a decompression bomb.
const MAX_CONTENT_EXPANSION: usize = 64;

/// PdfCraft patch: the decoded content of a page, produced a piece at a time, so that the whole of
/// it need not be in memory at once (see `interpret_page`). Plain streams and Flate streams without
/// a predictor are decoded as they are read. Any other stream is decoded whole when the reader is
/// made, as `page_stream` does, and the decoded streams share one `MAX_DECODED_STREAM`.
///
/// The decoded bytes of the page's streams are also cut off at a budget: `MAX_CONTENT_EXPANSION`
/// times their compressed size, counting each distinct stream once however many times `/Contents`
/// names it. Content that reaches the budget is cut short, and [`ContentReader::truncated`] says so.
pub struct ContentReader<'a> {
    pieces: VecDeque<Piece<'a>>,
    budget: usize,
    produced: usize,
    materialized: usize,
    truncated: bool,
}

/// One part of the content of a page, in order.
enum Piece<'a> {
    /// A stream's own bytes, which are not decoded.
    Plain { data: Cow<'a, [u8]>, pos: usize },
    /// A Flate stream, decoded as it is read.
    Flate { data: Cow<'a, [u8]>, state: FlateState },
    /// A stream that was decoded whole.
    Whole { data: Vec<u8>, pos: usize },
    /// The space that separates the streams of a `/Contents` array.
    Separator { written: bool },
}

impl Piece<'_> {
    fn exhausted(&self) -> bool {
        match self {
            Piece::Plain { data, pos } => *pos >= data.len(),
            Piece::Flate { data, state } => state.finished(data),
            Piece::Whole { data, pos } => *pos >= data.len(),
            Piece::Separator { written } => *written,
        }
    }

    /// Appends the next bytes of the piece to `out`, which may end at `limit` bytes.
    fn append_to(&mut self, out: &mut Vec<u8>, limit: usize) {
        match self {
            Piece::Plain { data, pos } => copy_from(data, pos, out, limit),
            Piece::Flate { data, state } => state.decode_into(data, out, limit),
            Piece::Whole { data, pos } => copy_from(data, pos, out, limit),
            Piece::Separator { written } => {
                out.push(b' ');
                *written = true;
            }
        }
    }
}

/// Copies the bytes of `data` from `pos` to `out`, up to `limit` bytes in all.
fn copy_from(data: &[u8], pos: &mut usize, out: &mut Vec<u8>, limit: usize) {
    let rest = data.get(*pos..).unwrap_or(&[]);
    let take = rest.len().min(limit.saturating_sub(out.len()));
    out.extend_from_slice(rest.get(..take).unwrap_or(&[]));
    *pos += take;
}

impl<'a> ContentReader<'a> {
    /// A reader of no content.
    fn empty() -> Self {
        Self {
            pieces: VecDeque::new(),
            budget: 0,
            produced: 0,
            materialized: 0,
            truncated: false,
        }
    }

    /// A reader of `streams`, the content of a page in order. `separated` says whether they come
    /// from an array, whose streams are separated by spaces. `cap` is the most bytes that streams
    /// decoded whole may make in all, and at least the budget's floor.
    fn new(streams: Vec<Stream<'a>>, separated: bool, cap: usize) -> Self {
        let mut counted = BTreeSet::new();
        let mut compressed = 0_usize;
        let mut pieces = VecDeque::new();
        let mut materialized = 0_usize;
        // The bytes that the streams decoded whole take up, separators included, as `page_stream`
        // counted them against `MAX_DECODED_STREAM`.
        let mut whole_used = 0_usize;
        let mut accepting_pieces = true;
        for stream in streams {
            let newly_counted = counted.insert(stream.obj_id());
            if !accepting_pieces {
                if newly_counted {
                    compressed = compressed.saturating_add(stream.raw_data().len());
                }
                continue;
            }

            let incremental = stream.incremental();
            let left = cap.saturating_sub(whole_used);
            if matches!(&incremental, Incremental::Whole) && left == 0 {
                accepting_pieces = false;
                if newly_counted {
                    compressed = compressed.saturating_add(stream.raw_data().len());
                }
                continue;
            }

            let raw = stream.raw_data();
            if newly_counted {
                compressed = compressed.saturating_add(raw.len());
            }

            match incremental {
                Incremental::Plain => pieces.push_back(Piece::Plain {
                    data: raw,
                    pos: 0,
                }),
                Incremental::Flate => {
                    let state = FlateState::new(&raw);
                    pieces.push_back(Piece::Flate { data: raw, state });
                }
                Incremental::Whole => {
                    let Ok(data) = stream.decoded_raw_within(raw, left) else {
                        continue;
                    };
                    let data = data.into_owned();
                    whole_used = whole_used.saturating_add(data.len());
                    materialized = materialized.saturating_add(data.len());
                    pieces.push_back(Piece::Whole { data, pos: 0 });
                }
            }

            if separated {
                whole_used = whole_used.saturating_add(1);
                pieces.push_back(Piece::Separator { written: false });
            }
        }
        let budget = cap.max(MAX_CONTENT_EXPANSION.saturating_mul(compressed));

        Self {
            pieces,
            budget,
            produced: 0,
            materialized,
            truncated: false,
        }
    }

    /// Appends up to `max` bytes of the decoded content to `out`, and returns how many it appended.
    /// That is fewer than `max` only at the end of the content, or where it is cut short.
    pub fn read_into(&mut self, out: &mut Vec<u8>, max: usize) -> usize {
        let start = out.len();
        let end = start.saturating_add(max);

        while out.len() < end {
            let Some(piece) = self.pieces.front_mut() else {
                break;
            };
            if piece.exhausted() {
                self.pieces.pop_front();
                continue;
            }

            let allowed = self.budget.saturating_sub(self.produced);
            if allowed == 0 {
                self.truncated = true;
                self.pieces.clear();
                break;
            }

            let before = out.len();
            piece.append_to(out, end.min(before.saturating_add(allowed)));
            self.produced += out.len() - before;
        }

        out.len() - start
    }

    /// The bytes of the content that were decoded whole when the reader was made. The caller
    /// charges these against its own budget, as it did for the whole content.
    pub fn materialized_len(&self) -> usize {
        self.materialized
    }

    /// Whether the content was cut short at its budget (see [`ContentReader`]).
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// A PDF page.
pub struct Page<'a> {
    inner: Dict<'a>,
    media_box: Rect,
    crop_box: Rect,
    rotation: Rotation,
    user_unit: f32,
    page_streams: OnceLock<Option<Vec<u8>>>,
    resources: Resources<'a>,
    ctx: ReaderContext<'a>,
}

impl<'a> Page<'a> {
    fn new(
        dict: &Dict<'a>,
        ctx: &PagesContext,
        resources: Resources<'a>,
        brute_force: bool,
    ) -> Option<Self> {
        // In general, pages without content are allowed, but in case we are brute-forcing
        // we ignore them.
        if brute_force && !dict.contains_key(CONTENTS) {
            return None;
        }

        let media_box = dict.get::<Rect>(MEDIA_BOX).or(ctx.media_box).unwrap_or(A4);

        let crop_box = dict
            .get::<Rect>(CROP_BOX)
            .or(ctx.crop_box)
            .unwrap_or(media_box);

        let rotation = match dict
            .get::<i32>(ROTATE)
            .or(ctx.rotate)
            .unwrap_or(0)
            .rem_euclid(360)
        {
            0 => Rotation::None,
            90 => Rotation::Horizontal,
            180 => Rotation::Flipped,
            270 => Rotation::FlippedHorizontal,
            _ => Rotation::None,
        };

        // PdfCraft patch: the page's own `/UserUnit` (it isn't inherited), read as Acrobat reads
        // it: a number from 1 up, at most `MAX_USER_UNIT`; anything else counts as 1.
        let user_unit = dict
            .get::<f64>(USER_UNIT)
            .filter(|u| *u >= 1.0)
            .map_or(1.0, |u| u.min(MAX_USER_UNIT) as f32);

        let ctx = resources.ctx.clone();
        let resources = Resources::for_page(
            dict.contains_key(RESOURCES).then(|| dict.get::<Dict<'_>>(RESOURCES).unwrap_or_default()),
            resources,
        );

        Some(Self {
            inner: dict.clone(),
            media_box,
            crop_box,
            rotation,
            user_unit,
            page_streams: OnceLock::new(),
            resources,
            ctx,
        })
    }

    fn operations_impl(&self) -> Option<UntypedIter<'_>> {
        let stream = self.page_stream()?;
        let iter = UntypedIter::new(stream);

        Some(iter)
    }

    /// PdfCraft patch: the content of the page, to be read a piece at a time (see
    /// [`ContentReader`]).
    pub fn content_reader(&self) -> ContentReader<'a> {
        self.content_reader_within(MAX_DECODED_STREAM)
    }

    /// PdfCraft patch: [`Page::content_reader`], with `cap` in place of `MAX_DECODED_STREAM`.
    fn content_reader_within(&self, cap: usize) -> ContentReader<'a> {
        if let Some(stream) = self.inner.get::<Stream<'a>>(CONTENTS) {
            ContentReader::new(vec![stream], false, cap)
        } else if let Some(array) = self.inner.get::<Array<'a>>(CONTENTS) {
            ContentReader::new(array.iter::<Stream<'a>>().collect(), true, cap)
        } else {
            warn!("contents entry of page was neither stream nor array of streams");

            ContentReader::empty()
        }
    }

    /// Return the decoded content stream of the page.
    pub fn page_stream(&self) -> Option<&[u8]> {
        let convert_single = |s: Stream<'_>| {
            let data = s.decoded().ok()?;
            // PdfCraft patch: take the decoded buffer instead of copying it (up to
            // `MAX_DECODED_STREAM` twice over).
            Some(data.into_owned())
        };

        self.page_streams
            .get_or_init(|| {
                if let Some(stream) = self.inner.get::<Stream<'_>>(CONTENTS) {
                    convert_single(stream)
                } else if let Some(array) = self.inner.get::<Array<'_>>(CONTENTS) {
                    let mut collected = vec![];

                    for stream in array.iter::<Stream<'_>>() {
                        // PdfCraft patch: the streams share one `MAX_DECODED_STREAM` (an array
                        // naming one inflating stream twenty times decoded 5 GB).
                        let left = MAX_DECODED_STREAM.saturating_sub(collected.len());
                        if left == 0 {
                            break;
                        }
                        let Ok(data) = stream.decoded_within(left) else {
                            continue;
                        };
                        // Exactly what this stream and its separator need: growing by doubling
                        // here held 1 GB for a 256 MiB stream.
                        collected.reserve_exact(data.len() + 1);
                        collected.extend_from_slice(&data);
                        // Streams must have at least one whitespace in-between.
                        collected.push(b' ');
                    }

                    Some(collected)
                } else {
                    warn!("contents entry of page was neither stream nor array of streams");

                    None
                }
            })
            .as_ref()
            .map(|d| d.as_slice())
    }

    /// Get the resources of the page.
    pub fn resources(&self) -> &Resources<'a> {
        &self.resources
    }

    /// Get the media box of the page.
    pub fn media_box(&self) -> Rect {
        self.media_box
    }

    /// Get the rotation of the page.
    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// Get the crop box of the page.
    pub fn crop_box(&self) -> Rect {
        self.crop_box
    }

    /// PdfCraft patch: the size of the page's user-space unit in points (`/UserUnit`).
    pub fn user_unit(&self) -> f32 {
        self.user_unit
    }

    /// Return the intersection of crop box and media box.
    pub fn intersected_crop_box(&self) -> Rect {
        self.crop_box().intersect(self.media_box())
    }

    /// Return the base dimensions of the page (same as `intersected_crop_box`, but with special
    /// handling applied for zero-area pages).
    pub fn base_dimensions(&self) -> (f32, f32) {
        let crop_box = self.intersected_crop_box();

        if (crop_box.width() as f32).is_nearly_zero() || (crop_box.height() as f32).is_nearly_zero()
        {
            (A4.width() as f32, A4.height() as f32)
        } else {
            // PdfCraft patch: retain a one-point floor after UserUnit, not one raw unit.
            // The stored unit is finite and within 1..=75000. Round upward only if the
            // reciprocal's f32 product would fall below one and trigger a placeholder.
            let mut raw_floor = 1.0 / self.user_unit;
            if raw_floor * self.user_unit < 1.0 {
                raw_floor = raw_floor.next_up();
            }
            let raw_floor = f64::from(raw_floor);
            (
                crop_box.width().max(raw_floor) as f32,
                crop_box.height().max(raw_floor) as f32,
            )
        }
    }

    /// Return the with and height of the page that should be assumed when rendering the page.
    ///
    /// Depending on the document, it is either based on the media box or the crop box
    /// of the page. In addition to that, it also takes the rotation of the page into account.
    pub fn render_dimensions(&self) -> (f32, f32) {
        let (base_width, base_height) = self.base_dimensions();
        // PdfCraft patch: in points, after `/UserUnit`.
        let (mut base_width, mut base_height) =
            (base_width * self.user_unit, base_height * self.user_unit);

        if matches!(
            self.rotation(),
            Rotation::Horizontal | Rotation::FlippedHorizontal
        ) {
            core::mem::swap(&mut base_width, &mut base_height);
        }

        (base_width, base_height)
    }

    /// Return an untyped iterator over the operators of the page's content stream.
    pub fn operations(&self) -> UntypedIter<'_> {
        self.operations_impl().unwrap_or(UntypedIter::empty())
    }

    /// Get the raw dictionary of the page.
    pub fn raw(&self) -> &Dict<'a> {
        &self.inner
    }

    /// Get the xref table (of the document the page belongs to).
    pub fn xref(&self) -> &'a XRef {
        self.ctx.xref()
    }

    /// Return a typed iterator over the operators of the page's content stream.
    pub fn typed_operations(&self) -> TypedIter<'_> {
        TypedIter::from_untyped(self.operations())
    }

    /// Return the initial transform that should be applied when rendering.
    ///
    /// This accounts for the mismatch between PDF's y-up and most renderers'
    /// y-down coordinate system, the rotation of the page and the offset of
    /// the crop box.
    pub fn initial_transform(&self, invert_y: bool) -> Transform {
        let crop_box = self.intersected_crop_box();
        let (_, base_height) = self.base_dimensions();
        // PdfCraft patch: the page in points, after `/UserUnit`.
        let base_height = base_height * self.user_unit;
        let (width, height) = self.render_dimensions();

        let horizontal_t = Transform::ROTATE_CW_90 * Transform::translate((0.0, -width as f64));
        let flipped_horizontal_t =
            Transform::translate((0.0, height as f64)) * Transform::ROTATE_CCW_90;

        let rotation_transform = match self.rotation() {
            Rotation::None => Transform::IDENTITY,
            Rotation::Horizontal => {
                if invert_y {
                    horizontal_t
                } else {
                    flipped_horizontal_t
                }
            }
            Rotation::Flipped => {
                Transform::scale(-1.0) * Transform::translate((-width as f64, -height as f64))
            }
            Rotation::FlippedHorizontal => {
                if invert_y {
                    flipped_horizontal_t
                } else {
                    horizontal_t
                }
            }
        };

        let inversion_transform = if invert_y {
            Transform::new([1.0, 0.0, 0.0, -1.0, 0.0, base_height as f64])
        } else {
            Transform::IDENTITY
        };

        let user_space = Transform::translate((-crop_box.x0, -crop_box.y0));
        // PdfCraft patch: user space in units of `/UserUnit` points. A page without it keeps
        // exactly the transform it had.
        let user_space = if self.user_unit == 1.0 {
            user_space
        } else {
            Transform::scale(f64::from(self.user_unit)) * user_space
        };

        rotation_transform * inversion_transform * user_space
    }
}

/// A structure keeping track of the resources of a page.
#[derive(Clone, Debug)]
pub struct Resources<'a> {
    parent: Option<Box<Self>>,
    ctx: ReaderContext<'a>,
    // PdfCraft patch: legacy forms without Resources use the page, not their caller.
    page_properties: Dict<'a>,
    /// The raw dictionary of external graphics states.
    pub ext_g_states: Dict<'a>,
    /// The raw dictionary of fonts.
    pub fonts: Dict<'a>,
    /// The effective property dictionary selected at page or form entry.
    pub properties: Dict<'a>,
    /// The raw dictionary of color spaces.
    pub color_spaces: Dict<'a>,
    /// The raw dictionary of x objects.
    pub x_objects: Dict<'a>,
    /// The raw dictionary of patterns.
    pub patterns: Dict<'a>,
    /// The raw dictionary of shadings.
    pub shadings: Dict<'a>,
}

impl<'a> Resources<'a> {
    /// Create a new `Resources` object from a dictionary with a parent.
    pub fn from_parent(resources: Dict<'a>, parent: Self) -> Self {
        let ctx = parent.ctx.clone();

        Self::new(resources, Some(parent), &ctx)
    }

    // PdfCraft patch: page-tree inheritance stops at any present Resources dictionary.
    fn for_page(resources: Option<Dict<'a>>, parent: Self) -> Self {
        let properties = resources.as_ref().map_or_else(|| parent.properties.clone(), |dict| dict.get::<Dict<'_>>(PROPERTIES).unwrap_or_default());
        let mut result = Self::from_parent(resources.unwrap_or_default(), parent);
        result.properties = properties.clone();
        result.page_properties = properties;
        result
    }

    /// Create form resources, using page Properties only when Resources is absent.
    /// Other resource categories retain their existing parent-chain lookup.
    // PdfCraft patch: ISO 32000-2, 7.8.3 and Table 93 require the page origin.
    pub fn for_form(resources: Option<Dict<'a>>, parent: Self) -> Self {
        let properties =
            resources.as_ref().map_or_else(|| parent.page_properties.clone(), |dict| dict.get::<Dict<'_>>(PROPERTIES).unwrap_or_default());
        let mut result = Self::from_parent(resources.unwrap_or_default(), parent);
        result.properties = properties;
        result
    }

    /// Create a new `Resources` object.
    pub(crate) fn new(resources: Dict<'a>, parent: Option<Self>, ctx: &ReaderContext<'a>) -> Self {
        let ext_g_states = resources.get::<Dict<'_>>(EXT_G_STATE).unwrap_or_default();
        let fonts = resources.get::<Dict<'_>>(FONT).unwrap_or_default();
        let color_spaces = resources.get::<Dict<'_>>(COLORSPACE).unwrap_or_default();
        let x_objects = resources.get::<Dict<'_>>(XOBJECT).unwrap_or_default();
        let patterns = resources.get::<Dict<'_>>(PATTERN).unwrap_or_default();
        let shadings = resources.get::<Dict<'_>>(SHADING).unwrap_or_default();
        let properties = resources.get::<Dict<'_>>(PROPERTIES).unwrap_or_default();

        // PdfCraft patch: generic child resources retain the page origin.
        let page_properties = parent.as_ref().map_or_else(|| properties.clone(), |parent| parent.page_properties.clone());
        let parent = parent.map(Box::new);

        Self {
            parent,
            ext_g_states,
            fonts,
            color_spaces,
            properties,
            page_properties,
            x_objects,
            patterns,
            shadings,
            ctx: ctx.clone(),
        }
    }

    fn get_resource<T: ObjectLike<'a>>(&self, name: &Name<'_>, dict: &Dict<'a>) -> Option<T> {
        dict.get::<T>(name.deref())
    }

    /// Get the parent in the resource, chain, if available.
    pub fn parent(&self) -> Option<&Self> {
        self.parent.as_deref()
    }

    /// Get an external graphics state by name.
    pub fn get_ext_g_state(&self, name: &Name<'_>) -> Option<Dict<'a>> {
        self.get_resource::<Dict<'_>>(name, &self.ext_g_states)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_ext_g_state(name)))
    }

    /// Get a color space by name.
    pub fn get_color_space(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.get_resource::<Object<'_>>(name, &self.color_spaces)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_color_space(name)))
    }

    /// Get a font by name.
    pub fn get_font(&self, name: &Name<'_>) -> Option<Dict<'a>> {
        self.get_resource::<Dict<'_>>(name, &self.fonts)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_font(name)))
    }

    /// Get a pattern by name.
    pub fn get_pattern(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.get_resource::<Object<'_>>(name, &self.patterns)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_pattern(name)))
    }

    /// Get an x object by name.
    pub fn get_x_object(&self, name: &Name<'_>) -> Option<Stream<'a>> {
        self.get_resource::<Stream<'_>>(name, &self.x_objects)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_x_object(name)))
    }

    /// Get a shading by name.
    pub fn get_shading(&self, name: &Name<'_>) -> Option<Object<'a>> {
        self.get_resource::<Object<'_>>(name, &self.shadings)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get_shading(name)))
    }
}

// <https://github.com/apache/pdfbox/blob/a53a70db16ea3133994120bcf1e216b9e760c05b/pdfbox/src/main/java/org/apache/pdfbox/pdmodel/common/PDRectangle.java#L38>
const POINTS_PER_INCH: f64 = 72.0;
const POINTS_PER_MM: f64 = 1.0 / (10.0 * 2.54) * POINTS_PER_INCH;

/// The dimension of an A4 page.
pub const A4: Rect = Rect {
    x0: 0.0,
    y0: 0.0,
    x1: 210.0 * POINTS_PER_MM,
    y1: 297.0 * POINTS_PER_MM,
};

pub(crate) mod cached {
    use crate::page::Pages;
    use crate::reader::ReaderContext;
    use crate::xref::XRef;
    use core::ops::Deref;

    // Keep in sync with the implementation in `sync`. We duplicate it here
    // to make it more visible since we have unsafe code here.
    #[cfg(feature = "std")]
    pub(crate) use std::sync::Arc;

    #[cfg(not(feature = "std"))]
    pub(crate) use alloc::rc::Rc as Arc;

    pub(crate) struct CachedPages {
        pages: Pages<'static>,
        // NOTE: `pages` references the data in `xref`, so it's important that `xref`
        // appears after `pages` in the struct definition to ensure correct drop order.
        _xref: Arc<XRef>,
    }

    impl CachedPages {
        pub(crate) fn new(xref: Arc<XRef>) -> Option<Self> {
            // SAFETY:
            // - The XRef's location is stable in memory:
            //   - We wrapped it in a `Arc` (or `Rc` in `no_std`), which implements `StableDeref`.
            //   - The struct owns the `Arc`, ensuring that the inner value is not dropped during the whole
            //     duration.
            // - The internal 'static lifetime is not leaked because its rewritten
            //   to the self-lifetime in `pages()`.
            let xref_reference: &'static XRef = unsafe { core::mem::transmute(xref.deref()) };

            let ctx = ReaderContext::new(xref_reference, false);
            let pages = xref_reference
                .get_with(xref.trailer_data().pages_ref, &ctx)
                .and_then(|p| Pages::new(&p, &ctx, xref_reference))
                .or_else(|| Pages::new_brute_force(&ctx, xref_reference))?;

            Some(Self { pages, _xref: xref })
        }

        pub(crate) fn get(&self) -> &Pages<'_> {
            &self.pages
        }
    }
}

#[cfg(test)]
mod content_reader_tests {
    use crate::Pdf;

    /// A zlib stream of `data` in stored blocks, which expands by nothing.
    fn stored_zlib(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let mut chunks = data.chunks(0xffff).peekable();
        if chunks.peek().is_none() {
            out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
        }
        while let Some(chunk) = chunks.next() {
            out.push(u8::from(chunks.peek().is_none()));
            let len = u16::try_from(chunk.len()).expect("a stored block holds at most 65535 bytes");
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(chunk);
        }
        out.extend_from_slice(&[0; 4]);
        out
    }

    /// A zlib stream of one literal byte and then `count` copies of a 258-byte match from it, in
    /// fixed Huffman codes: about 160 bytes of output for each byte of input.
    fn fixed_run_bomb(count: usize) -> Vec<u8> {
        struct Bits {
            out: Vec<u8>,
            buf: u8,
            len: u32,
        }
        impl Bits {
            /// Bits are sent least significant first.
            fn put(&mut self, value: u32, bits: u32) {
                for i in 0..bits {
                    self.buf |= (((value >> i) & 1) as u8) << self.len;
                    self.len += 1;
                    if self.len == 8 {
                        self.out.push(self.buf);
                        self.buf = 0;
                        self.len = 0;
                    }
                }
            }

            /// Huffman codes are sent most significant bit first.
            fn code(&mut self, code: u32, bits: u32) {
                for i in (0..bits).rev() {
                    self.put((code >> i) & 1, 1);
                }
            }
        }

        let mut bits = Bits {
            out: vec![0x78, 0x01],
            buf: 0,
            len: 0,
        };
        bits.put(1, 1); // The final block.
        bits.put(1, 2); // Fixed Huffman codes.
        bits.code(0x30 + u32::from(b'a'), 8); // The literal 'a'.
        for _ in 0..count {
            bits.code(0xc5, 8); // Length 258 (code 285), no extra bits.
            bits.code(0, 5); // Distance 1 (code 0), no extra bits.
        }
        bits.code(0, 7); // End of block (code 256).
        if bits.len > 0 {
            bits.out.push(bits.buf);
        }
        bits.out.extend_from_slice(&[0; 4]);
        bits.out
    }

    /// A one-page PDF whose content is `stream`, with the stream dictionary entries `dict`.
    fn page_pdf(stream: &[u8], dict: &str) -> Vec<u8> {
        let mut pdf = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R >> endobj\n4 0 obj << {dict} /Length {} >> stream\n",
            stream.len()
        )
        .into_bytes();
        pdf.extend_from_slice(stream);
        pdf.extend_from_slice(b"\nendstream endobj\ntrailer << /Root 1 0 R >>\n%%EOF\n");
        pdf
    }

    /// A one-page PDF with an array of content references to `streams`.
    fn page_pdf_array(streams: &[(&[u8], &str)], refs: &[usize]) -> Vec<u8> {
        let contents = refs
            .iter()
            .map(|index| format!("{} 0 R", 4 + index))
            .collect::<Vec<_>>()
            .join(" ");
        let mut pdf = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents [{contents}] >> endobj\n"
        )
        .into_bytes();
        for (index, (stream, dict)) in streams.iter().enumerate() {
            let object = 4 + index;
            pdf.extend_from_slice(
                format!("{object} 0 obj << {dict} /Length {} >> stream\n", stream.len())
                    .as_bytes(),
            );
            pdf.extend_from_slice(stream);
            pdf.extend_from_slice(b"\nendstream endobj\n");
        }
        pdf.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF\n");
        pdf
    }

    /// The content of the first page, read `piece` bytes at a time, with a budget of `cap`.
    fn read_content(pdf: Vec<u8>, cap: usize, piece: usize) -> (Vec<u8>, bool) {
        let doc = Pdf::new(pdf).expect("parses");
        let page = doc.pages().first().expect("one page");
        let mut content = page.content_reader_within(cap);
        let mut out = Vec::new();
        while content.read_into(&mut out, piece) > 0 {}

        (out, content.truncated())
    }

    #[test]
    fn plain_and_whole_pieces_keep_their_bytes() {
        let plain = b"q 1 0 0 1 0 0 cm Q\n";
        let whole = b"q 0 0 m Q\n";
        let encoded: String = whole.iter().map(|byte| format!("{byte:02x}")).collect::<String>() + ">";

        for piece in [1, 7, 4096, 65_536] {
            let (plain_out, plain_truncated) = read_content(page_pdf(plain, ""), 1 << 20, piece);
            assert_eq!(plain_out, plain, "plain piece {piece}");
            assert!(!plain_truncated, "plain piece {piece}");

            let (whole_out, whole_truncated) = read_content(
                page_pdf(encoded.as_bytes(), "/Filter /ASCIIHexDecode"),
                1 << 20,
                piece,
            );
            assert_eq!(whole_out, whole, "whole piece {piece}");
            assert!(!whole_truncated, "whole piece {piece}");
        }
    }

    #[test]
    fn repeated_content_reference_counts_compressed_bytes_once() {
        let data = vec![b'q'; 1 << 16];
        let pdf = page_pdf_array(&[(data.as_slice(), "")], &[0, 0]);
        let doc = Pdf::new(pdf).expect("parses");
        let page = doc.pages().first().expect("one page");
        let mut content = page.content_reader_within(1 << 20);
        let expected_budget = 64 * data.len();
        assert_eq!(content.budget, expected_budget);

        let mut out = Vec::new();
        while content.read_into(&mut out, 4096) > 0 {}
        let mut expected = data.clone();
        expected.push(b' ');
        expected.extend_from_slice(&data);
        expected.push(b' ');
        assert!(
            out == expected,
            "expected {} bytes, got {}",
            expected.len(),
            out.len()
        );
        assert!(!content.truncated());
    }

    #[test]
    fn whole_cap_still_counts_later_unique_streams() {
        let first = vec![b'q'; 1 << 20];
        let later = vec![b'Q'; 12_000];
        let hex = |data: &[u8]| {
            data.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
                + ">"
        };
        let first_raw = hex(&first);
        let later_raw = hex(&later);
        let pdf = page_pdf_array(
            &[
                (first_raw.as_bytes(), "/Filter /ASCIIHexDecode"),
                (later_raw.as_bytes(), "/Filter /ASCIIHexDecode"),
            ],
            &[0, 1],
        );
        let cap = 1 << 20;
        let doc = Pdf::new(pdf).expect("parses");
        let page = doc.pages().first().expect("one page");
        let mut content = page.content_reader_within(cap);
        let expected_budget = 64 * (first_raw.len() + later_raw.len());
        assert!(expected_budget > cap);
        assert_eq!(content.budget, expected_budget);
        assert_eq!(content.materialized_len(), cap);
        assert_eq!(content.pieces.len(), 2, "only the first whole stream and separator are kept");

        let mut out = Vec::new();
        while content.read_into(&mut out, 65_536) > 0 {}
        let mut expected = first;
        expected.push(b' ');
        assert_eq!(out, expected);
        assert!(!content.truncated());
    }

    #[test]
    fn content_past_the_cap_is_read_to_the_end() {
        let content: Vec<u8> = b"q 1 0 0 1 0 0 cm Q\n".iter().copied().cycle().take(3 << 20).collect();
        let pdf = page_pdf(&stored_zlib(&content), "/Filter /FlateDecode");
        // The cap is far below the content, but stored data expands by nothing, so nothing is cut.
        for piece in [7, 4096, 1 << 16] {
            let (out, truncated) = read_content(pdf.clone(), 1 << 20, piece);
            assert_eq!(out.len(), content.len(), "piece {piece}");
            assert!(out == content, "piece {piece}: the bytes are the content's");
            assert!(!truncated, "piece {piece}");
        }
    }

    #[test]
    fn a_stream_that_expands_past_the_budget_is_cut_there() {
        let cap = 1 << 20;
        let bomb = fixed_run_bomb(400_000);
        let (out, truncated) = read_content(page_pdf(&bomb, "/Filter /FlateDecode"), cap, 1 << 16);

        assert!(truncated, "the expansion is past the budget");
        // The budget is 64 times the compressed stream, which is more than the cap here.
        assert_eq!(out.len(), 64 * bomb.len());
        assert!(out.iter().all(|&b| b == b'a'));
    }

    #[test]
    fn a_small_expansion_under_the_cap_is_not_cut() {
        let bomb = fixed_run_bomb(1000);
        let (out, truncated) = read_content(page_pdf(&bomb, "/Filter /FlateDecode"), 1 << 20, 4096);

        assert!(!truncated);
        assert_eq!(out.len(), 1 + 258 * 1000);
    }
}
