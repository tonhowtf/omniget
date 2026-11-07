//! Copying pages between documents: the basis of Combine Files, Insert Pages from File,
//! Extract Pages and Split.
//!
//! A page is deep-copied with everything it references (content streams, resources, fonts,
//! images, annotations), renumbered into the destination. Objects shared between the copied
//! pages (a font used on every page) are copied once. The copy never follows `/Parent` from a
//! page, so the source page tree does not come along. It rewires what points *back* into the
//! document:
//! - an annotation's `/P` points at its new page;
//! - link and GoTo destinations that target a copied page point at the copy. Links to pages that
//!   were not copied are dropped, so they don't dangle;
//! - form-field widgets keep their field hierarchy, and the top-level fields are registered in the
//!   destination `/AcroForm`;
//! - structure-tree links (`/StructParent(s)`) and article beads (`/B`) are removed, because the
//!   source structure tree is not copied;
//! - named destinations are resolved against the source's name tree and become explicit ones;
//! - layers (optional content groups) used by the pages are registered in the destination's
//!   `/OCProperties`, keeping their default on/off state.

use std::collections::{HashMap, HashSet};

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::{INHERITABLE, OrganizeError, pages_root, rebuild, walk};

mod carry;
mod combine;
mod copy;
mod fields;

pub(crate) use carry::set_attachments;
use carry::{Mark, source_outline, write_outline_level};
pub use combine::{
    CombineOutcome, Combiner, Run, SplitBy, combine, combine_grouped, combine_grouped_with, combine_selected, combine_selected_with, combine_with,
    extract_pages, split, split_ranges,
};
use copy::SKIP_ON_PAGE;
pub(crate) use copy::{Copier, copy_object};
use fields::{add_kid, field_and_widget, register_fields, resolve_field_conflicts, split_own_field};
pub use fields::{combine_field_conflicts, field_name_conflicts};

/// How colliding fully-qualified field names are resolved when pages are imported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldConflicts {
    /// The incoming field's own `/T` is renamed to the first free `Name-1`, `Name-2`… variant,
    /// so it stays its own field (the default; Combine and Insert from File use it).
    #[default]
    Rename,
    /// The older behaviour: a field that arrives with a taken name becomes one field with the
    /// one already there. Duplicate, Replace, Extract and Split always use it.
    Merge,
}

/// What [`import_pages_with`] did beyond placing the pages.
pub struct ImportOutcome {
    /// The new pages, in order.
    pub pages: Vec<ObjRef>,
    /// Every field renamed to keep its name separate: old fully-qualified name → new one.
    pub renamed_fields: Vec<(String, String)>,
}

/// Copy pages `src_pages` (0-based, in the given order) of `src` into `dst`, inserting them at
/// position `at`, resolving field-name collisions per `conflicts`. Returns the new page
/// references in order and what the conflict resolution renamed.
///
/// Resources identical to ones already in `dst` (the same fonts, images, colour profiles) are
/// shared rather than stored twice (see `dedupe`).
pub fn import_pages_with(
    dst: &mut Document,
    src: &Document,
    src_pages: &[usize],
    at: usize,
    conflicts: FieldConflicts,
) -> Result<ImportOutcome, OrganizeError> {
    let before: HashSet<u32> = dst.object_numbers().into_iter().collect();
    let imported = match import_pages_mapped(dst, src, src_pages, at, conflicts) {
        Ok(imported) => imported,
        Err(e) => {
            // A refusal while copying (a source too deep to follow) has only added objects:
            // take them back, so the destination is as it was.
            for r in added_objects(dst, &before) {
                dst.free(r);
            }
            return Err(e);
        }
    };
    let created = added_objects(dst, &before);
    crate::dedupe::dedupe_resources(dst, &created, true);
    Ok(ImportOutcome { pages: imported.pages, renamed_fields: imported.renamed_fields })
}

/// The objects in `dst` that are not among `before`, the numbers it held before an import: the
/// ones the import added. They are not always numbered above the others: when no number is left
/// above the highest (the document holds object 4294967295, or its `/Size` is exhausted),
/// [`Document::add`] fills free numbers below it.
fn added_objects(dst: &Document, before: &HashSet<u32>) -> Vec<ObjRef> {
    dst.object_numbers().into_iter().filter(|n| !before.contains(n)).map(|n| ObjRef::new(n, dst.generation(n))).collect()
}

