pub mod mask;
pub mod output;
pub mod rembg;
pub mod sam;
pub mod source;

use std::collections::VecDeque;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use base64::Engine as _;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, GrayImage};
use ort::session::Session;
use serde::{Deserialize, Serialize};

use crate::core::tools::{onnx, ProgressFn};

pub use mask::{Edge, Fill};
pub use output::Format;
pub use sam::Click;

pub const WORK_SIDE: u32 = 1024;
const MASK_CACHE: usize = 24;
const PICTURE_CACHE: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    pub license: String,
    pub source: String,
    pub downloaded: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub runtime_installed: bool,
    pub runtime_downloadable: bool,
    pub models: Vec<ModelInfo>,
    pub refine: ModelInfo,
}

pub fn status() -> Status {
    let models = rembg::SPECS
        .iter()
        .filter_map(|s| onnx::find(s.id))
        .map(|m| ModelInfo {
            id: m.id.to_string(),
            name: m.name.to_string(),
            bytes: m.bytes,
            license: m.license.to_string(),
            source: m.source.to_string(),
            downloaded: onnx::is_downloaded(m.id),
        })
        .collect();
    Status {
        runtime_installed: crate::core::onnxrt::is_installed(),
        runtime_downloadable: crate::core::onnxrt::auto_variant_id().is_some(),
        models,
        refine: ModelInfo {
            id: sam::ID.to_string(),
            name: sam::NAME.to_string(),
            bytes: sam::total_bytes(),
            license: sam::LICENSE.to_string(),
            source: sam::SOURCE.to_string(),
            downloaded: sam::is_downloaded(),
        },
    }
}

pub fn is_model(id: &str) -> bool {
    id == sam::ID || rembg::spec(id).is_some()
}

