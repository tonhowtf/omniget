use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::validate_url;

pub const PROGRAMS: &[&str] = &["npx", "pnpm", "yarn", "bunx"];

const FLAGS: &[&str] = &["--yes", "--defaults", "--overwrite", "--bun"];

pub const MAX_OUTPUT_LINES: usize = 2000;
const MAX_LINE_CHARS: usize = 4000;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RunLine {
    pub stream: &'static str,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RunOutcome {
    pub ok: bool,
    pub code: Option<i32>,
    pub cancelled: bool,
    pub error: Option<String>,
    pub output: Vec<RunLine>,
}

impl RunOutcome {
    fn failed(error: &str) -> Self {
        Self {
            ok: false,
            code: None,
            cancelled: false,
            error: Some(error.into()),
            output: Vec::new(),
        }
    }
}

pub fn validate(program: &str, args: &[String]) -> Result<(), &'static str> {
    if !PROGRAMS.contains(&program) {
        return Err("run_program");
    }
    let mut rest: &[String] = args;
    if matches!(program, "pnpm" | "yarn") {
        match rest.first().map(String::as_str) {
            Some("dlx") => rest = &rest[1..],
            _ => return Err("run_args"),
        }
    }
    if program == "bunx" && rest.first().map(String::as_str) == Some("--bun") {
        rest = &rest[1..];
    }
    if rest.first().map(String::as_str) != Some("shadcn@latest") {
        return Err("run_args");
    }
    rest = &rest[1..];
    let action = rest.first().map(String::as_str);
    rest = match action {
        Some("add") | Some("init") => &rest[1..],
        _ => return Err("run_args"),
    };
    let mut targets = 0;
    for arg in rest {
        if FLAGS.contains(&arg.as_str()) {
            continue;
        }
        if action == Some("add") && validate_url(arg).is_ok() {
            targets += 1;
            continue;
        }
        return Err("run_args");
    }
    if action == Some("add") && targets == 0 {
        return Err("run_args");
    }
    if rest.len() > 24 {
        return Err("run_args");
    }
    Ok(())
}

fn resolve_program(program: &str) -> String {
    if cfg!(windows) && program != "bunx" {
        format!("{program}.cmd")
    } else {
        program.to_string()
    }
}

