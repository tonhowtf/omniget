//! Merging duplicate resources after pages are copied (execution plan M4.5).
//!
//! Combining a document with itself, or inserting pages from a file that uses the same fonts,
//! would otherwise store every font, image and colour profile once per source. Here, objects
//! that are byte-for-byte the same resource after copying are merged into one, and references
//! are rewritten to it.
//!
//! - Only **resource-like** objects are merged: streams (font files, images, form XObjects, ICC
//!   profiles, functions…), font, font-descriptor, graphics-state, pattern, shading, function and
//!   encoding dictionaries, and colour-space arrays. Objects whose *identity* matters (pages,
//!   annotations, form fields, layers, outline items, structure elements) are never merged.
//! - Two objects are equal when they are structurally identical with references compared after
//!   merging, so a font dictionary merges once its font file has. This runs to a fixpoint. A
//!   reference to a plain value (a name, number, boolean or string, such as ImageMagick's
//!   `/ColorSpace 10 0 R` → `/DeviceRGB`) compares as that value; the value objects themselves
//!   are never merged.
//! - Only `candidates` (objects created by the operation) are ever replaced or rewritten. Existing
//!   objects can serve as the merge target but are not changed, so an incremental save stays
//!   small.
//! - Merged objects may now be shared by several pages. Editing code must copy on write (cos
//!   objects are immutable values, so a changed resource is always a new object).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use pdfcraft_cos::{Document, ObjRef, Object};

const RESOURCE_TYPES: &[&[u8]] = &[b"Font", b"FontDescriptor", b"ExtGState", b"Pattern", b"Encoding", b"CMap", b"Halftone"];
const COLOR_SPACE_FAMILIES: &[&[u8]] = &[b"ICCBased", b"Indexed", b"Separation", b"DeviceN", b"CalRGB", b"CalGray", b"Lab", b"Pattern"];

fn is_resource(o: &Object) -> bool {
    match o {
        Object::Stream(s) => !matches!(s.dict.get(b"Type"), Some(Object::Name(n)) if n == b"XRef" || n == b"ObjStm"),
        Object::Dict(d) => {
            matches!(d.get(b"Type"), Some(Object::Name(n)) if RESOURCE_TYPES.contains(&n.as_slice()))
                || d.contains(b"ShadingType")
                || d.contains(b"FunctionType")
        }
        Object::Array(a) => matches!(a.first(), Some(Object::Name(n)) if COLOR_SPACE_FAMILIES.contains(&n.as_slice())),
        _ => false,
    }
}

/// Resource dictionary categories whose entries are shareable resources. `/Properties` is left
/// out on purpose: it names optional content groups, whose identity matters.
const RESOURCE_CATEGORIES: &[&[u8]] = &[b"ExtGState", b"ColorSpace", b"Pattern", b"Shading", b"XObject", b"Font"];

/// References listed in any `/Resources` dictionary reachable inside `o` (pages, forms, Type 3
/// fonts, annotation appearances), including the `/Resources` dictionary itself when indirect.
fn collect_resource_refs(doc: &Document, o: &Object, out: &mut HashSet<ObjRef>) {
    let dict = match o {
        Object::Dict(d) => d,
        Object::Stream(s) => &s.dict,
        Object::Array(a) => {
            for x in a {
                collect_resource_refs(doc, x, out);
            }
            return;
        }
        _ => return,
    };
    for (k, v) in dict.iter() {
        if k.as_slice() == b"Resources" {
            if let Object::Ref(r) = v {
                out.insert(*r);
            }
            let res = doc.resolve(v);
            if let Some(res) = res.as_dict() {
                for cat in RESOURCE_CATEGORIES {
                    let entries = res.get(cat).map(|c| doc.resolve(c));
                    if let Some(entries) = entries.as_ref().and_then(|e| e.as_dict()) {
                        out.extend(entries.iter().filter_map(|(_, v)| if let Object::Ref(r) = v { Some(*r) } else { None }));
                    }
                }
            }
        } else if matches!(v, Object::Dict(_) | Object::Array(_)) {
            collect_resource_refs(doc, v, out);
        }
    }
}

/// Union-find style lookup of the object a reference now stands for.
fn canon(map: &HashMap<ObjRef, ObjRef>, mut r: ObjRef) -> ObjRef {
    while let Some(n) = map.get(&r) {
        r = *n;
    }
    r
}