/// Insert Pages from File: [`import_pages_with`], and the inserted file's bookmarks come along
/// under one new bookmark titled `title` (the file's name) at its first inserted page, as Combine
/// Files does for each file. That bookmark goes among the top-level bookmarks where the pages went:
/// before the first one that points after them. Bookmarks to pages left out lose their destination;
/// a file without bookmarks adds none.
pub fn insert_file_pages(
    dst: &mut Document,
    src: &Document,
    title: &str,
    src_pages: &[usize],
    at: usize,
    conflicts: FieldConflicts,
) -> Result<ImportOutcome, OrganizeError> {
    let before: HashSet<u32> = dst.object_numbers().into_iter().collect();
    let imported = import_pages_mapped(dst, src, src_pages, at, conflicts)?;
    let created = added_objects(dst, &before);
    crate::dedupe::dedupe_resources(dst, &created, true);
    let outline = source_outline(src, imported.page_map);
    if let Some(first) = imported.pages.first()
        && !outline.is_empty()
    {
        let after = at.min(crate::page_count(dst)?.saturating_sub(imported.pages.len())).saturating_add(imported.pages.len());
        insert_file_bookmark(dst, &Mark { title: title.to_string(), page: *first, outline }, after)?;
    }
    Ok(ImportOutcome { pages: imported.pages, renamed_fields: imported.renamed_fields })
}

/// Add `mark` (a file's bookmark holding its own, collapsed) to the top level of `doc`'s outline,
/// before the first top-level bookmark that points at page `after` (0-based) or later.
fn insert_file_bookmark(doc: &mut Document, mark: &Mark, after: usize) -> Result<(), OrganizeError> {
    let index = crate::outline::top_level_bookmarks(doc).iter().position(|b| b.page.is_some_and(|p| p >= after));
    let root = crate::outline::container(doc, &[])?;
    let item = doc.add(Object::Null);
    let mut d = Dict::new();
    d.set(b"Title".to_vec(), Object::String(pdfcraft_cos::PdfString::text(&mark.title)));
    d.set(b"Dest".to_vec(), Object::Array(vec![Object::Ref(mark.page), Object::name("Fit")]));
    if let Some((f, l, n)) = write_outline_level(doc, item, &mark.outline)? {
        d.set(b"First".to_vec(), Object::Ref(f));
        d.set(b"Last".to_vec(), Object::Ref(l));
        d.set(b"Count".to_vec(), Object::Int(-(n as i64))); // collapsed
    }
    doc.set(item, Object::Dict(d));
    let mut top = crate::outline::children(doc, root);
    top.insert(index.unwrap_or(top.len()).min(top.len()), item);
    crate::outline::relink_and_recount(doc, root, &top)?;
    Ok(())
}

/// Copy pages `src_pages` (0-based, in the given order) of `src` into `dst`, inserting them at
/// position `at`. Returns the new page references in order.
///
/// Fields keep their names ([`FieldConflicts::Merge`]): this is what Duplicate and Replace
/// Pages use, where the incoming fields are copies of the document's own and renaming them
/// would silently change what the form exports. Combine and Insert from File, which bring in
/// another document's fields, use [`import_pages_with`] and [`FieldConflicts::Rename`].
///
/// Resources identical to ones already in `dst` (the same fonts, images, colour profiles) are
/// shared rather than stored twice (see `dedupe`).
pub fn import_pages(dst: &mut Document, src: &Document, src_pages: &[usize], at: usize) -> Result<Vec<ObjRef>, OrganizeError> {
    import_pages_with(dst, src, src_pages, at, FieldConflicts::Merge).map(|o| o.pages)
}

/// What `import_pages_mapped` copied.
struct Imported {
    /// The new pages, in order.
    pages: Vec<ObjRef>,
    /// Source page → destination page.
    page_map: HashMap<ObjRef, ObjRef>,
    /// Every source object copied → its copy.
    objects: HashMap<ObjRef, ObjRef>,
    /// Fields renamed to keep their names separate (old → new fully-qualified name).
    renamed_fields: Vec<(String, String)>,
}

