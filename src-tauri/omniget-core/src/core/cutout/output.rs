use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::core::convert::path_key;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[default]
    Png,
    Webp,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Webp => "webp",
        }
    }
}

pub fn encode(img: &RgbaImage, format: Format) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    let (w, h) = img.dimensions();
    let result = match format {
        Format::Png => {
            PngEncoder::new_with_quality(&mut out, CompressionType::Default, PngFilter::Adaptive)
                .write_image(img.as_raw(), w, h, ExtendedColorType::Rgba8)
        }
        Format::Webp => WebPEncoder::new_lossless(&mut out).write_image(
            img.as_raw(),
            w,
            h,
            ExtendedColorType::Rgba8,
        ),
    };
    result.map_err(|e| format!("The picture could not be saved: {e}."))?;
    Ok(out.into_inner())
}

pub async fn save(output_path: &Path, bytes: &[u8]) -> Result<u64, String> {
    if let Some(dir) = output_path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| format!("The folder could not be created: {e}."))?;
    }
    let name = output_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part = output_path.with_file_name(format!(".{name}.part"));
    let written = match tokio::fs::write(&part, bytes).await {
        Ok(()) => tokio::fs::rename(&part, output_path).await,
        Err(e) => Err(e),
    };
    if let Err(e) = written {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(format!("The file could not be saved: {e}."));
    }
    Ok(bytes.len() as u64)
}

fn moment(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else {
        format!("{m:02}m{s:02}s")
    }
}

pub fn plan(
    items: &[(PathBuf, Option<u64>)],
    out_dir: Option<&Path>,
    format: Format,
    overwrite: bool,
) -> Vec<PathBuf> {
    let inputs: HashSet<PathBuf> = items.iter().map(|(p, _)| path_key(p)).collect();
    let mut taken: HashSet<PathBuf> = HashSet::new();
    let ext = format.extension();
    items
        .iter()
        .map(|(input, frame)| {
            let dir = out_dir
                .map(Path::to_path_buf)
                .or_else(|| input.parent().map(Path::to_path_buf))
                .unwrap_or_default();
            let stem = input
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "picture".to_string());
            let base = match frame {
                Some(ms) => format!("{stem}-{}-cutout", moment(*ms)),
                None => format!("{stem}-cutout"),
            };
            let mut n = 0u32;
            loop {
                let name = if n == 0 {
                    format!("{base}.{ext}")
                } else {
                    format!("{base} ({n}).{ext}")
                };
                let candidate = dir.join(name);
                let key = path_key(&candidate);
                if !inputs.contains(&key)
                    && !taken.contains(&key)
                    && (overwrite || !candidate.exists())
                {
                    taken.insert(key);
                    return candidate;
                }
                n += 1;
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn names_never_collide_and_frames_carry_the_moment() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("cat.jpg");
        std::fs::write(dir.path().join("cat-cutout.png"), b"x").unwrap();
        let items = vec![
            (a.clone(), None),
            (dir.path().join("cat.png"), None),
            (dir.path().join("clip.mp4"), Some(83_500)),
            (dir.path().join("clip.mp4"), Some(3_723_000)),
        ];
        let out = plan(&items, None, Format::Png, false);
        let names: Vec<String> = out
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "cat-cutout (1).png",
                "cat-cutout (2).png",
                "clip-01m23s-cutout.png",
                "clip-1h02m03s-cutout.png",
            ]
        );
        let replaced = plan(&items[..1], None, Format::Webp, true);
        assert_eq!(replaced[0], dir.path().join("cat-cutout.webp"));
        let elsewhere = tempfile::tempdir().unwrap();
        let moved = plan(&items[..1], Some(elsewhere.path()), Format::Png, false);
        assert_eq!(moved[0], elsewhere.path().join("cat-cutout.png"));
    }

    #[tokio::test]
    async fn saving_leaves_no_partial_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("pasta com espaço").join("gato-cutout.png");
        assert_eq!(save(&dest, b"png").await.unwrap(), 3);
        assert_eq!(std::fs::read(&dest).unwrap(), b"png");
        let blocked = dir.path().join("taken");
        std::fs::create_dir(&blocked).unwrap();
        assert!(save(&blocked, b"png").await.is_err());
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".part")), "{names:?}");
    }

    #[test]
    fn both_formats_keep_transparency() {
        let mut img = RgbaImage::from_pixel(5, 5, Rgba([10, 20, 30, 0]));
        img.put_pixel(2, 2, Rgba([200, 100, 50, 255]));
        for format in [Format::Png, Format::Webp] {
            let bytes = encode(&img, format).unwrap();
            let back = image::load_from_memory(&bytes).unwrap().to_rgba8();
            assert_eq!(back.get_pixel(2, 2).0, [200, 100, 50, 255]);
            assert_eq!(back.get_pixel(0, 0).0[3], 0);
        }
    }
}
