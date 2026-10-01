use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use omniget_core::core::canvas::envs;
use omniget_core::core::canvas::model::NodeKind;
use omniget_core::core::canvas::notes::revision;
use omniget_core::core::canvas::store::CanvasStore;
use omniget_core::core::vcs::checkpoint;
use omniget_core::core::vcs::diff::{self, DiffOptions, DiffResult};
use omniget_core::core::vcs::repo::{self, GitCtx, RepoStatus};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::canvas::doc::host;

const MAX_TEXT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 5000;
const MAX_SEARCH_HITS: usize = 200;
const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".svelte-kit",
    "dist",
    "build",
    ".next",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub hidden: bool,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileText {
    pub path: String,
    pub abs_path: String,
    pub size: u64,
    pub revision: String,
    pub binary: bool,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

fn store(app: &AppHandle) -> std::sync::Arc<CanvasStore> {
    host().bind(app).store().clone()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_FILES_IO: {e}"))?
}

fn io(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        format!("CANVAS_FILES_NOT_FOUND: {e}")
    } else {
        format!("CANVAS_FILES_IO: {e}")
    }
}

fn allowed_roots(s: &CanvasStore, ws: &str) -> Result<Vec<PathBuf>, String> {
    let meta = s.workspace(ws)?;
    let mut out: Vec<String> = Vec::new();
    if let Some(d) = meta.working_directory.as_deref() {
        out.push(d.to_string());
    }
    for f in &meta.floors {
        if let Some(p) = f.path.as_deref() {
            out.push(p.to_string());
        }
        let trees = s
            .read(ws, &f.id, |doc| {
                doc.nodes
                    .iter()
                    .filter(|n| matches!(n.kind, NodeKind::FileTree))
                    .filter_map(|n| n.data_str("rootPath").map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.extend(trees);
    }
    Ok(out
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect())
}

fn resolve_root(s: &CanvasStore, ws: &str, root: &str) -> Result<PathBuf, String> {
    let root = root.trim();
    if root.is_empty() || !Path::new(root).is_absolute() {
        return Err("CANVAS_FILES_INVALID: root must be an absolute path".into());
    }
    let canon = std::fs::canonicalize(root).map_err(io)?;
    if !canon.is_dir() {
        return Err("CANVAS_FILES_INVALID: root is not a folder".into());
    }
    let ok = allowed_roots(s, ws)?.iter().any(|a| canon.starts_with(a));
    if !ok {
        return Err("CANVAS_FILES_DENIED: root is outside this workspace".into());
    }
    Ok(canon)
}

fn clean_rel(rel: &str) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for c in Path::new(rel.trim().trim_start_matches(['/', '\\'])).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return Err("CANVAS_FILES_DENIED: path escapes the root".into()),
        }
    }
    Ok(out)
}

fn resolve_existing(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let joined = root.join(clean_rel(rel)?);
    let canon = std::fs::canonicalize(&joined).map_err(io)?;
    if !canon.starts_with(root) {
        return Err("CANVAS_FILES_DENIED: path escapes the root".into());
    }
    Ok(canon)
}

fn resolve_new(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = clean_rel(rel)?;
    let name = rel
        .file_name()
        .ok_or_else(|| "CANVAS_FILES_INVALID: empty name".to_string())?
        .to_owned();
    let parent = rel.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let parent = std::fs::canonicalize(root.join(parent)).map_err(io)?;
    if !parent.starts_with(root) {
        return Err("CANVAS_FILES_DENIED: path escapes the root".into());
    }
    Ok(parent.join(name))
}

fn rel_of(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn millis(t: std::io::Result<SystemTime>) -> Option<u64> {
    t.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

fn image_mime(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        _ => return None,
    })
}

const MAX_MEDIA_BYTES: u64 = 64 * 1024 * 1024;

fn media_mime(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "opus" => "audio/opus",
        "pdf" => "application/pdf",
        _ => return None,
    })
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

fn read_file(root: &Path, abs: &Path) -> Result<FileText, String> {
    let meta = std::fs::metadata(abs).map_err(io)?;
    if meta.is_dir() {
        return Err("CANVAS_FILES_INVALID: path is a folder".into());
    }
    let size = meta.len();
    let rel = rel_of(root, abs);
    let base = FileText {
        path: rel,
        abs_path: abs.to_string_lossy().into_owned(),
        size,
        revision: String::new(),
        binary: false,
        truncated: false,
        text: None,
        data_url: None,
        mime: None,
    };
    if let Some(mime) = image_mime(abs) {
        if size > MAX_IMAGE_BYTES {
            return Ok(FileText {
                binary: true,
                truncated: true,
                mime: Some(mime.into()),
                ..base
            });
        }
        let bytes = std::fs::read(abs).map_err(io)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(FileText {
            binary: true,
            mime: Some(mime.into()),
            data_url: Some(format!("data:{mime};base64,{b64}")),
            ..base
        });
    }
    if let Some(mime) = media_mime(abs) {
        if size > MAX_MEDIA_BYTES {
            return Ok(FileText {
                binary: true,
                truncated: true,
                mime: Some(mime.into()),
                ..base
            });
        }
        let bytes = std::fs::read(abs).map_err(io)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(FileText {
            binary: true,
            mime: Some(mime.into()),
            data_url: Some(format!("data:{mime};base64,{b64}")),
            ..base
        });
    }
    if size > MAX_TEXT_BYTES {
        return Ok(FileText {
            truncated: true,
            binary: true,
            ..base
        });
    }
    let bytes = std::fs::read(abs).map_err(io)?;
    if looks_binary(&bytes) {
        return Ok(FileText {
            binary: true,
            ..base
        });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(FileText {
            revision: revision(&text),
            text: Some(text),
            ..base
        }),
        Err(_) => Ok(FileText {
            binary: true,
            ..base
        }),
    }
}

