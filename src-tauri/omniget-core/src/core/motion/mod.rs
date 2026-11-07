use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TIMELINE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_RECENT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LottieKind {
    Json,
    Dotlottie,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LottieFile {
    pub path: String,
    pub name: String,
    pub kind: LottieKind,
    pub size_bytes: u64,
    pub animation: String,
    pub animation_id: Option<String>,
    pub animation_count: usize,
    pub manifest: Option<Value>,
    pub embedded_images: usize,
}

#[derive(Debug, Clone)]
pub struct DotLottie {
    pub animation: String,
    pub animation_id: Option<String>,
    pub animation_count: usize,
    pub manifest: Option<Value>,
    pub embedded_images: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedFile {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageImage {
    pub name: String,
    pub mime: String,
    pub data_url: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentTimeline {
    pub id: String,
    pub name: String,
    pub saved_at_ms: u64,
    pub timeline: String,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("Could not open {}: {e}", file_name(path)))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file.", file_name(path)));
    }
    if meta.len() == 0 {
        return Err(format!("{} is empty (0 bytes).", file_name(path)));
    }
    if meta.len() > limit {
        return Err(format!(
            "{} is {} MB; the limit is {} MB.",
            file_name(path),
            meta.len() / 1024 / 1024,
            limit / 1024 / 1024
        ));
    }
    std::fs::read(path).map_err(|e| format!("Could not read {}: {e}", file_name(path)))
}

fn check_lottie_json(text: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("The file is not valid JSON ({e})."))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "The JSON is not a Lottie animation (no top-level object).".to_string())?;
    let has = |k: &str| obj.contains_key(k);
    if !(has("layers") && has("w") && has("h") && has("fr")) {
        return Err("The JSON is not a Lottie animation (missing layers, w, h or fr).".into());
    }
    Ok(value)
}

fn image_mime(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "image/png"
    }
}

fn zip_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    budget: &mut u64,
) -> Result<Option<Vec<u8>>, String> {
    let mut entry = match zip.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(format!("Could not read {name} inside the .lottie: {e}")),
    };
    if entry.size() > *budget {
        return Err("The .lottie unpacks to more than 64 MB; it is too large to open.".into());
    }
    let mut buf = Vec::with_capacity(entry.size() as usize);
    (&mut entry)
        .take(*budget + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Could not unpack {name}: {e}"))?;
    if buf.len() as u64 > *budget {
        return Err("The .lottie unpacks to more than 64 MB; it is too large to open.".into());
    }
    *budget -= buf.len() as u64;
    Ok(Some(buf))
}

fn embed_images<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    animation: &mut Value,
    budget: &mut u64,
) -> Result<usize, String> {
    let Some(assets) = animation.get_mut("assets").and_then(Value::as_array_mut) else {
        return Ok(0);
    };
    let mut count = 0;
    for asset in assets.iter_mut() {
        let Some(obj) = asset.as_object_mut() else {
            continue;
        };
        if obj.contains_key("layers") {
            continue;
        }
        let Some(p) = obj.get("p").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if p.starts_with("data:") {
            continue;
        }
        let base = p.rsplit('/').next().unwrap_or(&p).to_string();
        if base.is_empty() || base.contains("..") {
            continue;
        }
        let mut found = None;
        for dir in ["images/", "i/"] {
            if let Some(bytes) = zip_entry(zip, &format!("{dir}{base}"), budget)? {
                found = Some(bytes);
                break;
            }
        }
        if let Some(bytes) = found {
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            obj.insert(
                "p".into(),
                Value::String(format!("data:{};base64,{data}", image_mime(&base))),
            );
            obj.insert("u".into(), Value::String(String::new()));
            obj.insert("e".into(), Value::from(1));
            count += 1;
        }
    }
    Ok(count)
}