/// `import_pages`, also returning what was copied from where.
fn import_pages_mapped(
    dst: &mut Document,
    src: &Document,
    src_pages: &[usize],
    at: usize,
    conflicts: FieldConflicts,
) -> Result<Imported, OrganizeError> {
    let source = walk(src)?;
    if let Some(bad) = src_pages.iter().find(|i| **i >= source.len()) {
        return Err(OrganizeError::NoSuchPage(*bad));
    }
    let mut existing = walk(dst)?;
    let root = pages_root(dst)?;
    let mut copier = Copier::new(src, HashMap::new(), HashMap::new());
    // Allocate every destination page first, so destinations between copied pages resolve.
    let targets: Vec<(ObjRef, ObjRef, Dict)> = src_pages
        .iter()
        .map(|&i| {
            let (page, inherited) = &source[i];
            let new = match copier.pages.get(page) {
                Some(n) => *n, // the same page requested twice gets a second copy below
                None => dst.add(Object::Null),
            };
            copier.pages.insert(*page, new);
            (*page, new, inherited.clone())
        })
        .collect();
    let mut annot_pages = HashMap::new();
    let mut new_pages = Vec::new();
    let mut done: HashMap<ObjRef, ObjRef> = HashMap::new();
    for (page, new, inherited) in targets {
        // A page listed twice: make a second, independent page object sharing resources, and
        // with copies of its own annotations (an annotation belongs to one page: its `/P`, its
        // popup, a widget's place). Destinations still map to the pages copied.
        let repeat = done.contains_key(&page);
        let new = if repeat { dst.add(Object::Null) } else { new };
        let mut own = repeat.then(|| Copier::new(src, HashMap::new(), copier.pages.clone()));
        let src_dict = src.get(page).as_dict().cloned().unwrap_or_default();
        let resources = resources_with_ancestors(src, &src_dict);
        let mut d = Dict::new();
        for (k, v) in src_dict.iter() {
            if SKIP_ON_PAGE.contains(&k.as_slice()) || (k.as_slice() == b"Resources" && resources.is_some()) {
                continue;
            }
            if k.as_slice() == b"Annots" {
                let list = src.resolve(v).as_array().cloned().unwrap_or_default();
                let mut out = Vec::new();
                for a in list {
                    // On a repeated page a form field's widget becomes one more widget of the
                    // same field (copied already with the page's first copy): one field, one
                    // value, shown on both pages. Copying the field again would make two fields
                    // of one name, filled in separately.
                    let resolved = src.resolve(&a);
                    let field = resolved.as_dict().filter(|d| d.name(b"Subtype") == Some(b"Widget")).and_then(|d| d.reference(b"Parent"));
                    if repeat
                        && let Some(parent) = field
                        && let Some(copied) = copier.map.get(&parent).copied()
                    {
                        let r = dst.add(Object::Null);
                        let c = copier.copy_value(dst, &resolved, Some(r))?;
                        dst.set(r, c);
                        add_kid(dst, copied, r)?;
                        annot_pages.insert(r, new);
                        out.push(Object::Ref(r));
                        continue;
                    }
                    // A widget that is its own field (no parent to hang a second widget on): on
                    // its first copy it is split into a parent field and a widget child (#1810);
                    // every later copy of the page gets one more widget child of that field, so
                    // the pages no longer share one widget whose `/P` names only the last page.
                    let own_field = resolved.as_dict().is_some_and(|d| d.name(b"Subtype") == Some(b"Widget") && !d.contains(b"Parent"));
                    if own_field
                        && let Some(first) = a.as_ref().and_then(|r| copier.map.get(&r).copied())
                        && let Some(parent) = dst.get(first).as_dict().and_then(|d| d.reference(b"Parent"))
                    {
                        let r = dst.add(Object::Null);
                        let c = copier.copy_value(dst, &resolved, Some(r))?;
                        let Object::Dict(d) = c else { continue };
                        let (_, mut d) = field_and_widget(dst, &d);
                        d.set(b"Parent".to_vec(), Object::Ref(parent));
                        // The new widget is not a field root of its own.
                        copier.fields.retain(|f| *f != r);
                        dst.set(r, Object::Dict(d));
                        add_kid(dst, parent, r)?;
                        annot_pages.insert(r, new);
                        out.push(Object::Ref(r));
                        continue;
                    }
                    let annots = if own_field { &mut copier } else { own.as_mut().unwrap_or(&mut copier) };
                    // Direct (inline) annotation dictionaries become indirect objects, so every
                    // copied annotation can be fixed up the same way.
                    let r = match a {
                        Object::Ref(r) => annots.copy_ref(dst, r)?,
                        Object::Dict(_) => {
                            let r = dst.add(Object::Null);
                            let c = annots.copy_value(dst, &a, Some(r))?;
                            dst.set(r, c);
                            r
                        }
                        _ => continue,
                    };
                    if own_field && copier.fields.contains(&r) {
                        split_own_field(dst, &mut copier.fields, r);
                    }
                    annot_pages.insert(r, new);
                    out.push(Object::Ref(r));
                }
                d.set(b"Annots".to_vec(), Object::Array(out));
                continue;
            }
            d.set(k.clone(), copier.copy_value(dst, v, None)?);
        }
        if let Some(r) = resources {
            let r = copier.copy_value(dst, &r, None)?;
            d.set(b"Resources".to_vec(), r);
        }
        for k in INHERITABLE {
            if !d.contains(k)
                && let Some(v) = inherited.get(k)
            {
                let v = copier.copy_value(dst, v, None)?;
                d.set(k.to_vec(), v);
            }
        }
        d.set(b"Type".to_vec(), Object::name("Page"));
        d.set(b"Parent".to_vec(), Object::Ref(root));
        dst.set(new, Object::Dict(d));
        // The repeat's own annotations are fixed up, its fields and layers registered, with the rest.
        if let Some(own) = own {
            copier.annots.extend(own.annots);
            copier.fields.extend(own.fields);
            copier.ocgs.extend(own.ocgs);
        }
        done.insert(page, new);
        new_pages.push(new);
    }
    copier.fix_annotations(dst, &annot_pages)?;
    let renamed_fields = match conflicts {
        FieldConflicts::Rename => resolve_field_conflicts(dst, &copier.fields)?,
        FieldConflicts::Merge => Vec::new(),
    };
    register_fields(dst, &copier.fields)?;
    register_layers(dst, src, &copier.ocgs)?;
    let at = at.min(existing.len());
    let inserted: Vec<(ObjRef, Dict)> = new_pages.iter().map(|r| (*r, Dict::new())).collect();
    existing.splice(at..at, inserted);
    rebuild(dst, &existing)?;
    Ok(Imported { pages: new_pages, page_map: copier.pages, objects: copier.map, renamed_fields })
}

