use std::borrow::Cow;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use image::imageops::{self, FilterType};
use image::{GrayImage, Luma, RgbImage};
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use serde::{Deserialize, Serialize};

use crate::core::tools::{self, ProgressFn};

pub const ID: &str = "sam21";
pub const NAME: &str = "SAM 2.1 tiny";
pub const LICENSE: &str = "Apache-2.0";
pub const SOURCE: &str = "https://huggingface.co/cstr/sam2.1-hiera-tiny-ONNX";
const REVISION: &str = "12bdf0f712731bd0b52ccbff807683ce49b726b9";

const SIDE: u32 = 1024;
const MASK_SIDE: u32 = 256;
const PROPOSALS: usize = 4;
const SHARPNESS: f32 = 4.0;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

pub struct File {
    pub name: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

pub const FILES: [File; 2] = [
    File {
        name: "encoder.onnx",
        sha256: "4fda6e68561e27808cedffa8689749a622bba5f11ba66f4ca4e3a8a9f6360db5",
        bytes: 109_473_919,
    },
    File {
        name: "decoder.onnx",
        sha256: "0ae944e08e55814c19afa4d143ef07b1a3dbad2decd77c15ec3ab220b9f57247",
        bytes: 20_655_795,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Click {
    pub x: f32,
    pub y: f32,
    pub keep: bool,
}

pub fn total_bytes() -> u64 {
    FILES.iter().map(|f| f.bytes).sum()
}

pub fn dir() -> Option<PathBuf> {
    tools::onnx::models_dir().map(|d| d.join(ID))
}

fn url(file: &File) -> String {
    format!("{SOURCE}/resolve/{REVISION}/{}", file.name)
}

fn has(dir: &Path, file: &File) -> bool {
    std::fs::metadata(dir.join(file.name))
        .map(|m| m.is_file() && m.len() == file.bytes)
        .unwrap_or(false)
}

pub fn is_downloaded() -> bool {
    dir().is_some_and(|d| FILES.iter().all(|f| has(&d, f)))
}

fn sha256_of(path: &Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub async fn ensure(progress: &ProgressFn) -> anyhow::Result<PathBuf> {
    let dir = dir().ok_or_else(|| anyhow!("could not find the app data folder"))?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let client = tools::client()?;
    for file in &FILES {
        if has(&dir, file) {
            continue;
        }
        let pid = format!("onnx-model:{ID}:{}", file.name);
        let tmp = dir.join(format!(".{}.download", file.name));
        tools::download_to(&client, &url(file), &tmp, progress, &pid).await?;
        let dest = dir.join(file.name);
        let expected = file.sha256;
        let p = progress.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            tools::report(&p, &pid, "verify", 0, None, Some("checking sha256".into()));
            let got = sha256_of(&tmp)?;
            if got != expected {
                let _ = std::fs::remove_file(&tmp);
                return Err(anyhow!(
                    "the downloaded model does not match: expected sha256 {expected}, got {got}"
                ));
            }
            std::fs::rename(&tmp, &dest).with_context(|| format!("moving to {}", dest.display()))
        })
        .await
        .map_err(|e| anyhow!("verification task failed: {e}"))??;
    }
    Ok(dir)
}

pub struct Features {
    embed: Vec<f32>,
    high_res_0: Vec<f32>,
    high_res_1: Vec<f32>,
}

const EMBED_SHAPE: [i64; 4] = [1, 256, 64, 64];
const HIGH_RES_0_SHAPE: [i64; 4] = [1, 32, 256, 256];
const HIGH_RES_1_SHAPE: [i64; 4] = [1, 64, 128, 128];

fn len_of(shape: &[i64]) -> usize {
    shape.iter().product::<i64>() as usize
}

pub struct Sam {
    encoder: Session,
    decoder: Session,
}

pub fn input(img: &RgbImage) -> Vec<f32> {
    let square = imageops::resize(img, SIDE, SIDE, FilterType::Triangle);
    let plane = (SIDE * SIDE) as usize;
    let mut out = vec![0f32; plane * 3];
    for (i, px) in square.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (px.0[c] as f32 / 255.0 - MEAN[c]) / STD[c];
        }
    }
    out
}

