use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use ignore::overrides::{Override, OverrideBuilder};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{estimate_tokens, output_file, write_new, DevError};

pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_FILES: usize = 20_000;
pub const DEFAULT_LIMIT: usize = 100;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PackRequest {
    pub path: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub max_tokens: Option<usize>,
    pub output: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Full,
    Outline,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Gitignored,
    Binary,
    Lockfile,
    TooLarge,
    Secret,
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanFile {
    pub path: String,
    pub tokens: usize,
    pub mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Excluded {
    pub path: String,
    pub reason: Reason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SecretFinding {
    pub path: String,
    pub line: Option<usize>,
    pub rule: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reduced {
    pub path: String,
    pub to: Mode,
    pub tokens_before: usize,
    pub tokens_after: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub files: usize,
    pub tokens: usize,
    pub excluded: usize,
    pub secrets: usize,
    pub not_included: usize,
    pub over_budget: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub root: String,
    pub files: Vec<PlanFile>,
    pub total: usize,
    pub truncated: bool,
    pub excluded: Vec<Excluded>,
    pub secrets: Vec<SecretFinding>,
    pub reduced: Vec<Reduced>,
    pub degrade_order: [Mode; 3],
    pub totals: Totals,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Built {
    pub path: String,
    pub tokens: usize,
    pub bytes: usize,
    pub sha256: String,
    pub manifest: Plan,
}

struct Rule {
    name: &'static str,
    regex: regex::Regex,
    needs_digit: bool,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    [
        ("aws_access_key", r"\b(AKIA|ASIA)[0-9A-Z]{16}\b", false),
        ("private_key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----", false),
        (
            "api_token",
            r"\b(sk-[A-Za-z0-9_-]{20,}|xai-[A-Za-z0-9]{20,}|gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})",
            true,
        ),
    ]
    .into_iter()
    .filter_map(|(name, pattern, needs_digit)| {
        regex::Regex::new(pattern).ok().map(|regex| Rule {
            name,
            regex,
            needs_digit,
        })
    })
    .collect()
});

static SIGNATURE: LazyLock<Option<regex::Regex>> = LazyLock::new(|| {
    regex::Regex::new(
        r"^(\s*(#{1,6}\s|(pub(\([a-z]+\))?\s+)?((async|unsafe)\s+)*(fn|struct|enum|trait|impl|mod)\b|export\s|(async\s+)?(function|def)\b|(class|interface|func)\s)|(pub\s+)?(const|static|type)\s)",
    )
    .ok()
});

pub fn scan_secrets(text: &str) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for rule in RULES.iter() {
            let hit = rule
                .regex
                .find_iter(line)
                .any(|m| !rule.needs_digit || m.as_str().bytes().any(|b| b.is_ascii_digit()));
            if hit {
                found.push((index + 1, rule.name));
            }
        }
    }
    found
}

pub fn outline(text: &str) -> String {
    let Some(signature) = SIGNATURE.as_ref() else {
        return String::new();
    };
    let mut out = String::new();
    for line in text.lines().filter(|line| signature.is_match(line)) {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

pub fn lock_summary(name: &str, text: &str) -> Option<String> {
    let base = name.rsplit('/').next()?;
    let count = match base {
        "Cargo.lock" => {
            let packages = text.lines().filter(|l| l.trim() == "[[package]]").count();
            format!("{packages} packages")
        }
        "package-lock.json" => format!("{} packages", text.matches("\"node_modules/").count()),
        "pnpm-lock.yaml" | "yarn.lock" | "bun.lock" | "deno.lock" | "Gemfile.lock"
        | "poetry.lock" | "uv.lock" | "Pipfile.lock" | "composer.lock" | "flake.lock"
        | "go.sum" | "packages.lock.json" => format!("{} lines", text.lines().count()),
        _ => return None,
    };
    Some(format!("{name}: {count} (lockfile omitted)"))
}

fn is_env_file(name: &str) -> bool {
    name == ".env"
        || name.strip_prefix(".env.").is_some_and(|rest| {
            !matches!(
                rest,
                "example" | "sample" | "template" | "dist" | "defaults"
            )
        })
}

struct Entry {
    path: String,
    text: String,
    tokens: usize,
    full_tokens: usize,
    mode: Mode,
}

struct Collected {
    root: PathBuf,
    entries: Vec<Entry>,
    excluded: Vec<Excluded>,
    secrets: Vec<SecretFinding>,
    not_included: usize,
    over_budget: bool,
}

fn globs(root: &Path, patterns: &[String]) -> Result<Option<Override>, DevError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let bad = |e: ignore::Error| DevError::InvalidInput(format!("invalid glob: {e}"));
    let mut builder = OverrideBuilder::new(root);
    for pattern in patterns {
        builder.add(pattern).map_err(bad)?;
    }
    builder.build().map(Some).map_err(bad)
}

fn hits(matcher: &Override, relative: &Path) -> bool {
    matcher.matched(relative, false).is_whitelist()
        || relative
            .ancestors()
            .skip(1)
            .any(|dir| !dir.as_os_str().is_empty() && matcher.matched(dir, true).is_whitelist())
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn degrade(entries: &mut [Entry], budget: usize) -> bool {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|a, b| {
        entries[*b]
            .full_tokens
            .cmp(&entries[*a].full_tokens)
            .then_with(|| entries[*a].path.cmp(&entries[*b].path))
    });
    let total = |entries: &[Entry]| entries.iter().map(|e| e.tokens).sum::<usize>();
    for index in &order {
        if total(entries) <= budget {
            return false;
        }
        let entry = &mut entries[*index];
        entry.text = outline(&entry.text);
        entry.tokens = estimate_tokens(&entry.text);
        entry.mode = Mode::Outline;
    }
    for index in &order {
        if total(entries) <= budget {
            return false;
        }
        let entry = &mut entries[*index];
        entry.text = String::new();
        entry.tokens = estimate_tokens(&entry.path);
        entry.mode = Mode::Path;
    }
    total(entries) > budget
}

pub fn folder_counts(root: &Path, files: &[PathBuf]) -> Vec<(String, usize)> {
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for file in files {
        let relative = file.strip_prefix(root).unwrap_or(file);
        let mut parts = relative.components();
        let first = parts
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        let folder = match (first, parts.next()) {
            (Some(first), Some(_)) => first,
            _ => ".".to_string(),
        };
        *counts.entry(folder).or_default() += 1;
    }
    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    sorted.truncate(10);
    sorted
}

fn collect(request: &PackRequest) -> Result<Collected, DevError> {
    let root = PathBuf::from(&request.path);
    let unreadable = |path: &Path, message: String| DevError::Read {
        path: path.to_path_buf(),
        message,
    };
    let meta = std::fs::metadata(&root).map_err(|e| unreadable(&root, e.to_string()))?;
    if !meta.is_dir() {
        return Err(unreadable(&root, "not a folder".into()));
    }
    let include = globs(&root, &request.include)?;
    let exclude = globs(&root, &request.exclude)?;

    let walker = WalkBuilder::new(&root)
        .hidden(false)
        .ignore(false)
        .git_global(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in walker {
        let entry = entry.map_err(|e| unreadable(&root, e.to_string()))?;
        let kind = entry.file_type();
        let path = entry.into_path();
        if kind.is_some_and(|k| k.is_dir()) {
            dirs.push(path.clone());
        } else if kind.is_some_and(|k| k.is_file()) {
            files.push(path.clone());
            if files.len() > MAX_FILES {
                return Err(DevError::TooManyFiles {
                    limit: MAX_FILES,
                    folders: folder_counts(&root, &files),
                });
            }
        }
        seen.insert(path);
    }

    let mut excluded = Vec::new();
    for dir in &dirs {
        let children = std::fs::read_dir(dir).map_err(|e| unreadable(dir, e.to_string()))?;
        for child in children.flatten() {
            let path = child.path();
            if seen.contains(&path) || child.file_name() == ".git" {
                continue;
            }
            let mut shown = relative(&root, &path);
            if path.is_dir() {
                shown.push('/');
            }
            excluded.push(Excluded {
                path: shown,
                reason: Reason::Gitignored,
                summary: None,
            });
        }
    }

    let mut named: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|path| (relative(&root, &path), path))
        .collect();
    named.sort();
    let mut entries = Vec::new();
    let mut secrets = Vec::new();
    let mut not_included = 0;
    for (shown, path) in named {
        let mut exclude_as = |reason: Reason, summary: Option<String>| {
            excluded.push(Excluded {
                path: shown.clone(),
                reason,
                summary,
            })
        };
        if include
            .as_ref()
            .is_some_and(|m| !hits(m, Path::new(&shown)))
        {
            not_included += 1;
            continue;
        }
        if exclude.as_ref().is_some_and(|m| hits(m, Path::new(&shown))) {
            exclude_as(Reason::Excluded, None);
            continue;
        }
        let name = shown.rsplit('/').next().unwrap_or(&shown);
        if is_env_file(name) {
            exclude_as(Reason::Secret, None);
            secrets.push(SecretFinding {
                path: shown,
                line: None,
                rule: "env_file",
            });
            continue;
        }
        let size = std::fs::metadata(&path)
            .map_err(|e| unreadable(&path, e.to_string()))?
            .len();
        let lockfile = lock_summary(&shown, "").is_some();
        if size > MAX_FILE_BYTES && !lockfile {
            exclude_as(Reason::TooLarge, None);
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| unreadable(&path, e.to_string()))?;
        if lockfile {
            let summary = lock_summary(&shown, &String::from_utf8_lossy(&bytes));
            exclude_as(Reason::Lockfile, summary);
            continue;
        }
        let text = match String::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => text,
            _ => {
                exclude_as(Reason::Binary, None);
                continue;
            }
        };
        let findings = scan_secrets(&text);
        if !findings.is_empty() {
            exclude_as(Reason::Secret, None);
            secrets.extend(findings.into_iter().map(|(line, rule)| SecretFinding {
                path: shown.clone(),
                line: Some(line),
                rule,
            }));
            continue;
        }
        let tokens = estimate_tokens(&text);
        entries.push(Entry {
            path: shown,
            text,
            tokens,
            full_tokens: tokens,
            mode: Mode::Full,
        });
    }
    if entries.is_empty() {
        return Err(DevError::NoFiles);
    }
    excluded.sort_by(|a, b| a.path.cmp(&b.path));
    let over_budget = request
        .max_tokens
        .is_some_and(|budget| degrade(&mut entries, budget));
    Ok(Collected {
        root,
        entries,
        excluded,
        secrets,
        not_included,
        over_budget,
    })
}

fn manifest(collected: &Collected, limit: usize) -> Plan {
    let mut files: Vec<PlanFile> = collected
        .entries
        .iter()
        .map(|e| PlanFile {
            path: e.path.clone(),
            tokens: e.tokens,
            mode: e.mode,
        })
        .collect();
    files.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.path.cmp(&b.path)));
    let mut reduced: Vec<Reduced> = collected
        .entries
        .iter()
        .filter(|e| e.mode != Mode::Full)
        .map(|e| Reduced {
            path: e.path.clone(),
            to: e.mode,
            tokens_before: e.full_tokens,
            tokens_after: e.tokens,
        })
        .collect();
    reduced.sort_by(|a, b| {
        b.tokens_before
            .cmp(&a.tokens_before)
            .then_with(|| a.path.cmp(&b.path))
    });
    let total = files.len();
    let totals = Totals {
        files: total,
        tokens: files.iter().map(|f| f.tokens).sum(),
        excluded: collected.excluded.len(),
        secrets: collected.secrets.len(),
        not_included: collected.not_included,
        over_budget: collected.over_budget,
    };
    files.truncate(limit);
    reduced.truncate(limit);
    Plan {
        root: collected.root.to_string_lossy().into_owned(),
        truncated: total > limit
            || collected.excluded.len() > limit
            || collected.secrets.len() > limit,
        files,
        total,
        excluded: collected.excluded.iter().take(limit).cloned().collect(),
        secrets: collected.secrets.iter().take(limit).cloned().collect(),
        reduced,
        degrade_order: [Mode::Full, Mode::Outline, Mode::Path],
        totals,
    }
}

