use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::super::fsops;
use super::super::super::plan::{self, Op, PlanItem};
use super::super::super::protect::{self, Guard};
use super::super::super::store::Ctx;
use super::super::super::{Destination, Risk};
use super::{roots, skip_dir};

const MAX_BROKEN: usize = 1000;

fn is_litter(name: &str) -> bool {
    name == ".DS_Store"
        || name.eq_ignore_ascii_case("Thumbs.db")
        || name.eq_ignore_ascii_case("desktop.ini")
}

struct Walk<'a> {
    root: &'a Path,
    found: Vec<PathBuf>,
    broken: Vec<Value>,
    visited: u64,
}

impl Walk<'_> {
    fn visit(&mut self, dir: &Path) -> bool {
        self.visited += 1;
        if self.visited.is_multiple_of(1000) {
            super::super::super::progress(
                "find_empty_folders",
                "walk",
                self.visited,
                0,
                dir.to_string_lossy(),
            );
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut empty = true;
        let mut empty_children = Vec::new();
        for e in rd.flatten() {
            let path = e.path();
            let Ok(m) = std::fs::symlink_metadata(&path) else {
                empty = false;
                continue;
            };
            if fsops::is_link(&m) {
                empty = false;
                if m.file_type().is_symlink() && self.broken.len() < MAX_BROKEN {
                    if let Err(err) = std::fs::metadata(&path) {
                        if err.kind() == std::io::ErrorKind::NotFound {
                            let target = std::fs::read_link(&path)
                                .map(|t| t.to_string_lossy().to_string())
                                .unwrap_or_default();
                            self.broken.push(json!({ "path": path, "target": target }));
                        }
                    }
                }
                continue;
            }
            if m.is_dir() {
                if skip_dir(&path, self.root) {
                    empty = false;
                } else if self.visit(&path) {
                    empty_children.push(path);
                } else {
                    empty = false;
                }
            } else if !is_litter(&e.file_name().to_string_lossy()) {
                empty = false;
            }
        }
        if empty && dir != self.root && protect::check(dir, Guard::Picked, Some(self.root)).is_ok()
        {
            return true;
        }
        self.found.extend(empty_children);
        false
    }
}

pub fn find_empty_folders(ctx: &Ctx, a: &Value) -> Result<Value, String> {
    let (roots, skipped_roots) = roots(a);
    if roots.is_empty() {
        return Err("no folder to search: pass roots that exist".into());
    }
    let mut items = Vec::new();
    let mut broken = Vec::new();
    for root in &roots {
        let mut w = Walk {
            root,
            found: Vec::new(),
            broken: Vec::new(),
            visited: 0,
        };
        w.visit(root);
        broken.extend(w.broken);
        for dir in w.found {
            if protect::check(&dir, Guard::Picked, Some(root)).is_err() {
                continue;
            }
            let Ok(fp) = fsops::fingerprint(&dir) else {
                continue;
            };
            items.push(PlanItem {
                id: format!("empty_folders:{}", dir.display()),
                label: dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                detail: Some(dir.display().to_string()),
                group: "empty_folders".into(),
                bytes: fp.size,
                risk: Risk::Reversible,
                admin: false,
                op: Op::RemovePath {
                    path: dir,
                    destination: Destination::Trash,
                    fp,
                    root: Some(root.clone()),
                    guard: Guard::Picked,
                    contents_only: false,
                },
            });
        }
    }
    items.sort_by(|a, b| a.id.cmp(&b.id));
    broken.truncate(MAX_BROKEN);
    let total_bytes: u64 = items.iter().map(|i| i.bytes).sum();
    let preview: Vec<Value> = items
        .iter()
        .map(|it| json!({ "id": it.id, "label": it.label, "detail": it.detail, "group": it.group, "bytes": it.bytes, "risk": it.risk }))
        .collect();
    super::super::super::progress("find_empty_folders", "done", 1, 1, "");
    let plan_id = if items.is_empty() {
        None
    } else {
        Some(
            plan::offer(
                ctx,
                "empty_folders",
                items,
                super::super::super::plan::source_of(a),
            )?
            .id,
        )
    };
    Ok(json!({
        "items": preview,
        "total_bytes": total_bytes,
        "plan_id": plan_id,
        "broken_links": broken,
        "roots": roots,
        "skipped_roots": skipped_roots,
    }))
}
