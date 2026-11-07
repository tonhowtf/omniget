use std::path::Path;
use std::sync::{Arc, Mutex};

use ::image::DynamicImage;
use base64::Engine;
use serde::Serialize;

use super::{adjust, thumb, Edits};

pub const PREVIEW_SIZE: u32 = 1600;
const PREVIEW_QUALITY: u8 = 86;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Preview {
    pub src: String,
    pub width: u32,
    pub height: u32,
    pub full_width: u32,
    pub full_height: u32,
}

struct Base {
    key: (String, u64, i64),
    image: Arc<DynamicImage>,
    full: (u32, u32),
}

static BASE: Mutex<Option<Base>> = Mutex::new(None);

fn stamp(path: &Path) -> (u64, i64) {
    std::fs::metadata(path)
        .map(|m| {
            (
                m.len(),
                m.modified()
                    .map(super::meta::wall_clock)
                    .unwrap_or_default(),
            )
        })
        .unwrap_or_default()
}

fn base(path: &Path, format: &str) -> Result<(Arc<DynamicImage>, (u32, u32)), String> {
    let (size, modified) = stamp(path);
    let key = (path.to_string_lossy().into_owned(), size, modified);
    if let Ok(guard) = BASE.lock() {
        if let Some(b) = guard.as_ref().filter(|b| b.key == key) {
            return Ok((b.image.clone(), b.full));
        }
    }
    let full = thumb::decode_full(path, format)?;
    let dims = (full.width(), full.height());
    let image = Arc::new(thumb::downscale(&full, PREVIEW_SIZE));
    drop(full);
    if let Ok(mut guard) = BASE.lock() {
        *guard = Some(Base {
            key,
            image: image.clone(),
            full: dims,
        });
    }
    Ok((image, dims))
}

pub fn release() {
    if let Ok(mut guard) = BASE.lock() {
        *guard = None;
    }
}

pub fn render(path: &Path, format: &str, edits: &Edits) -> Result<Preview, String> {
    let (image, (fw, fh)) = base(path, format)?;
    let out = adjust::apply((*image).clone(), edits);
    let rgb = thumb::flatten(&out);
    let jpeg = thumb::jpeg(&rgb, PREVIEW_QUALITY)?;
    let rotated = edits.normalized().rotate % 180 == 90;
    let (full_width, full_height) = if rotated { (fh, fw) } else { (fw, fh) };
    Ok(Preview {
        src: format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(jpeg)
        ),
        width: rgb.width(),
        height: rgb.height(),
        full_width,
        full_height,
    })
}