pub fn prompt(clicks: &[Click]) -> (Vec<f32>, Vec<i64>) {
    let mut coords = Vec::with_capacity(clicks.len() * 2);
    let mut labels = Vec::with_capacity(clicks.len());
    for c in clicks {
        coords.push(c.x.clamp(0.0, 1.0) * SIDE as f32);
        coords.push(c.y.clamp(0.0, 1.0) * SIDE as f32);
        labels.push(i64::from(c.keep));
    }
    (coords, labels)
}

pub fn pick(scores: &[f32], clicks: usize) -> usize {
    if clicks > 1 {
        return 0;
    }
    scores
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, s)| s.is_finite())
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i)
}

pub fn coverage(logits: &[f32]) -> Option<GrayImage> {
    let side = MASK_SIDE as usize;
    if logits.len() < side * side {
        return None;
    }
    let at = |x: usize, y: usize| logits[y.min(side - 1) * side + x.min(side - 1)];
    let scale = MASK_SIDE as f32 / SIDE as f32;
    Some(GrayImage::from_fn(SIDE, SIDE, |x, y| {
        let fy = ((y as f32 + 0.5) * scale - 0.5).max(0.0);
        let fx = ((x as f32 + 0.5) * scale - 0.5).max(0.0);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let (x0, tx) = (fx.floor() as usize, fx.fract());
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
        let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
        let logit = top * (1.0 - ty) + bottom * ty;
        let p = 1.0 / (1.0 + (-logit * SHARPNESS).exp());
        Luma([(p * 255.0).round() as u8])
    }))
}

fn tensor(shape: &[i64], data: &[f32]) -> anyhow::Result<Tensor<f32>> {
    Tensor::from_array((shape.to_vec(), data.to_vec())).context("preparing a SAM 2.1 input")
}

impl Sam {
    pub fn load() -> anyhow::Result<Self> {
        let dir = dir().ok_or_else(|| anyhow!("could not find the app data folder"))?;
        if !is_downloaded() {
            return Err(anyhow!("SAM 2.1 has not been downloaded yet"));
        }
        crate::core::onnxrt::init()?;
        Ok(Self {
            encoder: tools::onnx::session_from_file(&dir.join(FILES[0].name))?,
            decoder: tools::onnx::session_from_file(&dir.join(FILES[1].name))?,
        })
    }

    pub fn encode(&mut self, img: &RgbImage) -> anyhow::Result<Features> {
        let image = Tensor::from_array((vec![1i64, 3, SIDE as i64, SIDE as i64], input(img)))
            .context("preparing the picture for SAM 2.1")?;
        let inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> =
            vec![(Cow::Borrowed("image"), image.into())];
        let out = self
            .encoder
            .run(inputs)
            .context("SAM 2.1 could not look at the picture")?;
        let take = |name: &str, shape: &[i64]| -> anyhow::Result<Vec<f32>> {
            let value = out
                .get(name)
                .ok_or_else(|| anyhow!("SAM 2.1 has no `{name}` output"))?;
            let (_, data) = value.try_extract_tensor::<f32>()?;
            if data.len() != len_of(shape) {
                return Err(anyhow!("SAM 2.1 returned `{name}` with the wrong size"));
            }
            Ok(data.to_vec())
        };
        Ok(Features {
            embed: take("image_embed", &EMBED_SHAPE)?,
            high_res_0: take("high_res_0", &HIGH_RES_0_SHAPE)?,
            high_res_1: take("high_res_1", &HIGH_RES_1_SHAPE)?,
        })
    }

