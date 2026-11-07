//! Moving a whole outline (or one bookmark's subtree) out of a document and into another
//! (ISO 32000-2 §12.3.3 outlines, §12.3.2.2 explicit destinations).
//!
//! [`outline_tree`] reads bookmarks as a portable tree of [`OutlineNode`]s: titles, nesting, open
//! state, colour, style and destinations as page index plus view in the file's own user space,
//! so writing a tree back with [`import_outline`] gives the same structure. A node whose page the
//! target lacks keeps its place in the tree without a destination, and [`unmapped`] names it, so
//! nothing is dropped silently. Not carried: actions other than a GoTo, named destinations (they
//! travel resolved to their page), and structure links (`/SE`).

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::outline::{LIST_DEPTH, children, container, recount, relink, show_children};
use crate::{OrganizeError, OutlineError, walk};

/// Most bookmarks one import writes.
pub const MAX_IMPORT_BOOKMARKS: usize = 100_000;

/// The destination types (Table 149) and how many numbers follow each.
const VIEWS: [(&[u8], usize); 8] = [(b"XYZ", 3), (b"Fit", 0), (b"FitH", 1), (b"FitV", 1), (b"FitR", 4), (b"FitB", 0), (b"FitBH", 1), (b"FitBV", 1)];

/// One bookmark and the bookmarks under it, independent of any document.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineNode {
    pub title: String,
    /// The 0-based page it goes to; `None` for a bookmark that goes nowhere in its document.
    pub page: Option<usize>,
    /// Where on the page: a destination type from Table 149 and exactly as many values as that
    /// type takes, in PDF user space (`None` keeps the reader's value). `None` goes to the page at
    /// the reader's position and zoom.
    pub view: Option<(Vec<u8>, Vec<Option<f64>>)>,
    /// The title's colour (`/C`), each component from 0 to 1; `None` is the default black.
    pub color: Option<[f32; 3]>,
    pub italic: bool,
    pub bold: bool,
    /// Shown expanded, when it has children.
    pub open: bool,
    pub children: Vec<OutlineNode>,
}

/// The tree [`outline_tree`] read.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineTree {
    pub nodes: Vec<OutlineNode>,
    /// Bookmarks nested deeper than the listing reaches (32 levels) were left out.
    pub truncated: bool,
}

/// The bookmark at `path` with everything under it, or the whole outline for `[]`.
pub fn outline_tree(doc: &Document, path: &[usize]) -> Result<OutlineTree, OutlineError> {
    // A whole outline's top level sits at depth 0; a subtree's root is the bookmark at `path`.
    let base = path.len().max(1);
    let mut truncated = false;
    // The open nodes, outermost first, each with its depth.
    let mut open: Vec<(usize, OutlineNode)> = Vec::new();
    let mut roots = Vec::new();
    for b in crate::bookmarks(doc).into_iter().filter(|b| b.path.starts_with(path) && !b.path.is_empty()) {
        let depth = b.path.len().saturating_sub(base);
        if b.path.len() > LIST_DEPTH && has_children(doc, b.obj) {
            truncated = true;
        }
        while open.last().is_some_and(|(d, _)| *d >= depth) {
            close(&mut open, &mut roots);
        }
        let view = b.view.and_then(|(kind, values)| portable_view(kind, values));
        let node =
            OutlineNode { title: b.title, page: b.page, view, color: b.color, italic: b.italic, bold: b.bold, open: b.open, children: Vec::new() };
        open.push((depth, node));
    }
    while !open.is_empty() {
        close(&mut open, &mut roots);
    }
    if !path.is_empty() && roots.is_empty() {
        return Err(OutlineError::NoSuchBookmark(path.to_vec()));
    }
    Ok(OutlineTree { nodes: roots, truncated })
}

fn close(open: &mut Vec<(usize, OutlineNode)>, roots: &mut Vec<OutlineNode>) {
    if let Some((_, node)) = open.pop() {
        match open.last_mut() {
            Some((_, parent)) => parent.children.push(node),
            None => roots.push(node),
        }
    }
}

