use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::detect;
use super::exec::command_for;
use super::owner::CommandSpec;
use super::plan::{self, Action, Plan};
use super::table;
use super::{ERR_BUSY, ERR_FAILED, ERR_NOT_FOUND, ERR_NOT_RUNNABLE, ERR_STALE};

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

type Locks = Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

fn lock_for(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    use std::sync::OnceLock;
    static L: OnceLock<Locks> = OnceLock::new();
    let map = L.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().unwrap_or_else(|e| e.into_inner());
    g.entry(key.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
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
                b.push_line("[omniget] tempo esgotado; processo encerrado");
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

pub async fn run_plan(plan_id: &str, on: ProgressFn) -> Result<RunResult, String> {
    let plan: Plan = plan::lookup(plan_id)
        .ok_or_else(|| format!("{ERR_NOT_FOUND}: plano {plan_id} expirou ou não existe"))?;
    if !plan.runnable {
        return Err(format!(
            "{ERR_NOT_RUNNABLE}: {}",
            plan.reason
                .clone()
                .unwrap_or_else(|| "plano só para copiar".into())
        ));
    }
    let tool = table::get(&plan.tool_id)
        .ok_or_else(|| format!("{ERR_NOT_FOUND}: ferramenta {}", plan.tool_id))?;

    let tool_lock = lock_for(&format!("tool:{}", tool.id));
    let _tool_guard = tool_lock
        .try_lock()
        .map_err(|_| format!("{ERR_BUSY}: {} já está instalando/atualizando", tool.id))?;
    let key_lock = lock_for(&plan.command.lock_key);
    let _key_guard = key_lock.lock().await;

    let before = detect::detect_one(tool).await;
    if plan.action == Action::Update {
        let fresh = before.owner.update.as_ref();
        let same = fresh
            .map(|f| f.program == plan.command.program && f.args == plan.command.args)
            .unwrap_or(false);
        if !(before.owner.proven && same) {
            return Err(format!(
                "{ERR_STALE}: a instalação mudou desde o plano ({}); gere o plano de novo",
                before.owner.evidence
            ));
        }
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let started = Instant::now();
    on(Progress {
        run_id: run_id.clone(),
        tool_id: tool.id.clone(),
        kind: "start".into(),
        stream: None,
        line: None,
        state: RunState::Running,
        command: Some(plan.command.display.clone()),
    });

    let (code, output, truncated) = match run_command(&run_id, &tool.id, &plan.command, &on).await {
        Ok(v) => v,
        Err(e) => {
            on(Progress {
                run_id: run_id.clone(),
                tool_id: tool.id.clone(),
                kind: "end".into(),
                stream: None,
                line: Some(e.clone()),
                state: RunState::Failed,
                command: None,
            });
            return Err(e);
        }
    };

    let after = detect::detect_one(tool).await;
    let mut warnings = plan.warnings.clone();
    let lower = output.to_ascii_lowercase();
    if lower.contains("allow-scripts")
        || lower.contains("scripts were not run")
        || lower.contains("install scripts") && lower.contains("skipped")
    {
        warnings
            .push("O npm avisou sobre scripts bloqueados; confira se o binário funciona.".into());
    }
    let state = if code == Some(0) && after.installed {
        if plan.action == Action::Update
            && before.version.is_some()
            && before.version == after.version
        {
            RunState::Unchanged
        } else {
            RunState::Succeeded
        }
    } else {
        if code == Some(0) && !after.installed {
            warnings
                .push("O comando terminou mas o binário não apareceu no PATH conhecido.".into());
        }
        RunState::Failed
    };
    let result = RunResult {
        run_id: run_id.clone(),
        plan_id: plan.id.clone(),
        tool_id: tool.id.clone(),
        state,
        exit_code: code,
        output,
        truncated,
        version_before: before.version,
        version_after: after.version,
        duration_ms: started.elapsed().as_millis() as u64,
        warnings,
    };
    if let Ok(mut m) = last_runs().lock() {
        m.insert(tool.id.clone(), result.clone());
    }
    on(Progress {
        run_id,
        tool_id: tool.id.clone(),
        kind: "end".into(),
        stream: None,
        line: None,
        state,
        command: None,
    });
    Ok(result)
}
