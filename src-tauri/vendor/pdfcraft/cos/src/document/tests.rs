use super::*;

std::thread_local! {
    /// How many times `Document::scan_for` read the file on this thread.
    pub(super) static FILE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A scan that doesn't find an object isn't made again: each reads the whole file, and a file can
/// refer to such an object from every object it has.
#[test]
fn what_a_scan_of_the_file_did_not_find_is_remembered() {
    let body = b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\n";
    let mut out = body.to_vec();
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    out.extend_from_slice(format!("{:010} 00000 n \n", 9).as_bytes());
    out.extend_from_slice(format!("{:010} 00000 n \n", body.iter().position(|b| *b == b'\n').unwrap() + 49).as_bytes());
    out.extend_from_slice(b"0000000001 00000 n \n");
    out.extend_from_slice(format!("trailer << /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    let doc = Document::open(Arc::new(out)).unwrap();
    FILE_SCANS.with(|scans| scans.set(0));
    for _ in 0..5 {
        assert!(doc.try_get(3).is_err());
    }
    assert_eq!(FILE_SCANS.with(std::cell::Cell::get), 1);
    let clone = doc.clone();
    assert!(clone.try_get(3).is_err());
    assert_eq!(FILE_SCANS.with(std::cell::Cell::get), 1, "a clone has the same file");
}

/// An object can be nothing but a reference to another. A chain of them is followed, and one that
/// goes round ends in what isn't a dictionary or a name, without hanging.
#[test]
fn references_to_references_are_followed() {
    let mut doc = Document::new_empty();
    let name = doc.add(Object::name("Target"));
    let one = doc.add(Object::Ref(name));
    let two = doc.add(Object::Ref(one));
    assert_eq!(doc.resolve(&Object::Ref(two)).as_name(), Some(b"Target".as_slice()));
    let a = doc.add(Object::Null);
    let b = doc.add(Object::Ref(a));
    doc.set(a, Object::Ref(b));
    assert_eq!(doc.resolve(&Object::Ref(a)).as_name(), None);
}

/// An object stream's `/DecodeParms` can be given by reference: without them a predictor isn't
/// undone, so its members were lost.
#[test]
fn an_object_stream_with_parameters_by_reference_is_read() {
    let members = b"3 0 << /Hidden (yes) >>";
    // PNG Sub, one row.
    let mut rows = vec![1u8];
    let mut before = 0u8;
    for b in members {
        rows.push(b.wrapping_sub(before));
        before = *b;
    }
    let coded = pdfcraft_filters::encode_flate(&rows);
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offs = [0usize; 7];
    let mut obj = |out: &mut Vec<u8>, n: usize, body: &[u8]| {
        offs[n] = out.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, 2, b"<< /Type /Pages /Kids [] /Count 0 /Extra 3 0 R >>");
    let mut stm = format!("<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /DecodeParms 6 0 R /Length {} >>\nstream\n", coded.len()).into_bytes();
    stm.extend_from_slice(&coded);
    stm.extend_from_slice(b"\nendstream");
    obj(&mut out, 4, &stm);
    let data = [2u8, 0, 4, 0];
    let mut xs = format!("<< /Type /XRef /Size 7 /W [1 2 1] /Index [3 1] /Length {} >>\nstream\n", data.len()).into_bytes();
    xs.extend_from_slice(&data);
    xs.extend_from_slice(b"\nendstream");
    obj(&mut out, 5, &xs);
    obj(&mut out, 6, format!("<< /Predictor 11 /Columns {} >>", members.len()).as_bytes());
    let table = out.len();
    out.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for (n, off) in offs.iter().enumerate().skip(1) {
        let line = if n == 3 { "0000000000 65535 f \n".to_string() } else { format!("{off:010} 00000 n \n") };
        out.extend_from_slice(line.as_bytes());
    }
    out.extend_from_slice(format!("trailer << /Size 7 /Root 1 0 R /XRefStm {} >>\nstartxref\n{table}\n%%EOF\n", offs[5]).as_bytes());
    let doc = Document::open(Arc::new(out)).unwrap();
    let hidden = doc.get(ObjRef::new(3, 0));
    assert_eq!(hidden.as_dict().and_then(|d| d.get(b"Hidden")).and_then(Object::as_string).map(|s| s.bytes.clone()), Some(b"yes".to_vec()));
}

/// Each entry of an `/Encrypt` dictionary, and of a crypt filter in it, reads the same by reference.
/// Read as absent, `/StmF` or `/CFM` was `/Identity`, so a whole file's streams stayed ciphertext.
#[test]
fn encrypt_dictionary_entries_read_the_same_by_reference() {
    let string = |s: &str| Object::String(crate::PdfString::text(s));
    let mut filter = Dict::new();
    filter.set(b"CFM".to_vec(), Object::name("AESV2"));
    filter.set(b"Length".to_vec(), Object::Int(5));
    let cf = |filter: Dict| {
        let mut cf = Dict::new();
        cf.set(b"StdCF".to_vec(), Object::Dict(filter));
        cf.set(b"Other".to_vec(), Object::Dict(Dict::new()));
        Object::Dict(cf)
    };
    let mut d = Dict::new();
    let entries = [
        ("Filter", Object::name("Standard")),
        ("V", Object::Int(4)),
        ("R", Object::Int(4)),
        ("Length", Object::Int(128)),
        ("P", Object::Int(-44)),
        ("O", string("owner")),
        ("U", string("user")),
        ("EncryptMetadata", Object::Bool(false)),
        ("CF", cf(filter.clone())),
        ("StmF", Object::name("StdCF")),
        ("StrF", Object::name("Other")),
        ("EFF", Object::name("StdCF")),
    ];
    for (key, value) in &entries {
        d.set(key.as_bytes().to_vec(), value.clone());
    }
    let mut doc = Document::new_empty();
    let direct = crate::security::encrypt_dict(&doc, &d);
    assert_eq!((direct.stm_f.as_slice(), direct.str_f.as_slice(), direct.ef_f.as_slice()), (&b"StdCF"[..], &b"Other"[..], &b"StdCF"[..]));
    assert_eq!(direct.length_bits, 40, "the filter's own length, in bytes");
    assert!(!direct.encrypt_metadata);
    assert_eq!(direct.crypt_filters.iter().filter(|(_, m)| *m == pdfcraft_crypt::Method::Aes128).count(), 1);
    for (key, value) in &entries {
        let mut one = d.clone();
        let r = doc.add(value.clone());
        one.set(key.as_bytes().to_vec(), Object::Ref(r));
        assert_eq!(crate::security::encrypt_dict(&doc, &one), direct, "/{key}");
    }
    for key in ["CFM", "Length"] {
        let mut by_reference = filter.clone();
        let r = doc.add(filter.get(key.as_bytes()).unwrap().clone());
        by_reference.set(key.as_bytes().to_vec(), Object::Ref(r));
        let mut one = d.clone();
        one.set(b"CF".to_vec(), cf(by_reference));
        assert_eq!(crate::security::encrypt_dict(&doc, &one), direct, "/{key} in the filter");
    }
}

/// /DecodeParms given by reference, whole, as array entries or as the numbers inside, are read
/// once made direct: here a Flate predictor, which the stream alone doesn't see. Only the entries the filters use are
/// resolved, and nothing is for a chain longer than a decoder takes.
#[test]
fn decode_parms_given_by_reference_are_read_once_direct() {
    let rows = [1u8, 2, 3, 4, 5, 6, 7, 8];
    let params = pdfcraft_filters::Params { predictor: 12, colors: 1, bits_per_component: 8, columns: 4, early_change: 1 };
    let coded = pdfcraft_filters::encode(&pdfcraft_filters::Filter::Flate, &params, &rows).unwrap();
    let mut parms = Dict::new();
    parms.set(b"Predictor".to_vec(), Object::Int(12));
    parms.set(b"Columns".to_vec(), Object::Int(4));
    let mut doc = Document::new_empty();
    let r = doc.add(Object::Dict(parms.clone()));
    let array = doc.add(Object::Array(vec![Object::Ref(r)]));
    let stream = |filters: usize, p: Object| {
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::Array(vec![Object::name("FlateDecode"); filters]));
        d.set(b"DecodeParms".to_vec(), p);
        Stream::from_raw(d, coded.clone())
    };
    for p in [Object::Ref(r), Object::Array(vec![Object::Ref(r)]), Object::Ref(array)] {
        let s = stream(1, p.clone());
        assert_ne!(s.decoded().unwrap(), rows, "{p:?} read as it is");
        assert_eq!(doc.with_direct_parms(&s).decoded().unwrap(), rows, "{p:?}");
    }
    // A number inside given by reference, which `Dict::int` doesn't follow: in the dictionary
    // itself, and in an array's entry.
    let twelve = doc.add(Object::Int(12));
    let mut inner = parms.clone();
    inner.set(b"Predictor".to_vec(), Object::Ref(twelve));
    for p in [Object::Dict(inner.clone()), Object::Array(vec![Object::Dict(inner)])] {
        let s = stream(1, p.clone());
        assert_ne!(s.decoded().unwrap(), rows, "{p:?} read as it is");
        assert_eq!(doc.with_direct_parms(&s).decoded().unwrap(), rows, "{p:?}");
    }
    // Only numbers and flags are copied: a key naming a big array, or an entry that is no
    // dictionary, is left out (a file could name one big object from every key and filter).
    let big = doc.add(Object::Array(vec![Object::Int(1); 100_000]));
    let mut hostile = Dict::new();
    hostile.set(b"Predictor".to_vec(), Object::Ref(big));
    hostile.set(b"Columns".to_vec(), Object::Ref(big));
    hostile.set(b"Colors".to_vec(), Object::Ref(twelve));
    let entry = doc.add(Object::Dict(hostile));
    let two = stream(2, Object::Array(vec![Object::Ref(entry), Object::Ref(big)]));
    let made = doc.with_direct_parms(&two);
    let made = made.dict.get(b"DecodeParms").and_then(Object::as_array).unwrap();
    let first = made[0].as_dict().unwrap();
    assert_eq!((first.contains(b"Predictor"), first.contains(b"Columns"), first.int(b"Colors")), (false, false, Some(12)));
    assert!(matches!(made[1], Object::Null), "{:?}", made[1]);
    let direct = stream(1, Object::Dict(parms));
    assert!(matches!(doc.with_direct_parms(&direct), Cow::Borrowed(_)), "nothing to resolve");
    assert_eq!(direct.decoded().unwrap(), rows);
    // Entries past the filters' are left out, unresolved; so is everything past the chain limit.
    let many = stream(1, Object::Array(vec![Object::Ref(r); 1000]));
    let made = doc.with_direct_parms(&many);
    assert_eq!(made.dict.get(b"DecodeParms").and_then(Object::as_array).map(Vec::len), Some(1));
    let long = stream(pdfcraft_filters::MAX_CHAIN + 1, Object::Array(vec![Object::Ref(r); 9]));
    assert!(matches!(doc.with_direct_parms(&long), Cow::Borrowed(_)));
    // A chain as long as a decoder takes is resolved, and a `/Crypt` in it doesn't count, as
    // the decoders leave it out: ASCIIHex as often as the limit allows, then the predictor's
    // Flate, its parameters by reference.
    let hex = |b: &[u8]| b.iter().flat_map(|x| format!("{x:02X}").into_bytes()).chain(*b">").collect::<Vec<u8>>();
    let wrapped = (1..pdfcraft_filters::MAX_CHAIN).fold(coded.clone(), |data, _| hex(&data));
    for crypt in [false, true] {
        let mut names = vec![Object::name("ASCIIHexDecode"); pdfcraft_filters::MAX_CHAIN - 1];
        names.push(Object::name("FlateDecode"));
        if crypt {
            names.insert(0, Object::name("Crypt"));
        }
        let mut entries = vec![Object::Null; names.len() - 1];
        entries.push(Object::Ref(r));
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::Array(names));
        d.set(b"DecodeParms".to_vec(), Object::Array(entries));
        let s = Stream::from_raw(d, wrapped.clone());
        assert_eq!(doc.with_direct_parms(&s).decoded().unwrap(), rows, "with /Crypt: {crypt}");
    }
}

/// A CCITT fax image reads through its /DecodeParms, given directly, in an array or by
/// reference (its numbers too), with /BlackIs1, and behind other filters; one whose /Columns
/// aren't its /Width is refused, /Height rows are decoded whatever /Rows says, and other images
/// decode through their filters.
#[test]
fn fax_images_decode_with_their_parameters() {
    // 10 × 3: the first five samples of each row white, the rest black.
    let bits = [0b1111_1000, 0, 0b1111_1000, 0, 0b1111_1000, 0];
    let coded = pdfcraft_filters::encode_g4(&bits, 10, 3).unwrap();
    let mut doc = Document::new_empty();
    let mut parms = Dict::new();
    parms.set(b"K".to_vec(), Object::Int(-1));
    parms.set(b"Columns".to_vec(), Object::Int(10));
    let indirect = doc.add(Object::Dict(parms.clone()));
    let image = |doc: &Document, parms: Option<Object>, width: i64, height: i64| {
        let mut d = Dict::new();
        d.set(b"Width".to_vec(), Object::Int(width));
        d.set(b"Height".to_vec(), Object::Int(height));
        d.set(b"Filter".to_vec(), Object::name("CCITTFaxDecode"));
        if let Some(p) = parms {
            d.set(b"DecodeParms".to_vec(), p);
        }
        doc.decoded_image(&Stream::from_raw(d, coded.clone()))
    };
    for p in [
        Object::Dict(parms.clone()),
        Object::Array(vec![Object::Dict(parms.clone())]),
        Object::Ref(indirect),
        Object::Array(vec![Object::Ref(indirect)]),
    ] {
        assert_eq!(image(&doc, Some(p.clone()), 10, 3).unwrap(), bits, "{p:?}");
    }
    let mut inverted = parms.clone();
    inverted.set(b"BlackIs1".to_vec(), Object::Bool(true));
    assert_eq!(
        image(&doc, Some(Object::Dict(inverted)), 10, 3).unwrap(),
        [0b0000_0111, 0b1100_0000, 0b0000_0111, 0b1100_0000, 0b0000_0111, 0b1100_0000]
    );
    assert_eq!(image(&doc, Some(Object::Dict(parms.clone())), 10, 2).unwrap(), bits[..4], "at most /Height rows");
    // Within a bound: no more rows than it holds.
    let mut bounded = Dict::new();
    bounded.set(b"Width".to_vec(), Object::Int(10));
    bounded.set(b"Height".to_vec(), Object::Int(3));
    bounded.set(b"Filter".to_vec(), Object::name("CCITTFaxDecode"));
    bounded.set(b"DecodeParms".to_vec(), Object::Dict(parms.clone()));
    assert_eq!(doc.decoded_image_within(&Stream::from_raw(bounded, coded.clone()), 5).unwrap(), bits[..4], "two rows of two bytes in five");
    let mut by_reference = Dict::new();
    by_reference.set(b"K".to_vec(), Object::Ref(doc.add(Object::Int(-1))));
    by_reference.set(b"Columns".to_vec(), Object::Ref(doc.add(Object::Int(10))));
    assert_eq!(image(&doc, Some(Object::Dict(by_reference)), 10, 3).unwrap(), bits, "/K and /Columns by reference");
    let mut understated = parms.clone();
    understated.set(b"Rows".to_vec(), Object::Int(1));
    assert_eq!(image(&doc, Some(Object::Dict(understated.clone())), 10, 3).unwrap(), bits, "/Height rows, whatever /Rows says");
    // The parameters as the file gives them, /Rows included, and Table 11's defaults.
    let mut d = Dict::new();
    d.set(b"Filter".to_vec(), Object::name("CCITTFaxDecode"));
    d.set(b"DecodeParms".to_vec(), Object::Dict(understated));
    let p = doc.fax_params(&Stream::from_raw(d.clone(), Vec::new())).unwrap();
    assert_eq!((p.k, p.columns, p.rows, p.end_of_block, p.black_is_1), (-1, 10, 1, true, false));
    d.set(b"Filter".to_vec(), Object::name("FlateDecode"));
    assert!(doc.fax_params(&Stream::from_raw(d, Vec::new())).is_none(), "not a fax image");
    assert!(matches!(image(&doc, Some(Object::Dict(parms.clone())), 12, 3), Err(CosError::Filter(_))), "/Columns other than /Width");
    assert!(matches!(image(&doc, None, 10, 3), Err(CosError::Filter(_))), "the default 1728 /Columns");
    let mut plain = Dict::new();
    plain.set(b"Filter".to_vec(), Object::name("FlateDecode"));
    assert_eq!(doc.decoded_image(&Stream::from_raw(plain, pdfcraft_filters::encode_flate(&bits))).unwrap(), bits);

    // Behind other filters, its parameters at its own place in the array (/Crypt counts).
    let behind = |filters: &[Object], parms: &[Object], data: Vec<u8>| {
        let mut d = Dict::new();
        d.set(b"Width".to_vec(), Object::Int(10));
        d.set(b"Height".to_vec(), Object::Int(3));
        d.set(b"Filter".to_vec(), Object::Array(filters.to_vec()));
        d.set(b"DecodeParms".to_vec(), Object::Array(parms.to_vec()));
        doc.decoded_image(&Stream::from_raw(d, data))
    };
    let (fax, null, own) = (Object::name("CCITTFaxDecode"), Object::Null, Object::Dict(parms.clone()));
    let hex = coded.iter().map(|b| format!("{b:02x}")).chain([">".to_string()]).collect::<String>().into_bytes();
    let flated = pdfcraft_filters::encode_flate(&coded);
    let ahx = [Object::name("AHx"), fax.clone()];
    assert_eq!(behind(&ahx, &[null.clone(), Object::Ref(indirect)], hex.clone()).unwrap(), bits, "behind ASCIIHexDecode");
    assert_eq!(
        behind(&[Object::name("FlateDecode"), Object::name("CCF")], &[null.clone(), own.clone()], flated.clone()).unwrap(),
        bits,
        "behind Flate"
    );
    assert_eq!(behind(&[Object::name("Crypt"), fax.clone()], &[null.clone(), own.clone()], coded.clone()).unwrap(), bits, "after /Crypt");
    // A /Crypt belongs first (§7.4.10); one after the fax filter leaves it its own entry.
    assert_eq!(behind(&[fax.clone(), Object::name("Crypt")], &[own.clone(), null.clone()], coded.clone()).unwrap(), bits, "before /Crypt");
    assert!(behind(&ahx, &[own.clone(), null.clone()], hex).is_err(), "the first entry is the other filter's");
    assert!(behind(&[fax.clone(), Object::name("FlateDecode")], &[own.clone(), null.clone()], flated).is_err(), "CCITT, then Flate");
    assert!(behind(&[Object::Int(5), fax], &[null, own], coded.clone()).is_err(), "a /Filter entry that isn't a name");
}

/// Images that share a `/JBIG2Globals` stream of the file decode it once a document: it is kept
/// for the next, in every clone. A globals stream edited since is read afresh (the kept one is the
/// file's), and objects added in memory aren't kept.
#[test]
fn shared_jbig2_globals_are_read_once() {
    let bits = [0b1111_1000, 0, 0b1111_1000, 0, 0b1111_1000, 0];
    let coded = pdfcraft_filters::encode_jbig2_generic(&bits, 10, 3).unwrap();
    let mut doc = Document::new_empty();
    let mut d = Dict::new();
    d.set(b"Filter".to_vec(), Object::name("FlateDecode"));
    let skipped = [0, 0, 0, 0, 0x3E, 0, 0, 0, 0, 0, 4, 0, 0, 0, 1];
    let globals = doc.add(Object::Stream(Stream::from_raw(d, pdfcraft_filters::encode_flate(&skipped))));
    let image = |doc: &mut Document| {
        let mut d = Dict::new();
        d.set(b"Width".to_vec(), Object::Int(10));
        d.set(b"Height".to_vec(), Object::Int(3));
        d.set(b"Filter".to_vec(), Object::name("JBIG2Decode"));
        let mut parms = Dict::new();
        parms.set(b"JBIG2Globals".to_vec(), Object::Ref(globals));
        d.set(b"DecodeParms".to_vec(), Object::Dict(parms));
        doc.add(Object::Stream(Stream::from_raw(d, coded.clone())))
    };
    let (a, b) = (image(&mut doc), image(&mut doc));
    let stream = |doc: &Document, r: ObjRef| match &*doc.get(r) {
        Object::Stream(s) => s.clone(),
        _ => panic!("not a stream"),
    };
    assert_eq!(doc.decoded_image(&stream(&doc, a)).unwrap(), bits);
    assert!(doc.jbig2_globals.lock().unwrap().is_empty(), "an object added in memory isn't kept");

    // The same objects read from a file, the catalog holding on to the images (a full save may
    // number them anew).
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"Images".to_vec(), Object::Array(vec![Object::Ref(a), Object::Ref(b)]))).unwrap();
    let file = crate::write_full(&doc, &crate::SaveOptions::default()).unwrap();
    let mut doc = Document::open(Arc::new(file)).unwrap();
    let images = doc.get(doc.root().unwrap()).as_dict().unwrap().get(b"Images").unwrap().as_array().unwrap().clone();
    let (a, b) = (images[0].as_ref().unwrap(), images[1].as_ref().unwrap());
    let parms = stream(&doc, a).dict.get(b"DecodeParms").unwrap().as_dict().unwrap().clone();
    let globals = parms.reference(b"JBIG2Globals").unwrap();
    let clone = doc.clone();
    assert_eq!(doc.decoded_image(&stream(&doc, a)).unwrap(), bits);
    assert_eq!(doc.jbig2_globals.lock().unwrap().len(), 1, "kept");
    let kept = doc.jbig2_globals.lock().unwrap()[&globals.num].clone();
    assert_eq!(clone.decoded_image(&stream(&clone, b)).unwrap(), bits, "the clone's other image");
    let shared = clone.jbig2_globals.lock().unwrap()[&globals.num].clone();
    assert!(Arc::ptr_eq(&kept, &shared), "the clone uses the one kept, not a second decode");
    doc.set(globals, Object::Stream(Stream::from_raw(Dict::new(), b"not JBIG2".to_vec())));
    assert!(doc.decoded_image(&stream(&doc, b)).is_err(), "the edited globals are read afresh");
    assert_eq!(clone.decoded_image(&stream(&clone, b)).unwrap(), bits, "the clone keeps the file's");
}

