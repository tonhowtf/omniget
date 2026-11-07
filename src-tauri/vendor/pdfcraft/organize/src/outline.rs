//! Editing bookmarks (the document outline, ISO 32000-2 §12.3.3), execution plan M4.6.
//!
//! Bookmarks are addressed by their **path**: child indices from the top level, so `[0, 2]` is
//! the third child of the first top-level bookmark. Paths follow `/First`–`/Next` order, which
//! is also the order viewers (and our inspector) list them in.
//!
//! Edits are surgical: they relink only the affected siblings (`/Parent`, `/Prev`, `/Next`,
//! `/First`, `/Last`) and recompute `/Count` values, and an object is rewritten only when one of
//! its values changes. Everything else on an item (colour, style, actions, unknown keys) is kept.
//! Walks are cycle-safe and iterative, so a broken or deep outline is never followed forever and
//! never overflows the stack. Listing is lazy: [`bookmark_page`] visits only the bookmarks up to
//! the end of the page it returns.

use std::collections::{HashMap, HashSet};

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::{OrganizeError, walk};

/// Deepest level a listing enters (the top level is 0). Bookmarks deeper than this are not
/// listed; edits still reach them by path.
pub(crate) const LIST_DEPTH: usize = 32;

/// Deepest name-tree node read when looking up named destinations.
const NAME_TREE_DEPTH: usize = 32;

/// Deepest chain of indirect or named destinations followed for one bookmark.
const DEST_DEPTH: u8 = 8;

/// A bookmark in listing order, with where it sits and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub obj: ObjRef,
    /// Child indices from the top level (the paths the edit functions take).
    pub path: Vec<usize>,
    pub title: String,
    /// Shown expanded (a positive `/Count`) when it has children.
    pub open: bool,
    /// The 0-based page it goes to, when that page is in this document.
    pub page: Option<usize>,
    /// Where on that page it goes: the destination's type (`XYZ`, `Fit`, `FitH`…) and the numbers
    /// after it, in PDF user space (`None` for `null` or anything that isn't a number).
    pub view: Option<(Vec<u8>, Vec<Option<f64>>)>,
    /// The title's colour (`/C`, DeviceRGB, ISO 32000-2 §12.3.3 Table 151), each component kept
    /// within 0–1; `None` for the default black or a value that isn't three numbers. The
    /// inspector's `OutlineItem::color` reads it the same way.
    pub color: Option<[f32; 3]>,
    /// The title's style (`/F`, Table 152): bit 1 italic, bit 2 bold.
    pub italic: bool,
    pub bold: bool,
}