#[tauri::command]
pub async fn canvas_files_list(
    app: AppHandle,
    workspace_id: String,
    root: String,
    dir: Option<String>,
    show_hidden: Option<bool>,
) -> Result<Vec<FileEntry>, String> {
    let s = store(&app);
    blocking(move || {
        let root = resolve_root(&s, &workspace_id, &root)?;
        let dir = resolve_existing(&root, dir.as_deref().unwrap_or(""))?;
        let show_hidden = show_hidden.unwrap_or(false);
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(io)? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            let hidden = name.starts_with('.');
            if hidden && !show_hidden {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let is_symlink = ft.is_symlink();
            let meta = std::fs::metadata(entry.path()).ok();
            if is_symlink {
                match std::fs::canonicalize(entry.path()) {
                    Ok(t) if t.starts_with(&root) => {}
                    _ => continue,
                }
            }
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            out.push(FileEntry {
                path: rel_of(&root, &dir.join(&name)),
                name,
                is_dir,
                is_symlink,
                hidden,
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                modified_ms: meta.as_ref().and_then(|m| millis(m.modified())),
            });
            if out.len() >= MAX_LIST_ENTRIES {
                break;
            }
        }
        out.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn canvas_files_read(
    app: AppHandle,
    workspace_id: String,
    root: String,
    path: String,
) -> Result<FileText, String> {
    let s = store(&app);
    blocking(move || {
        let root = resolve_root(&s, &workspace_id, &root)?;
        let abs = resolve_existing(&root, &path)?;
        read_file(&root, &abs)
    })
    .await
}

#[tauri::command]
pub async fn canvas_file_node_preview(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    node_id: String,
) -> Result<Option<String>, String> {
    let s = store(&app);
    blocking(move || {
        let path = s.read(&workspace_id, &floor_id, |d| {
            d.node(&node_id)
                .filter(|n| matches!(n.kind, NodeKind::File | NodeKind::Image))
                .and_then(|n| n.data_str("path").map(str::to_string))
        })?;
        let Some(path) = path else {
            return Ok(None);
        };
        let abs = Path::new(&path);
        let Some(mime) = image_mime(abs) else {
            return Ok(None);
        };
        match std::fs::metadata(abs) {
            Ok(m) if m.is_file() && m.len() <= MAX_IMAGE_BYTES => {}
            _ => return Ok(None),
        }
        let bytes = std::fs::read(abs).map_err(io)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(Some(format!("data:{mime};base64,{b64}")))
    })
    .await
}

#[tauri::command]
pub async fn canvas_files_write(
    app: AppHandle,
    workspace_id: String,
    root: String,
    path: String,
    text: String,
    if_revision: Option<String>,
) -> Result<FileText, String> {
    let s = store(&app);
    blocking(move || {
        if text.len() as u64 > MAX_TEXT_BYTES {
            return Err("CANVAS_FILES_INVALID: file is larger than 4 MB".into());
        }
        let root = resolve_root(&s, &workspace_id, &root)?;
        let abs = resolve_new(&root, &path)?;
        if abs.is_dir() {
            return Err("CANVAS_FILES_INVALID: path is a folder".into());
        }
        if let Some(expected) = if_revision.as_deref() {
            let current = match std::fs::read(&abs) {
                Ok(b) => revision(&String::from_utf8_lossy(&b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => revision(""),
                Err(e) => return Err(io(e)),
            };
            if current != expected {
                return Err(format!("CANVAS_FILE_CONFLICT: {current}"));
            }
        }
        let tmp = abs.with_file_name(format!(
            ".{}.omniget-tmp",
            abs.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        std::fs::write(&tmp, text.as_bytes()).map_err(io)?;
        if let Err(e) = std::fs::rename(&tmp, &abs) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io(e));
        }
        read_file(&root, &abs)
    })
    .await
}

#[tauri::command]
pub async fn canvas_files_create(
    app: AppHandle,
    workspace_id: String,
    root: String,
    path: String,
    dir: Option<bool>,
) -> Result<FileEntry, String> {
    let s = store(&app);
    blocking(move || {
        let root = resolve_root(&s, &workspace_id, &root)?;
        let abs = resolve_new(&root, &path)?;
        if abs.exists() {
            return Err("CANVAS_FILES_INVALID: already exists".into());
        }
        let is_dir = dir.unwrap_or(false);
        if is_dir {
            std::fs::create_dir(&abs).map_err(io)?;
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&abs)
                .map_err(io)?;
        }
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(FileEntry {
            path: rel_of(&root, &abs),
            hidden: name.starts_with('.'),
            name,
            is_dir,
            is_symlink: false,
            size: 0,
            modified_ms: millis(Ok(SystemTime::now())),
        })
    })
    .await
}

#[tauri::command]
pub async fn canvas_files_rename(
    app: AppHandle,
    workspace_id: String,
    root: String,
    from: String,
    to: String,
) -> Result<String, String> {
    let s = store(&app);
    blocking(move || {
        let root = resolve_root(&s, &workspace_id, &root)?;
        let src = resolve_existing(&root, &from)?;
        if src == root {
            return Err("CANVAS_FILES_DENIED: cannot rename the root".into());
        }
        let dst = resolve_new(&root, &to)?;
        if dst.exists() {
            return Err("CANVAS_FILES_INVALID: already exists".into());
        }
        std::fs::rename(&src, &dst).map_err(io)?;
        Ok(rel_of(&root, &dst))
    })
    .await
}

#[tauri::command]
pub async fn canvas_files_delete(
    app: AppHandle,
    workspace_id: String,
    root: String,
    path: String,
) -> Result<String, String> {
    let s = store(&app);
    blocking(move || {
        let root = resolve_root(&s, &workspace_id, &root)?;
        let src = resolve_existing(&root, &path)?;
        if src == root {
            return Err("CANVAS_FILES_DENIED: cannot delete the root".into());
        }
        let bin = s.workspace_dir(&workspace_id).join("trash").join("files");
        std::fs::create_dir_all(&bin).map_err(io)?;
        let stamp = millis(Ok(SystemTime::now())).unwrap_or(0);
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dst = bin.join(format!("{stamp}-{name}"));
        if std::fs::rename(&src, &dst).is_err() {
            if src.is_dir() {
                return Err("CANVAS_FILES_IO: folder is on another volume than the trash".into());
            }
            std::fs::copy(&src, &dst).map_err(io)?;
            std::fs::remove_file(&src).map_err(io)?;
        }
        Ok(dst.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
pub async fn canvas_files_search(
    app: AppHandle,
    workspace_id: String,
    root: String,
    query: String,
    content: Option<bool>,
    show_hidden: Option<bool>,
    limit: Option<usize>,
) -> Result<Vec<SearchHit>, String> {
    let s = store(&app);
    blocking(move || {
        let root = resolve_root(&s, &workspace_id, &root)?;
        let mut q = query.trim().to_lowercase();
        let mut content = content.unwrap_or(false);
        if let Some(rest) = q.strip_prefix('>') {
            q = rest.trim().to_string();
            content = true;
        }
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let words: Vec<&str> = q.split_whitespace().collect();
        let show_hidden = show_hidden.unwrap_or(false);
        let limit = limit.unwrap_or(MAX_SEARCH_HITS).min(MAX_SEARCH_HITS);
        let mut hits = Vec::new();
        let walker = walkdir::WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let name = e.file_name().to_string_lossy();
                if e.file_type().is_dir() && SKIP_DIRS.contains(&name.as_ref()) {
                    return false;
                }
                show_hidden || !name.starts_with('.')
            });
        for entry in walker.flatten() {
            if entry.depth() == 0 {
                continue;
            }
            let rel = rel_of(&root, entry.path());
            let is_dir = entry.file_type().is_dir();
            if !content {
                if name_matches(&words, &rel, is_dir) {
                    hits.push(SearchHit {
                        path: rel,
                        is_dir,
                        line: None,
                        preview: None,
                    });
                }
            } else if entry.file_type().is_file() {
                let Ok(meta) = entry.metadata() else { continue };
                if meta.len() > MAX_SEARCH_FILE_BYTES {
                    continue;
                }
                let Ok(bytes) = std::fs::read(entry.path()) else {
                    continue;
                };
                if looks_binary(&bytes) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                for (i, line) in text.lines().enumerate() {
                    if line.to_lowercase().contains(&q) {
                        let preview: String = line.trim().chars().take(160).collect();
                        hits.push(SearchHit {
                            path: rel.clone(),
                            is_dir: false,
                            line: Some(i as u32 + 1),
                            preview: Some(preview),
                        });
                        if hits.len() >= limit {
                            break;
                        }
                    }
                }
            }
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    })
    .await
}

fn name_matches(words: &[&str], rel: &str, is_dir: bool) -> bool {
    let lower = rel.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    words.iter().all(|w| {
        if w.contains('*') || w.contains('?') {
            if is_dir && w.contains('.') {
                return false;
            }
            let target = if w.contains('/') {
                lower.as_str()
            } else {
                name
            };
            glob_match(w.as_bytes(), target.as_bytes())
        } else {
            lower.contains(w)
        }
    })
}

fn glob_match(p: &[u8], t: &[u8]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

#[tauri::command]
pub async fn canvas_files_git_status(
    app: AppHandle,
    workspace_id: String,
    root: String,
) -> Result<RepoStatus, String> {
    let s = store(&app);
    let root = blocking(move || resolve_root(&s, &workspace_id, &root)).await?;
    Ok(repo::status(&root).await?)
}

#[tauri::command]
pub async fn canvas_files_git_diff(
    app: AppHandle,
    workspace_id: String,
    root: String,
    path: Option<String>,
) -> Result<DiffResult, String> {
    let s = store(&app);
    let root = blocking(move || resolve_root(&s, &workspace_id, &root)).await?;
    let paths = repo::discover(&root).await?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    let head_out = ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .unchecked()
        .run()
        .await?;
    let head = if head_out.ok() {
        head_out.trimmed()
    } else {
        ctx.git()
            .args(["hash-object", "-t", "tree", "--stdin"])
            .stdin(Vec::new())
            .run()
            .await?
            .trimmed()
    };
    let tree = checkpoint::live_tree(&ctx).await?;
    let mut res = diff::diff_trees(
        &ctx,
        &head,
        &tree,
        &DiffOptions {
            context: 3,
            max_bytes: diff::DEFAULT_MAX_BYTES,
            max_file_bytes: diff::DEFAULT_MAX_FILE_BYTES,
            ..Default::default()
        },
    )
    .await?;
    let prefix = rel_of(
        &std::fs::canonicalize(&paths.root).unwrap_or(paths.root.clone()),
        &root,
    );
    let filter = match path.as_deref().map(clean_rel).transpose()? {
        Some(p) if !p.as_os_str().is_empty() => {
            let p = p.to_string_lossy().replace('\\', "/");
            Some(if prefix.is_empty() {
                p
            } else {
                format!("{prefix}/{p}")
            })
        }
        _ if !prefix.is_empty() => Some(prefix.clone()),
        _ => None,
    };
    if let Some(f) = filter {
        res.files
            .retain(|d| d.path == f || d.path.starts_with(&format!("{f}/")));
        res.additions = res.files.iter().map(|d| d.additions).sum();
        res.deletions = res.files.iter().map(|d| d.deletions).sum();
    }
    Ok(res)
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GitOpArgs {
    pub message: Option<String>,
    pub paths: Option<Vec<String>>,
    pub branch: Option<String>,
    pub start: Option<String>,
    pub force: Option<bool>,
    pub index: Option<u32>,
    pub untracked: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictView {
    pub path: String,
    pub current_label: String,
    pub incoming_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incoming: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOpResult {
    pub action: String,
    pub ok: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branches: Option<Vec<repo::BranchInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stashes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<ConflictView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<RepoStatus>,
}

pub const GIT_ACTIONS: &[&str] = &[
    "status",
    "branches",
    "commit",
    "pull",
    "fetch",
    "push",
    "sync",
    "checkout",
    "newBranch",
    "deleteBranch",
    "merge",
    "mergeAbort",
    "stashPush",
    "stashApply",
    "stashPop",
    "stashDrop",
    "stashList",
    "stage",
    "unstage",
    "discard",
    "conflict",
    "removeLock",
];

const STALE_LOCK_SECS: u64 = 60;

fn git_err(e: omniget_core::core::vcs::VcsError) -> String {
    format!("CANVAS_GIT: {e}")
}

fn ref_arg(v: Option<&str>, what: &str) -> Result<String, String> {
    let r = v.map(str::trim).unwrap_or("");
    if r.is_empty() {
        return Err(format!("CANVAS_FILES_INVALID: {what} is required"));
    }
    if r.starts_with('-') || r.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("CANVAS_FILES_INVALID: bad {what} '{r}'"));
    }
    Ok(r.to_string())
}

fn rel_paths(paths: &Option<Vec<String>>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for p in paths.iter().flatten() {
        let c = clean_rel(p)?;
        let s = c.to_string_lossy().replace('\\', "/");
        if s.is_empty() {
            continue;
        }
        if s.starts_with('-') {
            return Err("CANVAS_FILES_INVALID: paths cannot start with '-'".into());
        }
        out.push(s);
    }
    Ok(out)
}

fn need_paths(paths: &[String], action: &str) -> Result<(), String> {
    if paths.is_empty() {
        return Err(format!("CANVAS_FILES_INVALID: {action} needs paths"));
    }
    Ok(())
}

fn joined(o: &omniget_core::core::vcs::runner::Output) -> String {
    let mut t = o.text().trim().to_string();
    let e = o.err_text();
    let e = e.trim();
    if !e.is_empty() {
        if !t.is_empty() {
            t.push('\n');
        }
        t.push_str(e);
    }
    t
}

async fn remove_stale_lock(ctx: &GitCtx, force: bool) -> Result<bool, String> {
    let dir = ctx.git_dir().await.map_err(git_err)?;
    let lock = dir.join("index.lock");
    let Ok(meta) = std::fs::metadata(&lock) else {
        return Ok(false);
    };
    let age = meta
        .modified()
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if !force && age < STALE_LOCK_SECS {
        return Ok(false);
    }
    std::fs::remove_file(&lock).map_err(io)?;
    Ok(true)
}

async fn conflicts_of(ctx: &GitCtx) -> Vec<String> {
    match ctx
        .git()
        .args(["diff", "--name-only", "--relative", "--diff-filter=U"])
        .unchecked()
        .run()
        .await
    {
        Ok(o) if o.ok() => o
            .text()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

async fn stage_blob(ctx: &GitCtx, stage: u8, path: &str) -> Option<String> {
    let o = ctx
        .git()
        .arg("show")
        .arg(format!(":{stage}:./{path}"))
        .max_output(MAX_TEXT_BYTES as usize)
        .unchecked()
        .run()
        .await
        .ok()?;
    o.ok().then(|| o.text())
}

async fn merge_source_name(ctx: &GitCtx) -> Option<String> {
    let msg_path = ctx
        .git()
        .args(["rev-parse", "--git-path", "MERGE_MSG"])
        .unchecked()
        .run()
        .await
        .ok()
        .filter(|o| o.ok())
        .map(|o| o.trimmed())?;
    let path = if Path::new(&msg_path).is_absolute() {
        PathBuf::from(&msg_path)
    } else {
        ctx.work_tree().join(&msg_path)
    };
    if let Some(name) = std::fs::read_to_string(path)
        .ok()
        .and_then(|m| parse_merge_msg(m.lines().next().unwrap_or("")))
    {
        return Some(name);
    }
    ctx.git()
        .args(["name-rev", "--name-only", "--exclude=tags/*", "MERGE_HEAD"])
        .unchecked()
        .run()
        .await
        .ok()
        .filter(|o| o.ok())
        .map(|o| o.trimmed())
        .filter(|s| !s.is_empty() && s != "undefined")
}

fn parse_merge_msg(line: &str) -> Option<String> {
    let rest = line.strip_prefix("Merge ")?;
    let start = rest.find('\'')? + 1;
    let len = rest[start..].find('\'')?;
    let name = &rest[start..start + len];
    (!name.is_empty()).then(|| name.to_string())
}

async fn git_op(
    ctx: &GitCtx,
    repo_prefix: &str,
    action: &str,
    a: &GitOpArgs,
    trash: &Path,
) -> Result<GitOpResult, String> {
    let root = ctx.work_tree().to_path_buf();
    let mut r = GitOpResult {
        action: action.to_string(),
        ok: true,
        output: String::new(),
        conflicts: Vec::new(),
        sha: None,
        branches: None,
        stashes: None,
        conflict: None,
        status: None,
    };
    let net = Some(std::time::Duration::from_secs(300));
    let paths = rel_paths(&a.paths)?;
    let is_write = !matches!(action, "status" | "branches" | "stashList" | "conflict");
    if is_write && action != "removeLock" {
        let _ = remove_stale_lock(ctx, false).await;
    }
    macro_rules! run {
        ($inv:expr) => {{
            let o = $inv.unchecked().run().await.map_err(git_err)?;
            let text = joined(&o);
            if !o.ok() {
                return Err(format!(
                    "CANVAS_GIT: {}",
                    if text.is_empty() {
                        "git failed".to_string()
                    } else {
                        text
                    }
                ));
            }
            text
        }};
    }
    match action {
        "status" => {}
        "branches" => {
            r.branches = Some(repo::branches(&root).await.map_err(git_err)?);
        }
        "commit" => {
            let files: Vec<String> = paths
                .iter()
                .map(|p| {
                    if repo_prefix.is_empty() {
                        p.clone()
                    } else {
                        format!("{repo_prefix}/{p}")
                    }
                })
                .collect();
            let c = omniget_core::core::vcs::actions::commit(
                &root,
                a.message.as_deref(),
                (!files.is_empty()).then_some(files.as_slice()),
            )
            .await
            .map_err(git_err)?;
            r.ok = c.status == "created";
            r.output = if r.ok {
                c.subject.clone()
            } else {
                "nothing to commit".into()
            };
            r.sha = c.sha;
        }
        "pull" | "sync" => {
            let o = ctx
                .git()
                .args(["pull", "--no-rebase", "--no-edit"])
                .timeout(net)
                .unchecked()
                .run()
                .await
                .map_err(git_err)?;
            r.output = joined(&o);
            if !o.ok() {
                r.ok = false;
                r.conflicts = conflicts_of(ctx).await;
                if r.conflicts.is_empty() {
                    return Err(format!("CANVAS_GIT: {}", r.output));
                }
            } else if action == "sync" {
                let p = omniget_core::core::vcs::actions::push(&root)
                    .await
                    .map_err(git_err)?;
                r.output = format!("{}\npush: {}", r.output, p.status);
            }
        }
        "fetch" => {
            r.output = run!(ctx.git().args(["fetch", "--all", "--prune"]).timeout(net));
        }
        "push" => {
            let p = omniget_core::core::vcs::actions::push(&root)
                .await
                .map_err(git_err)?;
            r.output = format!("{} {}", p.status, p.upstream);
        }
        "checkout" => {
            let b = ref_arg(a.branch.as_deref(), "branch")?;
            let remote = ctx
                .git()
                .args(["rev-parse", "--verify", "--quiet"])
                .arg(format!("refs/remotes/{b}"))
                .unchecked()
                .run()
                .await
                .map_err(git_err)?
                .ok();
            let local = ctx
                .git()
                .args(["rev-parse", "--verify", "--quiet"])
                .arg(format!("refs/heads/{b}"))
                .unchecked()
                .run()
                .await
                .map_err(git_err)?
                .ok();
            r.output = if remote && !local {
                let short = b.split_once('/').map(|(_, s)| s).unwrap_or(&b).to_string();
                let exists = ctx
                    .git()
                    .args(["rev-parse", "--verify", "--quiet"])
                    .arg(format!("refs/heads/{short}"))
                    .unchecked()
                    .run()
                    .await
                    .map_err(git_err)?
                    .ok();
                if exists {
                    run!(ctx.git().args(["switch", &short]))
                } else {
                    run!(ctx.git().args(["switch", "--track", &b]))
                }
            } else if local {
                run!(ctx.git().args(["switch", &b]))
            } else {
                run!(ctx.git().args(["checkout", &b, "--"]))
            };
        }
        "newBranch" => {
            let b = ref_arg(a.branch.as_deref(), "branch")?;
            let mut inv = ctx.git().args(["switch", "-c", &b]);
            if let Some(start) = a.start.as_deref().filter(|s| !s.trim().is_empty()) {
                inv = inv.arg(ref_arg(Some(start), "start")?);
            }
            r.output = run!(inv);
        }
        "deleteBranch" => {
            let b = ref_arg(a.branch.as_deref(), "branch")?;
            let flag = if a.force.unwrap_or(false) { "-D" } else { "-d" };
            r.output = run!(ctx.git().args(["branch", flag, &b]));
        }
        "merge" => {
            let b = ref_arg(a.branch.as_deref(), "branch")?;
            let o = ctx
                .git()
                .args(["merge", "--no-edit", &b])
                .timeout(net)
                .unchecked()
                .run()
                .await
                .map_err(git_err)?;
            r.output = joined(&o);
            if !o.ok() {
                r.ok = false;
                r.conflicts = conflicts_of(ctx).await;
                if r.conflicts.is_empty() {
                    return Err(format!("CANVAS_GIT: {}", r.output));
                }
            }
        }
        "mergeAbort" => {
            r.output = run!(ctx.git().args(["merge", "--abort"]));
        }
        "stashPush" => {
            let mut inv = ctx.git().args(["stash", "push"]);
            if a.untracked.unwrap_or(true) {
                inv = inv.arg("--include-untracked");
            }
            if let Some(m) = a.message.as_deref().filter(|m| !m.trim().is_empty()) {
                inv = inv.args(["-m", m.trim()]);
            }
            if !paths.is_empty() {
                inv = inv.arg("--").args(&paths);
            }
            r.output = run!(inv);
        }
        "stashApply" | "stashPop" | "stashDrop" => {
            let verb = &action[5..].to_lowercase();
            let at = format!("stash@{{{}}}", a.index.unwrap_or(0));
            let o = ctx
                .git()
                .args(["stash", verb.as_str(), &at])
                .unchecked()
                .run()
                .await
                .map_err(git_err)?;
            r.output = joined(&o);
            if !o.ok() {
                r.ok = false;
                r.conflicts = conflicts_of(ctx).await;
                if r.conflicts.is_empty() {
                    return Err(format!("CANVAS_GIT: {}", r.output));
                }
            }
        }
        "stashList" => {}
        "stage" => {
            need_paths(&paths, action)?;
            r.output = run!(ctx
                .git()
                .args(["--literal-pathspecs", "add", "-A", "--"])
                .args(&paths));
        }
        "unstage" => {
            need_paths(&paths, action)?;
            let has_head = ctx
                .git()
                .args(["rev-parse", "--verify", "--quiet", "HEAD"])
                .unchecked()
                .run()
                .await
                .map_err(git_err)?
                .ok();
            r.output = if has_head {
                run!(ctx
                    .git()
                    .args(["--literal-pathspecs", "restore", "--staged", "--"])
                    .args(&paths))
            } else {
                run!(ctx
                    .git()
                    .args(["--literal-pathspecs", "rm", "--cached", "-r", "-q", "--"])
                    .args(&paths))
            };
        }
        "discard" => {
            need_paths(&paths, action)?;
            let mut notes = Vec::new();
            for p in &paths {
                let tracked = ctx
                    .git()
                    .args(["--literal-pathspecs", "ls-files", "--error-unmatch", "--"])
                    .arg(p)
                    .unchecked()
                    .run()
                    .await
                    .map_err(git_err)?
                    .ok();
                let in_head = ctx
                    .git()
                    .args(["cat-file", "-e"])
                    .arg(format!("HEAD:./{p}"))
                    .unchecked()
                    .run()
                    .await
                    .map_err(git_err)?
                    .ok();
                if in_head {
                    run!(ctx
                        .git()
                        .args([
                            "--literal-pathspecs",
                            "restore",
                            "--source=HEAD",
                            "--staged",
                            "--worktree",
                            "--"
                        ])
                        .arg(p));
                    notes.push(format!("restored {p}"));
                } else {
                    if tracked {
                        run!(ctx
                            .git()
                            .args(["--literal-pathspecs", "rm", "--cached", "-r", "-q", "--"])
                            .arg(p));
                    }
                    let src = resolve_existing(&root, p)?;
                    std::fs::create_dir_all(trash).map_err(io)?;
                    let stamp = millis(Ok(SystemTime::now())).unwrap_or(0);
                    let name = src
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let dst = trash.join(format!("{stamp}-{name}"));
                    std::fs::rename(&src, &dst)
                        .or_else(|_| {
                            std::fs::copy(&src, &dst).and_then(|_| std::fs::remove_file(&src))
                        })
                        .map_err(io)?;
                    notes.push(format!("moved {p} to the trash"));
                }
            }
            r.output = notes.join("\n");
        }
        "conflict" => {
            need_paths(&paths, action)?;
            let p = &paths[0];
            let incoming = ctx
                .git()
                .args(["rev-parse", "--abbrev-ref", "MERGE_HEAD"])
                .unchecked()
                .run()
                .await
                .ok()
                .filter(|o| o.ok())
                .map(|o| o.trimmed())
                .filter(|s| !s.is_empty() && s != "MERGE_HEAD");
            let incoming = match incoming {
                Some(v) => Some(v),
                None => merge_source_name(ctx).await,
            };
            let head = ctx
                .git()
                .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
                .unchecked()
                .run()
                .await
                .ok()
                .filter(|o| o.ok())
                .map(|o| o.trimmed());
            r.conflict = Some(ConflictView {
                path: p.clone(),
                current_label: format!("Current ({})", head.unwrap_or_else(|| "HEAD".into())),
                incoming_label: format!(
                    "Incoming ({})",
                    incoming.unwrap_or_else(|| "MERGE_HEAD".into())
                ),
                base: stage_blob(ctx, 1, p).await,
                current: stage_blob(ctx, 2, p).await,
                incoming: stage_blob(ctx, 3, p).await,
            });
            r.ok = r
                .conflict
                .as_ref()
                .is_some_and(|c| c.current.is_some() || c.incoming.is_some());
        }
        "removeLock" => {
            let removed = remove_stale_lock(ctx, a.force.unwrap_or(false)).await?;
            r.ok = removed;
            r.output = if removed {
                "removed a stale index.lock".into()
            } else {
                "no stale index.lock".into()
            };
        }
        other => {
            return Err(format!(
                "CANVAS_FILES_INVALID: unknown git action '{other}' (one of: {})",
                GIT_ACTIONS.join(", ")
            ))
        }
    }
    if matches!(
        action,
        "stashList" | "stashPush" | "stashApply" | "stashPop" | "stashDrop"
    ) {
        let o = ctx
            .git()
            .args(["stash", "list"])
            .unchecked()
            .run()
            .await
            .map_err(git_err)?;
        r.stashes = Some(o.text().lines().map(str::to_string).collect());
    }
    if r.conflicts.is_empty() && is_write {
        r.conflicts = conflicts_of(ctx).await;
    }
    r.status = repo::status(&root).await.ok();
    Ok(r)
}

#[tauri::command]
pub async fn canvas_files_git_op(
    app: AppHandle,
    workspace_id: String,
    root: String,
    action: String,
    args: Option<GitOpArgs>,
) -> Result<GitOpResult, String> {
    let s = store(&app);
    let trash = s.workspace_dir(&workspace_id).join("trash").join("files");
    let root = blocking(move || resolve_root(&s, &workspace_id, &root)).await?;
    if !GIT_ACTIONS.contains(&action.as_str()) {
        return Err(format!(
            "CANVAS_FILES_INVALID: unknown git action '{action}' (one of: {})",
            GIT_ACTIONS.join(", ")
        ));
    }
    let paths = repo::discover(&root).await.map_err(git_err)?;
    let repo_root = std::fs::canonicalize(&paths.root).unwrap_or(paths.root.clone());
    let prefix = rel_of(&repo_root, &root);
    let ctx = GitCtx::Repo {
        work_tree: root.clone(),
    };
    git_op(&ctx, &prefix, &action, &args.unwrap_or_default(), &trash).await
}

#[tauri::command]
pub async fn canvas_files_kind(paths: Vec<String>) -> Result<Vec<bool>, String> {
    blocking(move || {
        Ok(paths
            .iter()
            .take(256)
            .map(|p| Path::new(p).is_dir())
            .collect())
    })
    .await
}

fn expand_local(p: &str, base: Option<&str>) -> Option<PathBuf> {
    let p = p.trim();
    if p.is_empty() || p.contains("://") || p.len() > 4096 {
        return None;
    }
    let p = p.strip_prefix("file://").unwrap_or(p);
    if let Some(rest) = p.strip_prefix("~/") {
        return dirs::home_dir().map(|h| h.join(rest));
    }
    let path = Path::new(p);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    let base = base.map(str::trim).filter(|b| !b.is_empty())?;
    let base = Path::new(base);
    base.is_absolute().then(|| base.join(path))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathHit {
    pub abs: String,
    pub is_dir: bool,
}

#[tauri::command]
pub async fn canvas_paths_probe(
    paths: Vec<String>,
    base: Option<String>,
) -> Result<Vec<Option<PathHit>>, String> {
    blocking(move || {
        Ok(paths
            .iter()
            .take(512)
            .map(|p| {
                let abs = expand_local(p, base.as_deref())?;
                let meta = std::fs::metadata(&abs).ok()?;
                Some(PathHit {
                    abs: abs.to_string_lossy().into_owned(),
                    is_dir: meta.is_dir(),
                })
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn canvas_image_data(
    path: String,
    base: Option<String>,
) -> Result<Option<String>, String> {
    blocking(move || {
        let Some(abs) = expand_local(&path, base.as_deref()) else {
            return Ok(None);
        };
        let Some(mime) = image_mime(&abs) else {
            return Ok(None);
        };
        let Ok(meta) = std::fs::metadata(&abs) else {
            return Ok(None);
        };
        if !meta.is_file() || meta.len() > MAX_IMAGE_BYTES {
            return Ok(None);
        }
        let bytes = std::fs::read(&abs).map_err(io)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(Some(format!("data:{mime};base64,{b64}")))
    })
    .await
}

#[tauri::command]
pub async fn canvas_files_definition(
    app: AppHandle,
    workspace_id: String,
    root: String,
    symbol: String,
    from: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SearchHit>, String> {
    let s = store(&app);
    blocking(move || {
        let root = resolve_root(&s, &workspace_id, &root)?;
        find_definitions(
            &root,
            &symbol,
            from.as_deref(),
            limit.unwrap_or(20).min(100),
        )
    })
    .await
}

fn definition_regex(symbol: &str) -> Result<regex::Regex, String> {
    let ok = !symbol.is_empty()
        && symbol.len() <= 120
        && symbol
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    if !ok {
        return Err("CANVAS_FILES_INVALID: not a symbol".into());
    }
    let w = regex::escape(symbol);
    let kw = r"(?:pub(?:\([^)]*\))?\s+)?(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:static\s+)?(?:unsafe\s+)?(?:const\s+)?";
    let decl = r"(?:fn|function\*?|def|class|struct|enum|trait|interface|type|const|let|var|val|mod|module|namespace|impl|func|protocol|extension|object|macro_rules!|union|record)";
    regex::Regex::new(&format!(
        r"^\s*(?:{kw}{decl}\s+{w}\b|(?:export\s+)?(?:const|let|var)\s+{w}\s*[:=]|{w}\s*[:=]\s*(?:async\s+)?(?:function\b|\([^)]*\)\s*=>)|#define\s+{w}\b)"
    ))
    .map_err(|e| format!("CANVAS_FILES_INVALID: {e}"))
}

fn find_definitions(
    root: &Path,
    symbol: &str,
    from: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>, String> {
    let re = definition_regex(symbol.trim())?;
    let mut hits = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_dir()
                && (SKIP_DIRS.contains(&name.as_ref()) || name.starts_with('.')))
        });
    let mut scanned = 0usize;
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        scanned += 1;
        if scanned > 20_000 {
            break;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        if looks_binary(&bytes) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let rel = rel_of(root, entry.path());
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                hits.push(SearchHit {
                    path: rel.clone(),
                    is_dir: false,
                    line: Some(i as u32 + 1),
                    preview: Some(line.trim().chars().take(160).collect()),
                });
            }
        }
        if hits.len() >= limit * 4 {
            break;
        }
    }
    hits.sort_by_key(|h| (Some(h.path.as_str()) != from, h.path.clone(), h.line));
    hits.truncate(limit);
    Ok(hits)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteArgs {
    #[serde(default)]
    pub environment: Option<serde_json::Value>,
    #[serde(default)]
    pub root: String,
    pub action: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub if_revision: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub content: Option<bool>,
    #[serde(default)]
    pub show_hidden: Option<bool>,
    #[serde(default)]
    pub dir: Option<bool>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub fn remote_env(
    s: &CanvasStore,
    ws: &str,
    environment: Option<&serde_json::Value>,
) -> Result<envs::EnvRecord, String> {
    let rec = match environment {
        None | Some(serde_json::Value::Null) => envs::load(s, ws),
        Some(serde_json::Value::String(w)) if w == "workspace" || w.is_empty() => envs::load(s, ws),
        Some(v) => envs::record_from_value(v)?,
    };
    let rec = envs::resolve(s, rec);
    if !rec.kind.is_remote() {
        return Err(
            "CANVAS_FILES_INVALID: this tree's environment runs on this machine (not remote)"
                .into(),
        );
    }
    Ok(rec)
}

pub fn node_is_remote(s: &CanvasStore, ws: &str, env: Option<&serde_json::Value>) -> bool {
    env.is_some_and(|v| !v.is_null()) && remote_env(s, ws, env).is_ok()
}

fn remote_join(root: &str, rel: &str) -> Result<String, String> {
    let clean = clean_rel(rel)?;
    let tail = clean
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let root = root.trim_end_matches('/');
    Ok(if tail.is_empty() {
        if root.is_empty() {
            "/".into()
        } else {
            root.to_string()
        }
    } else {
        format!("{root}/{tail}")
    })
}

const REMOTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

fn remote_cd(rec: &envs::EnvRecord, root: &str) -> String {
    let q = envs::sh_quote;
    if root.trim().is_empty() {
        match rec.dir.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(d) => format!("cd {} 2>/dev/null || cd \"$HOME\" || exit 3;", q(d)),
            None => "cd \"$HOME\" || exit 3;".into(),
        }
    } else {
        format!(
            "cd {} 2>/dev/null || {{ echo 'CANVAS_FILES_NOT_FOUND: no such folder' >&2; exit 3; }};",
            q(root)
        )
    }
}

async fn remote_run(
    rec: &envs::EnvRecord,
    script: String,
    stdin: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    envs::run_script(rec, &script, stdin, REMOTE_TIMEOUT).await
}

async fn remote_root(rec: &envs::EnvRecord, root: &str) -> Result<String, String> {
    let out = remote_run(rec, format!("{} pwd", remote_cd(rec, root)), None).await?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

fn remote_text(root: &str, rel: &str, abs: &str, bytes: Vec<u8>, size: u64) -> FileText {
    let p = Path::new(abs);
    let base = FileText {
        path: rel.to_string(),
        abs_path: abs.to_string(),
        size,
        revision: String::new(),
        binary: false,
        truncated: false,
        text: None,
        data_url: None,
        mime: None,
    };
    let _ = root;
    if let Some(mime) = image_mime(p).or_else(|| media_mime(p)) {
        if (bytes.len() as u64) < size {
            return FileText {
                binary: true,
                truncated: true,
                mime: Some(mime.into()),
                ..base
            };
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return FileText {
            binary: true,
            mime: Some(mime.into()),
            data_url: Some(format!("data:{mime};base64,{b64}")),
            ..base
        };
    }
    if size > MAX_TEXT_BYTES || looks_binary(&bytes) {
        return FileText {
            binary: true,
            truncated: size > MAX_TEXT_BYTES,
            ..base
        };
    }
    match String::from_utf8(bytes) {
        Ok(text) => FileText {
            revision: revision(&text),
            text: Some(text),
            ..base
        },
        Err(_) => FileText {
            binary: true,
            ..base
        },
    }
}

async fn remote_read(rec: &envs::EnvRecord, root: &str, rel: &str) -> Result<FileText, String> {
    let root = remote_root(rec, root).await?;
    let abs = remote_join(&root, rel)?;
    let p = Path::new(&abs);
    let cap = if image_mime(p).is_some() {
        MAX_IMAGE_BYTES
    } else if media_mime(p).is_some() {
        MAX_MEDIA_BYTES
    } else {
        MAX_TEXT_BYTES
    };
    let q = envs::sh_quote(&abs);
    let script = format!(
        "[ -d {q} ] && {{ echo 'CANVAS_FILES_INVALID: path is a folder' >&2; exit 4; }}; [ -f {q} ] || {{ echo 'CANVAS_FILES_NOT_FOUND: no such file' >&2; exit 4; }}; wc -c < {q} | tr -d ' '; head -c {cap} {q}"
    );
    let out = remote_run(rec, script, None).await?;
    let nl = out.iter().position(|b| *b == b'\n').unwrap_or(out.len());
    let size: u64 = String::from_utf8_lossy(&out[..nl])
        .trim()
        .parse()
        .unwrap_or(0);
    let body = out.get(nl + 1..).map(<[u8]>::to_vec).unwrap_or_default();
    let rel = clean_rel(rel)?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    Ok(remote_text(&root, &rel, &abs, body, size))
}

#[tauri::command]
pub async fn canvas_files_remote(
    app: AppHandle,
    workspace_id: String,
    args: RemoteArgs,
) -> Result<serde_json::Value, String> {
    let s = store(&app);
    remote_op(&s, &workspace_id, args).await
}

pub async fn remote_op(
    s: &CanvasStore,
    ws: &str,
    a: RemoteArgs,
) -> Result<serde_json::Value, String> {
    use serde_json::json;
    let rec = remote_env(s, ws, a.environment.as_ref())?;
    let q = envs::sh_quote;
    let rel = a.path.clone().unwrap_or_default();
    match a.action.as_str() {
        "resolve" => Ok(json!({ "root": remote_root(&rec, &a.root).await? })),
        "list" => {
            let dir_rel = clean_rel(&rel)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let sub = if dir_rel.is_empty() {
                String::new()
            } else {
                format!(
                    "cd {} 2>/dev/null || {{ echo 'CANVAS_FILES_NOT_FOUND: no such folder' >&2; exit 3; }};",
                    q(&dir_rel)
                )
            };
            let script = format!(
                "{cd} pwd; {sub} for f in * .[!.]* ..?*; do [ -e \"$f\" ] || [ -L \"$f\" ] || continue; l=0; [ -L \"$f\" ] && l=1; if [ -d \"$f\" ]; then printf 'd\\t%s\\t0\\t%s\\n' \"$l\" \"$f\"; else z=$(wc -c < \"$f\" 2>/dev/null | tr -d ' '); printf 'f\\t%s\\t%s\\t%s\\n' \"$l\" \"${{z:-0}}\" \"$f\"; fi; done | head -{MAX_LIST_ENTRIES}",
                cd = remote_cd(&rec, &a.root),
            );
            let out = String::from_utf8_lossy(&remote_run(&rec, script, None).await?).into_owned();
            let mut lines = out.lines();
            let root = lines.next().unwrap_or("").trim().to_string();
            let show_hidden = a.show_hidden.unwrap_or(false);
            let mut entries: Vec<FileEntry> = lines
                .filter_map(|l| {
                    let mut p = l.splitn(4, '\t');
                    let (k, link, size, name) = (p.next()?, p.next()?, p.next()?, p.next()?);
                    let hidden = name.starts_with('.');
                    if hidden && !show_hidden {
                        return None;
                    }
                    Some(FileEntry {
                        path: if dir_rel.is_empty() {
                            name.to_string()
                        } else {
                            format!("{dir_rel}/{name}")
                        },
                        name: name.to_string(),
                        is_dir: k == "d",
                        is_symlink: link == "1",
                        hidden,
                        size: size.parse().unwrap_or(0),
                        modified_ms: None,
                    })
                })
                .collect();
            entries.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            Ok(json!({ "root": root, "entries": entries }))
        }
        "read" => Ok(
            serde_json::to_value(remote_read(&rec, &a.root, &rel).await?)
                .map_err(|e| e.to_string())?,
        ),
        "write" => {
            let text = a.text.clone().unwrap_or_default();
            if let Some(want) = a.if_revision.as_deref().filter(|r| !r.is_empty()) {
                if let Ok(cur) = remote_read(&rec, &a.root, &rel).await {
                    if !cur.revision.is_empty() && cur.revision != want {
                        return Err(format!("CANVAS_FILE_CONFLICT: {}", cur.revision));
                    }
                }
            }
            let root = remote_root(&rec, &a.root).await?;
            let abs = remote_join(&root, &rel)?;
            remote_run(&rec, format!("cat > {}", q(&abs)), Some(text.into_bytes())).await?;
            Ok(serde_json::to_value(remote_read(&rec, &root, &rel).await?)
                .map_err(|e| e.to_string())?)
        }
        "create" => {
            let root = remote_root(&rec, &a.root).await?;
            let abs = remote_join(&root, &rel)?;
            let script = if a.dir.unwrap_or(false) {
                format!("[ -e {p} ] && {{ echo 'CANVAS_FILES_INVALID: already exists' >&2; exit 5; }}; mkdir -p {p}", p = q(&abs))
            } else {
                format!("[ -e {p} ] && {{ echo 'CANVAS_FILES_INVALID: already exists' >&2; exit 5; }}; : > {p}", p = q(&abs))
            };
            remote_run(&rec, script, None).await?;
            Ok(json!({ "ok": true, "path": rel }))
        }
        "rename" => {
            let root = remote_root(&rec, &a.root).await?;
            let from = remote_join(&root, &rel)?;
            let to = remote_join(&root, a.to.as_deref().unwrap_or(""))?;
            if to == root {
                return Err("CANVAS_FILES_INVALID: missing 'to'".into());
            }
            remote_run(
                &rec,
                format!(
                    "[ -e {t} ] && {{ echo 'CANVAS_FILES_INVALID: target exists' >&2; exit 5; }}; mv {f} {t}",
                    f = q(&from),
                    t = q(&to)
                ),
                None,
            )
            .await?;
            Ok(json!({ "ok": true }))
        }
        "delete" => {
            let root = remote_root(&rec, &a.root).await?;
            let abs = remote_join(&root, &rel)?;
            if abs == root || rel.trim().is_empty() {
                return Err("CANVAS_FILES_DENIED: will not delete the root".into());
            }
            remote_run(&rec, format!("rm -rf {}", q(&abs)), None).await?;
            Ok(json!({ "ok": true, "permanent": true }))
        }
        "search" => {
            let mut query = a.query.clone().unwrap_or_default().trim().to_lowercase();
            let mut content = a.content.unwrap_or(false);
            if let Some(rest) = query.strip_prefix('>') {
                query = rest.trim().to_string();
                content = true;
            }
            if query.is_empty() {
                return Ok(json!({ "hits": [] }));
            }
            let limit = a.limit.unwrap_or(MAX_SEARCH_HITS).min(MAX_SEARCH_HITS);
            let cd = remote_cd(&rec, &a.root);

            let mut hits: Vec<SearchHit> = Vec::new();
            if content {
                let names = SKIP_DIRS
                    .iter()
                    .map(|d| format!("-name {}", q(d)))
                    .collect::<Vec<_>>()
                    .join(" -o ");
                let script = format!(
                    "{cd} find . \\( {names} \\) -prune -o -type f -size -1024k -exec grep -Hins -F -e {} {{}} + 2>/dev/null | head -{}",
                    q(&query),
                    limit * 4
                );
                let out =
                    String::from_utf8_lossy(&remote_run(&rec, script, None).await?).into_owned();
                let show_hidden = a.show_hidden.unwrap_or(false);
                for l in out.lines() {
                    let l = l.trim_start_matches("./");
                    if hits.len() >= limit {
                        break;
                    }
                    let hidden = l
                        .split(':')
                        .next()
                        .is_some_and(|p| p.split('/').any(|c| c.starts_with('.')));
                    if hidden && !show_hidden {
                        continue;
                    }
                    let mut p = l.splitn(3, ':');
                    let (Some(path), Some(line), Some(text)) = (p.next(), p.next(), p.next())
                    else {
                        continue;
                    };
                    hits.push(SearchHit {
                        path: path.to_string(),
                        is_dir: false,
                        line: line.parse().ok(),
                        preview: Some(text.trim().chars().take(160).collect()),
                    });
                }
            } else {
                let names = SKIP_DIRS
                    .iter()
                    .map(|d| format!("-name {}", q(d)))
                    .collect::<Vec<_>>()
                    .join(" -o ");
                let script = format!(
                    "{cd} find . -maxdepth 12 \\( {names} \\) -prune -o -print 2>/dev/null | head -50000 | while IFS= read -r f; do if [ -d \"$f\" ]; then printf 'd\\t%s\\n' \"$f\"; else printf 'f\\t%s\\n' \"$f\"; fi; done"
                );
                let out =
                    String::from_utf8_lossy(&remote_run(&rec, script, None).await?).into_owned();
                let words: Vec<&str> = query.split_whitespace().collect();
                let show_hidden = a.show_hidden.unwrap_or(false);
                for l in out.lines() {
                    let Some((k, p)) = l.split_once('\t') else {
                        continue;
                    };
                    let rel = p.trim_start_matches("./");
                    if rel.is_empty() || rel == "." {
                        continue;
                    }
                    if !show_hidden && rel.split('/').any(|c| c.starts_with('.')) {
                        continue;
                    }
                    let is_dir = k == "d";
                    if name_matches(&words, rel, is_dir) {
                        hits.push(SearchHit {
                            path: rel.to_string(),
                            is_dir,
                            line: None,
                            preview: None,
                        });
                        if hits.len() >= limit {
                            break;
                        }
                    }
                }
            }
            Ok(json!({ "hits": hits }))
        }
        other => Err(format!(
            "CANVAS_FILES_INVALID: unknown remote action '{other}'"
        )),
    }
}