/// A JBIG2 image's size is read as given directly, as every caller reads it first, and an image
/// with neither is an error, not an image of no rows.
#[test]
fn a_jbig2_image_needs_its_size_given_directly() {
    let coded = pdfcraft_filters::encode_jbig2_generic(&[0xFF, 0xFF], 8, 2).unwrap();
    let mut doc = Document::new_empty();
    let size = doc.add(Object::Int(2));
    let image = |width: Object, height: Object| {
        let mut d = Dict::new();
        d.set(b"Width".to_vec(), width);
        d.set(b"Height".to_vec(), height);
        d.set(b"Filter".to_vec(), Object::name("JBIG2Decode"));
        doc.decoded_image(&Stream::from_raw(d, coded.clone()))
    };
    assert!(image(Object::Int(8), Object::Int(2)).is_ok());
    assert!(image(Object::Ref(size), Object::Int(2)).is_err(), "/Width by reference");
    assert!(image(Object::Int(8), Object::Ref(size)).is_err(), "/Height by reference");
}

/// A document that kept globals of 64 MiB keeps no more, and the images past that still read:
/// three streams of 22 MiB, shared by three images, are two kept and one decoded each time.
#[test]
fn kept_jbig2_globals_are_bounded_in_total() {
    let bits = [0b1111_1000, 0, 0b1111_1000, 0, 0b1111_1000, 0];
    let coded = pdfcraft_filters::encode_jbig2_generic(&bits, 10, 3).unwrap();
    let length = 22 * 1024 * 1024;
    let mut skipped = vec![0, 0, 0, 0, 0x3E, 0, 0];
    skipped.extend_from_slice(&u32::try_from(4 + length).unwrap().to_be_bytes());
    skipped.extend_from_slice(&[0, 0, 0, 1]);
    skipped.resize(skipped.len() + length, 0);
    let mut doc = Document::new_empty();
    let mut images = Vec::new();
    for _ in 0..3 {
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::name("FlateDecode"));
        let globals = doc.add(Object::Stream(Stream::from_raw(d, pdfcraft_filters::encode_flate(&skipped))));
        let mut d = Dict::new();
        d.set(b"Width".to_vec(), Object::Int(10));
        d.set(b"Height".to_vec(), Object::Int(3));
        d.set(b"Filter".to_vec(), Object::name("JBIG2Decode"));
        let mut parms = Dict::new();
        parms.set(b"JBIG2Globals".to_vec(), Object::Ref(globals));
        d.set(b"DecodeParms".to_vec(), Object::Dict(parms));
        images.push(Object::Ref(doc.add(Object::Stream(Stream::from_raw(d, coded.clone())))));
    }
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"Images".to_vec(), Object::Array(images))).unwrap();
    let doc = Document::open(Arc::new(crate::write_full(&doc, &crate::SaveOptions::default()).unwrap())).unwrap();
    let images = doc.get(doc.root().unwrap()).as_dict().unwrap().get(b"Images").unwrap().as_array().unwrap().clone();
    for image in &images {
        let Object::Stream(s) = &*doc.get(image.as_ref().unwrap()) else { panic!("not a stream") };
        assert_eq!(doc.decoded_image(s).unwrap(), bits, "every image reads");
    }
    let kept = doc.jbig2_globals.lock().unwrap();
    assert_eq!(kept.len(), 2, "two fit in 64 MiB, a third doesn't");
    assert!(kept.values().all(|g| g.capacity() == g.len()), "what is kept holds no more than it counts");
}

/// Globals whose `/DecodeParms` are given by reference aren't kept: that's another object, so
/// editing it changes what they decode to while the stream itself, the key kept by, stays the same.
#[test]
fn jbig2_globals_with_parameters_by_reference_are_read_afresh() {
    let bits = [0b1111_1000, 0, 0b1111_1000, 0, 0b1111_1000, 0];
    let coded = pdfcraft_filters::encode_jbig2_generic(&bits, 10, 3).unwrap();
    let skipped = [0, 0, 0, 0, 0x3E, 0, 0, 0, 0, 0, 4, 0, 0, 0, 1];
    let params = pdfcraft_filters::Params { predictor: 12, colors: 1, bits_per_component: 8, columns: 15, early_change: 1 };
    let mut doc = Document::new_empty();
    let mut parms = Dict::new();
    parms.set(b"Predictor".to_vec(), Object::Int(12));
    parms.set(b"Columns".to_vec(), Object::Int(15));
    let parms = doc.add(Object::Dict(parms));
    let mut d = Dict::new();
    d.set(b"Filter".to_vec(), Object::name("FlateDecode"));
    d.set(b"DecodeParms".to_vec(), Object::Ref(parms));
    let predicted = pdfcraft_filters::encode(&pdfcraft_filters::Filter::Flate, &params, &skipped).unwrap();
    let globals = doc.add(Object::Stream(Stream::from_raw(d, predicted)));
    let mut d = Dict::new();
    d.set(b"Width".to_vec(), Object::Int(10));
    d.set(b"Height".to_vec(), Object::Int(3));
    d.set(b"Filter".to_vec(), Object::name("JBIG2Decode"));
    let mut image_parms = Dict::new();
    image_parms.set(b"JBIG2Globals".to_vec(), Object::Ref(globals));
    d.set(b"DecodeParms".to_vec(), Object::Dict(image_parms));
    let image = doc.add(Object::Stream(Stream::from_raw(d, coded)));
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"Image".to_vec(), Object::Ref(image))).unwrap();
    let mut doc = Document::open(Arc::new(crate::write_full(&doc, &crate::SaveOptions::default()).unwrap())).unwrap();
    let image = doc.get(doc.root().unwrap()).as_dict().unwrap().reference(b"Image").unwrap();
    let stream = |doc: &Document| match &*doc.get(image) {
        Object::Stream(s) => s.clone(),
        _ => panic!("not a stream"),
    };
    assert_eq!(doc.decoded_image(&stream(&doc)).unwrap(), bits);
    assert!(doc.jbig2_globals.lock().unwrap().is_empty(), "not kept");
    let globals = stream(&doc).dict.get(b"DecodeParms").unwrap().as_dict().unwrap().reference(b"JBIG2Globals").unwrap();
    let Object::Stream(g) = &*doc.get(globals) else { panic!("not a stream") };
    let parms = g.dict.reference(b"DecodeParms").unwrap();
    doc.set(parms, Object::Dict(Dict::new()));
    assert!(doc.decoded_image(&stream(&doc)).is_err(), "without the predictor they read otherwise");
}

