//! A shared allowance for operations that decode many streams.

use std::collections::HashSet;

use pdfcraft_filters::FilterError;

use crate::{CosError, Document, ObjRef, Object, Stream};

/// The steps of decoding an operation over a document's images may take whatever the file's size.
/// A step of JBIG2 decoding ([`hayro_jbig2::Image::decode_within`]) is about a pixel, and takes 6 to
/// 16 ns, so this is 2 to 5 minutes, and about as long again for what an operation does with the
/// pixels. A page scanned at 300 dpi takes about 17 million, 8.4 for its pixels and that again to
/// decode them, so this alone reads about 1,000 of them. A byte into or out of a filter is a step
/// too, and takes less: 3 ns at most, or up to 19 ns through a predictor on samples of a bit.
const IMAGE_WORK: u64 = 1 << 34;
/// The steps each byte of the file adds to [`IMAGE_WORK`], so that a long scan is read whole: such a
/// page pays for itself with about 2 KB of the file, about the smallest a page of text is stored
/// in, and a stream of any filter this reads can be decoded about 8 times at Flate's best, 1,032 to
/// one. So what a hostile file can ask for grows with its length, by about 14 minutes for 10 MB.
const IMAGE_WORK_PER_BYTE: u64 = 1 << 13;

/// Decoded bytes one page's content walk may produce in all: its `/Contents` streams and every
/// form XObject it enters, each occurrence counted again.
pub const MAX_PAGE_DECODED: usize = 256 << 20;
/// Form XObjects one page's content walk may enter, each entry counted again.
pub const MAX_PAGE_FORMS: usize = 50_000;

/// The most pixels an image redaction decodes, to clear the ones under a mark or to store again
/// without the `/JBIG2Globals` it shares with a redacted image (`pdfcraft_redact`). Optimize has
/// JBIG2 images share a dictionary only within it (`pdfcraft_optimize`), so that redaction can
/// store each of them again: one it couldn't would refuse the redaction.
pub const MAX_REDACTED_IMAGE_PIXELS: u64 = 64 << 20;

/// The steps a sample of a JPEG costs: a few bytes can hold a page of one grey, and decoding,
/// resampling and storing it took about 20 to 45 ns a sample.
const JPEG_WORK: u64 = 2;
/// The steps a byte of a JPEG costs to read: its tables took 24 ns a byte before a size was known,
/// and images can share one, so reading it again for each has to count.
const JPEG_READ_WORK: u64 = 4;

/// A stream decode within a byte bound, with the bytes its filter stages decoded: the data or
/// the error, each with that count.
type Decoder = fn(&Stream, usize) -> Result<(Vec<u8>, usize), (FilterError, usize)>;

/// What one operation over a document's images may still spend decoding them: exporting them,
/// optimizing them, and storing LZW streams with Flate as it does, clearing them for redaction.
///
/// An image has its own limit, which doesn't bound a file of thousands of images made to reach it:
/// a few hundred bytes of JBIG2 can ask for seconds of decoding, 4 KB of Flate inside Flate for a
/// gigabyte, and an image that shares `/JBIG2Globals` decodes them again. So images share this
/// allowance, whatever they do with what they decode and whether or not it works, and past it they
/// aren't decoded. A caller then treats them as it does an image it can't read, and
/// [`ImageBudget::refused`] says how many there were.
///
/// An image also pays for its pixels once decoded, as what reads it goes through them one by one
/// however few bytes coded them: blank rows of fax or Flate take almost none. It pays for no more
/// than its data holds, a bit a pixel at least, so an image declaring a size its data doesn't fill
/// costs what a smaller one would, not the run's allowance. A JPEG pays for its samples when
/// it is decoded ([`ImageBudget::pay_jpeg`]), and is otherwise not charged, as JPEG 2000 isn't:
/// their decoders don't run here.
#[derive(Debug)]
pub struct ImageBudget {
    work: u64,
    refused: usize,
}

impl ImageBudget {
    /// The allowance for an operation on `doc`, which a longer file makes larger. The streams made
    /// or changed since it was opened count as the file's bytes do, so a document built in memory
    /// (combined, created, or given pages of scans) gets what it would once saved and opened again.
    pub fn new(doc: &Document) -> Self {
        let length = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        let edited = doc
            .scan_objects_subset(doc.modified_objects())
            .filter_map(|(_, object)| match &*object.ok()? {
                Object::Stream(s) => Some(length(s.raw.len())),
                _ => None,
            })
            .fold(0u64, u64::saturating_add);
        let bytes = length(doc.bytes().len()).saturating_add(edited);
        Self::with_work(IMAGE_WORK.saturating_add(bytes.saturating_mul(IMAGE_WORK_PER_BYTE)))
    }

    /// An allowance of `work` steps of decoding.
    pub fn with_work(work: u64) -> Self {
        Self { work, refused: 0 }
    }

