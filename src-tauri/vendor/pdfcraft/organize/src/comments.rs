//! Import comments from another PDF (Acrobat: the Comments list's Import Data File, with a PDF
//! chosen): each comment is copied onto the page with the same number, at the same place, with
//! its appearance, pop-up and replies.

use std::collections::{HashMap, HashSet};

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::import::{Copier, register_layers};
use crate::{OrganizeError, walk};

/// The annotation subtypes copied as comments: the markup annotations of ISO 32000-2 Table 171,
/// except redaction marks (`Redact`) and `Projection`.
const COMMENTS: [&[u8]; 16] = [
    b"Text",
    b"FreeText",
    b"Line",
    b"Square",
    b"Circle",
    b"Polygon",
    b"PolyLine",
    b"Highlight",
    b"Underline",
    b"Squiggly",
    b"StrikeOut",
    b"Stamp",
    b"Caret",
    b"Ink",
    b"FileAttachment",
    b"Sound",
];

/// The Locked annotation flag.
const LOCKED: i64 = 128;

/// One comment of the source: its entry in the page's `/Annots` and its dictionary.
struct Incoming {
    entry: Object,
    dict: Dict,
}

/// Copy the comments of `src` onto the pages of `dst`: the comments of page 1 onto page 1, those of
/// page 2 onto page 2, and so on, each at the position it has in `src`. Comments on pages past the
/// last page of `dst` are left out, and so is a reply whose comment is left out. Each copy keeps its
/// appearance, its pop-up, its replies and its layer. Links, form fields and redaction marks are
/// not copied.
///
/// A comment named (`/NM`) like one already on its page replaces that one, which keeps its object
/// so replies to it stay attached. A locked comment, or an annotation of that name that is no
/// comment (a link, a redaction mark, a form widget), is not replaced: the copy is added beside it
/// without the name. Returns the number of comments copied.
pub fn import_comments(dst: &mut Document, src: &Document) -> Result<usize, OrganizeError> {
    let from = walk(src)?;
    let to = walk(dst)?;
    let pages: Vec<(ObjRef, ObjRef)> = from.iter().zip(&to).map(|((s, _), (d, _))| (*s, *d)).collect();
    let mut incoming: Vec<Vec<Incoming>> = pages.iter().map(|(s, _)| page_comments(src, *s)).collect();
    let kept = threads_kept(&incoming);
    for list in &mut incoming {
        list.retain(|c| c.dict.get(b"IRT").is_none_or(|irt| irt.as_ref().is_some_and(|p| kept.contains(&p))));
    }
    let mut copier = Copier::new(src, HashMap::new(), pages.iter().copied().collect());
    // Every comment's object is chosen before any is copied, so a comment that another one reaches
    // through `/IRT` is copied into the object chosen for it by name.
    let mut lists: Vec<Option<PageAnnots>> = Vec::new();
    let mut planned: Vec<(usize, ObjRef, Dict)> = Vec::new();
    for (i, ((_, page), list)) in pages.iter().zip(incoming).enumerate() {
        if list.is_empty() {
            lists.push(None);
            continue;
        }
        let mut annots = PageAnnots::read(dst, *page);
        for Incoming { entry, dict } in list {
            // The same comment listed twice on a page is copied once.
            if entry.as_ref().is_some_and(|r| copier.map.contains_key(&r)) {
                continue;
            }
            let (target, dict) = reserve(dst, src, &mut annots, dict);
            if let Some(r) = entry.as_ref() {
                copier.map.insert(r, target);
            }
            annots.push(target);
            planned.push((i, target, dict));
        }
        lists.push(Some(annots));
    }
    let mut annot_pages = HashMap::new();
    for (i, target, dict) in &planned {
        let copy = copier.copy_value(dst, &Object::Dict(dict.clone()), Some(*target))?;
        dst.set(*target, copy);
        annot_pages.insert(*target, pages[*i].1);
    }
    for (i, target, _) in &planned {
        if let Some(popup) = dst.get(*target).as_dict().and_then(|d| d.reference(b"Popup"))
            && let Some(annots) = lists[*i].as_mut()
        {
            annots.push(popup);
            annot_pages.insert(popup, pages[*i].1);
        }
    }
    // A pop-up that only its `/Parent` ties to a copied comment comes along too.
    for (i, (from, to)) in pages.iter().enumerate() {
        let Some(annots) = lists[i].as_mut() else { continue };
        for (popup, parent) in page_popups(src, *from) {
            if copier.map.contains_key(&popup) || !copier.map.contains_key(&parent) {
                continue;
            }
            let r = dst.add(Object::Null);
            copier.map.insert(popup, r);
            let copy = copier.copy_value(dst, &src.get(popup), Some(r))?;
            dst.set(r, copy);
            annots.push(r);
            annot_pages.insert(r, *to);
        }
    }
    for ((_, page), annots) in pages.iter().zip(lists) {
        if let Some(annots) = annots {
            dst.update_dict(*page, |p| p.set(b"Annots".to_vec(), Object::Array(annots.into_list())))?;
        }
    }
    copier.fix_annotations(dst, &annot_pages)?;
    register_layers(dst, src, &copier.ocgs)?;
    Ok(planned.len())
}

