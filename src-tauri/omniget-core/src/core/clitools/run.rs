use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::exec::command_for;
use super::owner::CommandSpec;
use super::ERR_FAILED;

pub const MAX_OUTPUT: usize = 10_000;
pub const RUN_TIMEOUT: Duration = Duration::from_secs(20 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Succeeded,
    Failed,
    Unchanged,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub run_id: String,
    pub tool_id: String,
    pub kind: String,
    pub stream: Option<String>,
    pub line: Option<String>,
    pub state: RunState,
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub run_id: String,
    pub plan_id: String,
    pub tool_id: String,
    pub state: RunState,
    pub exit_code: Option<i32>,
    pub output: String,
    pub truncated: bool,
    pub version_before: Option<String>,
    pub version_after: Option<String>,
    pub duration_ms: u64,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct TailBuffer {
    text: String,
    truncated: bool,
}

impl TailBuffer {
    pub fn push_line(&mut self, line: &str) {
        self.text.push_str(line);
        self.text.push('\n');
        let chars = self.text.chars().count();
        if chars > MAX_OUTPUT {
            let drop = chars - MAX_OUTPUT;
            let cut = self
                .text
                .char_indices()
                .nth(drop)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len());
            self.text.drain(..cut);
            self.truncated = true;
        }
    }
    pub fn into_parts(self) -> (String, bool) {
        (self.text, self.truncated)
    }
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

fn last_runs() -> &'static Mutex<HashMap<String, RunResult>> {
    use std::sync::OnceLock;
    static R: OnceLock<Mutex<HashMap<String, RunResult>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn last_run(tool_id: &str) -> Option<RunResult> {
    last_runs().lock().ok()?.get(tool_id).cloned()
}

pub async fn run_command(
    run_id: &str,
    tool_id: &str,
    spec: &CommandSpec,
    on: &ProgressFn,
) -> Result<(Option<i32>, String, bool), String> {
    let mut cmd = command_for(std::path::Path::new(&spec.program));
    cmd.args(&spec.args)
        .envs(&spec.env)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = crate::core::process::spawn_retrying_busy(|| cmd.spawn())
        .map_err(|e| format!("{ERR_FAILED}: {}: {e}", spec.program))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let buf = Arc::new(Mutex::new(TailBuffer::default()));

    let pump = |stream: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>,
                name: &'static str| {
        let buf = buf.clone();
        let on = on.clone();
        let run_id = run_id.to_string();
        let tool_id = tool_id.to_string();
        async move {
            let Some(s) = stream else { return };
            let mut lines = BufReader::new(s).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let clean = strip_ansi(&line);
                if let Ok(mut b) = buf.lock() {
                    b.push_line(&clean);
                }
                on(Progress {
                    run_id: run_id.clone(),
                    tool_id: tool_id.clone(),
                    kind: "line".into(),
                    stream: Some(name.into()),
                    line: Some(clean),
                    state: RunState::Running,
                    command: None,
                });
            }
        }
    };
    let out_task = pump(
        stdout.map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        "stdout",
    );
    let err_task = pump(
        stderr.map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        "stderr",
    );
    let waited = tokio::time::timeout(RUN_TIMEOUT, async {
        tokio::join!(out_task, err_task);
        child.wait().await
    })
    .await;
    let code = match waited {
        Ok(Ok(status)) => status.code(),
        Ok(Err(e)) => return Err(format!("{ERR_FAILED}: {e}")),
        Err(_) => {
            if let Ok(mut b) = buf.lock() {
                b.push_line("[omniget] timed out; process terminated");
            }
            None
        }
    };
    let (text, truncated) =
        std::mem::take(&mut *buf.lock().unwrap_or_else(|e| e.into_inner())).into_parts();
    Ok((code, text, truncated))
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for n in it.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\r' {
            out.clear();
            continue;
        }
        out.push(c);
    }
    out
}
