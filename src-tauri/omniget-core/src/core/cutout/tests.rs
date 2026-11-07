use super::*;
use image::{Luma, Rgb, RgbImage};

fn circle_scene(w: u32, h: u32) -> (RgbImage, GrayImage) {
    let (cx, cy, r) = (w as f32 * 0.5, h as f32 * 0.55, h as f32 * 0.3);
    let inside = |x: u32, y: u32| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        dx * dx + dy * dy <= r * r
    };
    let picture = RgbImage::from_fn(w, h, |x, y| {
        if inside(x, y) {
            Rgb([220, 40, 40])
        } else {
            let shade = (60 + (y * 100 / h)) as u8;
            Rgb([shade, shade + 20, 200])
        }
    });
    let truth = GrayImage::from_fn(w, h, |x, y| Luma([if inside(x, y) { 255 } else { 0 }]));
    (picture, truth)
}

fn iou(a: &GrayImage, b: &GrayImage) -> f32 {
    let (mut both, mut either) = (0u32, 0u32);
    for (p, q) in a.pixels().zip(b.pixels()) {
        let (x, y) = (p.0[0] >= 128, q.0[0] >= 128);
        both += u32::from(x && y);
        either += u32::from(x || y);
    }
    both as f32 / either.max(1) as f32
}

fn alpha(img: &image::RgbaImage) -> GrayImage {
    GrayImage::from_fn(img.width(), img.height(), |x, y| {
        Luma([img.get_pixel(x, y).0[3]])
    })
}

