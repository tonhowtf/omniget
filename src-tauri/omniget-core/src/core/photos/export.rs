use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{adjust, thumb, Edits};
use crate::core::convert::{self, formats, ConvertOptions};

pub const EXPORT_FORMATS: &[&str] = &["jpeg", "png", "webp", "tiff"];

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub format: String,
    #[serde(default)]
    pub quality: Option<u8>,
    #[serde(default)]
    pub max_size: Option<u32>,
    #[serde(default)]
    pub out_dir: Option<String>,
    #[serde(default = "yes")]
    pub apply_edits: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ExportOutcome {
    pub input: String,
    pub output: Option<String>,
    pub size_bytes: u64,
    pub ok: bool,
    pub error: Option<String>,
}

impl ExportOutcome {
    pub fn failed(input: &str, error: impl Into<String>) -> ExportOutcome {
        ExportOutcome {
            input: input.to_string(),
            output: None,
            size_bytes: 0,
            ok: false,
            error: Some(error.into()),
        }
    }
}

pub fn validate(options: &ExportOptions) -> Result<(), String> {
    if !EXPORT_FORMATS.contains(&options.format.as_str()) {
        return Err(format!(
            "OmniGet can't export photos as {}.",
            options.format
        ));
    }
    if let Some(dir) = options.out_dir.as_deref().filter(|d| !d.trim().is_empty()) {
        if !Path::new(dir).is_dir() {
            return Err("The export folder doesn't exist. Choose another folder.".to_string());
        }
    }
    Ok(())
}

pub fn reserve(
    input: &Path,
    out_dir: &Path,
    format: &str,
    taken: &mut HashSet<PathBuf>,
) -> Result<PathBuf, String> {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "photo".to_string());
    for n in 0..10_000 {
        let candidate = out_dir.join(formats::output_name(&stem, format, n));
        if candidate == input || taken.contains(&candidate) {
            continue;
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => {
                taken.insert(candidate.clone());
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "Could not write to the export folder: {e}. Choose another folder."
                ))
            }
        }
    }
    Err("Could not find a free file name in the export folder.".to_string())
}

pub fn render(
    input: &Path,
    format: &str,
    edits: Option<&Edits>,
    options: &ExportOptions,
) -> Result<Vec<u8>, String> {
    let img = thumb::decode_full(input, format)?;
    let img = match edits.filter(|_| options.apply_edits) {
        Some(e) => adjust::apply(img, e),
        None => img,
    };
    let limit = options.max_size.filter(|v| *v > 0);
    let opts = ConvertOptions {
        quality: options.quality,
        max_width: limit,
        max_height: limit,
        ..Default::default()
    };
    convert::image::encode(img, &options.format, &opts)
}

pub fn render_safely(
    input: &Path,
    format: &str,
    edits: Option<&Edits>,
    options: &ExportOptions,
) -> Result<Vec<u8>, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render(input, format, edits, options)
    }))
    .unwrap_or_else(|_| {
        Err("This photo could not be exported. It may be damaged or use a variant OmniGet can't open yet.".to_string())
    })
}

pub fn save(
    input: &Path,
    bytes: &[u8],
    options: &ExportOptions,
    taken: &mut HashSet<PathBuf>,
) -> ExportOutcome {
    let label = input.to_string_lossy();
    let out_dir = match options.out_dir.as_deref().filter(|d| !d.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => match input.parent() {
            Some(p) => p.to_path_buf(),
            None => return ExportOutcome::failed(&label, "This photo has no folder."),
        },
    };
    let output = match reserve(input, &out_dir, &options.format, taken) {
        Ok(p) => p,
        Err(e) => return ExportOutcome::failed(&label, e),
    };
    if let Err(e) = std::fs::write(&output, bytes) {
        let _ = std::fs::remove_file(&output);
        return ExportOutcome::failed(&label, format!("Could not save the exported photo: {e}."));
    }
    ExportOutcome {
        input: label.into_owned(),
        output: Some(output.to_string_lossy().into_owned()),
        size_bytes: bytes.len() as u64,
        ok: true,
        error: None,
    }
}
