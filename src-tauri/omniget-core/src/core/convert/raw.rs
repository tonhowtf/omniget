use ::image::metadata::Orientation;
use ::image::{DynamicImage, ImageBuffer, Rgb, RgbImage};
use rawler::decoders::RawDecodeParams;
use rawler::imgop::develop::{ProcessingStep, RawDevelop};
use rawler::rawsource::RawSource;

const MIDDLE_GRAY: f32 = 0.18;
const MAX_GAIN: f32 = 4.0;
const SHOULDER: f32 = 0.8;
const CONTRAST: f32 = 0.25;

pub fn decode(bytes: &[u8]) -> Result<DynamicImage, String> {
    let bad = |e: rawler::RawlerError| format!("This camera RAW file could not be read: {e}.");
    let source = RawSource::new_from_slice(bytes);
    let decoder = rawler::get_decoder(&source)
        .map_err(|e| format!("OmniGet doesn't support this camera's RAW files yet ({e})."))?;
    let raw = decoder
        .raw_image(&source, &RawDecodeParams::default(), false)
        .map_err(bad)?;
    super::image::check_pixels(
        u32::try_from(raw.width).unwrap_or(u32::MAX),
        u32::try_from(raw.height).unwrap_or(u32::MAX),
    )?;
    let develop = RawDevelop {
        steps: RawDevelop::default()
            .steps
            .into_iter()
            .filter(|s| *s != ProcessingStep::SRgb)
            .collect(),
    };
    let linear = develop
        .develop_intermediate(&raw)
        .map_err(bad)?
        .to_dynamic_image()
        .ok_or_else(|| "This camera RAW file could not be developed.".to_string())?
        .into_rgb16();
    let mut img = DynamicImage::ImageRgb8(tone_map(&linear));
    let exif = u8::try_from(raw.orientation.to_u16()).unwrap_or(1);
    img.apply_orientation(Orientation::from_exif(exif).unwrap_or(Orientation::NoTransforms));
    Ok(img)
}

