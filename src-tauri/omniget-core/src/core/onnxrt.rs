use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Context};
use serde::Serialize;

pub const RUNTIME_VERSION: &str = "1.27.1";

const MIRROR_BASE: &str = "https://getomniget.com/dl/onnxruntime";
const RELEASE_BASE: &str = "https://github.com/microsoft/onnxruntime/releases/download";

struct Asset {
    id: &'static str,
    label: &'static str,
    file: &'static str,
    mirror_sha256: &'static str,
    mirror_bytes: u64,
    upstream_sha256: &'static str,
    os: &'static str,
}

struct DownloadSource {
    url: String,
    sha256: &'static str,
}

const ASSETS: &[Asset] = &[
    Asset {
        id: "osx-arm64",
        label: "macOS ARM64 (Apple Silicon)",
        file: "onnxruntime-osx-arm64-1.27.1.tgz",
        mirror_sha256: "5b7618eaa445f974c9076df91721af63fbae511bbe406d993421ced15591a37c",
        mirror_bytes: 10_595_517,
        upstream_sha256: "e42b77a7281cc6e55141bf44fcfbac2c782b823a491bbb6ac33c781dd991f8a6",
        os: "macos",
    },
    Asset {
        id: "linux-x64",
        label: "Linux x64",
        file: "onnxruntime-linux-x64-1.27.1.tgz",
        mirror_sha256: "8f55fffbde8b432883fa64668a29123d249cfc55506c326c52fbb0bcf8340f16",
        mirror_bytes: 8_605_381,
        upstream_sha256: "25b1ef1fea1acd210d63f8f24dc870ad6e077795ce1f54876252c6d3803c15af",
        os: "linux",
    },
    Asset {
        id: "linux-aarch64",
        label: "Linux ARM64",
        file: "onnxruntime-linux-aarch64-1.27.1.tgz",
        mirror_sha256: "e91f7b10552173fbe603d1a69f40d88d3c44bb66f4db7c0485c019519054e789",
        mirror_bytes: 7_592_739,
        upstream_sha256: "33c67e33d1e25b816878366ea276589a024f71f000e7ff955c4b33224d639edd",
        os: "linux",
    },
    Asset {
        id: "win-x64",
        label: "Windows x64",
        file: "onnxruntime-win-x64-1.27.1.zip",
        mirror_sha256: "0e313c04bd0f834f623f82ebdf1b0b261bdbdf69d483ee3d87fc2c39e4232151",
        mirror_bytes: 5_627_419,
        upstream_sha256: "2e00414a63fdef0914cd5a5ede6c707844878e0c08e1b6693842f0451b2df2a1",
        os: "windows",
    },
    Asset {
        id: "win-arm64",
        label: "Windows ARM64",
        file: "onnxruntime-win-arm64-1.27.1.zip",
        mirror_sha256: "4938d3892a6845761e76262ede9be695fd7372fe2131b3c4c47bf3998a773b89",
        mirror_bytes: 5_543_948,
        upstream_sha256: "6e22c2061ba6400b42a59663d700c8694e4e8fe654cf452c4700c24237407ae1",
        os: "windows",
    },
];

fn sources_for(asset: &Asset) -> [DownloadSource; 2] {
    [
        DownloadSource {
            url: format!("{MIRROR_BASE}/v{RUNTIME_VERSION}/{}", asset.file),
            sha256: asset.mirror_sha256,
        },
        DownloadSource {
            url: format!("{RELEASE_BASE}/v{RUNTIME_VERSION}/{}", asset.file),
            sha256: asset.upstream_sha256,
        },
    ]
}

pub fn lib_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

fn current_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

pub fn auto_variant_id() -> Option<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("win-x64");
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return Some("win-arm64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("osx-arm64");
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x64");
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Some("linux-aarch64");
    }
    #[allow(unreachable_code)]
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeVariant {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub recommended: bool,
}

pub fn list_variants() -> Vec<RuntimeVariant> {
    let os = current_os();
    let auto = auto_variant_id();
    ASSETS
        .iter()
        .filter(|a| a.os == os)
        .map(|a| RuntimeVariant {
            id: a.id.to_string(),
            label: a.label.to_string(),
            bytes: a.mirror_bytes,
            recommended: Some(a.id) == auto,
        })
        .collect()
}