fn has_children(doc: &Document, r: ObjRef) -> bool {
    doc.get(r).as_dict().is_some_and(|d| d.reference(b"First").is_some())
}

/// A destination's view as [`import_outline`] takes it: a known type with exactly its number of
/// values (extra ones dropped, missing ones `None`). An unknown type, or a FitR missing one of
/// its four values (the rectangle can't be kept), gives `None`.
fn portable_view(kind: Vec<u8>, mut values: Vec<Option<f64>>) -> Option<(Vec<u8>, Vec<Option<f64>>)> {
    let (_, arity) = VIEWS.iter().find(|(k, _)| *k == kind.as_slice())?;
    values.resize(*arity, None);
    if kind.as_slice() == b"FitR" && values.iter().any(Option::is_none) {
        return None;
    }
    Some((kind, values))
}

/// The nodes of `nodes` whose page `page_count` pages don't have, as paths into `nodes`
/// (0-based child indices), in tree order. [`import_outline`] keeps these without a destination.
pub fn unmapped(nodes: &[OutlineNode], page_count: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    visit(nodes, |path, node| {
        if node.page.is_some_and(|p| p >= page_count) {
            out.push(path.to_vec());
        }
    });
    out
}

/// How many values a view of destination type `kind` takes (Table 149), or `None` for a type
/// that is not one of them.
pub fn view_arity(kind: &[u8]) -> Option<usize> {
    VIEWS.iter().find(|(k, _)| *k == kind).map(|(_, arity)| *arity)
}

