use super::{Dict, Document, HashMap, ObjRef, Object, OrganizeError, named_dest};

/// Keys that must not be followed when copying: they point back up or across the source
/// document, and following them would drag in unrelated objects.
pub(super) const SKIP_ON_PAGE: &[&[u8]] = &[b"Parent", b"B", b"StructParents", b"Thumb", b"PieceInfo"];

const SKIP_ON_ANNOT: &[&[u8]] = &[b"StructParent", b"P"];

/// How deep a copy follows references and nested arrays and dictionaries, counted together.
/// Real documents stay well under this (a page's resources, fonts and descriptors, nested
/// form XObjects and field trees are a few levels each); a chain of thousands of indirect
/// objects, each shallow enough for the parser, would otherwise exhaust the stack (#1800).
const MAX_COPY_DEPTH: usize = 256;

/// Keys that stay on the widget when a combined field/widget is split (#1810): the annotation's
/// appearance, placement and page. Field state (name, value, flags, default appearance) moves to
/// the new parent field, where the widgets inherit it.
pub(super) const WIDGET_KEYS: &[&[u8]] = &[
    b"Type",
    b"Subtype",
    b"Rect",
    b"P",
    b"AP",
    b"AS",
    b"MK",
    b"BS",
    b"Border",
    b"C",
    b"CA",
    b"F",
    b"H",
    b"R",
    b"NM",
    b"M",
    b"StructParent",
    b"StructParents",
    b"OC",
    b"Tabs",
    b"A",
    b"Contents",
    b"BM",
    b"Lang",
    b"AF",
];

pub(crate) struct Copier<'a> {
    pub(super) src: &'a Document,
    /// Current recursion depth, bounded by [`MAX_COPY_DEPTH`].
    depth: usize,
    pub(crate) map: HashMap<ObjRef, ObjRef>,
    /// Source page → destination page, for rewriting `/P` and destinations.
    pub(super) pages: HashMap<ObjRef, ObjRef>,
    /// Destination objects that are annotation dictionaries (fixed up after copying).
    pub(super) annots: Vec<ObjRef>,
    /// Top-level form fields reached through widgets.
    pub(super) fields: Vec<ObjRef>,
    /// Optional content groups copied: (source, destination).
    pub(crate) ocgs: Vec<(ObjRef, ObjRef)>,
}

impl<'a> Copier<'a> {
    pub(crate) fn new(src: &'a Document, map: HashMap<ObjRef, ObjRef>, pages: HashMap<ObjRef, ObjRef>) -> Self {
        Copier { src, depth: 0, map, pages, annots: Vec::new(), fields: Vec::new(), ocgs: Vec::new() }
    }

    /// Copy an indirect object (once), returning its destination reference.
    pub(super) fn copy_ref(&mut self, dst: &mut Document, r: ObjRef) -> Result<ObjRef, OrganizeError> {
        if let Some(d) = self.map.get(&r) {
            return Ok(*d);
        }
        // Reserve the number first so cycles (field ↔ widget) terminate.
        let new = dst.add(Object::Null);
        self.map.insert(r, new);
        let obj = self.src.get(r);
        if obj.as_dict().is_some_and(|d| d.name(b"Type") == Some(b"OCG")) {
            self.ocgs.push((r, new));
        }
        let copied = self.copy_value(dst, &obj, Some(new))?;
        dst.set(new, copied);
        Ok(new)
    }

    /// One level deeper into the object graph, or the refusal when there are too many.
    fn descend(&mut self) -> Result<(), OrganizeError> {
        if self.depth >= MAX_COPY_DEPTH {
            return Err(OrganizeError::Invalid(format!("the pages refer to objects nested more than {MAX_COPY_DEPTH} levels deep")));
        }
        self.depth += 1;
        Ok(())
    }

    pub(crate) fn copy_value(&mut self, dst: &mut Document, o: &Object, this: Option<ObjRef>) -> Result<Object, OrganizeError> {
        if !matches!(o, Object::Ref(_) | Object::Array(_) | Object::Dict(_) | Object::Stream(_)) {
            return Ok(o.clone());
        }
        self.descend()?;
        let copied = match o {
            Object::Ref(r) => self.copy_ref(dst, *r).map(Object::Ref),
            Object::Array(a) => a.iter().map(|x| self.copy_value(dst, x, None)).collect::<Result<Vec<_>, _>>().map(Object::Array),
            Object::Dict(d) => self.copy_dict(dst, d, this).map(Object::Dict),
            Object::Stream(s) => self.copy_dict(dst, &s.dict, None).map(|dict| {
                let mut s = s.clone();
                s.dict = dict;
                Object::Stream(s)
            }),
            other => Ok(other.clone()),
        };
        self.depth = self.depth.saturating_sub(1);
        copied
    }

