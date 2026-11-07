//! kondo-lib 0.9, <https://github.com/tbillington/kondo>, MIT License,
//! Copyright (c) 2020 Trent Billington. Changes from upstream:

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{json, Value};

use super::super::fsops;
use super::super::plan::{self, Op, PlanItem};
use super::super::protect::{self, Guard};
use super::super::store::Ctx;
use super::super::tools::blocking;
use super::super::{Destination, Risk};

pub struct Kind {
    pub id: &'static str,
    pub name: &'static str,
    pub dirs: &'static [&'static str],
}

pub const KINDS: &[Kind] = &[
    Kind {
        id: "cargo",
        name: "Cargo",
        dirs: &["target", ".xwin-cache"],
    },
    Kind {
        id: "node",
        name: "Node",
        dirs: &["node_modules", ".angular"],
    },
    Kind {
        id: "react_native",
        name: "Node (React Native)",
        dirs: &[
            "node_modules",
            "android/build",
            "android/.gradle",
            "ios/build",
            "ios/DerivedData",
            "ios/Pods",
            ".expo",
            ".metro",
        ],
    },
    Kind {
        id: "unity",
        name: "Unity",
        dirs: &[
            "Library",
            "Temp",
            "Obj",
            "Logs",
            "MemoryCaptures",
            "Build",
            "Builds",
        ],
    },
    Kind {
        id: "stack",
        name: "Stack",
        dirs: &[".stack-work"],
    },
    Kind {
        id: "cabal",
        name: "Cabal",
        dirs: &["dist-newstyle"],
    },
    Kind {
        id: "sbt",
        name: "SBT",
        dirs: &["target", "project/target"],
    },
    Kind {
        id: "maven",
        name: "Maven",
        dirs: &["target"],
    },
    Kind {
        id: "gradle",
        name: "Gradle",
        dirs: &["build", ".gradle"],
    },
    Kind {
        id: "cmake",
        name: "CMake",
        dirs: &["build", "cmake-build-debug", "cmake-build-release"],
    },
    Kind {
        id: "unreal",
        name: "Unreal",
        dirs: &[
            "Binaries",
            "Build",
            "Saved",
            "DerivedDataCache",
            "Intermediate",
        ],
    },
    Kind {
        id: "jupyter",
        name: "Jupyter",
        dirs: &[".ipynb_checkpoints"],
    },
    Kind {
        id: "python",
        name: "Python",
        dirs: &[
            ".mypy_cache",
            ".nox",
            ".pytest_cache",
            ".ruff_cache",
            ".tox",
            ".hypothesis",
            "__pycache__",
            "__pypackages__",
        ],
    },
    Kind {
        id: "pixi",
        name: "Pixi",
        dirs: &[".pixi"],
    },
    Kind {
        id: "composer",
        name: "Composer",
        dirs: &["vendor"],
    },
    Kind {
        id: "pub",
        name: "Pub",
        dirs: &[
            "build",
            ".dart_tool",
            "linux/flutter/ephemeral",
            "windows/flutter/ephemeral",
        ],
    },
    Kind {
        id: "elixir",
        name: "Elixir",
        dirs: &["_build", ".elixir-tools", ".elixir_ls", ".lexical"],
    },
    Kind {
        id: "swift",
        name: "Swift",
        dirs: &[".build", ".swiftpm"],
    },
    Kind {
        id: "zig",
        name: "Zig",
        dirs: &["zig-cache", ".zig-cache", "zig-out"],
    },
    Kind {
        id: "godot4",
        name: "Godot 4.x",
        dirs: &[".godot"],
    },
    Kind {
        id: "dotnet",
        name: ".NET",
        dirs: &["bin", "obj"],
    },
    Kind {
        id: "turborepo",
        name: "Turborepo",
        dirs: &[".turbo"],
    },
    Kind {
        id: "terraform",
        name: "Terraform",
        dirs: &[".terraform"],
    },
    Kind {
        id: "cocoapods",
        name: "CocoaPods",
        dirs: &["Pods"],
    },
];

fn kind(id: &str) -> usize {
    KINDS
        .iter()
        .position(|k| k.id == id)
        .expect("kind in table")
}

