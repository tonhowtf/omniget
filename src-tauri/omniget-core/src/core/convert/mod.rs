pub mod cad;
pub mod data;
pub mod document;
pub mod external;
pub mod formats;
pub mod graph;
pub mod heic;
pub mod image;
pub mod jxl;
pub mod media;
pub mod psd;
pub mod raw;
pub mod svg;
pub mod vector;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub use external::{detect_tools, ToolStatus};
pub use formats::{Category, FormatInfo, FORMATS};
pub use graph::{Edge, Engine, Route, TargetOption};

#[derive(Deserialize, Serialize, Clone, Default, Debug)]
pub struct ConvertOptions {
    pub pretty: Option<bool>,
    pub sort_keys: Option<bool>,
    pub quality: Option<u8>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub overwrite: Option<bool>,
    pub vector_preset: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct InspectResult {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub format: Option<FormatInfo>,
    pub targets: Vec<TargetOption>,
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ConvertOutcome {
    pub input: String,
    pub output: Option<String>,
    pub size_bytes: u64,
    pub route: Option<Route>,
    pub ok: bool,
    pub error: Option<String>,
    pub skipped: bool,
}

impl ConvertOutcome {
    fn failed(input: &Path, route: Option<Route>, error: String) -> Self {
        Self {
            input: input.to_string_lossy().into_owned(),
            output: None,
            size_bytes: 0,
            route,
            ok: false,
            error: Some(error),
            skipped: false,
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub async fn inspect(path: &Path, tools: &ToolStatus) -> InspectResult {
    let mut res = InspectResult {
        path: path.to_string_lossy().into_owned(),
        name: file_name(path),
        size_bytes: 0,
        format: None,
        targets: Vec::new(),
        error: None,
    };
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => res.size_bytes = m.len(),
        Ok(_) => {
            res.error = Some("This is a folder, not a file. Pick the files inside it.".into());
            return res;
        }
        Err(_) => {
            res.error = Some("File not found. It may have been moved or deleted.".into());
            return res;
        }
    }
    if res.size_bytes == 0 {
        res.error = Some("This file is empty, so there is nothing to convert.".into());
        return res;
    }
    let owned = path.to_path_buf();
    let format = tokio::task::spawn_blocking(move || formats::detect(&owned))
        .await
        .ok()
        .flatten();
    match format {
        Some(f) => {
            res.targets = graph::targets(f.id, tools);
            res.format = Some(f.clone());
        }
        None => {
            res.error =
                Some("OmniGet doesn't recognize this file type. Check the file extension.".into());
        }
    }
    res
}

pub fn path_key(path: &Path) -> PathBuf {
    let abs = path
        .canonicalize()
        .ok()
        .or_else(|| {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Some(parent.canonicalize().ok()?.join(path.file_name()?))
        })
        .unwrap_or_else(|| path.to_path_buf());
    if cfg!(any(target_os = "macos", windows)) {
        PathBuf::from(abs.to_string_lossy().to_lowercase())
    } else {
        abs
    }
}

pub fn plan_outputs(
    items: &[(PathBuf, String)],
    out_dir: Option<&Path>,
    overwrite: bool,
) -> Vec<Option<PathBuf>> {
    let inputs: HashSet<PathBuf> = items.iter().map(|(p, _)| path_key(p)).collect();
    let mut taken: HashSet<PathBuf> = HashSet::new();
    items
        .iter()
        .map(|(input, target)| {
            let fmt = formats::by_id(target)?;
            if formats::detect(input).map(|f| f.id) == Some(fmt.id) {
                return None;
            }
            let dir = out_dir
                .map(Path::to_path_buf)
                .or_else(|| input.parent().map(Path::to_path_buf))
                .unwrap_or_default();
            let stem = stem_of(input);
            let overwrite = overwrite && !formats::is_folder(fmt.id);
            let mut n = 0u32;
            loop {
                let candidate = dir.join(formats::output_name(&stem, fmt.id, n));
                let key = path_key(&candidate);
                if !inputs.contains(&key)
                    && !taken.contains(&key)
                    && (overwrite || !candidate.exists())
                {
                    taken.insert(key);
                    return Some(candidate);
                }
                n += 1;
            }
        })
        .collect()
}

fn stem_of(input: &Path) -> String {
    input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string())
}

pub fn output_path(input: &Path, out_dir: &Path, format: &str, overwrite: bool) -> PathBuf {
    let stem = stem_of(input);
    let overwrite = overwrite && !formats::is_folder(format);
    let input_key = path_key(input);
    let mut n = 0;
    loop {
        let candidate = out_dir.join(formats::output_name(&stem, format, n));
        if path_key(&candidate) != input_key && (overwrite || !candidate.exists()) {
            return candidate;
        }
        n += 1;
    }
}

fn create_exclusive(path: &Path, format: &str) -> std::io::Result<()> {
    if formats::is_folder(format) {
        return std::fs::create_dir(path);
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
}

fn reserve_output(
    input: &Path,
    out_dir: &Path,
    format: &str,
    overwrite: bool,
) -> Result<PathBuf, String> {
    if overwrite && !formats::is_folder(format) {
        return Ok(output_path(input, out_dir, format, true));
    }
    for _ in 0..1000 {
        let candidate = output_path(input, out_dir, format, false);
        match create_exclusive(&candidate, format) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "Could not write to the output folder: {e}. Choose another folder."
                ))
            }
        }
    }
    Err("Could not find a free file name in the output folder.".to_string())
}