/// Choose the object a comment is copied into: the comment of `annots` with its name, or a new
/// object. Returns it with the dictionary to copy.
fn reserve(dst: &mut Document, src: &Document, annots: &mut PageAnnots, mut dict: Dict) -> (ObjRef, Dict) {
    let nm = dict.get(b"NM").and_then(|n| src.resolve(n).as_string().map(|s| s.to_text()));
    let same = nm.as_deref().and_then(|nm| annots.named(dst, nm));
    // A second incoming comment with the name of one added earlier is added without the name.
    let taken = nm.is_some_and(|nm| !annots.added.insert(nm));
    let target = match same {
        None if taken => {
            dict.remove(b"NM");
            dst.add(Object::Null)
        }
        Some((r, false)) => {
            annots.claimed.insert(r);
            // The replaced comment's pop-up goes with it: the one it names, and any whose `/Parent`
            // is the comment.
            if let Some(p) = dst.get(r).as_dict().and_then(|d| d.reference(b"Popup")) {
                annots.remove(p);
            }
            for p in annots.popups.get(&r).cloned().unwrap_or_default() {
                annots.remove(p);
            }
            r
        }
        Some((_, true)) => {
            dict.remove(b"NM");
            dst.add(Object::Null)
        }
        None => dst.add(Object::Null),
    };
    (target, dict)
}

/// The comments of `incoming` that go: a reply goes only with the comment it answers, and that one
/// only with the comment it answers, up to the first comment of the thread. A thread whose replies
/// answer each other in a loop goes, as all of its comments are there. Each comment is visited once.
fn threads_kept(incoming: &[Vec<Incoming>]) -> HashSet<ObjRef> {
    // For each comment: no parent, a parent comment, or a parent that is no comment of the source.
    let parents: HashMap<ObjRef, Option<Option<ObjRef>>> =
        incoming.iter().flatten().filter_map(|c| Some((c.entry.as_ref()?, c.dict.get(b"IRT").map(Object::as_ref)))).collect();
    let mut known: HashMap<ObjRef, bool> = HashMap::new();
    for &start in parents.keys() {
        let (mut path, mut on_path) = (Vec::new(), HashSet::new());
        let mut at = start;
        let kept = loop {
            if let Some(&k) = known.get(&at) {
                break k;
            }
            if !on_path.insert(at) {
                break true;
            }
            match parents.get(&at) {
                None => break false,
                Some(None) => {
                    path.push(at);
                    break true;
                }
                Some(Some(None)) => {
                    path.push(at);
                    break false;
                }
                Some(Some(Some(parent))) => {
                    path.push(at);
                    at = *parent;
                }
            }
        };
        for r in path {
            known.insert(r, kept);
        }
    }
    known.into_iter().filter_map(|(r, k)| k.then_some(r)).collect()
}