    /// The samples of image `s`, decoded and charged to the allowance: the one way to decode an
    /// image outside the renderer, so that an operation over many images can't decode them without
    /// one.
    ///
    /// They go through its filters ([`Stream::decoded`]), or, for a CCITT fax or JBIG2 image (its
    /// last filter), through the filters before it and then [`pdfcraft_filters::decode_ccitt_within`]
    /// with its [`Document::fax_params`], or [`pdfcraft_filters::decode_jbig2_within`] with its
    /// `/JBIG2Globals`, as `CCITTFaxDecode` and `JBIG2Decode` give them (ISO 32000-2 §7.4.6,
    /// §7.4.7). Parameters given by reference count, for every filter
    /// ([`Document::with_direct_parms`]). A fax image's `/Columns` has to be its `/Width`, and a
    /// JBIG2 image's page its `/Width` × `/Height`: PdfCraft's renderer draws the whole page into
    /// the image. Up to its `/Height` rows are decoded, whatever a fax image's `/Rows` says; damaged
    /// fax data gives fewer, so a caller checks it has the rows it needs. Damaged JBIG2 data, and
    /// globals that can't be read (within [`MAX_JBIG2_GLOBALS`](crate::MAX_JBIG2_GLOBALS)), are an
    /// error.
    ///
    /// No stage, the filters before a fax or JBIG2 decoder included, may produce more than `max`
    /// bytes, nor more than [`MAX_DECODED`](crate::MAX_DECODED). A JBIG2 image's `/JBIG2Globals`
    /// are another stream, shared between images, and are decoded within `MAX_JBIG2_GLOBALS`
    /// whatever `max` is. Each filter as it runs, the globals when they have to be read, and the fax
    /// or JBIG2 decoder spend from the allowance, and the image then pays for its pixels; with
    /// nothing left an image is refused before anything is read, its globals included.
    ///
    /// ```
    /// # let doc = pdfcraft_cos::Document::new_empty();
    /// # let s = pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), Vec::new());
    /// let samples = pdfcraft_cos::ImageBudget::new(&doc).decode(&doc, &s, 1 << 20);
    /// assert_eq!(samples.unwrap(), b"");
    /// ```
    ///
    /// `Document` has no way of its own to decode an image:
    ///
    /// ```compile_fail
    /// # let doc = pdfcraft_cos::Document::new_empty();
    /// # let s = pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), Vec::new());
    /// let _ = doc.decoded_image(&s);
    /// ```
    ///
    /// ```compile_fail
    /// # let doc = pdfcraft_cos::Document::new_empty();
    /// # let s = pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), Vec::new());
    /// let _ = doc.decoded_image_within(&s, 1 << 20);
    /// ```
    pub fn decode(&mut self, doc: &Document, s: &Stream, max: usize) -> Result<Vec<u8>, CosError> {
        let decoded = doc.decoded_image_spending(s, max, &mut self.work);
        self.count_refused(s, &decoded);
        self.pay_size(s, decoded?)
    }

    /// Image `s` through its filters alone, charged to the allowance: [`Stream::decoded_within`]
    /// with its `/DecodeParms` given by reference read too ([`Document::with_direct_parms`]), for an
    /// image that has no fax or JBIG2 decoder to go through.
    pub fn decode_filters(&mut self, doc: &Document, s: &Stream, max: usize) -> Result<Vec<u8>, CosError> {
        let decoded = doc.with_direct_parms(s).decoded_spending(max, &mut self.work);
        self.count_refused(s, &decoded);
        let decoded = decoded?;
        if s.filters().iter().all(|(f, _)| f.is_decodable()) { self.pay_size(s, decoded) } else { Ok(decoded) }
    }

    /// `data` through `chain`, any corruption being an error ([`pdfcraft_filters::decode_within`]),
    /// for a stream a run rewrites rather than an image it reads, charged to the allowance.
    pub fn decode_chain(
        &mut self,
        chain: &[(pdfcraft_filters::Filter, pdfcraft_filters::Params)],
        data: &[u8],
        max: usize,
    ) -> Result<Vec<u8>, FilterError> {
        pdfcraft_filters::decode_within(chain, data, max, &mut self.work)
    }

    /// Pays `steps` of other work on decoded data, or spends all there is and says so.
    pub fn spend(&mut self, steps: u64) -> bool {
        let left = self.work.checked_sub(steps);
        self.work = left.unwrap_or(0);
        left.is_some()
    }

    /// Pays for `pixels` a decode goes on to work through, or spends all there is, and the image
    /// is refused.
    pub fn pay_pixels(&mut self, pixels: u64) -> bool {
        let paid = self.spend(pixels);
        self.refused += usize::from(!paid);
        paid
    }

    /// Pays for reading JPEG `data` and decoding its `samples`, as [`ImageBudget::pay_pixels`] does.
    /// A caller pays before its tables are parsed, which can be most of the file.
    pub fn pay_jpeg(&mut self, samples: u64, data: &[u8]) -> bool {
        let reading = (data.len() as u64).saturating_mul(JPEG_READ_WORK);
        self.pay_pixels(samples.saturating_mul(JPEG_WORK).saturating_add(reading))
    }

    /// Pays for the pixels of image `s` that its `decoded` data holds, those of its `/Width` and
    /// `/Height` but no more than a bit of the data each, unless it is JBIG2, whose decoder pays for
    /// its page, or isn't decoded here. What can't be paid for refuses the image.
    fn pay_size(&mut self, s: &Stream, decoded: Vec<u8>) -> Result<Vec<u8>, CosError> {
        if !charged(s) || matches!(s.filters().last(), Some((pdfcraft_filters::Filter::Jbig2, _))) {
            return Ok(decoded);
        }
        let side = |key: &[u8]| s.dict.int(key).and_then(|v| u64::try_from(v).ok()).unwrap_or(0);
        let held = u64::try_from(decoded.len()).unwrap_or(u64::MAX).saturating_mul(8);
        if self.pay_pixels(side(b"Width").saturating_mul(side(b"Height")).min(held)) {
            Ok(decoded)
        } else {
            Err(CosError::Filter(FilterError::Spent.to_string()))
        }
    }

    /// An image that failed with nothing left was refused, unless it has a filter that costs
    /// nothing here, which fails whatever is left.
    fn count_refused(&mut self, s: &Stream, decoded: &Result<Vec<u8>, CosError>) {
        if decoded.is_err() && self.work == 0 && charged(s) {
            self.refused += 1;
        }
    }

    /// The decodes refused for lack of allowance: an image read twice over is counted twice.
    pub fn refused(&self) -> usize {
        self.refused
    }

    /// The steps of decoding still allowed.
    pub fn work_left(&self) -> u64 {
        self.work
    }
}