    pub fn segment(&mut self, f: &Features, clicks: &[Click]) -> anyhow::Result<GrayImage> {
        if clicks.is_empty() {
            return Err(anyhow!("SAM 2.1 needs at least one click"));
        }
        let (coords, labels) = prompt(clicks);
        let n = labels.len() as i64;
        let inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> = vec![
            (
                Cow::Borrowed("image_embed"),
                tensor(&EMBED_SHAPE, &f.embed)?.into(),
            ),
            (
                Cow::Borrowed("high_res_0"),
                tensor(&HIGH_RES_0_SHAPE, &f.high_res_0)?.into(),
            ),
            (
                Cow::Borrowed("high_res_1"),
                tensor(&HIGH_RES_1_SHAPE, &f.high_res_1)?.into(),
            ),
            (
                Cow::Borrowed("point_coords"),
                Tensor::from_array((vec![1i64, n, 2], coords))?.into(),
            ),
            (
                Cow::Borrowed("point_labels"),
                Tensor::from_array((vec![1i64, n], labels))?.into(),
            ),
        ];
        let out = self
            .decoder
            .run(inputs)
            .context("SAM 2.1 could not read the clicks")?;
        let (_, masks) = out["mask_logits"].try_extract_tensor::<f32>()?;
        let (_, scores) = out["iou"].try_extract_tensor::<f32>()?;
        let side = (MASK_SIDE * MASK_SIDE) as usize;
        if masks.len() != PROPOSALS * side || scores.len() != PROPOSALS {
            return Err(anyhow!("SAM 2.1 returned masks of the wrong size"));
        }
        let best = pick(scores, clicks.len());
        coverage(&masks[best * side..(best + 1) * side])
            .ok_or_else(|| anyhow!("SAM 2.1 returned an empty mask"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn clicks_become_points_in_the_model_square() {
        let (coords, labels) = prompt(&[
            Click {
                x: 0.5,
                y: 0.25,
                keep: true,
            },
            Click {
                x: 1.5,
                y: -1.0,
                keep: false,
            },
        ]);
        assert_eq!(coords, vec![512.0, 256.0, 1024.0, 0.0]);
        assert_eq!(labels, vec![1, 0]);
    }

    #[test]
    fn one_click_takes_the_best_proposal_and_more_take_the_single_mask() {
        assert_eq!(pick(&[0.9, 0.2, 0.8, 0.5], 1), 2);
        assert_eq!(pick(&[0.9, f32::NAN, f32::NAN, f32::NAN], 1), 0);
        assert_eq!(pick(&[0.1, 0.2, 0.8, 0.5], 3), 0);
    }

    #[test]
    fn logits_turn_into_a_full_size_mask() {
        let side = MASK_SIDE as usize;
        let mut logits = vec![-10.0f32; side * side];
        for y in 0..side {
            for x in 0..side / 2 {
                logits[y * side + x] = 10.0;
            }
        }
        let m = coverage(&logits).unwrap();
        assert_eq!(m.dimensions(), (SIDE, SIDE));
        assert_eq!(m.get_pixel(10, 500).0[0], 255);
        assert_eq!(m.get_pixel(1000, 500).0[0], 0);
        assert!(coverage(&logits[..10]).is_none());
    }

    #[test]
    fn input_is_stretched_and_normalised() {
        let img = RgbImage::from_pixel(30, 10, Rgb([255, 0, 0]));
        let data = input(&img);
        let plane = (SIDE * SIDE) as usize;
        assert_eq!(data.len(), plane * 3);
        assert!((data[0] - (1.0 - 0.485) / 0.229).abs() < 1e-4);
        assert!((data[plane] - (0.0 - 0.456) / 0.224).abs() < 1e-4);
    }

    #[test]
    fn the_download_is_pinned() {
        for file in &FILES {
            assert!(url(file).contains(REVISION));
            assert_eq!(file.sha256.len(), 64);
        }
        assert_eq!(total_bytes(), 130_129_714);
    }
}