pub fn read_dotlottie_bytes(bytes: &[u8]) -> Result<DotLottie, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("The .lottie is not a valid zip archive ({e})."))?;
    let mut budget = MAX_INPUT_BYTES;
    let manifest: Option<Value> = match zip_entry(&mut zip, "manifest.json", &mut budget)? {
        Some(buf) => serde_json::from_slice(&buf).ok(),
        None => None,
    };
    let manifest_ids: Vec<String> = manifest
        .as_ref()
        .and_then(|m| m.get("animations"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| a.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut entries: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.name_for_index(i).map(str::to_string))
        .filter(|n| {
            (n.starts_with("animations/") || n.starts_with("a/"))
                && n.to_ascii_lowercase().ends_with(".json")
        })
        .collect();
    entries.sort();
    let count = entries.len().max(manifest_ids.len());
    let mut candidates: Vec<(Option<String>, String)> = Vec::new();
    for id in &manifest_ids {
        if id.contains("..") || id.contains('/') {
            continue;
        }
        candidates.push((Some(id.clone()), format!("animations/{id}.json")));
        candidates.push((Some(id.clone()), format!("a/{id}.json")));
    }
    for name in &entries {
        let stem = name
            .rsplit('/')
            .next()
            .and_then(|f| f.strip_suffix(".json"))
            .map(str::to_string);
        candidates.push((stem, name.clone()));
    }
    let mut first_error: Option<String> = None;
    for (id, name) in candidates {
        let Some(buf) = zip_entry(&mut zip, &name, &mut budget)? else {
            continue;
        };
        let parsed = String::from_utf8(buf)
            .map_err(|_| format!("{name} inside the .lottie is not UTF-8 text."))
            .and_then(|text| check_lottie_json(&text).map(|animation| (text, animation)));
        let (text, mut animation) = match parsed {
            Ok(ok) => ok,
            Err(e) => {
                first_error.get_or_insert(e);
                continue;
            }
        };
        let embedded = embed_images(&mut zip, &mut animation, &mut budget)?;
        let out = if embedded > 0 {
            serde_json::to_string(&animation).map_err(|e| e.to_string())?
        } else {
            text
        };
        return Ok(DotLottie {
            animation: out,
            animation_id: id,
            animation_count: count,
            manifest,
            embedded_images: embedded,
        });
    }
    Err(first_error
        .unwrap_or_else(|| "The .lottie has no animation inside (no animations/*.json).".into()))
}

pub fn read_lottie(path: &Path) -> Result<LottieFile, String> {
    let bytes = read_limited(path, MAX_INPUT_BYTES)?;
    let size_bytes = bytes.len() as u64;
    let is_zip = bytes.starts_with(b"PK\x03\x04");
    let ext = extension(path);
    if is_zip || ext == "lottie" {
        if !is_zip {
            return Err("The .lottie is not a zip archive; it may be a renamed .json.".into());
        }
        let DotLottie {
            animation,
            animation_id,
            animation_count,
            manifest,
            embedded_images,
        } = read_dotlottie_bytes(&bytes)?;
        return Ok(LottieFile {
            path: path.to_string_lossy().into_owned(),
            name: file_name(path),
            kind: LottieKind::Dotlottie,
            size_bytes,
            animation,
            animation_id,
            animation_count,
            manifest,
            embedded_images,
        });
    }
    let text = String::from_utf8(bytes).map_err(|_| "The file is not UTF-8 text.".to_string())?;
    let text = text.trim_start_matches('\u{feff}').to_string();
    check_lottie_json(&text)?;
    Ok(LottieFile {
        path: path.to_string_lossy().into_owned(),
        name: file_name(path),
        kind: LottieKind::Json,
        size_bytes,
        animation: text,
        animation_id: None,
        animation_count: 1,
        manifest: None,
        embedded_images: 0,
    })
}

fn target_path(path: &str, allowed: &[&str]) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("No file was chosen.".into());
    }
    let p = PathBuf::from(trimmed);
    if !p.is_absolute() {
        return Err("The save location must be a full path.".into());
    }
    let ext = extension(&p);
    if !allowed.contains(&ext.as_str()) {
        return Err(format!("The file name must end in .{}.", allowed[0]));
    }
    match p.parent() {
        Some(parent) if parent.is_dir() => Ok(p),
        _ => Err("The folder for this file does not exist.".into()),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<SavedFile, String> {
    let tmp = path.with_extension(format!("{}.part", extension(path)));
    std::fs::write(&tmp, bytes).map_err(|e| format!("Could not write {}: {e}", file_name(path)))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Could not save {}: {e}", file_name(path)));
    }
    Ok(SavedFile {
        path: path.to_string_lossy().into_owned(),
        size_bytes: bytes.len() as u64,
    })
}

