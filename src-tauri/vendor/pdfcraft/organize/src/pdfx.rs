//! The print standard a document declares (PDF/X, ISO 15930), carried into the new documents
//! Extract, Split and Combine make from its pages (#263). Apart from its pages, a PDF/X file is
//! the catalog's `/OutputIntents` (the printing condition and its ICC profile) and the version
//! it declares, in the document information (`GTS_PDFXVersion`, `GTS_PDFXConformance`,
//! `Trapped`) and in its XMP metadata (`pdfxid:`).
//!
//! The new document gets an XMP packet of its own, holding that identification and the title.
//! The source's packet is not copied: it can also claim standards the new document no longer
//! meets, such as PDF/UA, whose structure tree is not copied.

use std::collections::HashMap;

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, Stream};

use crate::OrganizeError;

/// Decoding limit for an output profile or an XMP packet (ICC profiles are a few MB at most).
const MAX_DECODED: usize = 64 << 20;

/// What a document declares about the print standard it meets.
#[derive(Debug, Default)]
pub(crate) struct PrintStandard {
    /// The catalog's `/OutputIntents`, as written.
    intents: Option<Object>,
    /// Each output intent's type (`/S`), printing condition and decoded profile, to compare.
    conditions: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>,
    /// `GTS_PDFXVersion` ("PDF/X-4"), from the document information or else the XMP metadata.
    version: Option<String>,
    /// `GTS_PDFXConformance` ("PDF/X-1a:2001"), which PDF/X-1a and PDF/X-3 add.
    conformance: Option<String>,
    /// `Trapped`.
    trapped: Option<&'static str>,
    /// Whether the document has XMP metadata (a PDF/X-1a:2001 file, PDF 1.3, has none).
    xmp: bool,
}

impl PrintStandard {
    /// What `doc` declares.
    pub(crate) fn of(doc: &Document) -> Self {
        let catalog = doc.root().and_then(|r| doc.get(r).as_dict().cloned()).unwrap_or_default();
        let intents = catalog.get(b"OutputIntents").cloned();
        let list = intents.as_ref().and_then(|o| doc.resolve(o).as_array().cloned()).unwrap_or_default();
        let conditions = list
            .iter()
            .map(|oi| {
                let oi = doc.resolve(oi);
                let d = oi.as_dict();
                let kind = d.and_then(|d| d.name(b"S")).unwrap_or_default().to_vec();
                let id = d.and_then(|d| d.get(b"OutputConditionIdentifier")).and_then(|v| doc.resolve(v).as_string().map(|s| s.bytes.clone()));
                let profile = d.and_then(|d| d.get(b"DestOutputProfile")).map(|p| doc.resolve(p)).and_then(|p| match &*p {
                    // Undecodable: compare the bytes as stored.
                    Object::Stream(s) => Some(s.decoded_within(MAX_DECODED).unwrap_or_else(|_| s.raw.to_vec())),
                    _ => None,
                });
                (kind, id.unwrap_or_default(), profile.unwrap_or_default())
            })
            .collect();
        let xmp = metadata(doc, &catalog);
        let declared = |info_key: &str, xmp_key: &str| {
            crate::info(doc, info_key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .or_else(|| xmp.as_deref().and_then(|x| xmp_value(x, xmp_key)))
        };
        let trapped = info_trapped(doc).or_else(|| xmp.as_deref().and_then(|x| xmp_value(x, "pdf:Trapped")).and_then(|t| trapped(t.as_bytes())));
        Self {
            intents,
            conditions,
            version: declared("GTS_PDFXVersion", "pdfxid:GTS_PDFXVersion"),
            conformance: declared("GTS_PDFXConformance", "pdfxid:GTS_PDFXConformance"),
            trapped,
            xmp: xmp.is_some(),
        }
    }

    /// Whether the document declares output intents (its printing condition and profile).
    pub(crate) fn declares_output_intents(&self) -> bool {
        self.intents.is_some()
    }

    /// Whether `other` declares the same standard: the same version and trapping, and the same
    /// output intents (type, printing condition and profile).
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.conditions == other.conditions && self.version == other.version && self.conformance == other.conformance && self.trapped == other.trapped
    }
}