fn asset_by_id(id: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.id == id)
}

fn pick_asset(variant: Option<&str>) -> anyhow::Result<&'static Asset> {
    let wanted = variant
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "auto");
    if let Some(id) = wanted {
        return asset_by_id(id).ok_or_else(|| anyhow!("unknown ONNX Runtime variant: {id}"));
    }
    let auto = auto_variant_id().ok_or_else(|| {
        anyhow!(
            "Microsoft does not publish a ready ONNX Runtime for this system; \
             install it from a local file (the pip package `onnxruntime`, for example)"
        )
    })?;
    asset_by_id(auto).ok_or_else(|| anyhow!("variant {auto} is missing from the catalog"))
}

pub fn target_dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("onnxruntime"))
}

pub fn target_path() -> Option<PathBuf> {
    target_dir().map(|d| d.join(lib_filename()))
}

pub fn version_marker_path() -> Option<PathBuf> {
    target_dir().map(|d| d.join("onnxruntime.version"))
}

pub fn resolve_with_source() -> Option<(PathBuf, &'static str)> {
    if let Ok(raw) = std::env::var("ORT_DYLIB_PATH") {
        let p = PathBuf::from(raw.trim());
        if p.is_file() {
            return Some((p, "env"));
        }
    }
    let managed = target_path()?;
    if managed.is_file() {
        return Some((managed, "managed"));
    }
    None
}

pub fn resolve_path() -> Option<PathBuf> {
    resolve_with_source().map(|(p, _)| p)
}

pub fn is_installed() -> bool {
    resolve_with_source().is_some()
}

pub fn read_version_marker() -> Option<String> {
    let p = version_marker_path()?;
    let s = std::fs::read_to_string(&p).ok()?;
    let t = s.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub source: Option<String>,
    pub version: Option<String>,
    pub target_version: String,
    pub lib_filename: String,
    pub can_download: bool,
    pub variants: Vec<RuntimeVariant>,
}

pub fn status() -> RuntimeStatus {
    let found = resolve_with_source();
    RuntimeStatus {
        installed: found.is_some(),
        path: found.as_ref().map(|(p, _)| p.to_string_lossy().to_string()),
        source: found.as_ref().map(|(_, s)| (*s).to_string()),
        version: read_version_marker(),
        target_version: RUNTIME_VERSION.to_string(),
        lib_filename: lib_filename().to_string(),
        can_download: auto_variant_id().is_some(),
        variants: list_variants(),
    }
}

fn missing_runtime_error() -> anyhow::Error {
    let where_to = target_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "<app data folder>".into());
    if auto_variant_id().is_some() {
        anyhow!(
            "ONNX Runtime {} is not installed yet. Install it from the Models screen \
             (the download goes to {}) or point to a {} you already have.",
            RUNTIME_VERSION,
            where_to,
            lib_filename()
        )
    } else {
        anyhow!(
            "there is no official ONNX Runtime build for this system. \
             Point to a {} you already have (the pip package `onnxruntime` ships one); \
             it is copied to {}.",
            lib_filename(),
            where_to
        )
    }
}

static READY: OnceLock<PathBuf> = OnceLock::new();

pub fn init() -> anyhow::Result<PathBuf> {
    if let Some(p) = READY.get() {
        return Ok(p.clone());
    }
    let path = resolve_path().ok_or_else(missing_runtime_error)?;
    ort::init_from(&path)
        .map_err(|e| anyhow!("could not load {}: {}", path.display(), e))?
        .with_name("omniget")
        .commit();
    let _ = READY.set(path.clone());
    Ok(path)
}