/// One page of the outline listing (see [`bookmark_page`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BookmarkPage {
    pub bookmarks: Vec<Bookmark>,
    /// The offset where the next page starts, or `None` when this page is the last.
    pub next: Option<usize>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum OutlineError {
    #[error("there is no bookmark at {0:?}")]
    NoSuchBookmark(Vec<usize>),
    #[error("a bookmark cannot be moved inside itself")]
    IntoItself,
    #[error("the title must not be empty")]
    EmptyTitle,
    #[error("a bookmark's position must be a finite number")]
    BadPosition,
    #[error("a bookmark's colour must be three numbers from 0 to 1")]
    BadColor,
    #[error("this document has no tagged headings to make bookmarks from")]
    NoEntries,
    #[error("there are no bookmarks to add")]
    NothingToImport,
    #[error("the bookmark at {0:?} has a view that is not a known destination type with its values")]
    BadView(Vec<usize>),
    #[error("bookmarks may nest at most {0} levels deep")]
    TooDeep(usize),
    #[error("at most {0} bookmarks can be added at once")]
    TooManyBookmarks(usize),
    #[error("{0}")]
    Organize(#[from] OrganizeError),
    #[error("{0}")]
    Cos(#[from] pdfcraft_cos::CosError),
}

type Result<T> = std::result::Result<T, OutlineError>;

fn outline_root(doc: &Document) -> Option<ObjRef> {
    let root = doc.root()?;
    doc.get(root).as_dict()?.reference(b"Outlines")
}

/// The children of `parent` (an item or the outline root), in order. Cycle-safe.
fn children_of(doc: &Document, parent: ObjRef, seen: &mut HashSet<ObjRef>) -> Vec<ObjRef> {
    let mut out = Vec::new();
    let mut next = doc.get(parent).as_dict().and_then(|d| d.reference(b"First"));
    while let Some(r) = next {
        if !seen.insert(r) {
            break;
        }
        let item = doc.get(r);
        let Some(d) = item.as_dict() else { break };
        out.push(r);
        next = d.reference(b"Next");
    }
    out
}

/// Every bookmark item, parents before their children. Cycle-safe.
pub(crate) fn items(doc: &Document) -> Vec<ObjRef> {
    let Some(root) = outline_root(doc) else { return Vec::new() };
    let (mut seen, mut out, mut stack) = (HashSet::new(), Vec::new(), vec![root]);
    while let Some(parent) = stack.pop() {
        let kids = children_of(doc, parent, &mut seen);
        stack.extend(kids.iter().copied());
        out.extend(kids);
    }
    out
}

/// The outline in listing order: each bookmark once, siblings by `/Next`, children right after
/// their parent. An explicit stack keeps the walk iterative. A repeated or malformed item ends its
/// run of siblings there, so a cycle or a broken link stops the walk instead of looping.
struct Visit<'a> {
    doc: &'a Document,
    seen: HashSet<ObjRef>,
    /// One entry per open level: the next sibling to visit, and how many siblings it has visited.
    levels: Vec<(Option<ObjRef>, usize)>,
    /// The deepest level the walk enters.
    max_depth: usize,
    /// The bookmark just visited: its children are entered by the next step.
    enter: Option<ObjRef>,
}

impl<'a> Visit<'a> {
    fn new(doc: &'a Document, max_depth: usize) -> Option<Self> {
        let root = outline_root(doc)?;
        let first = doc.get(root).as_dict().and_then(|d| d.reference(b"First"));
        Some(Self { doc, seen: HashSet::from([root]), levels: vec![(first, 0)], max_depth, enter: None })
    }

    /// Visit the next bookmark, or `None` when the listing is done.
    fn step(&mut self) -> Option<ObjRef> {
        if let Some(first) = self.enter.take() {
            self.levels.push((Some(first), 0));
        }
        loop {
            let depth = self.levels.len().checked_sub(1)?;
            let level = self.levels.get_mut(depth)?;
            let Some(r) = level.0.take() else {
                self.levels.pop();
                continue;
            };
            if !self.seen.insert(r) {
                continue;
            }
            let item = self.doc.get(r);
            let Some(d) = item.as_dict() else { continue };
            level.0 = d.reference(b"Next");
            level.1 = level.1.saturating_add(1);
            self.enter = if depth < self.max_depth { d.reference(b"First") } else { None };
            return Some(r);
        }
    }

    /// Child indices from the top level to the bookmark last visited.
    fn path(&self) -> Vec<usize> {
        self.levels.iter().map(|(_, visited)| visited.saturating_sub(1)).collect()
    }
}

/// Page targets for a listing: the page index of each page object, and the named destinations
/// (read once, the first time one is needed).
struct Targets {
    pages: HashMap<ObjRef, usize>,
    names: Option<HashMap<Vec<u8>, Object>>,
}

impl Targets {
    fn new(doc: &Document) -> Self {
        let pages = walk(doc).unwrap_or_default().into_iter().enumerate().map(|(i, (r, _))| (r, i)).collect();
        Self { pages, names: None }
    }

    /// The named destination `key`: its name-tree entry, else its entry in the `/Dests` dictionary.
    fn named(&mut self, doc: &Document, key: &[u8]) -> Option<Object> {
        let names = self.names.get_or_insert_with(|| name_tree(doc));
        names.get(key).cloned().or_else(|| dests_dict(doc, key))
    }
}

/// Every entry of the catalog's `/Names /Dests` name tree, by name. The first entry of a name wins.
/// Iterative, and nodes deeper than `NAME_TREE_DEPTH` are skipped.
fn name_tree(doc: &Document) -> HashMap<Vec<u8>, Object> {
    let mut out = HashMap::new();
    let Some(root) = doc.root() else { return out };
    let Some(names) = doc.get(root).as_dict().and_then(|c| c.get(b"Names").cloned()) else { return out };
    let names = doc.resolve(&names);
    let Some(tree) = names.as_dict().and_then(|n| n.get(b"Dests").cloned()) else { return out };
    // Nodes still to read, with their depth. Kids go on the stack in reverse, so they are read in order.
    let mut stack = vec![(tree, 0usize)];
    let mut seen: HashSet<ObjRef> = HashSet::new();
    while let Some((raw, depth)) = stack.pop() {
        if depth > NAME_TREE_DEPTH {
            continue;
        }
        if let Object::Ref(r) = raw
            && !seen.insert(r)
        {
            continue;
        }
        let node = doc.resolve(&raw);
        let Some(d) = node.as_dict() else { continue };
        let pairs = d.get(b"Names").map(|n| doc.resolve(n));
        if let Some(pairs) = pairs.as_deref().and_then(Object::as_array) {
            for pair in pairs.chunks(2) {
                if let [k, v] = pair
                    && let Some(key) = name_key(doc, k)
                {
                    out.entry(key).or_insert_with(|| v.clone());
                }
            }
        }
        let kids = d.get(b"Kids").map(|k| doc.resolve(k));
        if let Some(kids) = kids.as_deref().and_then(Object::as_array) {
            for kid in kids.iter().rev() {
                stack.push((kid.clone(), depth + 1));
            }
        }
    }
    out
}

/// The bytes of a name-tree key, which is a string or a name.
fn name_key(doc: &Document, key: &Object) -> Option<Vec<u8>> {
    match &*doc.resolve(key) {
        Object::String(s) => Some(s.bytes.clone()),
        Object::Name(n) => Some(n.clone()),
        _ => None,
    }
}

/// A named destination in the catalog's PDF 1.1 `/Dests` dictionary.
fn dests_dict(doc: &Document, key: &[u8]) -> Option<Object> {
    let root = doc.root()?;
    let dests = doc.get(root).as_dict().and_then(|c| c.get(b"Dests").cloned())?;
    let dests = doc.resolve(&dests);
    dests.as_dict()?.get(key).cloned()
}

/// The explicit destination array (`[page /XYZ …]`) a destination stands for: itself, a dictionary
/// whose `/D` is one, or a named destination that resolves to one.
fn explicit_dest(doc: &Document, targets: &mut Targets, dest: &Object, depth: u8) -> Option<Vec<Object>> {
    if depth > DEST_DEPTH {
        return None;
    }
    let dest = doc.resolve(dest);
    match &*dest {
        Object::Array(a) => Some(a.clone()),
        Object::Dict(d) => explicit_dest(doc, targets, d.get(b"D")?, depth + 1),
        Object::String(s) => {
            let inner = targets.named(doc, &s.bytes)?;
            explicit_dest(doc, targets, &inner, depth + 1)
        }
        Object::Name(n) => {
            let inner = targets.named(doc, n)?;
            explicit_dest(doc, targets, &inner, depth + 1)
        }
        _ => None,
    }
}

/// The explicit destination array `dest` stands for: itself, a dictionary whose `/D` is one, or a
/// named destination (a name or string, ISO 32000-2 §12.3.2.4) looked up in the `/Names /Dests`
/// tree, then the catalog `/Dests`. `None` when a name names nothing.
pub(crate) fn resolve_dest(doc: &Document, dest: &Object) -> Option<Vec<Object>> {
    // Names only: the page map is not needed to look one up.
    let mut targets = Targets { pages: HashMap::new(), names: None };
    explicit_dest(doc, &mut targets, dest, 0)
}

/// The page index an explicit destination array names.
fn page_of_explicit(targets: &Targets, dest: &[Object]) -> Option<usize> {
    match dest.first()? {
        Object::Ref(page) => targets.pages.get(page).copied(),
        // Some producers write a page number in a remote-style destination.
        Object::Int(n) => usize::try_from(*n).ok().filter(|&p| p < targets.pages.len()),
        _ => None,
    }
}

/// The view an explicit destination array asks for: its type and the numbers after it.
fn view_of_explicit(doc: &Document, dest: &[Object]) -> Option<(Vec<u8>, Vec<Option<f64>>)> {
    let kind = dest.get(1).map(|k| doc.resolve(k)).and_then(|k| k.as_name().map(<[u8]>::to_vec))?;
    // At most the four numbers of /FitR.
    let numbers = dest.iter().skip(2).take(4).map(|n| doc.resolve(n).as_f64().filter(|v| v.is_finite())).collect();
    Some((kind, numbers))
}

/// The explicit destination an item goes to: its `/Dest` when that names a page here, else the
/// destination of its GoTo action `/A`.
fn item_dest(doc: &Document, targets: &mut Targets, d: &Dict) -> Option<Vec<Object>> {
    if let Some(dest) = d.get(b"Dest").and_then(|dest| explicit_dest(doc, targets, dest, 0))
        && page_of_explicit(targets, &dest).is_some()
    {
        return Some(dest);
    }
    let action = doc.resolve(d.get(b"A")?);
    let dest = action.as_dict()?.get(b"D")?;
    explicit_dest(doc, targets, dest, 0)
}

/// A bookmark's title as viewers show it: the decoded text, without NULs or surrounding space.
fn title_of(doc: &Document, d: &Dict) -> String {
    let title = d.get(b"Title").map(|t| doc.resolve(t));
    let text = match title.as_deref() {
        Some(Object::String(s)) => s.to_text(),
        Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
        _ => return String::new(),
    };
    text.trim_matches('\0').trim().to_string()
}

fn describe(doc: &Document, targets: &mut Targets, obj: ObjRef, path: Vec<usize>) -> Bookmark {
    let item = doc.get(obj);
    let (title, open, dest) = match item.as_dict() {
        Some(d) => (title_of(doc, d), d.int(b"Count").is_some_and(|c| c > 0), item_dest(doc, targets, d)),
        None => (String::new(), false, None),
    };
    let page = dest.as_deref().and_then(|d| page_of_explicit(targets, d));
    let view = dest.as_deref().filter(|_| page.is_some()).and_then(|d| view_of_explicit(doc, d));
    let color = item.as_dict().and_then(|d| d.get(b"C").map(|c| doc.resolve(c))).and_then(|c| title_color(doc, &c));
    let flags = item.as_dict().and_then(|d| d.get(b"F").and_then(|f| doc.resolve(f).as_int())).unwrap_or(0);
    Bookmark { obj, path, title, open, page, view, color, italic: flags & 1 != 0, bold: flags & 2 != 0 }
}

/// A bookmark title's `/C` as a colour: three numbers, each kept within 0–1 (ISO 32000-2
/// §12.3.3, Table 151: "An array of three numbers in the range 0.0 to 1.0"); a number past the
/// range, however large, is kept at its edge. Black, the default, and anything else give `None`.
fn title_color(doc: &Document, c: &Object) -> Option<[f32; 3]> {
    let [r, g, b] = c.as_array()?.as_slice() else { return None };
    let channel = |v: &Object| doc.resolve(v).as_f64().map(|v| v.clamp(0.0, 1.0) as f32);
    let rgb = [channel(r)?, channel(g)?, channel(b)?];
    (rgb != [0.0; 3]).then_some(rgb)
}

/// Lists up to `limit` bookmarks from position `offset`, entering no level deeper than `max_depth`.
fn list(doc: &Document, max_depth: usize, offset: usize, limit: usize) -> BookmarkPage {
    let mut bookmarks = Vec::new();
    let Some(mut visit) = Visit::new(doc, max_depth) else { return BookmarkPage { bookmarks, next: None } };
    for _ in 0..offset {
        if visit.step().is_none() {
            return BookmarkPage { bookmarks, next: None };
        }
    }
    let mut targets = Targets::new(doc);
    while bookmarks.len() < limit {
        let Some(obj) = visit.step() else { break };
        bookmarks.push(describe(doc, &mut targets, obj, visit.path()));
    }
    let next = if bookmarks.len() == limit && visit.step().is_some() { Some(offset.saturating_add(bookmarks.len())) } else { None };
    BookmarkPage { bookmarks, next }
}

/// Every bookmark, in listing order (bookmarks deeper than the listing depth are left out).
pub fn bookmarks(doc: &Document) -> Vec<Bookmark> {
    list(doc, LIST_DEPTH, 0, usize::MAX).bookmarks
}

/// The top-level bookmarks, in order.
pub fn top_level_bookmarks(doc: &Document) -> Vec<Bookmark> {
    list(doc, 0, 0, usize::MAX).bookmarks
}

/// One page of the bookmarks in listing order: up to `limit` from position `offset`, and the offset
/// where the next page starts. Pages are contiguous, so following `next` lists every bookmark once.
/// Only the bookmarks up to the end of the page are visited, so the first page does not depend on
/// how many bookmarks come after it.
pub fn bookmark_page(doc: &Document, offset: usize, limit: usize) -> BookmarkPage {
    list(doc, LIST_DEPTH, offset, limit)
}

/// The object of the bookmark at `path`, and its parent object (the outline root for top-level).
fn resolve_path(doc: &Document, path: &[usize]) -> Result<(ObjRef, ObjRef)> {
    let root = outline_root(doc).ok_or_else(|| OutlineError::NoSuchBookmark(path.to_vec()))?;
    if path.is_empty() {
        return Err(OutlineError::NoSuchBookmark(Vec::new()));
    }
    let mut parent = root;
    let mut seen = HashSet::from([root]);
    for (depth, i) in path.iter().enumerate() {
        let kids = children_of(doc, parent, &mut seen);
        let r = *kids.get(*i).ok_or_else(|| OutlineError::NoSuchBookmark(path.to_vec()))?;
        if depth + 1 == path.len() {
            return Ok((r, parent));
        }
        parent = r;
    }
    // The loop returns on the last index.
    Err(OutlineError::NoSuchBookmark(path.to_vec()))
}

/// The object that holds children at `parent_path` (`[]` = the outline root, created if needed).
pub(crate) fn container(doc: &mut Document, parent_path: &[usize]) -> Result<ObjRef> {
    if parent_path.is_empty() {
        if let Some(r) = outline_root(doc) {
            return Ok(r);
        }
        let mut o = Dict::new();
        o.set(b"Type".to_vec(), Object::name("Outlines"));
        let r = doc.add(Object::Dict(o));
        let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
        doc.update_dict(root, |c| c.set(b"Outlines".to_vec(), Object::Ref(r)))?;
        return Ok(r);
    }
    Ok(resolve_path(doc, parent_path)?.0)
}

/// Set `key` on the dictionary `r` (or remove it with `None`) only if it changes.
fn put(doc: &mut Document, r: ObjRef, key: &[u8], value: Option<Object>) -> Result<()> {
    let current = doc.get(r).as_dict().and_then(|d| d.get(key).cloned());
    if current == value {
        return Ok(());
    }
    doc.update_dict(r, |d| match value {
        Some(v) => d.set(key.to_vec(), v),
        None => {
            d.remove(key);
        }
    })?;
    Ok(())
}

/// The children of `parent` (an item or the outline root), in order. Cycle-safe.
pub(crate) fn children(doc: &Document, parent: ObjRef) -> Vec<ObjRef> {
    children_of(doc, parent, &mut HashSet::from([parent]))
}

/// [`relink`] `kids` under `parent`, then recompute every `/Count`.
pub(crate) fn relink_and_recount(doc: &mut Document, parent: ObjRef, kids: &[ObjRef]) -> Result<()> {
    relink(doc, parent, kids)?;
    recount(doc)
}

impl From<OutlineError> for OrganizeError {
    fn from(e: OutlineError) -> Self {
        match e {
            OutlineError::Organize(e) => e,
            OutlineError::Cos(e) => OrganizeError::Cos(e),
            other => OrganizeError::Invalid(other.to_string()),
        }
    }
}

/// Make `kids` the children of `parent`, in order: rewires `/Parent`, `/Prev`, `/Next`,
/// `/First` and `/Last`.
pub(crate) fn relink(doc: &mut Document, parent: ObjRef, kids: &[ObjRef]) -> Result<()> {
    for (i, k) in kids.iter().enumerate() {
        put(doc, *k, b"Parent", Some(Object::Ref(parent)))?;
        put(doc, *k, b"Prev", i.checked_sub(1).map(|p| Object::Ref(kids[p])))?;
        put(doc, *k, b"Next", kids.get(i + 1).map(|n| Object::Ref(*n)))?;
    }
    put(doc, parent, b"First", kids.first().map(|k| Object::Ref(*k)))?;
    put(doc, parent, b"Last", kids.last().map(|k| Object::Ref(*k)))?;
    Ok(())
}

/// A node whose `/Count` [`recount`] is working out.
struct Counting {
    node: ObjRef,
    kids: Vec<ObjRef>,
    /// The next child to count.
    next: usize,
    /// Items visible below the node, so far.
    visible: i64,
    is_root: bool,
}

/// Recompute every `/Count` (§12.3.3: open items count their visible descendants, closed items
/// the negative of what opening them would show; the root counts all visible items).
pub(crate) fn recount(doc: &mut Document) -> Result<()> {
    let Some(root) = outline_root(doc) else { return Ok(()) };
    let mut seen = HashSet::from([root]);
    let kids = children_of(doc, root, &mut seen);
    let mut stack = vec![Counting { node: root, kids, next: 0, visible: 0, is_root: true }];
    // A node is counted once its children are, so the walk keeps its own stack instead of recursing.
    while let Some(top) = stack.last_mut() {
        if let Some(&kid) = top.kids.get(top.next) {
            top.next += 1;
            let kids = children_of(doc, kid, &mut seen);
            stack.push(Counting { node: kid, kids, next: 0, visible: 0, is_root: false });
            continue;
        }
        let Some(done) = stack.pop() else { break };
        let count = if done.kids.is_empty() {
            None
        } else if done.is_root || doc.get(done.node).as_dict().and_then(|d| d.int(b"Count")).is_some_and(|c| c > 0) {
            Some(done.visible)
        } else {
            Some(-done.visible)
        };
        put(doc, done.node, b"Count", count.map(Object::Int))?;
        if let Some(parent) = stack.last_mut() {
            let open = doc.get(done.node).as_dict().and_then(|d| d.int(b"Count")).is_some_and(|c| c > 0);
            parent.visible = parent.visible.saturating_add(1 + if open { done.visible } else { 0 });
        }
    }
    Ok(())
}

fn destination(doc: &Document, page: usize) -> Result<Object> {
    destination_at(doc, page, None, None)
}

/// A destination on `page` (0-based) at the user-space point (`left`, `top`), either of which
/// may be left out to keep the reader's position in that direction.
fn destination_at(doc: &Document, page: usize, left: Option<f64>, top: Option<f64>) -> Result<Object> {
    if ![left, top].iter().flatten().all(|v| v.is_finite()) {
        return Err(OutlineError::BadPosition);
    }
    let pages = walk(doc)?;
    let (p, _) = pages.get(page).ok_or(OrganizeError::NoSuchPage(page))?;
    Ok(go_to_at(*p, left, top))
}

/// `/XYZ left top null`: the point (`left`, `top`) at the window's upper-left corner, at the
/// reader's zoom; a coordinate that is left out keeps the reader's position. With neither,
/// `/XYZ null null null` goes to the page, keeping the reader's zoom (what Acrobat writes for a
/// new bookmark when the view has no specific position).
fn go_to_at(page: ObjRef, left: Option<f64>, top: Option<f64>) -> Object {
    let coordinate = |v: Option<f64>| v.map_or(Object::Null, Object::Real);
    Object::Array(vec![Object::Ref(page), Object::name("XYZ"), coordinate(left), coordinate(top), Object::Null])
}

/// Add a bookmark titled `title` that goes to `page` (0-based), as child `index` of
/// `parent_path` (`[]` = top level; an index past the end appends). Returns its path.
pub fn add_bookmark(doc: &mut Document, parent_path: &[usize], index: usize, title: &str, page: usize) -> Result<Vec<usize>> {
    add_bookmark_at(doc, parent_path, index, title, page, None, None)
}

/// [`add_bookmark`] going to the user-space point (`left`, `top`) of `page`, as `/XYZ left top null`
/// (zoom kept). Either coordinate may be left out to keep the reader's position in that
/// direction; one that is not finite is refused.
pub fn add_bookmark_at(
    doc: &mut Document,
    parent_path: &[usize],
    index: usize,
    title: &str,
    page: usize,
    left: Option<f64>,
    top: Option<f64>,
) -> Result<Vec<usize>> {
    if title.trim().is_empty() {
        return Err(OutlineError::EmptyTitle);
    }
    let dest = destination_at(doc, page, left, top)?;
    let parent = container(doc, parent_path)?;
    let mut kids = children_of(doc, parent, &mut HashSet::from([parent]));
    let mut d = Dict::new();
    d.set(b"Title".to_vec(), Object::String(PdfString::text(title)));
    d.set(b"Dest".to_vec(), dest);
    let item = doc.add(Object::Dict(d));
    let at = index.min(kids.len());
    kids.insert(at, item);
    relink(doc, parent, &kids)?;
    show_children(doc, parent, parent_path)?;
    recount(doc)?;
    let mut path = parent_path.to_vec();
    path.push(at);
    Ok(path)
}

/// One bookmark of a generated tree (New Bookmarks from Structure, or from text styles).
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineEntry {
    /// 1 = directly under the new parent; deeper levels nest under the last shallower entry.
    pub level: u8,
    pub title: String,
    pub page: usize,
    /// The structure element it stands for (`/SE`).
    pub element: Option<ObjRef>,
    /// Where on the page it goes: this height (user space) at the top of the window, at the
    /// reader's zoom. `None` goes to the page, keeping the reader's position.
    pub top: Option<f64>,
}

/// Add a new top-level bookmark `parent_title` (first in the list) holding `entries` nested by
/// level, as Acrobat does for bookmarks made from structure; everything starts expanded.
/// Returns the parent's path.
pub fn add_bookmark_tree(doc: &mut Document, parent_title: &str, entries: &[OutlineEntry]) -> Result<Vec<usize>> {
    if entries.is_empty() {
        return Err(OutlineError::NoEntries);
    }
    let pages = walk(doc)?;
    let targets = entries
        .iter()
        .map(|e| pages.get(e.page).map(|p| p.0).ok_or(OrganizeError::NoSuchPage(e.page)))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut d = Dict::new();
    d.set(b"Title".to_vec(), Object::String(PdfString::text(parent_title)));
    let parent = doc.add(Object::Dict(d));
    // (level, item, its children so far); the bottom is the new parent.
    let mut stack: Vec<(u8, ObjRef, Vec<ObjRef>)> = vec![(0, parent, Vec::new())];
    let mut done: Vec<(ObjRef, Vec<ObjRef>)> = Vec::new();
    for (e, page) in entries.iter().zip(targets) {
        let mut d = Dict::new();
        d.set(b"Title".to_vec(), Object::String(PdfString::text(&e.title)));
        d.set(b"Dest".to_vec(), go_to_at(page, None, e.top));
        if let Some(se) = e.element {
            d.set(b"SE".to_vec(), Object::Ref(se));
        }
        let item = doc.add(Object::Dict(d));
        while stack.len() > 1 && stack.last().is_some_and(|s| s.0 >= e.level) {
            done.extend(stack.pop().map(|(_, r, kids)| (r, kids)));
        }
        if let Some(top) = stack.last_mut() {
            top.2.push(item);
        }
        stack.push((e.level, item, Vec::new()));
    }
    done.extend(stack.into_iter().map(|(_, r, kids)| (r, kids)));
    for (r, kids) in &done {
        relink(doc, *r, kids)?;
        if !kids.is_empty() && *r != parent {
            put(doc, *r, b"Count", Some(Object::Int(1)))?;
        }
    }
    let root = container(doc, &[])?;
    let mut top = children_of(doc, root, &mut HashSet::from([root]));
    top.insert(0, parent);
    relink(doc, root, &top)?;
    put(doc, parent, b"Count", Some(Object::Int(1)))?;
    recount(doc)?;
    Ok(vec![0])
}

/// Change a bookmark's title.
pub fn rename_bookmark(doc: &mut Document, path: &[usize], title: &str) -> Result<()> {
    if title.trim().is_empty() {
        return Err(OutlineError::EmptyTitle);
    }
    let (r, _) = resolve_path(doc, path)?;
    put(doc, r, b"Title", Some(Object::String(PdfString::text(title))))
}

/// Set the colour of a bookmark's title (`/C`, ISO 32000-2 §12.3.3, Table 151: "An array of three
/// numbers in the range 0.0 to 1.0, representing the components in the DeviceRGB colour space").
/// Black, the default, and `None` remove `/C`; a component outside 0–1 is refused.
pub fn set_bookmark_color(doc: &mut Document, path: &[usize], color: Option<[f64; 3]>) -> Result<()> {
    if color.is_some_and(|c| !c.iter().all(|v| (0.0..=1.0).contains(v))) {
        return Err(OutlineError::BadColor);
    }
    let (r, _) = resolve_path(doc, path)?;
    let value = color.filter(|c| *c != [0.0; 3]).map(|c| Object::Array(c.iter().map(|v| Object::Real(*v)).collect()));
    put(doc, r, b"C", value)
}

/// Make a bookmark's title bold or italic or not (`/F`, ISO 32000-2 §12.3.3, Table 152: bit 1
/// italic, bit 2 bold); `None` keeps that one as it is, and so are the other bits. When no bit is
/// left, `/F` is removed, its default being 0.
pub fn set_bookmark_style(doc: &mut Document, path: &[usize], bold: Option<bool>, italic: Option<bool>) -> Result<()> {
    let (r, _) = resolve_path(doc, path)?;
    let old = doc.get(r).as_dict().and_then(|d| d.get(b"F").and_then(|f| doc.resolve(f).as_int())).unwrap_or(0);
    let bit = |flags: i64, mask: i64, on: Option<bool>| match on {
        Some(true) => flags | mask,
        Some(false) => flags & !mask,
        None => flags,
    };
    let flags = bit(bit(old, 1, italic), 2, bold);
    put(doc, r, b"F", (flags != 0).then_some(Object::Int(flags)))
}

/// Point a bookmark at `page` (0-based), replacing its destination or GoTo action.
pub fn set_bookmark_page(doc: &mut Document, path: &[usize], page: usize) -> Result<()> {
    let dest = destination(doc, page)?;
    let (r, _) = resolve_path(doc, path)?;
    put(doc, r, b"Dest", Some(dest))?;
    put(doc, r, b"A", None) // `/Dest` and `/A` must not both be present (§12.3.3)
}

/// Remove a bookmark and everything under it. The removed items are unlinked (a full save drops
/// them; an incremental save leaves the old objects unreferenced).
pub fn delete_bookmark(doc: &mut Document, path: &[usize]) -> Result<()> {
    let (r, parent) = resolve_path(doc, path)?;
    let mut kids = children_of(doc, parent, &mut HashSet::from([parent]));
    kids.retain(|k| *k != r);
    relink(doc, parent, &kids)?;
    recount(doc)
}

/// Move the bookmark at `from` to child `index` of `to_parent` (counted after removing it from
/// its old place; past the end appends). Returns its new path.
pub fn move_bookmark(doc: &mut Document, from: &[usize], to_parent: &[usize], index: usize) -> Result<Vec<usize>> {
    if to_parent.starts_with(from) {
        return Err(OutlineError::IntoItself);
    }
    let (r, old_parent) = resolve_path(doc, from)?;
    let mut old = children_of(doc, old_parent, &mut HashSet::from([old_parent]));
    old.retain(|k| *k != r);
    relink(doc, old_parent, &old)?;
    // Removing `from` shifts later siblings: adjust a target path that runs through them.
    let mut to_parent = to_parent.to_vec();
    let depth = from.len() - 1;
    if to_parent.len() > depth && to_parent[..depth] == from[..depth] && to_parent[depth] > from[depth] {
        to_parent[depth] -= 1;
    }
    let parent = container(doc, &to_parent)?;
    let mut kids = children_of(doc, parent, &mut HashSet::from([parent]));
    let at = index.min(kids.len());
    kids.insert(at, r);
    relink(doc, parent, &kids)?;
    if parent != old_parent {
        show_children(doc, parent, &to_parent)?;
    }
    recount(doc)?;
    to_parent.push(at);
    Ok(to_parent)
}

/// Open `parent` so a bookmark just added or moved into it shows (the root is always open).
/// Only the sign of `/Count` matters here: [`recount`] sets its value.
pub(crate) fn show_children(doc: &mut Document, parent: ObjRef, parent_path: &[usize]) -> Result<()> {
    if !parent_path.is_empty() && doc.get(parent).as_dict().and_then(|d| d.int(b"Count")).is_none_or(|c| c <= 0) {
        put(doc, parent, b"Count", Some(Object::Int(1)))?;
    }
    Ok(())
}

/// Expand or collapse a bookmark that has children.
pub fn set_bookmark_open(doc: &mut Document, path: &[usize], open: bool) -> Result<()> {
    let (r, _) = resolve_path(doc, path)?;
    let count = doc.get(r).as_dict().and_then(|d| d.int(b"Count")).unwrap_or(0);
    if count != 0 && (count > 0) != open {
        put(doc, r, b"Count", Some(Object::Int(-count)))?;
        recount(doc)?;
    }
    Ok(())
}