pub fn detect(files: &[String], subdirs: &[String]) -> Vec<usize> {
    let has = |n: &str| files.iter().any(|f| f == n);
    let ends = |s: &str| files.iter().any(|f| f.ends_with(s));
    let sub = |n: &str| subdirs.iter().any(|d| d == n);
    let mut out = Vec::new();
    let mut add = |id: &str| {
        let i = kind(id);
        if !out.contains(&i) {
            out.push(i);
        }
    };
    if has("Cargo.toml") {
        add("cargo");
    }
    if has("package.json") {
        add(if sub("ios") || sub("android") {
            "react_native"
        } else {
            "node"
        });
    }
    if has("Assembly-CSharp.csproj") {
        add("unity");
    }
    if has("stack.yaml") {
        add("stack");
    }
    if has("cabal.project") {
        add("cabal");
    }
    if has("build.sbt") {
        add("sbt");
    }
    if has("pom.xml") {
        add("maven");
    }
    if has("build.gradle") || has("build.gradle.kts") {
        add("gradle");
    }
    if has("CMakeLists.txt") {
        add("cmake");
    }
    if ends(".uproject") {
        add("unreal");
    }
    if ends(".ipynb") {
        add("jupyter");
    }
    let python = [
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "Pipfile",
        "tox.ini",
    ]
    .iter()
    .any(|n| has(n))
        || files
            .iter()
            .any(|f| f.starts_with("requirements") && f.ends_with(".txt"));
    if python {
        add("python");
    }
    if has("pixi.toml") {
        add("pixi");
    }
    if has("composer.json") {
        add("composer");
    }
    if has("pubspec.yaml") {
        add("pub");
    }
    if has("mix.exs") {
        add("elixir");
    }
    if has("Package.swift") {
        add("swift");
    }
    if has("build.zig") {
        add("zig");
    }
    if has("project.godot") {
        add("godot4");
    }
    if (ends(".csproj") || ends(".fsproj"))
        && !has("project.godot")
        && !has("Assembly-CSharp.csproj")
    {
        add("dotnet");
    }
    if has("turbo.json") {
        add("turborepo");
    }
    if has(".terraform.lock.hcl") {
        add("terraform");
    }
    if has("Podfile") {
        add("cocoapods");
    }
    out
}

struct Project {
    path: PathBuf,
    kinds: Vec<usize>,
    newest: SystemTime,
    artifacts: Vec<(usize, PathBuf)>,
}

fn is_artifact_of(p: &Project, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(&p.path) else {
        return false;
    };
    p.kinds
        .iter()
        .any(|&k| KINDS[k].dirs.iter().any(|d| rel == Path::new(d)))
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn list(dir: &Path) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let Some(name) = e.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if ft.is_file() {
                files.push(name);
            } else if ft.is_dir() {
                dirs.push(name);
            }
        }
    }
    (files, dirs)
}

fn real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !fsops::is_link(&m))
}

fn walk_root(
    root: &Path,
    projects: &mut Vec<Project>,
    claimed: &mut HashSet<PathBuf>,
    visited: &mut u64,
) {
    let python = kind("python");
    let mut it = walkdir::WalkDir::new(root)
        .follow_links(false)
        .same_file_system(true)
        .into_iter();
    let mut stack: Vec<usize> = Vec::new();
    let pop = |stack: &mut Vec<usize>, projects: &mut Vec<Project>| {
        if let Some(child) = stack.pop() {
            if let Some(&parent) = stack.last() {
                let t = projects[child].newest;
                if t > projects[parent].newest {
                    projects[parent].newest = t;
                }
            }
        }
    };
    while let Some(entry) = it.next() {
        let Ok(e) = entry else { continue };
        let path = e.path();
        while let Some(&top) = stack.last() {
            if path.starts_with(&projects[top].path) {
                break;
            }
            pop(&mut stack, projects);
        }
        let ft = e.file_type();
        if ft.is_symlink() {
            continue;
        }
        let mtime = e.metadata().ok().and_then(|m| m.modified().ok());
        let bump = |projects: &mut Vec<Project>, stack: &[usize]| {
            if let (Some(&top), Some(t)) = (stack.last(), mtime) {
                if t > projects[top].newest {
                    projects[top].newest = t;
                }
            }
        };
        if !ft.is_dir() {
            bump(projects, &stack);
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if e.depth() > 0 {
            if is_hidden(&name) || name == "node_modules" {
                it.skip_current_dir();
                continue;
            }
            if stack.iter().any(|&i| is_artifact_of(&projects[i], path)) {
                it.skip_current_dir();
                continue;
            }
            if name == "__pycache__" {
                if let Some(&owner) = stack
                    .iter()
                    .rev()
                    .find(|&&i| projects[i].kinds.contains(&python))
                {
                    if claimed.insert(path.to_path_buf()) {
                        projects[owner].artifacts.push((python, path.to_path_buf()));
                    }
                }
                it.skip_current_dir();
                continue;
            }
        }
        *visited += 1;
        if (*visited).is_multiple_of(500) {
            super::super::progress(
                "dev_artifacts_scan",
                "walk",
                *visited,
                0,
                path.to_string_lossy(),
            );
        }
        bump(projects, &stack);
        let (files, subdirs) = list(path);
        let kinds = detect(&files, &subdirs);
        if kinds.is_empty() {
            continue;
        }
        let mut artifacts = Vec::new();
        for &k in &kinds {
            for d in KINDS[k].dirs {
                let p = path.join(d);
                if real_dir(&p) && claimed.insert(p.clone()) {
                    artifacts.push((k, p));
                }
            }
        }
        projects.push(Project {
            path: path.to_path_buf(),
            kinds,
            newest: mtime.unwrap_or(SystemTime::UNIX_EPOCH),
            artifacts,
        });
        stack.push(projects.len() - 1);
    }
    while !stack.is_empty() {
        pop(&mut stack, projects);
    }
}

pub fn default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        "Documents",
        "Developer",
        "Projects",
        "code",
        "src",
        "dev",
        "repos",
        "workspace",
        "git",
    ]
    .iter()
    .map(|d| home.join(d))
    .filter(|p| real_dir(p))
    .collect()
}

