use std::sync::Arc;

use pdfcraft_cos::{Document, Object, SaveOptions, write_full, write_incremental};

use super::*;

#[test]
fn page_rotation_resolves_inheritance_overrides_and_missing_pages() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    for page in 0..3 {
        assert_eq!(page_rotation(&doc, page).unwrap(), 90);
    }
    rotate_pages(&mut doc, &[1], -180).unwrap();
    assert_eq!(page_rotation(&doc, 1).unwrap(), 270);
    assert_eq!(page_rotation(&doc, 0).unwrap(), 90);
    assert_eq!(page_rotation(&doc, 3), Err(OrganizeError::NoSuchPage(3)));
}

#[test]
fn indirect_page_rotation_is_read_and_updated_from_its_effective_value() {
    let large = i64::MAX - i64::MAX % 90;
    for (current, degrees, expected) in [(270, 90, 0), (-90, 90, 0), (large, 90, 90), (90, -180, 270)] {
        for level in 0..3 {
            let mut doc = open(fixture());
            let rotation = doc.add(Object::Int(current));
            let page = pages(&doc).unwrap()[0].obj;
            let target = match level {
                0 => page,
                1 => doc.get(page).as_dict().unwrap().reference(b"Parent").unwrap(),
                _ => pages_root(&doc).unwrap(),
            };
            doc.update_dict(target, |d| d.set(b"Rotate".to_vec(), Object::Ref(rotation))).unwrap();
            let bytes = write_full(&doc, &SaveOptions::default()).unwrap();
            let mut doc = open(bytes);
            let rotation = walk(&doc).unwrap()[0].1.reference(b"Rotate").unwrap();
            let initial = page_rotation(&doc, 0).unwrap();
            rotate_pages(&mut doc, &[0], degrees).unwrap();
            assert_eq!(*doc.get(rotation), Object::Int(current), "shared rotation object unchanged");
            for saved in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
                let reopened = open(saved.clone());
                let pdf = hayro_syntax::Pdf::new(saved).unwrap();
                let dimensions = if expected % 180 == 0 { (300.0, 400.0) } else { (400.0, 300.0) };
                assert_eq!(pdf.pages()[0].render_dimensions(), dimensions, "written PDF has the correct orientation");
                assert_eq!(page_rotation(&reopened, 0).unwrap(), expected);
                assert_eq!(page_rotation(&reopened, 1).unwrap(), if level == 0 { 90 } else { current.rem_euclid(360) }, "sibling unchanged");
                assert_eq!(page_rotation(&reopened, 2).unwrap(), if level == 2 { current.rem_euclid(360) } else { 90 }, "other subtree unchanged");
            }
            assert_eq!(initial, current.rem_euclid(360), "effective indirect rotation at level {level}");
        }
    }
}

#[test]
fn malformed_indirect_rotations_keep_the_zero_fallback() {
    for value in [Object::Null, Object::name("invalid"), Object::Ref(ObjRef::new(99_999, 0))] {
        let mut doc = open(fixture());
        let rotation = doc.add(value);
        let page = pages(&doc).unwrap()[0].obj;
        doc.update_dict(page, |d| d.set(b"Rotate".to_vec(), Object::Ref(rotation))).unwrap();
        assert_eq!(page_rotation(&doc, 0).unwrap(), 0);
        rotate_pages(&mut doc, &[0], 90).unwrap();
        assert_eq!(page_rotation(&doc, 0).unwrap(), 90);
    }
}

#[test]
fn rotate_rejects_angles_that_are_not_multiples_of_90() {
    // The page tree only turns in quarter steps, so anything else used to be silently truncated
    // by the integer division. It has to be an error, and the document must stay as it was.
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    assert_eq!(rotate_pages(&mut doc, &[0], 45), Err(OrganizeError::Invalid("rotation must be a multiple of 90 degrees".into())));
    assert_eq!(rotate_pages(&mut doc, &[0], 91), Err(OrganizeError::Invalid("rotation must be a multiple of 90 degrees".into())));
    assert_eq!(page_rotation(&doc, 0).unwrap(), 90);
    rotate_pages(&mut doc, &[0], -90).unwrap();
    assert_eq!(page_rotation(&doc, 0).unwrap(), 0);
}

#[test]
fn rotate_handles_large_quarter_turns() {
    let large = i64::MAX - i64::MAX % 90;
    for (current, degrees, expected) in [(90, large, 90), (-90, -large, 270), (large, 90, 90), (-large, -90, 270), (180, large - 90, 90)] {
        for inherited in [false, true] {
            let mut doc = open(fixture());
            let target = if inherited { pages_root(&doc).unwrap() } else { pages(&doc).unwrap()[0].obj };
            doc.update_dict(target, |d| d.set(b"Rotate".to_vec(), Object::Int(current))).unwrap();
            rotate_pages(&mut doc, &[0], degrees).unwrap();
            assert_eq!(page_rotation(&doc, 0).unwrap(), expected);
        }
    }
}

/// A 3-page document with a nested page tree. MediaBox and Rotate are inherited from the root,
/// Resources from an intermediate node; each page's content says which page it is.
fn fixture() -> Vec<u8> {
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),                                                  // 1
        b"<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 3 /MediaBox [0 0 300 400] /Rotate 90 >>".to_vec(), // 2
        b"<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 /Resources << /Font << /F1 10 0 R >> >> >>".to_vec(), // 3
        b"<< /Type /Page /Parent 3 0 R /Contents 7 0 R >>".to_vec(),                                    // 4
        b"<< /Type /Page /Parent 3 0 R /Contents 8 0 R >>".to_vec(),                                    // 5
        b"<< /Type /Page /Parent 2 0 R /Contents 9 0 R /Resources << /Font << /F1 10 0 R >> >> /MediaBox [0 0 500 500] >>".to_vec(), // 6
        stream(b"BT /F1 12 Tf 20 20 Td (Page 1) Tj ET"),                                                // 7
        stream(b"BT /F1 12 Tf 20 20 Td (Page 2) Tj ET"),                                                // 8
        stream(b"BT /F1 12 Tf 20 20 Td (Page 3) Tj ET"),                                                // 9
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),                             // 10
        b"<< /Title (Original) /Producer (fixture) >>".to_vec(),                                        // 11
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 11 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn stream(body: &[u8]) -> Vec<u8> {
    let mut v = format!("<< /Length {} >>\nstream\n", body.len()).into_bytes();
    v.extend_from_slice(body);
    v.extend_from_slice(b"\nendstream");
    v
}

fn open(bytes: Vec<u8>) -> Document {
    Document::open(Arc::new(bytes)).expect("opens")
}

/// The text label of each page, via its content stream, in order.
fn labels(doc: &Document) -> Vec<String> {
    pages(doc)
        .unwrap()
        .iter()
        .map(|p| {
            let page = doc.get(p.obj);
            let c = page.as_dict().and_then(|d| d.get(b"Contents").cloned());
            match c.map(|c| doc.resolve(&c)).as_deref() {
                Some(Object::Stream(s)) => {
                    let data = s.decoded().unwrap();
                    let t = String::from_utf8_lossy(&data);
                    t.split('(').nth(1).and_then(|x| x.split(')').next()).unwrap_or("").to_string()
                }
                _ => "blank".into(),
            }
        })
        .collect()
}

/// Save incrementally, verify the prefix is untouched, and reopen — both with our reader and with
/// an independent parser (hayro-syntax) that must agree on the page count.
fn save_and_reopen(doc: &Document) -> Document {
    let original = doc.bytes().clone();
    let bytes = write_incremental(doc, &SaveOptions::default()).expect("saves");
    assert_eq!(&bytes[..original.len()], &original[..], "incremental save must not modify existing bytes");
    let reopened = open(bytes.clone());
    let independent = hayro_syntax::Pdf::new(bytes).expect("hayro opens our output");
    assert_eq!(independent.pages().len(), page_count(&reopened).unwrap(), "readers agree on page count");
    reopened
}

#[test]
fn walks_nested_tree_in_order() {
    let doc = open(fixture());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 3"]);
}

#[test]
fn rotate_uses_inherited_rotation_and_round_trips() {
    let mut doc = open(fixture());
    rotate_pages(&mut doc, &[0, 2], 90).unwrap();
    rotate_pages(&mut doc, &[2], -270).unwrap(); // page 3: inherited 90 + 90 - 270 = -90 ≡ 270
    let doc = save_and_reopen(&doc);
    let rot = |i: usize| {
        let p = pages(&doc).unwrap()[i].obj;
        doc.get(p).as_dict().and_then(|d| d.int(b"Rotate"))
    };
    assert_eq!(rot(0), Some(180), "inherited 90 + 90");
    assert_eq!(rot(1), None, "untouched page keeps inheriting");
    assert_eq!(rot(2), Some(270), "normalised into 0..360");
}

#[test]
fn delete_keeps_inherited_attributes_on_survivors() {
    let mut doc = open(fixture());
    delete_pages(&mut doc, &[0]).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 2", "Page 3"]);
    // Page 2 inherited Resources (from the intermediate node) and MediaBox/Rotate (from the root);
    // the tree was flattened, so they must now be on the page itself.
    let p = doc.get(pages(&doc).unwrap()[0].obj);
    let d = p.as_dict().unwrap();
    assert!(d.contains(b"Resources") && d.contains(b"MediaBox"));
    assert_eq!(d.int(b"Rotate"), Some(90));
    let root = doc.get(doc.get(doc.root().unwrap()).as_dict().unwrap().reference(b"Pages").unwrap());
    assert_eq!(root.as_dict().unwrap().int(b"Count"), Some(2));
}

#[test]
fn deleting_a_page_prunes_deep_named_destination_trees() {
    let mut doc = doc_a();
    let original_pages = pages(&doc).unwrap();
    let mut leaf = Dict::new();
    leaf.set(
        b"Names".to_vec(),
        Object::Array(vec![
            Object::String(PdfString::text("gone")),
            Object::Array(vec![Object::Ref(original_pages[0].obj), Object::name("Fit")]),
            Object::String(PdfString::text("kept")),
            Object::Array(vec![Object::Ref(original_pages[2].obj), Object::name("Fit")]),
        ]),
    );
    let leaf = doc.add(leaf);
    let mut tree = leaf;
    for _ in 0..16_384 {
        let mut node = Dict::new();
        node.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(tree)]));
        tree = doc.add(node);
    }
    let mut names = Dict::new();
    names.set(b"Dests".to_vec(), Object::Ref(tree));
    doc.update_dict(doc.root().unwrap(), |d| d.set(b"Names".to_vec(), Object::Dict(names))).unwrap();

    // Every node is a shallow indirect object: the page operation must not turn the
    // document's graph depth into call-stack depth.
    delete_pages(&mut doc, &[0]).unwrap();
    for bytes in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
        let reopened = open(bytes);
        assert_eq!(labels(&reopened), ["A2", "A3"]);
        let catalog = reopened.get(reopened.root().unwrap());
        let names = reopened.resolve(catalog.as_dict().unwrap().get(b"Names").unwrap());
        let mut node = reopened.resolve(names.as_dict().unwrap().get(b"Dests").unwrap());
        for _ in 0..16_384 {
            let child = node.as_dict().unwrap().get(b"Kids").unwrap().as_array().unwrap()[0].clone();
            node = reopened.resolve(&child);
        }
        let pairs = node.as_dict().unwrap().get(b"Names").and_then(Object::as_array).cloned().unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].as_string().unwrap().to_text(), "kept");
        assert_eq!(pairs[1].as_array().unwrap()[0].as_ref(), Some(pages(&reopened).unwrap()[1].obj));
    }
}

#[test]
fn destination_pruning_visits_shared_and_cyclic_nodes_once() {
    for inline_names in [false, true] {
        for inline_tree in [false, true] {
            let mut doc = doc_a();
            let gone = pages(&doc).unwrap()[0].obj;
            let mut leaf = Dict::new();
            leaf.set(
                b"Names".to_vec(),
                Object::Array(vec![Object::String(PdfString::text("gone")), Object::Array(vec![Object::Ref(gone), Object::name("Fit")])]),
            );
            leaf.set(b"Unknown".to_vec(), Object::Int(42));
            let leaf = doc.add(leaf);
            let cycle = doc.add(Object::Null);
            let mut node = Dict::new();
            node.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(cycle), Object::Ref(leaf), Object::Ref(leaf)]));
            doc.set(cycle, Object::Dict(node.clone()));
            let mut names = Dict::new();
            names.set(b"Dests".to_vec(), if inline_tree { Object::Dict(node) } else { Object::Ref(cycle) });
            let names = if inline_names { Object::Dict(names) } else { Object::Ref(doc.add(names)) };
            doc.update_dict(doc.root().unwrap(), |d| d.set(b"Names".to_vec(), names)).unwrap();

            assert_eq!(drop_named_destinations(&mut doc, &[gone]).unwrap(), [b"gone".to_vec()]);
            let leaf = doc.get(leaf);
            assert_eq!(leaf.as_dict().unwrap().get(b"Names").and_then(Object::as_array).unwrap().len(), 0);
            assert_eq!(leaf.as_dict().unwrap().int(b"Unknown"), Some(42));
        }
    }
}

#[test]
fn deleting_a_page_keeps_sixty_thousand_named_destinations_usable() {
    let mut doc = doc_a();
    let original_pages = pages(&doc).unwrap();
    let pairs: Vec<Object> = (0..60_000)
        .flat_map(|i| {
            let page = if i % 2 == 0 { original_pages[0].obj } else { original_pages[2].obj };
            [Object::String(PdfString::text(&format!("dest-{i:05}"))), Object::Array(vec![Object::Ref(page), Object::name("Fit")])]
        })
        .collect();
    let mut leaf = Dict::new();
    leaf.set(b"Names".to_vec(), Object::Array(pairs));
    leaf.set(b"Unknown".to_vec(), Object::Int(42));
    let leaf = doc.add(leaf);
    let mut names = Dict::new();
    names.set(b"Dests".to_vec(), Object::Ref(leaf));
    doc.update_dict(doc.root().unwrap(), |d| d.set(b"Names".to_vec(), Object::Dict(names))).unwrap();

    // A valid large book exceeds the former 100,000-entry budget (two entries per name).
    delete_pages(&mut doc, &[0]).unwrap();
    for bytes in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
        let reopened = open(bytes);
        assert_eq!(labels(&reopened), ["A2", "A3"]);
        let catalog = reopened.get(reopened.root().unwrap());
        let names = reopened.resolve(catalog.as_dict().unwrap().get(b"Names").unwrap());
        let leaf = reopened.resolve(names.as_dict().unwrap().get(b"Dests").unwrap());
        let leaf = leaf.as_dict().unwrap();
        assert_eq!(leaf.int(b"Unknown"), Some(42));
        let pairs = leaf.get(b"Names").unwrap().as_array().unwrap();
        assert_eq!(pairs.len(), 60_000, "30,000 surviving name/value pairs");
        let page = pages(&reopened).unwrap()[1].obj;
        for (pair, i) in pairs.as_chunks::<2>().0.iter().zip((1..60_000).step_by(2)) {
            assert_eq!(pair[0].as_string().unwrap().to_text(), format!("dest-{i:05}"));
            assert_eq!(pair[1].as_array().unwrap()[0].as_ref(), Some(page));
        }
    }
}

#[test]
fn deleted_pages_are_not_kept_by_what_points_at_them() {
    // A1 links to A3, a bookmark goes to A3, and A2 holds the form's only field. After deleting
    // A2 and A3 neither page, its content nor the field may stay in the saved file.
    let mut doc = doc_a();
    add_bookmark(&mut doc, &[], 0, "To A3", 2).unwrap();
    delete_pages(&mut doc, &[1, 2]).unwrap();
    let out = full_roundtrip(&doc);
    assert_eq!(labels(&out), ["A1"]);
    // The bookmark and A1's link now go nowhere instead of to a missing page.
    let mark = out.get(bookmarks(&out)[0].obj).as_dict().cloned().unwrap();
    assert!(!mark.contains(b"Dest") && !mark.contains(b"A"), "{mark:?}");
    let link = &annots(&out, 0)[0];
    assert!(!link.contains(b"A") && !link.contains(b"Dest"), "{link:?}");
    let mut page_objects = 0;
    for n in out.object_numbers() {
        let o = out.get(ObjRef::new(n, out.generation(n)));
        if let Object::Stream(s) = &*o {
            let data = String::from_utf8_lossy(&s.decoded().unwrap()).into_owned();
            assert!(!data.contains("(A2)") && !data.contains("(A3)"), "object {n} still holds a deleted page's content");
        }
        page_objects += usize::from(o.as_dict().is_some_and(|d| d.name(b"Type") == Some(b"Page")));
    }
    assert_eq!(page_objects, 1);
    let cat = out.get(out.root().unwrap()).as_dict().cloned().unwrap();
    let form = out.resolve(cat.get(b"AcroForm").unwrap());
    assert_eq!(form.as_dict().and_then(|f| f.get(b"Fields")).and_then(Object::as_array).map(Vec::len), Some(0), "the field went with its page");
}

#[test]
fn deleting_the_opening_page_drops_the_open_action() {
    let open_action = |doc: &Document| doc.get(doc.root().unwrap()).as_dict().unwrap().get(b"OpenAction").cloned();
    let set_open = |doc: &mut Document, value: Object| {
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| c.set(b"OpenAction".to_vec(), value)).unwrap();
    };
    let page = |doc: &Document, i: usize| pages(doc).unwrap()[i].obj;
    for as_action in [false, true] {
        let target = |doc: &Document, i: usize| {
            let dest = Object::Array(vec![Object::Ref(page(doc, i)), Object::Name(b"Fit".to_vec())]);
            if as_action {
                let mut a = Dict::new();
                a.set(b"S".to_vec(), Object::Name(b"GoTo".to_vec()));
                a.set(b"D".to_vec(), dest);
                Object::Dict(a)
            } else {
                dest
            }
        };
        // The opening page goes: no OpenAction is left to point at nothing.
        let mut doc = doc_a();
        let open = target(&doc, 0);
        set_open(&mut doc, open);
        delete_pages(&mut doc, &[0]).unwrap();
        let out = full_roundtrip(&doc);
        assert_eq!(labels(&out), ["A2", "A3"]);
        assert_eq!(open_action(&out), None);
        // Another page goes: the opening page keeps its OpenAction.
        let mut doc = doc_a();
        let open = target(&doc, 0);
        set_open(&mut doc, open);
        delete_pages(&mut doc, &[2]).unwrap();
        let out = full_roundtrip(&doc);
        assert_eq!(labels(&out), ["A1", "A2"]);
        assert!(open_action(&out).is_some());
    }
}

#[test]
fn cannot_delete_every_page_or_missing_pages() {
    let mut doc = open(fixture());
    assert_eq!(delete_pages(&mut doc, &[0, 1, 2]), Err(OrganizeError::WouldRemoveAllPages));
    assert_eq!(delete_pages(&mut doc, &[7]), Err(OrganizeError::NoSuchPage(7)));
    assert_eq!(rotate_pages(&mut doc, &[3], 90), Err(OrganizeError::NoSuchPage(3)));
    assert_eq!(OrganizeError::NoSuchPage(3).to_string(), "page 4 does not exist", "the message counts pages from 1 (#1550)");
    assert!(!doc.is_modified(), "failed operations leave the document untouched");
}

#[test]
fn move_pages_reorders() {
    let mut doc = open(fixture());
    move_pages(&mut doc, &[2], 0).unwrap();
    assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 2"]);
    move_pages(&mut doc, &[0, 1], 3).unwrap();
    assert_eq!(labels(&doc), ["Page 2", "Page 3", "Page 1"]);
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 2", "Page 3", "Page 1"]);
}

#[test]
fn reverse_pages_reorders() {
    let mut doc = open(fixture());
    reverse_pages(&mut doc, &[0, 1, 2]).unwrap();
    assert_eq!(labels(&doc), ["Page 3", "Page 2", "Page 1"]);
    // Unsorted, repeated indices name the same slots; page 1's slot stays put.
    reverse_pages(&mut doc, &[2, 1, 2]).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 2"]);
}

#[test]
fn cannot_reverse_missing_or_fewer_than_two_pages() {
    let mut doc = open(fixture());
    assert_eq!(reverse_pages(&mut doc, &[0, 3]), Err(OrganizeError::NoSuchPage(3)));
    assert!(matches!(reverse_pages(&mut doc, &[1, 1]), Err(OrganizeError::Invalid(_))));
    assert!(!doc.is_modified(), "failed operations leave the document untouched");
}

#[test]
fn inserts_blank_page() {
    let mut doc = open(fixture());
    insert_blank_page(&mut doc, 1, 612.0, 792.0).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 1", "blank", "Page 2", "Page 3"]);
    let hay = hayro_syntax::Pdf::new(doc.bytes().clone()).unwrap();
    let (w, h) = hay.pages().get(1).unwrap().render_dimensions();
    assert_eq!((w, h), (612.0, 792.0));
}

#[test]
fn document_info_edits_round_trip_with_unicode() {
    let mut doc = open(fixture());
    set_info(&mut doc, "Title", "Résumé — 履歴書").unwrap();
    set_info(&mut doc, "Author", "PdfCraft").unwrap();
    set_info(&mut doc, "Producer", "").unwrap(); // clearing removes the key
    let doc = save_and_reopen(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("Résumé — 履歴書"));
    assert_eq!(info(&doc, "Author").as_deref(), Some("PdfCraft"));
    assert_eq!(info(&doc, "Producer"), None);
}

/// Page transitions (§12.4.4): written as `/Trans` and `/Dur` with only the entries the style
/// reads and that differ from the defaults, removed again, checked before any page changes, and
/// kept through a save.
#[test]
fn page_transitions_are_set_removed_and_checked() {
    let mut doc = open(fixture());
    let wipe = Transition { duration: 2.5, direction: Some(270), ..Transition::new("Wipe") };
    set_transition(&mut doc, &[0, 2], &TransitionSpec { transition: Some(wipe), display: Some(5.0) }).unwrap();
    let d = page_dict(&doc, 0);
    let t = d.get(b"Trans").and_then(|o| o.as_dict()).expect("/Trans").clone();
    assert_eq!(t.name(b"Type"), Some(&b"Trans"[..]));
    assert_eq!(t.name(b"S"), Some(&b"Wipe"[..]));
    assert_eq!(t.get(b"D"), Some(&Object::Real(2.5)));
    assert_eq!(t.get(b"Di"), Some(&Object::Int(270)));
    assert!(
        t.get(b"Dm").is_none() && t.get(b"M").is_none() && t.get(b"SS").is_none() && t.get(b"B").is_none(),
        "entries Wipe does not read stay out: {t:?}"
    );
    assert_eq!(d.get(b"Dur"), Some(&Object::Real(5.0)));
    assert!(page_dict(&doc, 1).get(b"Trans").is_none(), "page 2 was not asked for");
    // Fly with everything non-default; the defaults themselves are not written.
    let fly = Transition { style: "Fly".into(), duration: 1.0, horizontal: false, inward: false, direction: None, scale: 0.5, opaque: true };
    set_transition(&mut doc, &[1], &TransitionSpec { transition: Some(fly), display: None }).unwrap();
    let t = page_dict(&doc, 1).get(b"Trans").and_then(|o| o.as_dict()).expect("/Trans").clone();
    assert_eq!((t.name(b"S"), t.name(b"M"), t.name(b"Di")), (Some(&b"Fly"[..]), Some(&b"O"[..]), Some(&b"None"[..])));
    assert_eq!((t.get(b"SS"), t.get(b"B")), (Some(&Object::Real(0.5)), Some(&Object::Bool(true))));
    assert!(t.get(b"D").is_none() && t.get(b"Dm").is_none(), "{t:?}");
    // Removed again.
    set_transition(&mut doc, &[0], &TransitionSpec { transition: None, display: None }).unwrap();
    let d = page_dict(&doc, 0);
    assert!(d.get(b"Trans").is_none() && d.get(b"Dur").is_none());
    // Checked before anything changes.
    for (what, spec) in [
        ("style", TransitionSpec { transition: Some(Transition::new("Swirl")), display: None }),
        ("direction", TransitionSpec { transition: Some(Transition { direction: Some(45), ..Transition::new("Wipe") }), display: None }),
        ("duration", TransitionSpec { transition: Some(Transition { duration: -1.0, ..Transition::new("Fade") }), display: None }),
        ("scale", TransitionSpec { transition: Some(Transition { scale: 0.0, ..Transition::new("Fly") }), display: None }),
        ("display", TransitionSpec { transition: None, display: Some(f32::NAN) }),
    ] {
        assert!(set_transition(&mut doc, &[2], &spec).is_err(), "{what} must be refused");
    }
    assert!(set_transition(&mut doc, &[7], &TransitionSpec { transition: None, display: None }).is_err(), "no such page");
    let t = page_dict(&doc, 2).get(b"Trans").and_then(|o| o.as_dict()).expect("page 3 unchanged").clone();
    assert_eq!(t.name(b"S"), Some(&b"Wipe"[..]));
    // Kept through a save.
    let doc = save_and_reopen(&doc);
    // (A whole number of seconds may come back as an integer: the writer's choice.)
    assert_eq!(page_dict(&doc, 2).get(b"Dur").and_then(Object::as_f64), Some(5.0));
}

/// Custom document properties (Document Properties ▸ Custom): any `/Info` key the spec does not
/// name, set and cleared like the standard ones, listed apart from them, and with the names
/// Acrobat refuses refused here too.
#[test]
fn custom_document_properties_are_listed_set_removed_and_their_names_checked() {
    let mut doc = open(fixture());
    assert!(custom_info(&doc).is_empty());
    set_info(&mut doc, "Title", "Report").unwrap();
    set_info(&mut doc, "Project", "Apollo").unwrap();
    set_info(&mut doc, "Revision", "7").unwrap();
    let mut listed = custom_info(&doc);
    listed.sort();
    assert_eq!(listed, [("Project".to_string(), "Apollo".to_string()), ("Revision".to_string(), "7".to_string())], "standard keys are not custom");
    assert_eq!(info(&doc, "Project").as_deref(), Some("Apollo"));
    set_info(&mut doc, "Project", "").unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(custom_info(&doc), [("Revision".to_string(), "7".to_string())], "cleared, and the rest survives a save");
    assert_eq!(info(&doc, "Title").as_deref(), Some("Report"));

    for bad in ["", "Title", "ModDate", "Bad Name", "a/b", "tab\tbed", "hash#tag", "(x)"] {
        assert!(check_custom_key(bad).is_err(), "{bad:?} must be refused");
    }
    let long = "x".repeat(MAX_CUSTOM_KEY_LEN + 1);
    assert!(check_custom_key(&long).is_err());
    for good in ["Project", "Révision", "部門", "key-2.0", &"y".repeat(MAX_CUSTOM_KEY_LEN)] {
        assert!(check_custom_key(good).is_ok(), "{good:?} is a usable name");
    }
    let mut doc = open(fixture());
    assert!(set_info(&mut doc, "Bad Name", "x").is_err(), "set_info checks custom names");
    assert!(custom_info(&doc).is_empty());
}