/// The comments in `page`'s `/Annots`, in order.
fn page_comments(src: &Document, page: ObjRef) -> Vec<Incoming> {
    let list = src.get(page).as_dict().and_then(|p| p.get(b"Annots").cloned()).map(|a| src.resolve(&a)).and_then(|a| a.as_array().cloned());
    list.unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let dict = src.resolve(&entry).as_dict().cloned()?;
            let subtype = dict.get(b"Subtype").map(|s| src.resolve(s)).and_then(|s| s.as_name().map(<[u8]>::to_vec))?;
            COMMENTS.contains(&subtype.as_slice()).then_some(Incoming { entry, dict })
        })
        .collect()
}

/// The pop-ups in `page`'s `/Annots` with the comment each belongs to: (pop-up, its `/Parent`).
fn page_popups(src: &Document, page: ObjRef) -> Vec<(ObjRef, ObjRef)> {
    let list = src.get(page).as_dict().and_then(|p| p.get(b"Annots").cloned()).map(|a| src.resolve(&a)).and_then(|a| a.as_array().cloned());
    list.unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            let r = entry.as_ref()?;
            let d = src.get(r).as_dict().cloned()?;
            (d.name(b"Subtype") == Some(b"Popup".as_slice())).then(|| d.reference(b"Parent").map(|p| (r, p)))?
        })
        .collect()
}

/// The annotation in `annots` named `nm` (compared as text, whatever the encoding), and whether it
/// must not be replaced: a locked comment, or an annotation that is no comment, such as a link, a
/// redaction mark or a form widget.
/// A page's `/Annots` while comments are added to it, indexed so that placing each comment does not
/// scan the whole list.
struct PageAnnots {
    list: Vec<Object>,
    members: HashSet<ObjRef>,
    /// The annotations on the page before the import, by `/NM`, in list order.
    names: HashMap<String, Vec<ObjRef>>,
    /// The pop-ups on the page before the import, by `/Parent`.
    popups: HashMap<ObjRef, Vec<ObjRef>>,
    removed: HashSet<ObjRef>,
    /// The annotations that an incoming comment has already replaced.
    claimed: HashSet<ObjRef>,
    /// The names of the incoming comments placed so far.
    added: HashSet<String>,
}

impl PageAnnots {
    /// The `/Annots` of `page`, with inline annotation dictionaries made indirect objects.
    fn read(dst: &mut Document, page: ObjRef) -> Self {
        let list: Vec<Object> = dst
            .get(page)
            .as_dict()
            .and_then(|p| p.get(b"Annots").cloned())
            .map(|a| dst.resolve(&a))
            .and_then(|a| a.as_array().cloned())
            .unwrap_or_default();
        let list: Vec<Object> = list
            .into_iter()
            .map(|a| match a {
                Object::Dict(_) => Object::Ref(dst.add(a)),
                a => a,
            })
            .collect();
        let (mut members, mut names, mut popups) = (HashSet::new(), HashMap::<String, Vec<ObjRef>>::new(), HashMap::<ObjRef, Vec<ObjRef>>::new());
        for r in list.iter().filter_map(Object::as_ref) {
            members.insert(r);
            let Some(d) = dst.get(r).as_dict().cloned() else { continue };
            if let Some(name) = d.get(b"NM").and_then(|n| dst.resolve(n).as_string().map(|s| s.to_text())) {
                names.entry(name).or_default().push(r);
            }
            if d.name(b"Subtype") == Some(b"Popup".as_slice())
                && let Some(parent) = d.reference(b"Parent")
            {
                popups.entry(parent).or_default().push(r);
            }
        }
        PageAnnots { list, members, names, popups, removed: HashSet::new(), claimed: HashSet::new(), added: HashSet::new() }
    }

    /// Add `r` at the end, unless the page already lists it.
    fn push(&mut self, r: ObjRef) {
        if self.removed.remove(&r) {
            self.list.retain(|a| a.as_ref() != Some(r));
        }
        if self.members.insert(r) {
            self.list.push(Object::Ref(r));
        }
    }