#[test]
fn backgrounds_deserialize_from_the_frontend_shape() {
    let none: Background = serde_json::from_str(r#"{"kind":"none"}"#).unwrap();
    assert_eq!(none, Background::None);
    let color: Background =
        serde_json::from_str(r##"{"kind":"color","color":"#00ff00"}"##).unwrap();
    assert_eq!(
        color,
        Background::Color {
            color: "#00ff00".into()
        }
    );
    let edge: Edge = serde_json::from_str(r#"{"feather":2.5}"#).unwrap();
    assert_eq!(edge.shift, 0.0);
}

#[tokio::test]
async fn backgrounds_turn_into_fills() {
    assert!(matches!(
        fill_for(&Background::None).await,
        Ok(Fill::Transparent)
    ));
    assert!(matches!(
        fill_for(&Background::Color {
            color: "#123456".into()
        })
        .await,
        Ok(Fill::Color([0x12, 0x34, 0x56]))
    ));
    assert!(fill_for(&Background::Color {
        color: "blue-ish".into()
    })
    .await
    .is_err());
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bg.png");
    RgbImage::from_pixel(3, 2, Rgb([1, 2, 3])).save(&p).unwrap();
    let Ok(Fill::Image(img)) = fill_for(&Background::Image {
        path: p.to_string_lossy().into_owned(),
    })
    .await
    else {
        panic!("expected an image fill");
    };
    assert_eq!(img.dimensions(), (3, 2));
}

#[test]
fn clicks_get_their_own_cache_entry() {
    let a = mask_key("k", "u2netp", &[]);
    let b = mask_key("k", "silueta", &[]);
    let c = mask_key(
        "k",
        "u2netp",
        &[Click {
            x: 0.5,
            y: 0.5,
            keep: true,
        }],
    );
    let d = mask_key(
        "k",
        "silueta",
        &[Click {
            x: 0.5,
            y: 0.5,
            keep: true,
        }],
    );
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_eq!(c, d);
}

#[test]
fn unknown_models_are_refused() {
    assert!(is_model("u2netp"));
    assert!(is_model(sam::ID));
    assert!(!is_model("u2net-human-seg"));
    assert_eq!(status().models.len(), rembg::SPECS.len());
}

#[test]
fn a_low_res_mask_cuts_the_full_picture_and_saves() {
    let (picture, truth) = circle_scene(1200, 800);
    let full = DynamicImage::ImageRgb8(picture);
    let work = shrink(&full, WORK_SIDE);
    assert_eq!((work.width(), work.height()), (1024, 683));
    let raw = image::imageops::resize(&truth, 320, 320, FilterType::Triangle);

    let cut_out = cut(&full, &raw, Edge::default(), &Fill::Transparent);
    assert_eq!(cut_out.dimensions(), (1200, 800));
    assert!(iou(&alpha(&cut_out), &truth) > 0.97);
    assert_eq!(cut_out.get_pixel(600, 440).0, [220, 40, 40, 255]);
    assert_eq!(cut_out.get_pixel(20, 20).0[3], 0);

    let grown = cut(
        &full,
        &raw,
        Edge {
            feather: 0.0,
            shift: 10.0,
        },
        &Fill::Transparent,
    );
    let count = |m: &GrayImage| m.pixels().filter(|p| p.0[0] >= 128).count();
    assert!(count(&alpha(&grown)) > count(&alpha(&cut_out)));

    let dir = tempfile::tempdir().unwrap();
    let out = output::plan(
        &[(dir.path().join("scene.jpg"), None)],
        None,
        Format::Webp,
        false,
    );
    std::fs::write(&out[0], output::encode(&cut_out, Format::Webp).unwrap()).unwrap();
    let back = image::open(&out[0]).unwrap().to_rgba8();
    assert_eq!(back.dimensions(), (1200, 800));
    assert_eq!(back.get_pixel(20, 20).0[3], 0);

    let p = preview(&work, &raw, Edge::default(), &Fill::Color([0, 0, 0])).unwrap();
    assert!(p.before.starts_with("data:image/jpeg;base64,"));
    assert!(p.after.starts_with("data:image/png;base64,"));
    assert_eq!((p.width, p.height), (1024, 683));
}

#[tokio::test]
async fn the_working_picture_is_shrunk_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("big.png");
    circle_scene(2000, 1000).0.save(&p).unwrap();
    let first = working_picture(&p, None).await.unwrap();
    assert_eq!((first.width(), first.height()), (1024, 512));
    let second = working_picture(&p, None).await.unwrap();
    assert!(Arc::ptr_eq(&first, &second));
}

fn fixture_dir() -> Option<std::path::PathBuf> {
    std::env::var("OMNIGET_CUTOUT_FIXTURES")
        .ok()
        .map(std::path::PathBuf::from)
}

#[tokio::test]
#[ignore = "downloads the ONNX Runtime and models; set OMNIGET_DATA_DIR and OMNIGET_CUTOUT_FIXTURES"]
async fn real_models_cut_a_circle_out() {
    let progress = crate::core::tools::noop_progress();
    let (picture, truth) = circle_scene(900, 600);
    let full = DynamicImage::ImageRgb8(picture);
    let work = shrink(&full, WORK_SIDE);
    if let Some(dir) = fixture_dir() {
        std::fs::create_dir_all(&dir).unwrap();
        full.save(dir.join("circle.png")).unwrap();
    }
    for model in ["u2netp", sam::ID] {
        install(model, &progress).await.unwrap();
    }
    let started = std::time::Instant::now();
    let auto = mask("circle", &work, "u2netp", &[]).unwrap();
    let auto_ms = started.elapsed().as_millis();
    let cut_auto = cut(&full, &auto, Edge::default(), &Fill::Transparent);
    let auto_iou = iou(&alpha(&cut_auto), &truth);

    let started = std::time::Instant::now();
    let click = [Click {
        x: 0.5,
        y: 0.55,
        keep: true,
    }];
    let refined = mask("circle", &work, "u2netp", &click).unwrap();
    let first_click_ms = started.elapsed().as_millis();
    let started = std::time::Instant::now();
    let two = [
        click[0],
        Click {
            x: 0.05,
            y: 0.05,
            keep: false,
        },
    ];
    let refined_two = mask("circle", &work, "u2netp", &two).unwrap();
    let next_click_ms = started.elapsed().as_millis();
    let cut_click = cut(&full, &refined, Edge::default(), &Fill::Transparent);
    let click_iou = iou(&alpha(&cut_click), &truth);
    let two_iou = iou(
        &alpha(&cut(
            &full,
            &refined_two,
            Edge::default(),
            &Fill::Transparent,
        )),
        &truth,
    );
    if let Some(dir) = fixture_dir() {
        cut_auto.save(dir.join("circle-u2netp.png")).unwrap();
        cut_click.save(dir.join("circle-sam21.png")).unwrap();
    }
    eprintln!(
        "u2netp iou {auto_iou:.3} in {auto_ms} ms; sam21 iou {click_iou:.3} first click {first_click_ms} ms, two clicks iou {two_iou:.3} in {next_click_ms} ms"
    );
    assert!(auto_iou > 0.9, "u2netp iou {auto_iou}");
    assert!(click_iou > 0.9, "sam21 iou {click_iou}");
    assert!(two_iou > 0.9, "sam21 two clicks iou {two_iou}");
}
