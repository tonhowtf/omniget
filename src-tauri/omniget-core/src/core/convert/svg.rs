use std::sync::{Arc, OnceLock};

use ::image::{DynamicImage, RgbaImage};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{fontdb, ImageHrefResolver, Options, Tree};

fn fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

pub fn rasterize(bytes: &[u8]) -> Result<DynamicImage, String> {
    let options = Options {
        fontdb: fonts(),
        image_href_resolver: ImageHrefResolver {
            resolve_data: ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..Options::default()
    };
    let tree = Tree::from_data(bytes, &options)
        .map_err(|e| format!("This SVG could not be read: {e}."))?;
    let size = tree.size().to_int_size();
    super::image::check_pixels(size.width(), size.height())?;
    let mut pixmap = Pixmap::new(size.width(), size.height())
        .ok_or_else(|| "This SVG has no visible size.".to_string())?;
    resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    for px in rgba.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(size.width(), size.height(), rgba)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| "This SVG could not be drawn.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_shapes_at_their_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect x="0" y="0" width="20" height="20" fill="#ff0000"/><rect x="20" y="0" width="20" height="20" fill="#0000ff" fill-opacity="0.5"/></svg>"##;
        let img = rasterize(svg).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (40, 20));
        assert_eq!(img.get_pixel(5, 5).0, [255, 0, 0, 255]);
        let half = img.get_pixel(30, 5).0;
        assert_eq!(half[2], 255);
        assert!((126..=129).contains(&half[3]), "{half:?}");
    }

    #[test]
    fn ignores_images_that_point_at_local_files() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("secret.png");
        RgbaImage::from_pixel(10, 10, ::image::Rgba([0, 255, 0, 255]))
            .save(&png)
            .unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="{}" width="10" height="10"/></svg>"#,
            png.display()
        );
        let img = rasterize(svg.as_bytes()).unwrap().to_rgba8();
        assert!(img.pixels().all(|p| p[3] == 0));
    }

    #[test]
    fn rejects_broken_svg() {
        assert!(rasterize(b"<svg").unwrap_err().contains("SVG"));
    }
}