    /// Take every entry for `r` off the page.
    fn remove(&mut self, r: ObjRef) {
        if self.members.remove(&r) {
            self.removed.insert(r);
        }
    }

    /// The first annotation on the page named `nm`, and whether it must be kept: it is locked, or
    /// it is no comment.
    fn named(&self, dst: &Document, nm: &str) -> Option<(ObjRef, bool)> {
        let mut listed = self.names.get(nm)?.iter().filter(|r| self.members.contains(r));
        let first = *listed.clone().next()?;
        // A second incoming comment with the name of one already replaced is added beside it.
        let Some(&r) = listed.find(|r| !self.claimed.contains(r)) else { return Some((first, true)) };
        let d = dst.get(r).as_dict().cloned()?;
        let flags = d.get(b"F").and_then(|f| dst.resolve(f).as_int()).unwrap_or(0);
        let subtype = d.get(b"Subtype").map(|s| dst.resolve(s));
        let comment = subtype.as_deref().and_then(Object::as_name).is_some_and(|s| COMMENTS.contains(&s));
        Some((r, flags & LOCKED != 0 || !comment || d.contains(b"FT")))
    }

    /// The new `/Annots`.
    fn into_list(self) -> Vec<Object> {
        let removed = self.removed;
        self.list.into_iter().filter(|a| a.as_ref().is_none_or(|r| !removed.contains(&r))).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pdfcraft_cos::{SaveOptions, write_full};

    use super::*;

    fn build(bodies: &[&str]) -> Document {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, b) in bodies.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{b}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
        Document::open(Arc::new(out)).expect("opens")
    }

    /// The reviewer's copy: three pages of comments, a link and a pop-up among them.
    fn reviewed() -> Document {
        build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [6 0 R 13 0 R 7 0 R 8 0 R 9 0 R 15 0 R 16 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [10 0 R 11 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [12 0 R] >>",
            "<< /Type /Annot /Subtype /Text /NM (note-1) /Contents (from the reviewer) /Rect [10 10 30 30] /Popup 13 0 R /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /IRT 6 0 R /Contents (a reply) /Rect [10 10 30 30] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Stamp /Name /Approved /Rect [50 50 150 100] /AP << /N 14 0 R >> /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /IRT 12 0 R /Contents (about page 3) /Rect [10 40 30 60] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /URI /URI (https://example.com/) >> /P 4 0 R >>",
            "<< /Type /Annot /Subtype /Highlight /Rect [0 0 50 20] /QuadPoints [0 20 50 20 0 0 50 0] /Contents (marked) /P 4 0 R >>",
            "<< /Type /Annot /Subtype /Square /Rect [0 0 100 100] /P 5 0 R >>",
            "<< /Type /Annot /Subtype /Popup /Rect [40 40 200 120] /Parent 6 0 R /P 3 0 R >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 100 50] /Length 13 >>\nstream\n0 0 100 50 re\nendstream",
            "<< /Type /Annot /Subtype /Text /NM (locked) /Contents (the reviewer's) /Rect [80 10 100 30] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Redact /Rect [0 200 20 220] /F 2 /P 3 0 R >>",
        ])
    }