/// A JBIG2 image reads through its decoder, behind other filters too, with the `/JBIG2Globals` of
/// its own `/DecodeParms` (a dictionary or its place in an array, /Crypt counting, by reference
/// too): globals that can't be read are an error, and ones at another filter's place aren't its
/// own, nor are those of a lone dictionary when other filters come first. Its page has to be its
/// /Width × /Height, and fit the bound.
#[test]
fn jbig2_images_decode_with_their_globals() {
    // 10 × 3: the first five samples of each row white, the rest black.
    let bits = [0b1111_1000, 0, 0b1111_1000, 0, 0b1111_1000, 0];
    let coded = pdfcraft_filters::encode_jbig2_generic(&bits, 10, 3).unwrap();
    let hex = coded.iter().map(|b| format!("{b:02x}")).chain([">".to_string()]).collect::<String>().into_bytes();
    let mut doc = Document::new_empty();
    // An extension segment (ISO/IEC 14492 7.4.14), which a decoder skips, Flate-encoded; and data
    // that isn't JBIG2.
    let globals = |doc: &mut Document, data: &[u8]| {
        let mut d = Dict::new();
        d.set(b"Filter".to_vec(), Object::name("FlateDecode"));
        let globals = doc.add(Object::Stream(Stream::from_raw(d, pdfcraft_filters::encode_flate(data))));
        let mut parms = Dict::new();
        parms.set(b"JBIG2Globals".to_vec(), Object::Ref(globals));
        Object::Dict(parms)
    };
    let (good, bad) = (globals(&mut doc, &[0, 0, 0, 0, 0x3E, 0, 0, 0, 0, 0, 4, 0, 0, 0, 1]), globals(&mut doc, b"not JBIG2"));
    let indirect = Object::Ref(doc.add(bad.clone()));
    let image = |filters: &[&str], parms: Option<Object>, data: &[u8], (width, height): (i64, i64), max: usize| {
        let mut d = Dict::new();
        d.set(b"Width".to_vec(), Object::Int(width));
        d.set(b"Height".to_vec(), Object::Int(height));
        d.set(b"Filter".to_vec(), Object::Array(filters.iter().map(|f| Object::name(f)).collect()));
        if let Some(p) = parms {
            d.set(b"DecodeParms".to_vec(), p);
        }
        doc.decoded_image_within(&Stream::from_raw(d, data.to_vec()), max)
    };
    let jbig2 = ["JBIG2Decode"];
    let full = (10, 3);
    assert_eq!(image(&jbig2, None, &coded, full, MAX_DECODED).unwrap(), bits);
    assert_eq!(image(&jbig2, Some(good.clone()), &coded, full, MAX_DECODED).unwrap(), bits, "globals it skips");
    for parms in [bad.clone(), Object::Array(vec![bad.clone()]), indirect.clone(), Object::Array(vec![indirect])] {
        let e = image(&jbig2, Some(parms.clone()), &coded, full, MAX_DECODED).unwrap_err();
        assert!(e.to_string().contains("JBIG2"), "{parms:?}: {e}");
    }
    let behind = ["ASCIIHexDecode", "JBIG2Decode"];
    assert_eq!(image(&behind, None, &hex, full, MAX_DECODED).unwrap(), bits, "behind ASCIIHexDecode");
    assert!(image(&behind, Some(Object::Array(vec![Object::Null, bad.clone()])), &hex, full, MAX_DECODED).is_err(), "its own globals");
    assert_eq!(image(&behind, Some(Object::Array(vec![bad.clone(), Object::Null])), &hex, full, MAX_DECODED).unwrap(), bits, "another's");
    assert_eq!(image(&behind, Some(bad.clone()), &hex, full, MAX_DECODED).unwrap(), bits, "a lone dictionary, the first filter's");
    let after_crypt = ["Crypt", "JBIG2Decode"];
    assert!(image(&after_crypt, Some(Object::Array(vec![Object::Null, bad.clone()])), &coded, full, MAX_DECODED).is_err(), "after /Crypt");
    assert_eq!(image(&after_crypt, Some(Object::Array(vec![bad, Object::Null])), &coded, full, MAX_DECODED).unwrap(), bits, "/Crypt's");

    for size in [(11, 3), (9, 3), (10, 2), (10, 4)] {
        assert!(image(&jbig2, None, &coded, size, MAX_DECODED).is_err(), "a page other than {size:?}");
    }
    assert_eq!(image(&jbig2, None, &coded, full, 6).unwrap(), bits, "the page within the bound");
    assert!(image(&jbig2, None, &coded, full, 5).is_err(), "the page past the bound");
}

#[test]
fn a_missing_file_id_is_derived_from_the_encryption_seed() {
    let mut doc = Document::new_empty();
    let seed = [9u8; 32];
    doc.set_encryption(&pdfcraft_crypt::NewEncryption {
        algorithm: pdfcraft_crypt::Algorithm::Aes256,
        user_password: "user",
        owner_password: "owner",
        permissions: -4,
        encrypt_metadata: true,
        seed,
    })
    .unwrap();
    let id = doc.trailer().get(b"ID").and_then(|o| o.as_array()).and_then(|a| a.first()).and_then(|o| o.as_string()).map(|s| s.bytes.clone());
    assert_eq!(id.as_deref(), Some(pdfcraft_crypt::file_id(&seed).as_slice()));

    let mut kept = Document::new_empty();
    let fixed = Object::String(crate::PdfString { bytes: b"0123456789abcdef".to_vec(), hex: true });
    kept.trailer_mut().set(b"ID".to_vec(), Object::Array(vec![fixed.clone(), fixed]));
    kept.set_encryption(&pdfcraft_crypt::NewEncryption {
        algorithm: pdfcraft_crypt::Algorithm::Aes256,
        user_password: "user",
        owner_password: "owner",
        permissions: -4,
        encrypt_metadata: true,
        seed,
    })
    .unwrap();
    let still = kept.trailer().get(b"ID").and_then(|o| o.as_array()).and_then(|a| a.first()).and_then(|o| o.as_string()).map(|s| s.bytes.clone());
    assert_eq!(still.as_deref(), Some(b"0123456789abcdef".as_slice()));
}

#[test]
fn a_read_state_matches_a_clone_and_no_changed_document() {
    let mut doc = Document::new_empty();
    let a = doc.add(Object::Int(1));
    let state = doc.read_state();
    let snapshot = doc.clone();
    assert!(state.matches(&doc) && state.matches(&snapshot));
    // The same value set again is still a change: another allocation.
    doc.set(a, Object::Int(1));
    assert!(!state.matches(&doc) && state.matches(&snapshot));
    let edited = doc.read_state();
    doc.free(a);
    assert!(!edited.matches(&doc));
    let mut trailer_changed = snapshot.clone();
    trailer_changed.trailer_mut().set(b"Info".to_vec(), Object::Null);
    assert!(!state.matches(&trailer_changed));
    // Another document's state never matches, even with the same changes.
    let mut other = Document::new_empty();
    other.add(Object::Int(1));
    assert!(!other.read_state().matches(&snapshot));
}

/// Build a classic-xref file from object bodies (object i+1 = bodies[i]).
pub(crate) fn build(bodies: &[&str], trailer: &str) -> Vec<u8> {
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

fn assert_compact_matches_materialized(doc: &Document) {
    assert!(matches!(doc.entries.as_ref(), XrefIndex::Compact { .. }), "valid xrefs should not retain a per-entry map");
    let (entries, _, revisions, _, _) = doc.read_xref_chain_materialized(&mut Vec::new()).unwrap();
    assert_eq!(revisions.as_slice(), doc.revisions());
    let max = entries.keys().next_back().copied().unwrap_or(0);
    assert_eq!(doc.entries.max_num(), max);
    for num in 0..=max {
        assert_eq!(doc.xref_entry(num), entries.get(&num).copied(), "object {num}");
    }
    let expected: Vec<_> = entries.iter().filter_map(|(&num, entry)| (!matches!(entry, XrefEntry::Free { .. })).then_some(num)).collect();
    assert_eq!(doc.object_numbers(), expected);
}

#[test]
fn compact_xref_tables_streams_and_revisions_match_the_materialized_reader() {
    let classic = Document::open(Arc::new(build(
        &["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "<< /Value 9 >>"],
        "/Root 1 0 R",
    )))
    .unwrap();
    assert_compact_matches_materialized(&classic);

    let mut sparse = b"%PDF-1.7\n".to_vec();
    let root_offset = sparse.len();
    sparse.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 5 0 R >>\nendobj\n");
    let pages_offset = sparse.len();
    sparse.extend_from_slice(b"5 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let xref = sparse.len();
    sparse.extend_from_slice(
        format!(
            "xref\n0 1\n0000000000 65535 f \n1 1\n{root_offset:010} 00000 n \n5 1\n{pages_offset:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let sparse = Document::open(Arc::new(sparse)).unwrap();
    assert_eq!(sparse.object_numbers(), vec![1, 5]);
    assert_compact_matches_materialized(&sparse);

    let mut sparse_stream = b"%PDF-1.7\n".to_vec();
    let mut offsets = [0u32; 8];
    offsets[1] = sparse_stream.len() as u32;
    sparse_stream.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 5 0 R >>\nendobj\n");
    offsets[5] = sparse_stream.len() as u32;
    sparse_stream.extend_from_slice(b"5 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    offsets[6] = sparse_stream.len() as u32;
    sparse_stream.extend_from_slice(b"6 0 obj\n<< /Unused true >>\nendobj\n");
    offsets[7] = sparse_stream.len() as u32;
    let xref_row = |kind: u8, offset: u32, generation: u16| {
        let mut row = Vec::with_capacity(7);
        row.push(kind);
        row.extend_from_slice(&offset.to_be_bytes());
        row.extend_from_slice(&generation.to_be_bytes());
        row
    };
    let mut rows = Vec::new();
    rows.extend(xref_row(0, 0, u16::MAX));
    rows.extend(xref_row(1, offsets[1], 0));
    rows.extend(xref_row(1, offsets[5], 0));
    rows.extend(xref_row(1, offsets[6], 0));
    rows.extend(xref_row(1, offsets[7], 0));
    sparse_stream.extend_from_slice(
        format!("7 0 obj\n<< /Type /XRef /Size 8 /Root 1 0 R /W [1 4 2] /Index [0 2 5 3] /Length {} >>\nstream\n", rows.len()).as_bytes(),
    );
    sparse_stream.extend_from_slice(&rows);
    sparse_stream.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{}\n%%EOF\n", offsets[7]).as_bytes());
    let sparse_stream = Document::open(Arc::new(sparse_stream)).unwrap();
    assert_eq!(sparse_stream.object_numbers(), vec![1, 5, 6, 7]);
    assert_compact_matches_materialized(&sparse_stream);

    let stream_bytes = crate::write_full(&classic, &crate::SaveOptions::default()).unwrap();
    let stream = Document::open(Arc::new(stream_bytes)).unwrap();
    assert!(stream.revisions().last().is_some_and(|revision| revision.is_stream));
    assert_compact_matches_materialized(&stream);

    let mut edited = classic.clone();
    edited.update_dict(ObjRef::new(1, 0), |dict| dict.set(b"Lang".to_vec(), Object::String(crate::PdfString::literal("en")))).unwrap();
    edited.free(ObjRef::new(3, 0));
    let incremental = crate::write_incremental(&edited, &crate::SaveOptions::default()).unwrap();
    let updated = Document::open(Arc::new(incremental)).unwrap();
    assert_eq!(updated.revisions().len(), 2);
    assert_compact_matches_materialized(&updated);
}

#[test]
fn malformed_size_uses_the_materialized_compatibility_path() {
    let mut bytes = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "<< /Value 9 >>"], "/Root 1 0 R");
    let marker = b"/Size 4";
    let at = bytes.windows(marker.len()).position(|window| window == marker).unwrap();
    bytes[at..at + marker.len()].copy_from_slice(b"/Size 2");
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert!(matches!(doc.entries.as_ref(), XrefIndex::Materialized(..)));
    assert_eq!(doc.next_num, 4);
    assert_eq!(doc.object_numbers(), vec![1, 2, 3]);
}

/// Entries the reader sets aside are remembered, so a caller comparing revisions can tell
/// that another reader may resolve the object differently.
#[test]
fn cross_reference_entries_that_are_set_aside_are_recorded() {
    let base = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "(three)"], "/Root 1 0 R");
    let plain = Document::open(Arc::new(base.clone())).unwrap();
    assert!(plain.xref_oddities().is_empty());
    assert!((1..=3).all(|n| plain.xref_entry_is_exact(n)));
    let prev = String::from_utf8_lossy(&base).rfind("xref\n0 4").unwrap();
    let third = plain.xref_entry(3).unwrap();

    // A later section: object 3 "in use" at offset 0, object 2 listed twice with different
    // offsets, and object 0 at offset 0 (the usual producer slip, which names no object).
    let mut bytes = base.clone();
    let section = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 1\n0000000000 65535 n \n3 1\n0000000000 00000 n \n2 1\n0000000058 00000 n \n2 1\n0000000009 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R /Prev {prev} >>\nstartxref\n{section}\n%%EOF\n").as_bytes(),
    );
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert_eq!(doc.xref_oddities(), [(section as u64, 3), (section as u64, 2)]);
    // What the reader resolves is what it resolved before: the first entry it kept.
    assert_eq!(doc.xref_entry(3), Some(third));
    assert_eq!(doc.xref_entry(2), Some(XrefEntry::InFile { offset: 58, generation: 0 }));
}

/// An entry that misses its object still resolves (by searching) but is not exact.
#[test]
fn an_entry_that_misses_its_object_is_found_by_searching_but_is_not_exact() {
    let base = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "(three)"], "/Root 1 0 R");
    let text = String::from_utf8_lossy(&base).into_owned();
    let at = text.find("3 0 obj").unwrap();
    let bytes = text.replacen(&format!("{at:010} 00000 n"), &format!("{:010} 00000 n", at + 2), 1).into_bytes();
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert_eq!(doc.get(ObjRef { num: 3, generation: 0 }).as_string().map(|s| s.bytes.clone()), Some(b"three".to_vec()));
    assert!(!doc.xref_entry_is_exact(3));
    assert!(doc.xref_entry_is_exact(1) && doc.xref_entry_is_exact(2));
}

