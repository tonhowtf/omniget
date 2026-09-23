//! Project stack detection for the wizard and for global agents (the "PROJECT
//! CONTEXT" block). Port of cct's `detectProject` without its bugs (estudo 75
//! 01 §2.11): every language found is reported (not only the first), the
//! Django/Flask file checks really look at the files, frameworks come from the
//! manifests of each ecosystem (package.json, pyproject/requirements/Pipfile,
//! Gemfile, Cargo.toml, go.mod), and the scan skips dependency and build dirs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Directories never scanned.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "venv",
    "env",
    "__pycache__",
    "dist",
    "build",
    "out",
    ".git",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Detected {
    pub id: String,
    /// Why (`package.json: react`, `Cargo.toml`, `*.py`).
    pub evidence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StackReport {
    pub root: PathBuf,
    /// `javascript`, `typescript`, `python`, `ruby`, `rust`, `go`.
    pub languages: Vec<Detected>,
    /// `react`, `next`, `vue`, `nuxt`, `angular`, `svelte`, `sveltekit`,
    /// `express`, `fastify`, `koa`, `nestjs`, `django`, `flask`, `fastapi`,
    /// `rails`, `sinatra`, `tauri`, `axum`, `actix`, `gin`, `echo`, `fiber`.
    pub frameworks: Vec<Detected>,
    /// `npm`, `pnpm`, `yarn`, `bun`, `pip`, `poetry`, `uv`, `bundler`, `cargo`, `go`.
    pub package_managers: Vec<String>,
    pub build_command: Option<String>,
    pub test_command: Option<String>,
    pub lint_command: Option<String>,
    pub dev_command: Option<String>,
    pub has_git: bool,
    /// Top-level folders (structure section of AGENTS.md).
    pub top_dirs: Vec<String>,
    /// Agent config already present (`CLAUDE.md`, `AGENTS.md`, `.cursor` …).
    pub agent_files: Vec<String>,
    pub name: Option<String>,
    pub description: Option<String>,
}

impl StackReport {
    pub fn has_language(&self, id: &str) -> bool {
        self.languages.iter().any(|l| l.id == id)
    }
    pub fn has_framework(&self, id: &str) -> bool {
        self.frameworks.iter().any(|l| l.id == id)
    }
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

/// Files up to `depth` levels under `root`, skipping dot dirs and [`SKIP_DIRS`].
fn walk(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if depth > 0 && !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                walk(&p, depth - 1, out);
            }
        } else if ft.is_file() {
            out.push(p);
            if out.len() > 20_000 {
                return;
            }
        }
    }
}

fn push(list: &mut Vec<Detected>, id: &str, evidence: impl Into<String>) {
    if !list.iter().any(|d| d.id == id) {
        list.push(Detected {
            id: id.into(),
            evidence: evidence.into(),
        });
    }
}

/// Dependency names of a package.json (deps + devDeps + peerDeps).
fn npm_deps(pkg: &Value) -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    for k in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(o) = pkg.get(k).and_then(|v| v.as_object()) {
            s.extend(o.keys().cloned());
        }
    }
    s
}

/// Python requirement names mentioned in any of the manifests (lowercase).
fn python_deps(root: &Path) -> (BTreeSet<String>, Vec<&'static str>) {
    let mut names = BTreeSet::new();
    let mut sources = Vec::new();
    let mut take = |text: &str| {
        for line in text.lines() {
            let l = line
                .trim()
                .trim_start_matches(['"', '\''])
                .to_ascii_lowercase();
            if l.is_empty() || l.starts_with('#') || l.starts_with('[') {
                continue;
            }
            let name: String = l
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
                .collect();
            if !name.is_empty() {
                names.insert(name.replace('_', "-"));
            }
        }
    };
    for (f, label) in [
        ("requirements.txt", "requirements.txt"),
        ("requirements-dev.txt", "requirements-dev.txt"),
        ("pyproject.toml", "pyproject.toml"),
        ("Pipfile", "Pipfile"),
        ("setup.py", "setup.py"),
        ("setup.cfg", "setup.cfg"),
    ] {
        if let Some(t) = read(&root.join(f)) {
            sources.push(label);
            // pyproject: dependency arrays are quoted strings; lines like
            // `"django>=5",` become `django`.
            take(&t);
        }
    }
    (names, sources)
}