pub fn save_png(path: &str, data_base64: &str) -> Result<SavedFile, String> {
    let target = target_path(path, &["png"])?;
    let raw = data_base64
        .split_once(',')
        .filter(|(head, _)| head.starts_with("data:"))
        .map(|(_, body)| body)
        .unwrap_or(data_base64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|_| "The image data was damaged before saving.".to_string())?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The image data is not a PNG.".into());
    }
    write_atomic(&target, &bytes)
}

pub fn validate_timeline(json: &str) -> Result<Value, String> {
    if json.len() as u64 > MAX_TIMELINE_BYTES {
        return Err("The timeline is larger than 8 MB.".into());
    }
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("The timeline is not valid JSON ({e})."))?;
    let ok = value.get("version").and_then(Value::as_u64) == Some(1)
        && value.get("tracks").is_some_and(Value::is_object);
    if !ok {
        return Err("This JSON is not an OmniGet Motion timeline (version 1 with tracks).".into());
    }
    Ok(value)
}

pub fn save_timeline(path: &str, json: &str) -> Result<SavedFile, String> {
    validate_timeline(json)?;
    let target = target_path(path, &["json"])?;
    write_atomic(&target, json.as_bytes())
}

pub fn load_timeline(path: &Path) -> Result<String, String> {
    let bytes = read_limited(path, MAX_TIMELINE_BYTES)?;
    let text = String::from_utf8(bytes).map_err(|_| "The file is not UTF-8 text.".to_string())?;
    validate_timeline(&text)?;
    Ok(text)
}

pub fn read_image(path: &Path) -> Result<StageImage, String> {
    let ext = extension(path);
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => return Err("Pick a PNG, JPEG, GIF, WebP or SVG image.".into()),
    };
    let bytes = read_limited(path, MAX_IMAGE_BYTES)?;
    let size_bytes = bytes.len() as u64;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(StageImage {
        name: file_name(path),
        mime: mime.into(),
        data_url: format!("data:{mime};base64,{data}"),
        size_bytes,
    })
}

pub fn recent_file(dir: &Path) -> PathBuf {
    dir.join("motion").join("recent.json")
}

pub fn recent_list(dir: &Path) -> Result<Vec<RecentTimeline>, String> {
    let bytes = match std::fs::read(recent_file(dir)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Could not read the Recent list: {e}")),
    };
    serde_json::from_slice::<Vec<RecentTimeline>>(&bytes).map_err(|_| {
        "The Recent list file is damaged. Saving to Recent again keeps a copy of it and starts a new list."
            .to_string()
    })
}

fn recent_list_or_backup(dir: &Path, now_ms: u64) -> Result<Vec<RecentTimeline>, String> {
    match recent_list(dir) {
        Ok(list) => Ok(list),
        Err(_) if recent_file(dir).is_file() => {
            let file = recent_file(dir);
            let backup = file.with_file_name(format!("recent.damaged-{now_ms:x}.json"));
            std::fs::rename(&file, &backup)
                .map_err(|e| format!("Could not set aside the damaged Recent list: {e}"))?;
            Ok(Vec::new())
        }
        Err(e) => Err(e),
    }
}

fn write_recent(dir: &Path, list: &[RecentTimeline]) -> Result<(), String> {
    let file = recent_file(dir);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create the app data folder: {e}"))?;
    }
    let body = serde_json::to_vec(list).map_err(|e| e.to_string())?;
    write_atomic(&file, &body).map(|_| ())
}

