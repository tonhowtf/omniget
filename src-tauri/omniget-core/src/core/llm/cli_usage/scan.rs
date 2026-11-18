use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::parse::{codex_tool_name, CliKind, CliUsageEntry, FileParser, Parsed, RateLimitSample};

const CHUNK: usize = 4 * 1024 * 1024;

pub const ERR_CLI_USAGE_STATE: &str = "ERR_CLI_USAGE_STATE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRoot {
    pub cli: CliKind,
    pub account: String,
    pub path: PathBuf,
}

impl ScanRoot {
    pub fn new(cli: CliKind, account: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            cli,
            account: account.into(),
            path: path.into(),
        }
    }

    pub fn guess(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let looks_codex = path
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some(".codex") | Some("codex")));
        Self::new(
            if looks_codex {
                CliKind::Codex
            } else {
                CliKind::Claude
            },
            "default",
            path,
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanState {
    offsets: HashMap<PathBuf, u64>,
}

impl ScanState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn offset(&self, path: &Path) -> u64 {
        self.offsets.get(path).copied().unwrap_or(0)
    }

    pub fn set_offset(&mut self, path: impl Into<PathBuf>, offset: u64) {
        self.offsets.insert(path.into(), offset);
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn prune_missing(&mut self) {
        self.offsets.retain(|p, _| p.exists());
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{ERR_CLI_USAGE_STATE}: {e}"))?;
        }
        let text =
            serde_json::to_string(self).map_err(|e| format!("{ERR_CLI_USAGE_STATE}: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("{ERR_CLI_USAGE_STATE}: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{ERR_CLI_USAGE_STATE}: {e}"))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ScanStats {
    pub files_seen: u32,
    pub files_read: u32,
    pub bytes_read: u64,
    pub lines: u64,
    pub partial_lines: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ScanOutcome {
    pub entries: Vec<CliUsageEntry>,
    pub limits: Vec<(String, RateLimitSample)>,
    pub stats: ScanStats,
}

pub fn scan(roots: &[PathBuf], state: &mut ScanState) -> Vec<CliUsageEntry> {
    let roots: Vec<ScanRoot> = roots.iter().map(ScanRoot::guess).collect();
    scan_roots(&roots, state).entries
}

pub fn scan_roots(roots: &[ScanRoot], state: &mut ScanState) -> ScanOutcome {
    scan_roots_since(roots, state, None)
}

pub fn scan_roots_since(
    roots: &[ScanRoot],
    state: &mut ScanState,
    modified_since: Option<std::time::SystemTime>,
) -> ScanOutcome {
    let started = std::time::Instant::now();
    let mut jobs: Vec<Job> = Vec::new();
    for root in roots {
        for (path, project) in walk_jsonl(&root.path) {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if let (Some(since), Ok(modified)) = (modified_since, meta.modified()) {
                if modified < since {
                    continue;
                }
            }
            let size = meta.len();
            let mut from = state.offset(&path);
            if from > size {
                from = 0;
            }
            jobs.push(Job {
                cli: root.cli,
                account: root.account.clone(),
                project,
                path,
                from,
                size,
            });
        }
    }
    let files_seen = jobs.len() as u32;
    let todo: Vec<Job> = jobs.into_iter().filter(|j| j.size > j.from).collect();
    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).max(1))
        .unwrap_or(1)
        .clamp(1, 4)
        .min(todo.len().max(1));
    let chunk = todo.len().div_ceil(threads.max(1)).max(1);
    let mut results: Vec<FileResult> = Vec::with_capacity(todo.len());
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for slice in todo.chunks(chunk) {
            handles.push(s.spawn(move || slice.iter().map(read_file).collect::<Vec<_>>()));
        }
        for h in handles {
            if let Ok(part) = h.join() {
                results.extend(part);
            }
        }
    });

    let mut out = ScanOutcome::default();
    out.stats.files_seen = files_seen;
    for r in results {
        out.stats.files_read += 1;
        out.stats.bytes_read += r.consumed.saturating_sub(r.from);
        out.stats.lines += r.lines;
        out.stats.partial_lines += u32::from(r.partial);
        state.set_offset(r.path, r.consumed);
        out.entries.extend(r.entries);
        out.limits.extend(r.limits);
    }
    out.entries.sort_by_key(|e| e.ts_ms);
    out.limits.sort_by_key(|(_, l)| l.observed_at_ms);
    out.stats.elapsed_ms = started.elapsed().as_millis() as u64;
    out
}

