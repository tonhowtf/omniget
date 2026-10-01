use crate::mcp::{
    artifacts,
    policy::{self, Principal},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    artifact_ids: Vec<String>,
    storyboard: String,
}
pub fn export(p: &Principal, args: Value) -> Result<Value, String> {
    let options: Options = serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT")?;
    if options.artifact_ids.is_empty()
        || options.artifact_ids.len() > 20
        || options.storyboard.len() > 20000
    {
        return Err("INVALID_ARGUMENT: bundle limits".into());
    }
    let root = super::native_agent_media::directory(p)?;
    let path = root.join(format!("{}.zip", uuid::Uuid::new_v4()));
    let result = (|| {
        let file = std::fs::File::create(&path).map_err(|_| "EXPORT_FAILED")?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut manifest = Vec::new();
        let mut total = 0u64;
        let mut thumbs = Vec::new();
        for id in options.artifact_ids {
            let a = artifacts::describe(p, &id)?;
            total = total.saturating_add(a.bytes);
            if total > 256 * 1024 * 1024 {
                return Err("EXPORT_FAILED: 256 MiB limit".into());
            }
            let extension = std::path::Path::new(&a.name)
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("");
            if !matches!(extension, "png" | "mp4") {
                return Err("INVALID_ARGUMENT: only PNG and MP4".into());
            }
            let mut snapshot = artifacts::open(p, &id, &a.digest)?;
            let sidecar = root.join(std::path::Path::new(&a.name).with_extension("json"));
            let metadata = std::fs::read(sidecar)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let name = format!(
                "{}/{}.{}",
                if extension == "png" {
                    if metadata.as_ref().is_some_and(|m| !m["crop"].is_null()) {
                        "crops"
                    } else {
                        "screens"
                    }
                } else {
                    "clips"
                },
                id,
                extension
            );
            zip.start_file(&name, opts).map_err(|_| "EXPORT_FAILED")?;
            if extension == "png" {
                let mut bytes = Vec::new();
                snapshot
                    .file
                    .take(32 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "EXPORT_FAILED")?;
                let img =
                    image::load_from_memory(&bytes).map_err(|_| "EXPORT_FAILED: invalid PNG")?;
                thumbs.push(img.thumbnail(320, 200).to_rgba8());
                zip.write_all(&bytes).map_err(|_| "EXPORT_FAILED")?;
            } else {
                std::io::copy(&mut snapshot.file, &mut zip).map_err(|_| "EXPORT_FAILED")?;
            }
            manifest.push(json!({"path":name,"artifact":a,"captureMetadata":metadata}));
        }
        for (name, bytes) in [
            ("brand/favicon.png", include_bytes!("../../static/favicon.png").as_slice()),
            ("brand/workspace-design.css", include_bytes!("../../src/lib/style/workspace-design.css").as_slice()),
            ("brand/REPOSITORY-LICENSE", include_bytes!("../../LICENSE").as_slice()),
            ("brand/sources.md", b"# Sources\n\nUnmodified static/favicon.png and src/lib/style/workspace-design.css from this Omniget checkout. Repository LICENSE included as source reference; this is not an additional trademark license.\n".as_slice()),
        ] { zip.start_file(name,opts).map_err(|_|"EXPORT_FAILED")?; zip.write_all(bytes).map_err(|_|"EXPORT_FAILED")?; }
        zip.start_file("manifest.json", opts)
            .map_err(|_| "EXPORT_FAILED")?;
        let metadata = json!({"schemaVersion":1,"appVersion":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"observedAt":chrono::Utc::now().to_rfc3339(),"files":manifest,"contactSheetOrder":"PNG entries in manifest order","warnings":["No external upload performed","Desktop captures are not mobile layouts"]});
        zip.write_all(&serde_json::to_vec_pretty(&metadata).map_err(|_| "EXPORT_FAILED")?)
            .map_err(|_| "EXPORT_FAILED")?;
        zip.start_file("storyboard.md", opts)
            .map_err(|_| "EXPORT_FAILED")?;
        zip.write_all(options.storyboard.as_bytes())
            .map_err(|_| "EXPORT_FAILED")?;
        if !thumbs.is_empty() {
            let mut sheet = image::RgbaImage::new(640, 200 * (thumbs.len() as u32).div_ceil(2));
            for (i, img) in thumbs.iter().enumerate() {
                image::imageops::overlay(
                    &mut sheet,
                    img,
                    (i % 2 * 320) as i64,
                    (i / 2 * 200) as i64,
                );
            }
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(sheet)
                .write_to(&mut png, image::ImageFormat::Png)
                .map_err(|_| "EXPORT_FAILED")?;
            zip.start_file("contact-sheet.png", opts)
                .map_err(|_| "EXPORT_FAILED")?;
            zip.write_all(&png.into_inner())
                .map_err(|_| "EXPORT_FAILED")?;
        }
        zip.finish().map_err(|_| "EXPORT_FAILED")?;
        policy::active(p)?;
        artifacts::grant_root(&p.id, &root)?;
        Ok(json!({"schemaVersion":1,"artifact":artifacts::register(p,&path)?,"manifest":metadata}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}