/// A page's `/Resources` with the names that only its page-tree ancestors list, or `None` when
/// they add nothing to the dictionary it would have anyway (its own, or else its nearest
/// ancestor's). Readers (ours included) look a resource name up the page and then up the tree,
/// so a page that lists its font but inherits its images still shows them. A copy hangs from
/// another tree, so it must list them itself or it draws nothing of them (#1628). The page's
/// entry wins over an ancestor's, and a nearer ancestor's over a farther one's.
fn resources_with_ancestors(src: &Document, page: &Dict) -> Option<Object> {
    let mut merged = page.get(b"Resources").map(|r| src.dict(r).unwrap_or_default());
    let mut added = false;
    let mut seen = std::collections::HashSet::new();
    let mut parent = page.reference(b"Parent");
    // Bounded: a damaged tree's `/Parent` links can loop or run deep.
    while let Some(r) = parent
        && seen.insert(r)
        && seen.len() <= 256
    {
        let node = src.get(r);
        let Some(node) = node.as_dict() else { break };
        parent = node.reference(b"Parent");
        let Some(theirs) = node.get(b"Resources").and_then(|r| src.dict(r)) else { continue };
        let Some(mine) = merged.as_mut() else {
            merged = Some(theirs);
            continue;
        };
        for (category, entries) in theirs.iter() {
            match (mine.get(category).map(|c| src.dict(c)), src.dict(entries)) {
                (None, _) => {
                    mine.set(category.clone(), entries.clone());
                    added = true;
                }
                (Some(Some(mut names)), Some(entries)) => {
                    let before = names.len();
                    for (name, value) in entries.iter() {
                        if !names.contains(name) {
                            names.set(name.clone(), value.clone());
                        }
                    }
                    if names.len() > before {
                        mine.set(category.clone(), Object::Dict(names));
                        added = true;
                    }
                }
                // `/ProcSet` and the like: the page's own value stands.
                _ => {}
            }
        }
    }
    merged.filter(|_| added).map(Object::Dict)
}