/// A classic table with `/Size` rows and object 0's free entry first, its subsection numbered
/// `1 n` instead of `0 n` (#2676): every row is one number off.
fn misnumbered_table(start: u32) -> Vec<u8> {
    let bodies = ["<< /Marker (first object) >>", "<< /Type /Pages /Kids [] /Count 0 >>", "<< /Type /Catalog /Pages 2 0 R >>"];
    let mut out = b"%PDF-1.7
"
    .to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(
            format!(
                "{} 0 obj
{body}
endobj
",
                i + 1
            )
            .as_bytes(),
        );
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n{start} 4\n0000000000 65535 f \n").as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size 4 /Root 3 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn a_table_numbered_from_1_that_begins_with_object_0_is_read_from_0() {
    let doc = Document::open(Arc::new(misnumbered_table(1))).unwrap();
    let first = doc.get(ObjRef::new(1, 0)).as_dict().cloned().expect("object 1 is not free");
    assert!(first.contains(b"Marker"));
    assert_eq!(doc.root(), Some(ObjRef::new(3, 0)));
    assert!(doc.repair_log().iter().any(|l| l.contains("numbered from 1")), "{:?}", doc.repair_log());
    // A correct table reads the same and logs nothing.
    let fine = Document::open(Arc::new(misnumbered_table(0))).unwrap();
    assert!(fine.get(ObjRef::new(1, 0)).as_dict().is_some_and(|d| d.contains(b"Marker")));
    assert!(!fine.repair_log().iter().any(|l| l.contains("numbered from 1")));
}

#[test]
fn a_table_that_really_starts_at_1_keeps_its_numbers() {
    // Object 1 deleted for good (generation 65535) in a subsection that does start at 1: its
    // second row locates object 2, so nothing is shifted.
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let pages = bytes.len();
    bytes.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let catalog = bytes.len();
    bytes.extend_from_slice(b"3 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 1\n0000000000 65535 f \n1 3\n0000000000 65535 f \n{pages:010} 00000 n \n{catalog:010} 00000 n \ntrailer\n<< /Size 4 /Root 3 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert!(doc.get(ObjRef::new(2, 0)).as_dict().is_some_and(|d| d.name(b"Type") == Some(b"Pages")));
    assert!(doc.get(ObjRef::new(1, 0)).as_dict().is_none(), "object 1 stays free");
    assert!(!doc.repair_log().iter().any(|l| l.contains("numbered from 1")));
}

/// A one-page scan: the page draws image 5 with content stream 4.
fn scanned_page() -> Vec<u8> {
    build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>",
            "<< /Length 21 >>\nstream\n10 0 0 10 0 0 cm /Im0 Do\nendstream",
            "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\nA\nendstream",
        ],
        "/Root 1 0 R",
    )
}

/// `bytes` with its cross-reference table replaced by `table` (the lines after `xref`).
fn with_table(bytes: &[u8], table: &[String], size: usize) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let at = text.rfind("\nxref\n").unwrap() + 1;
    let mut out = bytes[..at].to_vec();
    out.extend_from_slice(format!("xref\n{}\ntrailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{at}\n%%EOF\n", table.join("\n")).as_bytes());
    out
}

/// The table's entry lines, object 0 first.
fn table_entries(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let at = text.rfind("\nxref\n").unwrap() + 1;
    text[at..].lines().skip(2).take_while(|l| !l.starts_with("trailer")).map(str::to_string).collect()
}

/// The classic off-by-one: the subsection says it starts at 0 but lists objects 1 to 5, so
/// the last object has no entry. Viewers rebuild the table and show the page; a reader that
/// took the missing entry for a null object dropped the image when pages were copied (#1628).
#[test]
fn an_off_by_one_xref_subsection_still_finds_the_last_object() {
    let base = scanned_page();
    let entries = table_entries(&base);
    let mut table = vec!["0 5".to_string()];
    table.extend(entries[1..6].iter().cloned());
    let doc = Document::open(Arc::new(with_table(&base, &table, 5))).unwrap();
    let image = doc.get(ObjRef::new(5, 0));
    assert_eq!(image.as_dict().and_then(|d| d.name(b"Subtype")), Some(&b"Image"[..]), "{image:?}");
    assert!(matches!(&*image, Object::Stream(_)));
    assert!(matches!(&*doc.get(ObjRef::new(4, 0)), Object::Stream(s) if &s.raw[..] == b"10 0 0 10 0 0 cm /Im0 Do"));
    assert!(doc.repair_log().iter().any(|l| l.contains("object headers")), "{:?}", doc.repair_log());
}

/// An "in use" entry at offset 0 says nothing about where the object is: the object is
/// found by its header, and the repair is recorded.
#[test]
fn an_object_listed_at_offset_zero_is_found_by_its_header() {
    let base = scanned_page();
    for num in [4usize, 5] {
        let mut table = vec!["0 6".to_string()];
        table.extend(table_entries(&base));
        table[num + 1] = "0000000000 00000 n ".into();
        let doc = Document::open(Arc::new(with_table(&base, &table, 6))).unwrap();
        assert!(matches!(&*doc.get(ObjRef::new(num as u32, 0)), Object::Stream(_)), "object {num}");
        assert!(doc.repair_log().iter().any(|l| l.contains("object headers")), "object {num}: {:?}", doc.repair_log());
    }
}

/// A valid table that simply does not list every number below `/Size` is not repaired: an
/// unlisted number is a null object (§7.3.10), even when an old body with that number is
/// still in the file.
#[test]
fn a_consistent_table_with_unlisted_numbers_is_taken_as_it_is() {
    let base = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "(three)"], "/Root 1 0 R");
    let at = String::from_utf8_lossy(&base).rfind("\nxref\n").unwrap() + 1;
    let mut bytes = base[..at].to_vec();
    bytes.extend_from_slice(b"4 0 obj\n(stale)\nendobj\n");
    let mut table = vec!["0 4".to_string()];
    table.extend(table_entries(&base));
    let doc = Document::open(Arc::new(with_table(&[&bytes[..], &base[at..]].concat(), &table, 6))).unwrap();
    assert!(doc.repair_log().is_empty(), "{:?}", doc.repair_log());
    assert_eq!(*doc.get(ObjRef::new(4, 0)), Object::Null);
    assert!(matches!(&*doc.get(ObjRef::new(3, 0)), Object::String(_)));
}

/// Filling the gaps of a damaged table never brings back an object that an incremental
/// update freed: its body is still in the file, but the newer section says it is gone.
#[test]
fn an_object_freed_by_an_update_stays_freed_when_the_table_is_repaired() {
    let base = scanned_page();
    // The original table lists the content stream at offset 0, so the gaps get filled.
    let mut table = vec!["0 6".to_string()];
    table.extend(table_entries(&base));
    table[5] = "0000000000 00000 n ".into();
    let mut bytes = with_table(&base, &table, 6);
    let prev = String::from_utf8_lossy(&bytes).rfind("\nxref\n").unwrap() + 1;
    let section = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n5 1\n0000000000 00001 f \ntrailer\n<< /Size 6 /Root 1 0 R /Prev {prev} >>\nstartxref\n{section}\n%%EOF\n").as_bytes(),
    );
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert!(matches!(&*doc.get(ObjRef::new(4, 0)), Object::Stream(_)), "the gap is filled");
    assert_eq!(*doc.get(ObjRef::new(5, 0)), Object::Null, "the freed image stays freed");
    assert_eq!(doc.xref_entry(5), Some(XrefEntry::Free { next_generation: 1 }));
}

/// Repairing a damaged table fills only the numbers the damage leaves out: an old body of a
/// number no section lists, under an oversized `/Size`, stays a null object.
#[test]
fn a_repaired_table_leaves_an_unlisted_number_null() {
    let base = scanned_page();
    let at = String::from_utf8_lossy(&base).rfind("\nxref\n").unwrap() + 1;
    let entries = table_entries(&base);
    let mut offset_zero = vec!["0 6".to_string()];
    offset_zero.extend(entries.iter().cloned());
    offset_zero[5] = "0000000000 00000 n ".into();
    let mut off_by_one = vec!["0 5".to_string()];
    off_by_one.extend(entries[1..6].iter().cloned());
    for (name, table, found) in [("offset 0", offset_zero, 4), ("off by one", off_by_one, 5)] {
        let bytes = [&base[..at], b"7 0 obj\n(stale)\nendobj\n", &base[at..]].concat();
        let doc = Document::open(Arc::new(with_table(&bytes, &table, 9))).unwrap();
        assert!(matches!(&*doc.get(ObjRef::new(found, 0)), Object::Stream(_)), "{name}: the gap is filled");
        assert_eq!(*doc.get(ObjRef::new(7, 0)), Object::Null, "{name}: {:?}", doc.repair_log());
        assert_eq!(doc.xref_entry(7), None, "{name}");
    }
}

/// The last object of a subsection numbered one too low is filled from that subsection,
/// whatever later happens to the row before it: an incremental save that rewrites object 4,
/// or object 4's own row at offset 0.
#[test]
fn the_last_object_of_a_shifted_subsection_is_filled_whatever_its_neighbour_says() {
    let base = scanned_page();
    let entries = table_entries(&base);
    let mut off_by_one = vec!["0 5".to_string()];
    off_by_one.extend(entries[1..6].iter().cloned());
    let shifted = with_table(&base, &off_by_one, 5);
    let prev = String::from_utf8_lossy(&shifted).rfind("\nxref\n").unwrap() + 1;
    let mut updated = shifted.clone();
    let body = updated.len();
    updated.extend_from_slice(b"4 0 obj\n<< /Length 21 >>\nstream\n10 0 0 10 0 0 cm /Im0 Do\nendstream\nendobj\n");
    let section = updated.len();
    updated.extend_from_slice(
        format!("xref\n4 1\n{body:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Prev {prev} >>\nstartxref\n{section}\n%%EOF\n").as_bytes(),
    );
    let mut neighbour_at_zero = off_by_one.clone();
    neighbour_at_zero[5] = "0000000000 00000 n ".into();
    for (name, bytes) in [("updated", updated), ("row 4 at offset 0", with_table(&base, &neighbour_at_zero, 5))] {
        let doc = Document::open(Arc::new(bytes)).unwrap();
        let image = doc.get(ObjRef::new(5, 0));
        assert_eq!(image.as_dict().and_then(|d| d.name(b"Subtype")), Some(&b"Image"[..]), "{name}: {:?}", doc.repair_log());
    }
}

/// The image the first page draws as `/Im1`, followed through the page tree.
fn first_page_image(doc: &Document) -> Arc<Object> {
    let catalog = doc.get(doc.root().unwrap());
    let pages = doc.resolve(catalog.as_dict().unwrap().get(b"Pages").unwrap());
    let page = doc.resolve(&pages.as_dict().unwrap().get(b"Kids").unwrap().as_array().unwrap()[0]);
    let resources = doc.resolve(page.as_dict().unwrap().get(b"Resources").unwrap());
    let xobjects = doc.resolve(resources.as_dict().unwrap().get(b"XObject").unwrap());
    doc.resolve(xobjects.as_dict().unwrap().get(b"Im1").unwrap())
}

/// The opposite off-by-one (#1628): the subsection says it starts at 1, but its first entry
/// is object 0's free-list head, so every entry lands one number too high and object 1, the
/// image, reads as free. The offset the table gives object 2 holds `1 0 obj`, so the
/// subsection is read from 0, and the image survives a full save.
#[test]
fn a_subsection_from_one_that_opens_with_the_free_list_head_is_read_from_zero() {
    let base = build(
        &[
            "<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 12 >>\nstream\nABCDEFGHIJKL\nendstream",
            "<< /Length 29 >>\nstream\nq 72 0 0 72 0 0 cm /Im1 Do Q\nendstream",
            "<< /Type /Page /Parent 4 0 R /MediaBox [0 0 72 72] /Resources << /XObject << /Im1 1 0 R >> >> /Contents 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Catalog /Pages 4 0 R >>",
            "<< /Subject (synthetic) >>",
        ],
        "/Root 5 0 R /Info 6 0 R",
    );
    // /Size 7 is one short of the shifted numbers; /Size 8 covers them all. Junk before the
    // header moves every object past the offsets the table gives; it is not a comment, which
    // the lexer would skip on its way to `1 0 obj` anyway.
    for (size, junk) in [(7, ""), (8, ""), (7, "Content-Type: application/pdf\n\n")] {
        let text = String::from_utf8_lossy(&base).replace("\nxref\n0 7\n", "\nxref\n1 7\n").replace("/Size 7 ", &format!("/Size {size} "));
        let doc = Document::open(Arc::new(format!("{junk}{text}").into_bytes())).unwrap();
        let image = first_page_image(&doc);
        assert_eq!(image.as_dict().and_then(|d| d.name(b"Subtype")), Some(&b"Image"[..]), "/Size {size} {junk:?}: {image:?}");
        assert_eq!(doc.xref_entry(0), Some(XrefEntry::Free { next_generation: 65535 }), "/Size {size} {junk:?}");
        assert!(doc.repair_log().iter().any(|l| l.contains("from 0")), "/Size {size} {junk:?}: {:?}", doc.repair_log());
        let saved = Document::open(Arc::new(crate::write_full(&doc, &crate::SaveOptions::default()).unwrap())).unwrap();
        let image = first_page_image(&saved);
        assert!(matches!(&*image, Object::Stream(s) if &s.raw[..] == b"ABCDEFGHIJKL"), "/Size {size} {junk:?}: {image:?}");
    }
}

