use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use ::image::codecs::jpeg::JpegEncoder;
use ::image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use tokio_util::sync::CancellationToken;

use super::files::{self, RemoveMode};
use super::library::{Hashed, Library};
use super::similar::{self, DupKind};
use super::*;

fn gradient(w: u32, h: u32, seed: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        let v = ((x * 255 / w.max(1)) + seed * 37) % 256;
        let u = ((y * 255 / h.max(1)) + seed * 91) % 256;
        Rgb([v as u8, u as u8, ((x ^ y) % 256) as u8])
    })
}

fn blocks(seed: u64) -> RgbImage {
    RgbImage::from_fn(240, 160, |x, y| {
        let cell = u64::from(x / 30) * 31 + u64::from(y / 20) * 7 + 1;
        let v = (cell.wrapping_mul(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as u8;
        Rgb([v, v, v])
    })
}

fn save_png(path: &Path, img: &RgbImage) {
    img.save_with_format(path, ImageFormat::Png).unwrap();
}

fn jpeg_bytes(img: &RgbImage, q: u8) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, q)
        .encode_image(img)
        .unwrap();
    out.into_inner()
}

fn jpeg_with_exif(img: &RgbImage, date: &str, orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II*\0");
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&2u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&u32::from(orientation).to_le_bytes());
    tiff.extend_from_slice(&0x8769u16.to_le_bytes());
    tiff.extend_from_slice(&4u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&38u32.to_le_bytes());
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&0x9003u16.to_le_bytes());
    tiff.extend_from_slice(&2u16.to_le_bytes());
    tiff.extend_from_slice(&20u32.to_le_bytes());
    tiff.extend_from_slice(&56u32.to_le_bytes());
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff.extend_from_slice(date.as_bytes());
    tiff.push(0);
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let jpg = jpeg_bytes(img, 90);
    let mut out = jpg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpg[2..]);
    out
}

