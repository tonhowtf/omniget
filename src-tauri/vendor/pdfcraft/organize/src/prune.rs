//! Keeps only the XObjects a part's pages actually draw (Split / Extract Pages, #204).
//!
//! In a real book all pages share one `/Resources` dictionary, so every split part carried all
//! ~2,900 image XObjects while its 1,500 pages drew ~300 of them, and the part saved out at the
//! source's size. This pass follows each part's content streams (`Do`, form XObjects included)
//! and drops the `/XObject` entries no page names; the garbage-collecting rewrite on save then
//! drops the objects themselves.

use std::collections::{HashMap, HashSet};

use pdfcraft_cos::{CosError, DecodeBudget, Dict, Document, ObjRef, Object};

use crate::OrganizeError;

/// Form XObjects nested deeper than this are a broken file, not a reason to hang.
const MAX_FORM_DEPTH: u8 = 8;
/// Count every resource object and direct child; a graph beyond this is left unpruned.
const MAX_GLYPH_RESOURCE_WORK: usize = 10_000;

/// The dictionary a set of drawn names belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum XKey {
    /// An indirect `/XObject` dictionary.
    XObject(ObjRef),
    /// An inline `/XObject` dictionary inside an indirect resources object.
    Resources(ObjRef),
    /// Everything inline on the page itself.
    Page(ObjRef),
}

/// Who owns the resources currently being walked, for the key drawn names are recorded under.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Owner {
    /// An indirect resources object (a page's, or a form XObject's own).
    Resources(ObjRef),
    /// Resources inline on the page.
    Page(ObjRef),
}

struct Pruner<'a> {
    doc: &'a Document,
    used: HashMap<XKey, HashSet<Vec<u8>>>,
    /// Dictionaries whose drawn names can't be known for certain (undecodable content, nesting
    /// past the depth limit, Type 3 glyphs drawing through them). They are left untouched:
    /// pruning must never drop an XObject something still draws.
    unsure: HashSet<XKey>,
    /// An overlarge glyph-resource graph is not safe to prune at all.
    glyph_budget_exhausted: bool,
    glyph_work_left: usize,
    glyph_seen: HashSet<ObjRef>,
    /// An indirect font's explicit resources have one owner regardless of which page uses it.
    glyph_fonts: HashSet<ObjRef>,
    /// Forms already analysed, with the owner of the resources they drew against: drawing one
    /// again records nothing new, and re-reading it on every `Do` is exponential in the nesting.
    forms_seen: HashSet<(ObjRef, Owner)>,
}