pub fn recent_save(
    dir: &Path,
    id: Option<&str>,
    name: &str,
    json: &str,
    now_ms: u64,
) -> Result<Vec<RecentTimeline>, String> {
    validate_timeline(json)?;
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the timeline a name before keeping it in Recent.".into());
    }
    let name: String = name.chars().take(80).collect();
    let mut list = recent_list_or_backup(dir, now_ms)?;
    let id = id
        .filter(|id| list.iter().any(|r| r.id == *id))
        .map(str::to_string)
        .unwrap_or_else(|| {
            let mut n = now_ms;
            while list.iter().any(|r| r.id == format!("{n:x}")) {
                n += 1;
            }
            format!("{n:x}")
        });
    list.retain(|r| r.id != id);
    list.insert(
        0,
        RecentTimeline {
            id,
            name,
            saved_at_ms: now_ms,
            timeline: json.to_string(),
        },
    );
    list.truncate(MAX_RECENT);
    write_recent(dir, &list)?;
    Ok(list)
}

pub fn recent_remove(dir: &Path, id: &str) -> Result<Vec<RecentTimeline>, String> {
    let mut list = recent_list(dir)?;
    list.retain(|r| r.id != id);
    write_recent(dir, &list)?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const ANIM: &str = r#"{"v":"5.7.0","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[],"assets":[{"id":"img_0","w":10,"h":10,"u":"/images/","p":"img_0.png","e":0}]}"#;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-motion-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn dotlottie(manifest: Option<&str>, anim_path: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            if let Some(m) = manifest {
                w.start_file("manifest.json", opts).unwrap();
                w.write_all(m.as_bytes()).unwrap();
            }
            w.start_file(anim_path, opts).unwrap();
            w.write_all(ANIM.as_bytes()).unwrap();
            w.start_file("images/img_0.png", opts).unwrap();
            w.write_all(b"\x89PNG\r\n\x1a\nfake").unwrap();
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn reads_plain_json() {
        let dir = tmp();
        let p = dir.join("a.json");
        std::fs::write(&p, ANIM).unwrap();
        let f = read_lottie(&p).unwrap();
        assert_eq!(f.kind, LottieKind::Json);
        assert_eq!(f.animation, ANIM);
        let bad = dir.join("b.json");
        std::fs::write(&bad, r#"{"hello":1}"#).unwrap();
        assert!(read_lottie(&bad).unwrap_err().contains("not a Lottie"));
        let empty = dir.join("c.json");
        std::fs::write(&empty, "").unwrap();
        assert!(read_lottie(&empty).unwrap_err().contains("empty"));
    }

    #[test]
    fn unzips_dotlottie_with_manifest_and_images() {
        let bytes = dotlottie(
            Some(r#"{"version":"1","animations":[{"id":"intro"}]}"#),
            "animations/intro.json",
        );
        let d = read_dotlottie_bytes(&bytes).unwrap();
        assert_eq!(d.animation_id.as_deref(), Some("intro"));
        assert_eq!(d.animation_count, 1);
        assert!(d.manifest.is_some());
        assert_eq!(d.embedded_images, 1);
        let v: Value = serde_json::from_str(&d.animation).unwrap();
        let p = v["assets"][0]["p"].as_str().unwrap();
        assert!(p.starts_with("data:image/png;base64,"));
        assert_eq!(v["assets"][0]["e"], 1);
    }

    #[test]
    fn unzips_v2_layout_without_manifest() {
        let bytes = dotlottie(None, "a/main.json");
        let d = read_dotlottie_bytes(&bytes).unwrap();
        assert_eq!(d.animation_id.as_deref(), Some("main"));
        assert!(d.manifest.is_none());
        assert!(read_dotlottie_bytes(b"not a zip").is_err());
    }

    #[test]
    fn saves_png_only_to_valid_targets() {
        let dir = tmp();
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nrest");
        let target = dir.join("f.png");
        let saved = save_png(
            target.to_str().unwrap(),
            &format!("data:image/png;base64,{png}"),
        )
        .unwrap();
        assert_eq!(saved.size_bytes, 12);
        assert!(save_png(dir.join("f.exe").to_str().unwrap(), &png).is_err());
        assert!(save_png("relative.png", &png).is_err());
        assert!(save_png(dir.join("missing/f.png").to_str().unwrap(), &png).is_err());
        let not_png = base64::engine::general_purpose::STANDARD.encode(b"GIF89a");
        assert!(save_png(target.to_str().unwrap(), &not_png).is_err());
    }

    #[test]
    fn timeline_round_trip_and_recent() {
        let dir = tmp();
        let json = r#"{"version":1,"name":"A","duration":1000,"tracks":{}}"#;
        let file = dir.join("t.json");
        save_timeline(file.to_str().unwrap(), json).unwrap();
        assert_eq!(load_timeline(&file).unwrap(), json);
        assert!(save_timeline(file.to_str().unwrap(), r#"{"version":2}"#).is_err());
        assert!(recent_list(&dir).unwrap().is_empty());
        assert!(recent_save(&dir, None, "  ", json, 1).is_err());
        let list = recent_save(&dir, None, "A", json, 1).unwrap();
        assert_eq!(list.len(), 1);
        let id = list[0].id.clone();
        let list = recent_save(&dir, Some(&id), "A2", json, 2).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].saved_at_ms, 2);
        assert_eq!(list[0].name, "A2");
        let list = recent_save(&dir, None, "A2", json, 2).unwrap();
        assert_eq!(list.len(), 2);
        assert_ne!(list[0].id, list[1].id);
        let list = recent_save(&dir, Some("gone"), "B", json, 3).unwrap();
        assert_eq!(list.len(), 3);
        for i in 0..20 {
            recent_save(&dir, None, &format!("n{i}"), json, 10 + i).unwrap();
        }
        assert_eq!(recent_list(&dir).unwrap().len(), MAX_RECENT);
        let first = recent_list(&dir).unwrap()[0].id.clone();
        assert_eq!(recent_remove(&dir, &first).unwrap().len(), MAX_RECENT - 1);
    }

    #[test]
    fn damaged_recent_is_reported_and_kept_aside() {
        let dir = tmp();
        let json = r#"{"version":1,"name":"A","duration":1000,"tracks":{}}"#;
        std::fs::create_dir_all(recent_file(&dir).parent().unwrap()).unwrap();
        std::fs::write(recent_file(&dir), b"{not json").unwrap();
        assert!(recent_list(&dir).unwrap_err().contains("damaged"));
        assert!(recent_remove(&dir, "x").is_err());
        let list = recent_save(&dir, None, "A", json, 0xabc).unwrap();
        assert_eq!(list.len(), 1);
        let backup = recent_file(&dir).with_file_name("recent.damaged-abc.json");
        assert_eq!(std::fs::read(backup).unwrap(), b"{not json");
    }

    #[test]
    fn dotlottie_skips_a_broken_first_candidate() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file("manifest.json", opts).unwrap();
            w.write_all(br#"{"animations":[{"id":"stale"}]}"#).unwrap();
            w.start_file("animations/stale.json", opts).unwrap();
            w.write_all(br#"{"hello":1}"#).unwrap();
            w.start_file("animations/zz.json", opts).unwrap();
            w.write_all(ANIM.as_bytes()).unwrap();
            w.finish().unwrap();
        }
        let d = read_dotlottie_bytes(&buf.into_inner()).unwrap();
        assert_eq!(d.animation_id.as_deref(), Some("zz"));

        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            w.start_file("animations/only.json", opts).unwrap();
            w.write_all(br#"{"hello":1}"#).unwrap();
            w.finish().unwrap();
        }
        assert!(read_dotlottie_bytes(&buf.into_inner())
            .unwrap_err()
            .contains("not a Lottie"));
    }

    #[test]
    fn reads_stage_images() {
        let dir = tmp();
        let p = dir.join("s.svg");
        std::fs::write(&p, "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        let img = read_image(&p).unwrap();
        assert!(img.data_url.starts_with("data:image/svg+xml;base64,"));
        assert!(read_image(&dir.join("x.txt")).is_err());
    }
}