    fn copy_dict(&mut self, dst: &mut Document, d: &Dict, this: Option<ObjRef>) -> Result<Dict, OrganizeError> {
        let is_annot = d.name(b"Type") == Some(b"Annot") || (d.contains(b"Subtype") && d.contains(b"Rect") && !d.contains(b"Type"));
        let is_field = d.contains(b"FT") || (d.contains(b"T") && d.contains(b"Kids"));
        let mut out = Dict::new();
        for (k, v) in d.iter() {
            if is_annot && SKIP_ON_ANNOT.contains(&k.as_slice()) {
                continue;
            }
            // Destinations are rewritten afterwards (they may target pages copied later).
            if is_annot && (k.as_slice() == b"Dest" || k.as_slice() == b"A") {
                out.set(k.clone(), v.clone());
                continue;
            }
            out.set(k.clone(), self.copy_value(dst, v, None)?);
        }
        if let Some(this) = this {
            if is_annot {
                self.annots.push(this);
            }
            // A field (or widget) without a parent is a top-level field.
            if is_field && !d.contains(b"Parent") {
                self.fields.push(this);
            }
        }
        Ok(out)
    }

    /// Map a source destination (explicit array or reference to one) to the destination
    /// document; `None` if it targets a page that was not copied.
    pub(super) fn map_dest(&self, dest: &Object) -> Option<Object> {
        self.map_dest_depth(dest, 0)
    }

    fn map_dest_depth(&self, dest: &Object, depth: u8) -> Option<Object> {
        if depth > 4 {
            return None;
        }
        match dest {
            Object::Name(n) => self.map_dest_depth(&named_dest(self.src, n)?, depth + 1),
            Object::String(s) => self.map_dest_depth(&named_dest(self.src, &s.bytes)?, depth + 1),
            Object::Ref(_) => self.map_dest_depth(&self.src.resolve(dest), depth + 1),
            Object::Dict(d) => self.map_dest_depth(d.get(b"D")?, depth + 1),
            Object::Array(a) => {
                let page = a.first()?.as_ref()?;
                let new_page = *self.pages.get(&page)?;
                let mut a = a.clone();
                a[0] = Object::Ref(new_page);
                Some(Object::Array(a))
            }
            _ => None,
        }
    }

    /// Rewrite `/P`, `/Dest` and GoTo actions on copied annotations.
    pub(crate) fn fix_annotations(&mut self, dst: &mut Document, annot_pages: &HashMap<ObjRef, ObjRef>) -> Result<(), OrganizeError> {
        for &a in &std::mem::take(&mut self.annots) {
            let src_dict = dst.get(a).as_dict().cloned();
            let Some(d) = src_dict else { continue };
            let mut d = d;
            if let Some(p) = annot_pages.get(&a) {
                d.set(b"P".to_vec(), Object::Ref(*p));
            }
            if let Some(dest) = d.get(b"Dest").cloned() {
                let resolved = self.src.resolve(&dest);
                match self.map_dest(&resolved) {
                    Some(nd) => d.set(b"Dest".to_vec(), nd),
                    None => {
                        d.remove(b"Dest");
                    }
                }
            }
            if let Some(action) = d.get(b"A").cloned() {
                let action = self.src.resolve(&action);
                if let Some(ad) = action.as_dict() {
                    if ad.name(b"S") == Some(b"GoTo") {
                        let target = ad.get(b"D").map(|x| self.src.resolve(x));
                        match target.and_then(|t| self.map_dest(&t)) {
                            Some(nd) => {
                                let mut na = Dict::new();
                                na.set(b"S".to_vec(), Object::name("GoTo"));
                                na.set(b"D".to_vec(), nd);
                                d.set(b"A".to_vec(), Object::Dict(na));
                            }
                            None => {
                                d.remove(b"A");
                            }
                        }
                    } else {
                        // Other actions (URI, Launch, JavaScript…) carry no page references. The
                        // copier lends its map out and takes it back with the new copies in it,
                        // so actions sharing an object share its one copy.
                        let mut c = Copier::new(self.src, std::mem::take(&mut self.map), HashMap::new());
                        let copied = c.copy_value(dst, &Object::Dict(ad.clone()), None);
                        self.map = c.map;
                        d.set(b"A".to_vec(), copied?);
                    }
                }
            }
            dst.set(a, Object::Dict(d));
        }
        Ok(())
    }
}

/// Copy `o` from `src` into `dst`, reusing the copies listed in `map` (source → destination), so
/// an object the copied pages already brought along (an ICC profile) is not stored twice.
pub(crate) fn copy_object(dst: &mut Document, src: &Document, o: &Object, map: HashMap<ObjRef, ObjRef>) -> Result<Object, OrganizeError> {
    Copier::new(src, map, HashMap::new()).copy_value(dst, o, None)
}