pub fn roots_from(a: &Value, defaults: fn() -> Vec<PathBuf>) -> (Vec<PathBuf>, Vec<String>) {
    let mut skipped = Vec::new();
    let given: Vec<String> = a["roots"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut roots: Vec<PathBuf> = if given.is_empty() {
        defaults()
    } else {
        let home = dirs::home_dir();
        given
            .into_iter()
            .filter_map(|s| {
                let p = match (s.strip_prefix("~/"), &home) {
                    (Some(rest), Some(h)) => h.join(rest),
                    _ if s == "~" => home.clone()?,
                    _ => PathBuf::from(&s),
                };
                if p.is_absolute() && real_dir(&p) && protect::normalize(&p).is_some() {
                    Some(p)
                } else {
                    skipped.push(s);
                    None
                }
            })
            .collect()
    };
    roots.sort();
    roots.dedup();
    let all = roots.clone();
    roots.retain(|r| !all.iter().any(|o| o != r && r.starts_with(o)));
    (roots, skipped)
}

fn age_days(t: SystemTime) -> u64 {
    t.elapsed().map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

pub fn scan_with(ctx: &Ctx, a: &Value) -> Result<Value, String> {
    let (roots, skipped_roots) = roots_from(a, default_roots);
    let older = a["older_than_days"].as_u64().unwrap_or(0);
    super::super::progress("dev_artifacts_scan", "start", 0, roots.len() as u64, "");
    let mut projects = Vec::new();
    let mut claimed = HashSet::new();
    let mut visited = 0u64;
    for r in &roots {
        walk_root(r, &mut projects, &mut claimed, &mut visited);
    }
    projects.retain(|p| !p.artifacts.is_empty() && age_days(p.newest) >= older);

    let total = projects
        .iter()
        .map(|p| p.artifacts.len() as u64)
        .sum::<u64>();
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    let mut out_projects = Vec::new();
    let mut done = 0u64;
    for p in &projects {
        let age = age_days(p.newest);
        let project_name = p
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut bytes = 0u64;
        for (k, dir) in &p.artifacts {
            done += 1;
            super::super::progress(
                "dev_artifacts_scan",
                "measure",
                done,
                total,
                dir.to_string_lossy(),
            );
            if protect::check(dir, Guard::Picked, Some(&p.path)).is_err() {
                continue;
            }
            let Ok(fp) = fsops::fingerprint(dir) else {
                continue;
            };
            let (b, _) = fsops::measure_with(dir, &mut seen);
            bytes += b;
            let rel = dir
                .strip_prefix(&p.path)
                .unwrap_or(dir)
                .to_string_lossy()
                .to_string();
            items.push(PlanItem {
                id: format!("dev_artifacts:{}:{}", KINDS[*k].id, dir.display()),
                label: format!("{project_name}/{rel}"),
                detail: Some(format!(
                    "{} · {} days since last change",
                    p.path.display(),
                    age
                )),
                group: KINDS[*k].name.to_string(),
                bytes: b,
                risk: Risk::Destructive,
                admin: false,
                op: Op::RemovePath {
                    path: dir.clone(),
                    destination: Destination::Unlink,
                    fp,
                    root: Some(p.path.clone()),
                    guard: Guard::Picked,
                    contents_only: false,
                },
            });
        }
        out_projects.push(json!({
            "path": p.path,
            "kind": KINDS[p.kinds[0]].name,
            "kinds": p.kinds.iter().map(|&k| KINDS[k].name).collect::<Vec<_>>(),
            "bytes": bytes,
            "age_days": age,
        }));
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.bytes));
    out_projects.sort_by_key(|p| std::cmp::Reverse(p["bytes"].as_u64()));
    let total_bytes: u64 = items.iter().map(|i| i.bytes).sum();
    super::super::progress("dev_artifacts_scan", "done", total, total, "");
    let preview: Vec<Value> = items
        .iter()
        .map(|it| json!({ "id": it.id, "label": it.label, "detail": it.detail, "group": it.group, "bytes": it.bytes, "risk": it.risk }))
        .collect();
    let plan_id = if items.is_empty() {
        None
    } else {
        Some(plan::offer(ctx, "dev_artifacts", items, plan::source_of(a))?.id)
    };
    Ok(json!({
        "items": preview,
        "total_bytes": total_bytes,
        "plan_id": plan_id,
        "projects": out_projects,
        "roots": roots,
        "skipped_roots": skipped_roots,
    }))
}

pub async fn tool_scan(a: Value) -> Result<Value, String> {
    blocking(move || scan_with(&Ctx::app()?, &a)).await
}