/// Look up a named destination in the source (`/Dests` dictionary or `/Names /Dests` tree).
fn named_dest(src: &Document, name: &[u8]) -> Option<Object> {
    let catalog = src.get(src.root()?).as_dict().cloned()?;
    if let Some(dests) = catalog.get(b"Dests").map(|d| src.resolve(d))
        && let Some(v) = dests.as_dict().and_then(|d| d.get(name))
    {
        return Some(src.resolve(v).as_ref().clone());
    }
    let names = src.resolve(catalog.get(b"Names")?);
    let tree = names.as_dict()?.get(b"Dests")?.clone();
    name_tree_lookup(src, &tree, name, 0, &mut HashSet::new())
}

/// Name trees are read once per node: a node listed twice (or a cycle) is skipped.
fn name_tree_lookup(src: &Document, node: &Object, key: &[u8], depth: u8, seen: &mut HashSet<ObjRef>) -> Option<Object> {
    if depth > 32 || node.as_ref().is_some_and(|r| !seen.insert(r)) {
        return None;
    }
    #[cfg(test)]
    crate::work(1);
    let node = src.resolve(node);
    let d = node.as_dict()?;
    if let Some(names) = d.get(b"Names").map(|n| src.resolve(n)).and_then(|n| n.as_array().cloned()) {
        for pair in names.chunks(2) {
            if let [k, v] = pair
                && src.resolve(k).as_string().is_some_and(|s| s.bytes == key)
            {
                return Some(src.resolve(v).as_ref().clone());
            }
        }
    }
    let kids = d.get(b"Kids").map(|k| src.resolve(k)).and_then(|k| k.as_array().cloned()).unwrap_or_default();
    kids.iter().find_map(|k| name_tree_lookup(src, k, key, depth + 1, seen))
}

/// All (key, value) pairs of a name tree, in order, each node read once.
pub(crate) fn name_tree_entries(src: &Document, node: &Object, depth: u8, seen: &mut HashSet<ObjRef>, out: &mut Vec<(Vec<u8>, Object)>) {
    if depth > 32 || node.as_ref().is_some_and(|r| !seen.insert(r)) {
        return;
    }
    #[cfg(test)]
    crate::work(1);
    let node = src.resolve(node);
    let Some(d) = node.as_dict() else { return };
    if let Some(names) = d.get(b"Names").map(|n| src.resolve(n)).and_then(|n| n.as_array().cloned()) {
        for pair in names.chunks(2) {
            if let [k, v] = pair
                && let Some(k) = src.resolve(k).as_string()
            {
                out.push((k.bytes.clone(), v.clone()));
            }
        }
    }
    for k in d.get(b"Kids").map(|k| src.resolve(k)).and_then(|k| k.as_array().cloned()).unwrap_or_default() {
        name_tree_entries(src, &k, depth + 1, seen, out);
    }
}

/// Register copied optional content groups in the destination's `/OCProperties`, keeping each
/// layer's default visibility from the source.
pub(crate) fn register_layers(dst: &mut Document, src: &Document, ocgs: &[(ObjRef, ObjRef)]) -> Result<(), OrganizeError> {
    if ocgs.is_empty() {
        return Ok(());
    }
    let src_config: Dict = src
        .root()
        .and_then(|r| src.get(r).as_dict().cloned())
        .and_then(|c| c.get(b"OCProperties").map(|o| src.resolve(o)))
        .and_then(|o| o.as_dict().and_then(|d| d.get(b"D")).map(|d| src.resolve(d)))
        .and_then(|d| d.as_dict().cloned())
        .unwrap_or_default();
    let src_list = |k: &[u8]| src_config.get(k).map(|v| src.resolve(v)).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let (src_on, src_off) = (src_list(b"ON"), src_list(b"OFF"));
    // With `/BaseState /OFF`, a layer is hidden unless `/ON` lists it (§8.11.4.3, Table 101).
    let base_off = src_config.get(b"BaseState").map(|b| src.resolve(b)).is_some_and(|b| b.as_name() == Some(b"OFF".as_slice()));
    let hidden = |r: &Object| src_off.contains(r) || (base_off && !src_on.contains(r));
    let root = dst.root().ok_or(OrganizeError::NoPageTree)?;
    let catalog = dst.get(root).as_dict().cloned().ok_or(OrganizeError::NoPageTree)?;
    let (props_ref, mut props) = match catalog.get(b"OCProperties") {
        Some(Object::Ref(r)) => (Some(*r), dst.get(*r).as_dict().cloned().unwrap_or_default()),
        Some(Object::Dict(d)) => (None, d.clone()),
        _ => (None, Dict::new()),
    };
    let list = |d: &Dict, k: &[u8], doc: &Document| d.get(k).map(|v| doc.resolve(v)).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut all = list(&props, b"OCGs", dst);
    let mut config = props.get(b"D").map(|d| dst.resolve(d)).and_then(|d| d.as_dict().cloned()).unwrap_or_default();
    let (mut on, mut off, mut order) = (list(&config, b"ON", dst), list(&config, b"OFF", dst), list(&config, b"Order", dst));
    for (from, to) in ocgs {
        let r = Object::Ref(*to);
        if all.contains(&r) {
            continue;
        }
        all.push(r.clone());
        order.push(r.clone());
        if hidden(&Object::Ref(*from)) { off.push(r) } else { on.push(r) }
    }
    props.set(b"OCGs".to_vec(), Object::Array(all));
    config.set(b"ON".to_vec(), Object::Array(on));
    config.set(b"OFF".to_vec(), Object::Array(off));
    config.set(b"Order".to_vec(), Object::Array(order));
    props.set(b"D".to_vec(), Object::Dict(config));
    match props_ref {
        Some(r) => dst.set(r, Object::Dict(props)),
        None => dst.update_dict(root, |c| c.set(b"OCProperties".to_vec(), Object::Dict(props)))?,
    }
    Ok(())
}