/// An `/Info` dictionary written directly in the trailer: a cleared entry is removed, and the
/// other entries are kept.
#[test]
fn document_info_entries_are_cleared_from_an_info_dictionary_in_the_trailer() {
    let b = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".into(),
            body("X"),
        ],
        "/Root 1 0 R /Info << /Title (Report) /Project (Apollo) /Revision (7) >>",
    );
    let mut doc = open(b);
    set_info(&mut doc, "Missing", "").unwrap();
    assert!(matches!(doc.trailer().get(b"Info"), Some(Object::Dict(_))), "clearing an absent entry changes nothing");
    set_info(&mut doc, "Project", "").unwrap();
    set_info(&mut doc, "Title", "").unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(custom_info(&doc), [("Revision".to_string(), "7".to_string())]);
    assert_eq!(info(&doc, "Title"), None);
}

/// Padding is not part of a custom value, in any mix of NULs and whitespace, a value that is only
/// padding is not listed, and such entries don't count towards the 256 listed: a real property
/// after 300 blank ones is still there.
#[test]
fn custom_document_properties_drop_padding_before_the_cap() {
    let mut doc = open(fixture());
    set_info(&mut doc, "Title", "Report").unwrap();
    let Some(Object::Ref(info_ref)) = doc.trailer().get(b"Info").cloned() else { panic!("an /Info reference") };
    let text = |s: &str| Object::String(pdfcraft_cos::PdfString { bytes: s.as_bytes().to_vec(), hex: false });
    doc.update_dict(info_ref, |d| {
        d.set(b"Mixed".to_vec(), text(" \0Lab\0 "));
        d.set(b"Pad".to_vec(), text(" \0 \0"));
        for i in 0..300 {
            d.set(format!("Blank{i}").into_bytes(), text("  "));
        }
        d.set(b"Keep".to_vec(), text("yes"));
    })
    .unwrap();
    assert_eq!(custom_info(&doc), [("Mixed".to_string(), "Lab".to_string()), ("Keep".to_string(), "yes".to_string())]);
}

/// A one-page document whose catalog has `xmp` as its (Flate-compressed) XMP packet, and the
/// same title and author in `/Info`, as LibreOffice writes them.
fn doc_with_xmp(xmp: &[u8]) -> Document {
    let b = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".into(),
            body("X"),
            "<< /Title (Untitled 1) /Author (Old Author) >>".into(),
        ],
        "/Root 1 0 R /Info 5 0 R",
    );
    let mut doc = open(b);
    let mut dict = Dict::new();
    dict.set(b"Type".to_vec(), Object::name("Metadata"));
    dict.set(b"Subtype".to_vec(), Object::name("XML"));
    let m = doc.add(Object::Stream(pdfcraft_cos::Stream::flate(dict, xmp)));
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"Metadata".to_vec(), Object::Ref(m))).unwrap();
    full_roundtrip(&doc)
}

/// The catalog's metadata stream.
fn xmp_stream(doc: &Document) -> Option<pdfcraft_cos::Stream> {
    match &*doc.get(catalog(doc).reference(b"Metadata")?) {
        Object::Stream(s) => Some(s.clone()),
        _ => None,
    }
}

fn xmp_text(doc: &Document) -> Option<String> {
    xmp_stream(doc).map(|s| String::from_utf8(s.decoded().unwrap()).unwrap())
}

/// The values the XMP packet gives a property: the items of its rdf:Alt or rdf:Seq, else its text.
fn xmp_values(xmp: &str, name: &str) -> Vec<String> {
    let Some(start) = xmp.find(&format!("<{name}>")) else { return Vec::new() };
    let end = xmp[start..].find(&format!("</{name}>")).unwrap() + start;
    let inner = &xmp[start + name.len() + 2..end];
    if !inner.contains("<rdf:li") {
        return vec![inner.to_string()];
    }
    inner.split("<rdf:li").skip(1).map(|li| li[li.find('>').unwrap() + 1..li.find("</rdf:li>").unwrap()].to_string()).collect()
}

const LIBREOFFICE_XMP: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
  <rdf:Description rdf:about=\"\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"><pdf:Producer>Writer 1.0</pdf:Producer></rdf:Description>\n\
  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">Untitled 1</rdf:li></rdf:Alt></dc:title>\n\
   <dc:creator><rdf:Seq><rdf:li>Old Author</rdf:li></rdf:Seq></dc:creator>\n\
   <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">Old subject</rdf:li></rdf:Alt></dc:description>\n\
  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>";

#[test]
fn document_info_edits_update_the_xmp_packet() {
    // Viewers that read XMP first (pdf.js takes dc:title) showed the old title after a change.
    let mut doc = doc_with_xmp(LIBREOFFICE_XMP.as_bytes());
    assert_eq!(xmp_values(&xmp_text(&doc).unwrap(), "dc:title"), ["Untitled 1"]);
    set_info(&mut doc, "Title", "Sales Guide v1.0 & <draft>").unwrap();
    set_info(&mut doc, "Author", "Demo Consulting").unwrap();
    set_info(&mut doc, "Subject", "").unwrap();
    set_info(&mut doc, "Keywords", "alpha, beta").unwrap();
    set_info(&mut doc, "Rating", "5").unwrap(); // no XMP counterpart: /Info only
    let doc = save_and_reopen(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("Sales Guide v1.0 & <draft>"));
    let xmp = xmp_text(&doc).unwrap();
    assert_eq!(xmp_values(&xmp, "dc:title"), ["Sales Guide v1.0 &amp; &lt;draft&gt;"]);
    assert_eq!(xmp_values(&xmp, "dc:creator"), ["Demo Consulting"]);
    assert!(xmp_values(&xmp, "dc:description").is_empty(), "{xmp}");
    assert_eq!(xmp_values(&xmp, "pdf:Keywords"), ["alpha, beta"]);
    assert_eq!(xmp_values(&xmp, "pdf:Producer"), ["Writer 1.0"]);
    assert!(xmp.starts_with("<?xpacket begin=") && xmp.ends_with("<?xpacket end=\"w\"?>"), "{xmp}");
    // Stored unfiltered, still typed as metadata.
    let dict = xmp_stream(&doc).unwrap().dict;
    assert_eq!((dict.name(b"Type"), dict.name(b"Subtype"), dict.get(b"Filter")), (Some(&b"Metadata"[..]), Some(&b"XML"[..]), None));
}

/// A file that states its title, author and so on only in XMP (PDF 2.0 deprecates `/Info`) shows
/// them in Document Properties (#2373); `/Info` wins where it has a value.
#[test]
fn document_info_falls_back_to_the_xmp_packet() {
    let xmp = LIBREOFFICE_XMP
        .replace(
            "<rdf:li xml:lang=\"x-default\">Untitled 1</rdf:li>",
            "<rdf:li xml:lang=\"de\">Bericht</rdf:li><rdf:li xml:lang=\"x-default\">Report &amp; &#x201C;notes&#8221;</rdf:li>",
        )
        .replace("<rdf:li>Old Author</rdf:li>", "<rdf:li>Ada</rdf:li>\n<rdf:li>Grace</rdf:li>")
        .replace("<pdf:Producer>Writer 1.0</pdf:Producer>", "<pdf:Producer>Writer 1.0</pdf:Producer><pdf:Keywords/>");
    let mut doc = doc_with_xmp(xmp.as_bytes());
    // Both: /Info's own values, XMP for what /Info lacks.
    assert_eq!(info(&doc, "Title").as_deref(), Some("Untitled 1"));
    assert_eq!(info(&doc, "Subject").as_deref(), Some("Old subject"));
    // XMP only.
    doc.trailer_mut().remove(b"Info");
    let doc = full_roundtrip(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("Report & \u{201C}notes\u{201D}"));
    assert_eq!(info(&doc, "Author").as_deref(), Some("Ada; Grace"));
    assert_eq!(info(&doc, "Subject").as_deref(), Some("Old subject"));
    assert_eq!(info(&doc, "Producer").as_deref(), Some("Writer 1.0"));
    assert_eq!(info(&doc, "Keywords"), None, "an empty property is no value");
    assert_eq!(info(&doc, "Rating"), None, "no XMP counterpart");
    // The attribute form, and a packet that isn't UTF-8 is not read.
    let attrs = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                 <rdf:Description rdf:about=\"\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" pdf:Keywords='a, b &amp; c'/></rdf:RDF></x:xmpmeta>";
    let mut doc = doc_with_xmp(attrs.as_bytes());
    doc.trailer_mut().remove(b"Info");
    assert_eq!(info(&doc, "Keywords").as_deref(), Some("a, b & c"));
    let mut latin1 = LIBREOFFICE_XMP.as_bytes().to_vec();
    latin1.extend_from_slice(b"<!-- \xE9 -->");
    let mut doc = doc_with_xmp(&latin1);
    doc.trailer_mut().remove(b"Info");
    assert_eq!(info(&doc, "Subject"), None);
}

#[test]
fn document_info_edits_leave_files_without_usable_xmp_alone() {
    // No packet: none is added.
    let mut doc = open(fixture());
    set_info(&mut doc, "Title", "New").unwrap();
    assert_eq!(xmp_text(&save_and_reopen(&doc)), None);
    // A packet that isn't UTF-8 is not rewritten.
    let mut latin1 = LIBREOFFICE_XMP.as_bytes().to_vec();
    latin1.extend_from_slice(b"<!-- \xE9 -->");
    let mut doc = doc_with_xmp(&latin1);
    set_info(&mut doc, "Title", "New").unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("New"));
    assert_eq!(xmp_stream(&doc).unwrap().decoded().unwrap(), latin1);
}

#[test]
fn successive_incremental_saves_stack_revisions() {
    let mut doc = open(fixture());
    rotate_pages(&mut doc, &[0], 90).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(doc.revisions().len(), 2);
    let mut doc = doc;
    delete_pages(&mut doc, &[1]).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(doc.revisions().len(), 3);
    assert_eq!(labels(&doc), ["Page 1", "Page 3"]);
}

#[test]
fn full_save_drops_unreachable_objects() {
    let mut doc = open(fixture());
    delete_pages(&mut doc, &[0, 1]).unwrap();
    let bytes = write_full(&doc, &SaveOptions::default()).unwrap();
    let full = open(bytes.clone());
    assert_eq!(labels(&full), ["Page 3"]);
    // Deleted pages, their content streams and the orphaned intermediate node are gone.
    assert!(full.object_numbers().len() < doc.object_numbers().len());
    assert_eq!(info(&full, "Title").as_deref(), Some("Original"));
    assert_eq!(hayro_syntax::Pdf::new(bytes).unwrap().pages().len(), 1);
}

#[test]
fn deleting_a_page_drops_the_named_destinations_to_it() {
    // Four names (two in the /Names tree, two in the legacy /Dests dictionary) go to P1 or P3;
    // P2 has a link for each. Deleting P1 must drop its names and their links, keep P3's.
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests 6 0 R >> /Dests 7 0 R >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R >>".into(),                                          // 3
        "<< /Type /Page /Parent 2 0 R /Annots [8 0 R 9 0 R 10 0 R 11 0 R] >>".into(),      // 4
        "<< /Type /Page /Parent 2 0 R >>".into(),                                          // 5
        "<< /Names [(treeFirst) [3 0 R /Fit] (treeLast) [5 0 R /Fit]] >>".into(),          // 6
        "<< /legacyFirst [3 0 R /Fit] /legacyLast << /D [5 0 R /Fit] >> >>".into(),        // 7
        "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] /Dest (treeFirst) >>".into(),      // 8
        "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] /Dest (treeLast) >>".into(),       // 9
        "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] /Dest /legacyFirst >>".into(),     // 10
        "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] /A << /S /GoTo /D /legacyLast >> >>".into(), // 11
    ];
    let mut doc = open(build(&b, "/Root 1 0 R"));
    delete_pages(&mut doc, &[0]).unwrap();
    let out = full_roundtrip(&doc);
    let cat = out.get(out.root().unwrap()).as_dict().cloned().unwrap();
    let tree = out.resolve(out.resolve(cat.get(b"Names").unwrap()).as_dict().unwrap().get(b"Dests").unwrap());
    let pairs = tree.as_dict().unwrap().get(b"Names").and_then(Object::as_array).cloned().unwrap();
    assert_eq!(pairs.len(), 2, "{pairs:?}");
    assert_eq!(pairs[0].as_string().map(|s| s.bytes.clone()), Some(b"treeLast".to_vec()));
    let page3 = pages(&out).unwrap()[1].obj;
    assert_eq!(out.resolve(&pairs[1]).as_array().and_then(|a| a.first()).and_then(Object::as_ref), Some(page3));
    let legacy = out.resolve(cat.get(b"Dests").unwrap()).as_dict().cloned().unwrap();
    assert!(!legacy.contains(b"legacyFirst") && legacy.contains(b"legacyLast"), "{legacy:?}");
    let links = annots(&out, 0);
    assert!(!links[0].contains(b"Dest"), "{:?}", links[0]);
    assert!(links[1].contains(b"Dest"), "{:?}", links[1]);
    assert!(!links[2].contains(b"Dest"), "{:?}", links[2]);
    assert!(links[3].contains(b"A"), "{:?}", links[3]);
}

// ── Combine / extract / split ──────────────────────────────────────────────────────────────────

/// Build a classic-xref PDF from object bodies (object i+1 = bodies[i]).
fn build(bodies: &[String], trailer: &str) -> Vec<u8> {
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
    out.extend_from_slice(format!("trailer\n<< /Size {} {trailer} >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

fn body(text: &str) -> String {
    let s = format!("BT /F1 12 Tf 20 20 Td ({text}) Tj ET");
    format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len())
}

/// Document A: pages A1–A3 sharing one font. A1 links to A3 (GoTo action), A2 links to A1
/// (/Dest) and carries a text field widget; A3 has a note with a popup (a reference cycle).
fn doc_a() -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [12 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Resources << /Font << /F1 9 0 R >> >> /Annots [10 0 R] >>".into(), // 3
        "<< /Type /Page /Parent 2 0 R /Contents 7 0 R /Resources << /Font << /F1 9 0 R >> >> /Annots [11 0 R 12 0 R] >>".into(), // 4
        "<< /Type /Page /Parent 2 0 R /Contents 8 0 R /Resources << /Font << /F1 9 0 R >> >> /Annots [13 0 R] /StructParents 4 >>".into(), // 5
        body("A1"),                                                                  // 6
        body("A2"),                                                                  // 7
        body("A3"),                                                                  // 8
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),             // 9
        "<< /Type /Annot /Subtype /Link /Rect [0 0 50 50] /P 3 0 R /A << /S /GoTo /D [5 0 R /Fit] >> >>".into(), // 10
        "<< /Type /Annot /Subtype /Link /Rect [0 0 50 50] /P 4 0 R /Dest [3 0 R /XYZ 0 400 0] >>".into(), // 11
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Ada) /Rect [60 60 200 80] /P 4 0 R >>".into(), // 12
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (note) /Popup 14 0 R /P 5 0 R /StructParent 7 >>".into(), // 13
        "<< /Type /Annot /Subtype /Popup /Rect [40 40 200 120] /Parent 13 0 R >>".into(), // 14
        "<< /Title (Doc A) /Author (Alice) >>".into(),                               // 15
    ];
    open(build(&b, "/Root 1 0 R /Info 15 0 R"))
}

/// Document B: pages B1, B2 in a nested tree with MediaBox and Rotate inherited from the root.
fn doc_b() -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 2 /MediaBox [0 0 500 250] /Rotate 90 >>".into(),
        "<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 3 0 R /Contents 6 0 R /Resources << /Font << /F1 8 0 R >> >> >>".into(),
        "<< /Type /Page /Parent 3 0 R /Contents 7 0 R /Resources << /Font << /F1 8 0 R >> >> >>".into(),
        body("B1"),
        body("B2"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>".into(),
    ];
    open(build(&b, "/Root 1 0 R"))
}

/// Write in full, reopen with our reader, and require hayro to agree on the page count.
fn full_roundtrip(doc: &Document) -> Document {
    let bytes = write_full(doc, &SaveOptions::default()).expect("writes");
    let reopened = open(bytes.clone());
    let hay = hayro_syntax::Pdf::new(bytes).expect("hayro opens");
    assert_eq!(hay.pages().len(), page_count(&reopened).unwrap());
    reopened
}

fn page_dict(doc: &Document, i: usize) -> Dict {
    doc.get(pages(doc).unwrap()[i].obj).as_dict().cloned().unwrap()
}

fn annots(doc: &Document, i: usize) -> Vec<Dict> {
    let p = page_dict(doc, i);
    let list = p.get(b"Annots").map(|a| doc.resolve(a).as_array().cloned().unwrap_or_default()).unwrap_or_default();
    list.iter().map(|a| doc.resolve(a).as_dict().cloned().unwrap()).collect()
}

fn count_fonts(doc: &Document) -> usize {
    doc.object_numbers().iter().filter(|n| doc.get(ObjRef::new(**n, 0)).as_dict().is_some_and(|d| d.name(b"Type") == Some(b"Font"))).count()
}

#[test]
fn combine_concatenates_with_a_bookmark_per_file() {
    let out = full_roundtrip(&combine(&[("Report A", &doc_a()), ("Report B", &doc_b())]).unwrap());
    assert_eq!(labels(&out), ["A1", "A2", "A3", "B1", "B2"]);
    // Shared resources are copied once per source: one Helvetica, one Times.
    assert_eq!(count_fonts(&out), 2);
    // Bookmarks: one per source file, pointing at its first page.
    let cat = out.get(out.root().unwrap()).as_dict().cloned().unwrap();
    let outlines = out.get(cat.reference(b"Outlines").unwrap()).as_dict().cloned().unwrap();
    let first = out.get(outlines.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(first.get(b"Title").and_then(|t| t.as_string()).map(|s| s.to_text()).as_deref(), Some("Report A"));
    let second = out.get(first.reference(b"Next").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(second.get(b"Title").and_then(|t| t.as_string()).map(|s| s.to_text()).as_deref(), Some("Report B"));
    let dest_page = second.get(b"Dest").and_then(|d| d.as_array()).and_then(|a| a[0].as_ref()).unwrap();
    assert_eq!(dest_page, pages(&out).unwrap()[3].obj);
}

#[test]
fn combiner_adds_each_source_then_releases_it() {
    let bytes = Arc::new(fixture());
    let src = Document::open(bytes.clone()).unwrap();
    let mut combiner = crate::Combiner::new();
    combiner.add("A", &src, None).unwrap();
    drop(src);
    // No output intents to copy, so the combine keeps nothing of the source's file.
    assert_eq!(Arc::strong_count(&bytes), 1);
    let picked: &[usize] = &[1];
    combiner.add("B", &open(fixture()), Some(picked)).unwrap();
    let streamed = combiner.finish().unwrap();
    let whole = combine_selected(&[("A", &open(fixture()), None), ("B", &open(fixture()), Some(picked))]).unwrap();
    assert_eq!(write_full(&streamed, &SaveOptions::default()).unwrap(), write_full(&whole, &SaveOptions::default()).unwrap());
}

#[test]
fn combining_hostile_outlines_ends_their_runs_too() {
    // The bookmark cases of `hostile_outlines_end_their_runs_and_never_loop`, combined: each run ends
    // where it would repeat, and the source's bookmarks sit under its own entry.
    let cases: [(&[&str], &str); 3] = [
        (&["<< /Title (Self) /Parent 3 0 R /Next 5 0 R >>"], "H[Self]"),
        (&["<< /Title (A) /Parent 3 0 R /Next 6 0 R >>", "<< /Title (B) /Parent 3 0 R /Next 5 0 R >>"], "H[A,B]"),
        (&["<< /Title (A) /Parent 3 0 R /Next 99 0 R >>"], "H[A]"),
    ];
    for (items, listed) in cases {
        let out = full_roundtrip(&combine(&[("H", &outline_with(items))]).unwrap());
        assert_eq!(titles(&crate::bookmarks(&out)), [listed]);
    }
    // A bookmark whose first child is itself: nesting is cut off, so the combine still ends.
    let looped = outline_with(&["<< /Title (Self) /Parent 3 0 R /First 5 0 R >>"]);
    let out = full_roundtrip(&combine(&[("H", &looped)]).unwrap());
    assert_eq!(crate::bookmarks(&out).first().map(|b| b.title.as_str()), Some("H"));
}

#[test]
fn inherited_attributes_travel_with_copied_pages() {
    let out = full_roundtrip(&extract_pages(&doc_b(), &[1]).unwrap());
    let p = page_dict(&out, 0);
    assert_eq!(p.int(b"Rotate"), Some(90));
    assert!(p.contains(b"MediaBox") && p.contains(b"Resources"));
    let hay = hayro_syntax::Pdf::new(write_full(&out, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(hay.pages().first().unwrap().render_dimensions(), (250.0, 500.0), "rotated 90°");
}

#[test]
fn resources_split_between_page_and_tree_travel_together() {
    // The page names its own font but draws a form that only the page-tree node lists. Readers
    // (ours included) look a name up the whole chain, so the page shows; a copy that kept only
    // the page's own dictionary drew nothing (#1628).
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] /Resources << /XObject << /X0 5 0 R >> /Font << /F1 6 0 R /F2 6 0 R >> >> >>"
            .into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F2 7 0 R >> >> >>".into(),
        {
            let s = "q 100 0 0 100 50 50 cm /X0 Do Q BT /F2 12 Tf (x) Tj ET";
            format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len())
        },
        {
            let s = "0 0 1 rg 0 0 1 1 re f";
            format!("<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length {} >>\nstream\n{s}\nendstream", s.len())
        },
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".into(),
    ];
    let src = open(build(&b, "/Root 1 0 R"));
    let out = full_roundtrip(&combine(&[("S", &src), ("S again", &src)]).unwrap());
    for i in 0..2 {
        let res = out.dict(page_dict(&out, i).get(b"Resources").unwrap()).unwrap();
        let entry = |cat: &[u8], name: &[u8]| res.get(cat).and_then(|c| out.dict(c)).and_then(|d| d.get(name).map(|v| out.resolve(v)));
        let form = entry(b"XObject", b"X0").expect("the tree's form comes along");
        assert_eq!(form.as_dict().and_then(|d| d.name(b"Subtype")), Some(&b"Form"[..]));
        let font = |name: &[u8]| entry(b"Font", name).and_then(|f| f.as_dict().and_then(|d| d.name(b"BaseFont").map(<[u8]>::to_vec)));
        assert_eq!(font(b"F1").as_deref(), Some(&b"Helvetica"[..]), "a name only the tree has is added");
        assert_eq!(font(b"F2").as_deref(), Some(&b"Courier"[..]), "the page's own entry wins over the tree's");
    }
}

#[test]
fn a_scan_whose_xref_misses_the_image_combines_with_the_image() {
    // A cheap image-to-PDF converter's table: "0 5" followed by the entries of objects 1 to 5,
    // so the image (object 5) has none. Viewers rebuild the table and show the scan; the
    // combined copy got a null image and came out blank (#1628).
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>".into(),
        "<< /Length 26 >>\nstream\n100 0 0 100 0 0 cm /Im0 Do\nendstream".into(),
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\nA\nendstream".into(),
    ];
    let good = build(&b, "/Root 1 0 R");
    let text = String::from_utf8_lossy(&good).into_owned();
    let at = text.rfind("\nxref\n").unwrap() + 1;
    let entries: Vec<&str> = text[at..].lines().skip(3).take(5).collect();
    let mut bytes = good[..at].to_vec();
    bytes.extend_from_slice(format!("xref\n0 5\n{}\ntrailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{at}\n%%EOF\n", entries.join("\n")).as_bytes());
    let src = open(bytes);
    let out = full_roundtrip(&combine(&[("scan", &src)]).unwrap());
    let res = out.dict(page_dict(&out, 0).get(b"Resources").unwrap()).unwrap();
    let image = res.get(b"XObject").and_then(|x| out.dict(x)).and_then(|d| d.get(b"Im0").map(|v| out.resolve(v))).expect("the page lists its image");
    let Object::Stream(image) = &*image else { panic!("the image is a stream, not {image:?}") };
    assert_eq!(image.dict.name(b"Subtype"), Some(&b"Image"[..]));
    assert_eq!(image.decoded().unwrap(), b"A");
}

#[test]
fn links_are_rewired_to_copied_pages_or_dropped() {
    let a = doc_a();
    // Extract A3, A1 (reordered): A1's GoTo A3 must now point at the copy of A3 (index 0).
    let out = full_roundtrip(&extract_pages(&a, &[2, 0]).unwrap());
    assert_eq!(labels(&out), ["A3", "A1"]);
    let link = &annots(&out, 1)[0];
    let action = out.resolve(link.get(b"A").unwrap());
    let target = action.as_dict().unwrap().get(b"D").and_then(|d| d.as_array()).and_then(|a| a[0].as_ref()).unwrap();
    assert_eq!(target, pages(&out).unwrap()[0].obj);
    assert_eq!(link.reference(b"P"), Some(pages(&out).unwrap()[1].obj), "/P points at the new page");
    // Extract only A2: its link to A1 would dangle, so the destination is dropped.
    let out = full_roundtrip(&extract_pages(&a, &[1]).unwrap());
    let link = annots(&out, 0).into_iter().find(|d| d.name(b"Subtype") == Some(b"Link")).unwrap();
    assert!(!link.contains(b"Dest") && !link.contains(b"A"));
}

/// Issue #1810: a widget that is its own field was shared between both copies of a repeated
/// page, so only the last page could identify it (its `/P`), and per-page annotation edits were
/// impossible. Each copy must get its own widget sharing one parent field (name and value).
#[test]
fn repeated_page_splits_a_combined_field_widget() {
    let out = full_roundtrip(&extract_pages(&doc_a(), &[1, 1]).unwrap());
    let ps = pages(&out).unwrap();
    let widget = |i: usize| -> ObjRef {
        let list = page_dict(&out, i).get(b"Annots").map(|a| out.resolve(a).as_array().cloned().unwrap_or_default()).unwrap_or_default();
        list.iter()
            .filter_map(|a| a.as_ref())
            .find(|r| out.get(*r).as_dict().is_some_and(|d| d.name(b"Subtype") == Some(b"Widget")))
            .expect("a widget annotation")
    };
    let (w1, w2) = (widget(0), widget(1));
    assert_ne!(w1, w2, "each copy of the page has its own widget");
    for (i, w) in [(0, w1), (1, w2)] {
        let d = out.get(w).as_dict().unwrap().clone();
        for key in [b"T".as_slice(), b"V", b"FT"] {
            assert!(!d.contains(key), "a widget inherits field entries: {key:?}");
        }
        assert_eq!(d.reference(b"P"), Some(ps[i].obj), "/P names the page holding the widget");
        let parent = d.reference(b"Parent").expect("the widget has a parent field");
        let f = out.get(parent).as_dict().unwrap().clone();
        assert_eq!(f.get(b"T").and_then(|v| v.as_string()).map(|s| s.to_text()).as_deref(), Some("name"));
        assert_eq!(f.get(b"V").and_then(|v| v.as_string()).map(|s| s.to_text()).as_deref(), Some("Ada"));
        let kids: Vec<ObjRef> =
            f.get(b"Kids").and_then(|k| out.resolve(k).as_array().cloned()).unwrap_or_default().iter().filter_map(|k| k.as_ref()).collect();
        assert!(kids.contains(&w1) && kids.contains(&w2), "one field holds both widgets: {kids:?}");
        let fields: Vec<ObjRef> = catalog(&out)
            .get(b"AcroForm")
            .map(|f| out.resolve(f))
            .and_then(|f| f.as_dict().cloned())
            .and_then(|f| f.get(b"Fields").map(|x| out.resolve(x)))
            .and_then(|f| f.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|f| f.as_ref())
            .collect();
        assert_eq!(fields, [parent], "the parent is the one top-level field");
    }
}