fn sha256_of(path: &Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening {} to check its sha256", path.display()))?;
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

pub fn is_runtime_lib(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    if cfg!(target_os = "windows") {
        name.ends_with(".dll")
    } else if cfg!(target_os = "macos") {
        name.starts_with("libonnxruntime") && name.ends_with(".dylib")
    } else {
        name.starts_with("libonnxruntime.so")
    }
}

fn is_lib_entry(path: &Path) -> bool {
    let mut has_lib = false;
    for comp in path.components() {
        let s = comp.as_os_str().to_string_lossy();
        if s == "dSYM" || s.ends_with(".dSYM") {
            return false;
        }
        if s == "lib" {
            has_lib = true;
        }
    }
    if !has_lib {
        return false;
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .map(is_runtime_lib)
        .unwrap_or(false)
}

fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let dest = dir.join(name);
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
    }
    std::fs::rename(&tmp, &dest).with_context(|| format!("moving to {}", dest.display()))?;
    Ok(dest)
}

fn extract_libs(archive: &Path, is_zip: bool, dir: &Path) -> anyhow::Result<String> {
    let mut biggest: Option<(String, u64)> = None;
    let mut note = |name: &str, len: u64| {
        if biggest.as_ref().map(|(_, b)| len > *b).unwrap_or(true) {
            biggest = Some((name.to_string(), len));
        }
    };

    if is_zip {
        let file = std::fs::File::open(archive)
            .with_context(|| format!("opening {}", archive.display()))?;
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            if !entry.is_file() {
                continue;
            }
            let Some(path) = entry.enclosed_name() else {
                continue;
            };
            if !is_lib_entry(&path) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            write_atomic(dir, &name, &buf)?;
            note(&name, buf.len() as u64);
        }
    } else {
        let file = std::fs::File::open(archive)
            .with_context(|| format!("opening {}", archive.display()))?;
        let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
        let mut tar = tar::Archive::new(gz);
        for entry in tar.entries()? {
            let mut entry = entry?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry.path()?.to_path_buf();
            if !is_lib_entry(&path) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            write_atomic(dir, &name, &buf)?;
            note(&name, buf.len() as u64);
        }
    }

    let (name, _) =
        biggest.ok_or_else(|| anyhow!("the package has no ONNX Runtime library in lib/"))?;
    Ok(name)
}

fn make_canonical(dir: &Path, extracted: &str) -> anyhow::Result<PathBuf> {
    let canonical = dir.join(lib_filename());
    if extracted == lib_filename() {
        return Ok(canonical);
    }
    let src = dir.join(extracted);
    let tmp = dir.join(format!(".{}.tmp", lib_filename()));
    std::fs::copy(&src, &tmp)
        .with_context(|| format!("copying {} → {}", src.display(), tmp.display()))?;
    if canonical.exists() {
        let _ = std::fs::remove_file(&canonical);
    }
    std::fs::rename(&tmp, &canonical)
        .with_context(|| format!("moving to {}", canonical.display()))?;
    Ok(canonical)
}