#[derive(Debug, Clone)]
struct Job {
    cli: CliKind,
    account: String,
    project: String,
    path: PathBuf,
    from: u64,
    size: u64,
}

struct FileResult {
    path: PathBuf,
    from: u64,
    consumed: u64,
    lines: u64,
    partial: bool,
    entries: Vec<CliUsageEntry>,
    limits: Vec<(String, RateLimitSample)>,
}

fn read_file(job: &Job) -> FileResult {
    let mut res = FileResult {
        path: job.path.clone(),
        from: job.from,
        consumed: job.from,
        lines: 0,
        partial: false,
        entries: Vec::new(),
        limits: Vec::new(),
    };
    let session_hint = job
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let Ok(file) = std::fs::File::open(&job.path) else {
        return res;
    };
    let mut file = file;
    if job.from > 0 && file.seek(SeekFrom::Start(job.from)).is_err() {
        return res;
    }
    let mut parser = FileParser::new(job.cli, &job.account, &job.project, session_hint);
    let mut block = vec![0u8; CHUNK];
    let mut carry: Vec<u8> = Vec::with_capacity(CHUNK + 64 * 1024);
    loop {
        let n = match file.read(&mut block) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        carry.extend_from_slice(&block[..n]);
        let mut start = 0usize;
        while let Some(pos) = carry[start..].iter().position(|&b| b == b'\n') {
            let line = &carry[start..start + pos];
            res.consumed += (pos + 1) as u64;
            res.lines += 1;
            handle_line(line, job, &mut parser, &mut res);
            start += pos + 1;
        }
        carry.drain(..start);
    }
    res.partial = !carry.is_empty();
    res
}

fn handle_line(line: &[u8], job: &Job, parser: &mut FileParser, res: &mut FileResult) {
    if !super::parse::may_carry_usage(job.cli, line) {
        if job.cli == CliKind::Codex && super::parse::may_be_tool_call(line) {
            if let Ok(text) = std::str::from_utf8(line) {
                if let Some(tool) = codex_tool_name(text) {
                    res.entries.push(CliUsageEntry {
                        cli: CliKind::Codex,
                        account: job.account.clone(),
                        project: job.project.clone(),
                        ts_ms: super::parse::ts_ms(
                            serde_json::from_str::<serde_json::Value>(text)
                                .ok()
                                .as_ref()
                                .and_then(|v| v.get("timestamp")),
                        )
                        .unwrap_or(0),
                        tools: vec![tool],
                        ..Default::default()
                    });
                }
            }
        }
        return;
    }
    let Ok(text) = std::str::from_utf8(line) else {
        return;
    };
    match parser.feed(text) {
        Parsed::Usage(e) => res.entries.push(*e),
        Parsed::Limits(samples) => {
            for s in samples {
                res.limits.push((job.account.clone(), s));
            }
        }
        Parsed::Ignored => {}
    }
}

pub fn walk_jsonl(root: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, project)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if kind.is_dir() {
                let child = if project.is_empty() {
                    name
                } else {
                    project.clone()
                };
                stack.push((path, child));
            } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                let label = if project.is_empty() {
                    name.trim_end_matches(".jsonl").to_string()
                } else {
                    project.clone()
                };
                out.push((path, label));
            }
        }
    }
    out.sort();
    out
}