#[test]
fn splitting_a_combined_widget_keeps_annotation_actions_and_field_triggers() {
    let mut src = doc_a();
    let page = pages(&src).unwrap()[1].obj;
    let list = page_dict(&src, 1).get(b"Annots").and_then(|a| a.as_array()).cloned().unwrap();
    let widget =
        list.iter().filter_map(Object::as_ref).find(|r| src.get(*r).as_dict().is_some_and(|d| d.name(b"Subtype") == Some(b"Widget"))).unwrap();
    let mut action = Dict::new();
    action.set(b"S".to_vec(), Object::name("GoTo"));
    action.set(b"D".to_vec(), Object::Array(vec![Object::Ref(page), Object::name("Fit")]));
    let mut named = Dict::new();
    named.set(b"S".to_vec(), Object::name("Named"));
    named.set(b"N".to_vec(), Object::name("NextPage"));
    let mut aa = Dict::new();
    aa.set(b"E".to_vec(), Object::Dict(named.clone()));
    aa.set(b"K".to_vec(), Object::Dict(named));
    let aa = src.add(Object::Dict(aa));
    src.update_dict(widget, |d| {
        d.set(b"A".to_vec(), Object::Dict(action));
        d.set(b"AA".to_vec(), Object::Ref(aa));
    })
    .unwrap();
    let out = full_roundtrip(&extract_pages(&src, &[1, 1]).unwrap());
    let ps = pages(&out).unwrap();
    for i in 0..2 {
        let w = annots(&out, i).into_iter().find(|d| d.name(b"Subtype") == Some(b"Widget")).unwrap();
        let action = out.resolve(w.get(b"A").expect("the widget keeps its action"));
        let dest = action.as_dict().unwrap().get(b"D").and_then(|a| a.as_array()).unwrap()[0].as_ref().unwrap();
        assert!(ps.iter().any(|p| p.obj == dest), "the action targets a copied page");
        let waa = out.resolve(w.get(b"AA").expect("widget triggers"));
        assert!(waa.as_dict().unwrap().contains(b"E"));
        assert!(!waa.as_dict().unwrap().contains(b"K"));
        let parent = out.get(w.reference(b"Parent").unwrap()).as_dict().unwrap().clone();
        assert!(!parent.contains(b"A"));
        let faa = out.resolve(parent.get(b"AA").expect("field triggers"));
        assert!(faa.as_dict().unwrap().contains(b"K"));
        assert!(!faa.as_dict().unwrap().contains(b"E"));
    }
}

#[test]
fn splitting_a_combined_widget_keeps_its_annotation_only_keys() {
    let mut src = doc_a();
    let list = page_dict(&src, 1).get(b"Annots").and_then(|a| a.as_array()).cloned().unwrap();
    let widget =
        list.iter().filter_map(Object::as_ref).find(|r| src.get(*r).as_dict().is_some_and(|d| d.name(b"Subtype") == Some(b"Widget"))).unwrap();
    src.update_dict(widget, |d| {
        d.set(b"Contents".to_vec(), Object::String(PdfString::text("alt text")));
        d.set(b"BM".to_vec(), Object::name("Multiply"));
        d.set(b"Lang".to_vec(), Object::String(PdfString::text("en-US")));
        d.set(b"AF".to_vec(), Object::Array(Vec::new()));
    })
    .unwrap();
    let out = full_roundtrip(&extract_pages(&src, &[1, 1]).unwrap());
    for i in 0..2 {
        let w = annots(&out, i).into_iter().find(|d| d.name(b"Subtype") == Some(b"Widget")).unwrap();
        let parent = out.get(w.reference(b"Parent").unwrap()).as_dict().unwrap().clone();
        for key in [b"Contents".as_slice(), b"BM", b"Lang", b"AF"] {
            assert!(w.contains(key), "the widget keeps {key:?}");
            assert!(!parent.contains(key), "the parent field does not take {key:?}");
        }
    }
}

#[test]
fn repeated_page_gets_independent_annotations() {
    // A1 has a link, A2 a link and a form field, A3 a note with a popup; each is listed twice.
    let out = full_roundtrip(&extract_pages(&doc_a(), &[0, 0, 1, 1, 2, 2]).unwrap());
    let ps = pages(&out).unwrap();
    let refs: Vec<Vec<ObjRef>> = (0..6)
        .map(|i| page_dict(&out, i).get(b"Annots").map(|a| out.resolve(a).as_array().cloned().unwrap_or_default()).unwrap_or_default())
        .map(|list| list.iter().filter_map(|a| a.as_ref()).collect())
        .collect();
    for i in (0..6).step_by(2) {
        assert!(!refs[i].is_empty());
        assert_eq!(refs[i].len(), refs[i + 1].len());
        for (a, b) in refs[i].iter().zip(&refs[i + 1]) {
            assert_ne!(a, b, "each copy of the page has its own annotation objects");
        }
    }
    for (i, list) in refs.iter().enumerate() {
        for a in list.iter() {
            assert_eq!(out.get(*a).as_dict().unwrap().reference(b"P"), Some(ps[i].obj), "/P names the page holding the copy");
        }
    }
    // Each note keeps its own popup, pointing back at that note.
    let popups: Vec<ObjRef> = [4, 5].iter().map(|&i| out.get(refs[i][0]).as_dict().unwrap().reference(b"Popup").unwrap()).collect();
    assert_ne!(popups[0], popups[1]);
    for (k, p) in popups.iter().enumerate() {
        assert_eq!(out.get(*p).as_dict().unwrap().reference(b"Parent"), Some(refs[4 + k][0]));
    }
}

#[test]
fn form_fields_come_along_and_are_registered() {
    let out = full_roundtrip(&extract_pages(&doc_a(), &[1]).unwrap());
    let cat = out.get(out.root().unwrap()).as_dict().cloned().unwrap();
    let form = out.resolve(cat.get(b"AcroForm").unwrap());
    let fields = out.resolve(form.as_dict().unwrap().get(b"Fields").unwrap()).as_array().cloned().unwrap();
    assert_eq!(fields.len(), 1);
    let field = out.resolve(&fields[0]).as_dict().cloned().unwrap();
    assert_eq!(field.get(b"V").and_then(|v| v.as_string()).map(|s| s.to_text()).as_deref(), Some("Ada"));
    // The widget child, not the field, names the page (#1810).
    let kid =
        field.get(b"Kids").map(|k| out.resolve(k)).and_then(|k| k.as_array().cloned()).and_then(|k| k.first().and_then(|k| k.as_ref())).unwrap();
    assert_eq!(out.get(kid).as_dict().unwrap().reference(b"P"), Some(pages(&out).unwrap()[0].obj));
}

/// #4020: Extract keeps the bookmarks that go to the pages it copies, mapped to the copies. One whose
/// page was left out goes, unless a bookmark under it stays.
#[test]
fn extract_keeps_the_bookmarks_of_the_pages_it_copies() {
    let mut src = doc_a();
    crate::add_bookmark(&mut src, &[], 0, "One", 0).unwrap();
    crate::add_bookmark(&mut src, &[], 1, "Two", 1).unwrap();
    crate::add_bookmark(&mut src, &[1], 0, "Two.a", 2).unwrap();
    crate::add_bookmark(&mut src, &[], 2, "Three", 2).unwrap();
    let marks = |d: &Document| -> Vec<(String, usize, Option<usize>)> { bookmarks(d).into_iter().map(|b| (b.title, b.path.len(), b.page)).collect() };
    let s = |t: &str| t.to_string();

    let all = full_roundtrip(&extract_pages(&src, &[0, 1, 2]).unwrap());
    assert_eq!(marks(&all), [(s("One"), 1, Some(0)), (s("Two"), 1, Some(1)), (s("Two.a"), 2, Some(2)), (s("Three"), 1, Some(2))]);

    let last = full_roundtrip(&extract_pages(&src, &[2]).unwrap());
    assert_eq!(marks(&last), [(s("Two"), 1, None), (s("Two.a"), 2, Some(0)), (s("Three"), 1, Some(0))], "One's page was left out");

    let none = full_roundtrip(&extract_pages(&doc_a(), &[0]).unwrap());
    assert!(!none.get(none.root().unwrap()).as_dict().unwrap().contains(b"Outlines"), "no bookmarks, no outline");
}