/// Object 1 really freed: the subsection from 1 opens with a free entry, but the next entry's
/// offset holds object 2, not object 1. The table is taken as it is, and the old body of
/// object 1 still in the file stays freed.
#[test]
fn a_subsection_from_one_whose_next_offset_holds_object_two_is_taken_as_it_is() {
    let base = build(&["(freed)", "<< /Type /Catalog /Pages 3 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 2 0 R");
    let entries = table_entries(&base);
    let mut table = vec!["1 3".to_string(), "0000000000 65535 f ".to_string()];
    table.extend(entries[2..].iter().cloned());
    let text = String::from_utf8_lossy(&base).into_owned();
    let at = text.rfind("\nxref\n").unwrap() + 1;
    for junk in ["", "Content-Type: application/pdf\n\n"] {
        let mut bytes = junk.as_bytes().to_vec();
        bytes.extend_from_slice(&base[..at]);
        bytes.extend_from_slice(format!("xref\n{}\ntrailer\n<< /Size 4 /Root 2 0 R >>\nstartxref\n{at}\n%%EOF\n", table.join("\n")).as_bytes());
        let doc = Document::open(Arc::new(bytes)).unwrap();
        assert_eq!(doc.xref_entry(1), Some(XrefEntry::Free { next_generation: 65535 }), "{junk:?}");
        assert_eq!(*doc.get(ObjRef::new(1, 0)), Object::Null, "{junk:?}: the freed object stays freed");
        assert!(matches!(&*doc.get(ObjRef::new(2, 0)), Object::Dict(d) if d.name(b"Type") == Some(&b"Catalog"[..])), "{junk:?}");
        assert!(!doc.repair_log().iter().any(|l| l.contains("from 0")), "{junk:?}: {:?}", doc.repair_log());
        assert!(!junk.is_empty() || doc.repair_log().is_empty(), "{:?}", doc.repair_log());
    }
}

#[test]
fn opening_an_exhausted_classic_xref_repairs_and_allocates() {
    // The extra in-use row need not be loaded to exhaust the next object number.
    let bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\nxref\n0 3\n0000000000 65535 f \n0000000009 00000 n \n0000000058 00000 n \n4294967295 1\n0000000058 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n110\n%%EOF\n";
    assert_eq!(bytes.len(), 266);
    let mut doc = Document::open(Arc::new(bytes.to_vec())).unwrap();
    assert_eq!(doc.root(), Some(ObjRef::new(1, 0)));
    assert!(doc.repair_log().iter().any(|line| line.contains("exhausted xref")));
    assert_eq!(doc.add(Object::Null), ObjRef::new(3, 0));
}

#[test]
fn opening_an_exhausted_xref_stream_repairs_and_allocates() {
    let mut bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n".to_vec();
    let offset = bytes.len();
    bytes.extend_from_slice(b"3 0 obj\n<< /Type /XRef /Size 4 /Root 1 0 R /W [1 4 2] /Index [0 3 4294967295 1] /Length 28 >>\nstream\n");
    for (kind, position, generation) in [(0_u8, 0_u32, 65535_u16), (1, 9, 0), (1, 58, 0), (1, 58, 0)] {
        bytes.push(kind);
        bytes.extend_from_slice(&position.to_be_bytes());
        bytes.extend_from_slice(&generation.to_be_bytes());
    }
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{offset}\n%%EOF\n").as_bytes());
    let mut doc = Document::open(Arc::new(bytes)).unwrap();
    assert_eq!(doc.root(), Some(ObjRef::new(1, 0)));
    assert!(doc.repair_log().iter().any(|line| line.contains("exhausted xref")));
    assert_eq!(doc.add(Object::Null), ObjRef::new(4, 0));
}

#[test]
fn exhausted_size_and_high_edits_allocate_without_overwriting_objects() {
    let ordinary = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 1 0 R");
    for size in ["4294967295", "4294967296", "9223372036854775807"] {
        let bytes = String::from_utf8(ordinary.clone()).unwrap().replace("/Size 3", &format!("/Size {size}"));
        let mut doc = Document::open(Arc::new(bytes.into_bytes())).unwrap();
        assert!(doc.repair_log().iter().any(|line| line.contains("exhausted trailer")));
        assert_eq!(doc.add(Object::Int(42)), ObjRef::new(3, 0));
        doc.set(ObjRef::new(u32::MAX, 0), Object::Int(99));
        doc.set(ObjRef::new(u32::MAX - 1, 0), Object::Int(98));
        assert_eq!(doc.add(Object::Int(43)), ObjRef::new(4, 0));
        assert_eq!(doc.get(ObjRef::new(3, 0)).as_int(), Some(42));
        assert_eq!(doc.get(ObjRef::new(u32::MAX, 0)).as_int(), Some(99));
        assert_eq!(doc.get(ObjRef::new(1, 0)).as_dict().unwrap().name(b"Type"), Some(b"Catalog".as_slice()));
    }
    let mut doc = Document::new_empty();
    doc.set(ObjRef::new(u32::MAX - 2, 0), Object::Null);
    assert_eq!(doc.add(Object::Null), ObjRef::new(u32::MAX - 1, 0));
    assert_eq!(doc.add(Object::Null), ObjRef::new(3, 0));
    assert_eq!(doc.add(Object::Null), ObjRef::new(4, 0));
}

#[test]
fn exhausted_xref_controls_preserve_ordinary_ignored_and_repaired_entries() {
    let ordinary = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 1 0 R");
    for (extra, repaired) in
        [("", false), ("4294967295 1\n0000000000 00000 n \n", false), ("4294967295 2\n0000000058 00000 n \n0000000058 00000 n \n", true)]
    {
        let bytes = String::from_utf8(ordinary.clone()).unwrap().replacen("trailer\n", &format!("{extra}trailer\n"), 1);
        let mut doc = Document::open(Arc::new(bytes.into_bytes())).unwrap();
        assert_eq!(doc.root(), Some(ObjRef::new(1, 0)));
        assert_eq!(doc.get(ObjRef::new(1, 0)).as_dict().unwrap().name(b"Type"), Some(b"Catalog".as_slice()));
        assert_eq!(!doc.repair_log().is_empty(), repaired);
        if repaired {
            assert!(doc.repair_log().iter().any(|line| line.contains("xref") && line.contains("range")));
        }
        assert_eq!(doc.add(Object::Null), ObjRef::new(3, 0));
    }
}

#[test]
fn invalid_xref_object_ranges_are_reconstructed() {
    // Original in-memory PDF: an intact catalog followed by a damaged xref section.
    for stream in [false, true] {
        for (start, count) in [(i64::MAX, 2), (-1, 2), (4_294_967_296, 1), (4_294_967_295, 2)] {
            let mut bytes =
                b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n".to_vec();
            let offset = bytes.len();
            if stream {
                bytes.extend_from_slice(
                    format!("3 0 obj\n<< /Type /XRef /Root 1 0 R /Size 4 /W [1 1 1] /Index [{start} {count}] /Length 6 >>\nstream\n").as_bytes(),
                );
                bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
                bytes.extend_from_slice(b"\nendstream\nendobj\n");
            } else {
                bytes.extend_from_slice(
                    format!("xref\n{start} {count}\n0000000000 65535 f \n0000000000 65535 f \ntrailer\n<< /Root 1 0 R /Size 3 >>\n").as_bytes(),
                );
            }
            bytes.extend_from_slice(format!("startxref\n{offset}\n%%EOF\n").as_bytes());
            let doc = Document::open(Arc::new(bytes)).unwrap();
            assert!(
                doc.repair_log().iter().any(|line| line.contains("xref") && line.contains("range")),
                "stream={stream}, start={start}, count={count}: {:?}",
                doc.repair_log()
            );
            assert_eq!(doc.get(ObjRef::new(1, 0)).as_dict().unwrap().name(b"Type"), Some(b"Catalog".as_slice()));
        }
    }
}

#[test]
fn objects_only_an_unreadable_older_section_locates_are_found_by_scanning() {
    // The newest section lists the catalog only; its /Prev points past the end of the file,
    // so the page tree is reachable only by scanning object headers, as readers do (#1656).
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (num, body) in
        [(1, "<< /Type /Catalog /Pages 2 0 R >>"), (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"), (3, "<< /Type /Page /Parent 2 0 R >>")]
    {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 2\n0000000000 65535 f \n{:010} 00000 n \ntrailer\n<< /Root 1 0 R /Size 4 /Prev 999999 >>\nstartxref\n{xref}\n%%EOF\n",
            offsets[0]
        )
        .as_bytes(),
    );
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert_eq!(doc.get(ObjRef::new(2, 0)).as_dict().and_then(|d| d.name(b"Type")), Some(b"Pages".as_slice()));
    assert_eq!(doc.get(ObjRef::new(3, 0)).as_dict().and_then(|d| d.name(b"Type")), Some(b"Page".as_slice()));
    assert!(doc.repair_log().iter().any(|l| l.contains("999999")), "{:?}", doc.repair_log());
}

#[test]
fn self_referential_length_does_not_overflow() {
    let bytes = build(
        &["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "<< /Length 3 0 R >>\nstream\nabc\nendstream"],
        "/Root 1 0 R",
    );
    let doc = Document::open(Arc::new(bytes)).unwrap();
    // Either recovered via the endstream search or reported — but never a stack overflow.
    let o = doc.get(ObjRef::new(3, 0));
    if let Object::Stream(s) = o.as_ref() {
        assert_eq!(&s.raw[..], b"abc");
    }
}

#[test]
fn mutually_referential_lengths_do_not_overflow() {
    let bytes = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [] /Count 0 >>",
            "<< /Length 4 0 R >>\nstream\nabc\nendstream",
            "<< /Length 3 0 R >>\nstream\nxyz\nendstream",
        ],
        "/Root 1 0 R",
    );
    let doc = Document::open(Arc::new(bytes)).unwrap();
    let _ = doc.get(ObjRef::new(3, 0));
    let _ = doc.get(ObjRef::new(4, 0));
}

#[test]
fn reads_classic_revision_and_edits_are_copy_on_write() {
    let bytes = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 1 0 R");
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert_eq!(doc.revisions().len(), 1);
    assert!(doc.repair_log().is_empty());
    let mut edited = doc.clone();
    edited.update_dict(ObjRef::new(1, 0), |d| d.set(b"Lang".to_vec(), Object::String(crate::PdfString::literal("en")))).unwrap();
    assert!(edited.is_modified() && !doc.is_modified(), "clone is an independent snapshot");
    assert!(doc.get(ObjRef::new(1, 0)).as_dict().unwrap().get(b"Lang").is_none());
}

#[test]
fn object_numbers_merge_source_and_overlay_in_order() {
    let mut doc = Document::new_empty();
    doc.overlay.clear();
    doc.entries = Arc::new(XrefIndex::Materialized(
        Arc::new(BTreeMap::from([
            (0, XrefEntry::Free { next_generation: 65535 }),
            (2, XrefEntry::InFile { offset: 10, generation: 7 }),
            (4, XrefEntry::Free { next_generation: 9 }),
            (6, XrefEntry::InStream { stream: 80, index: 1 }),
            (8, XrefEntry::InFile { offset: 20, generation: 5 }),
            (10, XrefEntry::InFile { offset: 30, generation: 3 }),
            (u32::MAX, XrefEntry::InFile { offset: 40, generation: 1 }),
        ])),
        Arc::default(),
    ));
    doc.set(ObjRef::new(1, 2), Object::Int(1)); // before the first live source slot
    doc.set(ObjRef::new(2, 8), Object::Int(2)); // override, without duplication
    doc.set(ObjRef::new(4, 9), Object::Int(4)); // reuse a source free slot
    doc.free(ObjRef::new(6, 0)); // remove a compressed source object
    doc.free(ObjRef::new(7, 2)); // free a number absent from the source
    doc.free(ObjRef::new(10, 3));
    doc.set(ObjRef::new(11, 4), Object::Int(11));
    assert_eq!(doc.object_numbers(), vec![1, 2, 4, 8, 11, u32::MAX]);
    assert_eq!(doc.generation(2), 8);
    assert_eq!(doc.generation(4), 9);
    assert_eq!(doc.generation(6), 1);
    assert_eq!(doc.generation(8), 5);
    assert_eq!(doc.generation(10), 4);
    doc.overlay.clear();
    assert_eq!(doc.object_numbers(), vec![2, 6, 8, 10, u32::MAX]);
    doc.entries = Arc::new(XrefIndex::empty());
    assert!(doc.object_numbers().is_empty());
    doc.set(ObjRef::new(5, 7), Object::Int(5));
    assert_eq!(doc.object_numbers(), vec![5]);
}

#[test]
fn full_object_scans_preserve_edits_without_retaining_the_scan() {
    let mut bodies = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_owned(), "<< /Type /Pages /Kids [] /Count 0 >>".to_owned()];
    bodies.extend((0..400).map(|i| format!("<< /Value {i} >>")));
    let refs: Vec<_> = bodies.iter().map(String::as_str).collect();
    let mut doc = Document::open(Arc::new(build(&refs, "/Root 1 0 R"))).unwrap();
    doc.set(ObjRef::new(3, 0), Object::Int(900));
    doc.free(ObjRef::new(4, 0));
    let added = doc.add(Object::Int(901));
    let cached = doc.cache.lock().unwrap().len();
    let held: Vec<_> = doc.scan_objects().collect();
    assert_eq!(held.len(), 402);
    assert_eq!(doc.cache.lock().unwrap().len(), cached, "scanning must not populate the editor cache");
    assert!(doc.objstms.lock().unwrap().is_empty());
    assert!(!held.iter().any(|(r, _)| r.num == 4));
    assert_eq!(held.iter().find(|(r, _)| r.num == 3).unwrap().1.as_int(), Some(900));
    assert_eq!(held.iter().find(|(r, _)| *r == added).unwrap().1.as_int(), Some(901));
    assert_eq!(held.iter().find(|(r, _)| r.num == 400).unwrap().1.as_dict().unwrap().int(b"Value"), Some(397));
    // Previously returned objects survive cache eviction and dropping the source.
    drop(doc);
    assert_eq!(held.iter().find(|(r, _)| r.num == 128).unwrap().1.as_dict().unwrap().int(b"Value"), Some(125));
}