/// The value behind `r` when it is a plain value (name, number, boolean or string): it then
/// means that value wherever it is used, whichever object holds it (#878).
fn plain_value(doc: &Document, r: ObjRef) -> Option<std::sync::Arc<Object>> {
    let o = doc.get(r);
    matches!(*o, Object::Bool(_) | Object::Int(_) | Object::Real(_) | Object::Name(_) | Object::String(_)).then_some(o)
}

fn hash_obj(o: &Object, doc: &Document, map: &HashMap<ObjRef, ObjRef>, h: &mut impl Hasher) {
    match o {
        Object::Null => 0u8.hash(h),
        Object::Bool(b) => (1u8, b).hash(h),
        Object::Int(i) => (2u8, i).hash(h),
        Object::Real(f) => (3u8, f.to_bits()).hash(h),
        Object::String(s) => (4u8, &s.bytes).hash(h),
        Object::Name(n) => (5u8, n).hash(h),
        Object::Array(a) => {
            (6u8, a.len()).hash(h);
            for x in a {
                hash_obj(x, doc, map, h);
            }
        }
        Object::Dict(d) => {
            // Key order does not change meaning: combine per-entry hashes order-independently.
            let mut acc = 0u64;
            for (k, v) in d.iter() {
                let mut eh = std::collections::hash_map::DefaultHasher::new();
                k.hash(&mut eh);
                hash_obj(v, doc, map, &mut eh);
                acc = acc.wrapping_add(eh.finish());
            }
            (7u8, d.len(), acc).hash(h);
        }
        Object::Stream(s) => {
            8u8.hash(h);
            hash_obj(&Object::Dict(s.dict.clone()), doc, map, h);
            s.raw[..].hash(h);
        }
        Object::Ref(r) => {
            let c = canon(map, *r);
            match plain_value(doc, c) {
                Some(v) => hash_obj(&v, doc, map, h),
                None => (9u8, c.num, c.generation).hash(h),
            }
        }
    }
}

fn eq_obj(a: &Object, b: &Object, doc: &Document, map: &HashMap<ObjRef, ObjRef>) -> bool {
    // Plain values behind references compare as the values (as `hash_obj` hashes them).
    let plain = |o: &Object| if let Object::Ref(r) = o { plain_value(doc, canon(map, *r)) } else { None };
    match (plain(a), plain(b)) {
        (Some(x), Some(y)) => return eq_obj(&x, &y, doc, map),
        (Some(x), None) => return eq_obj(&x, b, doc, map),
        (None, Some(y)) => return eq_obj(a, &y, doc, map),
        (None, None) => {}
    }
    match (a, b) {
        (Object::Array(x), Object::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| eq_obj(p, q, doc, map)),
        (Object::Dict(x), Object::Dict(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| eq_obj(v, w, doc, map))),
        (Object::Stream(x), Object::Stream(y)) => x.raw == y.raw && eq_obj(&Object::Dict(x.dict.clone()), &Object::Dict(y.dict.clone()), doc, map),
        (Object::Ref(x), Object::Ref(y)) => canon(map, *x) == canon(map, *y),
        (Object::Real(x), Object::Real(y)) => x.to_bits() == y.to_bits(),
        (Object::String(x), Object::String(y)) => x.bytes == y.bytes,
        _ => a == b,
    }
}

fn rewrite(o: &Object, map: &HashMap<ObjRef, ObjRef>) -> Object {
    match o {
        Object::Ref(r) => Object::Ref(canon(map, *r)),
        Object::Array(a) => Object::Array(a.iter().map(|x| rewrite(x, map)).collect()),
        Object::Dict(d) => {
            let mut out = d.clone();
            for (k, v) in d.iter() {
                out.set(k.clone(), rewrite(v, map));
            }
            Object::Dict(out)
        }
        Object::Stream(s) => {
            let mut s = s.clone();
            if let Object::Dict(d) = rewrite(&Object::Dict(s.dict.clone()), map) {
                s.dict = d;
            }
            Object::Stream(s)
        }
        other => other.clone(),
    }
}

fn contains_ref(o: &Object, map: &HashMap<ObjRef, ObjRef>) -> bool {
    match o {
        Object::Ref(r) => map.contains_key(r),
        Object::Array(a) => a.iter().any(|x| contains_ref(x, map)),
        Object::Dict(d) => d.iter().any(|(_, v)| contains_ref(v, map)),
        Object::Stream(s) => s.dict.iter().any(|(_, v)| contains_ref(v, map)),
        _ => false,
    }
}

