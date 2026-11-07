use std::borrow::Cow;

use anyhow::{anyhow, Context};
use image::imageops::{self, FilterType};
use image::{GrayImage, RgbImage};
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;

#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub id: &'static str,
    pub side: u32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
}

const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

pub const SPECS: &[Spec] = &[
    Spec {
        id: "u2netp",
        side: 320,
        mean: IMAGENET_MEAN,
        std: IMAGENET_STD,
    },
    Spec {
        id: "silueta",
        side: 320,
        mean: IMAGENET_MEAN,
        std: IMAGENET_STD,
    },
    Spec {
        id: "isnet-general-use",
        side: 1024,
        mean: [0.5, 0.5, 0.5],
        std: [1.0, 1.0, 1.0],
    },
];

pub fn spec(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.id == id)
}

pub fn input(img: &RgbImage, spec: &Spec) -> Vec<f32> {
    let side = spec.side;
    let small = imageops::resize(img, side, side, FilterType::Lanczos3);
    let peak = small.as_raw().iter().copied().max().unwrap_or(0).max(1) as f32;
    let plane = (side * side) as usize;
    let mut out = vec![0f32; plane * 3];
    for (i, px) in small.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (px.0[c] as f32 / peak - spec.mean[c]) / spec.std[c];
        }
    }
    out
}

pub fn run(session: &mut Session, img: &RgbImage, spec: &Spec) -> anyhow::Result<GrayImage> {
    let side = spec.side as i64;
    let tensor = Tensor::from_array((vec![1i64, 3, side, side], input(img, spec)))
        .context("preparing the picture for the model")?;
    let in_name = session
        .inputs()
        .first()
        .map(|i| i.name().to_string())
        .ok_or_else(|| anyhow!("the model has no input"))?;
    let out_name = session
        .outputs()
        .first()
        .map(|o| o.name().to_string())
        .ok_or_else(|| anyhow!("the model has no output"))?;
    let inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> =
        vec![(Cow::Owned(in_name), tensor.into())];
    let out = session.run(inputs).context("running the model")?;
    let (_, values) = out[out_name.as_str()]
        .try_extract_tensor::<f32>()
        .context("reading the mask")?;
    super::mask::from_values(values, spec.side, spec.side)
        .ok_or_else(|| anyhow!("the model returned a mask of the wrong size"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn every_catalog_model_has_a_spec() {
        for spec in SPECS {
            let model = crate::core::tools::onnx::find(spec.id).expect("in the onnx catalog");
            assert_eq!(model.family, "rembg");
        }
    }

    #[test]
    fn input_is_channel_first_and_normalised_by_the_brightest_value() {
        let img = RgbImage::from_pixel(10, 7, Rgb([100, 50, 0]));
        let spec = spec("u2netp").unwrap();
        let data = input(&img, spec);
        let plane = 320 * 320;
        assert_eq!(data.len(), plane * 3);
        assert!((data[0] - (1.0 - 0.485) / 0.229).abs() < 1e-4);
        assert!((data[plane] - (0.5 - 0.456) / 0.224).abs() < 1e-4);
        assert!((data[2 * plane] - (0.0 - 0.406) / 0.225).abs() < 1e-4);
    }

    #[test]
    fn isnet_reads_a_bigger_square_with_its_own_normalisation() {
        let img = RgbImage::from_pixel(3, 3, Rgb([255, 255, 255]));
        let spec = spec("isnet-general-use").unwrap();
        let data = input(&img, spec);
        assert_eq!(data.len(), 1024 * 1024 * 3);
        assert!(data.iter().all(|v| (v - 0.5).abs() < 1e-5));
    }
}