#[test]
fn temporary_reader_keeps_file_views_and_evicts_data_of_its_own() {
    // A stream read from the file is a view into the input: caching it costs no copy, so
    // even a large one stays cached (its /Length object too) instead of being re-parsed.
    let body = "x".repeat(ObjectReader::STREAM_BYTES + 1);
    let stream = format!("<< /Length 4 0 R >>\nstream\n{body}\nendstream");
    let length = body.len().to_string();
    let bytes = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", &stream, &length], "/Root 1 0 R");
    let doc = Document::open(Arc::new(bytes)).unwrap();
    let mut reader = doc.object_reader();
    let read = reader.get(ObjRef::new(3, 0));
    let Object::Stream(s) = &*read else { panic!("stream") };
    assert!(s.raw.shares(&reader.document.data), "no copy of the file's bytes");
    assert!(reader.document.cache.lock().unwrap().contains_key(&3), "a view costs nothing to keep");
    // Data of its own (decrypted streams, strings) counts: past the budget the cache is
    // cleared after the next read, including a failed one.
    let owned = Object::Stream(crate::Stream::from_raw(Dict::new(), vec![b'y'; ObjectReader::STREAM_BYTES + 1]));
    let size = held_bytes(&owned, &reader.document.data, 0);
    reader.document.cache.lock().unwrap().insert(3, Arc::new(owned), size);
    assert_eq!(*reader.get(ObjRef::new(99, 0)), Object::Null);
    assert!(reader.document.cache.lock().unwrap().is_empty(), "owned data over budget is evicted");
}

#[test]
fn temporary_reader_and_full_save_do_not_fill_snapshot_caches() {
    let mut source = Document::new_empty();
    let objects: Vec<_> = (0..260)
        .map(|i| {
            let mut value = vec![b'x'; 48 * 1024];
            value[..4].copy_from_slice(&(i as u32).to_le_bytes());
            Object::Ref(source.add(Object::String(crate::PdfString::literal(value))))
        })
        .collect();
    source.update_dict(source.root().unwrap(), |d| d.set(b"Extension".to_vec(), Object::Array(objects))).unwrap();
    let bytes = Arc::new(crate::write_full(&source, &crate::SaveOptions::default()).unwrap());
    let doc = Document::open(bytes).unwrap();
    let root = doc.get(doc.root().unwrap());
    let references = root.as_dict().unwrap().get(b"Extension").unwrap().as_array().unwrap();
    let snapshot = doc.clone();
    let cached = doc.cache.lock().unwrap().len();
    let streams = doc.objstms.lock().unwrap().len();
    let mut reader = doc.object_reader();
    let mut held = None;
    for (i, reference) in references.iter().enumerate() {
        let Object::Ref(reference) = reference else { panic!("indirect object") };
        let object = reader.get(*reference);
        assert_eq!(&object.as_string().unwrap().bytes[..4], &(i as u32).to_le_bytes());
        if i == 0 {
            held = Some(object);
        }
        assert!(reader.document.cache.lock().unwrap().len() < ObjectReader::OBJECTS);
        let decoded = reader.document.objstms.lock().unwrap();
        assert!(decoded.len() < ObjectReader::OBJECTS);
        let bytes: usize = decoded.values().map(|s| s.data.capacity() + s.index.capacity() * std::mem::size_of::<Option<(u32, usize)>>()).sum();
        assert!(bytes <= ObjectReader::STREAM_BYTES || decoded.len() == 1, "only one oversized stream may remain");
    }
    drop(reader);
    assert_eq!(&held.unwrap().as_string().unwrap().bytes[..4], &0u32.to_le_bytes(), "owned results survive evictions");
    for object_streams in [false, true] {
        let saved = crate::write_full(&doc, &crate::SaveOptions { object_streams, ..Default::default() }).unwrap();
        assert!(Document::open(Arc::new(saved)).unwrap().repair_log().is_empty());
        assert_eq!(doc.cache.lock().unwrap().len(), cached, "full saves must not fill the editor cache");
        assert_eq!(doc.objstms.lock().unwrap().len(), streams);
        assert!(Arc::ptr_eq(&doc.cache, &snapshot.cache), "do not detach or clear caller snapshots");
    }
}

#[test]
fn streams_without_length_share_one_endstream_search() {
    let n = 2_000;
    let mut file = b"%PDF-1.7\n".to_vec();
    for k in 1..=n {
        file.extend_from_slice(format!("{k} 0 obj\n<< >>\nstream\nxx\n").as_bytes());
    }
    file.extend_from_slice(b"endstream\nendobj\n");
    file.extend_from_slice(
        format!("{0} 0 obj\n<< /Type /Catalog /Pages {1} 0 R >>\nendobj\n{1} 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n", n + 1, n + 2)
            .as_bytes(),
    );
    file.extend_from_slice(format!("trailer\n<< /Root {} 0 R /Size {} >>\n%%EOF\n", n + 1, n + 3).as_bytes());
    let len = file.len();
    crate::parser::ENDSTREAM_SCAN.with(|c| c.set(0));
    let doc = Document::open(Arc::new(file)).unwrap();
    for k in [1, n / 2, n] {
        assert!(matches!(doc.try_get(k).as_deref(), Ok(Object::Stream(_))), "object {k}");
    }
    let scanned = crate::parser::ENDSTREAM_SCAN.with(|c| c.get());
    assert!(scanned <= 4 * len, "{scanned} bytes searched in a {len}-byte file");
}

#[test]
fn editor_cache_counts_deeply_nested_strings() {
    let mut nested = Object::String(crate::PdfString::literal(vec![b'x'; 1 << 20]));
    for _ in 0..8 {
        nested = Object::Array(vec![nested]);
    }
    let mut source = Document::new_empty();
    let objects: Vec<_> = (0..48).map(|_| Object::Ref(source.add(nested.clone()))).collect();
    source.update_dict(source.root().unwrap(), |d| d.set(b"Extension".to_vec(), Object::Array(objects))).unwrap();
    let bytes = Arc::new(crate::write_full(&source, &crate::SaveOptions::default()).unwrap());
    let doc = Document::open(bytes).unwrap();
    let held = held_bytes(&nested, &doc.data, 0);
    assert!((1 << 20..(1 << 20) + 4096).contains(&held), "the string and a little tree: {held}");
    let root = doc.get(doc.root().unwrap());
    for reference in root.as_dict().unwrap().get(b"Extension").unwrap().as_array().unwrap() {
        let Object::Ref(reference) = reference else { panic!("indirect object") };
        drop(doc.get(*reference));
        let cache = doc.cache.lock().unwrap();
        let owned = cache.values().fold(0usize, |n, o| n.saturating_add(held_bytes(o, &doc.data, 0)));
        let strings = cache.values().filter(|o| o.as_array().is_some()).count();
        assert!(strings << 20 <= Document::EDITOR_CACHE_BYTES + (1 << 20), "{strings} MiB strings cached, {owned} bytes counted");
    }
}

#[test]
fn editor_cache_stays_bounded() {
    let mut source = Document::new_empty();
    let objects: Vec<_> = (0..(Document::EDITOR_CACHE_OBJECTS + 64))
        .map(|i| {
            let mut value = vec![b'x'; 1024];
            value[..4].copy_from_slice(&(i as u32).to_le_bytes());
            Object::Ref(source.add(Object::String(crate::PdfString::literal(value))))
        })
        .collect();
    source.update_dict(source.root().unwrap(), |d| d.set(b"Extension".to_vec(), Object::Array(objects))).unwrap();
    let bytes = Arc::new(crate::write_full(&source, &crate::SaveOptions::default()).unwrap());
    let doc = Document::open(bytes).unwrap();
    let root = doc.get(doc.root().unwrap());
    let references = root.as_dict().unwrap().get(b"Extension").unwrap().as_array().unwrap().clone();
    let mut held = None;
    for (i, reference) in references.iter().enumerate() {
        let Object::Ref(reference) = reference else { panic!("indirect object") };
        let object = doc.get(*reference);
        assert_eq!(&object.as_string().unwrap().bytes[..4], &(i as u32).to_le_bytes());
        if i == 0 {
            held = Some(object);
        }
        assert!(doc.cache.lock().unwrap().len() <= Document::EDITOR_CACHE_OBJECTS, "editing must not pin the whole object graph");
        // Eviction keeps the newcomer.
        assert!(doc.cache.lock().unwrap().contains_key(&reference.num), "eviction must keep the newcomer");
    }
    // Returned objects survive eviction, through both handles of the shared cache.
    assert_eq!(&held.unwrap().as_string().unwrap().bytes[..4], &0u32.to_le_bytes());
    let snapshot = doc.clone();
    let first = references[0].as_ref().unwrap();
    assert_eq!(&snapshot.get(first).as_string().unwrap().bytes[..4], &0u32.to_le_bytes());
}

#[test]
fn editor_cache_byte_bound_fires() {
    // 40 × 1 MB strings: far below the count cap, above the 32 MB byte budget, which is
    // checked after every insertion.
    let mut source = Document::new_empty();
    let objects: Vec<_> = (0..40)
        .map(|i| {
            let mut value = vec![b'y'; 1024 * 1024];
            value[..4].copy_from_slice(&(i as u32).to_le_bytes());
            Object::Ref(source.add(Object::String(crate::PdfString::literal(value))))
        })
        .collect();
    source.update_dict(source.root().unwrap(), |d| d.set(b"Extension".to_vec(), Object::Array(objects))).unwrap();
    let bytes = Arc::new(crate::write_full(&source, &crate::SaveOptions::default()).unwrap());
    let doc = Document::open(bytes).unwrap();
    let root = doc.get(doc.root().unwrap());
    let references = root.as_dict().unwrap().get(b"Extension").unwrap().as_array().unwrap().clone();
    let mut held = None;
    for (i, reference) in references.iter().enumerate() {
        let Object::Ref(reference) = reference else { panic!("indirect object") };
        let object = doc.get(*reference);
        assert_eq!(&object.as_string().unwrap().bytes[..4], &(i as u32).to_le_bytes());
        if i == 0 {
            held = Some(object);
        }
        // Eviction keeps the newcomer.
        assert!(doc.cache.lock().unwrap().contains_key(&reference.num), "eviction must keep the newcomer");
    }
    assert!(doc.cache.lock().unwrap().len() < references.len(), "the byte budget must evict even below the count cap");
    let last = references[references.len() - 1].as_ref().unwrap();
    assert!(doc.cache.lock().unwrap().contains_key(&last.num), "eviction must keep the newcomer");
    assert_eq!(&held.unwrap().as_string().unwrap().bytes[..4], &0u32.to_le_bytes());
}

#[test]
fn a_stream_limited_document_cannot_be_saved() {
    let bytes = Arc::new(crate::write_full(&Document::new_empty(), &crate::SaveOptions::default()).unwrap());
    let doc = Document::open_with_stream_limit(bytes.clone(), None, 1 << 20).unwrap();
    for saved in [crate::write_full(&doc, &crate::SaveOptions::default()), crate::write_incremental(&doc, &crate::SaveOptions::default())] {
        assert!(matches!(saved, Err(CosError::ReadOnlyLimit)), "{saved:?}");
    }
    let doc = Document::open(bytes).unwrap();
    assert!(crate::write_full(&doc, &crate::SaveOptions::default()).is_ok());
}

#[test]
fn inspection_stream_limit_applies_before_loading_compressed_objects() {
    let mut doc = Document::new_empty();
    let large = doc.add(Object::String(crate::PdfString::literal(vec![b'x'; 4096])));
    doc.update_dict(doc.root().unwrap(), |catalog| catalog.set(b"Extension".to_vec(), Object::Ref(large))).unwrap();
    let bytes = Arc::new(crate::write_full(&doc, &crate::SaveOptions::default()).unwrap());
    assert!(Document::open(bytes.clone()).is_ok());
    assert!(Document::open_with_stream_limit(bytes, None, 512).is_err());
}

/// A /Prev chain that loops back on itself stops at the loop (and says so).
#[test]
fn looping_revision_chains_stop() {
    let mut bytes = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 1 0 R");
    // An update whose /Prev points at itself.
    let at = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 1\n0000000000 65535 f \ntrailer << /Size 3 /Root 1 0 R /Prev {at} >>\nstartxref\n{at}\n%%EOF\n").as_bytes(),
    );
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert_eq!(doc.root(), Some(ObjRef::new(1, 0)));
    assert!(doc.repair_log().iter().any(|l| l.contains("loops")), "{:?}", doc.repair_log());
}

/// A hybrid-reference file: the classic table leaves object 3 out; the `/XRefStm` stream
/// says it lives in object stream 4.
#[test]
fn hybrid_reference_files_read_the_xref_stream_too() {
    let doc = Document::open(Arc::new(object_stream_file("3 0 ", "<< /Hidden (yes) >>", 0))).unwrap();
    assert!(doc.repair_log().is_empty(), "{:?}", doc.repair_log());
    assert_compact_matches_materialized(&doc);
    assert_eq!(text_of(&doc, 3, b"Hidden").as_deref(), Some("yes"));
}