/// Page `page` of `src` as a form XObject in `dst` (its content and resources, no annotations),
/// for backgrounds and watermarks taken from a PDF. The form's `/BBox` is the page's crop box
/// and its `/Matrix` undoes the page rotation, so it draws upright as displayed. Returns the
/// form and its displayed size in points.
pub fn page_as_form(dst: &mut Document, src: &Document, page: usize) -> Result<(ObjRef, (f64, f64)), OrganizeError> {
    use pdfcraft_cos::Stream;
    let all = walk(src)?;
    // `walk` gives the inheritable attributes (resources, boxes, rotation); the content is
    // the page's own.
    let (pr, d) = all.get(page).cloned().ok_or(OrganizeError::NoSuchPage(page))?;
    let own = src.get(pr).as_dict().cloned().unwrap_or_default();
    let rect = |k: &[u8]| -> Option<[f64; 4]> {
        let a = src.resolve(d.get(k)?);
        let v: Vec<f64> = a.as_array()?.iter().filter_map(|x| src.resolve(x).as_f64()).collect();
        (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
    };
    let bbox = rect(b"CropBox").or_else(|| rect(b"MediaBox")).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let (w, h) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
    let rotate = d.get(b"Rotate").and_then(|r| src.resolve(r).as_int()).unwrap_or(0).rem_euclid(360);
    // Content: the page's streams, decoded and joined.
    let content = match own.get(b"Contents") {
        Some(c) => crate::prune::content_bytes(src, c).map_err(|e| OrganizeError::Invalid(format!("page {} content: {e}", page + 1)))?,
        None => Vec::new(),
    };
    let mut copier = Copier::new(src, HashMap::new(), HashMap::new());
    let resources = match d.get(b"Resources") {
        Some(r) => copier.copy_value(dst, r, None)?,
        None => Object::Dict(Dict::new()),
    };
    register_layers(dst, src, &copier.ocgs)?;
    let mut fd = Dict::new();
    fd.set(b"Type".to_vec(), Object::name("XObject"));
    fd.set(b"Subtype".to_vec(), Object::name("Form"));
    fd.set(b"BBox".to_vec(), Object::Array(bbox.iter().map(|v| Object::Real(*v)).collect()));
    // Map the crop box to (0, 0)–(w, h) as displayed (turning a rotated page upright).
    let m: [f64; 6] = match rotate {
        90 => [0.0, -1.0, 1.0, 0.0, -bbox[1], bbox[2]],
        180 => [-1.0, 0.0, 0.0, -1.0, bbox[2], bbox[3]],
        270 => [0.0, 1.0, -1.0, 0.0, bbox[3], -bbox[0]],
        _ => [1.0, 0.0, 0.0, 1.0, -bbox[0], -bbox[1]],
    };
    fd.set(b"Matrix".to_vec(), Object::Array(m.iter().map(|v| Object::Real(*v)).collect()));
    fd.set(b"Resources".to_vec(), resources);
    let r = dst.add(Object::Stream(Stream::flate(fd, &content)));
    let size = if rotate == 90 || rotate == 270 { (h, w) } else { (w, h) };
    Ok((r, size))
}