pub async fn install(id: &str, progress: &ProgressFn) -> anyhow::Result<()> {
    if !is_model(id) {
        return Err(anyhow!("unknown model: {id}"));
    }
    if !crate::core::onnxrt::is_installed() {
        crate::core::onnxrt::install_runtime(None, progress).await?;
    }
    if id == sam::ID {
        sam::ensure(progress).await?;
    } else {
        onnx::ensure_model(id, progress).await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Background {
    #[default]
    None,
    Color {
        color: String,
    },
    Image {
        path: String,
    },
}

pub async fn fill_for(bg: &Background) -> Result<Fill, String> {
    match bg {
        Background::None => Ok(Fill::Transparent),
        Background::Color { color } => mask::parse_color(color)
            .map(Fill::Color)
            .ok_or_else(|| format!("\"{color}\" is not a color.")),
        Background::Image { path } => source::load(Path::new(path), None)
            .await
            .map(|img| Fill::Image(img.to_rgba8()))
            .map_err(|e| format!("The background picture could not be used. {e}")),
    }
}

pub async fn preview_fill(bg: &Background) -> Result<Fill, String> {
    match bg {
        Background::Image { path } => working_picture(Path::new(path), None)
            .await
            .map(|img| Fill::Image(img.to_rgba8()))
            .map_err(|e| format!("The background picture could not be used. {e}")),
        other => fill_for(other).await,
    }
}

pub fn shrink(img: &DynamicImage, side: u32) -> DynamicImage {
    if img.width().max(img.height()) <= side {
        return img.clone();
    }
    img.resize(side, side, FilterType::Triangle)
}

struct Engine {
    rembg: Option<(&'static str, Session)>,
    sam: Option<sam::Sam>,
    features: Option<(String, sam::Features)>,
    masks: VecDeque<(String, GrayImage)>,
}

impl Engine {
    const fn new() -> Self {
        Self {
            rembg: None,
            sam: None,
            features: None,
            masks: VecDeque::new(),
        }
    }
}

static ENGINE: Mutex<Engine> = Mutex::new(Engine::new());
static PICTURES: Mutex<VecDeque<(String, Arc<DynamicImage>)>> = Mutex::new(VecDeque::new());

fn engine() -> std::sync::MutexGuard<'static, Engine> {
    ENGINE.lock().unwrap_or_else(|e| e.into_inner())
}

fn pictures() -> std::sync::MutexGuard<'static, VecDeque<(String, Arc<DynamicImage>)>> {
    PICTURES.lock().unwrap_or_else(|e| e.into_inner())
}

fn remember<T: Clone>(list: &mut VecDeque<(String, T)>, key: String, value: T, cap: usize) {
    list.retain(|(k, _)| *k != key);
    list.push_front((key, value));
    list.truncate(cap);
}

fn recall<T: Clone>(list: &VecDeque<(String, T)>, key: &str) -> Option<T> {
    list.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

pub fn release() {
    pictures().clear();
    let mut e = engine();
    *e = Engine::new();
}

pub async fn working_picture(
    path: &Path,
    frame_ms: Option<u64>,
) -> Result<Arc<DynamicImage>, String> {
    let key = source::key(path, frame_ms);
    if let Some(hit) = recall(&pictures(), &key) {
        return Ok(hit);
    }
    let full = source::load(path, frame_ms).await?;
    let small = tokio::task::spawn_blocking(move || Arc::new(shrink(&full, WORK_SIDE)))
        .await
        .map_err(|e| format!("The picture could not be prepared: {e}"))?;
    remember(&mut pictures(), key, small.clone(), PICTURE_CACHE);
    Ok(small)
}

fn mask_key(key: &str, model: &str, clicks: &[Click]) -> String {
    if clicks.is_empty() {
        return format!("{key}#{model}");
    }
    let points: Vec<String> = clicks
        .iter()
        .map(|c| format!("{:.4},{:.4},{}", c.x, c.y, u8::from(c.keep)))
        .collect();
    format!("{key}#{}#{}", sam::ID, points.join(";"))
}

pub fn mask(
    key: &str,
    work: &DynamicImage,
    model: &str,
    clicks: &[Click],
) -> anyhow::Result<GrayImage> {
    let id = mask_key(key, model, clicks);
    let mut e = engine();
    if let Some(hit) = recall(&e.masks, &id) {
        return Ok(hit);
    }
    let rgb = work.to_rgb8();
    let raw = if clicks.is_empty() {
        let spec = rembg::spec(model).ok_or_else(|| anyhow!("unknown model: {model}"))?;
        if e.rembg.as_ref().map(|(m, _)| *m) != Some(spec.id) {
            e.rembg = None;
            e.rembg = Some((spec.id, onnx::session_for(spec.id)?));
        }
        let (_, session) = e.rembg.as_mut().ok_or_else(|| anyhow!("no model loaded"))?;
        rembg::run(session, &rgb, spec)?
    } else {
        if e.sam.is_none() {
            e.sam = Some(sam::Sam::load()?);
        }
        let engine = &mut *e;
        let model = engine
            .sam
            .as_mut()
            .ok_or_else(|| anyhow!("no model loaded"))?;
        if engine.features.as_ref().map(|(k, _)| k.as_str()) != Some(key) {
            engine.features = None;
            engine.features = Some((key.to_string(), model.encode(&rgb)?));
        }
        let (_, features) = engine
            .features
            .as_ref()
            .ok_or_else(|| anyhow!("no picture encoded"))?;
        model.segment(features, clicks)?
    };
    remember(&mut e.masks, id, raw.clone(), MASK_CACHE);
    Ok(raw)
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub before: String,
    pub after: String,
    pub width: u32,
    pub height: u32,
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn before_url(img: &DynamicImage) -> Result<String, String> {
    if img.color().has_alpha() {
        let bytes = output::encode(&img.to_rgba8(), Format::Png)?;
        return Ok(data_url("image/png", &bytes));
    }
    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&img.to_rgb8())
        .map_err(|e| e.to_string())?;
    Ok(data_url("image/jpeg", &out.into_inner()))
}

pub fn preview(
    work: &DynamicImage,
    raw: &GrayImage,
    edge: Edge,
    fill: &Fill,
) -> Result<Preview, String> {
    let after = mask::render(&work.to_rgba8(), raw, edge, fill);
    let bytes = output::encode(&after, Format::Png)?;
    Ok(Preview {
        before: before_url(work)?,
        after: data_url("image/png", &bytes),
        width: work.width(),
        height: work.height(),
    })
}

pub fn cut(full: &DynamicImage, raw: &GrayImage, edge: Edge, fill: &Fill) -> image::RgbaImage {
    mask::render(&full.to_rgba8(), raw, edge, fill)
}

#[cfg(test)]
mod tests;