#[test]
fn popup_cycles_copy_once_and_structure_links_are_removed() {
    let out = full_roundtrip(&extract_pages(&doc_a(), &[2]).unwrap());
    let p = page_dict(&out, 0);
    assert!(!p.contains(b"StructParents"));
    let note = &annots(&out, 0)[0];
    assert!(!note.contains(b"StructParent"));
    let popup = out.get(note.reference(b"Popup").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(
        out.get(popup.reference(b"Parent").unwrap()).as_dict().unwrap().get(b"Contents").and_then(|c| c.as_string()).map(|s| s.to_text()).as_deref(),
        Some("note")
    );
    // Nothing unrelated was dragged in: no pages A1/A2, no other content streams.
    assert_eq!(labels(&out), ["A3"]);
}

#[test]
fn inserting_pages_from_another_file_is_an_incremental_edit() {
    let mut a = doc_a();
    import_pages(&mut a, &doc_b(), &[1, 0], 1).unwrap();
    let a = save_and_reopen(&a); // checks the prefix invariant and hayro agreement
    assert_eq!(labels(&a), ["A1", "B2", "B1", "A2", "A3"]);
    assert_eq!(info(&a, "Title").as_deref(), Some("Doc A"));
    assert_eq!(import_pages(&mut a.clone(), &doc_b(), &[5], 0), Err(OrganizeError::NoSuchPage(5)));
}

#[test]
fn split_ranges_cover_every_page_once() {
    assert_eq!(split_ranges(5, &SplitBy::PageCount(2)), [0..2, 2..4, 4..5]);
    assert_eq!(split_ranges(5, &SplitBy::PageCount(0)), [0..1, 1..2, 2..3, 3..4, 4..5]);
    assert_eq!(split_ranges(5, &SplitBy::Before(vec![3, 1, 1, 9, 0])), [0..1, 1..3, 3..5]);
    assert_eq!(split_ranges(3, &SplitBy::PageCount(10)), vec![0..3]);
}

#[test]
fn split_produces_standalone_documents_with_metadata() {
    let parts = split(&doc_a(), &SplitBy::PageCount(1)).unwrap();
    assert_eq!(parts.len(), 3);
    for (i, part) in parts.iter().enumerate() {
        let part = full_roundtrip(part);
        assert_eq!(labels(&part), [format!("A{}", i + 1)]);
        assert_eq!(info(&part, "Author").as_deref(), Some("Alice"));
    }
}

// ── PDF/X (#263) ───────────────────────────────────────────────────────────────────────────────

/// PDF/X-4 identification in XMP, as an attribute, next to a PDF/UA claim.
const X4_XMP: &str = "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF \
                      xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" \
                      xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\" xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\" \
                      pdfxid:GTS_PDFXVersion=\"PDF/X-4\" pdfuaid:part=\"1\"/></rdf:RDF></x:xmpmeta><?xpacket end=\"r\"?>";

/// A two-page print file whose output intent prints to `condition` with `profile` as its ICC
/// profile, with `info` as its document information and `xmp` as its XMP metadata.
fn doc_print(condition: &str, profile: &str, info: &str, xmp: Option<&str>) -> Document {
    let metadata = if xmp.is_some() { " /Metadata 10 0 R" } else { "" };
    let mut b: Vec<String> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /OutputIntents [7 0 R]{metadata} >>"), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 252 144] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /TrimBox [9 9 243 135] >>".into(), // 3
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /TrimBox [9 9 243 135] >>".into(), // 4
        body("X1"),                                                                    // 5
        body("X2"),                                                                    // 6
        format!("<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier ({condition}) /DestOutputProfile 8 0 R >>"), // 7
        format!("<< /N 4 /Length {} >>\nstream\n{profile}\nendstream", profile.len()), // 8
        format!("<< {info} >>"),                                                       // 9
    ];
    if let Some(x) = xmp {
        b.push(format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{x}\nendstream", x.len())); // 10
    }
    open(build(&b, "/Root 1 0 R /Info 9 0 R"))
}

/// A PDF/X-4 business card printing to `condition` with `profile`.
fn doc_x4(condition: &str, profile: &str) -> Document {
    doc_print(condition, profile, "/Title (Card) /GTS_PDFXVersion (PDF/X-4) /Trapped /False", Some(X4_XMP))
}

/// What a document says about PDF/X.
#[derive(Debug, Default, PartialEq)]
struct Pdfx {
    /// Each output intent's printing condition and decoded profile.
    intents: Vec<(String, Vec<u8>)>,
    version: Option<String>,
    trapped: Option<Vec<u8>>,
    xmp: Option<String>,
}

fn pdfx_of(doc: &Document) -> Pdfx {
    let cat = catalog(doc);
    let list = cat.get(b"OutputIntents").map(|o| doc.resolve(o).as_array().cloned().unwrap_or_default()).unwrap_or_default();
    let intents = list
        .iter()
        .filter_map(|oi| {
            let d = doc.resolve(oi).as_dict().cloned()?;
            let id = d.get(b"OutputConditionIdentifier").and_then(|v| doc.resolve(v).as_string().map(|s| s.to_text())).unwrap_or_default();
            let profile = match d.get(b"DestOutputProfile").map(|p| doc.resolve(p)).as_deref() {
                Some(Object::Stream(s)) => s.decoded().unwrap(),
                _ => Vec::new(),
            };
            Some((id, profile))
        })
        .collect();
    let trapped = doc.trailer().get(b"Info").map(|i| doc.resolve(i)).and_then(|i| i.as_dict().and_then(|d| d.name(b"Trapped")).map(<[u8]>::to_vec));
    let xmp = match cat.get(b"Metadata").map(|m| doc.resolve(m)).as_deref() {
        Some(Object::Stream(s)) => Some(String::from_utf8(s.decoded().unwrap()).unwrap()),
        _ => None,
    };
    Pdfx { intents, version: info(doc, "GTS_PDFXVersion"), trapped, xmp }
}

#[test]
fn extract_and_split_keep_the_pdfx_output_intent_and_identification() {
    // #263: the parts had no /OutputIntents, no GTS_PDFXVersion and no XMP pdfxid, so a PDF/X
    // file stopped being one.
    let src = doc_x4("FOGRA39", "icc-fogra39");
    let mut outs = vec![extract_pages(&src, &[1]).unwrap()];
    outs.extend(split(&src, &SplitBy::PageCount(1)).unwrap());
    for out in &outs {
        let x = pdfx_of(&full_roundtrip(out));
        assert_eq!(x.intents, [("FOGRA39".to_string(), b"icc-fogra39".to_vec())]);
        assert_eq!(x.version.as_deref(), Some("PDF/X-4"));
        assert_eq!(x.trapped.as_deref(), Some(&b"False"[..]));
        let xmp = x.xmp.expect("XMP metadata");
        assert!(xmp.contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>"), "{xmp}");
        assert!(xmp.contains(">Card</rdf:li>") && xmp.contains("<pdf:Trapped>False</pdf:Trapped>"), "{xmp}");
        // The structure tree is not copied, so the source's PDF/UA claim must not be either.
        assert!(!xmp.contains("pdfuaid"), "{xmp}");
    }
}

#[test]
fn combine_keeps_pdfx_when_every_source_agrees() {
    // #263: a PDF/X-4 card combined with itself.
    let (a, b) = (doc_x4("FOGRA39", "icc-fogra39"), doc_x4("FOGRA39", "icc-fogra39"));
    let out = full_roundtrip(&combine(&[("a", &a), ("b", &b)]).unwrap());
    assert_eq!(labels(&out), ["X1", "X2", "X1", "X2"]);
    let x = pdfx_of(&out);
    assert_eq!(x.intents, [("FOGRA39".to_string(), b"icc-fogra39".to_vec())], "one output intent, not one per source");
    assert_eq!(x.version.as_deref(), Some("PDF/X-4"));
    assert!(x.xmp.is_some_and(|x| x.contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>")));
}

#[test]
fn combine_claims_no_pdfx_when_the_sources_disagree() {
    // Another printing condition, the same name with another profile, or a source that is not
    // PDF/X: no one standard describes the result, so it claims none.
    let x4 = doc_x4("FOGRA39", "icc-fogra39");
    for other in [doc_x4("GRACoL2013", "icc-gracol"), doc_x4("FOGRA39", "icc-other"), doc_b()] {
        let out = full_roundtrip(&combine(&[("a", &x4), ("b", &other)]).unwrap());
        assert_eq!(pdfx_of(&out), Pdfx::default());
    }
}

#[test]
fn documents_without_a_print_standard_gain_none() {
    assert_eq!(pdfx_of(&full_roundtrip(&extract_pages(&doc_a(), &[0]).unwrap())), Pdfx::default());
    assert_eq!(pdfx_of(&full_roundtrip(&combine(&[("a", &doc_a()), ("b", &doc_b())]).unwrap())), Pdfx::default());
}

#[test]
fn the_pdfx_version_comes_from_xmp_or_the_document_information() {
    // A PDF/X-4 file may name its version only in XMP (here as an element); a PDF/X-1a:2001 file
    // (PDF 1.3) only in its document information, with no XMP, and gets no XMP added.
    let xmp = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description \
               rdf:about=\"\" xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">\
               <pdfxid:GTS_PDFXVersion> PDF/X-4 </pdfxid:GTS_PDFXVersion><pdf:Trapped>True</pdf:Trapped></rdf:Description></rdf:RDF></x:xmpmeta>";
    let x = pdfx_of(&full_roundtrip(&extract_pages(&doc_print("FOGRA51", "icc", "/Title (Flyer)", Some(xmp)), &[0]).unwrap()));
    assert_eq!((x.version.as_deref(), x.trapped.as_deref()), (Some("PDF/X-4"), Some(&b"True"[..])));
    assert!(x.xmp.is_some_and(|x| x.contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>")));

    let x1a = doc_print("CGATS TR 001", "icc", "/GTS_PDFXVersion (PDF/X-1:2001) /GTS_PDFXConformance (PDF/X-1a:2001) /Trapped (False)", None);
    let out = full_roundtrip(&extract_pages(&x1a, &[0]).unwrap());
    let x = pdfx_of(&out);
    assert_eq!((x.version.as_deref(), x.trapped.as_deref()), (Some("PDF/X-1:2001"), Some(&b"False"[..])));
    assert_eq!(info(&out, "GTS_PDFXConformance").as_deref(), Some("PDF/X-1a:2001"));
    assert_eq!(x.xmp, None);
}

#[test]
fn malformed_output_intents_and_identification_do_not_fail() {
    // Untrusted input: intents that are not dictionaries, a missing profile, an intent that is
    // its own profile, an XMP packet that never closes, and a version full of markup and NULs.
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /OutputIntents [5 0 R 42 (junk) 6 0 R] /Metadata 7 0 R >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".into(),                        // 2
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".into(),                                          // 3
        body("M1"),                                                                                        // 4
        "<< /Type /OutputIntent /S /GTS_PDFX /DestOutputProfile 99 0 R >>".into(),                         // 5
        "<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (x) /DestOutputProfile 6 0 R >>".into(), // 6
        "<< /Length 30 >>\nstream\n<pdfxid:GTS_PDFXConformance>X\nendstream".into(),                       // 7
        "<< /GTS_PDFXVersion (<PDF/X-4 & \"more\">\\000) /Trapped /Maybe >>".into(),                       // 8
    ];
    let src = open(build(&b, "/Root 1 0 R /Info 8 0 R"));
    let out = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert_eq!(labels(&out), ["M1"]);
    let list = catalog(&out).get(b"OutputIntents").map(|o| out.resolve(o).as_array().cloned().unwrap_or_default()).unwrap_or_default();
    assert_eq!(list.len(), 4, "every entry is kept as written");
    let x = pdfx_of(&out);
    assert_eq!(x.trapped, None, "/Maybe is not a trapping state");
    let xmp = x.xmp.expect("XMP metadata");
    assert!(xmp.contains("<pdfxid:GTS_PDFXVersion>&lt;PDF/X-4 &amp; &quot;more&quot;&gt;</pdfxid:GTS_PDFXVersion>"), "{xmp}");
    assert!(!xmp.contains('\0') && !xmp.contains("GTS_PDFXConformance"), "{xmp}");
}

#[test]
fn removing_the_pdfx_identification_keeps_the_output_intent_and_other_claims() {
    let mut x4 = doc_x4("FOGRA51", "icc");
    assert!(matches!(remove_standard_id(&mut x4, Standard::PdfA), Err(OrganizeError::Invalid(_))));
    assert!(!x4.is_modified(), "a refused removal changes nothing");
    remove_standard_id(&mut x4, Standard::PdfX).unwrap();
    let x = pdfx_of(&full_roundtrip(&x4));
    assert_eq!((x.version, x.intents), (None, vec![("FOGRA51".to_string(), b"icc".to_vec())]));
    assert!(x.xmp.is_some_and(|x| !x.contains("pdfxid:GTS") && x.contains("pdfuaid:part=\"1\"")), "PDF/UA's claim stays");
    assert!(matches!(remove_standard_id(&mut x4, Standard::PdfX), Err(OrganizeError::Invalid(_))), "nothing left to remove");

    let mut x1a = doc_print("CGATS TR 001", "icc", "/GTS_PDFXVersion (PDF/X-1:2001) /GTS_PDFXConformance (PDF/X-1a:2001)", None);
    remove_standard_id(&mut x1a, Standard::PdfX).unwrap();
    assert_eq!((info(&x1a, "GTS_PDFXVersion"), info(&x1a, "GTS_PDFXConformance")), (None, None));
}

#[test]
fn removing_the_pdfa_identification_removes_its_xmp_properties_in_either_form() {
    let xmp = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description \
               rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" pdfaid:conformance='B'>\
               <pdfaid:part>2</pdfaid:part><dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">Report</rdf:li></rdf:Alt></dc:title>\
               </rdf:Description></rdf:RDF></x:xmpmeta>";
    let mut doc = doc_print("sRGB IEC61966-2.1", "icc", "/Title (Report)", Some(xmp));
    remove_standard_id(&mut doc, Standard::PdfA).unwrap();
    let x = pdfx_of(&full_roundtrip(&doc));
    let packet = x.xmp.unwrap();
    assert!(!packet.contains("pdfaid:part") && !packet.contains("pdfaid:conformance"), "{packet}");
    assert!(packet.contains(">Report</rdf:li>"), "the rest of the packet stays: {packet}");
    assert_eq!(x.intents.len(), 1, "the output intent stays");
}

#[test]
fn duplicate_pages_become_independent_copies() {
    let out = full_roundtrip(&extract_pages(&doc_b(), &[0, 0]).unwrap());
    assert_eq!(labels(&out), ["B1", "B1"]);
    let ps = pages(&out).unwrap();
    assert_ne!(ps[0].obj, ps[1].obj, "a page object may appear only once in the tree");
}

/// Document C: two pages. Page 1 has a link to the named destination `chap2` (page 2); page 2's
/// content uses a layer that is off by default. It has a bookmark tree and one attachment.
fn doc_c() -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests 9 0 R /EmbeddedFiles 13 0 R >> /OCProperties << /OCGs [8 0 R] /D << /OFF [8 0 R] >> >> /Outlines 10 0 R >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 7 0 R >> >> /Annots [<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Dest (chap2) >>] >>".into(), // 3
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> /Properties << /L1 8 0 R >> >> >>".into(), // 4
        body("C1"),                                                                   // 5
        body("C2"),                                                                   // 6
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),             // 7
        "<< /Type /OCG /Name (Draft marks) >>".into(),                                // 8
        "<< /Names [(chap2) [4 0 R /Fit]] >>".into(),                                 // 9
        "<< /Type /Outlines /First 11 0 R /Last 11 0 R /Count 1 >>".into(),          // 10
        "<< /Title (Chapter one) /Parent 10 0 R /Dest [3 0 R /Fit] /First 12 0 R /Last 12 0 R /Count 1 >>".into(), // 11
        "<< /Title (Section) /Parent 11 0 R /A << /S /GoTo /D (chap2) >> >>".into(),  // 12
        "<< /Names [(notes.txt) 14 0 R] >>".into(),                                   // 13
        "<< /Type /Filespec /F (notes.txt) /EF << /F 15 0 R >> >>".into(),            // 14
        "<< /Type /EmbeddedFile /Length 5 >>\nstream\nhello\nendstream".into(),       // 15
    ];
    open(build(&b, "/Root 1 0 R"))
}

fn catalog(doc: &Document) -> Dict {
    doc.get(doc.root().unwrap()).as_dict().cloned().unwrap()
}

#[test]
fn named_destinations_are_resolved_to_the_copies() {
    let out = full_roundtrip(&extract_pages(&doc_c(), &[0, 1]).unwrap());
    let link = &annots(&out, 0)[0];
    let dest = link.get(b"Dest").and_then(|d| d.as_array()).expect("explicit destination");
    assert_eq!(dest[0].as_ref(), Some(pages(&out).unwrap()[1].obj));
    // Without the target page, the link loses its destination instead of dangling.
    let alone = full_roundtrip(&extract_pages(&doc_c(), &[0]).unwrap());
    assert!(!annots(&alone, 0)[0].contains(b"Dest"));
}

#[test]
fn layers_are_registered_with_their_default_state() {
    let out = full_roundtrip(&extract_pages(&doc_c(), &[1]).unwrap());
    let props = out.resolve(catalog(&out).get(b"OCProperties").unwrap()).as_dict().cloned().unwrap();
    let ocgs = props.get(b"OCGs").and_then(|o| o.as_array()).unwrap().clone();
    assert_eq!(ocgs.len(), 1);
    let name = out.resolve(&ocgs[0]).as_dict().unwrap().get(b"Name").and_then(|n| n.as_string()).map(|s| s.to_text());
    assert_eq!(name.as_deref(), Some("Draft marks"));
    let d = props.get(b"D").and_then(|d| d.as_dict()).unwrap();
    assert_eq!(d.get(b"OFF").and_then(|o| o.as_array()).unwrap(), &ocgs, "stays off by default");
    // Page 1 uses no layer, so extracting it alone adds no OCProperties.
    let out = extract_pages(&doc_c(), &[0]).unwrap();
    assert!(!catalog(&out).contains(b"OCProperties"));
}

/// The top-level bookmarks' titles, in order.
fn top_titles(doc: &Document) -> Vec<String> {
    let Some(outlines) = catalog(doc).reference(b"Outlines") else { return Vec::new() };
    let mut item = doc.get(outlines).as_dict().and_then(|o| o.reference(b"First"));
    let mut titles = Vec::new();
    while let Some(r) = item {
        let d = doc.get(r).as_dict().cloned().unwrap();
        titles.push(d.get(b"Title").and_then(|t| t.as_string()).map(|s| s.to_text()).unwrap());
        item = d.reference(b"Next");
    }
    titles
}

#[test]
fn a_file_split_around_another_is_copied_once_with_one_bookmark() {
    // C's first page, then all of A, then C's second page.
    let (a, c) = (doc_a(), doc_c());
    let out = full_roundtrip(&crate::combine_grouped(&[(0, "C", &c, Some(&[0])), (1, "A", &a, None), (0, "C", &c, Some(&[1]))]).unwrap());
    assert_eq!(labels(&out), ["C1", "A1", "A2", "A3", "C2"]);
    assert_eq!(top_titles(&out), ["C", "A"], "one bookmark per file, not per run");
    let ps = pages(&out).unwrap();
    let outlines = out.get(catalog(&out).reference(b"Outlines").unwrap()).as_dict().cloned().unwrap();
    let file = out.get(outlines.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(file.get(b"Dest").and_then(|d| d.as_array()).unwrap()[0].as_ref(), Some(ps[0].obj), "C's bookmark: its first page shown");
    // Its own bookmarks follow its pages wherever they went: the section is on C2, now last.
    let chapter = out.get(file.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    let section = out.get(chapter.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(section.get(b"Dest").and_then(|d| d.as_array()).unwrap()[0].as_ref(), Some(ps[4].obj));
    // The link from C1 to C2 still works across A's pages between them.
    let link = &annots(&out, 0)[0];
    assert_eq!(link.get(b"Dest").and_then(|d| d.as_array()).expect("kept").first().and_then(|o| o.as_ref()), Some(ps[4].obj));
    // Its attachment once, not once per run.
    let names = out.resolve(catalog(&out).get(b"Names").unwrap()).as_dict().cloned().unwrap();
    let tree = out.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
    let keys: Vec<String> = tree.get(b"Names").and_then(|n| n.as_array()).unwrap().chunks(2).map(|p| p[0].as_string().unwrap().to_text()).collect();
    assert_eq!(keys, ["notes.txt"]);
}

#[test]
fn a_page_shown_twice_gets_its_own_annotations() {
    // One page with an indirect comment (and its popup), and a second page.
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),                                                          // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(),                    // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [6 0 R] >>".into(),                            // 3
        "<< /Type /Page /Parent 2 0 R /Contents 7 0 R >>".into(),                                            // 4
        body("D1"),                                                                                          // 5
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (note) /P 3 0 R /Popup 8 0 R >>".into(), // 6
        body("D2"),                                                                                          // 7
        "<< /Type /Annot /Subtype /Popup /Rect [10 10 50 50] /Parent 6 0 R >>".into(),                       // 8
    ];
    let d = open(build(&b, "/Root 1 0 R"));
    // Page 1 twice, from one file split around its page 2 (and the same through combine_selected).
    for out in [
        crate::combine_grouped(&[(0, "D", &d, Some(&[0, 1])), (0, "D", &d, Some(&[0]))]).unwrap(),
        combine_selected(&[("D", &d, Some(&[0, 1, 0]))]).unwrap(),
    ] {
        let out = full_roundtrip(&out);
        assert_eq!(labels(&out), ["D1", "D2", "D1"]);
        let ps = pages(&out).unwrap();
        let annot_of = |i: usize| page_dict(&out, i).get(b"Annots").and_then(|a| a.as_array()).and_then(|a| a[0].as_ref()).unwrap();
        let (first, again) = (annot_of(0), annot_of(2));
        assert_ne!(first, again, "two pages, two comments");
        for (annot, page) in [(first, ps[0].obj), (again, ps[2].obj)] {
            let a = out.get(annot).as_dict().cloned().unwrap();
            assert_eq!(a.reference(b"P"), Some(page), "each comment belongs to its own page");
            let popup = out.get(a.reference(b"Popup").unwrap()).as_dict().cloned().unwrap();
            assert_eq!(popup.reference(b"Parent"), Some(annot), "and its popup to it");
        }
    }
}

#[test]
fn grouped_runs_keep_their_order_repeats_and_separate_files() {
    let (a, b) = (doc_a(), doc_b());
    // B's page 2, A's page 1, B's page 1, A's page 1 again (a repeat is a page of its own).
    let out = full_roundtrip(
        &crate::combine_grouped(&[(0, "B", &b, Some(&[1])), (1, "A", &a, Some(&[0])), (0, "B", &b, Some(&[0])), (1, "A", &a, Some(&[0]))]).unwrap(),
    );
    assert_eq!(labels(&out), ["B2", "A1", "B1", "A1"]);
    assert_eq!(top_titles(&out), ["B", "A"]);
    let ps = pages(&out).unwrap();
    assert_ne!(ps[1].obj, ps[3].obj, "a page shown twice is two pages");
    // The same file added twice on purpose (two groups) keeps two bookmarks, as combine does.
    let twice = full_roundtrip(&crate::combine_grouped(&[(0, "A", &a, None), (1, "A again", &a, None)]).unwrap());
    assert_eq!(top_titles(&twice), ["A", "A again"]);
    // A page that isn't there is refused, as combine_selected does.
    assert!(matches!(crate::combine_grouped(&[(0, "A", &a, Some(&[9]))]), Err(crate::OrganizeError::NoSuchPage(9))));
}

#[test]
fn combine_nests_source_bookmarks_and_keeps_attachments() {
    let out = full_roundtrip(&combine(&[("C", &doc_c()), ("C again", &doc_c())]).unwrap());
    assert_eq!(labels(&out), ["C1", "C2", "C1", "C2"]);
    let outlines = out.get(catalog(&out).reference(b"Outlines").unwrap()).as_dict().cloned().unwrap();
    let file = out.get(outlines.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert!(file.int(b"Count").unwrap() < 0, "source bookmarks start collapsed");
    let chapter = out.get(file.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(chapter.get(b"Title").and_then(|t| t.as_string()).map(|s| s.to_text()).as_deref(), Some("Chapter one"));
    let section = out.get(chapter.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    let dest = section.get(b"Dest").and_then(|d| d.as_array()).expect("GoTo (named) became an explicit destination");
    assert_eq!(dest[0].as_ref(), Some(pages(&out).unwrap()[1].obj));
    // The second copy's bookmarks point into the second copy.
    let file2 = out.get(file.reference(b"Next").unwrap()).as_dict().cloned().unwrap();
    let chapter2 = out.get(file2.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(chapter2.get(b"Dest").and_then(|d| d.as_array()).unwrap()[0].as_ref(), Some(pages(&out).unwrap()[2].obj));
    // Attachments from both sources, the duplicate name disambiguated.
    let names = out.resolve(catalog(&out).get(b"Names").unwrap()).as_dict().cloned().unwrap();
    let tree = out.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
    let keys: Vec<String> = tree.get(b"Names").and_then(|n| n.as_array()).unwrap().chunks(2).map(|p| p[0].as_string().unwrap().to_text()).collect();
    assert_eq!(keys, ["notes.txt", "notes.txt (2)"]);
}

/// Insert Pages from File brings the inserted file's bookmarks along (they used to be dropped),
/// collapsed under one bookmark named after the file at its first inserted page, placed among the
/// host's bookmarks where the pages went. Destinations follow the copies; named ones are resolved.
#[test]
fn inserting_a_file_brings_its_bookmarks_under_one_named_after_it() {
    let mut host = open(fixture());
    add_bookmark(&mut host, &[], 0, "Start", 0).unwrap();
    add_bookmark(&mut host, &[], 1, "End", 2).unwrap();
    insert_file_pages(&mut host, &doc_c(), "annex", &[0, 1], 1, FieldConflicts::Rename).unwrap();
    let host = save_and_reopen(&host);
    assert_eq!(labels(&host), ["Page 1", "C1", "C2", "Page 2", "Page 3"]);
    let marks = bookmarks(&host);
    assert_eq!(titles(&marks), ["Start", "annex[Chapter one[Section]]", "End"]);
    let page_of = |title: &str| marks.iter().find(|b| b.title == title).and_then(|b| b.page);
    assert_eq!((page_of("annex"), page_of("Chapter one"), page_of("Section"), page_of("End")), (Some(1), Some(1), Some(2), Some(4)));
    assert!(!marks.iter().find(|b| b.title == "annex").unwrap().open, "the file's bookmarks start collapsed");
    // Inserted last, after every bookmark; only the pages chosen keep a destination.
    let mut host = open(fixture());
    add_bookmark(&mut host, &[], 0, "Start", 0).unwrap();
    insert_file_pages(&mut host, &doc_c(), "annex", &[0], 9, FieldConflicts::Rename).unwrap();
    let marks = bookmarks(&host);
    assert_eq!(titles(&marks), ["Start", "annex[Chapter one[Section]]"]);
    assert_eq!(marks.iter().find(|b| b.title == "Section").unwrap().page, None, "its page was left out");
    // A file without bookmarks adds none, and a host without an outline gets one only when needed.
    let mut host = open(fixture());
    insert_file_pages(&mut host, &doc_b(), "plain", &[0], 0, FieldConflicts::Rename).unwrap();
    assert!(bookmarks(&host).is_empty());
    assert!(!catalog(&host).contains(b"Outlines"));
}

/// Bookmarks say where on their page they go, through every kind of destination: explicit,
/// named (resolved), and a GoTo action's.
#[test]
fn bookmarks_report_the_view_their_destination_asks_for() {
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R /Names << /Dests << /Names [(sec) [4 0 R /FitH 300]] >> >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Outlines /First 6 0 R /Last 9 0 R /Count 4 >>".into(),
    ];
    objs.push("<< /Title (Explicit) /Parent 5 0 R /Next 7 0 R /Dest [4 0 R /XYZ 20 250 null] >>".into()); // 6
    objs.push("<< /Title (Named) /Parent 5 0 R /Prev 6 0 R /Next 8 0 R /Dest (sec) >>".into()); // 7
    objs.push("<< /Title (Action) /Parent 5 0 R /Prev 7 0 R /Next 9 0 R /A << /S /GoTo /D [3 0 R /FitR 10 20 110 (x) 999] >> >>".into()); // 8
    objs.push("<< /Title (Nowhere) /Parent 5 0 R /Prev 8 0 R /Dest (missing) >>".into()); // 9
    let doc = open(build(&objs, "/Root 1 0 R"));
    type View = Option<(Vec<u8>, Vec<Option<f64>>)>;
    let views: Vec<(String, Option<usize>, View)> = bookmarks(&doc).into_iter().map(|b| (b.title, b.page, b.view)).collect();
    assert_eq!(
        views,
        [
            ("Explicit".into(), Some(1), Some((b"XYZ".to_vec(), vec![Some(20.0), Some(250.0), None]))),
            ("Named".into(), Some(1), Some((b"FitH".to_vec(), vec![Some(300.0)]))),
            ("Action".into(), Some(0), Some((b"FitR".to_vec(), vec![Some(10.0), Some(20.0), Some(110.0), None]))),
            ("Nowhere".into(), None, None),
        ]
    );
}

#[test]
fn combine_and_insert_share_identical_resources() {
    let a = doc_a();
    let fonts = |d: &Document| {
        d.object_numbers()
            .into_iter()
            .filter(|n| d.get(ObjRef::new(*n, d.generation(*n))).as_dict().is_some_and(|x| x.name(b"Type") == Some(b"Font")))
            .count()
    };
    assert_eq!(fonts(&a), 1);
    // The same document three times: one font, and every page still renders its own text.
    let out = full_roundtrip(&crate::combine(&[("one", &a), ("two", &a), ("three", &a)]).unwrap());
    assert_eq!(fonts(&out), 1);
    assert_eq!(labels(&out), ["A1", "A2", "A3", "A1", "A2", "A3", "A1", "A2", "A3"]);
    // Inserting pages that use the same font reuses the font already there, without touching it.
    let mut d = doc_a();
    let font_before =
        d.object_numbers().into_iter().find(|n| d.get(ObjRef::new(*n, 0)).as_dict().is_some_and(|x| x.name(b"Type") == Some(b"Font"))).unwrap();
    crate::import_pages(&mut d, &doc_a(), &[0, 1], 3).unwrap();
    assert_eq!(fonts(&d), 1);
    assert!(!d.modified_objects().contains(&font_before));
    let d = full_roundtrip(&d);
    assert_eq!(labels(&d), ["A1", "A2", "A3", "A1", "A2"]);
}

/// A one-page document drawing one image, written the way ImageMagick writes it: the colour
/// space is an indirect `/DeviceRGB` object of its own (#878).
fn indirect_colour_space_doc() -> Document {
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(b"q 100 0 0 100 0 0 cm /Im0 Do Q"),
        [
            b"<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace 6 0 R /BitsPerComponent 8 /Length 6 >>\nstream\n".as_slice(),
            &[255, 0, 0, 0, 0, 255],
            b"\nendstream",
        ]
        .concat(),
        b"/DeviceRGB".to_vec(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    open(out)
}

#[test]
fn combine_stores_an_image_once_when_its_colour_space_is_an_indirect_name() {
    let images = |d: &Document| {
        d.object_numbers()
            .into_iter()
            .filter(|n| matches!(&*d.get(ObjRef::new(*n, d.generation(*n))), Object::Stream(s) if s.dict.name(b"Subtype") == Some(b"Image")))
            .count()
    };
    let src = indirect_colour_space_doc();
    let out = full_roundtrip(&crate::combine(&[("one", &src), ("two", &src), ("three", &src)]).unwrap());
    assert_eq!(images(&out), 1);
    assert_eq!(pages(&out).unwrap().len(), 3);
    // Inserting the same pages again reuses the image already there.
    let mut d = indirect_colour_space_doc();
    crate::import_pages(&mut d, &indirect_colour_space_doc(), &[0], 1).unwrap();
    assert_eq!(images(&full_roundtrip(&d)), 1);
}

// ---- bookmarks (M4.6) --------------------------------------------------------------------------

/// The titles with children in brackets (`Parent[Child,Child]`), one per top-level bookmark.
fn titles(b: &[crate::Bookmark]) -> Vec<String> {
    fn level(b: &[crate::Bookmark], at: &mut usize, depth: usize) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(x) = b.get(*at).filter(|x| x.path.len() == depth + 1) {
            *at += 1;
            let kids = level(b, at, depth + 1);
            out.push(if kids.is_empty() { x.title.clone() } else { format!("{}[{}]", x.title, kids.join(",")) });
        }
        out
    }
    let mut at = 0;
    level(b, &mut at, 0)
}

fn count_of(doc: &Document, r: ObjRef) -> Option<i64> {
    doc.get(r).as_dict().and_then(|d| d.int(b"Count"))
}

#[test]
fn bookmarks_add_rename_move_delete() {
    let mut d = doc_a();
    assert!(crate::bookmarks(&d).is_empty());
    assert_eq!(crate::add_bookmark(&mut d, &[], 0, "Intro", 0).unwrap(), vec![0]);
    assert_eq!(crate::add_bookmark(&mut d, &[], 9, "Results", 2).unwrap(), vec![1]);
    assert_eq!(crate::add_bookmark(&mut d, &[1], 0, "Table", 1).unwrap(), vec![1, 0]);
    crate::add_bookmark(&mut d, &[1], 1, "Chart", 2).unwrap();
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results[Table,Chart]"]);

    crate::rename_bookmark(&mut d, &[1, 1], "Figure 1").unwrap();
    assert_eq!(crate::move_bookmark(&mut d, &[1, 1], &[], 0).unwrap(), vec![0]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Figure 1", "Intro", "Results[Table]"]);
    // Into a later sibling: the target path is adjusted for the removal.
    assert_eq!(crate::move_bookmark(&mut d, &[0], &[2], 9).unwrap(), vec![1, 1]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results[Table,Figure 1]"]);
    assert_eq!(crate::move_bookmark(&mut d, &[1], &[1, 0], 0), Err(crate::OutlineError::IntoItself));
    // Into a bookmark with no children yet (Indent): it opens, so the moved one shows.
    assert_eq!(crate::move_bookmark(&mut d, &[1, 1], &[0], 0).unwrap(), vec![0, 0]);
    let intro = crate::bookmarks(&d)[0].obj;
    assert_eq!(count_of(&d, intro), Some(1));
    // Reordering inside a closed parent leaves it closed.
    crate::add_bookmark(&mut d, &[0], 1, "Second", 0).unwrap();
    crate::set_bookmark_open(&mut d, &[0], false).unwrap();
    crate::move_bookmark(&mut d, &[0, 1], &[0], 0).unwrap();
    assert_eq!(count_of(&d, intro), Some(-2));
    crate::delete_bookmark(&mut d, &[0, 0]).unwrap();
    crate::move_bookmark(&mut d, &[0, 0], &[1], 9).unwrap();
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results[Table,Figure 1]"]);
    crate::delete_bookmark(&mut d, &[1, 0]).unwrap();
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results[Figure 1]"]);

    // Counts: the root counts visible items; open items count their visible descendants.
    let b = crate::bookmarks(&d);
    let root = d.get(d.root().unwrap()).as_dict().unwrap().reference(b"Outlines").unwrap();
    assert_eq!(count_of(&d, root), Some(3));
    assert_eq!(count_of(&d, b[1].obj), Some(1));
    crate::set_bookmark_open(&mut d, &[1], false).unwrap();
    assert_eq!((count_of(&d, b[1].obj), count_of(&d, root)), (Some(-1), Some(2)));

    // Destinations point at the chosen page; errors are specific.
    crate::set_bookmark_page(&mut d, &[0], 2).unwrap();
    let dest = d.get(b[0].obj).as_dict().unwrap().get(b"Dest").cloned().unwrap();
    let pages = crate::walk(&d).unwrap();
    assert_eq!(dest.as_array().unwrap()[0], Object::Ref(pages[2].0));
    assert_eq!(crate::rename_bookmark(&mut d, &[5], "x"), Err(crate::OutlineError::NoSuchBookmark(vec![5])));
    assert_eq!(crate::rename_bookmark(&mut d, &[0], "  "), Err(crate::OutlineError::EmptyTitle));
    assert!(crate::add_bookmark(&mut d, &[], 0, "Nowhere", 7).is_err());

    // Everything survives a save and reopen.
    let back = full_roundtrip(&d);
    assert_eq!(titles(&crate::bookmarks(&back)), ["Intro", "Results[Figure 1]"]);
}

#[test]
fn a_bookmark_tree_nests_entries_by_level_under_a_new_first_bookmark() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "Existing", 0).unwrap();
    let e = |level, title: &str, page, element| crate::OutlineEntry { level, title: title.into(), page, element, top: None };
    let se = ObjRef::new(4242, 0);
    let entries = [e(1, "One", 0, Some(se)), e(3, "Deep", 1, None), e(2, "Two", 1, None), e(1, "Next", 2, None)];
    assert_eq!(crate::add_bookmark_tree(&mut d, "Untitled", &entries).unwrap(), vec![0]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Untitled[One[Deep,Two],Next]", "Existing"]);

    let b = crate::bookmarks(&d);
    let root = d.get(d.root().unwrap()).as_dict().unwrap().reference(b"Outlines").unwrap();
    // Listing order: Untitled, One, Deep, Two, Next, Existing.
    assert_eq!((count_of(&d, root), count_of(&d, b[0].obj), count_of(&d, b[1].obj)), (Some(6), Some(4), Some(2)));
    let one = d.get(b[1].obj).as_dict().cloned().unwrap();
    assert_eq!(one.get(b"SE"), Some(&Object::Ref(se)));
    let pages = crate::walk(&d).unwrap();
    let next = d.get(b[4].obj).as_dict().cloned().unwrap();
    assert_eq!(next.get(b"Dest").and_then(|x| x.as_array()).map(|a| a[0].clone()), Some(Object::Ref(pages[2].0)));

    assert_eq!(crate::add_bookmark_tree(&mut d, "Untitled", &[]), Err(crate::OutlineError::NoEntries));
    assert!(crate::add_bookmark_tree(&mut d, "Untitled", &[e(1, "Nowhere", 9, None)]).is_err());
    assert_eq!(titles(&crate::bookmarks(&full_roundtrip(&d))), ["Untitled[One[Deep,Two],Next]", "Existing"]);
}

#[test]
fn bookmark_edits_keep_unknown_keys_and_touch_few_objects() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "One", 0).unwrap();
    crate::add_bookmark(&mut d, &[], 1, "Two", 1).unwrap();
    let first = crate::bookmarks(&d)[0].obj;
    d.update_dict(first, |x| x.set(b"C".to_vec(), Object::Array(vec![Object::Real(1.0), Object::Int(0), Object::Int(0)]))).unwrap();
    let bytes = std::sync::Arc::new(write_full(&d, &SaveOptions::default()).unwrap());
    let mut d = Document::open(bytes).unwrap();
    crate::rename_bookmark(&mut d, &[0], "Uno").unwrap();
    let b = crate::bookmarks(&d);
    assert_eq!(b[0].title, "Uno");
    assert!(d.get(b[0].obj).as_dict().unwrap().get(b"C").is_some(), "colour kept");
    assert_eq!(d.modified_objects(), vec![b[0].obj.num], "a rename rewrites only that item");
}

/// A flat outline of `n` top-level bookmarks, `Item 0` to `Item n-1`, all going to one page.
fn flat_outline(n: usize) -> Vec<u8> {
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        format!("<< /Type /Outlines /First 5 0 R /Last {} 0 R /Count {n} >>", 4 + n),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ];
    for k in 0..n {
        let prev = if k == 0 { String::new() } else { format!(" /Prev {} 0 R", 4 + k) };
        let next = if k + 1 == n { String::new() } else { format!(" /Next {} 0 R", 6 + k) };
        b.push(format!("<< /Title (Item {k}) /Parent 3 0 R{prev}{next} /Dest [4 0 R /Fit] >>"));
    }
    build(&b, "/Root 1 0 R")
}

#[test]
fn a_bookmark_past_the_hundred_thousandth_is_listed() {
    let d = open(flat_outline(100_005));
    let all = crate::bookmarks(&d);
    assert_eq!(all.len(), 100_005);
    assert_eq!(all.last().map(|b| b.title.as_str()), Some("Item 100004"));
}

/// A chain of `n` nested bookmarks: each is the first child of the one before it.
fn deep_outline(n: usize) -> Vec<u8> {
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ];
    for k in 0..n {
        let parent = if k == 0 { 3 } else { 4 + k };
        let first = if k + 1 == n { String::new() } else { format!(" /First {} 0 R /Count -1", 6 + k) };
        b.push(format!("<< /Title (Deep {k}) /Parent {parent} 0 R{first} /Dest [4 0 R /Fit] >>"));
    }
    build(&b, "/Root 1 0 R")
}

#[test]
fn deep_bookmark_chains_edit_without_recursion() {
    let mut d = open(deep_outline(100_000));
    // Adding at the top level recounts the whole outline, which walks the entire chain.
    crate::add_bookmark(&mut d, &[], 1, "Extra", 0).unwrap();
    // Listing enters levels 0 to 32 of the chain; the rest is reached by path only.
    let listed = crate::bookmarks(&d);
    assert_eq!(listed.len(), 34);
    assert_eq!(listed.last().map(|b| (b.title.as_str(), b.path.clone())), Some(("Extra", vec![1])));
    assert_eq!(listed[32].path.len(), 33);
    assert_eq!(crate::bookmark_page(&d, 33, 10).bookmarks.len(), 1);
}

/// A one-page document whose outline is the given bookmark bodies, as objects 5, 6, … in order.
fn outline_with(items: &[&str]) -> Document {
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        format!("<< /Type /Outlines /First 5 0 R /Count {} >>", items.len()),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ];
    b.extend(items.iter().map(ToString::to_string));
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn hostile_outlines_end_their_runs_and_never_loop() {
    let listed = |items: &[&str]| titles(&crate::bookmarks(&outline_with(items)));
    // A bookmark that is its own next sibling, or its own first child.
    assert_eq!(listed(&["<< /Title (Self) /Parent 3 0 R /Next 5 0 R >>"]), ["Self"]);
    assert_eq!(listed(&["<< /Title (Self) /Parent 3 0 R /First 5 0 R >>"]), ["Self"]);
    // Two siblings that point at each other: the run ends where it would repeat.
    assert_eq!(listed(&["<< /Title (A) /Parent 3 0 R /Next 6 0 R >>", "<< /Title (B) /Parent 3 0 R /Next 5 0 R >>"]), ["A", "B"]);
    // A child that leads back to its ancestor.
    assert_eq!(listed(&["<< /Title (A) /Parent 3 0 R /First 6 0 R >>", "<< /Title (B) /Parent 5 0 R /First 5 0 R >>"]), ["A[B]"]);
    // A broken `/Next` (a missing object, then a number) and a missing first bookmark.
    assert_eq!(listed(&["<< /Title (A) /Parent 3 0 R /Next 99 0 R >>"]), ["A"]);
    assert_eq!(listed(&["<< /Title (A) /Parent 3 0 R /Next 6 0 R >>", "42"]), ["A"]);
    assert!(listed(&[]).is_empty());
}

#[test]
fn bookmark_pages_list_every_bookmark_once() {
    let d = open(flat_outline(250));
    let mut seen = Vec::new();
    let mut offset = 0;
    while let Some(next) = {
        let page = crate::bookmark_page(&d, offset, 100);
        assert!(page.bookmarks.len() <= 100);
        seen.extend(page.bookmarks.iter().map(|b| b.title.clone()));
        page.next
    } {
        assert_eq!(next, seen.len(), "a page ends where the next one starts");
        offset = next;
    }
    assert_eq!(seen.len(), 250);
    assert_eq!(seen.last().map(String::as_str), Some("Item 249"));

    let big = open(flat_outline(100_005));
    let first = crate::bookmark_page(&big, 0, 100);
    assert_eq!((first.bookmarks.len(), first.next), (100, Some(100)));
    let late = crate::bookmark_page(&big, 100_000, 100);
    assert_eq!(
        late.bookmarks.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(),
        ["Item 100000", "Item 100001", "Item 100002", "Item 100003", "Item 100004"]
    );
    assert_eq!((late.bookmarks[4].path.clone(), late.next), (vec![100_004], None));
}

#[test]
fn bookmarks_name_their_pages_through_every_kind_of_destination() {
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R /Names << /Dests 9 0 R >> >>".into(),
        "<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Outlines /First 6 0 R /Last 11 0 R /Count 6 >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Title (Named) /Parent 3 0 R /Next 7 0 R /Dest (intro) >>".into(),
        "<< /Title (GoTo) /Parent 3 0 R /Prev 6 0 R /Next 8 0 R /A << /S /GoTo /D [5 0 R /Fit] >> >>".into(),
        "<< /Title (Appendix) /Parent 3 0 R /Prev 7 0 R /Next 10 0 R /Dest /appendix >>".into(),
        "<< /Names [(appendix) 12 0 R (intro) [4 0 R /Fit]] >>".into(),
        "<< /Title <FEFF00480069> /Parent 3 0 R /Prev 8 0 R /Next 11 0 R /Dest [1 /Fit] >>".into(),
        "<< /Title (  Padded  ) /Parent 3 0 R /Prev 10 0 R /Dest [2 0 R /Fit] >>".into(),
        "<< /D [5 0 R /Fit] >>".into(),
    ];
    let d = open(build(&objs, "/Root 1 0 R"));
    let listed: Vec<String> = crate::bookmarks(&d).iter().map(|b| format!("{}@{:?}", b.title, b.page)).collect();
    assert_eq!(listed, ["Named@Some(0)", "GoTo@Some(1)", "Appendix@Some(1)", "Hi@Some(1)", "Padded@None"]);
}

// ---- page labels (M4.4) ------------------------------------------------------------------------

#[test]
fn number_pages_like_acrobat() {
    use crate::LabelStyle::*;
    let mut d = doc_a();
    crate::insert_blank_page(&mut d, 3, 300.0, 400.0).unwrap();
    crate::insert_blank_page(&mut d, 4, 300.0, 400.0).unwrap();
    crate::insert_blank_page(&mut d, 5, 300.0, 400.0).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["1", "2", "3", "4", "5", "6"]);
    // Front matter in roman numerals; later pages keep their labels.
    crate::number_pages(&mut d, 0, 1, LowerRoman, "", 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["i", "ii", "3", "4", "5", "6"]);
    // The body restarts at 1, an appendix gets a prefix.
    crate::number_pages(&mut d, 2, 5, Decimal, "", 1).unwrap();
    crate::number_pages(&mut d, 4, 5, Decimal, "A-", 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["i", "ii", "1", "2", "A-1", "A-2"]);
    assert_eq!(crate::page_label_ranges(&d).len(), 3, "continuations are merged");
    // Relabelling the middle keeps the pages after it.
    crate::number_pages(&mut d, 3, 3, UpperAlpha, "Fig ", 3).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["i", "ii", "1", "Fig C", "A-1", "A-2"]);
    assert!(crate::number_pages(&mut d, 4, 9, Decimal, "", 1).is_err());
    // Survives a save and reopen; back to plain numbers removes /PageLabels.
    let back = full_roundtrip(&d);
    assert_eq!(crate::page_labels(&back).unwrap(), ["i", "ii", "1", "Fig C", "A-1", "A-2"]);
    crate::number_pages(&mut d, 0, 5, Decimal, "", 1).unwrap();
    assert!(crate::page_label_ranges(&d).is_empty());
    assert!(d.get(d.root().unwrap()).as_dict().unwrap().get(b"PageLabels").is_none());
}

