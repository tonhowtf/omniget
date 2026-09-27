//! Disk: volumes (with macOS purgeable space), a usage tree in the style of
//! dust with drill-down from memory, the largest files as a Trash plan, and
//! physical disk health. Everything here only reads; the one change, moving
//! large files to the Trash, goes through a plan.

pub mod health;
pub mod large;
pub mod plist;
pub mod tree;
pub mod volumes;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::plan::{self, Source};
use super::store::Ctx;
use super::tools::blocking;

fn source_of(a: &Value) -> Source {
    match a.get("_source").and_then(Value::as_str) {
        Some("ui") => Source::Ui,
        Some("external") => Source::External,
        Some("cli") => Source::Cli,
        _ => Source::Agent,
    }
}

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "no home folder".to_string())
}

/// A folder the caller named: `~` expanded, absolute, an existing folder.
fn folder(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    let p = if raw == "~" {
        home()?
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home()?.join(rest)
    } else {
        PathBuf::from(raw)
    };
    if !p.is_absolute() {
        return Err(format!("BAD_PATH: {raw} is not an absolute path"));
    }
    if !p.is_dir() {
        return Err(format!("NOT_A_FOLDER: {}", p.display()));
    }
    // Resolve the root itself (`/tmp` → `/private/tmp`) so drill-down paths
    // match the scan; the walk below never follows a link.
    #[cfg(unix)]
    let p = std::fs::canonicalize(&p).unwrap_or(p);
    Ok(p)
}

pub async fn tool_volumes(_a: Value) -> Result<Value, String> {
    blocking(|| Ok(json!({ "volumes": volumes::list() }))).await
}

pub async fn tool_usage(a: Value) -> Result<Value, String> {
    let path = match a
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        Some(p) => folder(p)?,
        None => home()?,
    };
    let depth = a
        .get("depth")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .clamp(1, 6) as u32;
    let min_fraction = a
        .get("min_fraction")
        .and_then(Value::as_f64)
        .unwrap_or(0.01)
        .clamp(0.0, 0.5);
    let refresh = a.get("refresh").and_then(Value::as_bool).unwrap_or(false);
    blocking(move || usage(&path, depth, min_fraction, refresh)).await
}

fn usage(path: &Path, depth: u32, min_fraction: f64, refresh: bool) -> Result<Value, String> {
    let cached = if refresh {
        None
    } else {
        tree::cached_for(path)
    };
    let (scan, from_cache) = match cached {
        Some(s) if tree::find(&s, path).is_some() => (s, true),
        _ => (tree::remember(tree::scan(path)?), false),
    };
    let node = tree::find(&scan, path).ok_or("NOT_FOUND: folder not in the scan")?;
    let threshold = (node.bytes as f64 * min_fraction) as u64;
    let mut v = tree::view(node, path, depth, threshold);
    v["root"] = json!(scan.root.display().to_string());
    v["scanned_entries"] = json!(scan.entries);
    v["errors"] = json!({ "permission_denied": scan.denied, "other": scan.errors });
    v["cached"] = json!(from_cache);
    v["scanned_at"] = json!(scan.scanned_at);
    v["depth"] = json!(depth);
    v["min_fraction"] = json!(min_fraction);
    v["apparent"] = json!(false);
    Ok(v)
}

pub async fn tool_health(_a: Value) -> Result<Value, String> {
    blocking(|| Ok(health::health())).await
}

pub async fn tool_large_files(a: Value) -> Result<Value, String> {
    let roots: Vec<PathBuf> = match a.get("roots").and_then(Value::as_array) {
        Some(r) if !r.is_empty() => r
            .iter()
            .filter_map(Value::as_str)
            .take(20)
            .map(folder)
            .collect::<Result<_, _>>()?,
        _ => vec![home()?],
    };
    let min_mb = a
        .get("min_mb")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .max(1);
    let limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 500) as usize;
    let source = source_of(&a);
    blocking(move || {
        let ctx = Ctx::app()?;
        large_files(&ctx, &roots, min_mb, limit, source)
    })
    .await
}