/// Whether `s` can be decoded here and so charged: general filters, a fax or JBIG2 decoder last at
/// most. Those decoders anywhere else can't run.
fn charged(s: &Stream) -> bool {
    use pdfcraft_filters::Filter;
    s.readable_filters().is_ok_and(|chain| {
        chain.split_last().is_none_or(|((last, _), before)| {
            (last.is_decodable() || matches!(last, Filter::Jbig2 | Filter::CcittFax)) && before.iter().all(|(f, _)| f.is_decodable())
        })
    })
}

/// What one operation may still decode and enter across all the streams it reads.
///
/// [`MAX_DECODED`](crate::MAX_DECODED) bounds one decode. It does not bound a page whose
/// `/Contents` names the same compressed stream thousands of times, or a form drawn thousands of
/// times from other forms: each occurrence would get a fresh allowance. A `DecodeBudget` is
/// shared by every decode of one operation, so the total decoded bytes and form entries stay
/// under fixed limits however the file repeats its references. A decode that runs past the
/// remaining allowance spends all of it, so a bomb referenced many times is inflated once.
#[derive(Debug)]
pub struct DecodeBudget {
    /// The whole allowance, for errors.
    limit: usize,
    bytes: usize,
    forms: usize,
    seen: HashSet<ObjRef>,
}

impl DecodeBudget {
    /// An allowance of `bytes` decoded bytes and `forms` form entries.
    pub fn new(bytes: usize, forms: usize) -> Self {
        Self { limit: bytes, bytes, forms, seen: HashSet::new() }
    }

    /// The allowance for one page: [`MAX_PAGE_DECODED`] bytes and [`MAX_PAGE_FORMS`] forms.
    pub fn page() -> Self {
        Self::new(MAX_PAGE_DECODED, MAX_PAGE_FORMS)
    }

    /// Decoded bytes still allowed.
    pub fn remaining(&self) -> usize {
        self.bytes
    }

    /// Whether a decode was refused for lack of allowance or a form entry for lack of visits.
    pub fn exhausted(&self) -> bool {
        self.bytes == 0 || self.forms == 0
    }

    /// `stream` decoded like [`Stream::decoded`], charged to the allowance. Fails, without
    /// decoding, once the allowance is spent.
    pub fn decode(&mut self, stream: &Stream) -> Result<Vec<u8>, CosError> {
        self.charge(stream, usize::MAX, Stream::decoded_within_counted)
    }

    /// Like [`DecodeBudget::decode`], for a stream with a tighter bound of its own: decoding
    /// past `max` bytes is an error that charges `max`, the work it took to find out.
    pub fn decode_within(&mut self, stream: &Stream, max: usize) -> Result<Vec<u8>, CosError> {
        self.charge(stream, max, Stream::decoded_within_counted)
    }

    /// `stream` decoded like [`Stream::decoded_strict`] (any corruption is an error), charged to
    /// the allowance.
    pub fn decode_strict(&mut self, stream: &Stream) -> Result<Vec<u8>, CosError> {
        self.charge(stream, usize::MAX, Stream::decoded_strict_within_counted)
    }

    /// Decodes `stream` within `max` and the allowance, charging what every filter stage
    /// decoded: a stage's output counts even when a later stage shrinks it, and a corrupt stream
    /// pays for the bytes decoded before the error, so referencing a stream again and again
    /// spends the allowance by the work each decode does.
    fn charge(&mut self, stream: &Stream, max: usize, decode: Decoder) -> Result<Vec<u8>, CosError> {
        if self.bytes == 0 && !stream.raw.is_empty() {
            return Err(CosError::Filter(FilterError::LimitExceeded(self.limit).to_string()));
        }
        if max < self.bytes {
            let result = decode(stream, max);
            let spent = match &result {
                Ok((_, spent)) => *spent,
                Err((FilterError::LimitExceeded(_), _)) => max,
                Err((_, spent)) => *spent,
            };
            self.bytes = self.bytes.saturating_sub(spent);
            return result.map(|(data, _)| data).map_err(|(e, _)| CosError::Filter(e.to_string()));
        }
        match decode(stream, self.bytes) {
            Ok((data, spent)) => {
                self.bytes = self.bytes.saturating_sub(spent);
                Ok(data)
            }
            Err((FilterError::LimitExceeded(_), _)) => {
                self.bytes = 0;
                Err(CosError::Filter(FilterError::LimitExceeded(self.limit).to_string()))
            }
            Err((e, spent)) => {
                self.bytes = self.bytes.saturating_sub(spent);
                Err(CosError::Filter(e.to_string()))
            }
        }
    }

    /// Counts one form entry. `false` once [`DecodeBudget::new`]'s `forms` entries are spent:
    /// the caller leaves that form out.
    pub fn enter_form(&mut self) -> bool {
        match self.forms.checked_sub(1) {
            Some(left) => {
                self.forms = left;
                true
            }
            None => false,
        }
    }

    /// `true` the first time `r` is passed to this budget, for callers that need each
    /// referenced stream only once.
    pub fn first_visit(&mut self, r: ObjRef) -> bool {
        self.seen.insert(r)
    }