fn gem_names(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("gem ")?;
            let name = rest.trim().trim_start_matches(['"', '\'']);
            let end = name.find(['"', '\'']).unwrap_or(name.len());
            Some(name[..end].to_ascii_lowercase())
        })
        .collect()
}

fn cargo_deps(text: &str) -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    let mut in_deps = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_deps = l.contains("dependencies");
            continue;
        }
        if in_deps {
            if let Some((k, _)) = l.split_once('=') {
                s.insert(k.trim().trim_matches('"').to_string());
            }
        }
    }
    s
}

fn go_requires(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.contains('/') && !l.starts_with("module") && !l.starts_with("//"))
        .map(|l| l.trim_start_matches("require").trim().to_string())
        .collect()
}

/// Detects the stack of `root` (scan depth 2).
pub fn detect(root: &Path) -> StackReport {
    let mut r = StackReport {
        root: root.to_path_buf(),
        has_git: root.join(".git").exists(),
        ..Default::default()
    };
    let mut files = Vec::new();
    walk(root, 2, &mut files);
    let ext = |e: &str| {
        files
            .iter()
            .any(|p| p.extension().and_then(|x| x.to_str()) == Some(e))
    };
    let exists = |p: &str| root.join(p).exists();

    // Top dirs and agent files.
    if let Ok(rd) = std::fs::read_dir(root) {
        let mut dirs: Vec<String> = rd
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.') && !SKIP_DIRS.contains(&n.as_str()))
            .collect();
        dirs.sort();
        dirs.truncate(24);
        r.top_dirs = dirs;
    }
    for f in [
        "AGENTS.md",
        "CLAUDE.md",
        "GEMINI.md",
        ".claude",
        ".codex",
        ".cursor",
        ".cursorrules",
        ".github/copilot-instructions.md",
        ".gemini",
        ".opencode",
        "opencode.json",
        ".windsurf",
        ".kiro",
        ".mcp.json",
    ] {
        if exists(f) {
            r.agent_files.push(f.to_string());
        }
    }

    // JavaScript / TypeScript
    if let Some(pkg) =
        read(&root.join("package.json")).and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        r.name = pkg["name"].as_str().map(str::to_string);
        r.description = pkg["description"].as_str().map(str::to_string);
        let deps = npm_deps(&pkg);
        let ts = deps.contains("typescript") || exists("tsconfig.json");
        push(&mut r.languages, "javascript", "package.json");
        if ts {
            push(
                &mut r.languages,
                "typescript",
                if exists("tsconfig.json") {
                    "tsconfig.json"
                } else {
                    "package.json: typescript"
                },
            );
        }
        for (dep, fw) in [
            ("next", "next"),
            ("nuxt", "nuxt"),
            ("@sveltejs/kit", "sveltekit"),
            ("svelte", "svelte"),
            ("react", "react"),
            ("@types/react", "react"),
            ("vue", "vue"),
            ("@vue/cli-service", "vue"),
            ("@angular/core", "angular"),
            ("@nestjs/core", "nestjs"),
            ("express", "express"),
            ("fastify", "fastify"),
            ("koa", "koa"),
            ("@tauri-apps/api", "tauri"),
            ("electron", "electron"),
            ("vite", "vite"),
        ] {
            if deps.contains(dep) {
                push(&mut r.frameworks, fw, format!("package.json: {dep}"));
            }
        }
        let pm = if exists("pnpm-lock.yaml") {
            "pnpm"
        } else if exists("yarn.lock") {
            "yarn"
        } else if exists("bun.lockb") || exists("bun.lock") {
            "bun"
        } else {
            "npm"
        };
        r.package_managers.push(pm.into());
        let run = |s: &str| match pm {
            "npm" if s == "test" => "npm test".to_string(),
            "npm" => format!("npm run {s}"),
            other => format!("{other} {s}"),
        };
        let scripts = pkg["scripts"].as_object().cloned().unwrap_or_default();
        let has = |k: &str| scripts.contains_key(k);
        if has("build") {
            r.build_command = Some(run("build"));
        }
        if has("test")
            && !scripts["test"]
                .as_str()
                .unwrap_or("")
                .contains("no test specified")
        {
            r.test_command = Some(run("test"));
        }
        if has("lint") {
            r.lint_command = Some(run("lint"));
        } else if has("check") {
            r.lint_command = Some(run("check"));
        }
        if has("dev") {
            r.dev_command = Some(run("dev"));
        } else if has("start") {
            r.dev_command = Some(run("start"));
        }
    } else if ext("ts") || ext("tsx") {
        push(&mut r.languages, "typescript", "*.ts");
    } else if ext("js") || ext("mjs") {
        push(&mut r.languages, "javascript", "*.js");
    }

    // Python
    let (py, py_sources) = python_deps(root);
    if !py_sources.is_empty() || ext("py") {
        push(
            &mut r.languages,
            "python",
            py_sources.first().copied().unwrap_or("*.py"),
        );
        for (dep, fw) in [
            ("django", "django"),
            ("flask", "flask"),
            ("fastapi", "fastapi"),
        ] {
            if py.contains(dep) {
                push(&mut r.frameworks, fw, format!("python deps: {dep}"));
            }
        }
        // The original awaited `.length` of a promise here; these really look.
        if exists("manage.py") || files.iter().any(|p| p.ends_with("settings.py")) {
            push(&mut r.frameworks, "django", "manage.py / settings.py");
        }
        if exists("app.py")
            && read(&root.join("app.py"))
                .map(|t| t.contains("Flask("))
                .unwrap_or(false)
        {
            push(&mut r.frameworks, "flask", "app.py: Flask(");
        }
        let pm = if exists("uv.lock") {
            "uv"
        } else if exists("poetry.lock") {
            "poetry"
        } else {
            "pip"
        };
        r.package_managers.push(pm.into());
        if r.test_command.is_none()
            && (py.contains("pytest") || exists("pytest.ini") || exists("tests"))
        {
            r.test_command = Some(match pm {
                "uv" => "uv run pytest".into(),
                "poetry" => "poetry run pytest".into(),
                _ => "pytest".into(),
            });
        }
        if r.test_command.is_none() && r.has_framework("django") {
            r.test_command = Some("python manage.py test".into());
        }
        if r.lint_command.is_none() && (py.contains("ruff") || exists("ruff.toml")) {
            r.lint_command = Some("ruff check .".into());
        }
    }

    // Ruby
    let gemfile = read(&root.join("Gemfile"));
    if gemfile.is_some() || ext("rb") {
        push(
            &mut r.languages,
            "ruby",
            if gemfile.is_some() { "Gemfile" } else { "*.rb" },
        );
        let gems = gemfile.as_deref().map(gem_names).unwrap_or_default();
        if gems.contains("rails")
            || exists("config/application.rb")
            || exists("config/routes.rb")
            || read(&root.join("Rakefile"))
                .map(|t| t.contains("Rails.application.load_tasks"))
                .unwrap_or(false)
        {
            push(
                &mut r.frameworks,
                "rails",
                "Gemfile / config/application.rb",
            );
        }
        if gems.contains("sinatra") {
            push(&mut r.frameworks, "sinatra", "Gemfile: sinatra");
        }
        r.package_managers.push("bundler".into());
        if r.test_command.is_none() {
            if gems.contains("rspec") || gems.contains("rspec-rails") || exists("spec") {
                r.test_command = Some("bundle exec rspec".into());
            } else if r.has_framework("rails") {
                r.test_command = Some("bin/rails test".into());
            }
        }
        if r.lint_command.is_none() && gems.contains("rubocop") {
            r.lint_command = Some("bundle exec rubocop".into());
        }
    }

    // Rust
    let cargo = read(&root.join("Cargo.toml"));
    let cargo_nested = files
        .iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml"))
        .cloned();
    if cargo.is_some() || cargo_nested.is_some() || ext("rs") {
        let text = cargo
            .clone()
            .or_else(|| cargo_nested.as_deref().and_then(read))
            .unwrap_or_default();
        push(
            &mut r.languages,
            "rust",
            if cargo.is_some() {
                "Cargo.toml".to_string()
            } else {
                cargo_nested
                    .as_ref()
                    .map(|p| {
                        p.strip_prefix(root)
                            .unwrap_or(p)
                            .to_string_lossy()
                            .to_string()
                    })
                    .unwrap_or_else(|| "*.rs".into())
            },
        );
        let deps = cargo_deps(&text);
        for (dep, fw) in [
            ("tauri", "tauri"),
            ("axum", "axum"),
            ("actix-web", "actix"),
            ("rocket", "rocket"),
            ("tokio", "tokio"),
        ] {
            if deps.contains(dep) {
                push(&mut r.frameworks, fw, format!("Cargo.toml: {dep}"));
            }
        }
        r.package_managers.push("cargo".into());
        let prefix = if cargo.is_none() {
            cargo_nested
                .as_ref()
                .and_then(|p| p.parent())
                .and_then(|d| d.strip_prefix(root).ok())
                .map(|d| format!("cd {} && ", d.to_string_lossy()))
                .unwrap_or_default()
        } else {
            String::new()
        };
        r.build_command
            .get_or_insert(format!("{prefix}cargo build"));
        r.test_command.get_or_insert(format!("{prefix}cargo test"));
        r.lint_command
            .get_or_insert(format!("{prefix}cargo clippy"));
    }

    // Go
    let gomod = read(&root.join("go.mod"));
    if gomod.is_some() || ext("go") {
        push(
            &mut r.languages,
            "go",
            if gomod.is_some() { "go.mod" } else { "*.go" },
        );
        let req = gomod.as_deref().map(go_requires).unwrap_or_default();
        for (dep, fw) in [
            ("github.com/gin-gonic/gin", "gin"),
            ("github.com/labstack/echo", "echo"),
            ("github.com/gofiber/fiber", "fiber"),
        ] {
            if req.iter().any(|l| l.starts_with(dep)) {
                push(&mut r.frameworks, fw, format!("go.mod: {dep}"));
            }
        }
        r.package_managers.push("go".into());
        r.build_command.get_or_insert("go build ./...".into());
        r.test_command.get_or_insert("go test ./...".into());
        r.lint_command.get_or_insert("go vet ./...".into());
    }
    r
}