fn move_folder(from: &Path, to: &Path) -> Result<(), String> {
    let entries =
        std::fs::read_dir(from).map_err(|e| format!("Could not read the layers: {e}."))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Could not read the layers: {e}."))?;
        move_file(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

pub fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("Could not save the converted file: {e}."))?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

fn unavailable_error(from: &FormatInfo, to: &FormatInfo, tools: &ToolStatus) -> String {
    match graph::missing_tool(from.id, to.id, tools) {
        Some(engine) => format!(
            "{} is not installed. Install it to convert {} to {}.",
            external::tool_name(engine),
            from.label,
            to.label
        ),
        None => format!("OmniGet can't convert {} to {}.", from.label, to.label),
    }
}

fn validate(path: &Path, format: &str) -> Result<(), String> {
    if formats::is_folder(format) {
        let has_files = std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some());
        return if has_files {
            Ok(())
        } else {
            Err("The conversion produced no files.".into())
        };
    }
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if len == 0 {
        return Err("The conversion produced an empty file. Try another target format.".into());
    }
    if formats::category_of(format) == Some(Category::Data) {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        data::read_value(&bytes, format)
            .map_err(|e| format!("The converted file did not read back correctly. {e}"))?;
    }
    Ok(())
}

async fn native_step(
    input: PathBuf,
    output: PathBuf,
    from: &'static str,
    to: &'static str,
    opts: ConvertOptions,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        if let Some(limit) = image::input_limit(from) {
            let size = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);
            if size > limit {
                return Err(format!(
                    "This file is too large to convert (over {} MB).",
                    limit / (1024 * 1024)
                ));
            }
        }
        if formats::is_folder(to) {
            let bytes =
                std::fs::read(&input).map_err(|e| format!("Could not read the file: {e}."))?;
            return psd::export_layers(&bytes, &output).map(|_| ());
        }
        let category = formats::category_of(from);
        match category {
            Some(Category::Data) => {
                let bytes =
                    std::fs::read(&input).map_err(|e| format!("Could not read the file: {e}."))?;
                let out = data::convert_bytes(&bytes, from, to, &opts)?;
                std::fs::write(&output, out)
                    .map_err(|e| format!("Could not write the output: {e}."))
            }
            Some(Category::Image) => {
                let bytes =
                    std::fs::read(&input).map_err(|e| format!("Could not read the file: {e}."))?;
                let out = image::convert_bytes(&bytes, from, to, &opts)?;
                std::fs::write(&output, out)
                    .map_err(|e| format!("Could not write the output: {e}."))
            }
            Some(Category::Document) => document::convert_file(&input, from, &output, to),
            Some(Category::Cad) => cad::convert_file(&input, from, &output, to),
            _ => Err(format!("OmniGet can't convert {from} to {to} by itself.")),
        }
    })
    .await
    .map_err(|e| {
        if e.is_panic() {
            "This file could not be converted. It may be damaged or use a variant OmniGet can't read yet.".to_string()
        } else {
            format!("The conversion stopped unexpectedly: {e}.")
        }
    })?
}