impl Pruner<'_> {
    /// Mark the `/XObject` dictionary of `resources` as one not to prune.
    fn distrust(&mut self, resources: &Dict, owner: Owner) {
        if let Some((_, key)) = self.xobjects(resources, owner) {
            self.unsure.insert(key);
        }
    }

    fn glyph_work(&mut self, count: usize) -> bool {
        if !self.glyph_budget_exhausted
            && let Some(left) = self.glyph_work_left.checked_sub(count)
        {
            self.glyph_work_left = left;
            return true;
        }
        self.glyph_budget_exhausted = true;
        false
    }

    /// Walk one object's direct children without cloning them or recursing. Referenced objects
    /// go on a separate work list, so their borrowed dictionaries stay alive during this pass.
    fn protect_glyph_tree(&mut self, object: &Object, referenced: &mut Vec<ObjRef>) {
        if !self.glyph_work(1) {
            return;
        }
        let mut pending = vec![object];
        while let Some(object) = pending.pop() {
            match object {
                Object::Ref(r) => {
                    // A glyph's forms, patterns and soft masks can share either the XObject
                    // dictionary itself or a resource owner with content we do interpret.
                    self.unsure.insert(XKey::XObject(*r));
                    self.unsure.insert(XKey::Resources(*r));
                    self.unsure.insert(XKey::Page(*r));
                    if self.glyph_seen.insert(*r) {
                        referenced.push(*r);
                    }
                }
                Object::Array(array) => {
                    if !self.glyph_work(array.len()) {
                        return;
                    }
                    pending.extend(array);
                }
                Object::Dict(_) | Object::Stream(_) => {
                    if let Some(dict) = object.as_dict() {
                        if !self.glyph_work(dict.len()) {
                            return;
                        }
                        pending.extend(dict.iter().map(|(_, value)| value));
                    }
                }
                _ => {}
            }
        }
    }

    /// Glyph programs are not interpreted here. Keep every resource owner reachable from their
    /// resource graph, including nested forms, patterns and soft masks. Data bytes are never
    /// decoded; references are visited once and all direct children count against the budget.
    fn protect_glyph_object(&mut self, object: &Object) {
        let mut referenced = Vec::new();
        self.protect_glyph_tree(object, &mut referenced);
        while !self.glyph_budget_exhausted {
            let Some(r) = referenced.pop() else { break };
            let object = self.doc.get(r);
            self.protect_glyph_tree(&object, &mut referenced);
        }
    }

    fn protect_type3_resources(&mut self, resources: &Dict, owner: Owner) {
        if self.glyph_budget_exhausted {
            return;
        }
        let Some(fonts) = resources.get(b"Font").map(|f| self.doc.resolve(f)) else { return };
        let Some(fonts) = fonts.as_dict() else { return };
        // Bound discovery too, but don't spend the glyph-graph budget repeatedly examining an
        // ordinary shared font catalog on every page of a large document.
        if fonts.len() > MAX_GLYPH_RESOURCE_WORK {
            self.glyph_budget_exhausted = true;
            return;
        }
        for (_, font) in fonts.iter() {
            if self.glyph_budget_exhausted {
                return;
            }
            let font_ref = font.as_ref();
            let resolved = self.doc.resolve(font);
            let Some(font) = resolved.as_dict().filter(|f| f.name(b"Subtype") == Some(b"Type3")) else { continue };
            if let Some(glyph_resources) = font.get(b"Resources") {
                if let Some(r) = font_ref
                    && !self.glyph_fonts.insert(r)
                {
                    continue;
                }
                self.protect_glyph_object(glyph_resources);
            } else {
                self.distrust(resources, owner);
                // This root dictionary is already owned by the content caller. Only its
                // referenced descendants can share additional owners elsewhere in the file.
                for (_, value) in resources.iter() {
                    self.protect_glyph_object(value);
                    if self.glyph_budget_exhausted {
                        return;
                    }
                }
            }
        }
    }

    /// The `/XObject` dictionary of `resources`, with the key its drawn names belong to.
    fn xobjects(&self, resources: &Dict, owner: Owner) -> Option<(Dict, XKey)> {
        let x = resources.get(b"XObject")?;
        if let Some(r) = x.as_ref() {
            return Some((self.doc.resolve(x).as_dict()?.clone(), XKey::XObject(r)));
        }
        let key = match owner {
            Owner::Resources(r) => XKey::Resources(r),
            Owner::Page(p) => XKey::Page(p),
        };
        Some((self.doc.resolve(x).as_dict()?.clone(), key))
    }

    /// Record every XObject name `data` draws with `Do`, descending into form XObjects.
    fn content(&mut self, data: &[u8], resources: &Dict, owner: Owner, depth: u8) {
        self.protect_type3_resources(resources, owner);
        let Some((xobjects, key)) = self.xobjects(resources, owner) else {
            return;
        };
        if depth > MAX_FORM_DEPTH {
            self.unsure.insert(key);
            return;
        }
        for op in pdfcraft_content::parse(data).ops {
            if op.op.as_slice() != b"Do" {
                continue;
            }
            let Some(name) = op.name(0) else { continue };
            self.used.entry(key).or_default().insert(name.to_vec());
            let Some(r) = xobjects.get(name).and_then(Object::as_ref) else { continue };
            let Object::Stream(s) = &*self.doc.get(r) else { continue };
            if s.dict.name(b"Subtype") != Some(b"Form") {
                continue;
            }
            let (inner, inner_owner) = match s.dict.get(b"Resources") {
                Some(res) => match res.as_ref() {
                    Some(rr) => match self.doc.get(rr).as_dict().cloned() {
                        Some(d) => (d, Owner::Resources(rr)),
                        None => continue,
                    },
                    None => match self.doc.resolve(res).as_dict().cloned() {
                        // The form carries its resources inline: filter them on the form itself.
                        Some(d) => (d, Owner::Resources(r)),
                        None => continue,
                    },
                },
                // A form without its own resources draws against the ones it was invoked with.
                None => (resources.clone(), owner),
            };
            if !self.forms_seen.insert((r, inner_owner)) {
                continue;
            }
            #[cfg(test)]
            crate::work(1);
            match s.decoded() {
                Ok(data) => self.content(&data, &inner, inner_owner, depth + 1),
                // What the form draws is unknown: keep everything it could reach.
                Err(_) => {
                    self.protect_type3_resources(&inner, inner_owner);
                    self.distrust(&inner, inner_owner);
                }
            }
        }
    }
}

/// The page's content, concatenated (`/Contents` may be one stream or an array) with a space
/// between pieces as the renderer joins them, decoded against one page's [`DecodeBudget`]. An
/// error when a stream can't be decoded or the budget runs out: the page may draw names that
/// can't be seen.
pub(crate) fn content_bytes(doc: &Document, contents: &Object) -> Result<Vec<u8>, CosError> {
    let mut out = Vec::new();
    for data in DecodeBudget::page().decode_contents(doc, contents) {
        out.extend(data?);
        out.push(b' ');
    }
    Ok(out)
}