pub async fn run<F>(
    cwd: &Path,
    program: &str,
    args: &[String],
    token: CancellationToken,
    on_line: F,
) -> RunOutcome
where
    F: Fn(&'static str, String) + Send + Sync,
{
    if let Err(code) = validate(program, args) {
        return RunOutcome::failed(code);
    }
    if !cwd.is_dir() {
        return RunOutcome::failed("project_missing");
    }
    stream_process(cwd, &resolve_program(program), args, token, on_line).await
}

async fn stream_process<F>(
    cwd: &Path,
    program: &str,
    args: &[String],
    token: CancellationToken,
    on_line: F,
) -> RunOutcome
where
    F: Fn(&'static str, String) + Send + Sync,
{
    let mut cmd = crate::core::process::command(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .env("CI", "1")
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    crate::core::process::suspend_on_spawn(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let code = if e.kind() == std::io::ErrorKind::NotFound {
                "run_not_found"
            } else {
                "run_spawn"
            };
            return RunOutcome::failed(code);
        }
    };
    let mut tree = match crate::core::process::ProcessTreeGuard::new(&child) {
        Ok(t) => t,
        Err(_) => {
            let _ = child.start_kill();
            return RunOutcome::failed("run_spawn");
        }
    };
    let collected: Mutex<VecDeque<RunLine>> = Mutex::new(VecDeque::new());
    let push = |stream: &'static str, line: String| {
        let line: String = line.chars().take(MAX_LINE_CHARS).collect();
        if let Ok(mut buf) = collected.lock() {
            if buf.len() >= MAX_OUTPUT_LINES {
                buf.pop_front();
            }
            buf.push_back(RunLine {
                stream,
                line: line.clone(),
            });
        }
        on_line(stream, line);
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let read_out = async {
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                push("stdout", line);
            }
        }
    };
    let read_err = async {
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                push("stderr", line);
            }
        }
    };
    let finished = async {
        tokio::join!(read_out, read_err);
        child.wait().await
    };
    let status = tokio::select! {
        status = finished => Some(status),
        _ = token.cancelled() => None,
    };
    let mut outcome = match status {
        Some(Ok(s)) => {
            tree.release();
            RunOutcome {
                ok: s.success(),
                code: s.code(),
                cancelled: false,
                error: if s.success() {
                    None
                } else {
                    Some("run_exit".into())
                },
                output: Vec::new(),
            }
        }
        Some(Err(_)) => {
            tree.terminate();
            RunOutcome::failed("run_spawn")
        }
        None => {
            tree.terminate();
            RunOutcome {
                cancelled: true,
                ..RunOutcome::failed("run_cancelled")
            }
        }
    };
    drop(child);
    outcome.output = collected.into_inner().map(Vec::from).unwrap_or_default();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    const ITEM: &str = "https://ui.shadcn.com/r/styles/new-york-v4/button.json";

    #[test]
    fn accepts_known_shapes() {
        assert!(validate("pnpm", &v(&["dlx", "shadcn@latest", "add", ITEM, "--yes"])).is_ok());
        assert!(validate("yarn", &v(&["dlx", "shadcn@latest", "add", ITEM])).is_ok());
        assert!(validate("npx", &v(&["shadcn@latest", "add", ITEM, "--yes"])).is_ok());
        assert!(validate("bunx", &v(&["--bun", "shadcn@latest", "add", ITEM])).is_ok());
        assert!(validate("npx", &v(&["shadcn@latest", "init", "--yes", "--defaults"])).is_ok());
    }

    #[test]
    fn rejects_anything_else() {
        assert!(validate("sh", &v(&["-c", "rm -rf /"])).is_err());
        assert!(validate("npx", &v(&["evil@latest", "add", ITEM])).is_err());
        assert!(validate("pnpm", &v(&["shadcn@latest", "add", ITEM])).is_err());
        assert!(validate("npx", &v(&["shadcn@latest", "add"])).is_err());
        assert!(validate("npx", &v(&["shadcn@latest", "add", "http://x.dev/a.json"])).is_err());
        assert!(validate("npx", &v(&["shadcn@latest", "add", ITEM, "--cwd", "/"])).is_err());
        assert!(validate("npx", &v(&["shadcn@latest", "init", ITEM])).is_err());
        assert!(validate("npx", &v(&["shadcn@latest", "build"])).is_err());
    }

    #[tokio::test]
    async fn run_refuses_invalid_before_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let out = run(
            dir.path(),
            "sh",
            &v(&["-c", "echo hi"]),
            CancellationToken::new(),
            |_, _| {},
        )
        .await;
        assert!(!out.ok);
        assert_eq!(out.error.as_deref(), Some("run_program"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stream_returns_every_line_in_the_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let out = stream_process(
            dir.path(),
            "sh",
            &v(&[
                "-c",
                "echo one; echo two >&2; echo Created 1 file:; echo '  - a.tsx'",
            ]),
            CancellationToken::new(),
            |_, _| {},
        )
        .await;
        assert!(out.ok, "{out:?}");
        let lines: Vec<&str> = out.output.iter().map(|l| l.line.as_str()).collect();
        assert!(lines.contains(&"one"));
        assert!(lines.contains(&"two"));
        let stdout: Vec<&str> = out
            .output
            .iter()
            .filter(|l| l.stream == "stdout")
            .map(|l| l.line.as_str())
            .collect();
        assert_eq!(stdout.last(), Some(&"  - a.tsx"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_kills_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let token = CancellationToken::new();
        let pid = std::sync::Arc::new(std::sync::Mutex::new(None::<i32>));
        let seen = pid.clone();
        let cancel = token.clone();
        let out = stream_process(
            dir.path(),
            "sh",
            &v(&["-c", "sleep 30 & echo $!; wait"]),
            token,
            move |_, line| {
                if let Ok(p) = line.trim().parse::<i32>() {
                    *seen.lock().unwrap() = Some(p);
                    cancel.cancel();
                }
            },
        )
        .await;
        assert!(out.cancelled);
        let grandchild = pid.lock().unwrap().expect("pid printed");
        let mut gone = false;
        for _ in 0..50 {
            // SAFETY: signal 0 only checks whether the pid exists.
            if unsafe { libc::kill(grandchild, 0) } != 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(gone, "grandchild {grandchild} survived the cancel");
    }
}