fn export_one(
    input: &Path,
    format: &str,
    edits: Option<&Edits>,
    options: &ExportOptions,
    taken: &mut HashSet<PathBuf>,
) -> ExportOutcome {
    match export::render_safely(input, format, edits, options) {
        Ok(bytes) => export::save(input, &bytes, options, taken),
        Err(e) => ExportOutcome::failed(&input.to_string_lossy(), e),
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    lib: Library,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Pictures");
    std::fs::create_dir_all(root.join("Trip")).unwrap();
    let lib = Library::new(dir.path().join("data"));
    Fixture {
        root,
        lib,
        _dir: dir,
    }
}

fn index(f: &Fixture) -> Progress {
    index::run(
        &f.lib,
        std::slice::from_ref(&f.root),
        &CancellationToken::new(),
        &|_| {},
    )
}

#[test]
fn exif_date_and_orientation_are_read() {
    let f = fixture();
    let path = f.root.join("a.jpg");
    std::fs::write(
        &path,
        jpeg_with_exif(&gradient(40, 20, 1), "2024:07:15 18:30:05", 6),
    )
    .unwrap();
    let exif = meta::read_exif(&path);
    assert_eq!(exif.orientation, Some(6));
    let expected = chrono::NaiveDate::from_ymd_opt(2024, 7, 15)
        .unwrap()
        .and_hms_opt(18, 30, 5)
        .unwrap()
        .and_utc()
        .timestamp();
    assert_eq!(exif.taken_at, Some(expected));
    assert_eq!(meta::parse_datetime(b"0000:00:00 00:00:00"), None);
    assert_eq!(meta::parse_datetime(b"garbage"), None);
}

#[test]
fn indexing_builds_thumbnails_and_skips_unchanged_files() {
    let f = fixture();
    save_png(&f.root.join("one.png"), &gradient(800, 600, 1));
    std::fs::write(
        f.root.join("Trip/two.jpg"),
        jpeg_bytes(&gradient(300, 900, 2), 85),
    )
    .unwrap();
    std::fs::write(f.root.join("Trip/broken.jpg"), b"not a jpeg").unwrap();
    std::fs::write(f.root.join("notes.txt"), b"hello").unwrap();
    std::fs::create_dir_all(f.root.join(".hidden")).unwrap();
    save_png(&f.root.join(".hidden/secret.png"), &gradient(10, 10, 3));

    let first = index(&f);
    assert_eq!(first.stage, Stage::Done);
    assert_eq!(
        (first.found, first.total, first.done, first.failed),
        (3, 3, 3, 1)
    );

    let photos = f.lib.list().unwrap();
    assert_eq!(photos.len(), 3);
    let one = photos.iter().find(|p| p.name == "one.png").unwrap();
    assert_eq!(
        (one.width, one.height, one.format.as_str()),
        (800, 600, "png")
    );
    let thumb = image::open(one.thumb.as_ref().unwrap()).unwrap();
    assert_eq!(thumb.width().max(thumb.height()), thumb::THUMB_SIZE);
    let broken = photos.iter().find(|p| p.name == "broken.jpg").unwrap();
    assert!(broken.error.is_some() && broken.thumb.is_none());

    let second = index(&f);
    assert_eq!((second.found, second.total), (3, 0));

    std::fs::remove_file(f.root.join("one.png")).unwrap();
    let third = index(&f);
    assert_eq!(third.found, 2);
    assert_eq!(f.lib.list().unwrap().len(), 2);
    assert!(!Path::new(one.thumb.as_ref().unwrap()).exists());
}

#[test]
fn cancelled_index_stops_without_forgetting() {
    let f = fixture();
    save_png(&f.root.join("one.png"), &gradient(50, 50, 1));
    assert_eq!(index(&f).stage, Stage::Done);
    let token = CancellationToken::new();
    token.cancel();
    let p = index::run(&f.lib, std::slice::from_ref(&f.root), &token, &|_| {});
    assert_eq!(p.stage, Stage::Cancelled);
    assert_eq!(f.lib.list().unwrap().len(), 1);
}

#[test]
fn offline_folder_keeps_its_photos_and_edits() {
    let f = fixture();
    let drive = f.root.parent().unwrap().join("Drive");
    std::fs::create_dir_all(&drive).unwrap();
    save_png(&drive.join("far.png"), &gradient(50, 50, 2));
    save_png(&f.root.join("near.png"), &gradient(50, 50, 3));
    let roots = vec![f.root.clone(), drive.clone()];
    let run = |roots: &[PathBuf]| index::run(&f.lib, roots, &CancellationToken::new(), &|_| {});
    assert_eq!(run(&roots).stage, Stage::Done);
    let far = drive.join("far.png").to_string_lossy().into_owned();
    let edits = Edits {
        exposure: 20,
        ..Default::default()
    };
    f.lib.set_edits(&far, Some(&edits)).unwrap();

    std::fs::rename(&drive, drive.with_file_name("Unplugged")).unwrap();
    std::fs::remove_file(f.root.join("near.png")).unwrap();
    assert_eq!(run(&roots).stage, Stage::Done);
    let left: Vec<String> = f.lib.list().unwrap().into_iter().map(|p| p.path).collect();
    assert_eq!(left, vec![far.clone()]);
    assert_eq!(f.lib.edits(&far).unwrap(), Some(edits));

    assert_eq!(run(std::slice::from_ref(&f.root)).stage, Stage::Done);
    assert!(f.lib.list().unwrap().is_empty());
}

#[test]
fn folders_start_unset_then_persist_in_order() {
    let f = fixture();
    assert_eq!(f.lib.folders().unwrap(), None);
    let chosen = vec![PathBuf::from("/b"), PathBuf::from("/a")];
    f.lib.set_folders(&chosen).unwrap();
    assert_eq!(f.lib.folders().unwrap(), Some(chosen));
    f.lib.set_folders(&[]).unwrap();
    assert_eq!(f.lib.folders().unwrap(), Some(vec![]));
}

#[test]
fn dhash_is_stable_under_resize_and_recompression() {
    let base = DynamicImage::ImageRgb8(blocks(7));
    let small = DynamicImage::ImageRgb8(
        base.resize(120, 80, image::imageops::FilterType::Triangle)
            .to_rgb8(),
    );
    let recompressed = image::load_from_memory(&jpeg_bytes(&base.to_rgb8(), 40)).unwrap();
    let other = DynamicImage::ImageRgb8(blocks(99));
    let h = |i: &DynamicImage| thumb::dhash(&i.to_luma8());
    assert!((h(&base) ^ h(&small)).count_ones() <= 4);
    assert!((h(&base) ^ h(&recompressed)).count_ones() <= 4);
    assert!((h(&base) ^ h(&other)).count_ones() > 12);
}

fn hashed(path: &str, size: u64, w: u32, dhash: u64, content: Option<&str>) -> Hashed {
    Hashed {
        path: path.to_string(),
        size,
        width: w,
        height: w,
        taken_at: None,
        modified_at: 0,
        dhash: Some(dhash),
        content: content.map(str::to_string),
    }
}

#[test]
fn groups_exact_and_similar_without_chaining() {
    let a: u64 = 0x0F0F_F0F0_3C3C_A5A5;
    let near = a ^ 0b111;
    let chain = a ^ (0b1111_1111 << 20);
    let items = vec![
        hashed("/p/copy.jpg", 100, 100, a, Some("x")),
        hashed("/p/orig.jpg", 100, 100, a, Some("x")),
        hashed("/p/big.jpg", 500, 400, near, None),
        hashed("/p/far.jpg", 90, 100, chain, None),
        hashed("/p/flat1.jpg", 10, 50, 0, None),
        hashed("/p/flat2.jpg", 11, 50, 0, None),
    ];
    let exact_only = similar::group(&items, 0);
    assert_eq!(exact_only.len(), 1);
    assert_eq!(exact_only[0].kind, DupKind::Exact);
    assert_eq!(exact_only[0].paths.len(), 2);
    assert_eq!(exact_only[0].bytes, 100);

    let groups = similar::group(&items, 8);
    assert_eq!(groups.len(), 1);
    let g = &groups[0];
    assert_eq!(g.kind, DupKind::Similar);
    assert_eq!(g.keep, "/p/big.jpg");
    let mut paths = g.paths.clone();
    paths.sort();
    assert_eq!(paths, vec!["/p/big.jpg", "/p/copy.jpg", "/p/orig.jpg"]);
}

#[test]
fn content_is_hashed_only_for_same_size_files() {
    let f = fixture();
    let bytes = jpeg_bytes(&gradient(64, 64, 4), 80);
    std::fs::write(f.root.join("a.jpg"), &bytes).unwrap();
    std::fs::write(f.root.join("Trip/b.jpg"), &bytes).unwrap();
    save_png(&f.root.join("c.png"), &gradient(64, 64, 9));
    index(&f);
    let mut items = f.lib.hashed().unwrap();
    similar::fill_content(&f.lib, &mut items).unwrap();
    let with_content = items.iter().filter(|h| h.content.is_some()).count();
    assert_eq!(with_content, 2);
    let stored = f.lib.hashed().unwrap();
    assert_eq!(stored.iter().filter(|h| h.content.is_some()).count(), 2);
    let groups = similar::group(&items, 0);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].kind, DupKind::Exact);
}