/// A hybrid-reference file whose classic table leaves object 3 out, and whose `/XRefStm`
/// stream puts it at position `member` of object stream 4, which holds `header` (the number
/// and offset pairs) and then `members`.
fn object_stream_file(header: &str, members: &str, member: u8) -> Vec<u8> {
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offs = [0usize; 6];
    let mut obj = |out: &mut Vec<u8>, n: usize, body: &[u8]| {
        offs[n] = out.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, 2, b"<< /Type /Pages /Kids [] /Count 0 /Extra 3 0 R >>");
    let count = header.split_whitespace().count() / 2;
    let inner = format!("{header}{members}");
    let mut stm = format!("<< /Type /ObjStm /N {count} /First {} /Length {} >>\nstream\n", header.len(), inner.len()).into_bytes();
    stm.extend_from_slice(inner.as_bytes());
    stm.extend_from_slice(b"\nendstream");
    obj(&mut out, 4, &stm);
    let data = [2u8, 0, 4, member];
    let mut xs = format!("<< /Type /XRef /Size 6 /W [1 2 1] /Index [3 1] /Length {} >>\nstream\n", data.len()).into_bytes();
    xs.extend_from_slice(&data);
    xs.extend_from_slice(b"\nendstream");
    obj(&mut out, 5, &xs);
    let table = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for (n, off) in offs.iter().enumerate().skip(1) {
        let line = if n == 3 { "0000000000 65535 f \n".to_string() } else { format!("{off:010} 00000 n \n") };
        out.extend_from_slice(line.as_bytes());
    }
    out.extend_from_slice(format!("trailer << /Size 6 /Root 1 0 R /XRefStm {} >>\nstartxref\n{table}\n%%EOF\n", offs[5]).as_bytes());
    out
}

/// Many revisions naming one hybrid /XRefStm read it once.
#[test]
fn a_hybrid_stream_named_by_many_revisions_is_read_once() {
    let mut out = object_stream_file("3 0 ", "<< /Hidden (yes) >>", 0);
    let stream = find(&out, b"5 0 obj", 0, out.len()).unwrap();
    let mut prev = rfind(&out, b"\nxref\n", 0).unwrap() + 1;
    let revisions = 200;
    for _ in 0..revisions {
        let at = out.len();
        out.extend_from_slice(format!("xref\ntrailer << /Size 6 /Root 1 0 R /XRefStm {stream} /Prev {prev} >>\nstartxref\n{at}\n%%EOF\n").as_bytes());
        prev = at;
    }
    let doc = Document::open(Arc::new(out)).unwrap();
    SECTION_READS.with(|c| c.set(0));
    let (entries, ..) = doc.read_xref_chain_materialized(&mut Vec::new()).unwrap();
    let reads = SECTION_READS.with(|c| c.get());
    assert!(reads <= revisions + 2, "{reads} sections read for {} revisions", revisions + 1);
    assert!(matches!(entries.get(&3), Some(XrefEntry::InStream { stream: 4, .. })), "{:?}", entries.get(&3));
    assert_eq!(text_of(&doc, 3, b"Hidden").as_deref(), Some("yes"));
    assert_compact_matches_materialized(&doc);
}

/// An older section that names an /XRefStm a newer section already read still has its own
/// table compared with that stream, so an entry it sets aside is recorded for it.
#[test]
fn a_repeated_hybrid_stream_records_each_sections_conflicts() {
    let mut out = object_stream_file("3 0 ", "<< /Hidden (yes) >>", 0);
    let stream = find(&out, b"5 0 obj", 0, out.len()).unwrap();
    let catalog = find(&out, b"1 0 obj", 0, out.len()).unwrap();
    let base = rfind(&out, b"\nxref\n", 0).unwrap() + 1;
    // This section's table places object 3 at the catalog; the stream puts it in object stream 4.
    let older = out.len();
    out.extend_from_slice(
        format!("xref\n3 1\n{catalog:010} 00000 n \ntrailer << /Size 6 /Root 1 0 R /XRefStm {stream} /Prev {base} >>\nstartxref\n{older}\n%%EOF\n")
            .as_bytes(),
    );
    let newer = out.len();
    out.extend_from_slice(format!("xref\ntrailer << /Size 6 /Root 1 0 R /XRefStm {stream} /Prev {older} >>\nstartxref\n{newer}\n%%EOF\n").as_bytes());
    let doc = Document::open(Arc::new(out)).unwrap();
    assert_eq!(doc.xref_oddities(), [(older as u64, 3)]);
    assert_eq!(text_of(&doc, 3, b"Hidden").as_deref(), Some("yes"));
}

fn text_of(doc: &Document, num: u32, key: &[u8]) -> Option<String> {
    doc.get(ObjRef::new(num, 0)).as_dict().and_then(|d| d.get(key).and_then(|v| v.as_string().map(|s| s.to_text())))
}

/// An object stream member numbered 2^32 + 3 is not object 3 (#1815): the number used to be
/// cast to `u32`, which dropped the high bits.
#[test]
fn object_stream_numbers_beyond_u32_are_not_truncated() {
    let control = Document::open(Arc::new(object_stream_file("3 0 ", "<< /Member (yes) >>", 0))).unwrap();
    assert_eq!(text_of(&control, 3, b"Member").as_deref(), Some("yes"));
    let doc = Document::open(Arc::new(object_stream_file("4294967299 0 ", "<< /Member (yes) >>", 0))).unwrap();
    assert!(matches!(*doc.get(ObjRef::new(3, 0)), Object::Null), "{:?}", doc.get(ObjRef::new(3, 0)));
    assert!(!doc.xref_entry_is_exact(3));
}

/// Member starts are checked rather than cast, so they cannot wrap on any target (#1815).
#[test]
fn object_stream_member_starts_are_checked() {
    assert_eq!(objstm_member(3, 4, Some(10), 20), Some((3, 14)));
    assert_eq!(objstm_member(3, 9, Some(10), 20), Some((3, 19)), "the last byte");
    assert_eq!(objstm_member(3, 10, Some(10), 20), None, "at the end of the data");
    assert_eq!(objstm_member(3, 1, Some(usize::MAX), 20), None, "/First plus the offset overflows");
    assert_eq!(objstm_member(3, i64::MAX, Some(10), 20), None);
    assert_eq!(objstm_member(3, -1, Some(10), 20), None);
    assert_eq!(objstm_member(3, 0, None, 20), None, "/First does not fit in usize");
    assert_eq!(objstm_member(i64::from(u32::MAX), 0, Some(0), 1), Some((u32::MAX, 0)));
    assert_eq!(objstm_member(i64::from(u32::MAX) + 1, 0, Some(0), 1), None);
    assert_eq!(objstm_member(-1, 0, Some(0), 1), None);
}

/// A member at a negative offset is unusable, not the stream's first member (#1815): the
/// offset used to be clamped to zero, which read the object at `/First`.
#[test]
fn object_stream_negative_offsets_are_refused() {
    let doc = Document::open(Arc::new(object_stream_file("7 0 3 -1 ", "<< /First (yes) >>", 1))).unwrap();
    assert!(matches!(*doc.get(ObjRef::new(3, 0)), Object::Null), "{:?}", doc.get(ObjRef::new(3, 0)));
    let control = Document::open(Arc::new(object_stream_file("7 0 3 19 ", "<< /First (yes) >> << /Second (yes) >>", 1))).unwrap();
    assert_eq!(text_of(&control, 3, b"Second").as_deref(), Some("yes"));
}

#[test]
fn revision_ends_split_incremental_updates() {
    let mut bytes = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 1 0 R");
    let first = bytes.len();
    let doc = Document::open(Arc::new(bytes.clone())).unwrap();
    assert_eq!(doc.revision_ends(), vec![first]);
    let mut edited = doc.clone();
    edited.update_dict(ObjRef::new(1, 0), |d| d.set(b"Lang".to_vec(), Object::String(crate::PdfString::literal("en")))).unwrap();
    bytes = crate::write_incremental(&edited, &crate::SaveOptions::default()).unwrap();
    let doc = Document::open(Arc::new(bytes.clone())).unwrap();
    let ends = doc.revision_ends();
    assert_eq!(ends.len(), 2, "{ends:?}");
    assert!(ends[0] >= first && ends[0] < bytes.len() && ends[1] == bytes.len(), "{ends:?} of {}", bytes.len());
    // The first revision opens on its own, without the update.
    let old = Document::open(Arc::new(bytes[..ends[0]].to_vec())).unwrap();
    assert!(old.get(ObjRef::new(1, 0)).as_dict().unwrap().get(b"Lang").is_none());
}

/// qpdf's `xref-compressed-in-compressed.pdf` in miniature: object 3 is an object stream
/// holding object 4, and the cross-reference stream says object 3 itself lives in stream 99.
fn compressed_in_compressed_fixture() -> Document {
    let mut out = b"%PDF-1.5\n".to_vec();
    let at = |out: &mut Vec<u8>, num: u32, body: &str| {
        let off = out.len();
        out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
        off as u16
    };
    let o1 = at(&mut out, 1, "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>");
    let o2 = at(&mut out, 2, "<< /Type /Pages /Kids [] /Count 0 >>");
    let inner = "<< /Type /Outlines /Count 0 >>";
    at(&mut out, 3, &format!("<< /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n4 0 {inner}\nendstream", 4 + inner.len()));
    // /W [1 2 1]: object 3 is said to live in stream 99 (which does not exist), 4 in stream 3.
    let rows: Vec<[u8; 4]> =
        vec![[0, 0, 0, 255], [1, 0, o1 as u8, 0], [1, 0, o2 as u8, 0], [2, 0, 99, 0], [2, 0, 3, 0], [1, (out.len() >> 8) as u8, out.len() as u8, 0]];
    let rows: Vec<u8> = rows.concat();
    let xref = out.len();
    out.extend_from_slice(
        format!("5 0 obj\n<< /Type /XRef /W [1 2 1] /Index [0 6] /Size 6 /Root 1 0 R /Length {} >>\nstream\n", rows.len()).as_bytes(),
    );
    out.extend_from_slice(&rows);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    Document::open(Arc::new(out)).unwrap()
}

/// An object stream whose own cross-reference entry says it is compressed inside another
/// stream (which the format forbids, and which does not exist here: qpdf's
/// `xref-compressed-in-compressed.pdf`, #2367) could not be loaded, so every object in it
/// read as null and a full save wrote `/Outlines null`. The stream is now found by its own
/// header, as a plain object with a wrong offset is.
#[test]
fn an_object_stream_marked_as_compressed_is_found_by_its_header() {
    let doc = compressed_in_compressed_fixture();
    assert_eq!(doc.revisions().len(), 1, "the chain itself reads: {:?}", doc.repair_log());
    let outlines = doc.get(ObjRef::new(4, 0));
    assert_eq!(outlines.as_dict().and_then(|d| d.name(b"Type")), Some(&b"Outlines"[..]), "{outlines:?}");
    let saved = crate::write_full(&doc, &crate::SaveOptions::default()).unwrap();
    let back = Document::open(Arc::new(saved)).unwrap();
    let catalog = back.get(back.root().unwrap());
    let outlines = back.resolve(catalog.as_dict().unwrap().get(b"Outlines").unwrap());
    assert_eq!(outlines.as_dict().and_then(|d| d.name(b"Type")), Some(&b"Outlines"[..]), "the full save keeps it: {outlines:?}");
}

/// The header fallback is for file entries only: once the stream is freed (or replaced) in
/// the overlay, its objects must not come back from the file's copy of it.
#[test]
fn a_freed_object_stream_is_not_found_by_its_header() {
    let doc = compressed_in_compressed_fixture();
    assert!(doc.get(ObjRef::new(4, 0)).as_dict().is_some());
    let mut edited = doc.clone();
    edited.free(ObjRef::new(3, 0));
    let fresh = edited.object_reader().document;
    assert!(matches!(*fresh.get(ObjRef::new(4, 0)), Object::Null), "{:?}", fresh.get(ObjRef::new(4, 0)));
    let mut replaced = doc.clone();
    replaced.set(ObjRef::new(3, 0), Object::Int(0));
    let fresh = replaced.object_reader().document;
    assert!(matches!(*fresh.get(ObjRef::new(4, 0)), Object::Null), "{:?}", fresh.get(ObjRef::new(4, 0)));
}

/// The same once the free is saved: the revision's table says the stream is free, and
/// neither reading the file back nor rewriting it in full brings its objects back.
#[test]
fn a_container_freed_in_a_saved_revision_is_not_found_by_its_header() {
    let mut edited = compressed_in_compressed_fixture();
    edited.free(ObjRef::new(3, 0));
    let incremental = crate::write_incremental(&edited, &crate::SaveOptions::default()).unwrap();
    let reopened = Document::open(Arc::new(incremental)).unwrap();
    assert_eq!(reopened.revisions().len(), 2);
    assert!(matches!(reopened.xref_entry(3), Some(XrefEntry::Free { .. })));
    assert!(matches!(*reopened.get(ObjRef::new(4, 0)), Object::Null), "{:?}", reopened.get(ObjRef::new(4, 0)));
    for object_streams in [true, false] {
        let opts = crate::SaveOptions { object_streams, ..Default::default() };
        let full = crate::write_full(&reopened, &opts).unwrap();
        let back = Document::open(Arc::new(full)).unwrap();
        let catalog = back.get(back.root().unwrap());
        let outlines = catalog.as_dict().unwrap().get(b"Outlines").map(|o| back.resolve(o));
        assert!(outlines.as_ref().is_none_or(|o| matches!(**o, Object::Null)), "the full save keeps it out: {outlines:?}");
    }
}