/// Give `dst` the print standard `src` declares: its output intents, copied with their profiles
/// (through `map`, what the pages already brought along), and, for PDF/X, the document
/// information entries and the XMP metadata that identify it. The output intents are copied from
/// `src`, so a standard that declares them needs the source document (`None` otherwise).
pub(crate) fn carry(dst: &mut Document, src: Option<&Document>, standard: &PrintStandard, map: HashMap<ObjRef, ObjRef>) -> Result<(), OrganizeError> {
    let root = dst.root().ok_or(OrganizeError::NoPageTree)?;
    if let Some(intents) = &standard.intents {
        let src = src.ok_or_else(|| OrganizeError::Invalid("the output intents' source document was not kept".into()))?;
        let copied = crate::import::copy_object(dst, src, intents, map)?;
        dst.update_dict(root, |c| c.set(b"OutputIntents".to_vec(), copied))?;
    }
    let Some(version) = &standard.version else { return Ok(()) };
    crate::set_info_entry(dst, b"GTS_PDFXVersion", Some(Object::String(PdfString::text(version))))?;
    if let Some(c) = &standard.conformance {
        crate::set_info_entry(dst, b"GTS_PDFXConformance", Some(Object::String(PdfString::text(c))))?;
    }
    if let Some(t) = standard.trapped {
        crate::set_info_entry(dst, b"Trapped", Some(Object::name(t)))?;
    }
    if standard.xmp {
        let mut md = Dict::new();
        md.set(b"Type".to_vec(), Object::name("Metadata"));
        md.set(b"Subtype".to_vec(), Object::name("XML"));
        // Uncompressed, so other tools can find it.
        let m = dst.add(Object::Stream(Stream::from_raw(md, packet(dst, standard, version).into_bytes())));
        dst.update_dict(root, |c| c.set(b"Metadata".to_vec(), Object::Ref(m)))?;
    }
    Ok(())
}

/// The XMP packet identifying `doc` as `version`, with its title and trapping.
fn packet(doc: &Document, standard: &PrintStandard, version: &str) -> String {
    let mut props = String::new();
    if let Some(t) = crate::info(doc, "Title").filter(|t| !t.trim().is_empty()) {
        props.push_str(&format!("   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n", esc(&t)));
    }
    if let Some(t) = standard.trapped {
        props.push_str(&format!("   <pdf:Trapped>{t}</pdf:Trapped>\n"));
    }
    props.push_str(&format!("   <pdfxid:GTS_PDFXVersion>{}</pdfxid:GTS_PDFXVersion>\n", esc(version)));
    if let Some(c) = &standard.conformance {
        props.push_str(&format!("   <pdfxid:GTS_PDFXConformance>{}</pdfxid:GTS_PDFXConformance>\n", esc(c)));
    }
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
  <rdf:Description rdf:about=\"\"\n\
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n\
    xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"\n\
    xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\">\n\
{props}  </rdf:Description>\n\
 </rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>"
    )
}

/// The document information's `/Trapped` (a name, or a string in older files).
fn info_trapped(doc: &Document) -> Option<&'static str> {
    let info = doc.resolve(doc.trailer().get(b"Info")?);
    let v = doc.resolve(info.as_dict()?.get(b"Trapped")?);
    match &*v {
        Object::Name(n) => trapped(n),
        Object::String(s) => trapped(&s.bytes),
        _ => None,
    }
}

fn trapped(v: &[u8]) -> Option<&'static str> {
    match v {
        b"True" => Some("True"),
        b"False" => Some("False"),
        b"Unknown" => Some("Unknown"),
        _ => None,
    }
}

/// The catalog's XMP metadata, as text.
fn metadata(doc: &Document, catalog: &Dict) -> Option<String> {
    match &*doc.resolve(catalog.get(b"Metadata")?) {
        Object::Stream(s) => s.decoded_within(MAX_DECODED).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
        _ => None,
    }
}

/// A simple XMP property, written as an attribute (`pdfxid:GTS_PDFXVersion="PDF/X-4"`) or as an
/// element (`<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>`).
fn xmp_value(xmp: &str, key: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let attr = format!("{key}={quote}");
        if let Some(rest) = xmp.find(&attr).and_then(|i| xmp.get(i + attr.len()..)) {
            return rest.find(quote).and_then(|end| rest.get(..end)).map(unescape).filter(|v| !v.is_empty());
        }
    }
    let open = format!("<{key}>");
    let rest = xmp.get(xmp.find(&open)? + open.len()..)?;
    let value = rest.get(..rest.find(&format!("</{key}>"))?)?;
    Some(unescape(value.trim())).filter(|v| !v.is_empty())
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// `s` as XML text: markup escaped, and characters XML 1.0 does not allow left out.
pub(crate) fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| (!c.is_control() || matches!(c, '\t' | '\n' | '\r')) && !matches!(c, '\u{FFFE}' | '\u{FFFF}'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