#[test]
fn edits_rotate_crop_and_tone() {
    let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(40, 20, Rgb([100, 100, 100])));
    let rotated = adjust::apply(
        img.clone(),
        &Edits {
            rotate: 90,
            ..Default::default()
        },
    );
    assert_eq!((rotated.width(), rotated.height()), (20, 40));

    let cropped = adjust::apply(
        img.clone(),
        &Edits {
            crop: Some(Crop {
                x: 0.25,
                y: 0.5,
                w: 0.5,
                h: 0.5,
            }),
            ..Default::default()
        },
    );
    assert_eq!((cropped.width(), cropped.height()), (20, 10));

    let px = |e: Edits| adjust::apply(img.clone(), &e).to_rgb8().get_pixel(0, 0).0;
    let brighter = px(Edits {
        exposure: 50,
        ..Default::default()
    });
    assert!(brighter[0] > 120);
    let warm = px(Edits {
        temperature: 100,
        ..Default::default()
    });
    assert!(warm[0] > 100 && warm[2] < 100);
    assert_eq!(px(Edits::default()), [100, 100, 100]);

    let colorful = DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, Rgb([180, 90, 60])));
    let gray = adjust::apply(
        colorful,
        &Edits {
            saturation: -100,
            ..Default::default()
        },
    )
    .to_rgb8()
    .get_pixel(0, 0)
    .0;
    assert!(gray[0] == gray[1] && gray[1] == gray[2]);

    let dark = px(Edits {
        contrast: 80,
        ..Default::default()
    });
    assert!(dark[0] < 100);
}