/// A trailer whose /Prev is written as the reference `N 0 R` (tiff2pdf did this; the files
/// are JHOVE's PDF-HUL-157 cases, #2366) used to end the cross-reference chain silently, so
/// every object of the earlier sections read as null: a four-page file showed one page, and
/// a full save wrote the three missing kids as null. The offset is read from the reference.
#[test]
fn a_prev_written_as_a_reference_still_reaches_the_earlier_sections() {
    let base =
        build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R >>"], "/Root 1 0 R");
    let prev = String::from_utf8_lossy(&base).rfind("xref\n0 4").unwrap();
    let update = |prev_entry: &str| {
        let mut bytes = base.clone();
        let at = bytes.len();
        bytes.extend_from_slice(b"4 0 obj\n<< /Title (Update) >>\nendobj\n");
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n4 1\n{at:010} 00000 n \ntrailer\n<< /Size 5 /Root 1 0 R /Info 4 0 R /Prev {prev_entry} >>\nstartxref\n{xref}\n%%EOF\n")
                .as_bytes(),
        );
        Document::open(Arc::new(bytes)).unwrap()
    };
    let good = update(&prev.to_string());
    let bad = update(&format!("{prev} 0 R"));
    assert!(good.repair_log().is_empty(), "{:?}", good.repair_log());
    assert_eq!(bad.revisions().len(), 2, "{:?}", bad.repair_log());
    assert_eq!(bad.repair_log().len(), 1, "a note about the /Prev: {:?}", bad.repair_log());
    assert_eq!(bad.xref_entry(3), good.xref_entry(3), "the page of the first section is reached");
    assert_eq!(bad.get(ObjRef::new(3, 0)).as_dict().and_then(|d| d.name(b"Type")), Some(&b"Page"[..]));
    assert_eq!(
        bad.get(ObjRef::new(4, 0)).as_dict().and_then(|d| d.get(b"Title").and_then(|t| t.as_string().map(|s| s.to_text()))),
        Some("Update".into())
    );
}

/// iText's `indirectPrev.pdf` in miniature: the update's trailer gives `/Prev 4 0 R`, and
/// object 4, listed in that update, holds the offset of the first section. Read as offset 4
/// (inside the header), the chain broke and the file was rebuilt without its revisions.
#[test]
fn a_prev_reference_to_an_object_holding_the_offset_reads_that_offset() {
    let mut bytes =
        build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R >>"], "/Root 1 0 R");
    let prev = String::from_utf8_lossy(&bytes).rfind("xref\n0 4").unwrap();
    let mut at = Vec::new();
    for (num, body) in
        [(2, "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_string()), (4, prev.to_string()), (5, "<< /Type /Page /Parent 2 0 R >>".into())]
    {
        at.push(bytes.len());
        bytes.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n2 1\n{:010} 00000 n \n4 2\n{:010} 00000 n \n{:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Prev 4 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            at[0], at[1], at[2]
        )
        .as_bytes(),
    );
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert_eq!(doc.revisions().len(), 2, "{:?}", doc.repair_log());
    assert_eq!(doc.repair_log().len(), 1, "a note about the /Prev, and no rebuild: {:?}", doc.repair_log());
    assert!(doc.repair_log()[0].contains(&format!("the offset {prev}")), "{:?}", doc.repair_log());
    assert_eq!(doc.get(ObjRef::new(3, 0)).as_dict().and_then(|d| d.name(b"Type")), Some(&b"Page"[..]));
    assert_compact_matches_materialized(&doc);
}

/// A file whose only trailers are cross-reference stream dictionaries, with its `startxref`
/// out of reach (here a second file appended after it, cut off before its own
/// cross-reference data, as in #2365), is rebuilt from its object headers. The rebuild used
/// to look only for the `trailer` keyword, so /Info and /ID were lost, and a full save after
/// editing one Info field dropped every other.
#[test]
fn reconstruction_takes_the_trailer_from_a_cross_reference_stream() {
    let table = build(
        &["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "<< /Title (Kept) /Creator (Me) >>"],
        "/Root 1 0 R /Info 3 0 R",
    );
    let packed = crate::write_full(&Document::open(Arc::new(table)).unwrap(), &crate::SaveOptions::default()).unwrap();
    let text = String::from_utf8_lossy(&packed);
    assert!(text.contains("/XRef") && !text.contains("trailer"), "a cross-reference stream is the only trailer");
    let mut bytes = packed.clone();
    let appended = format!("%PDF-1.3\n90 0 obj\n<< /Length 5000 >>\nstream\n{}\nendstream\nendobj\n", "x".repeat(5000));
    bytes.extend_from_slice(appended.as_bytes());
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert!(doc.revisions().is_empty(), "rebuilt: {:?}", doc.repair_log());
    let info = doc.trailer().reference(b"Info").expect("the cross-reference stream's /Info");
    let field = |d: &Document, key: &[u8]| d.get(info).as_dict().unwrap().get(key).unwrap().as_string().unwrap().bytes.clone();
    assert_eq!(field(&doc, b"Title"), b"Kept");
    assert!(doc.trailer().get(b"ID").is_some(), "and its /ID");
    let mut edited = doc.clone();
    edited.update_dict(info, |d| d.set(b"Subject".to_vec(), Object::String(crate::PdfString::literal("new")))).unwrap();
    let saved = crate::write_full(&edited, &crate::SaveOptions::default()).unwrap();
    let back = Document::open(Arc::new(saved)).unwrap();
    assert!(back.repair_log().is_empty(), "{:?}", back.repair_log());
    let info = back.trailer().reference(b"Info").expect("the full save keeps /Info");
    let field = |key: &[u8]| back.get(info).as_dict().unwrap().get(key).unwrap().as_string().unwrap().bytes.clone();
    assert_eq!((field(b"Title"), field(b"Creator"), field(b"Subject")), (b"Kept".to_vec(), b"Me".to_vec(), b"new".to_vec()));
}

/// A rebuild prefers a trailer whose /Root is the catalog (as `root_is_catalog` judges it): a
/// later cross-reference stream whose /Root points at the Info dictionary does not replace an
/// earlier good trailer, or a full save would write the Info dictionary as /Root.
#[test]
fn reconstruction_skips_a_trailer_whose_root_is_not_the_catalog() {
    let mut bytes =
        build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>", "<< /Title (Kept) >>"], "/Root 1 0 R /Info 3 0 R");
    bytes.extend_from_slice(b"4 0 obj\n<< /Type /XRef /Size 5 /W [1 1 1] /Root 3 0 R /Length 3 >>\nstream\nabc\nendstream\nendobj\n");
    let appended = format!("%PDF-1.3\n90 0 obj\n<< /Length 5000 >>\nstream\n{}\nendstream\nendobj\n", "x".repeat(5000));
    bytes.extend_from_slice(appended.as_bytes());
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert!(doc.repair_log().iter().any(|l| l.contains("rebuilt")), "{:?}", doc.repair_log());
    assert_eq!(doc.trailer().reference(b"Root").map(|r| r.num), Some(1), "the catalog, not the Info dictionary");
    assert_eq!(doc.trailer().reference(b"Info").map(|r| r.num), Some(3));
}

#[test]
fn damaged_xref_is_reconstructed() {
    let mut bytes = build(&["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>"], "/Root 1 0 R");
    let n = bytes.len();
    bytes[n - 12..n - 6].copy_from_slice(b"999999"); // point startxref into nowhere
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert!(doc.revisions().is_empty());
    assert!(!doc.repair_log().is_empty());
    assert_eq!(doc.root(), Some(ObjRef::new(1, 0)));
}

#[test]
fn a_decompression_bomb_in_an_xref_stream_is_refused_and_the_file_reconstructed() {
    // 32 MiB of rows behind two FlateDecode filters (a few hundred bytes in the file) for
    // /W [1 1 1]: at 3 bytes a row that is more than 8,388,607 objects' worth, so it is not
    // decoded (in full: the limit applies while inflating) and the objects are found by
    // reconstruction instead.
    let bomb = pdfcraft_filters::encode_flate(&pdfcraft_filters::encode_flate(&vec![0u8; 32 << 20]));
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let o1 = bytes.len();
    bytes.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let o2 = bytes.len();
    bytes.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let x = bytes.len();
    bytes.extend_from_slice(
        format!("3 0 obj\n<< /Type /XRef /Size 4 /W [1 1 1] /Root 1 0 R /Filter [/FlateDecode /FlateDecode] /Length {} >>\nstream\n", bomb.len())
            .as_bytes(),
    );
    bytes.extend_from_slice(&bomb);
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{x}\n%%EOF\n").as_bytes());
    assert!(o1 < o2 && o2 < x);
    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert_eq!(doc.root(), Some(ObjRef::new(1, 0)));
    assert!(doc.repair_log().iter().any(|l| l.contains("limit")), "{:?}", doc.repair_log());
}

#[test]
fn a_stream_whose_length_is_itself_and_never_ends_loads_quickly() {
    // From the nightly fuzz job: `6 0 obj << /Length 6 0 R >> stream` with no `endstream`
    // after it. Every parse attempt failed, and each attempt resolved the length by loading
    // object 6 again, retrying the same ways down to the nesting limit: about 2^32 parses.
    // Objects being loaded are now a seen-set, lengths are memoised, and the second parse
    // only runs when the header is shifted.
    let mut bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n".to_vec();
    bytes.extend_from_slice(b"6 0 obj\n<< /Length 6 0 R >>\nstream\nno end marker follows\n");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Ok(doc) = Document::open(Arc::new(bytes)) {
            let _ = doc.try_get(6);
        }
        let _ = tx.send(());
    });
    rx.recv_timeout(std::time::Duration::from_secs(20)).expect("loading must not retry exponentially");
}

#[test]
fn a_restored_revision_keeps_what_the_saved_file_held_in_object_streams() {
    // The earlier revision: an empty document read back from a file whose catalog and pages
    // sit in an object stream.
    let earlier = Document::open(Arc::new(crate::write_full(&Document::new_empty(), &crate::SaveOptions::default()).unwrap())).unwrap();
    // The saved revision adds an object the catalog links to, so a full save numbers its objects
    // anew; an undo then returns to the earlier graph.
    let mut later = earlier.clone();
    let extra = later.add(Object::Int(7));
    let root = later.root().unwrap();
    later.update_dict(root, |c| c.set(b"Extra".to_vec(), Object::Ref(extra))).unwrap();
    let saved = crate::write_full(&later, &crate::SaveOptions::default()).unwrap();
    let mut base = Document::open(Arc::new(saved)).unwrap();
    assert!(
        base.object_numbers().iter().any(|&n| matches!(base.xref_entry(n), Some(XrefEntry::InStream { .. }))),
        "the saved file holds objects in a stream"
    );
    base.overlay_changes_from(&earlier).unwrap();
    let file = crate::write_incremental(&base, &crate::SaveOptions::default()).unwrap();
    let reread = Document::open(Arc::new(file)).unwrap();
    let catalog = reread.get(reread.root().unwrap()).as_dict().unwrap().clone();
    assert!(catalog.get(b"Extra").is_none(), "the restored catalog");
    let pages = reread.resolve(catalog.get(b"Pages").unwrap());
    assert_eq!(pages.as_dict().and_then(|d| d.name(b"Type")), Some(&b"Pages"[..]), "the pages survive: {pages:?}");
}

/// The cache keeps the sum of what its objects hold as they come and go, so neither reader has
/// to measure the cache again on every read (#5873).
#[test]
fn the_object_cache_keeps_its_total_as_objects_come_and_go() {
    let data = Arc::new(Vec::new());
    let one = Arc::new(Object::String(crate::PdfString::literal(vec![b'x'; 100])));
    let two = Arc::new(Object::Array(vec![Object::Int(1); 10]));
    let (a, b) = (held_bytes(&one, &data, 0), held_bytes(&two, &data, 0));
    let mut cache = super::state::ObjectCache::default();
    cache.insert(1, one.clone(), a);
    cache.insert(2, two, b);
    assert_eq!(cache.owned(), a + b);
    cache.insert(2, one, a);
    assert_eq!(cache.owned(), a + a, "a replaced object no longer counts");
    cache.retain(|n| *n == 1);
    assert_eq!((cache.len(), cache.owned()), (1, a));
    cache.clear();
    assert_eq!((cache.len(), cache.owned()), (0, 0));
}

/// A dictionary of numbers owns no string or stream bytes, but parsed it is tens of thousands
/// of values: InDesign's per-page `/PieceInfo`. Counting only strings and streams, the editing
/// cache kept a thousand such pages and a 35 MB book held 1.5 GB (#5873). What the parsed tree
/// holds counts now, so the byte budget bounds them.
#[test]
fn the_editor_cache_charges_a_dictionary_for_the_values_it_holds() {
    let entries: String = (0..2000).map(|i| format!("/{i}[{i} 141 3 116.22 -375.993 1.0]")).collect();
    let dict = format!("<< /PieceInfo << /InDesign << /Map << {entries} >> >> >> >>");
    let count = 300;
    let refs: String = (0..count).map(|i| format!("{} 0 R ", i + 2)).collect();
    let mut bodies = vec![format!("<< /Type /Catalog /Extension [{refs}] >>")];
    bodies.extend((0..count).map(|_| dict.clone()));
    let bodies: Vec<&str> = bodies.iter().map(String::as_str).collect();
    let doc = Document::open(Arc::new(build(&bodies, "/Root 1 0 R"))).unwrap();
    let one = doc.get(crate::ObjRef::new(2, 0));
    let held = held_bytes(&one, &doc.data, 0);
    assert!(held > 2000 * std::mem::size_of::<Object>(), "the parsed tree counts: {held} bytes");
    for i in 0..count {
        drop(doc.get(crate::ObjRef::new(i as u32 + 2, 0)));
    }
    let cache = doc.cache.lock().unwrap();
    assert!(cache.owned() <= Document::EDITOR_CACHE_BYTES, "{} bytes held", cache.owned());
    assert!(cache.len() < count, "the byte budget evicted some, below the count cap: {} kept", cache.len());
    let sum = cache.values().fold(0usize, |n, o| n.saturating_add(held_bytes(o, &doc.data, 0)));
    assert_eq!(cache.owned(), sum, "the kept total is the cache's own");
}