/// Deleting pages moves the label ranges with the pages they label (#2091): later ranges start that
/// many pages earlier and keep numbering on, a range whose first page went starts on the next page
/// left, and a range left with no pages gives way to the one after it.
#[test]
fn deleting_pages_keeps_the_label_ranges_on_their_pages() {
    use crate::{LabelRange, LabelStyle::*};
    let ten = || {
        let mut d = doc_a();
        for at in 3..10 {
            crate::insert_blank_page(&mut d, at, 300.0, 400.0).unwrap();
        }
        let ranges = [
            LabelRange { start: 0, style: LowerRoman, prefix: String::new(), first: 1 },
            LabelRange { start: 3, style: Decimal, prefix: String::new(), first: 1 },
            LabelRange { start: 7, style: Decimal, prefix: "A-".into(), first: 1 },
        ];
        crate::set_page_label_ranges(&mut d, &ranges).unwrap();
        assert_eq!(crate::page_labels(&d).unwrap(), ["i", "ii", "iii", "1", "2", "3", "4", "A-1", "A-2", "A-3"]);
        d
    };
    for (deleted, expected) in [
        (&[4, 5][..], &["i", "ii", "iii", "1", "2", "A-1", "A-2", "A-3"][..]),
        (&[3], &["i", "ii", "iii", "1", "2", "3", "A-1", "A-2", "A-3"]),
        (&[3, 4, 5, 6], &["i", "ii", "iii", "A-1", "A-2", "A-3"]),
    ] {
        let mut d = ten();
        crate::delete_pages(&mut d, deleted).unwrap();
        assert_eq!(crate::page_labels(&d).unwrap(), expected, "after deleting {deleted:?}");
        assert_eq!(crate::page_labels(&full_roundtrip(&d)).unwrap(), expected, "reopened after deleting {deleted:?}");
    }
}

#[test]
fn page_label_limits_reject_unsupported_edits_before_mutating() {
    use crate::LabelStyle::*;
    for (style, prefix, first) in [
        (LowerRoman, String::new(), 1_025_000),
        (LowerAlpha, String::new(), 26 * 1024 + 1),
        (Decimal, "x".repeat(1025), 1),
        (Decimal, String::new(), u32::MAX),
    ] {
        let mut d = doc_a();
        let before = d.modified_objects();
        assert!(crate::number_pages(&mut d, 0, 1, style, &prefix, first).is_err());
        assert_eq!(d.modified_objects(), before, "a refused label edit must not change the tree");
        assert!(crate::page_label_ranges(&d).is_empty());
    }
}

#[test]
fn page_label_limits_reject_unsupported_ranges_before_mutating() {
    let mut d = doc_a();
    let before = d.modified_objects();
    let ranges = [crate::LabelRange { start: 0, style: crate::LabelStyle::LowerRoman, prefix: String::new(), first: 1_025_000 }];
    assert!(crate::set_page_label_ranges(&mut d, &ranges).is_err());
    assert_eq!(d.modified_objects(), before);
}

#[test]
fn page_label_limits_reject_cyclic_old_trees_without_rewriting() {
    let mut d = doc_a();
    let tree = d.add(Object::Null);
    let mut dict = Dict::new();
    dict.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(tree)]));
    d.set(tree, Object::Dict(dict));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let before = d.modified_objects();
    assert!(crate::page_labels(&d).is_err());
    assert!(crate::page_label_ranges(&d).is_empty(), "compatibility query has documented empty fallback");
    assert!(crate::number_pages(&mut d, 0, 1, crate::LabelStyle::Decimal, "", 1).is_err());
    assert_eq!(d.modified_objects(), before);
    assert_eq!(d.get(d.root().unwrap()).as_dict().unwrap().reference(b"PageLabels"), Some(tree));
}

#[test]
fn page_label_limits_reject_oversized_old_arrays_without_rewriting() {
    let mut d = doc_a();
    let mut dict = Dict::new();
    dict.set(b"Kids".to_vec(), Object::Array(vec![Object::Null; pdfcraft_cos::page_labels::MAX_LABEL_TREE_WORK + 1]));
    let tree = d.add(Object::Dict(dict));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let before = d.modified_objects();
    assert!(crate::page_labels(&d).is_err());
    assert!(crate::number_pages(&mut d, 0, 1, crate::LabelStyle::Decimal, "", 1).is_err());
    assert_eq!(d.modified_objects(), before);
}

#[test]
fn page_label_limits_keep_preview_fallback_and_valid_boundary_edits() {
    use crate::LabelStyle::*;
    assert_eq!(LowerRoman.format(u32::MAX), u32::MAX.to_string());
    assert_eq!(LowerAlpha.format(u32::MAX), u32::MAX.to_string());
    let mut d = doc_a();
    crate::number_pages(&mut d, 0, 2, None, &"x".repeat(1024), 1).unwrap();
    assert!(crate::page_labels(&d).unwrap().iter().all(|s| s.len() == 1024));
}

#[test]
fn page_label_limits_preserve_prefix_only_max_first_and_aliases() {
    let mut d = doc_a();
    crate::number_pages(&mut d, 0, 2, crate::LabelStyle::None, "prefix", u32::MAX).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["prefix", "prefix", "prefix"]);
    crate::number_pages(&mut d, 0, 0, crate::LabelStyle::Decimal, "", 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["1", "prefix", "prefix"]);

    let prefix = d.add(Object::String(PdfString::text("alias")));
    let prefix_alias = d.add(Object::Ref(prefix));
    let mut spec = Dict::new();
    spec.set(b"P".to_vec(), Object::Ref(prefix_alias));
    spec.set(b"St".to_vec(), Object::Int(i64::MAX)); // unused without /S
    let spec = d.add(Object::Dict(spec));
    let spec_alias = d.add(Object::Ref(spec));
    let nums = d.add(Object::Array(vec![Object::Int(0), Object::Ref(spec_alias)]));
    let nums_alias = d.add(Object::Ref(nums));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Ref(nums_alias));
    let tree = d.add(Object::Dict(tree));
    let tree_alias = d.add(Object::Ref(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree_alias))).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["alias", "alias", "alias"]);
    crate::number_pages(&mut d, 0, 0, crate::LabelStyle::Decimal, "", 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["1", "alias", "alias"]);
}

#[test]
fn page_label_limits_refuse_reference_only_cycles_without_rewriting() {
    let mut d = doc_a();
    let tree = d.add(Object::Null);
    d.set(tree, Object::Ref(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let before = d.modified_objects();
    assert!(crate::page_labels(&d).is_err());
    assert!(crate::number_pages(&mut d, 0, 1, crate::LabelStyle::Decimal, "", 1).is_err());
    assert_eq!(d.modified_objects(), before);
}

#[test]
fn page_label_limits_validate_the_same_ranges_that_are_written() {
    use crate::{LabelRange, LabelStyle};
    let mut d = doc_a();
    let duplicates = [
        LabelRange { start: 0, style: LabelStyle::None, prefix: "ok".into(), first: 1 },
        LabelRange { start: 0, style: LabelStyle::None, prefix: "x".repeat(1024), first: 1 },
    ];
    let before = d.modified_objects();
    assert!(crate::set_page_label_ranges(&mut d, &duplicates).unwrap_err().to_string().contains("unique"));
    assert_eq!(d.modified_objects(), before);
    let unsorted = [
        LabelRange { start: 1, style: LabelStyle::UpperAlpha, prefix: "".into(), first: 1 },
        LabelRange { start: 0, style: LabelStyle::LowerRoman, prefix: "".into(), first: 4 },
    ];
    crate::set_page_label_ranges(&mut d, &unsorted).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["iv", "A", "B"]);
    let catalog = d.get(d.root().unwrap());
    let tree = d.resolve(catalog.as_dict().unwrap().get(b"PageLabels").unwrap());
    let nums = tree.as_dict().unwrap().get(b"Nums").unwrap().as_array().unwrap();
    assert_eq!(nums[0].as_int(), Some(0));
    assert_eq!(nums[2].as_int(), Some(1));
}

/// Each label dictionary is its own object, referenced from `/Nums`, as Acrobat writes them. A
/// direct dictionary is just as valid, but some readers (pdfannots2json, through unipdf) skip it
/// and fall back to physical page numbers (#1763).
#[test]
fn page_labels_are_written_as_indirect_dictionaries() {
    use crate::{LabelRange, LabelStyle};
    let mut d = doc_a();
    let ranges = [
        LabelRange { start: 0, style: LabelStyle::LowerRoman, prefix: String::new(), first: 1 },
        LabelRange { start: 1, style: LabelStyle::Decimal, prefix: "A-".into(), first: 324 },
        LabelRange { start: 2, style: LabelStyle::None, prefix: "Cover".into(), first: 1 },
    ];
    crate::set_page_label_ranges(&mut d, &ranges).unwrap();
    let back = full_roundtrip(&d);
    let catalog = back.get(back.root().unwrap());
    let tree = back.resolve(catalog.as_dict().unwrap().get(b"PageLabels").unwrap());
    let nums = tree.as_dict().unwrap().get(b"Nums").unwrap().as_array().unwrap();
    assert_eq!(nums.len(), 6);
    let expected = [(0, Some(b"r".as_slice()), None, None), (1, Some(b"D".as_slice()), Some("A-"), Some(324)), (2, None, Some("Cover"), None)];
    for (pair, (start, style, prefix, first)) in nums.chunks(2).zip(expected) {
        assert_eq!(pair[0].as_int(), Some(start));
        let Object::Ref(r) = &pair[1] else { panic!("the label of page {start} is not an indirect object: {:?}", pair[1]) };
        let spec = back.get(*r);
        let spec = spec.as_dict().unwrap();
        assert_eq!(spec.name(b"S"), style);
        assert_eq!(spec.get(b"P").and_then(|p| p.as_string()).map(|p| p.to_text()).as_deref(), prefix);
        assert_eq!(spec.int(b"St"), first);
    }
    assert_eq!(crate::page_label_ranges(&back), ranges);
    assert_eq!(crate::page_labels(&back).unwrap(), ["i", "A-324", "Cover"]);
}

#[test]
fn page_label_duplicate_starts_keep_the_later_range() {
    let mut d = doc_a();
    let mut a = Dict::new();
    a.set(b"P".to_vec(), Object::String(PdfString::text("first")));
    let mut b = Dict::new();
    b.set(b"P".to_vec(), Object::String(PdfString::text("last")));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(a), Object::Int(0), Object::Dict(b)]));
    let tree = d.add(Object::Dict(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap()[0], "last");
    crate::number_pages(&mut d, 0, 0, crate::LabelStyle::Decimal, "", 1).unwrap();
}

/// Two kids naming objects the file doesn't have both resolve to a fresh `Null`; they are not
/// the same node, so the tree isn't refused as cyclic.
#[test]
fn page_label_missing_kids_are_not_a_cycle() {
    let mut d = doc_a();
    let mut leaf = Dict::new();
    let mut spec = Dict::new();
    spec.set(b"P".to_vec(), Object::String(PdfString::text("p-")));
    spec.set(b"S".to_vec(), Object::name("D"));
    leaf.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(spec)]));
    let leaf = d.add(Object::Dict(leaf));
    let mut tree = Dict::new();
    let missing = |n| Object::Ref(pdfcraft_cos::ObjRef { num: n, generation: 0 });
    tree.set(b"Kids".to_vec(), Object::Array(vec![missing(9_000), missing(9_001), Object::Ref(leaf)]));
    let tree = d.add(Object::Dict(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap()[0], "p-1");
}

/// A range whose labels would be too long shows physical page numbers; other ranges keep
/// their labels.
#[test]
fn page_label_too_long_range_falls_back_alone() {
    let mut d = doc_a();
    let mut ok = Dict::new();
    ok.set(b"P".to_vec(), Object::String(PdfString::text("ok-")));
    ok.set(b"S".to_vec(), Object::name("D"));
    let mut huge = Dict::new();
    huge.set(b"S".to_vec(), Object::name("r"));
    huge.set(b"St".to_vec(), Object::Int(1_025_000));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(ok), Object::Int(1), Object::Dict(huge)]));
    let tree = d.add(Object::Dict(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let labels = crate::page_labels(&d).unwrap();
    assert_eq!(labels[0], "ok-1");
    assert_eq!(labels[1], "2");
}

/// A document whose page tree is one flat node over `pages` leaves. The label tests below need
/// more pages than `doc_a` has.
fn flat_doc(pages: usize) -> Document {
    let kids = (0..pages).map(|i| format!("{} 0 R", i + 3)).collect::<Vec<_>>().join(" ");
    let mut objs = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(), format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>")];
    objs.extend((0..pages).map(|_| "<< /Type /Page /Parent 2 0 R >>".to_string()));
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    open(out)
}

/// Each label is valid (1,004 bytes), but the 5,000 of them total about 5 MB, past the old 4 MiB
/// limit on the whole sequence. Every label is readable.
#[test]
fn page_labels_past_four_mib_in_total_stay_readable() {
    let mut d = flat_doc(5_000);
    let mut spec = Dict::new();
    spec.set(b"P".to_vec(), Object::String(PdfString::text(&"x".repeat(1000))));
    spec.set(b"S".to_vec(), Object::name("D"));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(spec)]));
    let tree = d.add(Object::Dict(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let labels = crate::page_labels(&d).unwrap();
    assert_eq!(labels.len(), 5_000);
    assert_eq!(labels[0], format!("{}1", "x".repeat(1000)));
    assert_eq!(labels[4_999], format!("{}5000", "x".repeat(1000)));
}

/// The same 5,000 labels written by an edit: the edit is accepted and reads back.
#[test]
fn number_pages_past_four_mib_in_total_is_accepted() {
    let mut d = flat_doc(5_000);
    crate::number_pages(&mut d, 0, 4_999, crate::LabelStyle::Decimal, &"x".repeat(1000), 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap()[4_999], format!("{}5000", "x".repeat(1000)));
}

/// Each range keeps its own decoded prefix, so a small file sharing one prefix object asks for a
/// copy per range (about 200 MiB at the tree-work limit). Past 4 MiB of decoded prefixes in all
/// the tree is refused, and so is relabelling it, which changes nothing.
#[test]
fn page_label_prefixes_are_bounded_in_total() {
    // `ranges` ranges naming one shared prefix object of `prefix` bytes, without a style.
    let shared = |ranges: i64, prefix: usize| {
        let mut d = doc_a();
        let prefix = d.add(Object::String(PdfString::text(&"x".repeat(prefix))));
        let mut spec = Dict::new();
        spec.set(b"P".to_vec(), Object::Ref(prefix));
        let spec = d.add(Object::Dict(spec));
        let mut tree = Dict::new();
        tree.set(b"Nums".to_vec(), Object::Array((0..ranges).flat_map(|i| [Object::Int(i), Object::Ref(spec)]).collect()));
        let tree = d.add(Object::Dict(tree));
        d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
        d
    };
    let mut d = shared(5_000, 1_000);
    let e = crate::page_labels(&d).unwrap_err().to_string();
    assert!(e.contains("decoded prefixes exceed 4 MiB in total"), "{e}");
    let before = d.modified_objects();
    assert!(crate::number_pages(&mut d, 0, 1, crate::LabelStyle::Decimal, "", 1).is_err());
    assert_eq!(d.modified_objects(), before, "a refused relabelling changes nothing");
    // Exactly 4 MiB of prefixes is still read.
    let max = pdfcraft_cos::page_labels::MAX_PREFIX_TOTAL_BYTES / 1024;
    assert!(crate::page_labels(&shared(max as i64, 1024)).unwrap().iter().all(|l| l.len() == 1024));
}

#[test]
fn page_boxes_default_inherit_and_set() {
    let mut doc = open(fixture());
    let b = page_boxes(&doc).unwrap();
    // Pages 1–2 inherit a 300×400 media box; page 3 has its own 500×500. Crop defaults to media.
    assert_eq!(b[0][0], [0.0, 0.0, 300.0, 400.0]);
    assert_eq!(b[2][1], [0.0, 0.0, 500.0, 500.0]);
    set_page_box(&mut doc, &[0, 2], PageBox::Crop, BoxSpec::Margins([10.0, 20.0, 30.0, 40.0])).unwrap();
    set_page_box(&mut doc, &[1], PageBox::Trim, BoxSpec::Rect([-50.0, 50.0, 100.0, 1000.0])).unwrap();
    let doc = open(write_incremental(&doc, &SaveOptions::default()).unwrap());
    let b = page_boxes(&doc).unwrap();
    assert_eq!(b[0][1], [10.0, 20.0, 270.0, 360.0]);
    assert_eq!(b[2][1], [10.0, 20.0, 470.0, 460.0], "margins apply to each page's own media box");
    assert_eq!(b[1][3], [0.0, 50.0, 100.0, 400.0], "clipped to the media box");
    assert_eq!(b[1][4], b[1][1], "art box defaults to the crop box");
    // Removing goes back to the default; impossible requests change nothing.
    let mut doc = doc;
    set_page_box(&mut doc, &[0], PageBox::Crop, BoxSpec::Remove).unwrap();
    assert_eq!(page_boxes(&doc).unwrap()[0][1], [0.0, 0.0, 300.0, 400.0]);
    let before = page_boxes(&doc).unwrap();
    assert!(matches!(set_page_box(&mut doc, &[0, 1], PageBox::Crop, BoxSpec::Margins([200.0, 0.0, 200.0, 0.0])), Err(OrganizeError::InvalidBox(_))));
    assert!(matches!(set_page_box(&mut doc, &[0], PageBox::Media, BoxSpec::Remove), Err(OrganizeError::InvalidBox(_))));
    assert_eq!(page_boxes(&doc).unwrap(), before);
    assert_eq!(set_page_box(&mut doc, &[7], PageBox::Crop, BoxSpec::Remove), Err(OrganizeError::NoSuchPage(7)));
}

#[test]
fn duplicate_pages_inserts_copies_after_the_last() {
    let mut doc = open(fixture());
    duplicate_pages(&mut doc, &[0, 1], None).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 1", "Page 2", "Page 3"]);
}

#[test]
fn duplicate_pages_at_inserts_the_copies_there() {
    let mut doc = open(fixture());
    duplicate_pages(&mut doc, &[2], Some(0)).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 2", "Page 3"]);
    // An index past the end lands at the end.
    let mut doc = open(fixture());
    duplicate_pages(&mut doc, &[0], Some(99)).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 3", "Page 1"]);
}

#[test]
fn replace_pages_swaps_content_but_keeps_annotations() {
    // Page 2 of the target gets an annotation; replacing its content keeps it.
    let mut doc = open(fixture());
    let p2 = pages(&doc).unwrap()[1].obj;
    let annot = doc.add(Object::Dict({
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("Annot"));
        d.set(b"Subtype".to_vec(), Object::name("Square"));
        d.set(b"Rect".to_vec(), Object::Array(vec![0.into(), 0.into(), 10.into(), 10.into()]));
        d
    }));
    doc.update_dict(p2, |d| d.set(b"Annots".to_vec(), Object::Array(vec![Object::Ref(annot)]))).unwrap();
    let src = open(fixture());
    replace_pages(&mut doc, &[1], &src, &[2]).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(labels(&doc), ["Page 1", "Page 3", "Page 3"]);
    let p2 = doc.get(pages(&doc).unwrap()[1].obj);
    assert!(p2.as_dict().unwrap().contains(b"Annots"), "the original page's annotations stay");
    let mut doc = doc;
    assert!(replace_pages(&mut doc, &[0, 1], &src, &[0]).is_err(), "counts must match");
}

#[test]
fn a_page_becomes_a_form_xobject_upright() {
    let src = open(fixture());
    let mut dst = Document::new_empty();
    // Page 1 inherits a 300 × 400 media box, rotation 90 and its font resources.
    let (r, size) = crate::page_as_form(&mut dst, &src, 0).unwrap();
    assert_eq!(size, (400.0, 300.0), "displayed size of a quarter-turned page");
    let obj = dst.get(r);
    let Object::Stream(s) = &*obj else { panic!("a stream") };
    let m: Vec<f64> = s.dict.get(b"Matrix").unwrap().as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert_eq!(m, vec![0.0, -1.0, 1.0, 0.0, 0.0, 300.0]);
    assert!(String::from_utf8_lossy(&s.decoded().unwrap()).contains("(Page 1) Tj"));
    let res = s.dict.get(b"Resources").and_then(|r| dst.resolve(r).as_dict().cloned()).unwrap();
    assert!(res.get(b"Font").is_some(), "inherited resources come along");
    assert!(crate::page_as_form(&mut dst, &src, 7).is_err());
}

#[test]
fn initial_view_round_trips_and_keeps_scripts() {
    use crate::view::{Layout, Magnification, Navigation};
    let mut doc = open(fixture());
    assert_eq!(crate::initial_view(&doc), crate::InitialView::default());
    let v = crate::InitialView {
        navigation: Navigation::Bookmarks,
        layout: Layout::TwoUpContinuousCoverPage,
        magnification: Magnification::Percent(150.0),
        page: 2,
        fit_window: true,
        display_title: true,
        hide_toolbar: true,
        language: Some("fr-FR".into()),
        right_to_left: true,
        ..Default::default()
    };
    crate::set_initial_view(&mut doc, &v).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(crate::initial_view(&doc), v);
    // A script opening action survives a change that needs no destination.
    let mut doc = doc;
    let root = doc.root().unwrap();
    let mut js = pdfcraft_cos::Dict::new();
    js.set(b"S".to_vec(), Object::name("JavaScript"));
    js.set(b"JS".to_vec(), pdfcraft_cos::PdfString::text("app.alert('hi')"));
    doc.update_dict(root, |c| c.set(b"OpenAction".to_vec(), Object::Dict(js))).unwrap();
    crate::set_initial_view(&mut doc, &crate::InitialView { layout: Layout::SinglePage, ..Default::default() }).unwrap();
    assert!(doc.get(root).as_dict().unwrap().get(b"OpenAction").and_then(|o| o.as_dict()).is_some_and(|d| d.name(b"S") == Some(b"JavaScript")));
    assert!(crate::set_initial_view(&mut doc, &crate::InitialView { page: 9, ..Default::default() }).is_err());
}

#[test]
fn an_unchanged_opening_destination_keeps_its_position_and_name() {
    use crate::view::{Magnification, Navigation};
    let page2 = Object::Ref(pdfcraft_cos::ObjRef::new(5, 0));
    let xyz = Object::Array(vec![page2.clone(), Object::name("XYZ"), Object::Int(100), Object::Int(500), Object::Int(2)]);
    let mut named = pdfcraft_cos::Dict::new();
    named.set(b"S".to_vec(), Object::name("GoTo"));
    named.set(b"D".to_vec(), Object::name("there"));
    let mut dests = pdfcraft_cos::Dict::new();
    dests.set(b"there".to_vec(), Object::Array(vec![page2, Object::name("Fit")]));
    let mut prefs = pdfcraft_cos::Dict::new();
    prefs.set(b"NonFullScreenPageMode".to_vec(), Object::name("UseOutlines"));
    for action in [xyz, Object::Dict(named)] {
        let mut doc = open(fixture());
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| {
            c.set(b"OpenAction".to_vec(), action.clone());
            c.set(b"Dests".to_vec(), Object::Dict(dests.clone()));
            c.set(b"PageMode".to_vec(), Object::name("FullScreen"));
            c.set(b"ViewerPreferences".to_vec(), Object::Dict(prefs.clone()));
        })
        .unwrap();
        // Another option changes: the opening action is written back as it was, and so is the
        // full-screen panel, which the dialog doesn't edit.
        let v = crate::initial_view(&doc);
        assert_eq!(v.non_full_screen, Navigation::Bookmarks);
        crate::set_initial_view(&mut doc, &crate::InitialView { hide_toolbar: true, ..v.clone() }).unwrap();
        let catalog = doc.get(root).as_dict().cloned().unwrap();
        assert_eq!(catalog.get(b"OpenAction"), Some(&action));
        let doc = save_and_reopen(&doc);
        assert_eq!(crate::initial_view(&doc), crate::InitialView { hide_toolbar: true, ..v.clone() });
        // A changed magnification writes a destination of its own.
        let mut doc = doc;
        crate::set_initial_view(&mut doc, &crate::InitialView { magnification: Magnification::FitWidth, ..v }).unwrap();
        let catalog = doc.get(root).as_dict().cloned().unwrap();
        assert_ne!(catalog.get(b"OpenAction"), Some(&action));
        assert_eq!(crate::initial_view(&doc).magnification, Magnification::FitWidth);
    }
}

/// A document whose `/OpenAction` is `action`, with the named destination `there` (page 3 at
/// 200%) in the catalog `/Dests` and `here` (page 1, fit) in a `/Names /Dests` tree.
fn doc_opening_at(action: Object) -> Document {
    let page1 = Object::Ref(pdfcraft_cos::ObjRef::new(4, 0));
    let page3 = Object::Ref(pdfcraft_cos::ObjRef::new(6, 0));
    let mut dests = pdfcraft_cos::Dict::new();
    dests.set(b"there".to_vec(), Object::Array(vec![page3, Object::name("XYZ"), Object::Null, Object::Null, Object::Int(2)]));
    let mut tree = pdfcraft_cos::Dict::new();
    tree.set(b"Names".to_vec(), Object::Array(vec![Object::String(PdfString::text("here")), Object::Array(vec![page1, Object::name("Fit")])]));
    let mut names = pdfcraft_cos::Dict::new();
    names.set(b"Dests".to_vec(), Object::Dict(tree));
    let mut doc = open(fixture());
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| {
        c.set(b"OpenAction".to_vec(), action);
        c.set(b"Dests".to_vec(), Object::Dict(dests));
        c.set(b"Names".to_vec(), Object::Dict(names));
    })
    .unwrap();
    doc
}

