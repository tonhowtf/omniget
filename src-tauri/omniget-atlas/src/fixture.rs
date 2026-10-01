//! Synthetic source art: coloured squares with a marked bottom-centre pixel.
//!
//! The tests build their fixture here instead of committing binary art, and the
//! same generator gives anyone a source tree to try `atlas-pack` on without
//! waiting for the real sheets.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use image::{Rgba, RgbaImage};

/// A directory under the system temp dir that deletes itself when dropped.
pub struct TempDir {
    path: PathBuf,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl TempDir {
    pub fn new(tag: &str) -> io::Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "omniget-atlas-{tag}-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A square of `colour` with an opaque marker pixel at the bottom centre, so a
/// test can tell frames apart after packing.
pub fn square(w: u32, h: u32, colour: [u8; 3]) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([colour[0], colour[1], colour[2], 255]));
    img.put_pixel(w / 2, h - 1, Rgba([255, 255, 255, 255]));
    img
}

fn save(img: &RgbaImage, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    img.save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| io::Error::other(e.to_string()))
}

/// Writes `<root>/<sheet>/<anim>/<DIR>/<n>.png` for the five unique directions.
/// Every frame of one animation has the same size, so the default pivot is
/// stable and `atlas-check` passes.
pub fn write_character(
    root: &Path,
    sheet: &str,
    anims: &[(&str, u32)],
    w: u32,
    h: u32,
) -> io::Result<()> {
    for (anim_index, (anim, frames)) in anims.iter().enumerate() {
        for (dir_index, dir) in crate::schema::UNIQUE_DIRS.iter().enumerate() {
            for n in 0..*frames {
                let colour = [
                    (30 + anim_index * 40) as u8,
                    (60 + dir_index * 30) as u8,
                    (90 + n * 10) as u8,
                ];
                save(
                    &square(w, h, colour),
                    &root
                        .join(sheet)
                        .join(anim)
                        .join(dir)
                        .join(format!("{n}.png")),
                )?;
            }
        }
    }
    Ok(())
}

/// Writes a `casa-v1`-shaped tile source: one floor, one wall with height, one
/// object, plus the `tiles.json` that describes them.
pub fn write_tiles(root: &Path) -> io::Result<()> {
    save(
        &square(32, 16, [120, 90, 60]),
        &root.join("floor").join("wood.png"),
    )?;
    save(
        &square(32, 48, [160, 80, 70]),
        &root.join("wall").join("brick.png"),
    )?;
    save(
        &square(24, 32, [70, 110, 140]),
        &root.join("object").join("desk.png"),
    )?;
    std::fs::write(
        root.join("tiles.json"),
        r#"{
  "tiles": {
    "floor/wood": { "height": 0 },
    "wall/brick": { "height": 32, "occludes": true, "walkable": false },
    "object/desk": { "height": 16, "occludes": true, "footprint": [2, 1], "walkable": false }
  }
}
"#,
    )
}

/// A manifest declaring fps and an explicit pivot for one animation.
pub fn write_manifest(root: &Path, body: &str) -> io::Result<()> {
    std::fs::write(root.join("manifest.json"), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dir_creates_and_removes_itself() {
        let path;
        {
            let dir = TempDir::new("selftest").expect("temp dir");
            path = dir.path().to_path_buf();
            assert!(path.is_dir());
        }
        assert!(!path.exists(), "the temp dir must be gone after the drop");
    }

    #[test]
    fn write_character_lays_out_the_expected_tree() {
        let dir = TempDir::new("layout").expect("temp dir");
        write_character(dir.path(), "omni", &[("walk", 3)], 12, 16).expect("write");
        for d in crate::schema::UNIQUE_DIRS {
            assert!(
                dir.path().join("omni/walk").join(d).join("2.png").is_file(),
                "{d}"
            );
        }
        assert!(
            !dir.path().join("omni/walk/E").exists(),
            "mirrored dirs are never written"
        );
    }
}