async fn run_step(
    step: &Edge,
    input: &Path,
    output: &Path,
    opts: &ConvertOptions,
    cancel: &CancellationToken,
    progress: &(dyn Fn(f64) + Send + Sync),
) -> Result<(), String> {
    match step.engine {
        Engine::Native => {
            native_step(
                input.to_path_buf(),
                output.to_path_buf(),
                step.from,
                step.to,
                opts.clone(),
            )
            .await
        }
        Engine::Ffmpeg => {
            media::convert(input, step.from, output, step.to, opts, cancel, progress).await
        }
        Engine::Pandoc => external::pandoc(input, step.from, output, step.to, cancel).await,
        Engine::ImageMagick => {
            external::magick(input, step.from, output, opts.quality, cancel).await
        }
        Engine::LibreOffice => external::soffice(input, output, step.to, cancel).await,
    }
}

enum Dest<'a> {
    Dir(Option<&'a Path>),
    File(&'a Path),
}

pub async fn run(
    input: &Path,
    target: &str,
    out_dir: Option<&Path>,
    opts: &ConvertOptions,
    tools: &ToolStatus,
    cancel: CancellationToken,
    progress: impl Fn(f64) + Send + Sync,
) -> ConvertOutcome {
    run_inner(
        input,
        target,
        Dest::Dir(out_dir),
        opts,
        tools,
        cancel,
        progress,
    )
    .await
}

pub async fn run_to(
    input: &Path,
    target: &str,
    output: &Path,
    opts: &ConvertOptions,
    tools: &ToolStatus,
    cancel: CancellationToken,
    progress: impl Fn(f64) + Send + Sync,
) -> ConvertOutcome {
    run_inner(
        input,
        target,
        Dest::File(output),
        opts,
        tools,
        cancel,
        progress,
    )
    .await
}

fn folder_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

fn reserve_exact(
    input: &Path,
    path: &Path,
    format: &str,
    overwrite: bool,
) -> Result<PathBuf, String> {
    if path_key(path) == path_key(input) {
        return Err(
            "The converted file would replace the original. Choose another output folder.".into(),
        );
    }
    if overwrite {
        return Ok(path.to_path_buf());
    }
    match create_exclusive(path, format) {
        Ok(()) => Ok(path.to_path_buf()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let dir = path.parent().unwrap_or(Path::new("."));
            reserve_output(input, dir, format, false)
        }
        Err(e) => Err(format!(
            "Could not write to the output folder: {e}. Choose another folder."
        )),
    }
}

async fn run_inner(
    input: &Path,
    target: &str,
    dest: Dest<'_>,
    opts: &ConvertOptions,
    tools: &ToolStatus,
    cancel: CancellationToken,
    progress: impl Fn(f64) + Send + Sync,
) -> ConvertOutcome {
    let meta = match std::fs::metadata(input) {
        Ok(m) if m.is_file() => m,
        _ => {
            return ConvertOutcome::failed(
                input,
                None,
                "File not found. It may have been moved or deleted.".into(),
            )
        }
    };
    if meta.len() == 0 {
        return ConvertOutcome::failed(
            input,
            None,
            "This file is empty, so there is nothing to convert.".into(),
        );
    }
    let Some(target_fmt) = formats::by_id(target) else {
        return ConvertOutcome::failed(input, None, format!("Unknown target format \"{target}\"."));
    };
    let owned = input.to_path_buf();
    let source = tokio::task::spawn_blocking(move || formats::detect(&owned))
        .await
        .ok()
        .flatten();
    let Some(source) = source else {
        return ConvertOutcome::failed(
            input,
            None,
            "OmniGet doesn't recognize this file type. Check the file extension.".into(),
        );
    };
    if source.id == target_fmt.id {
        return ConvertOutcome {
            input: input.to_string_lossy().into_owned(),
            output: None,
            size_bytes: meta.len(),
            route: Some(Route::default()),
            ok: true,
            error: None,
            skipped: true,
        };
    }
    let Some(route) = graph::plan(source.id, target_fmt.id, tools) else {
        return ConvertOutcome::failed(input, None, unavailable_error(source, target_fmt, tools));
    };
    let dir = match dest {
        Dest::File(p) => p.parent().map(Path::to_path_buf),
        Dest::Dir(d) => d
            .map(Path::to_path_buf)
            .or_else(|| input.parent().map(Path::to_path_buf)),
    }
    .filter(|d| !d.as_os_str().is_empty())
    .unwrap_or_else(|| PathBuf::from("."));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return ConvertOutcome::failed(
            input,
            Some(route),
            format!("Could not create the output folder: {e}. Choose another folder."),
        );
    }
    let work = match tempfile::tempdir() {
        Ok(w) => w,
        Err(e) => {
            return ConvertOutcome::failed(
                input,
                Some(route),
                format!("Could not create a temporary folder: {e}."),
            )
        }
    };
    let progress = Arc::new(progress);
    let total = route.steps.len() as f64;
    let mut current = input.to_path_buf();
    for (i, step) in route.steps.iter().enumerate() {
        if cancel.is_cancelled() {
            return ConvertOutcome::failed(input, Some(route.clone()), "Cancelled.".into());
        }
        let next = work
            .path()
            .join(format!("step{i}.{}", formats::extension_of(step.to)));
        let p = progress.clone();
        let base = i as f64;
        let report = move |f: f64| p(((base + f.clamp(0.0, 1.0)) / total).min(1.0));
        report(0.0);
        if let Err(e) = run_step(step, &current, &next, opts, &cancel, &report).await {
            return ConvertOutcome::failed(input, Some(route.clone()), e);
        }
        if let Err(e) = validate(&next, step.to) {
            return ConvertOutcome::failed(input, Some(route.clone()), e);
        }
        report(1.0);
        current = next;
        if cancel.is_cancelled() {
            return ConvertOutcome::failed(input, Some(route.clone()), "Cancelled.".into());
        }
    }
    let folder = formats::is_folder(target_fmt.id);
    let overwrite = opts.overwrite.unwrap_or(false) && !folder;
    let reserved = match dest {
        Dest::File(p) => reserve_exact(input, p, target_fmt.id, overwrite),
        Dest::Dir(_) => reserve_output(input, &dir, target_fmt.id, overwrite),
    };
    let discard = |d: &Path| {
        if folder {
            let _ = std::fs::remove_dir_all(d);
        } else if !overwrite {
            let _ = std::fs::remove_file(d);
        }
    };
    if cancel.is_cancelled() {
        if let Ok(d) = &reserved {
            discard(d);
        }
        return ConvertOutcome::failed(input, Some(route), "Cancelled.".into());
    }
    let dest = match reserved {
        Ok(d) => d,
        Err(e) => return ConvertOutcome::failed(input, Some(route), e),
    };
    let moved = if folder {
        move_folder(&current, &dest)
    } else {
        move_file(&current, &dest)
    };
    if let Err(e) = moved {
        discard(&dest);
        return ConvertOutcome::failed(input, Some(route), e);
    }
    let size = if folder {
        folder_size(&dest)
    } else {
        std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0)
    };
    progress(1.0);
    ConvertOutcome {
        input: input.to_string_lossy().into_owned(),
        output: Some(dest.to_string_lossy().into_owned()),
        size_bytes: size,
        route: Some(route),
        ok: true,
        error: None,
        skipped: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_outputs_never_targets_batch_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let a_json = dir.path().join("a.json");
        let a_yaml = dir.path().join("a.yaml");
        std::fs::write(&a_json, b"{}").unwrap();
        std::fs::write(&a_yaml, b"x: 1").unwrap();
        let to_yaml = vec![
            (a_json.clone(), "yaml".to_string()),
            (a_yaml.clone(), "yaml".to_string()),
        ];
        let plan = plan_outputs(&to_yaml, None, true);
        assert_eq!(
            plan[0].as_deref(),
            Some(dir.path().join("a (1).yaml").as_path())
        );
        assert_eq!(plan[1], None);
        let to_toml = vec![
            (a_json.clone(), "toml".to_string()),
            (a_yaml.clone(), "toml".to_string()),
        ];
        let plan = plan_outputs(&to_toml, None, true);
        assert_eq!(
            plan[0].as_deref(),
            Some(dir.path().join("a.toml").as_path())
        );
        assert_eq!(
            plan[1].as_deref(),
            Some(dir.path().join("a (1).toml").as_path())
        );
    }

    #[tokio::test]
    async fn overwrite_batch_keeps_other_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let a_json = dir.path().join("a.json");
        let a_yaml = dir.path().join("a.yaml");
        std::fs::write(&a_json, br#"{"from": "json"}"#).unwrap();
        std::fs::write(&a_yaml, b"from: yaml\n").unwrap();
        let items = vec![
            (a_json.clone(), "toml".to_string()),
            (a_yaml.clone(), "toml".to_string()),
        ];
        let o = ConvertOptions {
            overwrite: Some(true),
            ..Default::default()
        };
        let plan = plan_outputs(&items, None, true);
        for ((input, target), out) in items.iter().zip(&plan) {
            let r = run_to(
                input,
                target,
                out.as_deref().unwrap(),
                &o,
                &ToolStatus::default(),
                CancellationToken::new(),
                |_| {},
            )
            .await;
            assert!(r.ok, "{:?}", r.error);
        }
        assert_eq!(std::fs::read_to_string(&a_yaml).unwrap(), "from: yaml\n");
        assert!(std::fs::read_to_string(dir.path().join("a.toml"))
            .unwrap()
            .contains("json"));
        assert!(std::fs::read_to_string(dir.path().join("a (1).toml"))
            .unwrap()
            .contains("yaml"));
        let r = run_to(
            &a_json,
            "yaml",
            &a_json,
            &o,
            &ToolStatus::default(),
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert!(!r.ok);
    }

    #[tokio::test]
    async fn cancel_during_native_step_leaves_no_output() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("notes.md");
        std::fs::write(&input, "# Hi\n\ntext").unwrap();
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let r = run(
            &input,
            "html",
            None,
            &ConvertOptions::default(),
            &ToolStatus::default(),
            cancel,
            move |_| trigger.cancel(),
        )
        .await;
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("Cancelled."));
        assert!(!dir.path().join("notes.html").exists());
    }

    #[test]
    fn output_name_collision() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.png");
        std::fs::write(&input, b"x").unwrap();
        assert_eq!(
            output_path(&input, dir.path(), "jpeg", false),
            dir.path().join("a.jpg")
        );
        std::fs::write(dir.path().join("a.jpg"), b"x").unwrap();
        assert_eq!(
            output_path(&input, dir.path(), "jpeg", false),
            dir.path().join("a (1).jpg")
        );
        assert_eq!(
            output_path(&input, dir.path(), "jpeg", true),
            dir.path().join("a.jpg")
        );
        assert_eq!(
            output_path(&input, dir.path(), "png", true),
            dir.path().join("a (1).png")
        );
        std::fs::create_dir(dir.path().join("a layers")).unwrap();
        assert_eq!(
            output_path(&input, dir.path(), "layers", true),
            dir.path().join("a layers (1)")
        );
    }

    #[tokio::test]
    async fn run_converts_and_skips() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("data.json");
        std::fs::write(&input, br#"{"a": [1, 2], "b": {"c": "d"}}"#).unwrap();
        let tools = ToolStatus::default();
        let o = ConvertOptions::default();
        let out = run(
            &input,
            "yaml",
            None,
            &o,
            &tools,
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert!(out.ok, "{:?}", out.error);
        assert_eq!(
            out.output.as_deref(),
            Some(dir.path().join("data.yaml").to_str().unwrap())
        );
        let again = run(
            &input,
            "yaml",
            None,
            &o,
            &tools,
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert!(again.output.unwrap().ends_with("data (1).yaml"));
        let skip = run(
            &input,
            "json",
            None,
            &o,
            &tools,
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert!(skip.skipped && skip.ok);
    }

    #[tokio::test]
    async fn run_explains_failures() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.json");
        std::fs::write(&empty, b"").unwrap();
        let tools = ToolStatus::default();
        let o = ConvertOptions::default();
        let r = run(
            &empty,
            "yaml",
            None,
            &o,
            &tools,
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert!(!r.ok && r.error.unwrap().contains("empty"));
        let doc = dir.path().join("a.docx");
        std::fs::write(&doc, b"PK").unwrap();
        let r = run(
            &doc,
            "pdf",
            None,
            &o,
            &tools,
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert_eq!(
            r.error.as_deref(),
            Some("LibreOffice is not installed. Install it to convert Word to PDF.")
        );
    }

    fn layered_psd() -> Vec<u8> {
        use photocraft_psd::{LayerSpec, PixelData, PsdBuilder};
        let mut b = PsdBuilder::new(4, 4);
        b.push_layer(LayerSpec::new(
            "Back",
            0,
            0,
            4,
            4,
            PixelData::Rgba8([0, 0, 255, 255].repeat(16)),
        ));
        b.push_layer(LayerSpec::new(
            "Front",
            1,
            1,
            2,
            2,
            PixelData::Rgba8([255, 0, 0, 255].repeat(4)),
        ));
        b.composite(PixelData::Rgba8([9, 9, 9, 255].repeat(16)));
        b.to_bytes().unwrap()
    }

    #[tokio::test]
    async fn psd_becomes_png_and_a_layers_folder() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("art.psd");
        std::fs::write(&input, layered_psd()).unwrap();
        let tools = ToolStatus::default();
        let o = ConvertOptions::default();
        let png = run(
            &input,
            "png",
            None,
            &o,
            &tools,
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert!(png.ok, "{:?}", png.error);
        let img = ::image::open(png.output.unwrap()).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [9, 9, 9, 255]);
        for expected in ["art layers", "art layers (1)"] {
            let r = run(
                &input,
                "layers",
                None,
                &o,
                &tools,
                CancellationToken::new(),
                |_| {},
            )
            .await;
            assert!(r.ok, "{:?}", r.error);
            let folder = dir.path().join(expected);
            assert_eq!(r.output.as_deref(), Some(folder.to_str().unwrap()));
            let mut names: Vec<String> = std::fs::read_dir(&folder)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(names, ["01 Front.png", "02 Back.png"]);
            assert!(r.size_bytes > 0);
        }
        let planned = plan_outputs(&[(input.clone(), "layers".to_string())], None, true);
        assert_eq!(
            planned[0].as_deref(),
            Some(dir.path().join("art layers (2)").as_path())
        );
    }

    #[tokio::test]
    async fn picture_becomes_svg_with_the_chosen_style() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("logo.png");
        let mut img = ::image::RgbaImage::from_pixel(40, 40, ::image::Rgba([255, 255, 255, 255]));
        for (x, y, p) in img.enumerate_pixels_mut() {
            if (10..30).contains(&x) && (10..30).contains(&y) {
                *p = ::image::Rgba([200, 30, 30, 255]);
            }
        }
        img.save(&input).unwrap();
        let o = ConvertOptions {
            vector_preset: Some("bw".into()),
            ..Default::default()
        };
        let r = run(
            &input,
            "svg",
            None,
            &o,
            &ToolStatus::default(),
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert!(r.ok, "{:?}", r.error);
        let svg = std::fs::read_to_string(r.output.unwrap()).unwrap();
        assert!(svg.contains("#000000") && !svg.contains("#C81E1E"));
        let bad = ConvertOptions {
            vector_preset: Some("neon".into()),
            ..Default::default()
        };
        let r = run(
            &input,
            "svg",
            None,
            &bad,
            &ToolStatus::default(),
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert!(r.error.unwrap().contains("bw, colors or lineart"));
    }

    #[tokio::test]
    async fn run_md_to_epub_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("notes.md");
        std::fs::write(&input, "# Hello\n\nWorld").unwrap();
        let progress = std::sync::Mutex::new(Vec::new());
        let r = run(
            &input,
            "epub",
            None,
            &ConvertOptions::default(),
            &ToolStatus::default(),
            CancellationToken::new(),
            |p| progress.lock().unwrap().push(p),
        )
        .await;
        assert!(r.ok, "{:?}", r.error);
        assert_eq!(progress.lock().unwrap().last().copied(), Some(1.0));
    }
}