/// The resources of a page (as `walk` reports them) with their owner.
fn resolve_resources(doc: &Document, resources: &Object, page: ObjRef) -> Option<(Dict, Owner)> {
    match resources {
        Object::Ref(r) => doc.get(*r).as_dict().cloned().map(|d| (d, Owner::Resources(*r))),
        Object::Dict(d) => Some((d.clone(), Owner::Page(page))),
        _ => None,
    }
}

/// Drop the entries `names` never draws. Returns how many were removed.
fn drop_unused(xobjects: &mut Dict, names: &HashSet<Vec<u8>>) -> usize {
    let unused: Vec<Vec<u8>> = xobjects.iter().map(|(k, _)| k.clone()).filter(|k| !names.contains(k)).collect();
    for k in &unused {
        xobjects.remove(k);
    }
    unused.len()
}

/// Filter the dictionary `key` names to `names`.
fn filter_at(doc: &mut Document, key: XKey, names: &HashSet<Vec<u8>>) -> Result<usize, OrganizeError> {
    match key {
        XKey::XObject(r) => {
            let mut removed = 0;
            doc.update_dict(r, |d| removed = drop_unused(d, names))?;
            Ok(removed)
        }
        XKey::Resources(r) => {
            let Some(d) = doc.get(r).as_dict().cloned() else { return Ok(0) };
            let Some(x) = d.get(b"XObject") else { return Ok(0) };
            if x.as_ref().is_some() {
                return Ok(0); // recorded as XObject(r) instead
            }
            let Some(mut xd) = doc.resolve(x).as_dict().cloned() else { return Ok(0) };
            let removed = drop_unused(&mut xd, names);
            if removed > 0 {
                doc.update_dict(r, |d| d.set(b"XObject".to_vec(), Object::Dict(xd.clone())))?;
            }
            Ok(removed)
        }
        XKey::Page(p) => {
            let Some(pd) = doc.get(p).as_dict().cloned() else { return Ok(0) };
            let Some(res) = pd.get(b"Resources") else { return Ok(0) };
            if res.as_ref().is_some() {
                return Ok(0); // recorded as Resources(r) instead
            }
            let Some(mut rd) = doc.resolve(res).as_dict().cloned() else { return Ok(0) };
            let Some(x) = rd.get(b"XObject") else { return Ok(0) };
            if x.as_ref().is_some() {
                return Ok(0);
            }
            let Some(mut xd) = doc.resolve(x).as_dict().cloned() else { return Ok(0) };
            let removed = drop_unused(&mut xd, names);
            if removed > 0 {
                rd.set(b"XObject".to_vec(), Object::Dict(xd));
                doc.update_dict(p, |d| d.set(b"Resources".to_vec(), Object::Dict(rd.clone())))?;
            }
            Ok(removed)
        }
    }
}

/// Drop the page-resource entries the document's pages never draw. Returns how many were removed.
pub(crate) fn prune_unused_xobjects(doc: &mut Document) -> Result<usize, OrganizeError> {
    let pages = crate::walk(doc)?;
    let mut pruner = Pruner {
        doc,
        used: HashMap::new(),
        unsure: HashSet::new(),
        glyph_budget_exhausted: false,
        glyph_work_left: MAX_GLYPH_RESOURCE_WORK,
        glyph_seen: HashSet::new(),
        glyph_fonts: HashSet::new(),
        forms_seen: HashSet::new(),
    };
    for (page, attrs) in &pages {
        let Some(res) = attrs.get(b"Resources") else { continue };
        let Some((res_dict, owner)) = resolve_resources(pruner.doc, res, *page) else { continue };
        let Some(pd) = pruner.doc.get(*page).as_dict().cloned() else { continue };
        let Some(contents) = pd.get(b"Contents").cloned() else { continue };
        match content_bytes(pruner.doc, &contents) {
            Ok(data) if data.is_empty() => {}
            Ok(data) => pruner.content(&data, &res_dict, owner, 0),
            Err(_) => {
                pruner.protect_type3_resources(&res_dict, owner);
                pruner.distrust(&res_dict, owner);
            }
        }
    }
    let Pruner { used, unsure, glyph_budget_exhausted, .. } = pruner;
    if glyph_budget_exhausted {
        return Ok(0);
    }
    let mut removed = 0;
    for (key, names) in used {
        if !unsure.contains(&key) {
            removed += filter_at(doc, key, &names)?;
        }
    }
    Ok(removed)
}