fn open_action(doc: &Document) -> Option<Object> {
    doc.get(doc.root().unwrap()).as_dict().and_then(|c| c.get(b"OpenAction").cloned())
}

/// ISO 32000-2 §12.3.2.4: a named destination stands for the explicit one it names. Choosing
/// page 1 in Document Properties on a document that opens at a named destination elsewhere
/// writes a destination to page 1; it used to read the name as page 1 and change nothing.
#[test]
fn choosing_page_one_replaces_a_named_opening_destination() {
    use crate::view::Magnification;
    let page1 = Object::Ref(pdfcraft_cos::ObjRef::new(4, 0));
    let mut goto = pdfcraft_cos::Dict::new();
    goto.set(b"S".to_vec(), Object::name("GoTo"));
    goto.set(b"D".to_vec(), Object::name("there"));
    for action in [Object::Dict(goto), Object::name("there"), Object::String(PdfString::text("there"))] {
        let mut doc = doc_opening_at(action.clone());
        // The dialog shows where the document really opens.
        let v = crate::initial_view(&doc);
        assert_eq!((v.page, v.magnification), (2, Magnification::Percent(200.0)), "{action:?}");
        // Page 1 at the same magnification: an explicit destination to page 1.
        crate::set_initial_view(&mut doc, &crate::InitialView { page: 0, ..v.clone() }).unwrap();
        let written = Object::Array(vec![page1.clone(), Object::name("XYZ"), Object::Null, Object::Null, Object::Real(2.0)]);
        assert_eq!(open_action(&doc), Some(written), "{action:?}");
        let reopened = save_and_reopen(&doc);
        assert_eq!(crate::initial_view(&reopened), crate::InitialView { page: 0, ..v.clone() }, "{action:?}");
        // Page 1 at the default magnification: no opening action, so the first page opens.
        let mut doc = doc_opening_at(action.clone());
        crate::set_initial_view(&mut doc, &crate::InitialView { page: 0, magnification: Magnification::Default, ..v }).unwrap();
        assert_eq!(open_action(&doc), None, "{action:?}");
        assert_eq!(crate::initial_view(&save_and_reopen(&doc)).page, 0, "{action:?}");
    }
}

/// A named opening destination that already goes to the chosen page and magnification is kept,
/// and so is one that names nothing (it reads as page 1) until another page is chosen.
#[test]
fn a_named_opening_destination_is_kept_while_it_still_holds() {
    use crate::view::Magnification;
    let page2 = Object::Ref(pdfcraft_cos::ObjRef::new(5, 0));
    for (action, magnification) in
        [(Object::String(PdfString::text("here")), Magnification::FitPage), (Object::name("nowhere"), Magnification::Default)]
    {
        let mut doc = doc_opening_at(action.clone());
        let v = crate::initial_view(&doc);
        assert_eq!((v.page, v.magnification), (0, magnification), "{action:?}");
        crate::set_initial_view(&mut doc, &crate::InitialView { hide_toolbar: true, ..v.clone() }).unwrap();
        assert_eq!(open_action(&doc), Some(action.clone()), "{action:?}");
        crate::set_initial_view(&mut doc, &crate::InitialView { page: 1, ..v.clone() }).unwrap();
        let kind = if magnification == Magnification::FitPage { "Fit" } else { "XYZ" };
        assert_eq!(
            open_action(&doc).as_ref().and_then(Object::as_array).map(|a| (a.first().cloned(), a.get(1).cloned())),
            Some((Some(page2.clone()), Some(Object::name(kind)))),
            "{action:?}"
        );
    }
}

// ── Split / Extract keep only the resources the part draws (#204) ──────────────────────────────

/// A content stream with no text operators (an image draw).
fn raw(content: &str) -> String {
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len())
}

fn image(width: u32, height: u32, payload: usize) -> String {
    let data = "x".repeat(payload);
    format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {payload} >>\nstream\n{data}\nendstream"
    )
}

/// Document D: two pages sharing one resources dictionary with two image XObjects; each page
/// draws a different one — the shared-dictionary shape of #204.
fn doc_images() -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),                                                        // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] /Resources 7 0 R >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".into(),                                          // 3
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".into(),                                          // 4
        raw("/ImA Do"),                                                                                    // 5
        raw("/ImB Do"),                                                                                    // 6
        "<< /XObject << /ImA 8 0 R /ImB 9 0 R >> >>".into(),                                               // 7
        image(1, 1, 1),                                                                                    // 8
        image(1, 1, 2000),                                                                                 // 9
    ];
    open(build(&b, "/Root 1 0 R"))
}

fn xobject_names(doc: &Document, page: usize) -> Vec<Vec<u8>> {
    let resources = page_dict(doc, page).get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).expect("page resources");
    let xobjects = resources.get(b"XObject").map(|x| doc.resolve(x)).and_then(|x| x.as_dict().cloned()).expect("xobject dict");
    xobjects.iter().map(|(k, _)| k.clone()).collect()
}

#[test]
fn extract_keeps_only_the_images_the_part_draws() {
    let src = doc_images();
    let whole = write_full(&src, &SaveOptions::default()).expect("writes").len();
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImA"), "the drawn image stays");
    assert!(!names.iter().any(|n| n.as_slice() == b"ImB"), "an image no page of the part draws must go");
    let part_bytes = write_full(&part, &SaveOptions::default()).expect("writes").len();
    assert!(part_bytes + 1000 < whole, "the part must not carry the other page's image: {part_bytes} vs {whole}");
}

#[test]
fn extract_keeps_the_union_when_the_part_has_both_pages() {
    let part = full_roundtrip(&extract_pages(&doc_images(), &[0, 1]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImA"));
    assert!(names.iter().any(|n| n.as_slice() == b"ImB"));
}

/// Like `doc_images`, but the second page's content can't be decoded and the first page also
/// lists a font: pruning must not guess what the unreadable page draws.
fn doc_images_with(page2_content: &str, font: &str) -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] /Resources 7 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".into(),
        raw("/ImA Do"),
        page2_content.into(),
        format!("<< /XObject << /ImA 8 0 R /ImB 9 0 R >> {font} >>"),
        image(1, 1, 1),
        image(1, 1, 2000),
    ];
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn extract_keeps_images_an_undecodable_page_might_draw() {
    let bogus = "<< /Length 7 /Filter /NoSuchFilter >>\nstream\n/ImB Do\nendstream";
    let part = full_roundtrip(&extract_pages(&doc_images_with(bogus, ""), &[0, 1]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "an image the unreadable page may draw must stay");
}

#[test]
fn extract_keeps_images_type3_glyphs_may_draw() {
    // A Type 3 font without its own /Resources draws through the page's: its glyphs could `Do`.
    let font = "/Font << /T3 << /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /CharProcs << >> /Encoding << /Differences [] >> /FirstChar 0 /LastChar 0 /Widths [0] >> >>";
    let part = full_roundtrip(&extract_pages(&doc_images_with(&raw("/ImB Do"), font), &[0]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "pruning must not run through Type 3 resources");
}

/// The font has its own resources, but its XObject dictionary is also used by the page.
/// Looking only at page `Do` operators must not discard the image painted by a glyph.
fn doc_type3_shared_xobjects(font_resources: &str) -> Document {
    let objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources 5 0 R >>".into(),
        raw("q 30 0 0 30 10 10 cm /ImA Do Q BT /T3 50 Tf 50 80 Td (A) Tj ET"),
        "<< /XObject 6 0 R /Font << /T3 7 0 R >> >>".into(),
        "<< /ImA 9 0 R /ImB 10 0 R /Unused 11 0 R >>".into(),
        format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /CharProcs << /A 8 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [1] /Resources {font_resources} >>"
        ),
        raw("1 0 0 0 1 1 d1 /ImB Do"),
        image(1, 1, 1),
        image(1, 1, 1),
        image(1, 1, 2000),
    ];
    open(build(&objects, "/Root 1 0 R"))
}

#[test]
fn extract_keeps_shared_xobjects_used_by_type3_own_resources() {
    for resources in ["<< /XObject 6 0 R >>", "5 0 R"] {
        let src = doc_type3_shared_xobjects(resources);
        let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
        let names = xobject_names(&part, 0);
        assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "the Type 3 glyph's image must survive extraction: {resources}");
    }
}

#[test]
fn extract_still_prunes_page_xobjects_when_type3_resources_are_separate() {
    let src = doc_type3_shared_xobjects("<< /XObject << /ImB 10 0 R >> >>");
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert_eq!(xobject_names(&part, 0), [b"ImA".to_vec()], "only the page's unused entries are removed");
    let resources = part.resolve(page_dict(&part, 0).get(b"Resources").unwrap());
    let fonts = part.resolve(resources.as_dict().unwrap().get(b"Font").unwrap());
    let font = part.resolve(fonts.as_dict().unwrap().get(b"T3").unwrap());
    let resources = part.resolve(font.as_dict().unwrap().get(b"Resources").unwrap());
    let xobjects = part.resolve(resources.as_dict().unwrap().get(b"XObject").unwrap());
    assert!(xobjects.as_dict().unwrap().contains(b"ImB"), "the separate glyph resource survives the full rewrite");
}

fn doc_type3_glyph_form_with_shared_resources(cycle: bool) -> Document {
    let mut doc = doc_type3_shared_xobjects("<< >>");
    let mut resources = pdfcraft_cos::Dict::new();
    resources.set(b"XObject".to_vec(), Object::Ref(ObjRef::new(6, 0)));
    let mut form = pdfcraft_cos::Dict::new();
    form.set(b"Type".to_vec(), Object::name("XObject"));
    form.set(b"Subtype".to_vec(), Object::name("Form"));
    form.set(b"BBox".to_vec(), Object::Array(vec![Object::Int(0), Object::Int(0), Object::Int(1), Object::Int(1)]));
    form.set(b"Resources".to_vec(), Object::Dict(resources));
    let form = doc.add(Object::Stream(pdfcraft_cos::Stream::from_raw(form, b"/ImB Do".to_vec())));
    let mut xobjects = pdfcraft_cos::Dict::new();
    xobjects.set(b"Fm".to_vec(), Object::Ref(form));
    let mut resources = pdfcraft_cos::Dict::new();
    resources.set(b"XObject".to_vec(), Object::Dict(xobjects));
    doc.update_dict(ObjRef::new(7, 0), |font| font.set(b"Resources".to_vec(), Object::Dict(resources))).unwrap();
    doc.set(ObjRef::new(8, 0), Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"1 0 0 0 1 1 d1 /Fm Do".to_vec())));
    if cycle {
        doc.update_dict(ObjRef::new(6, 0), |xobjects| xobjects.set(b"Again".to_vec(), Object::Ref(form))).unwrap();
    }
    full_roundtrip(&doc)
}

#[test]
fn extract_keeps_shared_resources_reached_through_a_type3_glyph_form() {
    for cycle in [false, true] {
        let src = doc_type3_glyph_form_with_shared_resources(cycle);
        let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
        assert!(xobject_names(&part, 0).iter().any(|n| n.as_slice() == b"ImB"), "a glyph's nested form draws ImB; resource cycle: {cycle}");
    }
}

#[test]
fn extract_keeps_an_indirect_resource_owner_shared_with_a_type3_font() {
    let mut src = doc_type3_shared_xobjects("5 0 R");
    let xobjects = src.get(ObjRef::new(6, 0)).as_dict().unwrap().clone();
    src.update_dict(ObjRef::new(5, 0), |resources| resources.set(b"XObject".to_vec(), Object::Dict(xobjects))).unwrap();
    let src = full_roundtrip(&src);
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert!(xobject_names(&part, 0).iter().any(|n| n.as_slice() == b"ImB"), "the resource owner itself is shared, with inline XObjects");
}

#[test]
fn extract_skips_pruning_when_direct_glyph_resource_children_exceed_the_budget() {
    let mut src = doc_type3_shared_xobjects("<< /XObject << /ImB 10 0 R >> >>");
    let font = src.get(ObjRef::new(7, 0));
    let mut resources = font.as_dict().unwrap().get(b"Resources").unwrap().as_dict().unwrap().clone();
    let mut unknown = vec![Object::Null; 20_000];
    unknown.push(Object::Ref(ObjRef::new(6, 0)));
    resources.set(b"Unknown".to_vec(), Object::Array(unknown));
    src.update_dict(ObjRef::new(7, 0), |font| font.set(b"Resources".to_vec(), Object::Dict(resources))).unwrap();
    let src = full_roundtrip(&src);
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert_eq!(xobject_names(&part, 0), xobject_names(&src, 0), "an incomplete glyph resource walk cannot authorize pruning any owners");
}

fn doc_type3_resources_of_undecodable_content(form: bool) -> Document {
    let mut doc = doc_type3_shared_xobjects("<< /XObject 6 0 R >>");
    doc.update_dict(ObjRef::new(5, 0), |resources| {
        resources.remove(b"Font");
    })
    .unwrap();
    doc.set(ObjRef::new(4, 0), Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"/ImA Do".to_vec())));
    let mut fonts = pdfcraft_cos::Dict::new();
    fonts.set(b"T3".to_vec(), Object::Ref(ObjRef::new(7, 0)));
    let mut glyph_resources = pdfcraft_cos::Dict::new();
    glyph_resources.set(b"Font".to_vec(), Object::Dict(fonts));
    let mut unreadable = pdfcraft_cos::Dict::new();
    unreadable.set(b"Filter".to_vec(), Object::name("NoSuchFilter"));
    if form {
        unreadable.set(b"Type".to_vec(), Object::name("XObject"));
        unreadable.set(b"Subtype".to_vec(), Object::name("Form"));
        unreadable.set(b"BBox".to_vec(), Object::Array(vec![Object::Int(0), Object::Int(0), Object::Int(200), Object::Int(200)]));
        unreadable.set(b"Resources".to_vec(), Object::Dict(glyph_resources.clone()));
    }
    let unreadable = doc.add(Object::Stream(pdfcraft_cos::Stream::from_raw(unreadable, b"BT /T3 50 Tf 50 80 Td (A) Tj ET".to_vec())));
    let (contents, resources) = if form {
        let contents = doc.add(Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"/Fm Do".to_vec())));
        let mut xobjects = pdfcraft_cos::Dict::new();
        xobjects.set(b"Fm".to_vec(), Object::Ref(unreadable));
        let mut resources = pdfcraft_cos::Dict::new();
        resources.set(b"XObject".to_vec(), Object::Dict(xobjects));
        (contents, resources)
    } else {
        (unreadable, glyph_resources)
    };
    let mut page = pdfcraft_cos::Dict::new();
    page.set(b"Type".to_vec(), Object::name("Page"));
    page.set(b"Parent".to_vec(), Object::Ref(ObjRef::new(2, 0)));
    page.set(b"Contents".to_vec(), Object::Ref(contents));
    page.set(b"Resources".to_vec(), Object::Dict(resources));
    let page = doc.add(page);
    doc.update_dict(ObjRef::new(2, 0), |pages| {
        pages.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(ObjRef::new(3, 0)), Object::Ref(page)]));
        pages.set(b"Count".to_vec(), Object::Int(2));
    })
    .unwrap();
    full_roundtrip(&doc)
}

#[test]
fn extract_keeps_type3_glyph_resources_of_an_undecodable_page() {
    let part = full_roundtrip(&extract_pages(&doc_type3_resources_of_undecodable_content(false), &[0, 1]).unwrap());
    assert!(xobject_names(&part, 0).iter().any(|n| n.as_slice() == b"ImB"), "an unreadable page's font shares a different page's XObject dictionary");
    let contents = part.resolve(page_dict(&part, 1).get(b"Contents").unwrap());
    let Object::Stream(contents) = &*contents else { panic!("content stream") };
    assert_eq!(contents.dict.name(b"Filter"), Some(b"NoSuchFilter".as_slice()));
    assert_eq!(contents.raw.as_ref(), b"BT /T3 50 Tf 50 80 Td (A) Tj ET");
}

#[test]
fn extract_keeps_type3_glyph_resources_of_an_undecodable_form() {
    let part = full_roundtrip(&extract_pages(&doc_type3_resources_of_undecodable_content(true), &[0, 1]).unwrap());
    assert!(xobject_names(&part, 0).iter().any(|n| n.as_slice() == b"ImB"), "an unreadable form's font shares a different page's XObject dictionary");
}

#[test]
fn extract_still_prunes_separate_resources_when_many_pages_share_a_type3_font() {
    let mut src = doc_type3_shared_xobjects("<< /XObject << /ImB 10 0 R >> >>");
    src.set(ObjRef::new(4, 0), Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"/ImA Do".to_vec())));
    let page = src.get(ObjRef::new(3, 0)).as_dict().unwrap().clone();
    let mut kids = vec![Object::Ref(ObjRef::new(3, 0))];
    for _ in 1..4_000 {
        kids.push(Object::Ref(src.add(page.clone())));
    }
    src.update_dict(ObjRef::new(2, 0), |pages| {
        pages.set(b"Kids".to_vec(), Object::Array(kids));
        pages.set(b"Count".to_vec(), Object::Int(4_000));
    })
    .unwrap();
    let src = full_roundtrip(&src);
    let part = full_roundtrip(&extract_pages(&src, &(0..4_000).collect::<Vec<_>>()).unwrap());
    assert_eq!(xobject_names(&part, 0), [b"ImA".to_vec()], "one shared font's direct resource graph is protected once");
}

// Both sides of the comparison go through the same pdftoppm, so the assertion is about our two
// documents rather than about Poppler's output, and no Poppler version can change the verdict.
// It skips where pdftoppm is absent and fails where PDFCRAFT_ORACLES_REQUIRED is set.
#[test]
fn extract_type3_shared_xobjects_matches_poppler_pixels() {
    let pdftoppm = pdfcraft_oracle::or_skip!(pdfcraft_oracle::pdftoppm());
    let dir = std::env::temp_dir().join(format!("pdfcraft-type3-extract-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixtures = [
        doc_type3_shared_xobjects("<< /XObject 6 0 R >>"),
        doc_type3_shared_xobjects("5 0 R"),
        doc_type3_shared_xobjects("<< /XObject << /ImB 10 0 R >> >>"),
        doc_type3_glyph_form_with_shared_resources(false),
        doc_type3_glyph_form_with_shared_resources(true),
    ];
    for (i, src) in fixtures.into_iter().enumerate() {
        let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
        let render = |doc: &Document, name: &str| {
            let path = dir.join(format!("{i}-{name}.pdf"));
            std::fs::write(&path, write_full(doc, &SaveOptions::default()).unwrap()).unwrap();
            let output = pdftoppm.command().args(["-singlefile", "-r", "72", "-gray"]).arg(path).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
            output.stdout
        };
        let before = render(&src, "source");
        let after = render(&part, "extracted");
        assert!(before.starts_with(b"P5\n200 200\n255\n"));
        assert!(before.iter().filter(|&&v| v < 200).count() > 2500, "the source contains a drawn glyph");
        assert_eq!(after, before, "extraction must keep every rendered pixel in fixture {i}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// A page listed twice whose widget's field keeps `/Kids` as an array object of its own: the
/// second widget joins that array, and the field keeps every widget it had.
#[test]
fn a_repeated_widget_joins_an_indirect_kids_array() {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>".into(),                   // 3
        "<< /Type /Page /Parent 2 0 R /Annots [8 0 R] >>".into(),                   // 4
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [10 10 90 30] /P 3 0 R >>".into(), // 5
        "<< /FT /Tx /T (name) /V (Ada) /Kids 7 0 R >>".into(),                      // 6
        "[5 0 R 8 0 R]".into(),                                                     // 7
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [10 40 90 60] /P 4 0 R >>".into(), // 8
    ];
    let src = open(build(&b, "/Root 1 0 R"));
    let out = full_roundtrip(&extract_pages(&src, &[0, 0, 1]).unwrap());
    let form = out.resolve(catalog(&out).get(b"AcroForm").unwrap());
    let fields = form.as_dict().unwrap().get(b"Fields").and_then(Object::as_array).cloned().unwrap();
    assert_eq!(fields.len(), 1, "one field");
    let field = out.resolve(&fields[0]).as_dict().cloned().unwrap();
    let kids = out.resolve(field.get(b"Kids").unwrap()).as_array().cloned().unwrap();
    assert_eq!(kids.len(), 3, "both original widgets and the repeat's own: {kids:?}");
}

/// A one-page document whose form holds one text field named `name` (a widget that is its own
/// field, the smallest form a page can carry).
fn doc_with_field(name: &str) -> Document {
    doc_with_fields(&[name])
}

/// A one-page document whose form holds one widget field per name, in order.
fn doc_with_fields(names: &[&str]) -> Document {
    let first = 5; // object number of the first field
    let refs: Vec<String> = names.iter().enumerate().map(|(i, _)| format!("{} 0 R", first + i)).collect();
    let mut b: Vec<String> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{}] >> >>", refs.join(" ")), // 1
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),                // 2
        format!("<< /Type /Page /Parent 2 0 R /Annots [{}] >>", refs.join(" ")),                   // 3
        "<< /Title (form) >>".into(),                                                              // 4
    ];
    for name in names.iter() {
        b.push(format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T ({name}) /V (x) /Rect [60 60 200 80] /P 3 0 R >>")); // 5 + i
    }
    open(build(&b, "/Root 1 0 R /Info 4 0 R"))
}

/// The /T names of a document's /AcroForm /Fields, in order.
fn form_field_names(doc: &Document) -> Vec<String> {
    let Some(form) = catalog(doc).get(b"AcroForm").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()) else {
        return Vec::new();
    };
    form.get(b"Fields")
        .map(|f| doc.resolve(f))
        .and_then(|f| f.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|f| {
            doc.resolve(f)
                .as_dict()
                .and_then(|d| d.get(b"T").cloned())
                .map(|t| doc.resolve(&t))
                .and_then(|t| t.as_string().map(|s| String::from_utf8_lossy(&s.bytes).into_owned()))
                .unwrap_or_default()
        })
        .collect()
}

/// Two documents whose forms both name a field Text1 cannot just be put together: fields of
/// one fully-qualified name are one field, so the values would blur into it. The incoming
/// field is renamed to the first free variant, Text1-1.
#[test]
fn import_renames_a_colliding_field_name() {
    let mut dst = doc_with_field("Text1");
    let src = doc_with_field("Text1");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1", "Text1-1"]);
}

/// The rename takes the first free variant: with Text1 and Text1-1 already in the form, a
/// colliding Text1 becomes Text1-2 (not Text1-1, which would collide again).
#[test]
fn import_renames_to_the_first_free_variant() {
    let mut dst = doc_with_fields(&["Text1", "Text1-1"]);
    let src = doc_with_field("Text1");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1", "Text1-1", "Text1-2"]);
}

/// A one-page document whose form holds one text field whose `/T` is a UTF-16BE hex string —
/// the standard encoding for non-ASCII names — decoding to `name`.
fn doc_with_field_utf16(name: &str) -> Document {
    let t: String = std::iter::once(0xFEFFu16).chain(name.encode_utf16()).map(|u| format!("{u:04X}")).collect();
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>".into(),                   // 3
        "<< /Title (form) >>".into(),                                               // 4
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T <{t}> /V (x) /Rect [60 60 200 80] /P 3 0 R >>"), // 5
    ];
    open(build(&b, "/Root 1 0 R /Info 4 0 R"))
}

/// The decoded `/T` names of a document's /AcroForm /Fields, in order (whatever encoding each
/// was written in).
fn decoded_form_field_names(doc: &Document) -> Vec<String> {
    let Some(form) = catalog(doc).get(b"AcroForm").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()) else {
        return Vec::new();
    };
    form.get(b"Fields")
        .map(|f| doc.resolve(f))
        .and_then(|f| f.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|f| {
            doc.resolve(f)
                .as_dict()
                .and_then(|d| d.get(b"T").cloned())
                .map(|t| doc.resolve(&t))
                .and_then(|t| t.as_string().map(|s| s.to_text()))
                .unwrap_or_default()
        })
        .collect()
}

/// A field's name is its text, not the bytes it was written in: a destination field named in
/// PDFDocEncoding and an incoming UTF-16BE field that decode to the same name are one name,
/// so the incoming field is renamed out of the way (not silently blurred into the other).
#[test]
fn import_renames_by_decoded_name_whatever_its_encoding() {
    let mut dst = doc_with_field("Text1");
    let src = doc_with_field_utf16("Text1");
    let out = import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(out.renamed_fields, [("Text1".to_string(), "Text1-1".to_string())]);
    assert_eq!(decoded_form_field_names(&dst), ["Text1", "Text1-1"]);
}

/// Renaming a non-ASCII name keeps it readable: the new `/T` stores the new name as text
/// (UTF-16BE again), never as mojibake of the old bytes.
#[test]
fn a_renamed_non_ascii_name_stays_readable() {
    let mut dst = doc_with_field_utf16("Förm");
    let src = doc_with_field_utf16("Förm");
    let out = import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(out.renamed_fields, [("Förm".to_string(), "Förm-1".to_string())]);
    assert_eq!(decoded_form_field_names(&dst), ["Förm", "Förm-1"]);
}

/// A one-page document whose form holds one field tree: a root field `root` with one child
/// field `kid` (a merged field-and-widget, as page tools make them).
fn doc_with_tree(root: &str, kid: &str) -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Annots [6 0 R] >>".into(),                   // 3
        "<< /Title (form) >>".into(),                                               // 4
        format!("<< /FT /Tx /T ({root}) /Kids [6 0 R] >>"),                         // 5
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T ({kid}) /Parent 5 0 R /Rect [60 60 200 80] /P 3 0 R >>"), // 6
    ];
    open(build(&b, "/Root 1 0 R /Info 4 0 R"))
}

/// A field's name is its place in the tree, not just its own `/T`: an incoming field called
/// x.b collides with the child b of the destination's field x, and is renamed out of the way.
#[test]
fn import_renames_a_nested_field_whose_full_name_collides() {
    let mut dst = doc_with_tree("x", "b");
    let src = doc_with_field("x.b");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["x", "x.b-1"]);
}

/// The collision may sit below the top level: the destination holds a field literally named
/// a.b, and the incoming tree a ▸ b walks into it. Only the colliding child is renamed — the
/// incoming root a never collided, so it keeps its name.
#[test]
fn import_renames_the_colliding_child_not_the_whole_tree() {
    let mut dst = doc_with_field("a.b");
    let src = doc_with_tree("a", "b");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["a.b", "a"], "the incoming root keeps its name");
    // The incoming tree is the second /Fields entry; its child was renamed.
    let fields = catalog(&dst)
        .get(b"AcroForm")
        .map(|f| dst.resolve(f))
        .and_then(|f| f.as_dict().cloned())
        .unwrap()
        .get(b"Fields")
        .map(|l| dst.resolve(l))
        .and_then(|l| l.as_array().cloned())
        .unwrap();
    let kid = dst.resolve(&fields[1]).as_dict().unwrap().get(b"Kids").map(|k| dst.resolve(k)).and_then(|k| k.as_array().cloned()).unwrap();
    let kid_t = dst.resolve(&kid[0]).as_dict().unwrap().get(b"T").cloned().unwrap();
    let resolved = dst.resolve(&kid_t);
    let name = resolved.as_string().unwrap();
    assert_eq!(String::from_utf8_lossy(&name.bytes), "b-1", "the child's /T is renamed, the FQN follows the root");
}