/// The "PROJECT CONTEXT" block cct's global agents appended to the prompt.
pub fn context_block(r: &StackReport) -> String {
    let mut s = String::from("PROJECT CONTEXT\n");
    if let Some(n) = &r.name {
        s.push_str(&format!("- Project: {n}\n"));
    }
    if !r.languages.is_empty() {
        s.push_str(&format!(
            "- Languages: {}\n",
            r.languages
                .iter()
                .map(|l| l.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !r.frameworks.is_empty() {
        s.push_str(&format!(
            "- Frameworks: {}\n",
            r.frameworks
                .iter()
                .map(|l| l.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (label, c) in [
        ("Build", &r.build_command),
        ("Test", &r.test_command),
        ("Lint", &r.lint_command),
    ] {
        if let Some(c) = c {
            s.push_str(&format!("- {label}: `{c}`\n"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_every_language_and_real_framework_files() {
        let root = std::env::temp_dir().join(format!("stack-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(root.join("backend/app")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"web","scripts":{"test":"vitest","build":"vite build"},"devDependencies":{"typescript":"5","vite":"5"},"dependencies":{"react":"19"}}"#,
        )
        .unwrap();
        std::fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(root.join("requirements.txt"), "Django>=5\npytest\n").unwrap();
        std::fs::write(root.join("backend/app/settings.py"), "").unwrap();
        std::fs::write(root.join("node_modules/x/a.go"), "").unwrap();
        let r = detect(&root);
        let langs: Vec<&str> = r.languages.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(langs, vec!["javascript", "typescript", "python"]);
        assert!(r.has_framework("react") && r.has_framework("django") && r.has_framework("vite"));
        assert_eq!(r.test_command.as_deref(), Some("pnpm test"));
        assert_eq!(r.build_command.as_deref(), Some("pnpm build"));
        assert!(!r.has_language("go"), "node_modules is skipped");
        let _ = std::fs::remove_dir_all(&root);
    }
}