pub fn default_roots(accounts: &[(String, PathBuf)]) -> Vec<ScanRoot> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let claude = home.join(".claude").join("projects");
        if claude.is_dir() {
            roots.push(ScanRoot::new(CliKind::Claude, "default", claude));
        }
        let codex = home.join(".codex").join("sessions");
        if codex.is_dir() {
            roots.push(ScanRoot::new(CliKind::Codex, "default", codex));
        }
    }
    for (account, dir) in accounts {
        let claude = dir.join("projects");
        if claude.is_dir() {
            roots.push(ScanRoot::new(CliKind::Claude, account.clone(), claude));
        }
        let codex = dir.join("sessions");
        if codex.is_dir() {
            roots.push(ScanRoot::new(CliKind::Codex, account.clone(), codex));
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "omniget-cli-usage-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn line(ts: &str, id: &str, out: u64) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"s1","timestamp":"{ts}","requestId":"{id}","message":{{"id":"{id}","role":"assistant","model":"claude-opus-5","content":[{{"type":"tool_use","name":"Bash","input":{{}}}}],"usage":{{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":{out}}}}}}}"#
        )
    }

    #[test]
    fn a_second_scan_reads_nothing_new() {
        let dir = tmpdir("incremental");
        let f = dir.join("a.jsonl");
        std::fs::write(
            &f,
            format!("{}\n", line("2026-03-04T12:00:00.000Z", "r1", 5)),
        )
        .unwrap();
        let mut state = ScanState::new();
        let first = scan(std::slice::from_ref(&dir), &mut state);
        assert_eq!(first.len(), 1);
        let second = scan(std::slice::from_ref(&dir), &mut state);
        assert!(second.is_empty(), "warm scan must read nothing");

        let mut fh = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        writeln!(fh, "{}", line("2026-03-04T12:05:00.000Z", "r2", 7)).unwrap();
        drop(fh);
        let third = scan(std::slice::from_ref(&dir), &mut state);
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].output_tokens, 7);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_truncated_last_line_is_ignored_then_resumed() {
        let dir = tmpdir("truncated");
        let f = dir.join("a.jsonl");
        let whole = line("2026-03-04T12:00:00.000Z", "r1", 5);
        let next = line("2026-03-04T12:01:00.000Z", "r2", 9);
        let cut = &next[..40];
        std::fs::write(&f, format!("{whole}\n{cut}")).unwrap();
        let mut state = ScanState::new();
        let first = scan(std::slice::from_ref(&dir), &mut state);
        assert_eq!(first.len(), 1, "the half-written line must not count");
        std::fs::write(&f, format!("{whole}\n{next}\n")).unwrap();
        let second = scan(std::slice::from_ref(&dir), &mut state);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].output_tokens, 9);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_rewritten_shorter_file_is_read_from_zero() {
        let dir = tmpdir("rewrite");
        let f = dir.join("a.jsonl");
        std::fs::write(
            &f,
            format!(
                "{}\n{}\n",
                line("2026-03-04T12:00:00.000Z", "r1", 5),
                line("2026-03-04T12:01:00.000Z", "r2", 6)
            ),
        )
        .unwrap();
        let mut state = ScanState::new();
        assert_eq!(scan(std::slice::from_ref(&dir), &mut state).len(), 2);
        std::fs::write(
            &f,
            format!("{}\n", line("2026-03-04T13:00:00.000Z", "r3", 8)),
        )
        .unwrap();
        let again = scan(std::slice::from_ref(&dir), &mut state);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].output_tokens, 8);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_jsonl_files_are_ever_opened() {
        let dir = tmpdir("creds");
        std::fs::write(
            dir.join(".credentials.json"),
            "{\"accessToken\":\"secret\"}",
        )
        .unwrap();
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        std::fs::write(dir.join("a.jsonl.lock"), "x").unwrap();
        std::fs::write(
            dir.join("a.jsonl"),
            format!("{}\n", line("2026-03-04T12:00:00.000Z", "r1", 5)),
        )
        .unwrap();
        let files = walk_jsonl(&dir);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0.file_name().unwrap(), "a.jsonl");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_walk_is_recursive_and_labels_the_project() {
        let dir = tmpdir("walk");
        let proj = dir.join("-home-lorem-ipsum").join("subagents");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("agent-1.jsonl"), "").unwrap();
        std::fs::write(dir.join("-home-lorem-ipsum").join("s1.jsonl"), "").unwrap();
        let files = walk_jsonl(&dir);
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|(_, p)| p == "-home-lorem-ipsum"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn state_survives_a_save_and_load_round_trip() {
        let dir = tmpdir("state");
        let f = dir.join("cli-usage-state.json");
        let mut state = ScanState::new();
        state.set_offset(dir.join("a.jsonl"), 4242);
        state.save(&f).unwrap();
        let back = ScanState::load(&f);
        assert_eq!(back.offset(&dir.join("a.jsonl")), 4242);
        assert_eq!(back.len(), 1);
        let mut back = back;
        back.prune_missing();
        assert!(back.is_empty(), "the file does not exist any more");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_cli_is_guessed_from_the_folder() {
        assert_eq!(
            ScanRoot::guess(PathBuf::from("/home/lorem/.codex/sessions")).cli,
            CliKind::Codex
        );
        assert_eq!(
            ScanRoot::guess(PathBuf::from("/home/lorem/.claude/projects")).cli,
            CliKind::Claude
        );
    }

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cli_usage_fixtures")
    }

    #[test]
    fn the_claude_fixture_parses_the_way_the_real_file_does() {
        let dir = tmpdir("fixture-claude");
        std::fs::copy(
            fixtures().join("claude-2.1.275.jsonl"),
            dir.join("claude-2.1.275.jsonl"),
        )
        .unwrap();
        let mut state = ScanState::new();
        let out = scan_roots(
            &[ScanRoot::new(CliKind::Claude, "default", &dir)],
            &mut state,
        );
        assert_eq!(out.entries.len(), 4, "{:?}", out.entries);
        assert_eq!(out.stats.partial_lines, 1);
        assert!(out.entries.iter().any(|e| e.sidechain));
        assert!(out
            .entries
            .iter()
            .any(|e| e.tools.iter().any(|t| t == "Grep")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_old_claude_fixture_still_parses() {
        let dir = tmpdir("fixture-old");
        std::fs::copy(
            fixtures().join("claude-2.1.226.jsonl"),
            dir.join("claude-2.1.226.jsonl"),
        )
        .unwrap();
        let mut state = ScanState::new();
        let out = scan_roots(
            &[ScanRoot::new(CliKind::Claude, "default", &dir)],
            &mut state,
        );
        assert_eq!(out.entries.len(), 2);
        assert!(out.entries.iter().all(|e| e.reasoning_tokens == 0));
        assert_eq!(out.entries[0].input_tokens, 12_100);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_codex_fixture_yields_usage_tools_and_windows() {
        let dir = tmpdir("fixture-codex");
        std::fs::copy(
            fixtures().join("codex-rollout.jsonl"),
            dir.join("codex-rollout.jsonl"),
        )
        .unwrap();
        let mut state = ScanState::new();
        let out = scan_roots(&[ScanRoot::new(CliKind::Codex, "work", &dir)], &mut state);
        let billed: Vec<_> = out
            .entries
            .iter()
            .filter(|e| e.input_tokens + e.output_tokens > 0)
            .collect();
        assert_eq!(billed.len(), 2);
        assert_eq!(billed[0].model, "gpt-6-astra");
        assert_eq!(billed[1].model, "gpt-6-mini");
        assert!(out
            .entries
            .iter()
            .any(|e| e.tools.iter().any(|t| t == "exec")));
        assert_eq!(out.limits.len(), 2);
        assert_eq!(out.limits[0].0, "work");
        assert_eq!(out.stats.partial_lines, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn logs_untouched_since_the_cutoff_are_not_read() {
        let dir = tmpdir("since");
        let old = dir.join("old.jsonl");
        let new = dir.join("new.jsonl");
        std::fs::write(
            &old,
            format!("{}\n", line("2026-03-01T12:00:00.000Z", "r-old", 5)),
        )
        .unwrap();
        std::fs::write(
            &new,
            format!("{}\n", line("2026-03-04T12:00:00.000Z", "r-new", 5)),
        )
        .unwrap();
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3_600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(cutoff - std::time::Duration::from_secs(86_400))
            .unwrap();
        let roots = [ScanRoot::new(CliKind::Claude, "me", &dir)];
        let recent = scan_roots_since(&roots, &mut ScanState::default(), Some(cutoff));
        assert_eq!(recent.stats.files_seen, 1);
        assert_eq!(recent.entries.len(), 1);
        let all = scan_roots(&roots, &mut ScanState::default());
        assert_eq!(all.entries.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