pub fn plan(request: &PackRequest) -> Result<Plan, DevError> {
    let collected = collect(request)?;
    Ok(manifest(&collected, request.limit.unwrap_or(DEFAULT_LIMIT)))
}

fn render(collected: &Collected) -> String {
    let label = |mode: Mode| match mode {
        Mode::Full => "",
        Mode::Outline => " (outline)",
        Mode::Path => " (path only)",
    };
    let mut out = format!("# Pack of {} files\n\n## Files\n", collected.entries.len());
    for entry in &collected.entries {
        out.push_str(&format!("{}{}\n", entry.path, label(entry.mode)));
    }
    let summaries: Vec<&str> = collected
        .excluded
        .iter()
        .filter_map(|e| e.summary.as_deref())
        .collect();
    if !summaries.is_empty() {
        out.push_str("\n## Dependencies\n");
        for summary in summaries {
            out.push_str(summary);
            out.push('\n');
        }
    }
    for entry in &collected.entries {
        out.push_str(&format!(
            "\n===== {}{} =====\n",
            entry.path,
            label(entry.mode)
        ));
        out.push_str(&entry.text);
        if !entry.text.is_empty() && !entry.text.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

pub fn build(request: &PackRequest) -> Result<Built, DevError> {
    let collected = collect(request)?;
    let text = render(&collected);
    let path = match &request.output {
        Some(path) => PathBuf::from(path),
        None => output_file("pack", "txt"),
    };
    write_new(&path, text.as_bytes())?;
    Ok(Built {
        path: path.to_string_lossy().into_owned(),
        tokens: estimate_tokens(&text),
        bytes: text.len(),
        sha256: hex::encode(Sha256::digest(text.as_bytes())),
        manifest: manifest(&collected, request.limit.unwrap_or(DEFAULT_LIMIT)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_counted_by_top_folder_largest_first() {
        let root = Path::new("/r");
        let files: Vec<PathBuf> = ["/r/a/1", "/r/a/b/2", "/r/c/3", "/r/top.txt", "/r/a/4"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(
            folder_counts(root, &files),
            vec![
                ("a".to_string(), 3),
                (".".to_string(), 1),
                ("c".to_string(), 1)
            ]
        );
    }

    fn aws_key() -> String {
        format!("AKIA{}", "IOSFODNN7EXAMPLE")
    }

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        dir
    }

    fn request(dir: &Path) -> PackRequest {
        PackRequest {
            path: dir.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    fn paths(plan: &Plan) -> Vec<&str> {
        let mut out: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
        out.sort();
        out
    }

    fn reason_of(plan: &Plan, path: &str) -> Option<Reason> {
        plan.excluded
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.reason)
    }

    #[test]
    fn gitignored_paths_are_left_out_and_reported_once_at_their_top() {
        let dir = tree(&[
            (".gitignore", "ignored/\n*.log\n"),
            ("src/a.rs", "fn a() {}\n"),
            ("ignored/deep/x.txt", "x"),
            ("ignored/y.txt", "y"),
            ("debug.log", "log"),
            (".git/HEAD", "ref: refs/heads/main"),
        ]);
        let plan = plan(&request(dir.path())).unwrap();
        assert_eq!(paths(&plan), vec![".gitignore", "src/a.rs"]);
        assert_eq!(reason_of(&plan, "ignored/"), Some(Reason::Gitignored));
        assert_eq!(reason_of(&plan, "debug.log"), Some(Reason::Gitignored));
        assert_eq!(plan.excluded.len(), 2);
    }

    #[test]
    fn binaries_and_oversized_files_are_left_out_with_the_reason() {
        let dir = tree(&[("a.txt", "text")]);
        std::fs::write(
            dir.path().join("logo.png"),
            [0x89, b'P', b'N', b'G', 0, 1, 2],
        )
        .unwrap();
        std::fs::write(dir.path().join("latin1.txt"), [b'c', b'a', b'f', 0xe9]).unwrap();
        std::fs::write(
            dir.path().join("huge.txt"),
            vec![b'a'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let plan = plan(&request(dir.path())).unwrap();
        assert_eq!(paths(&plan), vec!["a.txt"]);
        assert_eq!(reason_of(&plan, "logo.png"), Some(Reason::Binary));
        assert_eq!(reason_of(&plan, "latin1.txt"), Some(Reason::Binary));
        assert_eq!(reason_of(&plan, "huge.txt"), Some(Reason::TooLarge));
    }

    #[test]
    fn a_lockfile_becomes_a_one_line_dependency_summary() {
        let lock = "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"b\"\nversion = \"2.0.0\"\n";
        let dir = tree(&[
            ("Cargo.lock", lock),
            ("pnpm-lock.yaml", "a:\n  b: 1\n"),
            ("main.rs", "fn main() {}\n"),
        ]);
        let plan = plan(&request(dir.path())).unwrap();
        assert_eq!(paths(&plan), vec!["main.rs"]);
        let entry = plan
            .excluded
            .iter()
            .find(|e| e.path == "Cargo.lock")
            .unwrap();
        assert_eq!(entry.reason, Reason::Lockfile);
        assert_eq!(
            entry.summary.as_deref(),
            Some("Cargo.lock: 2 packages (lockfile omitted)")
        );
        let entry = plan
            .excluded
            .iter()
            .find(|e| e.path == "pnpm-lock.yaml")
            .unwrap();
        assert_eq!(
            entry.summary.as_deref(),
            Some("pnpm-lock.yaml: 2 lines (lockfile omitted)")
        );
    }

    #[test]
    fn lock_summaries_count_packages_where_the_format_is_known() {
        assert_eq!(lock_summary("src/lib.rs", ""), None);
        assert_eq!(
            lock_summary(
                "package-lock.json",
                "{\"packages\":{\"\":{},\"node_modules/a\":{},\"node_modules/b\":{}}}"
            )
            .as_deref(),
            Some("package-lock.json: 2 packages (lockfile omitted)")
        );
        assert!(lock_summary("yarn.lock", "a\nb\n").is_some());
        assert!(lock_summary("go.sum", "a\n").is_some());
    }

    #[test]
    fn secret_rules_report_the_line_and_the_rule() {
        let text = format!(
            "let a = 1;\nconst KEY = \"{}\";\n-----BEGIN RSA PRIVATE KEY-----\nopenai = \"sk-{}\"\nxai = \"xai-{}\"\ngh = \"ghp_{}\"\n",
            aws_key(),
            "proj-Ab3dEf6hIj9lMn2pQr5tUv8x",
            "Ab3dEf6hIj9lMn2pQr5tUv8xYz1",
            "Ab3dEf6hIj9lMn2pQr5tUv8xYz1Ab3dEf6hIj"
        );
        assert_eq!(
            scan_secrets(&text),
            vec![
                (2, "aws_access_key"),
                (3, "private_key"),
                (4, "api_token"),
                (5, "api_token"),
                (6, "api_token"),
            ]
        );
    }

    #[test]
    fn ordinary_code_is_not_a_secret() {
        let text = "let task-runner = risk-management-policy-for-everyone;\nimport sk-learn-preprocessing-utilities-module\nlet akia = \"AKIA\";\n";
        assert_eq!(scan_secrets(text), vec![]);
    }

    #[test]
    fn a_file_with_a_secret_is_blocked_and_the_value_never_leaves() {
        let key = aws_key();
        let dir = tree(&[
            (
                "config.ts",
                &format!("export const a = 1;\n\nconst k = \"{key}\";\n"),
            ),
            (".env", "TOKEN=abc\n"),
            (".env.local", "TOKEN=abc\n"),
            (".env.example", "TOKEN=\n"),
            ("ok.ts", "export const b = 2;\n"),
        ]);
        let plan = plan(&request(dir.path())).unwrap();
        assert_eq!(paths(&plan), vec![".env.example", "ok.ts"]);
        assert_eq!(reason_of(&plan, "config.ts"), Some(Reason::Secret));
        assert_eq!(reason_of(&plan, ".env"), Some(Reason::Secret));
        assert_eq!(
            plan.secrets,
            vec![
                SecretFinding {
                    path: ".env".into(),
                    line: None,
                    rule: "env_file"
                },
                SecretFinding {
                    path: ".env.local".into(),
                    line: None,
                    rule: "env_file"
                },
                SecretFinding {
                    path: "config.ts".into(),
                    line: Some(3),
                    rule: "aws_access_key"
                },
            ]
        );
        assert_eq!(plan.totals.secrets, 3);
        assert!(!serde_json::to_string(&plan).unwrap().contains(&key));
    }

    #[test]
    fn tokens_are_estimated_per_file_and_summed() {
        let dir = tree(&[("a.txt", "12345678"), ("b.txt", "123456789")]);
        let plan = plan(&request(dir.path())).unwrap();
        assert_eq!(
            plan.files,
            vec![
                PlanFile {
                    path: "b.txt".into(),
                    tokens: 3,
                    mode: Mode::Full
                },
                PlanFile {
                    path: "a.txt".into(),
                    tokens: 2,
                    mode: Mode::Full
                },
            ]
        );
        assert_eq!(plan.totals.tokens, 5);
        assert_eq!(plan.totals.files, 2);
        assert!(!plan.totals.over_budget);
    }

    #[test]
    fn include_and_exclude_globs_select_files() {
        let dir = tree(&[
            ("src/a.rs", "a"),
            ("src/deep/b.rs", "b"),
            ("docs/c.md", "c"),
            ("README.md", "r"),
        ]);
        let mut req = request(dir.path());
        req.include = vec!["src/**".into()];
        let plan1 = plan(&req).unwrap();
        assert_eq!(paths(&plan1), vec!["src/a.rs", "src/deep/b.rs"]);
        assert_eq!(plan1.totals.not_included, 2);
        req.include = vec!["src".into()];
        assert_eq!(
            paths(&plan(&req).unwrap()),
            vec!["src/a.rs", "src/deep/b.rs"]
        );
        req.include = vec![];
        req.exclude = vec!["*.md".into(), "src/deep".into()];
        let plan2 = plan(&req).unwrap();
        assert_eq!(paths(&plan2), vec!["src/a.rs"]);
        assert_eq!(reason_of(&plan2, "README.md"), Some(Reason::Excluded));
        assert_eq!(reason_of(&plan2, "src/deep/b.rs"), Some(Reason::Excluded));
    }

    #[test]
    fn a_bad_glob_is_invalid_input() {
        let dir = tree(&[("a.txt", "a")]);
        let mut req = request(dir.path());
        req.include = vec!["src/{a".into()];
        assert!(matches!(plan(&req), Err(DevError::InvalidInput(_))));
    }

    #[test]
    fn the_file_list_is_capped_and_says_so() {
        let dir = tree(&[("a.txt", "a"), ("b.txt", "bb bb bb"), ("c.txt", "c")]);
        let mut req = request(dir.path());
        req.limit = Some(1);
        let plan = plan(&req).unwrap();
        assert_eq!(paths(&plan), vec!["b.txt"]);
        assert_eq!(plan.total, 3);
        assert!(plan.truncated);
    }

    #[test]
    fn outline_keeps_signatures_and_headings() {
        let text = "# Title\nprose\n## Part\npub fn run(a: u8) -> u8 {\n    a + 1\n}\nexport function go() {\n  return 1;\n}\nclass Thing:\n    def method(self):\n        pass\nstruct P;\n";
        assert_eq!(
            outline(text),
            "# Title\n## Part\npub fn run(a: u8) -> u8 {\nexport function go() {\nclass Thing:\n    def method(self):\nstruct P;\n"
        );
    }

    fn body(lines: usize) -> String {
        let mut text = String::from("pub fn entry() {\n");
        for n in 0..lines {
            text.push_str(&format!("    let value_{n} = compute({n});\n"));
        }
        text.push_str("}\n");
        text
    }

    #[test]
    fn over_budget_the_largest_files_drop_to_an_outline_first() {
        let dir = tree(&[
            ("big.rs", &body(200)),
            ("mid.rs", &body(40)),
            ("small.rs", &body(4)),
        ]);
        let full = plan(&request(dir.path())).unwrap();
        let big = full.files[0].tokens;
        let mut req = request(dir.path());
        req.max_tokens = Some(full.totals.tokens - big + 50);
        let plan = plan(&req).unwrap();
        assert_eq!(plan.files[0].path, "mid.rs");
        let reduced = &plan.reduced;
        assert_eq!(reduced.len(), 1);
        assert_eq!(reduced[0].path, "big.rs");
        assert_eq!(reduced[0].to, Mode::Outline);
        assert_eq!(reduced[0].tokens_before, big);
        assert_eq!(reduced[0].tokens_after, 5);
        assert!(plan.totals.tokens <= req.max_tokens.unwrap());
        assert!(!plan.totals.over_budget);
        assert_eq!(plan.degrade_order, [Mode::Full, Mode::Outline, Mode::Path]);
    }

    #[test]
    fn when_outlines_are_not_enough_files_drop_to_path_only() {
        let dir = tree(&[
            ("big.rs", &body(200)),
            ("mid.rs", &body(40)),
            ("small.rs", &body(4)),
        ]);
        let mut req = request(dir.path());
        req.max_tokens = Some(9);
        let plan = plan(&req).unwrap();
        let mode = |path: &str| plan.files.iter().find(|f| f.path == path).unwrap().mode;
        assert_eq!(mode("big.rs"), Mode::Path);
        assert_eq!(mode("mid.rs"), Mode::Path);
        assert_eq!(mode("small.rs"), Mode::Outline);
        assert!(plan.totals.tokens <= 9);
        assert_eq!(plan.reduced.len(), 3);
        req.max_tokens = Some(1);
        assert!(super::plan(&req).unwrap().totals.over_budget);
    }

    #[test]
    fn build_writes_the_pack_to_a_file_and_returns_only_the_path_and_manifest() {
        let dir = tree(&[
            ("src/a.rs", "pub fn a() {}\n"),
            ("Cargo.lock", "[[package]]\nname = \"a\"\n"),
            ("notes.md", "# Notes\n"),
        ]);
        let out = tempfile::tempdir().unwrap();
        let mut req = request(dir.path());
        req.output = Some(out.path().join("pack.txt").to_string_lossy().into_owned());
        let built = build(&req).unwrap();
        let text = std::fs::read_to_string(&built.path).unwrap();
        assert!(text.contains("===== src/a.rs =====\npub fn a() {}\n"));
        assert!(text.contains("===== notes.md =====\n# Notes\n"));
        assert!(text.contains("Cargo.lock: 1 packages (lockfile omitted)"));
        assert!(text.find("notes.md =====").unwrap() < text.find("src/a.rs =====").unwrap());
        assert_eq!(built.tokens, super::super::estimate_tokens(&text));
        assert_eq!(built.bytes, text.len());
        assert_eq!(built.manifest.totals.files, 2);
        assert!(!serde_json::to_string(&built)
            .unwrap()
            .contains("pub fn a()"));

        req.output = Some(out.path().join("again.txt").to_string_lossy().into_owned());
        assert_eq!(build(&req).unwrap().sha256, built.sha256);
        assert!(matches!(build(&req), Err(DevError::OutputExists(_))));
    }

    #[test]
    fn build_marks_reduced_files_in_the_pack() {
        let dir = tree(&[("big.rs", &body(200)), ("small.rs", &body(2))]);
        let out = tempfile::tempdir().unwrap();
        let mut req = request(dir.path());
        req.max_tokens = Some(40);
        req.output = Some(out.path().join("p.txt").to_string_lossy().into_owned());
        let built = build(&req).unwrap();
        let text = std::fs::read_to_string(&built.path).unwrap();
        assert!(text.contains("===== big.rs (outline) =====\npub fn entry() {\n"));
        assert!(!text.contains("value_150"));
    }

    #[test]
    fn build_without_output_writes_a_new_file_in_the_temp_folder() {
        let dir = tree(&[("a.txt", "a")]);
        let built = build(&request(dir.path())).unwrap();
        assert!(Path::new(&built.path).starts_with(std::env::temp_dir()));
        std::fs::remove_file(&built.path).unwrap();
    }

    #[test]
    fn nothing_to_pack_and_a_missing_folder_are_errors() {
        let dir = tree(&[("a.bin", "\0\0")]);
        assert_eq!(plan(&request(dir.path())), Err(DevError::NoFiles));
        assert!(matches!(
            plan(&request(Path::new("/no/such/folder"))),
            Err(DevError::Read { .. })
        ));
    }

    #[test]
    #[ignore = "smoke: packs this repository's src/lib/superpowers, run by hand with --ignored"]
    fn the_superpowers_folder_of_this_repo_is_planned() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/lib/superpowers");
        let plan = plan(&request(&folder)).unwrap();
        println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        assert!(plan.files.iter().any(|f| f.path == "catalog.ts"));
        assert_eq!(
            plan.totals.tokens,
            plan.files.iter().map(|f| f.tokens).sum::<usize>()
        );
    }
}