    /// Each stream `contents` names, decoded against the allowance, in order. `contents` is a
    /// page's `/Contents`: one stream, or an array whose non-stream entries are left out.
    /// A stream that fails to decode, or arrives after the allowance is spent, is an `Err` in its
    /// place, so the caller chooses between skipping it and giving up.
    pub fn decode_contents(&mut self, doc: &Document, contents: &Object) -> Vec<Result<Vec<u8>, CosError>> {
        match &*doc.resolve(contents) {
            Object::Stream(s) => vec![self.decode(s)],
            Object::Array(list) => list
                .iter()
                .filter_map(|entry| match &*doc.resolve(entry) {
                    Object::Stream(s) => Some(self.decode(s)),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use crate::Dict;

    fn jbig2(parms: Option<Object>) -> Stream {
        let mut dict = Dict::new();
        dict.set(b"Width".to_vec(), Object::Int(16));
        dict.set(b"Height".to_vec(), Object::Int(2));
        dict.set(b"Filter".to_vec(), Object::name("JBIG2Decode"));
        if let Some(parms) = parms {
            dict.set(b"DecodeParms".to_vec(), parms);
        }
        Stream::from_raw(dict, pdfcraft_filters::encode_jbig2_generic(&[0xFF; 4], 16, 2).unwrap())
    }

    /// Each image spends what it took from one allowance, and those past it are refused and
    /// counted, whatever their filters. What can't be decoded here costs nothing, and fails
    /// without being one of these.
    #[test]
    fn the_images_of_an_operation_share_its_allowance() {
        let doc = Document::new_empty();
        let image = jbig2(None);
        let mut budget = ImageBudget::with_work(u64::MAX);
        let samples = budget.decode(&doc, &image, 1 << 20).unwrap();
        let cost = u64::MAX - budget.work_left();
        assert!(cost > 0);
        let mut budget = ImageBudget::with_work(3 * cost);
        for left in [2, 1, 0] {
            assert_eq!(budget.decode(&doc, &image, 1 << 20).unwrap(), samples);
            assert_eq!(budget.work_left(), left * cost);
        }
        assert_eq!(budget.refused(), 0, "the allowance held exactly three");
        for refused in [1, 2] {
            assert!(budget.decode(&doc, &image, 1 << 20).is_err());
            assert_eq!(budget.refused(), refused);
        }
        assert!(budget.decode(&doc, &Stream::flate(Dict::new(), b"samples"), 1 << 20).is_err());
        assert!(budget.decode(&doc, &Stream::from_raw(Dict::new(), b"samples".to_vec()), 1 << 20).is_err());
        assert_eq!(budget.refused(), 4, "other images share it");
        for name in ["NoSuchFilter", "DCTDecode", "JPXDecode"] {
            let mut dict = Dict::new();
            dict.set(b"Filter".to_vec(), Object::name(name));
            assert!(budget.decode(&doc, &Stream::from_raw(dict, b"data".to_vec()), 1 << 20).is_err());
            assert_eq!(budget.refused(), 4, "{name} isn't one of these");
        }
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::Int(5));
        assert!(budget.decode(&doc, &Stream::from_raw(dict, b"data".to_vec()), 1 << 20).is_err());
        assert_eq!(budget.refused(), 4, "a /Filter that can't be read isn't either");
    }

    /// What a file of images each inflating a megabyte asks for: 4 KB of Flate inside Flate can ask
    /// for a gigabyte, and these aren't JBIG2, so without the allowance nothing would stop them.
    #[test]
    fn filter_bombs_stop_at_the_allowance() {
        let doc = Document::new_empty();
        let samples = vec![0; 1 << 20];
        let inner = pdfcraft_filters::encode_flate(&samples);
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::Array(vec![Object::name("FlateDecode"), Object::name("FlateDecode")]));
        let bomb = Stream::from_raw(dict, pdfcraft_filters::encode_flate(&inner));
        let cost = (bomb.raw.len() + inner.len() + samples.len()) as u64;
        let mut budget = ImageBudget::with_work(2 * cost + 1);
        for left in [cost + 1, 1] {
            assert_eq!(budget.decode(&doc, &bomb, 1 << 20).unwrap(), samples);
            assert_eq!(budget.work_left(), left);
        }
        assert!(budget.decode(&doc, &bomb, 1 << 20).is_err());
        assert_eq!((budget.work_left(), budget.refused()), (0, 1), "the first it can't pay for ends it");
    }

    /// Filters in front of a JBIG2 decoder pay as they run, on top of what the decoder takes, and
    /// one that fails has still paid: nothing about the JBIG2 data is known by then, and each image
    /// of a file can do this.
    #[test]
    fn filters_in_front_of_jbig2_pay_whether_or_not_they_work() {
        let doc = Document::new_empty();
        let alone = jbig2(None);
        let mut budget = ImageBudget::with_work(u64::MAX);
        let samples = budget.decode(&doc, &alone, 1 << 20).unwrap();
        let decoding = u64::MAX - budget.work_left();
        let behind = |raw: Vec<u8>| {
            let mut dict = alone.dict.clone();
            dict.set(b"Filter".to_vec(), Object::Array(vec![Object::name("FlateDecode"), Object::name("JBIG2Decode")]));
            Stream::from_raw(dict, raw)
        };

        let image = behind(pdfcraft_filters::encode_flate(&alone.raw));
        let mut budget = ImageBudget::with_work(u64::MAX);
        assert_eq!(budget.decode(&doc, &image, 1 << 20).unwrap(), samples);
        assert_eq!(u64::MAX - budget.work_left(), decoding + (image.raw.len() + alone.raw.len()) as u64);

        // Past this image's limit: an error, with what finding out took paid and the rest left.
        let bomb = behind(pdfcraft_filters::encode_flate(&vec![0; 1 << 16]));
        let mut budget = ImageBudget::with_work(u64::MAX);
        assert!(budget.decode(&doc, &bomb, 1000).is_err());
        assert_eq!(u64::MAX - budget.work_left(), bomb.raw.len() as u64 + 1000);
        assert_eq!(budget.refused(), 0, "that image's limit isn't the run's");

        // A small allowance is a limit too: it stops there, and the image is refused.
        let mut budget = ImageBudget::with_work(bomb.raw.len() as u64 + 10);
        assert!(budget.decode(&doc, &bomb, 1 << 20).is_err());
        assert_eq!((budget.work_left(), budget.refused()), (0, 1));
    }

    /// Globals that are read again for every image pay every time, and fail having paid. Kept
    /// ones have no filters left to undo, so they pay for being read and nothing more.
    #[test]
    fn globals_pay_for_their_filters_each_time_they_are_decoded() {
        let skipped = [0, 0, 0, 0, 0x3E, 0, 0, 0, 0, 0, 4, 0, 0, 0, 1];
        let raw = pdfcraft_filters::encode_flate(&skipped);
        let stream = |doc: &Document, r: ObjRef| match &*doc.get(r) {
            Object::Stream(s) => s.clone(),
            _ => panic!("not a stream"),
        };
        let spent = |doc: &Document, s: &Stream| {
            let mut budget = ImageBudget::with_work(u64::MAX);
            budget.decode(doc, s, 1 << 20).unwrap();
            u64::MAX - budget.work_left()
        };
        let mut doc = Document::new_empty();
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::name("FlateDecode"));
        let globals = doc.add(Object::Stream(Stream::from_raw(dict, raw.clone())));
        let mut parms = Dict::new();
        parms.set(b"JBIG2Globals".to_vec(), Object::Ref(globals));
        let image = doc.add(Object::Stream(jbig2(Some(Object::Dict(parms)))));
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| c.set(b"Image".to_vec(), Object::Ref(image))).unwrap();

        // Added in memory, so not kept: the filters are undone for each read.
        let s = stream(&doc, image);
        let first = spent(&doc, &s);
        assert_eq!(spent(&doc, &s), first, "not kept, so paid for again");

        // Read from a file, so kept: the second read has no filters to undo.
        let file = crate::write_full(&doc, &crate::SaveOptions::default()).unwrap();
        let doc = Document::open(std::sync::Arc::new(file)).unwrap();
        let image = doc.get(doc.root().unwrap()).as_dict().unwrap().reference(b"Image").unwrap();
        let s = stream(&doc, image);
        let filters = (raw.len() + skipped.len()) as u64;
        assert_eq!(spent(&doc, &s), first, "the first read undoes them");
        assert_eq!(spent(&doc, &s), first - filters, "kept");

        // Globals that fail pay too, with the image not refused: these go past their own limit.
        let over = pdfcraft_filters::encode_flate(&vec![0; crate::MAX_JBIG2_GLOBALS + 1]);
        let mut doc = Document::new_empty();
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::name("FlateDecode"));
        let globals = doc.add(Object::Stream(Stream::from_raw(dict, over.clone())));
        let mut parms = Dict::new();
        parms.set(b"JBIG2Globals".to_vec(), Object::Ref(globals));
        let image = jbig2(Some(Object::Dict(parms)));
        let mut budget = ImageBudget::with_work(u64::MAX);
        assert!(budget.decode(&doc, &image, 1 << 20).unwrap_err().to_string().contains("globals"));
        assert_eq!(u64::MAX - budget.work_left(), (over.len() + crate::MAX_JBIG2_GLOBALS) as u64);
        assert_eq!(budget.refused(), 0);
    }

    /// A fax image pays for what comes before its decoder and for the rows it asks for, and past
    /// the allowance it is refused like any other.
    #[test]
    fn fax_images_share_the_allowance() {
        let doc = Document::new_empty();
        let (w, h) = (4096u32, 64u32);
        let coded = pdfcraft_filters::encode_g4(&vec![0xFF; 512 * h as usize], w, h).unwrap();
        let mut parms = Dict::new();
        parms.set(b"K".to_vec(), Object::Int(-1));
        parms.set(b"Columns".to_vec(), Object::Int(i64::from(w)));
        let mut dict = Dict::new();
        dict.set(b"Width".to_vec(), Object::Int(i64::from(w)));
        dict.set(b"Height".to_vec(), Object::Int(i64::from(h)));
        dict.set(b"Filter".to_vec(), Object::Array(vec![Object::name("FlateDecode"), Object::name("CCITTFaxDecode")]));
        dict.set(b"DecodeParms".to_vec(), Object::Array(vec![Object::Null, Object::Dict(parms)]));
        let image = Stream::from_raw(dict, pdfcraft_filters::encode_flate(&coded));
        let pixels = u64::from(w) * u64::from(h);
        let cost = (image.raw.len() + coded.len() + 4 * coded.len() + 512 * h as usize) as u64 + pixels;
        let mut budget = ImageBudget::with_work(cost);
        assert_eq!(budget.decode(&doc, &image, 1 << 20).unwrap().len(), 512 * h as usize);
        assert_eq!(budget.work_left(), 0, "exactly that");
        assert!(budget.decode(&doc, &image, 1 << 20).is_err());
        assert_eq!(budget.refused(), 1);
    }

    /// An image pays for its pixels however few bytes hold them, or a blank page of fax or Flate
    /// would cost a run a second for a few bytes' worth of allowance; a JBIG2 image doesn't pay
    /// twice, and one this doesn't decode pays nothing. Past what is left it isn't read.
    #[test]
    fn an_image_pays_for_its_pixels() {
        let doc = Document::new_empty();
        let blank = |filter: Option<&str>, data: Vec<u8>| {
            let mut dict = Dict::new();
            dict.set(b"Width".to_vec(), Object::Int(4096));
            dict.set(b"Height".to_vec(), Object::Int(1024));
            if let Some(filter) = filter {
                dict.set(b"Filter".to_vec(), Object::name(filter));
            }
            Stream::from_raw(dict, data)
        };
        let rows = vec![0xFFu8; 512 * 1024];
        let flate = blank(Some("FlateDecode"), pdfcraft_filters::encode_flate(&rows));
        let cost = (flate.raw.len() + rows.len()) as u64 + 4096 * 1024;
        let mut budget = ImageBudget::with_work(cost);
        assert_eq!(budget.decode(&doc, &flate, 1 << 24).unwrap(), rows);
        assert_eq!((budget.work_left(), budget.refused()), (0, 0), "exactly that");
        // Its filters alone pay the same.
        let mut budget = ImageBudget::with_work(cost);
        assert_eq!(budget.decode_filters(&doc, &flate, 1 << 24).unwrap(), rows);
        assert_eq!(budget.work_left(), 0);
        // Short of its pixels it is refused, having paid for its filters.
        let mut budget = ImageBudget::with_work(4096 * 1024 - 1);
        assert!(budget.decode(&doc, &flate, 1 << 24).unwrap_err().to_string().contains("more image decoding than"));
        assert_eq!((budget.work_left(), budget.refused()), (0, 1));
        let mut budget = ImageBudget::with_work(4096 * 1024 - 1);
        assert!(budget.decode_filters(&doc, &flate, 1 << 24).is_err());
        assert_eq!((budget.work_left(), budget.refused()), (0, 1));
        // What isn't decoded here pays for nothing, and fails as it would.
        let mut budget = ImageBudget::with_work(10);
        assert!(budget.decode(&doc, &blank(Some("DCTDecode"), b"data".to_vec()), 1 << 24).is_err());
        assert_eq!((budget.work_left(), budget.refused()), (10, 0));
        // Unfiltered data pays too, and a size that isn't one is no size.
        let mut budget = ImageBudget::with_work(u64::MAX);
        budget.decode(&doc, &blank(None, rows.clone()), 1 << 24).unwrap();
        assert_eq!(u64::MAX - budget.work_left(), rows.len() as u64 + 4096 * 1024);
        let mut odd = Dict::new();
        odd.set(b"Width".to_vec(), Object::Int(-4));
        odd.set(b"Height".to_vec(), Object::Int(9));
        let mut budget = ImageBudget::with_work(u64::MAX);
        budget.decode(&doc, &Stream::from_raw(odd, vec![1, 2]), 1 << 24).unwrap();
        assert_eq!(u64::MAX - budget.work_left(), 2);
        // A JPEG pays twice over for each sample, and four times for each byte it is read from.
        let mut budget = ImageBudget::with_work(30);
        assert!(budget.pay_jpeg(9, b"ab"));
        assert_eq!((budget.work_left(), budget.refused()), (4, 0));
        assert!(!budget.pay_jpeg(0, b"ab"));
        assert_eq!((budget.work_left(), budget.refused()), (0, 1));
    }

    /// An image pays for the pixels its data holds, not those its size declares: one claiming
    /// 131072 by 131072 pixels with a few bytes of Flate costs what those bytes give, and the
    /// images after it are decoded as they would be without it.
    #[test]
    fn an_image_pays_for_no_more_pixels_than_its_data_holds() {
        let doc = Document::new_empty();
        let sized = |side: i64, data: &[u8]| {
            let mut dict = Dict::new();
            dict.set(b"Width".to_vec(), Object::Int(side));
            dict.set(b"Height".to_vec(), Object::Int(side));
            Stream::flate(dict, data)
        };
        let claims = sized(131_072, &[0; 16]);
        let image = sized(64, &[7; 64 * 64]);
        let cost = |s: &Stream| {
            let mut budget = ImageBudget::with_work(u64::MAX);
            budget.decode(&doc, s, 1 << 24).unwrap();
            u64::MAX - budget.work_left()
        };
        assert_eq!(cost(&claims), (claims.raw.len() + 16 + 16 * 8) as u64, "its bytes, and a pixel a bit of them");
        assert_eq!(cost(&image), (image.raw.len() + 64 * 64) as u64 + 64 * 64, "its bytes, and its pixels");
        for decode in [ImageBudget::decode, ImageBudget::decode_filters] {
            let mut budget = ImageBudget::with_work(cost(&claims) + cost(&image));
            assert_eq!(decode(&mut budget, &doc, &claims, 1 << 24).unwrap(), [0; 16]);
            assert_eq!(decode(&mut budget, &doc, &image, 1 << 24).unwrap(), [7; 64 * 64], "the image after it is decoded");
            assert_eq!((budget.work_left(), budget.refused()), (0, 0));
        }
    }

    /// A stream a run rewrites is read strictly and charged, and what the caller does with it after
    /// comes out of the same allowance: what can't be paid for spends all of it.
    #[test]
    fn a_rewritten_stream_shares_the_allowance() {
        use pdfcraft_filters::{Filter, FilterError, Params};
        let samples = vec![7u8; 1000];
        let raw = pdfcraft_filters::encode_flate(&samples);
        let chain = [(Filter::Flate, Params::default())];
        let cost = (raw.len() + samples.len()) as u64;
        let mut budget = ImageBudget::with_work(cost + 10);
        assert_eq!(budget.decode_chain(&chain, &raw, 1 << 20).unwrap(), samples);
        assert_eq!(budget.work_left(), 10);
        assert!(budget.spend(4));
        assert_eq!(budget.work_left(), 6);
        assert!(budget.spend(6), "exactly what is left");
        assert_eq!(budget.work_left(), 0);
        assert!(budget.spend(0));
        let mut budget = ImageBudget::with_work(5);
        assert!(!budget.spend(6));
        assert_eq!(budget.work_left(), 0, "spends all of it");
        assert!(matches!(budget.decode_chain(&chain, &raw, 1 << 20), Err(FilterError::Spent)));
        let cut = raw.get(..raw.len() / 2).unwrap();
        assert!(matches!(ImageBudget::with_work(u64::MAX).decode_chain(&chain, cut, 1 << 20), Err(FilterError::Corrupt { .. })), "strictly");
    }

    /// An image read through its filters alone reads parameters given by reference, is charged, and
    /// is refused like the others. It isn't given to a decoder.
    #[test]
    fn filters_alone_are_charged_too() {
        let mut doc = Document::new_empty();
        let p = pdfcraft_filters::Params { predictor: 12, columns: 4, ..pdfcraft_filters::Params::default() };
        let rows = vec![7u8; 400];
        let mut parms = Dict::new();
        parms.set(b"Predictor".to_vec(), Object::Int(12));
        parms.set(b"Columns".to_vec(), Object::Int(4));
        let parms = doc.add(Object::Dict(parms));
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::name("FlateDecode"));
        dict.set(b"DecodeParms".to_vec(), Object::Ref(parms));
        let raw = pdfcraft_filters::encode(&pdfcraft_filters::Filter::Flate, &p, &rows).unwrap();
        let image = Stream::from_raw(dict, raw);
        let mut budget = ImageBudget::with_work(u64::MAX);
        assert_eq!(budget.decode_filters(&doc, &image, 1 << 20).unwrap(), rows);
        let cost = u64::MAX - budget.work_left();
        // The tag byte of each of its 100 rows counts, which the predictor takes off.
        assert_eq!(cost, (image.raw.len() + rows.len() + 100) as u64);
        let mut budget = ImageBudget::with_work(cost - 1);
        assert!(budget.decode_filters(&doc, &image, 1 << 20).is_err());
        assert_eq!((budget.work_left(), budget.refused()), (0, 1));
        let mut budget = ImageBudget::with_work(u64::MAX);
        assert!(budget.decode_filters(&doc, &jbig2(None), 1 << 20).is_err());
        assert_eq!(budget.work_left(), u64::MAX, "no decoder here, so nothing was read");
        // Nor for a fax image, nor for a decoder that isn't last, however large they say they are.
        for chain in [vec!["CCITTFaxDecode"], vec!["CCITTFaxDecode", "FlateDecode"], vec!["JBIG2Decode", "FlateDecode"]] {
            let mut dict = Dict::new();
            dict.set(b"Width".to_vec(), Object::Int(1000));
            dict.set(b"Height".to_vec(), Object::Int(1000));
            dict.set(b"Filter".to_vec(), Object::Array(chain.iter().map(|f| Object::name(f)).collect()));
            let image = Stream::from_raw(dict, b"data".to_vec());
            let mut budget = ImageBudget::with_work(u64::MAX);
            assert!(budget.decode_filters(&doc, &image, 1 << 20).is_err());
            assert_eq!(budget.work_left(), u64::MAX, "{chain:?}");
        }
        // Through `decode` a decoder that isn't last pays nothing either; one that is does.
        let mut dict = Dict::new();
        dict.set(b"Width".to_vec(), Object::Int(1000));
        dict.set(b"Height".to_vec(), Object::Int(1000));
        dict.set(b"Filter".to_vec(), Object::Array(vec![Object::name("JBIG2Decode"), Object::name("FlateDecode")]));
        let mut budget = ImageBudget::with_work(u64::MAX);
        assert!(budget.decode(&doc, &Stream::from_raw(dict, b"data".to_vec()), 1 << 20).is_err());
        assert_eq!(budget.work_left(), u64::MAX);
    }

    /// The images of a file share its globals, so an image refused doesn't read them: this one's
    /// can't be read, which is how to tell.
    #[test]
    fn an_image_past_the_allowance_does_not_read_its_globals() {
        let mut doc = Document::new_empty();
        let mut unreadable = Dict::new();
        unreadable.set(b"Filter".to_vec(), Object::name("NoSuchFilter"));
        let globals = doc.add(Object::Stream(Stream::from_raw(unreadable, b"data".to_vec())));
        let mut parms = Dict::new();
        parms.set(b"JBIG2Globals".to_vec(), Object::Ref(globals));
        let image = jbig2(Some(Object::Dict(parms)));
        let read = ImageBudget::with_work(u64::MAX).decode(&doc, &image, 1 << 20).unwrap_err().to_string();
        assert!(read.contains("globals"), "{read}");
        let refused = ImageBudget::with_work(0).decode(&doc, &image, 1 << 20).unwrap_err().to_string();
        assert!(refused.contains("more image decoding than"), "{refused}");
    }

    /// A longer file gets more, by its length, and an empty one the fixed part.
    #[test]
    fn a_longer_file_allows_more_decoding() {
        assert_eq!(ImageBudget::new(&Document::new_empty()).work_left(), IMAGE_WORK);
        let pdf = b"%PDF-1.7\n1 0 obj << /Type /Catalog >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF".to_vec();
        let len = pdf.len() as u64;
        let doc = Document::open(std::sync::Arc::new(pdf)).unwrap();
        assert_eq!(ImageBudget::new(&doc).work_left(), IMAGE_WORK + len * IMAGE_WORK_PER_BYTE);
    }

    /// A document made in memory, as Combine, Create and inserted scans make one, gets for the
    /// streams it was given what it would get for them once saved: what isn't a stream adds nothing.
    #[test]
    fn streams_made_in_memory_allow_more_decoding_too() {
        let mut doc = Document::new_empty();
        doc.add(Object::Stream(Stream::from_raw(Dict::new(), vec![0; 3000])));
        doc.add(Object::Dict(Dict::new()));
        doc.add(Object::Stream(Stream::flate(Dict::new(), &[1; 100])));
        let flate = pdfcraft_filters::encode_flate(&[1; 100]).len() as u64;
        assert_eq!(ImageBudget::new(&doc).work_left(), IMAGE_WORK + (3000 + flate) * IMAGE_WORK_PER_BYTE);
        // A stream of an opened file changed in memory counts with the file.
        let pdf = b"%PDF-1.7\n1 0 obj << /Type /Catalog >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF".to_vec();
        let len = pdf.len() as u64;
        let mut doc = Document::open(std::sync::Arc::new(pdf)).unwrap();
        doc.add(Object::Stream(Stream::from_raw(Dict::new(), vec![0; 500])));
        assert_eq!(ImageBudget::new(&doc).work_left(), IMAGE_WORK + (len + 500) * IMAGE_WORK_PER_BYTE);
    }

    // What the constants' comments promise, for a page scanned at 300 dpi: about 1,000 of them
    // whatever the file's size, and a page paying for itself with 2 KB of the file.
    const PAGE: u64 = 17_000_000;
    const _: () = assert!(IMAGE_WORK >= 1_000 * PAGE);
    const _: () = assert!(IMAGE_WORK_PER_BYTE * 2_100 >= PAGE);
    // And that a stream can be decoded about 8 times at Flate's best, 1,032 to one, and the byte in.
    const _: () = assert!(IMAGE_WORK_PER_BYTE >= 7 * 1_033);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dict;

    #[test]
    fn repeated_references_share_one_allowance() {
        let mut doc = Document::new_empty();
        let bomb = doc.add(Object::Stream(Stream::flate(Dict::new(), &vec![b' '; 1 << 20])));
        let contents = Object::Array(vec![Object::Ref(bomb); 64]);
        let mut budget = DecodeBudget::new(10 << 20, 1);
        let decoded = budget.decode_contents(&doc, &contents);
        assert_eq!(decoded.len(), 64);
        assert_eq!(decoded.iter().filter(|d| d.is_ok()).count(), 10);
        let total: usize = decoded.iter().flatten().map(Vec::len).sum();
        assert_eq!(total, 10 << 20);
        assert!(budget.exhausted());
        // A decode past the allowance spends it, so a bomb is inflated at most once more.
        let mut budget = DecodeBudget::new((1 << 20) - 1, 1);
        assert!(budget.decode_contents(&doc, &contents).iter().all(Result::is_err));
        assert_eq!(budget.remaining(), 0);
        let mut budget = DecodeBudget::new((1 << 20) - 1, 1);
        let Object::Stream(stream) = &*doc.get(bomb) else { panic!("not a stream") };
        assert!(budget.decode_strict(stream).is_err());
        assert_eq!(budget.remaining(), 0);

        let mut budget = DecodeBudget::new(10 << 20, 1);
        assert!(budget.decode_within(stream, 1000).is_err());
        assert_eq!(budget.remaining(), (10 << 20) - 1000);
        assert_eq!(budget.decode_within(stream, 2 << 20).map(|d| d.len()).ok(), Some(1 << 20));
        assert_eq!(budget.remaining(), (9 << 20) - 1000);

        assert!(budget.enter_form());
        assert!(!budget.enter_form());
        assert!(budget.first_visit(bomb));
        assert!(!budget.first_visit(bomb));
    }

    /// A corrupt stream fails a strict decode after decoding part of it: each reference pays
    /// for that part, so references to it can't decode without limit.
    #[test]
    fn failed_strict_decodes_charge_what_they_decoded() {
        let whole = Stream::flate(Dict::new(), &vec![b' '; 1 << 20]);
        let cut = whole.raw.len() / 2;
        let corrupt = Stream::from_raw(whole.dict.clone(), whole.raw.get(..cut).unwrap_or_default().to_vec());
        let partial = corrupt.decoded().map(|d| d.len()).unwrap_or(0);
        assert!(partial > 0, "a cut flate stream decodes in part");
        let mut budget = DecodeBudget::new(10 << 20, 1);
        assert!(budget.decode_strict(&corrupt).is_err());
        assert_eq!(budget.remaining(), (10 << 20) - partial);
        let mut budget = DecodeBudget::new(10 << 20, 1);
        assert!(budget.decode_within(&corrupt, 1 << 30).is_ok());
        assert_eq!(budget.remaining(), (10 << 20) - partial);
    }

    /// A stream that fails its reading as zlib, and then as raw deflate after decoding all of it,
    /// gives nothing back, and each reference still pays for what it decoded.
    #[test]
    fn a_decode_that_gives_nothing_back_still_pays() {
        let mut data = vec![0x78, 0x01, 0x06, 0xFE, 0xF9];
        data.extend(vec![0u8; 0x0601]);
        data.push(0x07);
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::name("FlateDecode"));
        let stream = Stream::from_raw(dict, data);
        let mut budget = DecodeBudget::new(10 * 0x0601, 1);
        assert!(budget.decode(&stream).unwrap().is_empty());
        assert_eq!(budget.remaining(), 9 * 0x0601);
    }

    /// A chain whose later stage shrinks the data pays for every stage's output: a Flate stage
    /// that inflates to hex text, which ASCIIHex then halves, charges both.
    #[test]
    fn decodes_charge_every_stage_of_a_chain() {
        let hex: Vec<u8> = std::iter::repeat_n(b"ab".as_slice(), 1 << 19).flatten().copied().collect();
        let flate = Stream::flate(Dict::new(), &hex);
        let mut dict = Dict::new();
        dict.set(b"Filter".to_vec(), Object::Array(vec![Object::name("FlateDecode"), Object::name("ASCIIHexDecode")]));
        let chained = Stream::from_raw(dict, flate.raw.to_vec());
        assert_eq!(chained.decoded().map(|d| d.len()).ok(), Some(1 << 19));
        let charged = (1 << 20) + (1 << 19);
        let mut budget = DecodeBudget::new(10 << 20, 1);
        assert_eq!(budget.decode(&chained).map(|d| d.len()).ok(), Some(1 << 19));
        assert_eq!(budget.remaining(), (10 << 20) - charged);
        assert!(budget.decode_within(&chained, 2 << 20).is_ok());
        assert_eq!(budget.remaining(), (10 << 20) - 2 * charged);
        assert!(budget.decode_strict(&chained).is_ok());
        assert_eq!(budget.remaining(), (10 << 20) - 3 * charged);
    }
}