    /// The document receiving the comments: two pages, three comments of its own on page 1.
    fn original() -> Document {
        build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [5 0 R 6 0 R 7 0 R 8 0 R] >>",
            "<< /Type /Page /Parent 2 0 R >>",
            "<< /Type /Annot /Subtype /Text /NM (note-1) /Contents (older text) /Rect [10 10 30 30] /Popup 8 0 R /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /NM (locked) /F 128 /Contents (mine) /Rect [80 10 100 30] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Square /NM (keep) /Rect [100 100 200 200] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Popup /Rect [40 40 200 120] /Parent 5 0 R /P 3 0 R >>",
        ])
    }

    fn page_annots(doc: &Document, page: usize) -> Vec<(ObjRef, Dict)> {
        let (p, _) = walk(doc).unwrap()[page].clone();
        let list =
            doc.get(p).as_dict().and_then(|d| d.get(b"Annots").cloned()).map(|a| doc.resolve(&a).as_array().cloned().unwrap()).unwrap_or_default();
        list.iter().map(|a| (a.as_ref().unwrap(), doc.resolve(a).as_dict().cloned().unwrap())).collect()
    }

    fn text(d: &Dict, key: &[u8]) -> Option<String> {
        d.get(key).and_then(|v| v.as_string()).map(|s| s.to_text())
    }

    fn by_contents<'a>(list: &'a [(ObjRef, Dict)], contents: &str) -> &'a (ObjRef, Dict) {
        list.iter().find(|(_, d)| text(d, b"Contents").as_deref() == Some(contents)).unwrap_or_else(|| panic!("no comment {contents:?}"))
    }

    #[test]
    fn comments_of_another_pdf_land_on_the_pages_with_the_same_numbers() {
        let mut doc = original();
        let src = reviewed();
        assert_eq!(import_comments(&mut doc, &src).unwrap(), 5, "four comments on page 1 and the highlight on page 2");
        let pages: Vec<ObjRef> = walk(&doc).unwrap().into_iter().map(|(p, _)| p).collect();
        let one = page_annots(&doc, 0);
        for (_, d) in &one {
            assert_eq!(d.reference(b"P"), Some(pages[0]), "each annotation names the page it is on");
        }

        // The note named like one already there replaces it in place, with its new pop-up.
        let (note, d) = by_contents(&one, "from the reviewer");
        assert_eq!(*note, ObjRef::new(5, 0), "the replaced comment keeps its object");
        let popup = d.reference(b"Popup").unwrap();
        assert_ne!(popup, ObjRef::new(8, 0));
        assert_eq!(doc.get(popup).as_dict().unwrap().reference(b"Parent"), Some(*note));
        let popups: Vec<ObjRef> = one.iter().filter(|(_, d)| d.name(b"Subtype") == Some(b"Popup".as_slice())).map(|(r, _)| *r).collect();
        assert_eq!(popups, [popup], "the replaced note's pop-up goes, the copy's is listed once");
        assert_eq!(by_contents(&one, "a reply").1.reference(b"IRT"), Some(*note), "the reply answers the note");

        // The locked comment of that name stays; the copy is added beside it, unnamed.
        assert_eq!(text(&by_contents(&one, "mine").1, b"NM").as_deref(), Some("locked"));
        assert_eq!(text(&by_contents(&one, "the reviewer's").1, b"NM"), None);
        assert!(one.iter().any(|(_, d)| text(d, b"NM").as_deref() == Some("keep")), "the document's own comments stay");

        // The stamp keeps its appearance; the redaction mark is not copied.
        let (_, stamp) = one.iter().find(|(_, d)| d.name(b"Subtype") == Some(b"Stamp".as_slice())).unwrap();
        let ap = doc.resolve(stamp.get(b"AP").unwrap()).as_dict().cloned().unwrap();
        assert!(matches!(&*doc.resolve(ap.get(b"N").unwrap()), Object::Stream(_)), "the appearance stream is copied");
        assert!(one.iter().all(|(_, d)| d.name(b"Subtype") != Some(b"Redact".as_slice())));

        // Page 3 is past the document's last page: its square and the reply to it are left out.
        assert!(one.iter().all(|(_, d)| text(d, b"Contents").as_deref() != Some("about page 3")));
        assert_eq!(one.len(), 7, "three of the document's own, three new copies and the note's new pop-up");

        // Page 2 takes the highlight, not the link.
        let two = page_annots(&doc, 1);
        assert_eq!(two.len(), 1);
        assert_eq!(two[0].1.name(b"Subtype"), Some(b"Highlight".as_slice()));
        assert_eq!(two[0].1.reference(b"P"), Some(pages[1]));

        let saved = Document::open(Arc::new(write_full(&doc, &SaveOptions::default()).unwrap())).unwrap();
        assert_eq!((page_annots(&saved, 0).len(), page_annots(&saved, 1).len()), (7, 1), "a saved copy reads back the same");
    }

    #[test]
    fn a_reply_on_an_earlier_page_reaches_its_comment_by_name() {
        // Page 1 holds a reply to a note on page 2, named like the document's own note there.
        let src = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [6 0 R 7 0 R] >>",
            "<< /Type /Annot /Subtype /Text /IRT 6 0 R /Contents (a reply) /Rect [10 10 30 30] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /NM (note-2) /Contents (from the reviewer) /Rect [10 10 30 30] /P 4 0 R >>",
            // A pop-up that only its /Parent ties to the note.
            "<< /Type /Annot /Subtype /Popup /Rect [50 50 210 130] /Open true /Parent 6 0 R /P 4 0 R >>",
        ]);
        let mut doc = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R >>",
            "<< /Type /Page /Parent 2 0 R /Annots [5 0 R 6 0 R] >>",
            // The same name in UTF-16BE, and a pop-up the note does not name.
            "<< /Type /Annot /Subtype /Text /NM <FEFF006E006F00740065002D0032> /Contents (older text) /Rect [10 10 30 30] /P 4 0 R >>",
            "<< /Type /Annot /Subtype /Popup /Rect [40 40 200 120] /Parent 5 0 R /P 4 0 R >>",
        ]);
        assert_eq!(import_comments(&mut doc, &src).unwrap(), 2);
        let two = page_annots(&doc, 1);
        assert_eq!(two.len(), 2, "the note replaces the document's own, whose pop-up goes, and its own pop-up comes");
        assert_eq!((two[0].0, text(&two[0].1, b"Contents").as_deref()), (ObjRef::new(5, 0), Some("from the reviewer")));
        assert_ne!(two[1].0, ObjRef::new(6, 0));
        assert_eq!(two[1].1.reference(b"Parent"), Some(ObjRef::new(5, 0)));
        assert_eq!(two[1].1.get(b"Open"), Some(&Object::Bool(true)));
        assert_eq!(two[1].1.reference(b"P"), Some(walk(&doc).unwrap()[1].0));
        let one = page_annots(&doc, 0);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].1.reference(b"IRT"), Some(ObjRef::new(5, 0)), "the reply answers the replaced note");
    }

    #[test]
    fn a_comment_keeps_its_layer_and_the_layer_its_visibility() {
        let src = build(&[
            "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>",
            "<< >>",
            "<< /Type /Annot /Subtype /Square /Rect [0 0 50 50] /OC 6 0 R /P 3 0 R >>",
            "<< /Type /OCG /Name (Review) >>",
        ]);
        let mut doc = original();
        import_comments(&mut doc, &src).unwrap();
        let (_, square) =
            page_annots(&doc, 0).into_iter().find(|(_, d)| d.name(b"Subtype") == Some(b"Square".as_slice()) && d.contains(b"OC")).unwrap();
        let layer = square.reference(b"OC").unwrap();
        let catalog = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
        let props = doc.resolve(catalog.get(b"OCProperties").unwrap()).as_dict().cloned().unwrap();
        assert_eq!(props.get(b"OCGs"), Some(&Object::Array(vec![Object::Ref(layer)])));
        let config = props.get(b"D").and_then(Object::as_dict).cloned().unwrap();
        assert_eq!(config.get(b"OFF"), Some(&Object::Array(vec![Object::Ref(layer)])), "the layer stays hidden");
    }

    #[test]
    fn a_layer_hidden_by_the_base_state_stays_hidden() {
        let src = build(&[
            "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [6 0 R] /D << /BaseState /OFF >> >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>",
            "<< >>",
            "<< /Type /Annot /Subtype /Square /Rect [0 0 50 50] /OC 6 0 R /P 3 0 R >>",
            "<< /Type /OCG /Name (Review) >>",
        ]);
        let mut doc = original();
        import_comments(&mut doc, &src).unwrap();
        let (_, square) =
            page_annots(&doc, 0).into_iter().find(|(_, d)| d.name(b"Subtype") == Some(b"Square".as_slice()) && d.contains(b"OC")).unwrap();
        let layer = square.reference(b"OC").unwrap();
        let catalog = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
        let props = doc.resolve(catalog.get(b"OCProperties").unwrap()).as_dict().cloned().unwrap();
        let config = props.get(b"D").and_then(Object::as_dict).cloned().unwrap();
        assert_eq!(config.get(b"OFF"), Some(&Object::Array(vec![Object::Ref(layer)])), "the layer stays hidden");
        assert_eq!(config.get(b"ON"), Some(&Object::Array(vec![])));
    }

    #[test]
    fn a_long_thread_goes_with_its_first_comment_and_a_loop_of_replies_stays() {
        // Page 1: replies 6, 7, … each answering the next, the last answering the square on page 3,
        // which the document has no page for. Then two replies that answer each other.
        let n = 20_000;
        let first = 6;
        let mut bodies: Vec<String> =
            vec!["<< /Type /Catalog /Pages 2 0 R >>".into(), "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >>".into()];
        let list: String = (first..first + n + 2).map(|o| format!("{o} 0 R ")).collect();
        bodies.push(format!("<< /Type /Page /Parent 2 0 R /Annots [{list}] >>"));
        bodies.push("<< /Type /Page /Parent 2 0 R >>".into());
        let square = first + n + 2;
        bodies.push(format!("<< /Type /Page /Parent 2 0 R /Annots [{square} 0 R] >>"));
        for o in first..first + n {
            let parent = if o + 1 == first + n { square } else { o + 1 };
            bodies.push(format!("<< /Type /Annot /Subtype /Text /IRT {parent} 0 R /Rect [0 0 10 10] /P 3 0 R >>"));
        }
        let (a, b) = (first + n, first + n + 1);
        bodies.push(format!("<< /Type /Annot /Subtype /Text /IRT {b} 0 R /Contents (one) /Rect [0 0 10 10] /P 3 0 R >>"));
        bodies.push(format!("<< /Type /Annot /Subtype /Text /IRT {a} 0 R /Contents (two) /Rect [0 0 10 10] /P 3 0 R >>"));
        bodies.push("<< /Type /Annot /Subtype /Square /Rect [0 0 100 100] /P 5 0 R >>".into());
        let src = build(&bodies.iter().map(String::as_str).collect::<Vec<_>>());
        let mut doc = original();
        let started = std::time::Instant::now();
        assert_eq!(import_comments(&mut doc, &src).unwrap(), 2, "only the two replies that answer each other");
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
        let one = page_annots(&doc, 0);
        let (one_ref, one_dict) = by_contents(&one, "one");
        let (two_ref, two_dict) = by_contents(&one, "two");
        assert_eq!((one_dict.reference(b"IRT"), two_dict.reference(b"IRT")), (Some(*two_ref), Some(*one_ref)));
    }

    #[test]
    fn only_a_comment_is_replaced_by_name() {
        let src = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R 6 0 R] >>",
            "<< /Type /Annot /Subtype /Text /NM (link-1) /Contents (from the reviewer) /Rect [10 10 30 30] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Square /NM (movie-1) /Contents (a square) /Rect [40 40 60 60] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /NM (note-3) /Contents (newer text) /Rect [70 70 90 90] /P 3 0 R >>",
        ]);
        // A link and a screen annotation named like two of the comments, and a comment whose
        // subtype is an indirect object.
        let mut doc = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R 6 0 R] >>",
            "<< /Type /Annot /Subtype /Link /NM (link-1) /Rect [0 0 10 10] /A << /S /URI /URI (https://example.com/) >> /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Screen /NM (movie-1) /Rect [100 100 200 200] /P 3 0 R >>",
            "<< /Type /Annot /Subtype 7 0 R /NM (note-3) /Contents (older text) /Rect [70 70 90 90] /P 3 0 R >>",
            "/Text",
        ]);
        assert_eq!(import_comments(&mut doc, &src).unwrap(), 3);
        let one = page_annots(&doc, 0);
        assert_eq!(one.len(), 5, "the link and the screen annotation stay, two comments are added and one replaced");
        let (note, d) = by_contents(&one, "newer text");
        assert_eq!((*note, text(d, b"NM").as_deref()), (ObjRef::new(6, 0), Some("note-3")), "the comment is replaced in place");
        assert_eq!((one[0].0, one[0].1.name(b"Subtype")), (ObjRef::new(4, 0), Some(b"Link".as_slice())));
        assert_eq!(text(&one[0].1, b"NM").as_deref(), Some("link-1"));
        assert_eq!((one[1].0, one[1].1.name(b"Subtype")), (ObjRef::new(5, 0), Some(b"Screen".as_slice())));
        assert_eq!(text(&one[1].1, b"NM").as_deref(), Some("movie-1"));
        assert_eq!(text(&by_contents(&one, "from the reviewer").1, b"NM"), None, "the copy is added without the name");
        assert_eq!(text(&by_contents(&one, "a square").1, b"NM"), None);
    }

    #[test]
    fn two_comments_with_one_name_replace_one_comment_and_add_the_other() {
        let src = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>",
            "<< /Type /Annot /Subtype /Text /NM (twin) /Contents (first) /Rect [10 10 30 30] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /NM (twin) /Contents (second) /Rect [40 40 60 60] /P 3 0 R >>",
        ]);
        let mut doc = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>",
            "<< /Type /Annot /Subtype /Text /NM (twin) /Contents (older text) /Rect [10 10 30 30] /P 3 0 R >>",
        ]);
        assert_eq!(import_comments(&mut doc, &src).unwrap(), 2);
        let one = page_annots(&doc, 0);
        assert_eq!(one.len(), 2, "one comment replaced and one added");
        let (first, d) = by_contents(&one, "first");
        assert_eq!((*first, text(d, b"NM").as_deref()), (ObjRef::new(4, 0), Some("twin")));
        assert_eq!(text(&by_contents(&one, "second").1, b"NM"), None, "the second copy is added without the name");
    }

    #[test]
    fn two_new_comments_with_one_name_keep_it_once() {
        let src = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>",
            "<< /Type /Annot /Subtype /Text /NM (twin) /Contents (first) /Rect [10 10 30 30] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Text /NM (twin) /Contents (second) /Rect [40 40 60 60] /P 3 0 R >>",
        ]);
        let mut doc = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R >>",
        ]);
        assert_eq!(import_comments(&mut doc, &src).unwrap(), 2);
        let one = page_annots(&doc, 0);
        assert_eq!(one.len(), 2);
        assert_eq!(text(&by_contents(&one, "first").1, b"NM").as_deref(), Some("twin"));
        assert_eq!(text(&by_contents(&one, "second").1, b"NM"), None, "the second copy is added without the name");
    }

    #[test]
    fn an_inline_comment_is_replaced_by_name() {
        let src = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>",
            "<< /Type /Annot /Subtype /Text /NM (inline) /Contents (newer text) /Rect [10 10 30 30] /P 3 0 R >>",
        ]);
        let mut doc = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [<< /Type /Annot /Subtype /Text /NM (inline) /Contents (older text) /Rect [10 10 30 30] >>] >>",
        ]);
        assert_eq!(import_comments(&mut doc, &src).unwrap(), 1);
        let one = page_annots(&doc, 0);
        assert_eq!(one.len(), 1, "the inline comment is replaced, not kept beside the copy");
        assert_eq!(text(&by_contents(&one, "newer text").1, b"NM").as_deref(), Some("inline"));
    }

    #[test]
    fn a_pdf_without_comments_copies_none() {
        let mut doc = original();
        let before = page_annots(&doc, 0).len();
        let src = build(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>",
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /P 3 0 R >>",
        ]);
        assert_eq!(import_comments(&mut doc, &src).unwrap(), 0);
        assert_eq!(page_annots(&doc, 0).len(), before);
    }
}