#[test]
fn edits_normalize_and_keep_alpha() {
    let e = Edits {
        exposure: 400,
        rotate: 450,
        crop: Some(Crop {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }),
        ..Default::default()
    }
    .normalized();
    assert_eq!((e.exposure, e.rotate, e.crop), (100, 90, None));
    assert!(Edits {
        crop: Some(Crop {
            x: 0.9,
            y: 0.9,
            w: 0.001,
            h: 0.5
        }),
        ..Default::default()
    }
    .is_identity());

    let rgba = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([50, 60, 70, 33])));
    let out = adjust::apply(
        rgba,
        &Edits {
            exposure: 30,
            ..Default::default()
        },
    );
    assert_eq!(out.to_rgba8().get_pixel(1, 1)[3], 33);
}

#[test]
fn edits_are_stored_and_cleared() {
    let f = fixture();
    let path = f.root.join("one.png");
    save_png(&path, &gradient(30, 30, 1));
    index(&f);
    let key = path.to_string_lossy().into_owned();
    let e = Edits {
        contrast: 20,
        ..Default::default()
    };
    f.lib.set_edits(&key, Some(&e)).unwrap();
    assert_eq!(f.lib.edits(&key).unwrap(), Some(e));
    assert!(f.lib.list().unwrap()[0].edited);
    f.lib.set_edits(&key, Some(&Edits::default())).unwrap();
    assert_eq!(f.lib.edits(&key).unwrap(), None);
}

#[test]
fn export_applies_edits_size_and_never_overwrites() {
    let f = fixture();
    let path = f.root.join("one.jpg");
    std::fs::write(&path, jpeg_bytes(&gradient(400, 200, 1), 90)).unwrap();
    let out = f.root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let options = ExportOptions {
        format: "jpeg".into(),
        quality: Some(70),
        max_size: Some(100),
        out_dir: Some(out.to_string_lossy().into_owned()),
        apply_edits: true,
    };
    export::validate(&options).unwrap();
    let edits = Edits {
        rotate: 90,
        ..Default::default()
    };
    let mut taken = HashSet::new();
    let a = export_one(&path, "jpeg", Some(&edits), &options, &mut taken);
    let b = export_one(&path, "jpeg", Some(&edits), &options, &mut taken);
    assert!(a.ok && b.ok, "{a:?} {b:?}");
    assert_eq!(
        a.output.as_deref(),
        Some(out.join("one.jpg").to_string_lossy().as_ref())
    );
    assert_eq!(
        b.output.as_deref(),
        Some(out.join("one (1).jpg").to_string_lossy().as_ref())
    );
    let img = image::open(out.join("one.jpg")).unwrap();
    assert_eq!((img.width(), img.height()), (50, 100));

    let same_dir = ExportOptions {
        out_dir: None,
        format: "png".into(),
        ..options.clone()
    };
    let c = export_one(&path, "jpeg", None, &same_dir, &mut taken);
    assert_eq!(
        c.output.as_deref(),
        Some(f.root.join("one.png").to_string_lossy().as_ref())
    );

    assert!(export::validate(&ExportOptions {
        format: "svg".into(),
        ..options.clone()
    })
    .is_err());
    let broken = f.root.join("broken.jpg");
    std::fs::write(&broken, b"nope").unwrap();
    let d = export_one(&broken, "jpeg", None, &options, &mut taken);
    assert!(!d.ok && d.error.is_some());
    assert!(!out.join("broken.jpg").exists());
}