pub async fn install_runtime(
    variant: Option<String>,
    progress: &crate::core::tools::ProgressFn,
) -> anyhow::Result<PathBuf> {
    let asset = pick_asset(variant.as_deref())?;
    let dir = target_dir().ok_or_else(|| anyhow!("could not find the app data folder"))?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let sources = sources_for(asset);
    let tmp = crate::core::tools::temp_dir().join(asset.file);
    let client = crate::core::tools::client()?;
    let mut verified_source = None;
    let mut errors: Vec<String> = Vec::new();
    for source in &sources {
        let _ = std::fs::remove_file(&tmp);
        match crate::core::tools::download_to(&client, &source.url, &tmp, progress, "onnxruntime")
            .await
        {
            Ok(_) => {
                crate::core::tools::report(
                    progress,
                    "onnxruntime",
                    "verify",
                    0,
                    None,
                    Some("checking sha256".into()),
                );
                let tmp_check = tmp.clone();
                let got = tokio::task::spawn_blocking(move || sha256_of(&tmp_check))
                    .await
                    .map_err(|e| anyhow!("verification task failed: {e}"))??;
                if got == source.sha256 {
                    verified_source = Some(source.url.clone());
                    break;
                }
                let _ = std::fs::remove_file(&tmp);
                let msg = format!(
                    "the downloaded ONNX Runtime package from {} does not match: expected sha256 {}, got {}",
                    source.url, source.sha256, got
                );
                tracing::warn!("[onnxrt] {msg}");
                errors.push(msg);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                tracing::warn!("[onnxrt] mirror/source {} failed: {e}", source.url);
                errors.push(e.to_string());
            }
        }
    }
    if verified_source.is_none() {
        return Err(anyhow!(
            "{}",
            errors
                .first()
                .cloned()
                .unwrap_or_else(|| "could not download ONNX Runtime".into())
        ));
    }

    let asset_file = asset.file.to_string();
    let is_zip = asset_file.ends_with(".zip");
    let dir_for_task = dir.clone();
    let tmp_for_task = tmp.clone();
    let p = progress.clone();
    let out = tokio::task::spawn_blocking(move || -> anyhow::Result<PathBuf> {
        crate::core::tools::report(
            &p,
            "onnxruntime",
            "extract",
            0,
            None,
            Some("extracting".into()),
        );
        let extracted = extract_libs(&tmp_for_task, is_zip, &dir_for_task)?;
        let canonical = make_canonical(&dir_for_task, &extracted)?;
        let _ = std::fs::remove_file(&tmp_for_task);
        Ok(canonical)
    })
    .await
    .map_err(|e| anyhow!("extraction task failed: {e}"))??;

    if let Some(marker) = version_marker_path() {
        let _ = std::fs::write(&marker, format!("{} ({})", RUNTIME_VERSION, asset.file));
    }
    strip_quarantine(&out).await;
    crate::core::tools::report(progress, "onnxruntime", "done", 1, Some(1), None);
    Ok(out)
}

#[cfg(target_os = "macos")]
async fn strip_quarantine(path: &Path) {
    let p = path.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        crate::core::process::std_command("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(&p)
            .output()
    })
    .await;
}

#[cfg(not(target_os = "macos"))]
async fn strip_quarantine(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_try_project_mirror_before_github() {
        for asset in ASSETS {
            let sources = sources_for(asset);
            assert_eq!(
                sources[0].url,
                format!(
                    "https://getomniget.com/dl/onnxruntime/v{RUNTIME_VERSION}/{}",
                    asset.file
                )
            );
            assert_eq!(sources[0].sha256, asset.mirror_sha256);
            assert_eq!(sources[0].sha256.len(), 64);
            assert_eq!(
                sources[1].url,
                format!("https://github.com/microsoft/onnxruntime/releases/download/v{RUNTIME_VERSION}/{}", asset.file)
            );
            assert_eq!(sources[1].sha256, asset.upstream_sha256);
            assert_eq!(sources[1].sha256.len(), 64);
        }
    }

    #[test]
    fn extract_libs_reads_tgz_and_zip_lib_layout() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let lib = lib_filename();
        let out_tgz = dir.path().join("out_tgz");
        std::fs::create_dir_all(&out_tgz).unwrap();
        let tgz_path = dir.path().join("ort.tgz");
        {
            let f = std::fs::File::create(&tgz_path).unwrap();
            let enc = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut tar = tar::Builder::new(enc);
            let data = b"fake-onnxruntime-binary";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(
                &mut header,
                format!("onnxruntime-test/lib/{lib}"),
                &data[..],
            )
            .unwrap();
            tar.finish().unwrap();
        }
        let picked = extract_libs(&tgz_path, false, &out_tgz).unwrap();
        assert_eq!(picked, lib);
        assert_eq!(
            std::fs::read(out_tgz.join(lib)).unwrap(),
            b"fake-onnxruntime-binary"
        );

        let out_zip = dir.path().join("out_zip");
        std::fs::create_dir_all(&out_zip).unwrap();
        let zip_path = dir.path().join("ort.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file(format!("onnxruntime-test/lib/{lib}"), opts)
                .unwrap();
            w.write_all(b"fake-onnxruntime-zip").unwrap();
            w.finish().unwrap();
        }
        let picked_zip = extract_libs(&zip_path, true, &out_zip).unwrap();
        assert_eq!(picked_zip, lib);
        assert_eq!(
            std::fs::read(out_zip.join(lib)).unwrap(),
            b"fake-onnxruntime-zip"
        );
    }
}