/// A field name is taken even when it hangs only off a page's annotations: a widget that is
/// its own field, never registered in `/AcroForm /Fields`, still names a field, and an import
/// must not silently blur into it.
#[test]
fn import_renames_away_from_an_orphaned_widgets_name() {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),                                 // 1 (no /AcroForm)
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>".into(),                   // 3
        "<< /Title (orphan) >>".into(),                                             // 4
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Text1) /V (x) /Rect [60 60 200 80] /P 3 0 R >>".into(), // 5
    ];
    let mut dst = open(build(&b, "/Root 1 0 R /Info 4 0 R"));
    let src = doc_with_field("Text1");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1-1"], "the orphaned Text1 must count as taken");
}

/// Callers can keep the older behaviour — an incoming field of a taken name becomes one field
/// with the one already there — by asking to merge. Nothing is renamed.
#[test]
fn import_can_merge_into_same_named_fields_instead() {
    let mut dst = doc_with_field("Text1");
    let src = doc_with_field("Text1");
    let outcome = import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Merge).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1", "Text1"]);
    assert!(outcome.renamed_fields.is_empty(), "merge renames nothing");
}

/// Duplicating a page copies the document's own fields: the copies keep their names (one field
/// shown twice, as before field-conflict renaming existed) instead of silently becoming Text1-1.
#[test]
fn duplicating_a_page_with_a_field_keeps_the_field_name() {
    let mut doc = doc_with_field("Text1");
    crate::duplicate_pages(&mut doc, &[0], None).unwrap();
    assert_eq!(crate::page_count(&doc).unwrap(), 2);
    assert_eq!(form_field_names(&doc), ["Text1", "Text1"], "nothing renamed");
}

/// Replace Pages keeps the field names too.
#[test]
fn replacing_a_page_renames_no_fields() {
    let mut doc = doc_with_field("Text1");
    crate::replace_pages(&mut doc, &[0], &doc_with_field("Text1"), &[0]).unwrap();
    assert!(form_field_names(&doc).iter().all(|n| n == "Text1"), "{:?}", form_field_names(&doc));
}

/// Extract and Split copy one document: two same-named field roots (malformed but common) stay
/// one field by name rather than being split apart as Name and Name-1.
#[test]
fn extract_and_split_keep_same_named_fields_together() {
    let src = doc_with_fields(&["Text1", "Text1"]);
    let out = extract_pages(&src, &[0]).unwrap();
    assert_eq!(form_field_names(&out), ["Text1", "Text1"]);
    for part in split(&src, &SplitBy::PageCount(1)).unwrap() {
        assert_eq!(form_field_names(&part), ["Text1", "Text1"]);
    }
}

/// An import says what it renamed, so callers can tell the user — and their scripts — that a
/// field they knew by one name now goes by another.
#[test]
fn import_reports_the_fields_it_renamed() {
    let mut dst = doc_with_fields(&["Text1", "Text1-1"]);
    let src = doc_with_field("Text1");
    let outcome = import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(outcome.renamed_fields, [("Text1".to_string(), "Text1-2".to_string())]);
    assert_eq!(outcome.pages.len(), 1);
}

/// A renamed tree survives a save and reopen: the parent keeps its new name, the child still
/// hangs under it with its own name unchanged, and an independent parser reads the same form.
#[test]
fn a_renamed_field_tree_survives_a_save_and_reopen() {
    let mut dst = doc_with_tree("a", "b");
    let src = doc_with_tree("a", "b");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    let out = full_roundtrip(&dst);
    assert_eq!(form_field_names(&out), ["a", "a-1"], "the incoming root was renamed");
    let fields = catalog(&out)
        .get(b"AcroForm")
        .map(|f| out.resolve(f))
        .and_then(|f| f.as_dict().cloned())
        .unwrap()
        .get(b"Fields")
        .map(|l| out.resolve(l))
        .and_then(|l| l.as_array().cloned())
        .unwrap();
    let root = out.resolve(&fields[1]).as_dict().cloned().unwrap();
    let kids = root.get(b"Kids").map(|k| out.resolve(k)).and_then(|k| k.as_array().cloned()).unwrap();
    let kid = out.resolve(&kids[0]).as_dict().cloned().unwrap();
    assert_eq!(kid.reference(b"Parent"), fields[1].as_ref(), "the child still hangs under the renamed root");
    let kid_t = kid.get(b"T").cloned().unwrap();
    let resolved = out.resolve(&kid_t);
    assert_eq!(String::from_utf8_lossy(&resolved.as_string().unwrap().bytes), "b", "the child keeps its own name");
}

/// The pre-scan says what an import of `src` into `dst` would rename, without changing
/// anything — the Combine tab shows it before anything is combined.
#[test]
fn field_name_conflicts_previews_what_an_import_renames() {
    let dst = doc_with_fields(&["Text1", "Text1-1"]);
    let src = doc_with_field("Text1");
    assert_eq!(field_name_conflicts(&dst, &src), [("Text1".to_string(), "Text1-2".to_string())]);
    let apart = doc_with_field("Signature1");
    assert!(field_name_conflicts(&dst, &apart).is_empty(), "nothing collides, nothing to preview");
}

/// The collision is not only with the destination: two incoming fields of one name would blur
/// into each other just the same, so the second is renamed too.
#[test]
fn import_renames_a_field_colliding_with_another_incoming_field() {
    let mut dst = doc_with_field("Other");
    let src = doc_with_fields(&["Text1", "Text1"]);
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Other", "Text1", "Text1-1"]);
}

/// A page listed twice shows its widget on both copies — also when the field was renamed: each
/// copy has its own widget, both widgets belong to the one renamed field (#1810), and no second
/// Text1 appears.
#[test]
fn a_repeated_page_joins_the_renamed_field() {
    let mut dst = doc_with_field("Text1");
    let src = doc_with_field("Text1");
    import_pages_with(&mut dst, &src, &[0, 0], 2, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1", "Text1-1"], "one field, renamed once");
    let fields = catalog(&dst)
        .get(b"AcroForm")
        .map(|f| dst.resolve(f))
        .and_then(|f| f.as_dict().cloned())
        .unwrap()
        .get(b"Fields")
        .map(|l| dst.resolve(l))
        .and_then(|l| l.as_array().cloned())
        .unwrap();
    let renamed = fields[1].clone();
    let mut widgets = Vec::new();
    for page in [1, 2] {
        let list = page_dict(&dst, page).get(b"Annots").map(|a| dst.resolve(a)).and_then(|a| a.as_array().cloned()).unwrap();
        let widget = list.iter().find_map(|a| a.as_ref()).expect("a widget on the page");
        widgets.push(widget);
        assert_eq!(dst.get(widget).as_dict().unwrap().reference(b"Parent"), renamed.as_ref(), "page {page}'s widget belongs to the renamed field");
    }
    assert_ne!(widgets[0], widgets[1], "each copy has its own widget");
}

/// Combining forms keeps same-named fields separate — the second file's colliding field is
/// renamed — and the combine says what it renamed.
#[test]
fn combine_renames_colliding_field_names_and_reports_them() {
    let a = doc_with_field("Text1");
    let b = doc_with_field("Text1");
    let out = combine_selected_with(&[("a", &a as &Document, None), ("b", &b, None)], FieldConflicts::Rename).unwrap();
    assert_eq!(out.renamed_fields, [("Text1".to_string(), "Text1-1".to_string())]);
    assert_eq!(form_field_names(&out.doc), ["Text1", "Text1-1"]);
    let merged = combine_selected_with(&[("a", &a, None), ("b", &b, None)], FieldConflicts::Merge).unwrap();
    assert!(merged.renamed_fields.is_empty());
    assert_eq!(form_field_names(&merged.doc), ["Text1", "Text1"], "merge keeps the older behaviour");
}

/// The grouped combine (a file's pages split around others') renames colliding names between
/// files just the same, while a file combined with itself once keeps its fields whole.
#[test]
fn combine_grouped_renames_colliding_field_names() {
    let a = doc_with_field("Text1");
    let b = doc_with_field("Text1");
    let runs: Vec<Run<'_>> = vec![(0, "a", &a, None), (1, "b", &b, None)];
    let out = combine_grouped_with(&runs, FieldConflicts::Rename).unwrap();
    assert_eq!(out.renamed_fields, [("Text1".to_string(), "Text1-1".to_string())]);
    assert_eq!(form_field_names(&out.doc), ["Text1", "Text1-1"]);
    let split: Vec<Run<'_>> = vec![(0, "a", &a, Some(&[0])), (0, "a", &a, None), (1, "b", &b, None)];
    let once = combine_grouped_with(&split, FieldConflicts::Rename).unwrap();
    assert_eq!(
        once.renamed_fields,
        [("Text1".to_string(), "Text1-1".to_string())],
        "the grouped file is copied once; only b's colliding field renames"
    );
}

/// The pre-flight scan over a whole combine list: what each file's fields would be renamed,
/// in order, exactly as the combine would rename them — the Combine tab shows this before
/// anything is combined.
#[test]
fn combine_field_conflicts_previews_each_file_in_order() {
    let a = doc_with_field("Text1");
    let b = doc_with_field("Text1");
    let c = doc_with_fields(&["Text1", "Other"]);
    let preview = combine_field_conflicts(&[&a, &b, &c]);
    assert_eq!(preview[0], [], "the first file collides with nothing");
    assert_eq!(preview[1], [("Text1".to_string(), "Text1-1".to_string())]);
    assert_eq!(preview[2], [("Text1".to_string(), "Text1-2".to_string())], "the cascade continues past the earlier rename");
}

/// A one-page source whose page refers (`/Foo`) to object 6, followed by `tail` as objects 6, 7, …
fn chained(tail: &[String]) -> Document {
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Foo 6 0 R >>".into(),
        body("C1"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    b.extend_from_slice(tail);
    open(build(&b, "/Root 1 0 R"))
}

/// [`chained`] with `n` objects, each referring to the next.
fn chain(n: usize) -> Document {
    let mut tail: Vec<String> = (0..n).map(|i| format!("<< /Next {} 0 R >>", 7 + i)).collect();
    tail.push("<< /End true >>".into());
    chained(&tail)
}

/// #1800: thousands of shallow objects, each referring to the next, used to recurse once per
/// object and exhaust the stack. The copy is refused instead, and the destination is as it was;
/// a chain within the budget and a short cycle copy as before.
#[test]
fn a_chain_of_objects_too_deep_to_copy_is_refused_and_leaves_the_destination_alone() {
    // On a thread with less stack than any platform's main thread, so the budget is proven.
    let run = std::thread::Builder::new()
        .stack_size(512 << 10)
        .spawn(|| {
            let mut dst = open(fixture());
            let before = (labels(&dst), dst.object_numbers());
            let err = import_pages(&mut dst, &chain(20_000), &[0], 0).unwrap_err();
            assert!(matches!(&err, OrganizeError::Invalid(m) if m.contains("levels deep")), "{err:?}");
            assert_eq!((labels(&dst), dst.object_numbers()), before, "the destination is as it was");
            import_pages(&mut dst, &chain(100), &[0], 0).unwrap();
            let cycle = chained(&["<< /Next 7 0 R >>".into(), "<< /Next 6 0 R >>".into()]);
            import_pages(&mut dst, &cycle, &[0], 0).unwrap();
            assert_eq!(labels(&dst)[..3], ["C1", "C1", "Page 1"]);
        })
        .unwrap()
        .join();
    assert!(run.is_ok(), "the copy must not overflow the stack");
}

/// The fixture after an update that adds object 20 and declares `/Size 4294967295`. No number is
/// left above that, so [`Document::add`] fills free numbers from 12 on, the gap below object 20 first.
fn fixture_with_an_exhausted_size() -> Document {
    let mut bytes = fixture();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let prev: usize = text.rsplit("startxref").next().and_then(|t| t.split_whitespace().next()).and_then(|n| n.parse().ok()).expect("startxref");
    let added = bytes.len();
    bytes.extend_from_slice(b"20 0 obj\n<< /Unused true >>\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n20 1\n{added:010} 00000 n \ntrailer\n<< /Size 4294967295 /Root 1 0 R /Info 11 0 R /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let doc = open(bytes);
    assert!(doc.repair_log().iter().any(|l| l.contains("exhausted trailer")), "{:?}", doc.repair_log());
    doc
}

/// The fixture holding object 4294967295, as a document does once an edit writes to an object of
/// that number (a damaged file's catalog, say). [`Document::add`] then fills free numbers below it.
fn fixture_holding_object_4294967295() -> Document {
    let mut doc = open(fixture());
    doc.set(ObjRef::new(u32::MAX, 0), Object::String(PdfString::text("top")));
    doc
}

/// Documents whose new objects are numbered in the gaps below their highest number.
fn documents_numbering_new_objects_below_their_highest() -> [(&'static str, Document); 2] {
    [("an exhausted /Size", fixture_with_an_exhausted_size()), ("object 4294967295", fixture_holding_object_4294967295())]
}

/// Duplicate, Replace and Insert Pages took the objects numbered above the destination's highest
/// number for the ones the import added. When new objects fill gaps below it, the copies escaped
/// resource sharing (each import stored the fonts again), and with object 4294967295 the number
/// after the highest overflowed: a panic in debug builds, and in release a wrap to 0 that counted
/// every object as added. The import now compares the object numbers before and after.
#[test]
fn an_import_shares_the_resources_it_added_wherever_they_are_numbered() {
    let mut plain = open(fixture());
    duplicate_pages(&mut plain, &[0], None).unwrap();
    for (what, mut doc) in documents_numbering_new_objects_below_their_highest() {
        let before = doc.object_numbers();
        duplicate_pages(&mut doc, &[0], None).unwrap();
        assert_eq!(labels(&doc), ["Page 1", "Page 1", "Page 2", "Page 3"], "{what}");
        assert_eq!(count_fonts(&doc), count_fonts(&plain), "{what}: the copy shares the font, as it does in the plain fixture");
        insert_file_pages(&mut doc, &open(fixture()), "Inserted", &[2], 0, FieldConflicts::Rename).unwrap();
        assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 1", "Page 2", "Page 3"], "{what}");
        assert_eq!(count_fonts(&doc), count_fonts(&plain), "{what}: the inserted page shares the font too");
        let after = doc.object_numbers();
        assert!(before.iter().all(|n| after.contains(n)), "{what}: every object the document had is still there");
        assert_eq!(labels(&full_roundtrip(&doc)), ["Page 3", "Page 1", "Page 1", "Page 2", "Page 3"], "{what}");
    }
}

/// A refused import takes back what it added, which in these documents is numbered below their
/// highest object, and leaves the objects they had.
#[test]
fn a_refused_import_takes_back_what_it_added_wherever_it_is_numbered() {
    let run = std::thread::Builder::new()
        .stack_size(512 << 10)
        .spawn(|| {
            for (what, mut dst) in documents_numbering_new_objects_below_their_highest() {
                let before = (labels(&dst), dst.object_numbers());
                let err = import_pages(&mut dst, &chain(20_000), &[0], 0).unwrap_err();
                assert!(matches!(&err, OrganizeError::Invalid(m) if m.contains("levels deep")), "{what}: {err:?}");
                assert_eq!((labels(&dst), dst.object_numbers()), before, "{what}: the destination is as it was");
            }
        })
        .unwrap()
        .join();
    assert!(run.is_ok(), "a refused import must leave the document as it was, without panicking");
}

/// What Edit ▸ Attach file added survives Combine files (upstream's writer), names
/// with accents included, and the combined document takes more attachments and deletions.
#[test]
fn attachments_added_in_the_app_survive_combine_and_more_can_be_attached() {
    let embedded = |doc: &Document, key: &str| -> (Vec<u8>, Option<String>) {
        let names = doc.resolve(catalog(doc).get(b"Names").unwrap()).as_dict().cloned().unwrap();
        let tree = doc.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
        let pairs = tree.get(b"Names").and_then(|n| n.as_array()).cloned().unwrap();
        let at = pairs.chunks(2).position(|p| p[0].as_string().unwrap().to_text() == key).unwrap_or_else(|| panic!("no attachment {key}"));
        let spec = doc.resolve(&pairs[at * 2 + 1]).as_dict().cloned().unwrap();
        let desc = spec.get(b"Desc").and_then(Object::as_string).map(|s| s.to_text());
        let ef = spec.get(b"EF").and_then(Object::as_dict).unwrap().get(b"F").cloned().unwrap();
        let Object::Stream(stream) = doc.resolve(&ef).as_ref().clone() else { panic!("the embedded file is a stream") };
        (stream.decoded().unwrap(), desc)
    };
    let mut a = Document::open(Arc::new(fixture())).unwrap();
    add_attachment(&mut a, "invoice.csv", b"a,b", Some("March"), Some("text/csv"), None).unwrap();
    add_attachment(&mut a, "año.txt", b"uno", None, None, None).unwrap();
    let mut b = Document::open(Arc::new(fixture())).unwrap();
    add_attachment(&mut b, "invoice.csv", b"x,y", None, None, None).unwrap();
    add_attachment(&mut b, "año.txt", b"dos", None, None, None).unwrap();
    let a = save_and_reopen(&a);
    let b = save_and_reopen(&b);
    let mut out = full_roundtrip(&combine(&[("A", &a), ("B", &b)]).unwrap());
    // Both files' attachments, the second copies numbered as text (not bytes glued onto UTF-16).
    assert_eq!(attachment_keys(&out), ["invoice.csv", "invoice.csv (2)", "año.txt", "año.txt (2)"]);
    assert_eq!(embedded(&out, "invoice.csv"), (b"a,b".to_vec(), Some("March".into())));
    assert_eq!(embedded(&out, "invoice.csv (2)"), (b"x,y".to_vec(), None));
    assert_eq!(embedded(&out, "año.txt (2)").0, b"dos");
    // The combined document goes on taking attachments, and losing them, like any other.
    assert_eq!(add_attachment(&mut out, "invoice.csv", b"z", None, None, None).unwrap(), "invoice.csv (3)");
    delete_attachment(&mut out, "invoice.csv (2)").unwrap();
    let out = full_roundtrip(&out);
    assert_eq!(attachment_keys(&out), ["invoice.csv", "invoice.csv (3)", "año.txt", "año.txt (2)"]);
    assert_eq!(embedded(&out, "invoice.csv (3)").0, b"z");
}

#[test]
fn attachments_are_added_listed_and_deleted() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    assert!(attachment_keys(&doc).is_empty());
    // A path loses its folder; the key is the file name.
    let key = add_attachment(&mut doc, "/Users/eva/Desktop/invoice.csv", b"a,b\n1,2\n", Some("March"), Some("text/csv"), Some("D:20261012120000Z"))
        .unwrap();
    assert_eq!(key, "invoice.csv");
    // The same name again is kept apart.
    assert_eq!(add_attachment(&mut doc, "invoice.csv", b"x", None, None, None).unwrap(), "invoice.csv (2)");
    assert_eq!(add_attachment(&mut doc, "  ", b"x", None, None, None), Err(OrganizeError::Invalid("the attachment needs a file name".into())));
    let doc = save_and_reopen(&doc);
    assert_eq!(attachment_keys(&doc), ["invoice.csv", "invoice.csv (2)"]);
    // What was written: a Filespec with the embedded stream, its size and type, and the description.
    let root = doc.root().unwrap();
    let names = doc.get(root).as_dict().unwrap().get(b"Names").cloned().unwrap();
    let tree = doc.resolve(&names).as_dict().unwrap().get(b"EmbeddedFiles").cloned().unwrap();
    let tree = doc.resolve(&tree);
    let pairs = tree.as_dict().unwrap().get(b"Names").cloned().unwrap();
    let pairs = doc.resolve(&pairs).as_array().cloned().unwrap();
    assert_eq!(pairs.len(), 4);
    let spec = doc.resolve(&pairs[1]);
    let spec = spec.as_dict().unwrap();
    assert_eq!(spec.get(b"Type"), Some(&Object::name("Filespec")));
    assert_eq!(spec.get(b"UF").and_then(Object::as_string).map(PdfString::to_text).as_deref(), Some("invoice.csv"));
    assert_eq!(spec.get(b"Desc").and_then(Object::as_string).map(PdfString::to_text).as_deref(), Some("March"));
    let ef = spec.get(b"EF").and_then(Object::as_dict).unwrap().get(b"F").cloned().unwrap();
    let ef = doc.resolve(&ef);
    let Object::Stream(stream) = ef.as_ref() else { panic!("the embedded file is a stream") };
    assert_eq!(stream.dict.get(b"Subtype"), Some(&Object::Name(b"text/csv".to_vec())));
    let params = stream.dict.get(b"Params").and_then(Object::as_dict).unwrap();
    assert_eq!(params.get(b"Size"), Some(&Object::Int(8)));
    assert_eq!(stream.decoded().unwrap(), b"a,b\n1,2\n");
    // Deleting: the one named goes, the other stays; the last one takes the tree with it.
    let mut doc = doc;
    delete_attachment(&mut doc, "invoice.csv").unwrap();
    assert_eq!(attachment_keys(&doc), ["invoice.csv (2)"]);
    assert_eq!(delete_attachment(&mut doc, "invoice.csv"), Err(OrganizeError::Invalid("there is no attachment named invoice.csv".into())));
    delete_attachment(&mut doc, "invoice.csv (2)").unwrap();
    let doc = save_and_reopen(&doc);
    assert!(attachment_keys(&doc).is_empty());
    assert!(doc.get(doc.root().unwrap()).as_dict().unwrap().get(b"Names").is_none());
}

#[test]
fn rotate_repeated_indices_accumulate_from_the_current_page_value() {
    for level in 0..3 {
        for indices in [vec![0, 0], vec![0, 1, 0]] {
            let mut doc = open(fixture());
            let rotation = doc.add(Object::Int(90));
            let page = pages(&doc).unwrap()[0].obj;
            let target = match level {
                0 => page,
                1 => doc.get(page).as_dict().unwrap().reference(b"Parent").unwrap(),
                _ => pages_root(&doc).unwrap(),
            };
            doc.update_dict(target, |d| d.set(b"Rotate".to_vec(), Object::Ref(rotation))).unwrap();
            let mut doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
            let rotation = walk(&doc).unwrap()[0].1.reference(b"Rotate").unwrap();
            let original_labels = labels(&doc);
            rotate_pages(&mut doc, &indices, 90).unwrap();
            assert_eq!(page_rotation(&doc, 0).unwrap(), 270, "level {level}, indices {indices:?}");
            assert_eq!(*doc.get(rotation), Object::Int(90), "shared source angle is unchanged");
            for saved in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
                let reopened = open(saved.clone());
                assert_eq!(page_rotation(&reopened, 0).unwrap(), 270);
                assert_eq!(page_rotation(&reopened, 1).unwrap(), if indices.len() == 3 { 180 } else { 90 });
                assert_eq!(page_rotation(&reopened, 2).unwrap(), 90);
                assert_eq!(labels(&reopened), original_labels);
                let independent = hayro_syntax::Pdf::new(saved).unwrap();
                assert_eq!(independent.pages()[0].render_dimensions(), (400.0, 300.0));
            }
        }
    }
}

#[test]
fn a_bookmark_can_go_to_a_point_on_its_page_and_survives_save() {
    let mut d = doc_a();
    // No position: as before, `/XYZ null null null`, the page top at the reader's zoom.
    crate::add_bookmark(&mut d, &[], 9, "Top", 0).unwrap();
    // Only a height (a bookmark made at the top of the window), a full point, and only a left.
    assert_eq!(crate::add_bookmark_at(&mut d, &[], 9, "Section", 1, None, Some(612.25)).unwrap(), vec![1]);
    crate::add_bookmark_at(&mut d, &[], 9, "Table", 1, Some(72.5), Some(300.0)).unwrap();
    crate::add_bookmark_at(&mut d, &[1], 9, "Inside", 2, Some(10.0), None).unwrap();
    let view = |d: &Document, title: &str| crate::bookmarks(d).into_iter().find(|b| b.title == title).unwrap().view;
    let xyz = |left, top| Some((b"XYZ".to_vec(), vec![left, top, None]));
    let check = |d: &Document| {
        assert_eq!(view(d, "Top"), xyz(None, None));
        assert_eq!(view(d, "Section"), xyz(None, Some(612.25)));
        assert_eq!(view(d, "Table"), xyz(Some(72.5), Some(300.0)));
        assert_eq!(view(d, "Inside"), xyz(Some(10.0), None));
        let page_of = |title: &str| crate::bookmarks(d).into_iter().find(|b| b.title == title).unwrap().page;
        assert_eq!((page_of("Top"), page_of("Section"), page_of("Table"), page_of("Inside")), (Some(0), Some(1), Some(1), Some(2)));
    };
    check(&d);
    check(&full_roundtrip(&d));
}

#[test]
fn a_bookmark_position_that_is_not_a_number_is_refused_and_changes_nothing() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 9, "Keep", 0).unwrap();
    for (left, top) in [(Some(f64::NAN), None), (None, Some(f64::INFINITY)), (Some(1.0), Some(f64::NEG_INFINITY))] {
        assert_eq!(crate::add_bookmark_at(&mut d, &[], 9, "Bad", 0, left, top), Err(crate::OutlineError::BadPosition), "{left:?} {top:?}");
    }
    assert_eq!(titles(&crate::bookmarks(&d)), ["Keep"]);
    // The page is still checked, and so is the title.
    assert!(crate::add_bookmark_at(&mut d, &[], 9, "Nowhere", 7, Some(1.0), Some(1.0)).is_err());
    assert_eq!(crate::add_bookmark_at(&mut d, &[], 9, " ", 0, Some(1.0), Some(1.0)), Err(crate::OutlineError::EmptyTitle));
    assert_eq!(titles(&crate::bookmarks(&d)), ["Keep"]);
}

#[test]
fn page_walk_skips_repeated_and_cyclic_kids_without_copying_attributes() {
    let mut doc = open(fixture());
    let (root, branch) = (ObjRef { num: 2, generation: 0 }, ObjRef { num: 3, generation: 0 });
    let (first, second) = (ObjRef { num: 4, generation: 0 }, ObjRef { num: 5, generation: 0 });
    // The branch node carries Resources for its pages: a cycle back to it, then both pages
    // 128 times over.
    let mut kids = vec![Object::Ref(branch)];
    kids.extend([Object::Ref(first), Object::Ref(second)].into_iter().cycle().take(256));
    doc.update_dict(branch, |d| d.set(b"Kids".to_vec(), Object::Array(kids))).unwrap();
    let mut walk = PageWalk::new(&doc, root);
    let (page, attrs) = walk.next().unwrap();
    assert_eq!(page, first);
    assert!(walk.pending.len() <= 3, "repeated kids retained {} pending entries", walk.pending.len());
    assert!(attrs.get(b"Resources").and_then(|r| r.as_dict()).is_some_and(|r| r.contains(b"Font")), "Resources from the branch");
    assert_eq!(attrs.get(b"Rotate").and_then(Object::as_int), Some(90), "Rotate from the root");
    assert_eq!(walk.next().map(|(p, _)| p), Some(second));
    let (_, last) = walk.next().unwrap();
    let media_box = last.get(b"MediaBox").and_then(Object::as_array).map(|m| m.iter().filter_map(Object::as_int).collect::<Vec<_>>());
    assert_eq!(media_box, Some(vec![0, 0, 500, 500]), "own MediaBox kept over the root's");
    assert!(walk.next().is_none());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 3"]);
}