fn large_files(
    ctx: &Ctx,
    roots: &[PathBuf],
    min_mb: u64,
    limit: usize,
    source: Source,
) -> Result<Value, String> {
    let (found, scanned) = large::find(roots, min_mb.saturating_mul(1_000_000), limit);
    let pairs = large::items(&found);
    let items: Vec<Value> = pairs
        .iter()
        .map(|(it, f)| large::item_json(it, f))
        .collect();
    let total: u64 = pairs.iter().map(|(it, _)| it.bytes).sum();
    let plan_id = if pairs.is_empty() {
        None
    } else {
        Some(
            plan::offer(
                ctx,
                "large_files",
                pairs.into_iter().map(|(it, _)| it).collect(),
                source,
            )?
            .id,
        )
    };
    Ok(json!({
        "items": items,
        "total_bytes": total,
        "plan_id": plan_id,
        "scanned": scanned,
        "roots": roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>(),
        "min_mb": min_mb,
    }))
}

/// Actions only the screen may run (`washmybelly_ui` with `disk.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    match action {
        "reveal" => {
            let raw = a
                .get("path")
                .and_then(Value::as_str)
                .ok_or("path is required")?;
            let p = PathBuf::from(raw);
            if !p.is_absolute() || std::fs::symlink_metadata(&p).is_err() {
                return Err(format!("NOT_FOUND: {raw}"));
            }
            blocking(move || reveal(&p).map(|_| json!({ "ok": true }))).await
        }
        _ => Err(format!("disk: unknown screen action {action}")),
    }
}

/// Show a file or folder in the system file manager. The path is absolute
/// (checked above), so it can never be read as an option.
fn reveal(p: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::core::process::std_command("open")
            .arg("-R")
            .arg(p)
            .status()
            .map_err(|e| e.to_string())
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err("open -R failed".into())
                }
            })
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer wants `/select,"<path>"` as one raw argument; a Windows
        // path cannot hold a double quote, so nothing escapes the quotes.
        let s = p.display().to_string();
        if s.contains('"') {
            return Err("bad path".into());
        }
        crate::core::process::std_command("explorer")
            .raw_arg(format!("/select,\"{s}\""))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = if p.is_dir() {
            p.to_path_buf()
        } else {
            p.parent().unwrap_or(p).to_path_buf()
        };
        crate::core::process::std_command("xdg-open")
            .arg(dir)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_answers_drill_down_from_cache() {
        let d = tempfile::tempdir().unwrap();
        let r = folder(&d.path().display().to_string()).unwrap();
        std::fs::create_dir_all(r.join("a/b")).unwrap();
        std::fs::write(r.join("a/b/f.bin"), vec![1u8; 50_000]).unwrap();
        let first = usage(&r, 2, 0.0, false).unwrap();
        assert_eq!(first["cached"], false);
        assert_eq!(first["files"], 1);
        assert!(first["name"].is_string() && first["children"].is_array());
        let sub = usage(&r.join("a/b"), 2, 0.0, false).unwrap();
        assert_eq!(sub["cached"], true);
        assert_eq!(sub["files"], 1);
        let again = usage(&r, 2, 0.0, true).unwrap();
        assert_eq!(again["cached"], false);
    }

    #[test]
    fn folder_rules() {
        assert!(folder("relative/path").unwrap_err().starts_with("BAD_PATH"));
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        std::fs::write(&f, "x").unwrap();
        assert!(folder(&f.display().to_string())
            .unwrap_err()
            .starts_with("NOT_A_FOLDER"));
    }

    #[test]
    fn large_files_offer_a_plan() {
        let d = tempfile::tempdir().unwrap();
        let root = folder(&d.path().display().to_string())
            .unwrap()
            .join("files");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("movie.mov"), vec![1u8; 2_100_000]).unwrap();
        std::fs::write(root.join("small.txt"), vec![1u8; 10]).unwrap();
        let ctx = Ctx::at(&d.path().join("state"));
        let v = large_files(&ctx, std::slice::from_ref(&root), 1, 10, Source::Ui).unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 1);
        assert!(v["plan_id"].is_string());
        assert_eq!(v["items"][0]["group"], "video");
        let none = large_files(&ctx, &[root], 500, 10, Source::Ui).unwrap();
        assert!(none["plan_id"].is_null());
        assert_eq!(none["total_bytes"], 0);
    }

    #[tokio::test]
    async fn unknown_screen_action() {
        assert!(ui_action("nope", json!({})).await.is_err());
        assert!(ui_action("reveal", json!({ "path": "relative" }))
            .await
            .is_err());
    }
}