/// Call `f` with each node's path and the node, parents first. Iterative.
fn visit<'a>(nodes: &'a [OutlineNode], mut f: impl FnMut(&[usize], &'a OutlineNode)) {
    // One entry per open level: its nodes and how many of them have been visited.
    let mut stack: Vec<(&'a [OutlineNode], usize)> = vec![(nodes, 0)];
    // The visited node's path, kept in step with the stack rather than rebuilt per node.
    let mut path: Vec<usize> = Vec::new();
    while let Some(level) = stack.last_mut() {
        let list = level.0;
        let index = level.1;
        let Some(node) = list.get(index) else {
            stack.pop();
            continue;
        };
        level.1 = index.saturating_add(1);
        path.truncate(stack.len().saturating_sub(1));
        path.push(index);
        f(&path, node);
        if !node.children.is_empty() {
            stack.push((&node.children, 0));
        }
    }
}

/// Check every node before anything is written: depth (counting the `base` levels above the
/// tree), count, colour and view.
fn check(nodes: &[OutlineNode], base: usize) -> Result<(), OutlineError> {
    let mut problem = None;
    let mut count = 0usize;
    visit(nodes, |path, node| {
        count = count.saturating_add(1);
        if problem.is_some() {
            return;
        }
        if base.saturating_add(path.len()) > LIST_DEPTH.saturating_add(1) {
            problem = Some(OutlineError::TooDeep(LIST_DEPTH.saturating_add(1)));
        } else if node.color.is_some_and(|c| !c.iter().all(|v| (0.0..=1.0).contains(v))) {
            problem = Some(OutlineError::BadColor);
        } else if node.view.as_ref().is_some_and(|v| !view_is_valid(v)) {
            problem = Some(OutlineError::BadView(path.to_vec()));
        }
    });
    if count > MAX_IMPORT_BOOKMARKS {
        return Err(OutlineError::TooManyBookmarks(MAX_IMPORT_BOOKMARKS));
    }
    problem.map_or(Ok(()), Err)
}

/// A known type with its number of values, each finite; `/FitR` needs all four.
fn view_is_valid((kind, values): &(Vec<u8>, Vec<Option<f64>>)) -> bool {
    VIEWS.iter().any(|(k, arity)| *k == kind.as_slice() && *arity == values.len())
        && values.iter().flatten().all(|v| v.is_finite())
        && (kind.as_slice() != b"FitR" || values.iter().all(Option::is_some))
}

/// Write `nodes` as children of the bookmark at `parent_path` (`[]` = top level), starting at
/// child `index` (past the end appends), or, with `replace`, in place of the outline's
/// top-level bookmarks (then `parent_path` must be `[]`). Nodes whose page this document lacks
/// are written without a destination (see [`unmapped`]). Everything is checked before anything
/// is written. Returns the path of the first node written.
pub fn import_outline(
    doc: &mut Document,
    parent_path: &[usize],
    index: usize,
    nodes: &[OutlineNode],
    replace: bool,
) -> Result<Vec<usize>, OutlineError> {
    if nodes.is_empty() {
        return Err(OutlineError::NothingToImport);
    }
    if replace && !parent_path.is_empty() {
        return Err(OrganizeError::Invalid("replace applies to the whole outline: leave the parent out".into()).into());
    }
    check(nodes, parent_path.len())?;
    let pages: Vec<ObjRef> = walk(doc)?.into_iter().map(|(r, _)| r).collect();
    let parent = container(doc, parent_path)?;
    let top = write_nodes(doc, &pages, nodes)?;
    let mut kids = if replace { Vec::new() } else { children(doc, parent) };
    let at = index.min(kids.len());
    kids.splice(at..at, top);
    relink(doc, parent, &kids)?;
    show_children(doc, parent, parent_path)?;
    recount(doc)?;
    let mut path = parent_path.to_vec();
    path.push(at);
    Ok(path)
}

/// Add one item per node, linked under each other, and return the top-level items in order.
fn write_nodes(doc: &mut Document, pages: &[ObjRef], nodes: &[OutlineNode]) -> Result<Vec<ObjRef>, OutlineError> {
    // Each written item with its node, then its children's items once they are written.
    let mut written: Vec<(ObjRef, Vec<ObjRef>)> = Vec::new();
    let mut top = Vec::new();
    // (the nodes of one level, the next to write, the index in `written` of their parent)
    let mut stack: Vec<(&[OutlineNode], usize, Option<usize>)> = vec![(nodes, 0, None)];
    while let Some(level) = stack.last_mut() {
        let (list, parent) = (level.0, level.2);
        let Some(node) = list.get(level.1) else {
            stack.pop();
            continue;
        };
        level.1 = level.1.saturating_add(1);
        let item = doc.add(Object::Dict(item_dict(node, pages)));
        match parent.and_then(|p| written.get_mut(p)) {
            Some((_, kids)) => kids.push(item),
            None => top.push(item),
        }
        written.push((item, Vec::new()));
        if !node.children.is_empty() {
            stack.push((&node.children, 0, Some(written.len().saturating_sub(1))));
        }
    }
    for (item, kids) in &written {
        relink(doc, *item, kids)?;
    }
    Ok(top)
}

/// The item's own entries; the links and `/Count`'s value come from [`relink`] and [`recount`]
/// (only the sign of the `/Count` written here matters: it keeps the open state).
fn item_dict(node: &OutlineNode, pages: &[ObjRef]) -> Dict {
    let mut d = Dict::new();
    d.set(b"Title".to_vec(), Object::String(PdfString::text(&node.title)));
    if let Some(page) = node.page.and_then(|p| pages.get(p)) {
        let (kind, values) = node.view.clone().unwrap_or_else(|| (b"XYZ".to_vec(), vec![None; 3]));
        let mut dest = vec![Object::Ref(*page), Object::Name(kind)];
        dest.extend(values.into_iter().map(|v| v.map_or(Object::Null, Object::Real)));
        d.set(b"Dest".to_vec(), Object::Array(dest));
    }
    if let Some(c) = node.color.filter(|c| *c != [0.0; 3]) {
        d.set(b"C".to_vec(), Object::Array(c.iter().map(|v| Object::Real(f64::from(*v))).collect()));
    }
    let flags = i64::from(node.italic) | i64::from(node.bold) << 1;
    if flags != 0 {
        d.set(b"F".to_vec(), Object::Int(flags));
    }
    if !node.children.is_empty() {
        d.set(b"Count".to_vec(), Object::Int(if node.open { 1 } else { -1 }));
    }
    d
}