#[test]
fn preview_rotates_and_reports_size() {
    let f = fixture();
    let path = f.root.join("wide.png");
    save_png(&path, &gradient(2400, 1200, 1));
    let p = preview::render(
        &path,
        "png",
        &Edits {
            rotate: 270,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(p.src.starts_with("data:image/jpeg;base64,"));
    assert_eq!(
        (p.width, p.height),
        (preview::PREVIEW_SIZE / 2, preview::PREVIEW_SIZE)
    );
    assert_eq!((p.full_width, p.full_height), (1200, 2400));
    preview::release();
}

#[test]
fn remove_and_move_only_touch_library_photos() {
    let f = fixture();
    let a = f.root.join("a.png");
    let b = f.root.join("b.png");
    let c = f.root.join("c.png");
    for (i, p) in [&a, &b, &c].iter().enumerate() {
        save_png(p, &gradient(20, 20, i as u32));
    }
    index(&f);
    let outsider = f.root.join("outside.txt");
    std::fs::write(&outsider, b"keep").unwrap();
    let text = |p: &Path| p.to_string_lossy().into_owned();

    let r = files::remove(&f.lib, &[text(&a), text(&outsider)], RemoveMode::Delete);
    assert_eq!(r.done, vec![text(&a)]);
    assert_eq!(r.failed.len(), 1);
    assert!(!a.exists() && outsider.exists());

    let dest = f.root.join("Trip");
    save_png(&dest.join("b.png"), &gradient(5, 5, 9));
    let roots = vec![f.root.clone()];
    let m = files::move_to(&f.lib, &[text(&b)], &dest, &roots);
    assert_eq!(m.moved_to, vec![text(&dest.join("b (1).png"))]);
    assert!(!b.exists() && dest.join("b (1).png").exists());
    let names: Vec<String> = f.lib.list().unwrap().into_iter().map(|p| p.path).collect();
    assert!(names.contains(&text(&dest.join("b (1).png"))));

    let elsewhere = tempfile::tempdir().unwrap();
    let m2 = files::move_to(&f.lib, &[text(&c)], elsewhere.path(), &roots);
    assert_eq!(m2.done.len(), 1);
    let names: Vec<String> = f.lib.list().unwrap().into_iter().map(|p| p.path).collect();
    assert!(!names.contains(&text(&c)));
}

#[test]
fn gallery_formats_match_extensions() {
    assert_eq!(format_of(Path::new("/x/IMG_1.HEIC")), Some("heic"));
    assert_eq!(format_of(Path::new("/x/a.NEF")), Some("raw"));
    assert_eq!(format_of(Path::new("/x/a.psd")), Some("psd"));
    assert_eq!(format_of(Path::new("/x/a.svg")), None);
    assert_eq!(format_of(Path::new("/x/a.mp4")), None);
    assert_eq!(format_of(Path::new("/x/noext")), None);
}

#[test]
#[ignore = "needs OMNIGET_PHOTOS_FIXTURES with real camera files"]
fn real_files_index_and_export() {
    let Some(root) = std::env::var_os("OMNIGET_PHOTOS_FIXTURES").map(PathBuf::from) else {
        return;
    };
    let data = tempfile::tempdir().unwrap();
    let lib = Library::new(data.path());
    let started = std::time::Instant::now();
    let p = index::run(
        &lib,
        std::slice::from_ref(&root),
        &CancellationToken::new(),
        &|_| {},
    );
    eprintln!("indexed {} in {:?}", p.done, started.elapsed());
    for photo in lib.list().unwrap() {
        if photo.format == "raw" {
            let full = thumb::decode_full(Path::new(&photo.path), "raw").unwrap();
            assert_eq!(
                (photo.width, photo.height),
                (full.width(), full.height()),
                "{}",
                photo.name
            );
        }
        eprintln!(
            "{:<28} {:>5}x{:<5} taken={:?} err={:?}",
            photo.name, photo.width, photo.height, photo.taken_at, photo.error
        );
        assert!(photo.error.is_none(), "{}: {:?}", photo.name, photo.error);
    }
    let mut items = lib.hashed().unwrap();
    similar::fill_content(&lib, &mut items).unwrap();
    for g in similar::group(&items, similar::DEFAULT_DISTANCE) {
        eprintln!("{:?} keep={} {:?}", g.kind, g.keep, g.paths);
    }
    let out = tempfile::tempdir().unwrap();
    let options = ExportOptions {
        format: "jpeg".into(),
        quality: Some(85),
        max_size: Some(1200),
        out_dir: Some(out.path().to_string_lossy().into_owned()),
        apply_edits: true,
    };
    let mut taken = HashSet::new();
    for photo in lib.list().unwrap() {
        let o = export_one(
            Path::new(&photo.path),
            &photo.format,
            None,
            &options,
            &mut taken,
        );
        assert!(o.ok, "{}: {:?}", photo.name, o.error);
    }
}
