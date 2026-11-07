use super::copy::Copier;
use super::{Dict, Document, HashMap, HashSet, ObjRef, Object, OrganizeError, name_tree_entries};

/// Copy a source's document-level attachments (`/Names /EmbeddedFiles`) into `dst`.
pub(super) fn collect_attachments(dst: &mut Document, src: &Document, out: &mut Vec<(Vec<u8>, Object)>) -> Result<(), OrganizeError> {
    let Some(tree) = src
        .root()
        .and_then(|r| src.get(r).as_dict().cloned())
        .and_then(|c| c.get(b"Names").map(|n| src.resolve(n)))
        .and_then(|n| n.as_dict().and_then(|d| d.get(b"EmbeddedFiles").cloned()))
    else {
        return Ok(());
    };
    let mut entries = Vec::new();
    name_tree_entries(src, &tree, 0, &mut HashSet::new(), &mut entries);
    let mut copier = Copier::new(src, HashMap::new(), HashMap::new());
    for (k, v) in entries {
        let v = copier.copy_value(dst, &v, None)?;
        out.push((k, v));
    }
    Ok(())
}

/// Write attachments as a flat, sorted `/EmbeddedFiles` name tree; duplicate names get " (2)"…
pub(crate) fn set_attachments(dst: &mut Document, mut entries: Vec<(Vec<u8>, Object)>) -> Result<(), OrganizeError> {
    if entries.is_empty() {
        return Ok(());
    }
    // Names are compared and numbered as text: a key written as UTF-16 (a name with an accent
    // or Chinese letters) must not get ASCII bytes appended to it.
    let mut seen = std::collections::HashSet::new();
    for (k, _) in entries.iter_mut() {
        let base = pdfcraft_cos::PdfString::literal(k.clone()).to_text();
        let mut text = base.clone();
        let mut n = 2;
        while !seen.insert(text.clone()) {
            text = format!("{base} ({n})");
            n += 1;
        }
        if text != base {
            *k = pdfcraft_cos::PdfString::text(&text).bytes;
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut names = Vec::new();
    for (k, v) in entries {
        names.push(Object::String(pdfcraft_cos::PdfString::literal(k)));
        names.push(v);
    }
    let mut tree = Dict::new();
    tree.set(b"Names".to_vec(), Object::Array(names));
    let tree = dst.add(tree);
    let root = dst.root().ok_or(OrganizeError::NoPageTree)?;
    // `/Names` may be an indirect dictionary (Acrobat writes it so): update it where it is, so
    // its other trees (/Dests, /JavaScript…) stay.
    if let Some(Object::Ref(names)) = dst.get(root).as_dict().and_then(|c| c.get(b"Names").cloned())
        && dst.get(names).as_dict().is_some()
    {
        dst.update_dict(names, |n| n.set(b"EmbeddedFiles".to_vec(), Object::Ref(tree)))?;
        return Ok(());
    }
    dst.update_dict(root, |c| {
        let mut n = match c.get(b"Names") {
            Some(Object::Dict(d)) => d.clone(),
            _ => Dict::new(),
        };
        n.set(b"EmbeddedFiles".to_vec(), Object::Ref(tree));
        c.set(b"Names".to_vec(), Object::Dict(n));
    })?;
    Ok(())
}

/// A source's entry in the combined outline (see [`add_outline`]): its title, its first page in the
/// result, and its own bookmarks, read from the source when it was added.
pub(super) struct Mark {
    pub(super) title: String,
    pub(super) page: ObjRef,
    pub(super) outline: Vec<OutlineNode>,
}

/// One of a source's bookmarks, read and ready to write: its title, colour and flags, its destination
/// in the result (`None` when the page it points at was not copied), whether it is open, and its own
/// bookmarks.
pub(super) struct OutlineNode {
    title: Option<Object>,
    colour: Option<Object>,
    flags: Option<Object>,
    dest: Option<Object>,
    open: bool,
    children: Vec<OutlineNode>,
}

/// The bookmarks of `src`, with destinations mapped through `pages` (source page to copied page). They
/// are read while `src` is at hand, so the source itself need not be kept.
pub(super) fn source_outline(src: &Document, pages: HashMap<ObjRef, ObjRef>) -> Vec<OutlineNode> {
    let first = src
        .root()
        .and_then(|r| src.get(r).as_dict().cloned())
        .and_then(|c| c.get(b"Outlines").map(|o| src.resolve(o)))
        .and_then(|o| o.as_dict().and_then(|d| d.reference(b"First")));
    let mapper = Copier::new(src, HashMap::new(), pages);
    let mut budget = 10_000usize;
    outline_level(&mapper, first, 0, &mut budget)
}

/// `nodes` without the bookmarks whose page was not copied (no destination left), unless a bookmark
/// under them stays. [`outline_level`] already bounds the depth.
pub(super) fn kept_bookmarks(nodes: Vec<OutlineNode>) -> Vec<OutlineNode> {
    nodes
        .into_iter()
        .filter_map(|mut n| {
            n.children = kept_bookmarks(n.children);
            (n.dest.is_some() || !n.children.is_empty()).then_some(n)
        })
        .collect()
}

/// One level of bookmarks from `first` (its siblings), each with its children. Cycle- and size-safe:
/// at most `budget` bookmarks are read in all, and nesting stops at 32 levels.
fn outline_level(m: &Copier<'_>, first: Option<ObjRef>, depth: u8, budget: &mut usize) -> Vec<OutlineNode> {
    if depth > 32 {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut made = Vec::new();
    let mut cur = first;
    while let Some(r) = cur {
        if !seen.insert(r) || *budget == 0 {
            break;
        }
        *budget -= 1;
        let Some(item) = m.src.get(r).as_dict().cloned() else { break };
        cur = item.reference(b"Next");
        let title = item.get(b"Title").map(|t| m.src.resolve(t).as_ref().clone());
        let colour = item.get(b"C").map(|v| m.src.resolve(v).as_ref().clone());
        let flags = item.get(b"F").map(|v| m.src.resolve(v).as_ref().clone());
        let target = item.get(b"Dest").cloned().or_else(|| {
            let a = m.src.resolve(item.get(b"A")?);
            let a = a.as_dict()?;
            (a.name(b"S") == Some(b"GoTo")).then(|| a.get(b"D").cloned()).flatten()
        });
        let dest = target.and_then(|t| m.map_dest(&t));
        let open = item.int(b"Count").is_some_and(|c| c > 0);
        let children = outline_level(m, item.reference(b"First"), depth + 1, budget);
        made.push(OutlineNode { title, colour, flags, dest, open, children });
    }
    made
}

/// Write `nodes` as one level under `parent`: each bookmark is created, then its children, depth first
/// and in order, so the object numbers come out as they always did. Returns the level's first and last
/// item and how many items it has (`None` when it is empty).
pub(super) fn write_outline_level(
    dst: &mut Document,
    parent: ObjRef,
    nodes: &[OutlineNode],
) -> Result<Option<(ObjRef, ObjRef, usize)>, OrganizeError> {
    let mut made: Vec<ObjRef> = Vec::new();
    for node in nodes {
        let new = dst.add(Object::Null);
        let mut d = Dict::new();
        if let Some(t) = &node.title {
            d.set(b"Title".to_vec(), t.clone());
        }
        if let Some(c) = &node.colour {
            d.set(b"C".to_vec(), c.clone());
        }
        if let Some(f) = &node.flags {
            d.set(b"F".to_vec(), f.clone());
        }
        if let Some(dest) = &node.dest {
            d.set(b"Dest".to_vec(), dest.clone());
        }
        d.set(b"Parent".to_vec(), Object::Ref(parent));
        if let Some(prev) = made.last() {
            d.set(b"Prev".to_vec(), Object::Ref(*prev));
            dst.update_dict(*prev, |p| p.set(b"Next".to_vec(), Object::Ref(new)))?;
        }
        if let Some((f, l, n)) = write_outline_level(dst, new, &node.children)? {
            d.set(b"First".to_vec(), Object::Ref(f));
            d.set(b"Last".to_vec(), Object::Ref(l));
            // Keep the source's open/closed state.
            d.set(b"Count".to_vec(), Object::Int(if node.open { n as i64 } else { -(n as i64) }));
        }
        dst.set(new, Object::Dict(d));
        made.push(new);
    }
    Ok(match (made.first(), made.last()) {
        (Some(first), Some(last)) => Some((*first, *last, made.len())),
        _ => None,
    })
}

/// Replace the outline with one bookmark per source (title → its first page), each holding a collapsed
/// copy of that source's own bookmarks with destinations mapped to the copied pages.
pub(super) fn add_outline(doc: &mut Document, marks: &[Mark]) -> Result<(), OrganizeError> {
    if marks.is_empty() {
        return Ok(());
    }
    let outlines = doc.add(Object::Null);
    let items: Vec<ObjRef> = marks.iter().map(|_| doc.add(Object::Null)).collect();
    let (Some(&first_item), Some(&last_item)) = (items.first(), items.last()) else { return Ok(()) };
    for (i, (mark, r)) in marks.iter().zip(&items).enumerate() {
        let mut d = Dict::new();
        d.set(b"Title".to_vec(), Object::String(pdfcraft_cos::PdfString::text(&mark.title)));
        d.set(b"Parent".to_vec(), Object::Ref(outlines));
        d.set(b"Dest".to_vec(), Object::Array(vec![Object::Ref(mark.page), Object::name("Fit")]));
        if i > 0 {
            d.set(b"Prev".to_vec(), Object::Ref(items[i - 1]));
        }
        if let Some(next) = items.get(i + 1) {
            d.set(b"Next".to_vec(), Object::Ref(*next));
        }
        if let Some((f, l, n)) = write_outline_level(doc, *r, &mark.outline)? {
            d.set(b"First".to_vec(), Object::Ref(f));
            d.set(b"Last".to_vec(), Object::Ref(l));
            d.set(b"Count".to_vec(), Object::Int(-(n as i64))); // collapsed
        }
        doc.set(*r, Object::Dict(d));
    }
    let mut o = Dict::new();
    o.set(b"Type".to_vec(), Object::name("Outlines"));
    o.set(b"First".to_vec(), Object::Ref(first_item));
    o.set(b"Last".to_vec(), Object::Ref(last_item));
    o.set(b"Count".to_vec(), Object::Int(items.len() as i64));
    doc.set(outlines, Object::Dict(o));
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    doc.update_dict(root, |c| {
        c.set(b"Outlines".to_vec(), Object::Ref(outlines));
        c.set(b"PageMode".to_vec(), Object::name("UseOutlines"));
    })?;
    Ok(())
}
