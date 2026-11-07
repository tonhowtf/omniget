use std::cmp::Ordering;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::metadata::Orientation;
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use pdfcraft_cos::{Dict, Document, Object, Stream};
use pdfcraft_organize::insert_blank_page;
use serde::Deserialize;

use super::{Control, PdfError};

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "tga", "ico",
];

const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
const A4: (f64, f64) = (595.28, 841.89);
const LETTER: (f64, f64) = (612.0, 792.0);
const MARGIN: f64 = 28.35;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageSize {
    Fit,
    A4,
    Letter,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ImageLayout {
    pub size: PageSize,
    pub margin: bool,
}

struct Picture {
    stream: Stream,
    smask: Option<Stream>,
    width: u32,
    height: u32,
    orientation: Orientation,
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

pub fn expand_images(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(path)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_image(p))
                .collect();
            found.sort_by(|a, b| {
                let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned());
                natural_cmp(&name(a).unwrap_or_default(), &name(b).unwrap_or_default())
            });
            out.extend(found);
        } else if path.is_file() && is_image(path) {
            out.push(path.clone());
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

fn jpeg_layout(bytes: &[u8]) -> Option<(u8, u8)> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if sof {
            let precision = *bytes.get(i + 4)?;
            let components = *bytes.get(i + 9)?;
            return Some((precision, components));
        }
        i += 2 + len;
    }
    None
}

fn image_dict(width: u32, height: u32, gray: bool) -> Dict {
    let mut d = Dict::new();
    d.set(b"Type".to_vec(), Object::name("XObject"));
    d.set(b"Subtype".to_vec(), Object::name("Image"));
    d.set(b"Width".to_vec(), Object::Int(width.into()));
    d.set(b"Height".to_vec(), Object::Int(height.into()));
    d.set(
        b"ColorSpace".to_vec(),
        Object::name(if gray { "DeviceGray" } else { "DeviceRGB" }),
    );
    d.set(b"BitsPerComponent".to_vec(), Object::Int(8));
    d
}

fn load(path: &Path) -> Result<Picture, PdfError> {
    let failed = || PdfError::Image(path.to_string_lossy().into_owned());
    if std::fs::metadata(path)?.len() > MAX_IMAGE_BYTES {
        return Err(failed());
    }
    let bytes = std::fs::read(path)?;
    let reader = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| failed())?;
    let format = reader.format();
    let mut decoder = reader.into_decoder().map_err(|_| failed())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (width, height) = decoder.dimensions();
    let color = decoder.color_type();
    if format == Some(ImageFormat::Jpeg)
        && matches!(jpeg_layout(&bytes), Some((8, 1 | 3)))
        && matches!(color, ColorType::L8 | ColorType::Rgb8)
    {
        drop(decoder);
        let mut dict = image_dict(width, height, color == ColorType::L8);
        dict.set(b"Filter".to_vec(), Object::name("DCTDecode"));
        return Ok(Picture {
            stream: Stream::from_raw(dict, bytes),
            smask: None,
            width,
            height,
            orientation,
        });
    }
    let img = DynamicImage::from_decoder(decoder).map_err(|_| failed())?;
    let gray = !img.color().has_color();
    let smask = img
        .color()
        .has_alpha()
        .then(|| img.to_luma_alpha8())
        .and_then(|la| {
            let alpha: Vec<u8> = la.pixels().map(|p| p.0[1]).collect();
            alpha
                .iter()
                .any(|a| *a != 255)
                .then(|| Stream::flate(image_dict(width, height, true), &alpha))
        });
    let pixels = if gray {
        img.to_luma8().into_raw()
    } else {
        img.to_rgb8().into_raw()
    };
    Ok(Picture {
        stream: Stream::flate(image_dict(width, height, gray), &pixels),
        smask,
        width,
        height,
        orientation,
    })
}