/// Merge duplicate resources among `candidates` (into each other or into equal objects already
/// in `doc`), rewrite references inside `candidates`, and free the merged-away objects.
/// When `index_existing` is false, only candidates are compared (a brand-new document).
/// Returns how many objects were removed.
pub fn dedupe_resources(doc: &mut Document, candidates: &[ObjRef], index_existing: bool) -> usize {
    let cand: HashSet<ObjRef> = candidates.iter().copied().collect();
    let mut sorted: Vec<ObjRef> = candidates.to_vec();
    sorted.sort_unstable_by_key(|r| r.num);
    // Existing objects first, so candidates merge into them rather than the reverse.
    let mut all: Vec<(ObjRef, std::sync::Arc<Object>)> = Vec::new();
    if index_existing {
        for num in doc.object_numbers() {
            let r = ObjRef::new(num, doc.generation(num));
            if !cand.contains(&r) {
                all.push((r, doc.get(r)));
            }
        }
    }
    all.extend(sorted.iter().map(|r| (*r, doc.get(*r))));
    let mut listed = HashSet::new();
    for (_, o) in &all {
        collect_resource_refs(doc, o, &mut listed);
    }
    let universe: Vec<(ObjRef, std::sync::Arc<Object>)> =
        all.into_iter().filter(|(r, o)| is_resource(o) || (listed.contains(r) && !matches!(**o, Object::Null))).collect();

    let mut map: HashMap<ObjRef, ObjRef> = HashMap::new();
    // A fixpoint: each round can make referring objects equal. Depth is small in practice.
    for _round in 0..16 {
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
        let mut merged = 0;
        for (i, (r, o)) in universe.iter().enumerate() {
            if map.contains_key(r) {
                continue;
            }
            let mut h = std::collections::hash_map::DefaultHasher::new();
            hash_obj(o, doc, &map, &mut h);
            let bucket = buckets.entry(h.finish()).or_default();
            let target = bucket.iter().map(|j| &universe[*j]).find(|(_, other)| eq_obj(o, other, doc, &map)).map(|(t, _)| *t);
            match target {
                Some(t) if cand.contains(r) => {
                    map.insert(*r, t);
                    merged += 1;
                }
                _ => bucket.push(i),
            }
        }
        if merged == 0 {
            break;
        }
    }
    if map.is_empty() {
        return 0;
    }
    for r in &sorted {
        if map.contains_key(r) {
            continue;
        }
        let o = doc.get(*r);
        if contains_ref(&o, &map) {
            let new = rewrite(&o, &map);
            doc.set(*r, new);
        }
    }
    for r in map.keys() {
        doc.free(*r);
    }
    map.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_cos::{Dict, Stream};

    fn stream(data: &[u8]) -> Object {
        Object::Stream(Stream { dict: Dict::new(), raw: data.to_vec().into() })
    }

    fn font(doc: &mut Document, file: ObjRef, name: &str) -> ObjRef {
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("Font"));
        d.set(b"BaseFont".to_vec(), Object::name(name));
        d.set(b"FontFile".to_vec(), Object::Ref(file));
        doc.add(Object::Dict(d))
    }

    #[test]
    fn merges_bottom_up_and_keeps_identity_objects() {
        let mut doc = Document::new_empty();
        let f1 = doc.add(stream(b"glyphs"));
        let f2 = doc.add(stream(b"glyphs"));
        let f3 = doc.add(stream(b"other"));
        let a = font(&mut doc, f1, "A");
        let b = font(&mut doc, f2, "A"); // equal to `a` once f2 merges into f1
        let c = font(&mut doc, f3, "A");
        let mut annot = Dict::new();
        annot.set(b"Type".to_vec(), Object::name("Annot"));
        let n1 = doc.add(Object::Dict(annot.clone()));
        let n2 = doc.add(Object::Dict(annot));
        let mut page = Dict::new();
        page.set(b"Fonts".to_vec(), Object::Array(vec![Object::Ref(a), Object::Ref(b), Object::Ref(c)]));
        let p = doc.add(Object::Dict(page));
        let removed = dedupe_resources(&mut doc, &[f1, f2, f3, a, b, c, n1, n2, p], false);
        assert_eq!(removed, 2);
        let page = doc.get(p);
        let fonts = page.as_dict().unwrap().get(b"Fonts").unwrap().as_array().unwrap().clone();
        assert_eq!(fonts, vec![Object::Ref(a), Object::Ref(a), Object::Ref(c)]);
        assert!(!doc.object_numbers().contains(&f2.num) && !doc.object_numbers().contains(&b.num));
        assert!(doc.object_numbers().contains(&n2.num), "annotations are never merged");
    }

    #[test]
    fn untyped_resources_are_found_through_resource_dictionaries() {
        let mut doc = Document::new_empty();
        let mut gs = Dict::new();
        gs.set(b"CA".to_vec(), Object::Real(0.5));
        let g1 = doc.add(Object::Dict(gs.clone()));
        let g2 = doc.add(Object::Dict(gs.clone()));
        let g3 = doc.add(Object::Dict(gs)); // not listed as a resource: left alone
        let page = |doc: &mut Document, g: ObjRef| {
            let mut ext = Dict::new();
            ext.set(b"G0".to_vec(), Object::Ref(g));
            let mut res = Dict::new();
            res.set(b"ExtGState".to_vec(), Object::Dict(ext));
            let mut p = Dict::new();
            p.set(b"Type".to_vec(), Object::name("Page"));
            p.set(b"Resources".to_vec(), Object::Dict(res));
            doc.add(Object::Dict(p))
        };
        let p1 = page(&mut doc, g1);
        let p2 = page(&mut doc, g2);
        assert_eq!(dedupe_resources(&mut doc, &[g1, g2, g3, p1, p2], false), 1);
        assert!(doc.object_numbers().contains(&g3.num));
        let nums = doc.object_numbers();
        assert!(nums.contains(&p1.num) && nums.contains(&p2.num), "pages are never merged");
        assert_eq!(doc.get(p1), doc.get(p2), "both pages now use the one graphics state");
    }

    fn image(doc: &mut Document, colour_space: Object) -> ObjRef {
        let mut d = Dict::new();
        d.set(b"Subtype".to_vec(), Object::name("Image"));
        d.set(b"ColorSpace".to_vec(), colour_space);
        doc.add(Object::Stream(Stream { dict: d, raw: b"pixels".to_vec().into() }))
    }

    #[test]
    fn references_to_plain_values_compare_as_the_values() {
        // ImageMagick writes `/ColorSpace 10 0 R` with `10 0 obj /DeviceRGB`, one per file (#878).
        let mut doc = Document::new_empty();
        let rgb1 = doc.add(Object::name("DeviceRGB"));
        let rgb2 = doc.add(Object::name("DeviceRGB"));
        let grey = doc.add(Object::name("DeviceGray"));
        let a = image(&mut doc, Object::Ref(rgb1));
        let b = image(&mut doc, Object::Ref(rgb2));
        let c = image(&mut doc, Object::name("DeviceRGB")); // written directly: the same image
        let d = image(&mut doc, Object::Ref(grey)); // same pixels, another colour space
        let mut page = Dict::new();
        page.set(b"Images".to_vec(), Object::Array([a, b, c, d].map(Object::Ref).to_vec()));
        let p = doc.add(Object::Dict(page));
        assert_eq!(dedupe_resources(&mut doc, &[rgb1, rgb2, grey, a, b, c, d, p], false), 2);
        let page = doc.get(p);
        let images = page.as_dict().unwrap().get(b"Images").unwrap().as_array().unwrap().clone();
        assert_eq!(images, [a, a, a, d].map(Object::Ref).to_vec());
        let nums = doc.object_numbers();
        assert!(nums.contains(&rgb1.num) && nums.contains(&rgb2.num), "the values themselves are never merged");
    }

    #[test]
    fn existing_objects_are_targets_but_never_changed() {
        let mut doc = Document::new_empty();
        let old = doc.add(stream(b"icc"));
        let new = doc.add(stream(b"icc"));
        let mut holder = Dict::new();
        holder.set(b"CS".to_vec(), Object::Array(vec![Object::name("ICCBased"), Object::Ref(new)]));
        let h = doc.add(Object::Dict(holder));
        assert_eq!(dedupe_resources(&mut doc, &[new, h], true), 1);
        assert!(doc.object_numbers().contains(&old.num));
        let cs = doc.get(h).as_dict().unwrap().get(b"CS").unwrap().clone();
        assert_eq!(cs, Object::Array(vec![Object::name("ICCBased"), Object::Ref(old)]));
    }
}