pub fn tone_map(linear: &ImageBuffer<Rgb<u16>, Vec<u16>>) -> RgbImage {
    let gain = exposure_gain(linear);
    let lut: Vec<u8> = (0..=u16::MAX)
        .map(|v| {
            let x = curve(f32::from(v) / f32::from(u16::MAX) * gain);
            (x * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect();
    let mut out = RgbImage::new(linear.width(), linear.height());
    for (dst, src) in out.pixels_mut().zip(linear.pixels()) {
        *dst = Rgb(src.0.map(|c| lut[usize::from(c)]));
    }
    out
}

fn exposure_gain(linear: &ImageBuffer<Rgb<u16>, Vec<u16>>) -> f32 {
    let pixels = linear.as_raw().chunks_exact(3);
    let step = (pixels.len() / 200_000).max(1);
    let (mut sum, mut count) = (0f64, 0u32);
    for p in pixels.step_by(step) {
        let y = (0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
            / f64::from(u16::MAX);
        sum += (y + 1e-4).ln();
        count += 1;
    }
    if count == 0 {
        return 1.0;
    }
    let average = (sum / f64::from(count)).exp() as f32;
    (MIDDLE_GRAY / average.max(1e-4)).clamp(1.0, MAX_GAIN)
}

fn curve(x: f32) -> f32 {
    let x = if x > SHOULDER {
        SHOULDER + (1.0 - SHOULDER) * (1.0 - (-(x - SHOULDER) / (1.0 - SHOULDER)).exp())
    } else {
        x.max(0.0)
    };
    let srgb = if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    let s = srgb * srgb * (3.0 - 2.0 * srgb);
    srgb + CONTRAST * (s - srgb)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    enum Field {
        Short(Vec<u16>),
        Long(Vec<u32>),
        Byte(Vec<u8>),
        Ascii(&'static str),
        Rational(Vec<(u32, u32)>),
        SRational(Vec<(i32, i32)>),
    }

    pub(crate) fn tiny_dng(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 2) as usize);
        for y in 0..height {
            for x in 0..width {
                let red_side = x < width / 2;
                let v: u16 = match (y % 2, x % 2, red_side) {
                    (0, 0, true) | (1, 1, false) => 50000,
                    (0, 0, false) | (1, 1, true) => 4000,
                    _ => 12000,
                };
                pixels.extend(v.to_le_bytes());
            }
        }
        let srgb_to_xyz_inverse =
            [32404, -15371, -4985, -9692, 18760, 415, 556, -2040, 10572].map(|v| (v, 10000));
        let tags: Vec<(u16, Field)> = vec![
            (254, Field::Long(vec![0])),
            (256, Field::Long(vec![width])),
            (257, Field::Long(vec![height])),
            (258, Field::Short(vec![16])),
            (259, Field::Short(vec![1])),
            (262, Field::Short(vec![32803])),
            (271, Field::Ascii("OmniGet")),
            (272, Field::Ascii("Test")),
            (273, Field::Long(vec![0])),
            (274, Field::Short(vec![orientation])),
            (277, Field::Short(vec![1])),
            (278, Field::Long(vec![height])),
            (279, Field::Long(vec![pixels.len() as u32])),
            (284, Field::Short(vec![1])),
            (33421, Field::Short(vec![2, 2])),
            (33422, Field::Byte(vec![0, 1, 1, 2])),
            (50706, Field::Byte(vec![1, 4, 0, 0])),
            (50708, Field::Ascii("OmniGet Test")),
            (50717, Field::Long(vec![65535])),
            (50721, Field::SRational(srgb_to_xyz_inverse.to_vec())),
            (50728, Field::Rational(vec![(1, 1); 3])),
            (50778, Field::Short(vec![21])),
        ];
        let extra_start = 8 + 2 + tags.len() as u32 * 12 + 4;
        let mut extra: Vec<u8> = Vec::new();
        let mut ifd: Vec<u8> = (tags.len() as u16).to_le_bytes().to_vec();
        let mut strip_offset_at = 0;
        for (tag, field) in &tags {
            let (kind, count, bytes): (u16, usize, Vec<u8>) = match field {
                Field::Short(v) => (3, v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
                Field::Long(v) => (4, v.len(), v.iter().flat_map(|x| x.to_le_bytes()).collect()),
                Field::Byte(v) => (1, v.len(), v.clone()),
                Field::Ascii(s) => (2, s.len() + 1, [s.as_bytes(), &[0]].concat()),
                Field::Rational(v) => (
                    5,
                    v.len(),
                    v.iter()
                        .flat_map(|(a, b)| [a.to_le_bytes(), b.to_le_bytes()].concat())
                        .collect(),
                ),
                Field::SRational(v) => (
                    10,
                    v.len(),
                    v.iter()
                        .flat_map(|(a, b)| [a.to_le_bytes(), b.to_le_bytes()].concat())
                        .collect(),
                ),
            };
            if *tag == 273 {
                strip_offset_at = ifd.len() + 8;
            }
            ifd.extend(tag.to_le_bytes());
            ifd.extend(kind.to_le_bytes());
            ifd.extend((count as u32).to_le_bytes());
            if bytes.len() <= 4 {
                let mut inline = bytes;
                inline.resize(4, 0);
                ifd.extend(inline);
            } else {
                ifd.extend((extra_start + extra.len() as u32).to_le_bytes());
                extra.extend(bytes);
                if extra.len() % 2 == 1 {
                    extra.push(0);
                }
            }
        }
        ifd.extend(0u32.to_le_bytes());
        let data_start = extra_start + extra.len() as u32;
        ifd[strip_offset_at..strip_offset_at + 4].copy_from_slice(&data_start.to_le_bytes());
        [
            b"II*\0".as_slice(),
            &8u32.to_le_bytes(),
            &ifd,
            &extra,
            &pixels,
        ]
        .concat()
    }

    #[test]
    fn develops_colors_and_applies_orientation() {
        let img = decode(&tiny_dng(64, 48, 1)).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (64, 48));
        let red = img.get_pixel(8, 24);
        let blue = img.get_pixel(56, 24);
        assert!(red[0] > red[2] + 80, "{red:?}");
        assert!(blue[2] > blue[0] + 80, "{blue:?}");
        let rotated = decode(&tiny_dng(64, 48, 6)).unwrap().to_rgb8();
        assert_eq!(rotated.dimensions(), (48, 64));
        let top = rotated.get_pixel(24, 8);
        assert!(top[0] > top[2] + 80, "{top:?}");
    }

    #[test]
    fn curve_is_monotonic_and_bounded() {
        let mut last = -1.0;
        for i in 0..=1000 {
            let y = curve(i as f32 / 250.0);
            assert!(y >= last && (0.0..=1.0).contains(&y), "{i}: {y}");
            last = y;
        }
        assert_eq!(curve(0.0), 0.0);
        assert!(curve(100.0) > 0.99);
    }

    #[test]
    fn exposure_lifts_dark_scenes_up_to_a_cap() {
        let dark = ImageBuffer::from_pixel(64, 64, Rgb([2000u16, 2000, 2000]));
        assert!((exposure_gain(&dark) - MAX_GAIN).abs() < 1e-3);
        let gray = ImageBuffer::from_pixel(64, 64, Rgb([8000u16, 8000, 8000]));
        let gain = exposure_gain(&gray);
        assert!(gain > 1.0 && gain < MAX_GAIN, "{gain}");
        let bright = ImageBuffer::from_pixel(64, 64, Rgb([40000u16, 40000, 40000]));
        assert_eq!(exposure_gain(&bright), 1.0);
    }

    #[test]
    fn rejects_files_that_are_not_raw() {
        let err = decode(b"definitely not a raw file").unwrap_err();
        assert!(err.contains("RAW"), "{err}");
    }
}
