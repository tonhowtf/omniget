use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, GrayImage, ImageFormat, RgbImage};
use pdfcraft_cos::{Document, ObjRef, Object, Stream};
use pdfcraft_filters::Filter;
use serde::{Deserialize, Serialize};

use super::{Control, PdfError};

const MAX_PIXELS: u64 = 60_000_000;
const MAX_WORKERS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Light,
    Balanced,
    Strong,
}

impl Preset {
    fn max_side(self) -> u32 {
        match self {
            Preset::Light => 2400,
            Preset::Balanced => 1600,
            Preset::Strong => 1100,
        }
    }

    fn quality(self) -> u8 {
        match self {
            Preset::Light => 80,
            Preset::Balanced => 68,
            Preset::Strong => 55,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CompressStats {
    pub images: usize,
    pub streams: usize,
}

fn components(doc: &Document, cs: Option<&Object>) -> Option<u8> {
    let cs = doc.resolve(cs?);
    match &*cs {
        Object::Name(n) if n == b"DeviceRGB" => Some(3),
        Object::Name(n) if n == b"DeviceGray" => Some(1),
        Object::Array(a) if a.first().and_then(Object::as_name) == Some(b"ICCBased") => {
            match &*doc.resolve(a.get(1)?) {
                Object::Stream(s) => match s.dict.int(b"N") {
                    Some(n @ (1 | 3)) => Some(n as u8),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

fn pixels(s: &Stream, width: u32, height: u32, comps: u8) -> Option<DynamicImage> {
    let filters = s.filters();
    if let [(Filter::Dct, _)] = filters.as_slice() {
        let img = image::load_from_memory_with_format(&s.raw, ImageFormat::Jpeg).ok()?;
        return match (comps, &img) {
            (1, DynamicImage::ImageLuma8(_)) | (3, DynamicImage::ImageRgb8(_))
                if img.width() == width && img.height() == height =>
            {
                Some(img)
            }
            _ => None,
        };
    }
    let lossless = filters.iter().all(|(f, _)| {
        matches!(
            f,
            Filter::Flate | Filter::Lzw | Filter::RunLength | Filter::Ascii85 | Filter::AsciiHex
        )
    });
    if !lossless {
        return None;
    }
    let expected = u64::from(width) * u64::from(height) * u64::from(comps);
    let data = s.decoded_within(usize::try_from(expected).ok()?).ok()?;
    if data.len() as u64 != expected {
        return None;
    }
    if comps == 1 {
        GrayImage::from_raw(width, height, data).map(DynamicImage::ImageLuma8)
    } else {
        RgbImage::from_raw(width, height, data).map(DynamicImage::ImageRgb8)
    }
}

fn recompress(doc: &Document, s: &Stream, preset: Preset) -> Option<Stream> {
    let d = &s.dict;
    let masked = matches!(d.get(b"ImageMask"), Some(Object::Bool(true)))
        || matches!(d.get(b"Mask"), Some(Object::Array(_)))
        || d.contains(b"Decode");
    if masked || d.int(b"BitsPerComponent") != Some(8) {
        return None;
    }
    let width = u32::try_from(d.int(b"Width")?).ok()?;
    let height = u32::try_from(d.int(b"Height")?).ok()?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }
    let comps = components(doc, d.get(b"ColorSpace"))?;
    let img = pixels(s, width, height, comps)?;
    let side = preset.max_side();
    let img = if width.max(height) > side {
        img.resize(side, side, FilterType::Triangle)
    } else {
        img
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, preset.quality())
        .encode_image(&img)
        .ok()?;
    if out.len() as f64 > s.raw.len() as f64 * 0.9 {
        return None;
    }
    let mut dict = d.clone();
    dict.set(b"Filter".to_vec(), Object::name("DCTDecode"));
    dict.remove(b"DecodeParms");
    dict.set(b"Width".to_vec(), Object::Int(img.width().into()));
    dict.set(b"Height".to_vec(), Object::Int(img.height().into()));
    Some(Stream::from_raw(dict, out))
}

pub fn compress(
    doc: &mut Document,
    preset: Preset,
    control: &Control<'_>,
) -> Result<CompressStats, PdfError> {
    let refs: Vec<ObjRef> = doc
        .object_numbers()
        .into_iter()
        .map(|n| ObjRef::new(n, doc.generation(n)))
        .collect();
    let mut soft_masks = HashSet::new();
    let mut images = Vec::new();
    let mut stats = CompressStats::default();
    for r in refs {
        let obj = doc.get(r);
        let Object::Stream(s) = &*obj else { continue };
        if let Some(mask) = s.dict.reference(b"SMask") {
            soft_masks.insert(mask);
        }
        if s.dict.name(b"Subtype") == Some(b"Image") {
            images.push((r, obj.clone()));
        } else if !s.dict.contains(b"Filter")
            && s.dict.name(b"Type") != Some(b"Metadata")
            && s.raw.len() > 64
        {
            let packed = Stream::flate(s.dict.clone(), &s.raw);
            doc.set(r, Object::Stream(packed));
            stats.streams += 1;
        }
    }
    images.retain(|(r, _)| !soft_masks.contains(r));
    let total = images.len();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<(ObjRef, Stream)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, MAX_WORKERS);
    let shared: &Document = doc;
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if control.check().is_err() {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((r, obj)) = images.get(i) else { break };
                if let Object::Stream(s) = &**obj {
                    if let Some(packed) = recompress(shared, s, preset) {
                        if let Ok(mut list) = results.lock() {
                            list.push((*r, packed));
                        }
                    }
                }
                (control.progress)(done.fetch_add(1, Ordering::Relaxed) + 1, total);
            });
        }
    });
    control.check()?;
    let results = results.into_inner().unwrap_or_default();
    stats.images = results.len();
    for (r, s) in results {
        doc.set(r, Object::Stream(s));
    }
    Ok(stats)
}

pub fn compress_bytes(
    bytes: Vec<u8>,
    preset: Preset,
    control: &Control<'_>,
) -> Result<(Vec<u8>, CompressStats), PdfError> {
    let mut doc = Document::open(Arc::new(bytes))?;
    let stats = compress(&mut doc, preset, control)?;
    Ok((super::files::to_bytes(&doc)?, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_cos::Dict;
    use std::sync::atomic::AtomicBool;

    fn noisy(width: u32, height: u32) -> Vec<u8> {
        let mut seed = 0x2545_f491_u32;
        (0..width * height * 3)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                ((i / 3 % width) as u8).wrapping_add((seed % 24) as u8)
            })
            .collect()
    }

    fn photo_pdf(width: u32, height: u32) -> Vec<u8> {
        let mut doc = Document::new_empty();
        let mut dict = Dict::new();
        dict.set(b"Type".to_vec(), Object::name("XObject"));
        dict.set(b"Subtype".to_vec(), Object::name("Image"));
        dict.set(b"Width".to_vec(), Object::Int(width.into()));
        dict.set(b"Height".to_vec(), Object::Int(height.into()));
        dict.set(b"ColorSpace".to_vec(), Object::name("DeviceRGB"));
        dict.set(b"BitsPerComponent".to_vec(), Object::Int(8));
        let image = doc.add(Object::Stream(Stream::flate(dict, &noisy(width, height))));
        let page = pdfcraft_organize::insert_blank_page(&mut doc, 0, 600.0, 400.0).unwrap();
        let content = doc.add(Object::Stream(Stream::from_raw(
            Dict::new(),
            b"q 600 0 0 400 0 0 cm /Im0 Do Q\n".repeat(4),
        )));
        let mut xobjects = Dict::new();
        xobjects.set(b"Im0".to_vec(), Object::Ref(image));
        let mut resources = Dict::new();
        resources.set(b"XObject".to_vec(), Object::Dict(xobjects));
        doc.update_dict(page, |d| {
            d.set(b"Resources".to_vec(), Object::Dict(resources));
            d.set(b"Contents".to_vec(), Object::Ref(content));
        })
        .unwrap();
        super::super::files::to_bytes(&doc).unwrap()
    }

    fn image_sizes(bytes: &[u8]) -> Vec<(i64, i64, Vec<u8>)> {
        let doc = Document::open(Arc::new(bytes.to_vec())).unwrap();
        doc.object_numbers()
            .into_iter()
            .filter_map(|n| {
                let obj = doc.get(ObjRef::new(n, doc.generation(n)));
                let Object::Stream(s) = &*obj else {
                    return None;
                };
                (s.dict.name(b"Subtype") == Some(b"Image")).then(|| {
                    (
                        s.dict.int(b"Width").unwrap(),
                        s.dict.int(b"Height").unwrap(),
                        s.dict.name(b"Filter").unwrap_or_default().to_vec(),
                    )
                })
            })
            .collect()
    }

    #[test]
    fn shrinks_large_images_into_a_smaller_valid_pdf() {
        let input = photo_pdf(3000, 2000);
        let cancel = AtomicBool::new(false);
        let seen = Mutex::new(Vec::new());
        let progress = |done: usize, total: usize| seen.lock().unwrap().push((done, total));
        let control = Control {
            cancel: &cancel,
            progress: &progress,
        };
        let (out, stats) = compress_bytes(input.clone(), Preset::Balanced, &control).unwrap();
        assert!(
            out.len() * 2 < input.len(),
            "{} -> {}",
            input.len(),
            out.len()
        );
        assert_eq!(stats.images, 1);
        assert_eq!(stats.streams, 1);
        assert_eq!(seen.into_inner().unwrap(), vec![(1, 1)]);
        assert_eq!(image_sizes(&out), vec![(1600, 1067, b"DCTDecode".to_vec())]);
        let doc = Document::open(Arc::new(out)).unwrap();
        assert_eq!(pdfcraft_organize::page_count(&doc).unwrap(), 1);
    }

    #[test]
    fn leaves_images_alone_when_recompressing_does_not_pay() {
        let mut doc = Document::new_empty();
        let mut dict = Dict::new();
        dict.set(b"Subtype".to_vec(), Object::name("Image"));
        dict.set(b"Width".to_vec(), Object::Int(64));
        dict.set(b"Height".to_vec(), Object::Int(64));
        dict.set(b"ColorSpace".to_vec(), Object::name("DeviceGray"));
        dict.set(b"BitsPerComponent".to_vec(), Object::Int(8));
        let flat = doc.add(Object::Stream(Stream::flate(
            dict.clone(),
            &[128u8; 64 * 64],
        )));
        dict.set(b"BitsPerComponent".to_vec(), Object::Int(1));
        let mono = doc.add(Object::Stream(Stream::flate(dict, &[0u8; 64 * 8])));
        let cancel = AtomicBool::new(false);
        let control = Control {
            cancel: &cancel,
            progress: &|_, _| {},
        };
        let stats = compress(&mut doc, Preset::Strong, &control).unwrap();
        assert_eq!(stats.images, 0);
        for r in [flat, mono] {
            let obj = doc.get(r);
            let Object::Stream(s) = &*obj else { panic!() };
            assert_eq!(s.dict.name(b"Filter"), Some(&b"FlateDecode"[..]));
        }
    }

    #[test]
    fn packs_plain_streams_but_keeps_xmp_metadata_readable() {
        let mut doc = Document::new_empty();
        let body = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"></x:xmpmeta>".repeat(8);
        let mut meta = Dict::new();
        meta.set(b"Type".to_vec(), Object::name("Metadata"));
        meta.set(b"Subtype".to_vec(), Object::name("XML"));
        let meta = doc.add(Object::Stream(Stream::from_raw(meta, body.clone())));
        let plain = doc.add(Object::Stream(Stream::from_raw(Dict::new(), body.clone())));
        let cancel = AtomicBool::new(false);
        let control = Control {
            cancel: &cancel,
            progress: &|_, _| {},
        };
        let stats = compress(&mut doc, Preset::Light, &control).unwrap();
        assert_eq!(stats.streams, 1);
        let obj = doc.get(meta);
        let Object::Stream(s) = &*obj else { panic!() };
        assert!(!s.dict.contains(b"Filter"));
        assert_eq!(&s.raw[..], &body[..]);
        let obj = doc.get(plain);
        let Object::Stream(s) = &*obj else { panic!() };
        assert_eq!(s.decoded().unwrap(), body);
    }

    #[test]
    fn cancel_stops_before_writing() {
        let cancel = AtomicBool::new(true);
        let control = Control {
            cancel: &cancel,
            progress: &|_, _| {},
        };
        assert!(matches!(
            compress_bytes(photo_pdf(200, 100), Preset::Light, &control),
            Err(PdfError::Cancelled)
        ));
    }

    #[test]
    fn presets_get_stronger() {
        let all = [Preset::Light, Preset::Balanced, Preset::Strong];
        assert!(all.windows(2).all(|p| p[0].max_side() > p[1].max_side()));
        assert!(all.windows(2).all(|p| p[0].quality() > p[1].quality()));
    }
}