/// A one-page document whose page carries `annots` (object bodies 5…), with `extra` bodies after them.
fn page_with_annots(annots: &[String], extra: &[String], catalog_extra: &str) -> Document {
    let n = annots.len();
    let refs: String = (0..n).map(|i| format!("{} 0 R ", i + 5)).collect();
    let mut b: Vec<String> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Annots [{refs}] >>"),
        body("P1"),
    ];
    b.extend(annots.iter().cloned());
    b.extend(extra.iter().cloned());
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn registering_many_imported_fields_is_linear() {
    let n = 2_000;
    let widgets: Vec<String> = (0..n).map(|i| format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (f{i}) /Rect [0 0 10 10] >>")).collect();
    let src = page_with_annots(&widgets, &[], "");
    take_work();
    let out = extract_pages(&src, &[0]).unwrap();
    let work = take_work();
    assert!(work <= 4 * n, "{work} steps registering {n} fields");
    let form = out.resolve(catalog(&out).get(b"AcroForm").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(form.get(b"Fields").and_then(|f| f.as_array()).map(Vec::len), Some(n));
}

#[test]
fn renaming_many_same_named_fields_resumes_the_suffix_search() {
    let n = 2_000;
    let widgets: Vec<String> = (0..n).map(|_| "<< /Type /Annot /Subtype /Widget /FT /Tx /T (x) /Rect [0 0 10 10] >>".to_string()).collect();
    let src = page_with_annots(&widgets, &[], "");
    take_work();
    let renames = combine_field_conflicts(&[&src]).remove(0);
    let work = take_work();
    assert!(work <= 4 * n, "{work} suffix attempts for {n} fields");
    // Still the first free variant, in order.
    assert_eq!(renames.len(), n - 1);
    assert_eq!(renames.first(), Some(&("x".to_string(), "x-1".to_string())));
    assert_eq!(renames.last(), Some(&("x".to_string(), format!("x-{}", n - 1))));
}

#[test]
fn annotation_actions_share_the_objects_they_copy() {
    // Fifty links whose URI actions all point at one string object.
    let n = 50;
    let links: Vec<String> =
        (0..n).map(|_| format!("<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /URI /URI {} 0 R >> >>", n + 5)).collect();
    let src = page_with_annots(&links, &["(https://example.com/shared)".into()], "");
    let out = extract_pages(&src, &[0]).unwrap();
    let copies = out
        .object_numbers()
        .into_iter()
        .filter(|&o| out.get(ObjRef::new(o, out.generation(o))).as_string().is_some_and(|s| s.bytes == b"https://example.com/shared"))
        .count();
    assert_eq!(copies, 1, "the shared URI is copied once, not once per action");
}

#[test]
fn name_trees_with_shared_kids_are_walked_once_per_node() {
    // Each level lists the next one twice: 2^depth paths through `depth` nodes.
    let depth = 10;
    let first = 6; // after the catalog, pages, page, content and the link
    let mut extra = Vec::new();
    for tree in 0..2 {
        let base = first + tree * (depth + 1);
        for level in 0..depth {
            let next = base + level + 1;
            extra.push(format!("<< /Kids [{next} 0 R {next} 0 R] >>"));
        }
        extra.push(if tree == 0 {
            "<< /Names [(a.txt) << /Type /Filespec /F (a.txt) >>] >>".into()
        } else {
            "<< /Names [(other) [3 0 R /Fit]] >>".into()
        });
    }
    let link = "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Dest (missing) >>".to_string();
    let catalog_extra = format!("/Names << /EmbeddedFiles {first} 0 R /Dests {} 0 R >>", first + depth + 1);
    let src = page_with_annots(&[link], &extra, &catalog_extra);
    take_work();
    let out = combine(&[("a", &src)]).unwrap();
    let work = take_work();
    assert!(work <= 8 * (depth + 1), "{work} name-tree nodes visited");
    let names = out.resolve(catalog(&out).get(b"Names").unwrap()).as_dict().cloned().unwrap();
    let tree = out.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(tree.get(b"Names").and_then(|n| n.as_array()).map(Vec::len), Some(2), "one attachment, listed once");
}

#[test]
fn a_form_drawing_itself_repeatedly_is_analysed_once() {
    let form = "/F Do /F Do /F Do /F Do";
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /F 5 0 R /Unused 6 0 R >> >> >>".into(),
        "<< /Length 5 >>\nstream\n/F Do\nendstream".into(),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /F 5 0 R >> >> /Length {} >>\nstream\n{form}\nendstream",
            form.len()
        ),
        "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Length 0 >>\nstream\n\nendstream".into(),
    ];
    let src = open(build(&b, "/Root 1 0 R"));
    take_work();
    let out = extract_pages(&src, &[0]).unwrap();
    let work = take_work();
    assert!(work <= 4, "{work} form analyses");
    let page = out.get(pages(&out).unwrap()[0].obj).as_dict().cloned().unwrap();
    let res = out.resolve(page.get(b"Resources").unwrap()).as_dict().cloned().unwrap();
    let x = out.resolve(res.get(b"XObject").unwrap()).as_dict().cloned().unwrap();
    assert!(x.contains(b"F") && !x.contains(b"Unused"), "pruning still keeps what is drawn and drops the rest");
}

/// One page whose /Contents names the same 1 MiB stream 300 times (a few kilobytes on disk).
fn repeated_contents() -> Document {
    use pdfcraft_cos::{Dict, Stream};
    let mut doc = Document::new_empty();
    let stream = doc.add(Stream::flate(Dict::new(), &vec![b' '; 1 << 20]));
    let pages = doc.add(Object::Null);
    let page: Dict = [
        (b"Type".to_vec(), Object::Name(b"Page".to_vec())),
        (b"Parent".to_vec(), Object::Ref(pages)),
        (b"MediaBox".to_vec(), Object::Array([0, 0, 200, 300].map(Object::from).to_vec())),
        (b"Contents".to_vec(), Object::Array(vec![Object::Ref(stream); 300])),
    ]
    .into_iter()
    .collect();
    let page = doc.add(page);
    let tree: Dict = [
        (b"Type".to_vec(), Object::Name(b"Pages".to_vec())),
        (b"Kids".to_vec(), Object::Array(vec![Object::Ref(page)])),
        (b"Count".to_vec(), Object::Int(1)),
    ]
    .into_iter()
    .collect();
    doc.set(pages, Object::Dict(tree));
    let catalog: Dict = [(b"Type".to_vec(), Object::Name(b"Catalog".to_vec())), (b"Pages".to_vec(), Object::Ref(pages))].into_iter().collect();
    let catalog = doc.add(catalog);
    doc.trailer_mut().set(b"Root".to_vec(), Object::Ref(catalog));
    doc
}

#[test]
fn repeated_content_references_share_one_decode_budget() {
    let src = repeated_contents();
    let mut dst = Document::new_empty();
    if let Ok((form, _)) = page_as_form(&mut dst, &src, 0) {
        let Object::Stream(s) = &*dst.get(form) else { panic!("not a stream") };
        let len = s.decoded().unwrap().len();
        assert!(len <= pdfcraft_cos::MAX_PAGE_DECODED + 300, "{len} bytes");
    }
}

/// #3193: the bookmark list reads a bookmark's colour and style as the inspector does (ISO
/// 32000-2 §12.3.3, Tables 151 and 152): `/C` within 0–1, black and malformed values as none,
/// `/F` bit 1 italic and bit 2 bold, through indirect values too.
#[test]
fn bookmarks_report_their_colour_and_style() {
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Outlines /First 5 0 R /Last 10 0 R /Count 6 >>".into(),
    ];
    objs.push("<< /Title (Plain) /Parent 4 0 R /Next 6 0 R /Dest [3 0 R /Fit] >>".into()); // 5
    objs.push("<< /Title (Red bold) /Parent 4 0 R /Prev 5 0 R /Next 7 0 R /Dest [3 0 R /Fit] /C [1 0 0] /F 2 >>".into()); // 6
    objs.push("<< /Title (Indirect) /Parent 4 0 R /Prev 6 0 R /Next 8 0 R /Dest [3 0 R /Fit] /C 11 0 R /F 12 0 R >>".into()); // 7
    objs.push("<< /Title (Black) /Parent 4 0 R /Prev 7 0 R /Next 9 0 R /Dest [3 0 R /Fit] /C [0 0 0] /F 1 >>".into()); // 8
    objs.push("<< /Title (Bright) /Parent 4 0 R /Prev 8 0 R /Next 10 0 R /Dest [3 0 R /Fit] /C [2 -1 0.5] >>".into()); // 9
    objs.push("<< /Title (Short) /Parent 4 0 R /Prev 9 0 R /Dest [3 0 R /Fit] /C [1 0] /F 4 >>".into()); // 10
    objs.push("[0 0.5 0]".into()); // 11
    objs.push("3".into()); // 12
    let doc = open(build(&objs, "/Root 1 0 R"));
    let got: Vec<(String, Option<[f32; 3]>, bool, bool)> = bookmarks(&doc).into_iter().map(|b| (b.title, b.color, b.italic, b.bold)).collect();
    assert_eq!(
        got,
        [
            ("Plain".into(), None, false, false),
            ("Red bold".into(), Some([1.0, 0.0, 0.0]), false, true),
            ("Indirect".into(), Some([0.0, 0.5, 0.0]), true, true),
            ("Black".into(), None, true, false),
            ("Bright".into(), Some([1.0, 0.0, 0.5]), false, false),
            ("Short".into(), None, false, false),
        ]
    );
}

/// #3193: setting a bookmark's style and colour writes what ISO 32000-2 §12.3.3 asks for: bold is
/// `/F 2` and bold italic `/F 3` (Table 152), and neither removes `/F`, whose default is 0; a
/// style change leaves `/C` alone and a colour change leaves `/F`; black, the default (Table 151),
/// removes `/C`. Bits the table doesn't define are kept, and a colour outside 0–1 is refused.
#[test]
fn bookmark_style_and_colour_are_set_as_the_table_says() {
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>".into(),
    ];
    objs.push("<< /Title (Plain) /Parent 4 0 R /Next 6 0 R /Dest [3 0 R /Fit] >>".into()); // 5
    objs.push("<< /Title (Odd bits) /Parent 4 0 R /Prev 5 0 R /Dest [3 0 R /Fit] /F 4 /C [1 0 0] >>".into()); // 6
    let mut doc = open(build(&objs, "/Root 1 0 R"));
    let raw = |doc: &Document, n: u32, key: &[u8]| doc.get(ObjRef { num: n, generation: 0 }).as_dict().and_then(|d| d.get(key).cloned());
    crate::set_bookmark_style(&mut doc, &[0], Some(true), None).unwrap();
    assert_eq!(raw(&doc, 5, b"F"), Some(Object::Int(2)), "bold");
    crate::set_bookmark_style(&mut doc, &[0], None, Some(true)).unwrap();
    assert_eq!(raw(&doc, 5, b"F"), Some(Object::Int(3)), "bold italic");
    crate::set_bookmark_color(&mut doc, &[0], Some([0.0, 0.5, 0.0])).unwrap();
    assert_eq!(raw(&doc, 5, b"C"), Some(Object::Array(vec![Object::Real(0.0), Object::Real(0.5), Object::Real(0.0)])));
    assert_eq!(raw(&doc, 5, b"F"), Some(Object::Int(3)), "a colour change keeps the style");
    crate::set_bookmark_style(&mut doc, &[0], Some(false), Some(false)).unwrap();
    assert_eq!(raw(&doc, 5, b"F"), None, "regular removes /F");
    assert!(raw(&doc, 5, b"C").is_some(), "a style change keeps the colour");
    crate::set_bookmark_color(&mut doc, &[0], Some([0.0; 3])).unwrap();
    assert_eq!(raw(&doc, 5, b"C"), None, "black removes /C");

    crate::set_bookmark_style(&mut doc, &[1], Some(true), None).unwrap();
    assert_eq!(raw(&doc, 6, b"F"), Some(Object::Int(6)), "bold beside a bit the table doesn't define");
    crate::set_bookmark_style(&mut doc, &[1], Some(false), Some(false)).unwrap();
    assert_eq!(raw(&doc, 6, b"F"), Some(Object::Int(4)), "that bit stays");
    crate::set_bookmark_color(&mut doc, &[1], None).unwrap();
    assert_eq!(raw(&doc, 6, b"C"), None, "no colour is black");
    for bad in [[1.5, 0.0, 0.0], [0.0, -0.1, 0.0], [f64::NAN, 0.0, 0.0]] {
        assert_eq!(crate::set_bookmark_color(&mut doc, &[1], Some(bad)), Err(crate::OutlineError::BadColor), "{bad:?}");
    }
    assert_eq!(crate::set_bookmark_style(&mut doc, &[7], Some(true), None), Err(crate::OutlineError::NoSuchBookmark(vec![7])));

    crate::set_bookmark_style(&mut doc, &[0], Some(true), Some(true)).unwrap();
    crate::set_bookmark_color(&mut doc, &[0], Some([0.0, 0.0, 1.0])).unwrap();
    let saved = open(write_full(&doc, &SaveOptions::default()).unwrap());
    let got: Vec<_> = bookmarks(&saved).into_iter().map(|b| (b.color, b.bold, b.italic)).collect();
    assert_eq!(got, [(Some([0.0, 0.0, 1.0]), true, true), (None, false, false)]);
}

/// #3502: a `/Contents` array's pieces are joined with a space, as the renderer joins them, so a
/// `%` comment at the end of a piece without a line break continues through the first line of the
/// next piece in both resource pruning (`extract_pages`) and `page_as_form`.
#[test]
fn contents_pieces_join_with_a_space_across_a_trailing_comment() {
    let build_doc = |first_piece: &str| {
        let b: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),                                        // 1
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".into(),        // 2
            "<< /Type /Page /Parent 2 0 R /Contents [4 0 R 5 0 R] /Resources 6 0 R >>".into(), // 3
            raw(first_piece),                                                                  // 4
            raw("/ImHidden Do\n/ImShown Do"),                                                  // 5
            "<< /XObject << /ImHidden 7 0 R /ImShown 8 0 R >> >>".into(),                      // 6
            image(1, 1, 1),                                                                    // 7
            image(1, 1, 1),                                                                    // 8
        ];
        open(build(&b, "/Root 1 0 R"))
    };

    // Without a trailing newline in piece 1, `% comment` swallows `/ImHidden Do` on the first
    // line of piece 2; only `/ImShown` is drawn.
    let commented = build_doc("q Q % comment without newline");
    let extracted = full_roundtrip(&extract_pages(&commented, &[0]).unwrap());
    assert_eq!(xobject_names(&extracted, 0), [b"ImShown".to_vec()], "commented-out Do across piece boundary is pruned");

    let mut form_dst = Document::new_empty();
    let (form_ref, _) = crate::page_as_form(&mut form_dst, &commented, 0).unwrap();
    let Object::Stream(form_stream) = &*form_dst.get(form_ref) else { panic!("form stream") };
    let form_bytes = form_stream.decoded().unwrap();
    let drawn: Vec<Vec<u8>> = pdfcraft_content::parse(&form_bytes)
        .ops
        .into_iter()
        .filter(|op| op.op == b"Do")
        .filter_map(|op| op.operands.first().and_then(Object::as_name).map(|n| n.to_vec()))
        .collect();
    assert_eq!(drawn, [b"ImShown".to_vec()], "page_as_form keeps the first line of piece 2 inside the comment");

    // Control: when piece 1 ends the comment with a newline, `/ImHidden Do` on piece 2's first
    // line is a real operator and stays in both the extracted page and the form XObject.
    let newline_ended = build_doc("q Q % comment with newline\n");
    let extracted_ctrl = full_roundtrip(&extract_pages(&newline_ended, &[0]).unwrap());
    let mut ctrl_names = xobject_names(&extracted_ctrl, 0);
    ctrl_names.sort();
    assert_eq!(ctrl_names, [b"ImHidden".to_vec(), b"ImShown".to_vec()]);
}

/// Intro (closed, red bold, at a point) > [Part (italic, FitH), Figure (FitR)], then Results.
fn outline_source() -> Document {
    let mut d = doc_a();
    crate::add_bookmark_at(&mut d, &[], 0, "Intro", 0, Some(20.0), Some(380.0)).unwrap();
    crate::add_bookmark(&mut d, &[0], 0, "Part", 1).unwrap();
    crate::add_bookmark(&mut d, &[0], 1, "Figure", 2).unwrap();
    crate::add_bookmark(&mut d, &[], 1, "Results", 2).unwrap();
    crate::set_bookmark_color(&mut d, &[0], Some([1.0, 0.0, 0.0])).unwrap();
    crate::set_bookmark_style(&mut d, &[0], Some(true), None).unwrap();
    crate::set_bookmark_style(&mut d, &[0, 0], None, Some(true)).unwrap();
    let pages = crate::walk(&d).unwrap();
    let (part, figure) = (crate::bookmarks(&d)[1].obj, crate::bookmarks(&d)[2].obj);
    let dest = |p: usize, rest: Vec<Object>| Object::Array([vec![Object::Ref(pages[p].0)], rest].concat());
    d.update_dict(part, |x| x.set(b"Dest".to_vec(), dest(1, vec![Object::name("FitH"), Object::Real(300.0)]))).unwrap();
    let fit_r = vec![Object::name("FitR"), Object::Int(10), Object::Int(20), Object::Int(110), Object::Int(220)];
    d.update_dict(figure, |x| x.set(b"Dest".to_vec(), dest(2, fit_r))).unwrap();
    crate::set_bookmark_open(&mut d, &[0], false).unwrap();
    d
}

fn node(title: &str, page: Option<usize>, children: Vec<crate::OutlineNode>) -> crate::OutlineNode {
    crate::OutlineNode { title: title.into(), page, view: None, color: None, italic: false, bold: false, open: true, children }
}

/// The `/Prev`, `/Last` and `/Parent` links agree with `/First`–`/Next` at every level.
fn assert_back_links(doc: &Document) {
    let root = doc.get(doc.root().unwrap()).as_dict().unwrap().reference(b"Outlines").unwrap();
    let mut stack = vec![root];
    while let Some(parent) = stack.pop() {
        let kids = crate::outline::children(doc, parent);
        let d = doc.get(parent).as_dict().cloned().unwrap();
        assert_eq!(d.reference(b"Last"), kids.last().copied());
        for (i, k) in kids.iter().enumerate() {
            let kd = doc.get(*k).as_dict().cloned().unwrap();
            assert_eq!(kd.reference(b"Prev"), i.checked_sub(1).map(|p| kids[p]));
            assert_eq!(kd.reference(b"Parent"), Some(parent));
        }
        stack.extend(kids);
    }
}

#[test]
fn an_exported_outline_imports_into_another_document_unchanged() {
    let source = outline_source();
    let tree = crate::outline_tree(&source, &[]).unwrap();
    assert!(!tree.truncated);
    let intro = &tree.nodes[0];
    assert_eq!((intro.title.as_str(), intro.page, intro.open, intro.bold, intro.color), ("Intro", Some(0), false, true, Some([1.0, 0.0, 0.0])));
    assert_eq!(intro.view, Some((b"XYZ".to_vec(), vec![Some(20.0), Some(380.0), None])));
    assert_eq!(intro.children[0].view, Some((b"FitH".to_vec(), vec![Some(300.0)])));
    assert!(intro.children[0].italic);
    assert_eq!(intro.children[1].view, Some((b"FitR".to_vec(), vec![Some(10.0), Some(20.0), Some(110.0), Some(220.0)])));

    let mut target = doc_a();
    assert_eq!(crate::import_outline(&mut target, &[], 0, &tree.nodes, false).unwrap(), vec![0]);
    assert_back_links(&target);
    let back = full_roundtrip(&target);
    assert_eq!(crate::outline_tree(&back, &[]).unwrap(), tree);
    // The closed parent stays closed: its count is negative and the root counts only what shows.
    let first = crate::bookmarks(&back)[0].obj;
    let root = back.get(back.root().unwrap()).as_dict().unwrap().reference(b"Outlines").unwrap();
    assert_eq!((count_of(&back, first), count_of(&back, root)), (Some(-2), Some(2)));
}

#[test]
fn a_subtree_exports_alone_and_a_missing_path_is_refused() {
    let source = outline_source();
    let tree = crate::outline_tree(&source, &[0]).unwrap();
    assert_eq!(tree.nodes.len(), 1);
    assert_eq!(tree.nodes[0].title, "Intro");
    assert_eq!(tree.nodes[0].children.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["Part", "Figure"]);
    let leaf = crate::outline_tree(&source, &[0, 1]).unwrap();
    assert_eq!((leaf.nodes.len(), leaf.nodes[0].title.as_str()), (1, "Figure"));
    assert_eq!(crate::outline_tree(&source, &[5]), Err(crate::OutlineError::NoSuchBookmark(vec![5])));
    assert_eq!(crate::outline_tree(&doc_a(), &[]).unwrap().nodes, []);
}

#[test]
fn a_page_the_target_lacks_keeps_its_bookmark_without_a_destination() {
    let tree = crate::outline_tree(&outline_source(), &[]).unwrap();
    // doc_b has two pages: Figure and Results go to the third.
    assert_eq!(crate::unmapped(&tree.nodes, 2), [vec![0, 1], vec![1]]);
    let mut target = doc_b();
    crate::import_outline(&mut target, &[], 0, &tree.nodes, false).unwrap();
    let listed = crate::bookmarks(&target);
    assert_eq!(titles(&listed), ["Intro[Part,Figure]", "Results"]);
    assert_eq!(listed.iter().map(|b| b.page).collect::<Vec<_>>(), [Some(0), Some(1), None, None]);
    assert!(target.get(listed[2].obj).as_dict().unwrap().get(b"Dest").is_none());
}

#[test]
fn a_fit_r_missing_a_value_copies_with_its_page_and_no_view() {
    let mut src = outline_source();
    let pages = crate::walk(&src).unwrap();
    let figure = crate::bookmarks(&src)[2].obj;
    let short = vec![Object::Ref(pages[2].0), Object::name("FitR"), Object::Int(10), Object::Int(20), Object::Int(110)];
    src.update_dict(figure, |x| x.set(b"Dest".to_vec(), Object::Array(short))).unwrap();
    let tree = crate::outline_tree(&src, &[]).unwrap();
    let copied = &tree.nodes[0].children[1];
    assert_eq!((copied.page, copied.view.clone()), (Some(2), None));
    let mut target = doc_a();
    crate::import_outline(&mut target, &[], 0, &tree.nodes, false).unwrap();
    assert_eq!(titles(&crate::bookmarks(&target)), ["Intro[Part,Figure]", "Results"]);
}

#[test]
fn an_import_appends_inserts_or_replaces() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "Old A", 0).unwrap();
    crate::add_bookmark(&mut d, &[], 1, "Old B", 1).unwrap();
    let new = [node("New", Some(2), vec![node("Child", Some(1), vec![])])];
    assert_eq!(crate::import_outline(&mut d, &[], 1, &new, false).unwrap(), vec![1]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Old A", "New[Child]", "Old B"]);
    assert_eq!(crate::import_outline(&mut d, &[2], 9, &new, false).unwrap(), vec![2, 0]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Old A", "New[Child]", "Old B[New[Child]]"]);
    assert_back_links(&d);
    assert_eq!(crate::import_outline(&mut d, &[], 5, &new, true).unwrap(), vec![0]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["New[Child]"]);
    assert_back_links(&d);
    let refused = crate::import_outline(&mut d, &[0], 0, &new, true);
    assert!(matches!(refused, Err(crate::OutlineError::Organize(crate::OrganizeError::Invalid(_)))));
    assert_eq!(crate::import_outline(&mut d, &[], 0, &[], false), Err(crate::OutlineError::NothingToImport));
    assert_eq!(crate::import_outline(&mut d, &[7], 0, &new, false), Err(crate::OutlineError::NoSuchBookmark(vec![7])));
}

#[test]
fn a_bad_node_anywhere_refuses_the_whole_import() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "Kept", 0).unwrap();
    let before = crate::bookmarks(&d);
    let with = |f: &dyn Fn(&mut crate::OutlineNode)| {
        let mut child = node("Child", Some(0), vec![]);
        f(&mut child);
        vec![node("Fine", Some(0), vec![]), node("Parent", Some(0), vec![child])]
    };
    let bad_views: [(&[u8], Vec<Option<f64>>); 4] =
        [(b"XYZ", vec![Some(1.0)]), (b"Zoom", vec![]), (b"FitH", vec![Some(f64::NAN)]), (b"FitR", vec![Some(0.0), None, Some(1.0), Some(1.0)])];
    for (kind, values) in bad_views {
        let nodes = with(&|n| n.view = Some((kind.to_vec(), values.clone())));
        assert_eq!(crate::import_outline(&mut d, &[], 0, &nodes, false), Err(crate::OutlineError::BadView(vec![1, 0])), "{kind:?}");
    }
    let nodes = with(&|n| n.color = Some([0.5, 1.5, 0.0]));
    assert_eq!(crate::import_outline(&mut d, &[], 0, &nodes, false), Err(crate::OutlineError::BadColor));
    let nodes = with(&|n| n.color = Some([f32::NAN, 0.0, 0.0]));
    assert_eq!(crate::import_outline(&mut d, &[], 0, &nodes, false), Err(crate::OutlineError::BadColor));
    let mut deep = node("Leaf", None, vec![]);
    for _ in 0..33 {
        deep = node("Level", None, vec![deep]);
    }
    assert_eq!(crate::import_outline(&mut d, &[], 0, &[deep], false), Err(crate::OutlineError::TooDeep(33)));
    // 33 levels fit at the top, but not under a bookmark: the deepest would be past the listing.
    let mut chain = node("Leaf", None, vec![]);
    for _ in 0..32 {
        chain = node("Level", None, vec![chain]);
    }
    assert_eq!(crate::import_outline(&mut d, &[0], 0, std::slice::from_ref(&chain), false), Err(crate::OutlineError::TooDeep(33)));
    let many = vec![node("Item", None, vec![]); crate::MAX_IMPORT_BOOKMARKS + 1];
    assert_eq!(crate::import_outline(&mut d, &[], 0, &many, false), Err(crate::OutlineError::TooManyBookmarks(crate::MAX_IMPORT_BOOKMARKS)));
    assert_eq!(crate::bookmarks(&d), before);
}

#[test]
fn an_outline_deeper_than_the_listing_is_exported_as_truncated() {
    let tree = crate::outline_tree(&open(deep_outline(40)), &[]).unwrap();
    assert!(tree.truncated);
    assert!(!crate::outline_tree(&open(deep_outline(33)), &[]).unwrap().truncated);
    // What was exported imports back: 33 levels is the deepest an import takes.
    let mut target = doc_a();
    crate::import_outline(&mut target, &[], 0, &tree.nodes, false).unwrap();
    assert_eq!(crate::bookmarks(&target).len(), 33);
}