fn turned(o: Orientation) -> bool {
    matches!(
        o,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

pub(crate) fn placement(o: Orientation, x: f64, y: f64, w: f64, h: f64) -> [f64; 6] {
    match o {
        Orientation::NoTransforms => [w, 0.0, 0.0, h, x, y],
        Orientation::Rotate180 => [-w, 0.0, 0.0, -h, x + w, y + h],
        Orientation::Rotate90 => [0.0, -h, w, 0.0, x, y + h],
        Orientation::Rotate270 => [0.0, h, -w, 0.0, x + w, y],
        Orientation::FlipHorizontal => [-w, 0.0, 0.0, h, x + w, y],
        Orientation::FlipVertical => [w, 0.0, 0.0, -h, x, y + h],
        Orientation::Rotate90FlipH => [0.0, -h, -w, 0.0, x + w, y + h],
        Orientation::Rotate270FlipH => [0.0, h, w, 0.0, x, y],
    }
}

fn page_and_box(layout: ImageLayout, shown: (f64, f64)) -> ((f64, f64), [f64; 4]) {
    let (sw, sh) = shown;
    let page = match layout.size {
        PageSize::Fit => {
            let k = A4.1 / sw.max(sh);
            (sw * k, sh * k)
        }
        PageSize::A4 | PageSize::Letter => {
            let (a, b) = if layout.size == PageSize::A4 {
                A4
            } else {
                LETTER
            };
            if sw > sh {
                (b, a)
            } else {
                (a, b)
            }
        }
    };
    let margin = if layout.margin { MARGIN } else { 0.0 };
    let (aw, ah) = (page.0 - 2.0 * margin, page.1 - 2.0 * margin);
    let k = (aw / sw).min(ah / sh);
    let (w, h) = (sw * k, sh * k);
    (page, [(page.0 - w) / 2.0, (page.1 - h) / 2.0, w, h])
}

fn number(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

pub fn images_to_pdf(
    paths: &[PathBuf],
    layout: ImageLayout,
    control: &Control<'_>,
) -> Result<Document, PdfError> {
    if paths.is_empty() {
        return Err(PdfError::Empty);
    }
    let mut doc = Document::new_empty();
    for (i, path) in paths.iter().enumerate() {
        control.check()?;
        let mut picture = load(path)?;
        if let Some(mask) = picture.smask.take() {
            let mask = doc.add(Object::Stream(mask));
            picture
                .stream
                .dict
                .set(b"SMask".to_vec(), Object::Ref(mask));
        }
        let (iw, ih) = (f64::from(picture.width), f64::from(picture.height));
        let shown = if turned(picture.orientation) {
            (ih, iw)
        } else {
            (iw, ih)
        };
        let (page_size, [x, y, w, h]) = page_and_box(layout, shown);
        let m = placement(picture.orientation, x, y, w, h);
        let content = format!(
            "q {} cm /Im0 Do Q",
            m.iter().map(|v| number(*v)).collect::<Vec<_>>().join(" ")
        );
        let image = doc.add(Object::Stream(picture.stream));
        let contents = doc.add(Object::Stream(Stream::flate(
            Dict::new(),
            content.as_bytes(),
        )));
        let page = insert_blank_page(&mut doc, i, page_size.0, page_size.1)?;
        let mut xobjects = Dict::new();
        xobjects.set(b"Im0".to_vec(), Object::Ref(image));
        let mut resources = Dict::new();
        resources.set(b"XObject".to_vec(), Object::Dict(xobjects));
        doc.update_dict(page, |d| {
            d.set(b"Resources".to_vec(), Object::Dict(resources));
            d.set(b"Contents".to_vec(), Object::Ref(contents));
        })?;
        (control.progress)(i + 1, paths.len());
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn control(cancel: &AtomicBool) -> Control<'_> {
        Control {
            cancel,
            progress: &|_, _| {},
        }
    }

    fn sample(w: u32, h: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| Rgb([(x * 40) as u8, (y * 40) as u8, 7]))
    }

    #[test]
    fn placement_matches_how_the_image_crate_orients_pixels() {
        let all = [
            Orientation::NoTransforms,
            Orientation::Rotate90,
            Orientation::Rotate180,
            Orientation::Rotate270,
            Orientation::FlipHorizontal,
            Orientation::FlipVertical,
            Orientation::Rotate90FlipH,
            Orientation::Rotate270FlipH,
        ];
        let src = sample(3, 2);
        for o in all {
            let mut shown = DynamicImage::ImageRgb8(src.clone());
            shown.apply_orientation(o);
            let shown = shown.to_rgb8();
            let (sw, sh) = (f64::from(shown.width()), f64::from(shown.height()));
            let [a, b, c, d, e, f] = placement(o, 0.0, 0.0, sw, sh);
            for (col, row, px) in src.enumerate_pixels() {
                let u = (f64::from(col) + 0.5) / 3.0;
                let v = 1.0 - (f64::from(row) + 0.5) / 2.0;
                let (x, y) = (a * u + c * v + e, b * u + d * v + f);
                let (sx, sy) = (x.floor() as u32, (sh - y).floor() as u32);
                assert_eq!(shown.get_pixel(sx, sy), px, "{o:?} at {col},{row}");
            }
        }
    }

    #[test]
    fn jpeg_layout_reads_precision_and_components() {
        let mut jpeg = Vec::new();
        DynamicImage::ImageRgb8(sample(4, 4))
            .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(jpeg_layout(&jpeg), Some((8, 3)));
        assert_eq!(jpeg_layout(b"not a jpeg"), None);
    }

    #[test]
    fn natural_order_puts_page_2_before_page_10() {
        let mut names = vec!["scan 10.png", "Scan 2.png", "scan 1.png", "cover.jpg"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            vec!["cover.jpg", "scan 1.png", "Scan 2.png", "scan 10.png"]
        );
    }

    #[test]
    fn folders_expand_to_their_images_in_natural_order() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["p10.png", "p2.png", "notes.txt", "p1.jpg"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let loose = dir.path().join("p2.png");
        let got = expand_images(&[dir.path().to_path_buf(), loose]);
        let names: Vec<String> = got
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["p1.jpg", "p2.png", "p10.png"]);
    }

    #[test]
    fn builds_one_page_per_image_keeping_jpegs_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let jpg = dir.path().join("photo.jpg");
        DynamicImage::ImageRgb8(sample(60, 30)).save(&jpg).unwrap();
        let png = dir.path().join("logo.png");
        RgbaImage::from_fn(20, 40, |x, _| {
            Rgba([200, 10, 10, if x < 10 { 0 } else { 255 }])
        })
        .save(&png)
        .unwrap();
        let cancel = AtomicBool::new(false);
        let layout = ImageLayout {
            size: PageSize::A4,
            margin: true,
        };
        let doc = images_to_pdf(&[jpg.clone(), png], layout, &control(&cancel)).unwrap();
        let bytes = super::super::files::to_bytes(&doc).unwrap();
        let jpeg_bytes = std::fs::read(&jpg).unwrap();
        assert!(bytes
            .windows(jpeg_bytes.len())
            .any(|w| w == jpeg_bytes.as_slice()));
        let doc = Document::open(Arc::new(bytes)).unwrap();
        let boxes = pdfcraft_organize::page_boxes(&doc).unwrap();
        assert_eq!(boxes.len(), 2);
        assert!(
            (boxes[0][0][2] - A4.1).abs() < 0.01,
            "landscape photo on a landscape page"
        );
        assert!(
            (boxes[1][0][3] - A4.1).abs() < 0.01,
            "portrait logo on a portrait page"
        );
    }

    #[test]
    fn fit_pages_follow_the_image_shape() {
        let ((pw, ph), [x, y, w, h]) = page_and_box(
            ImageLayout {
                size: PageSize::Fit,
                margin: false,
            },
            (1000.0, 500.0),
        );
        assert!((pw - A4.1).abs() < 1e-6 && (ph - A4.1 / 2.0).abs() < 1e-6);
        assert_eq!([x, y], [0.0, 0.0]);
        assert!((w - pw).abs() < 1e-6 && (h - ph).abs() < 1e-6);
    }

    #[test]
    fn unreadable_images_and_cancel_stop_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("broken.png");
        std::fs::write(&bad, b"not an image").unwrap();
        let cancel = AtomicBool::new(false);
        let layout = ImageLayout {
            size: PageSize::Fit,
            margin: false,
        };
        assert!(matches!(
            images_to_pdf(std::slice::from_ref(&bad), layout, &control(&cancel)),
            Err(PdfError::Image(_))
        ));
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            images_to_pdf(&[bad], layout, &control(&cancel)),
            Err(PdfError::Cancelled)
        ));
    }
}
