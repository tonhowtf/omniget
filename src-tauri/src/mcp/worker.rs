//! Queue adapter: the existing core downloader runs in a per-execution worker.
//! No second queue/engine/database. This adapter is only constructed after MCP
//! domain authorization and receives exact local exceptions from trusted code.
use super::policy::{self, Principal};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use omniget_core::{
    core::egress::{Broker, LimitReason, Policy},
    models::{
        media::{DownloadOptions, DownloadResult, MediaInfo},
        progress::ProgressUpdate,
    },
    platforms::PlatformDownloader,
};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::PathBuf, sync::Mutex, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Child,
    sync::{mpsc, OnceCell},
};
use tokio_util::sync::CancellationToken;

/// Fixed error context; no provider URL, command or raw stderr survives here.
#[derive(Debug)]
pub struct WorkerFailure {
    pub code: String,
    pub retry_after_seconds: Option<u64>,
}
impl std::fmt::Display for WorkerFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self.code.as_str() {
            "RATE_LIMITED" => "HTTP 429 rate limited",
            "AUTH_REQUIRED" => "HTTP 401 authentication required",
            "ACCESS_DENIED" => "HTTP 403 access denied",
            "NOT_FOUND" => "HTTP 404 not found",
            // Codes keep their token first: diagnosis matches on it and the
            // desktop's generic "not found"/"429" heuristics must not relabel it.
            "FORMAT_UNAVAILABLE" => "FORMAT_UNAVAILABLE: requested format is not available within the requested options",
            "BROKEN_SOURCE" => "BROKEN_SOURCE: the source stopped serving part of the media (fragment failure)",
            "BLOCKED_BY_PLATFORM" => "BLOCKED_BY_PLATFORM: the platform blocked access from this network",
            "BOT_CHALLENGE" => "BOT_CHALLENGE: the platform requires a bot challenge",
            "EXTRACTOR_FAILURE" => "EXTRACTOR_FAILURE: unable to extract this page; the site may have changed",
            "SERVER_ERROR" => "SERVER_ERROR: the server had a temporary error (HTTP 5xx); retry later",
            "EGRESS_FAILED" => "EGRESS_FAILED: network egress failed (proxy, TLS or DNS) before the platform answered; retry later",
            // OmniGet's own egress policy refused the connection: not a remote
            // 403, and only a local grant (or a public URL) changes it.
            "EGRESS_BLOCKED" => "EGRESS_BLOCKED: blocked by OmniGet network policy, not by the platform; grant this local address to the connection in OmniGet or use a public URL",
            // OmniGet's per-download budgets (a policy refusal, not an engine
            // or platform failure): the media is larger or longer than allowed.
            "LIMIT_BYTES" => "LIMIT_BYTES: stopped at OmniGet's per-download size budget; choose a lower quality or a shorter item",
            "LIMIT_TIME" => "LIMIT_TIME: stopped at OmniGet's per-download time budget; choose a lower quality or a shorter item",
            "LIMIT_CONNECTIONS" => "LIMIT_CONNECTIONS: stopped at OmniGet's per-download connection budget",
            _ => self.code.as_str(),
        };
        write!(f, "{label}")?;
        if let Some(seconds) = self.retry_after_seconds {
            write!(f, "; retry_after_seconds={seconds}")?;
        }
        Ok(())
    }
}
impl std::error::Error for WorkerFailure {}
/// The worker's raw engine error, for the local log only: bounded, control
/// characters removed, URLs redacted (signed links carry secrets) and local
/// profile paths cut to file names. Worker output is untrusted input.
fn operator_diagnostic(event: &Value) -> Option<String> {
    let raw = event["detail"].as_str()?;
    let clean: String = raw
        .chars()
        .take(2000)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let redacted = crate::core::flight_recorder::redact(&clean);
    Some(omniget_core::core::paths::redact_private_paths(
        &redacted, None,
    ))
}
pub struct WorkerDownloader {
    principal: Principal,
    url: String,
    helper: PathBuf,
    allowed_sockets: BTreeSet<std::net::SocketAddr>,
    selected_media: Mutex<Option<(String, MediaInfo)>>,
    dependencies: OnceCell<Value>,
    /// Private per-job directory where the inspect worker leaves yt-dlp's
    /// info JSON for the download worker (one extraction instead of two).
    info_json: std::sync::OnceLock<Option<InfoJsonDir>>,
}
impl WorkerDownloader {
    pub fn new(
        principal: Principal,
        url: String,
        helper: Option<PathBuf>,
        _allowed_sockets: BTreeSet<std::net::SocketAddr>,
    ) -> Result<Self> {
        policy::active(&principal).map_err(|_| anyhow!("PRINCIPAL_INACTIVE"))?;
        let allowed_sockets = super::network_grants::allowed(&principal)
            .map_err(|_| anyhow!("NETWORK_GRANTS_UNAVAILABLE"))?;
        let parsed = reqwest::Url::parse(&url).map_err(|_| anyhow!("INVALID_URL"))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(anyhow!("INVALID_URL"));
        }
        let helper = match helper {
            Some(path) => path,
            None => default_helper()?,
        };
        if !helper.is_absolute() || !helper.is_file() {
            return Err(anyhow!("WORKER_UNAVAILABLE"));
        }
        Ok(Self {
            principal,
            url,
            helper,
            allowed_sockets,
            selected_media: Mutex::new(None),
            dependencies: OnceCell::new(),
            info_json: std::sync::OnceLock::new(),
        })
    }
    fn check_network_grants(&self) -> Result<()> {
        let current = super::network_grants::allowed(&self.principal)
            .map_err(|_| anyhow!("NETWORK_GRANTS_UNAVAILABLE"))?;
        if current != self.allowed_sockets {
            return Err(anyhow!("NETWORK_GRANTS_CHANGED"));
        }
        Ok(())
    }
    /// Paginated collection metadata through the same principal/egress boundary.
    pub async fn collection(&self, offset: u64, limit: u64) -> Result<Value> {
        if offset > 10000 || !(1..=50).contains(&limit) {
            return Err(anyhow!("INVALID_PAGE"));
        }
        let event = self
            .dispatch(
                json!({"operation":"collection","offset":offset,"limit":limit}),
                CancellationToken::new(),
                None,
            )
            .await?;
        event
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("WORKER_COLLECTION"))
    }
    async fn tools(&self) -> Value {
        self.dependencies.get_or_init(||async{
            // find_ytdlp_cached may provision a zipapp; don't invoke it here.
            let ytdlp=omniget_core::core::ytdlp::managed_ytdlp_path().filter(|p|p.is_file());
            // Onedir (0.4 s per process against 13-25 s for the onefile) when installed.
            let ytdlp=omniget_core::core::ytdlp::managed_ytdlp_onedir_exe().or(ytdlp);
            let (ffmpeg,ffprobe,node,deno)=tokio::join!(
                omniget_core::core::dependencies::find_tool("ffmpeg"),
                omniget_core::core::dependencies::find_tool("ffprobe"),
                omniget_core::core::dependencies::find_tool("node"),
                omniget_core::core::dependencies::find_tool("deno"));
            json!({"ytdlp_path":ytdlp,"ffmpeg_path":ffmpeg,"ffprobe_path":ffprobe,"node_path":node,"deno_path":deno})
        }).await.clone()
    }
    async fn dispatch(
        &self,
        mut request: Value,
        cancel: CancellationToken,
        progress: Option<mpsc::Sender<ProgressUpdate>>,
    ) -> Result<Value> {
        let started = std::time::Instant::now();
        let operation = request["operation"].as_str().unwrap_or("?").to_owned();
        policy::active(&self.principal).map_err(|_| anyhow!("PRINCIPAL_INACTIVE"))?;
        if cancel.is_cancelled() {
            return Err(anyhow!("CANCELLED"));
        }
        self.check_network_grants()?;
        let runtime = RuntimeDir::create()?;
        let mut policy = Policy::default();
        policy.local_allowances = self.allowed_sockets.clone();
        let broker = Broker::start(policy, cancel.clone())
            .await
            .map_err(|_| anyhow!("EGRESS_UNAVAILABLE"))?;
        request["version"] = json!(1);
        request["url"] = json!(self.url);
        request["proxy"] = json!(broker.proxy_url());
        request["runtime_dir"] = json!(runtime.0);
        let info_json = self
            .info_json
            .get_or_init(|| InfoJsonDir::create().ok())
            .as_ref()
            .map(|d| d.0.clone());
        if let Some(dir) = &info_json {
            request["info_json_dir"] = json!(dir);
        }
        for (key, value) in self
            .tools()
            .await
            .as_object()
            .ok_or_else(|| anyhow!("WORKER_DEPENDENCIES"))?
        {
            if request.get(key).is_none() || request[key].is_null() {
                request[key] = value.clone();
            }
        }
        // Match the canonical paths granted by Seatbelt (e.g. /tmp aliases).
        for key in [
            "output_dir",
            "ytdlp_path",
            "ffmpeg_path",
            "ffprobe_path",
            "node_path",
            "deno_path",
        ] {
            if let Some(raw) = request[key].as_str() {
                request[key] = json!(PathBuf::from(raw)
                    .canonicalize()
                    .map_err(|_| anyhow!("WORKER_PATH_UNAVAILABLE"))?);
            }
        }
        let bytes = serde_json::to_vec(&request).map_err(|_| anyhow!("WORKER_ENCODING"))?;
        if bytes.len() > 65536 {
            return Err(anyhow!("WORKER_INPUT_LIMIT"));
        }
        policy::active(&self.principal).map_err(|_| anyhow!("PRINCIPAL_REVOKED"))?;
        self.check_network_grants()?;
        let reads = [
            "ytdlp_path",
            "ffmpeg_path",
            "ffprobe_path",
            "node_path",
            "deno_path",
        ]
        .iter()
        .filter_map(|key| request[*key].as_str().map(PathBuf::from))
        .collect::<Vec<_>>();
        let onedir = request["ytdlp_path"]
            .as_str()
            .and_then(|p| onedir_root(std::path::Path::new(p)));
        let mut binaries = reads;
        binaries.push(self.helper.clone());
        let prepared_at = started.elapsed();
        let mut reads = tokio::select! {
            _ = cancel.cancelled() => return Err(anyhow!("CANCELLED")),
            result = omniget_core::core::engine_files::dependency_files(&binaries) =>
                result.map_err(|_| anyhow!("WORKER_DEPENDENCY_FILES_UNAVAILABLE"))?,
        };
        let dependencies_at = started.elapsed();
        // yt-dlp onedir loads _internal/ (base_library.zip, extension modules via
        // dlopen) on demand: those are not Mach-O load commands, so the whole
        // directory is granted read-only (Seatbelt subpath).
        if let Some(dir) = onedir {
            reads.push(dir);
        }
        self.check_network_grants()?;
        if cancel.is_cancelled() {
            return Err(anyhow!("CANCELLED"));
        }
        let mut writes = vec![runtime.0.clone()];
        if let Some(dir) = info_json {
            writes.push(dir);
        }
        if let Some(output) = request["output_dir"].as_str() {
            writes.push(PathBuf::from(output));
        }
        let mut command = broker
            .confined_command_with_files(&self.helper, &reads, &writes)
            .map_err(|_| anyhow!("WORKER_SANDBOX_UNAVAILABLE"))?;
        command.current_dir(&runtime.0).env("TMPDIR", &runtime.0);
        let child = command
            .spawn()
            .map_err(|_| anyhow!("WORKER_START_FAILED"))?;
        let spawned_at = started.elapsed();
        let mut first_event_at = None;
        let mut first_byte_at = None;
        let mut output_closed_at = None;
        let mut process = ProcessGuard::new(child)?;
        let mut input = process
            .child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("WORKER_STDIN"))?;
        // Retain only a bounded number of stderr bytes, then discard. Never send
        // raw stderr to diagnostics: core subprocess errors may include secrets.
        let stderr = process
            .child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("WORKER_STDERR"))?;
        let stderr_task = tokio::spawn(async move {
            let mut stderr = stderr;
            let mut b = [0; 4096];
            let mut discarded = 0u64;
            loop {
                match stderr.read(&mut b).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => discarded = discarded.saturating_add(n as u64),
                }
            }
            discarded
        });
        let stdout = process
            .child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("WORKER_STDOUT"))?;
        let result=async{
            tokio::time::timeout(Duration::from_secs(5),input.write_all(&bytes)).await.map_err(|_|anyhow!("WORKER_INPUT_TIMEOUT"))?.map_err(|_|anyhow!("WORKER_INPUT_FAILED"))?;
            drop(input); // EOF is the request framing boundary.
            let mut reader=BufReader::new(stdout);let mut pending_line=Vec::new();let mut total=0usize;let mut final_result=None;
            let mut tick=tokio::time::interval(Duration::from_millis(250));
            loop{
                tokio::select!{
                    _=cancel.cancelled()=>return Err(anyhow!("CANCELLED")),
                    _=tick.tick()=>{
                        policy::active(&self.principal).map_err(|_|anyhow!("PRINCIPAL_REVOKED"))?;
                        self.check_network_grants()?;
                        if broker.is_revoked(){return Err(match broker.limit_reason(){Some(reason)=>anyhow!(WorkerFailure{code:reason.code().into(),retry_after_seconds:None}),None=>anyhow!("EGRESS_REVOKED_OR_BUDGET")})}
                    },
                    line=bounded_line(&mut reader, &mut pending_line)=>{
                        let Some(line)=line? else{break};
                        total=total.checked_add(line.len()).ok_or_else(||anyhow!("WORKER_OUTPUT_LIMIT"))?;
                        if total>32*1024*1024{return Err(anyhow!("WORKER_OUTPUT_LIMIT"))}
                        let event:Value=serde_json::from_slice(&line).map_err(|_|anyhow!("WORKER_INVALID_OUTPUT"))?;
                        first_event_at.get_or_insert_with(||started.elapsed());
                        if first_byte_at.is_none()&&event["downloaded_bytes"].as_u64().is_some_and(|b|b>0){first_byte_at=Some(started.elapsed());}
                        match event["type"].as_str(){
                            Some("progress")=>{
                                if final_result.is_some(){return Err(anyhow!("WORKER_OUTPUT_ORDER"))}
                                if let Some(tx)=&progress{
                                    let p=progress_from_event(&event);
                                    // Progress must not prevent revocation polling.
                                    let _=tx.try_send(p);
                                }
                            },
                            Some("result")|Some("media_info")=>{if final_result.replace(event).is_some(){return Err(anyhow!("WORKER_DUPLICATE_RESULT"))}},
                            Some("diagnostic")=>{if let Some(detail)=operator_diagnostic(&event){tracing::error!("worker diagnostic (operator log only): {detail}");}},
                            _=>return Err(anyhow!("WORKER_INVALID_OUTPUT")),
                        }
                    }
                }
            }
            output_closed_at=Some(started.elapsed());
            let status=tokio::time::timeout(Duration::from_secs(5),process.child.wait()).await.map_err(|_|anyhow!("WORKER_EXIT_TIMEOUT"))?.map_err(|_|anyhow!("WORKER_WAIT_FAILED"))?;
            policy::active(&self.principal).map_err(|_|anyhow!("PRINCIPAL_REVOKED"))?;
                        self.check_network_grants()?;
            let event=final_result.ok_or_else(||anyhow!("WORKER_NO_RESULT"))?;
            if event["success"].as_bool()==Some(false){
                // Only fixed machine codes, never worker-controlled raw text.
                let code=event["code"].as_str().filter(|s|s.len()<=64&&s.bytes().all(|b|b.is_ascii_uppercase()||b==b'_')).unwrap_or("WORKER_FAILED");
                return Err(anyhow!(WorkerFailure { code: code.to_owned(), retry_after_seconds: event["retry_after_seconds"].as_u64() }));
            }
            if !status.success(){return Err(anyhow!("WORKER_EXIT_FAILED"))}
            Ok(event)
        }.await;
        let result = result
            .map_err(|e| egress_blocked(e, broker.policy_denials()))
            .map_err(|e| egress_limit(e, broker.limit_reason()));
        broker.revoke();
        let group = process.pid;
        process.terminate();
        let reaped = matches!(
            tokio::time::timeout(Duration::from_secs(5), process.child.wait()).await,
            Ok(Ok(_))
        );
        stderr_task.abort();
        let _ = stderr_task.await;
        broker.shutdown().await;
        let group_gone = tokio::time::timeout(Duration::from_secs(5), async {
            while !process_group_gone(group) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .is_ok();
        // Operator log only: where the fixed cost of a worker run goes
        // (prepare -> dependency grants -> spawn -> first event/byte -> exit).
        let ms = |d: Option<Duration>| d.map_or(-1, |d| d.as_millis() as i64);
        tracing::info!(
            "worker timing op={operation} ok={} prepare_ms={} deps_ms={} spawn_ms={} first_event_ms={} first_byte_ms={} output_closed_ms={} total_ms={}",
            result.is_ok(),
            prepared_at.as_millis(),
            dependencies_at.as_millis(),
            spawned_at.as_millis(),
            ms(first_event_at),
            ms(first_byte_at),
            ms(output_closed_at),
            started.elapsed().as_millis()
        );
        if !reaped || !group_gone {
            return Err(anyhow!("WORKER_TERMINATION_UNCONFIRMED"));
        }
        result
    }
}
/// A worker failure after the broker refused a connection by policy is
/// OmniGet's block. Only codes the refusal itself can produce are relabelled
/// (the engine saw a 403 or an opaque failure); a platform statement stays.
/// One worker `progress` line as a queue update. `percent: null` (or a
/// missing number) means the worker does not know the total: the update is
/// indeterminate instead of a 0% that the queue would read as real (D-04).
fn progress_from_event(event: &Value) -> ProgressUpdate {
    let phase = match event["phase"].as_str() {
        Some("downloading") => "downloading",
        Some("merging") => "merging",
        Some("converting") => "converting",
        _ => "running",
    };
    let percent = event["percent"].as_f64().filter(|p| p.is_finite());
    let update = ProgressUpdate {
        percent: percent.unwrap_or(0.0),
        downloaded_bytes: event["downloaded_bytes"].as_u64(),
        total_bytes: event["total_bytes"].as_u64(),
        speed_bps: event["speed_bps"].as_f64(),
        eta_seconds: event["eta_seconds"].as_u64(),
        phase: Some(phase.into()),
        ..Default::default()
    };
    if percent.is_some() {
        update
    } else {
        update.indeterminate()
    }
}

fn egress_blocked(error: anyhow::Error, policy_denials: u64) -> anyhow::Error {
    let ambiguous = error.downcast_ref::<WorkerFailure>().is_some_and(|f| {
        matches!(
            f.code.as_str(),
            "ACCESS_DENIED" | "ENGINE_FAILED" | "WORKER_FAILED" | "BROKEN_SOURCE"
        )
    });
    if policy_denials > 0 && ambiguous {
        anyhow!(WorkerFailure {
            code: "EGRESS_BLOCKED".into(),
            retry_after_seconds: None
        })
    } else {
        error
    }
}
/// A failure after a broker budget cut is that budget: the engine only saw its
/// connections close (fragment failure, opaque error). Cancellation and
/// authorization changes are independent and stay as they are.
fn egress_limit(error: anyhow::Error, reason: Option<LimitReason>) -> anyhow::Error {
    let Some(reason) = reason else { return error };
    let independent = matches!(
        error.to_string().as_str(),
        "CANCELLED" | "PRINCIPAL_REVOKED" | "PRINCIPAL_INACTIVE" | "NETWORK_GRANTS_CHANGED"
    );
    if independent {
        error
    } else {
        anyhow!(WorkerFailure {
            code: reason.code().into(),
            retry_after_seconds: None
        })
    }
}
fn default_helper() -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|_| anyhow!("WORKER_UNAVAILABLE"))?;
    Ok(exe
        .parent()
        .ok_or_else(|| anyhow!("WORKER_UNAVAILABLE"))?
        .join(if cfg!(windows) {
            "omniget-worker.exe"
        } else {
            "omniget-worker"
        }))
}
/// Discovery is conservative; execution still validates dependencies and sandbox.
pub fn isolation_available() -> bool {
    cfg!(target_os = "macos") && default_helper().is_ok_and(|path| path.is_file())
}
struct RuntimeDir(PathBuf);
impl RuntimeDir {
    fn create() -> Result<Self> {
        let root = omniget_core::core::paths::app_data_dir()
            .ok_or_else(|| anyhow!("WORKER_RUNTIME_UNAVAILABLE"))?
            .join("worker-runs");
        std::fs::create_dir_all(&root).map_err(|_| anyhow!("WORKER_RUNTIME_UNAVAILABLE"))?;
        let path = root.join(uuid::Uuid::new_v4().to_string());
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .map_err(|_| anyhow!("WORKER_RUNTIME_UNAVAILABLE"))?;
        Ok(Self(
            path.canonicalize()
                .map_err(|_| anyhow!("WORKER_RUNTIME_UNAVAILABLE"))?,
        ))
    }
}
impl Drop for RuntimeDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// Per-job info JSON directory (0700), removed with the job. Stale ones left
/// by a crash are swept when the next one is created.
struct InfoJsonDir(PathBuf);
impl InfoJsonDir {
    fn create() -> Result<Self> {
        let root = omniget_core::core::paths::app_data_dir()
            .ok_or_else(|| anyhow!("WORKER_RUNTIME_UNAVAILABLE"))?
            .join("cache")
            .join("worker-info-json");
        std::fs::create_dir_all(&root).map_err(|_| anyhow!("WORKER_RUNTIME_UNAVAILABLE"))?;
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let stale = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(24 * 3600));
                if stale {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        let path = root.join(uuid::Uuid::new_v4().to_string());
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .map_err(|_| anyhow!("WORKER_RUNTIME_UNAVAILABLE"))?;
        Ok(Self(
            path.canonicalize()
                .map_err(|_| anyhow!("WORKER_RUNTIME_UNAVAILABLE"))?,
        ))
    }
}
impl Drop for InfoJsonDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// The yt-dlp onedir (`<dir>/yt-dlp_macos` + `<dir>/_internal/`) that holds
/// this executable, if it is one. Only the trusted managed path reaches here.
fn onedir_root(exe: &std::path::Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    (exe.is_file() && dir.join("_internal").is_dir() && dir.parent().is_some())
        .then(|| dir.to_path_buf())
}
fn process_group_gone(pid: i32) -> bool {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        // ESRCH is 3 on the supported macOS backend. Permission errors are not an ack.
        unsafe { kill(-pid, 0) == -1 && std::io::Error::last_os_error().raw_os_error() == Some(3) }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}
struct ProcessGuard {
    child: Child,
    pid: i32,
}
impl ProcessGuard {
    fn new(child: Child) -> Result<Self> {
        let pid = child.id().ok_or_else(|| anyhow!("WORKER_PID"))? as i32;
        Ok(Self { child, pid })
    }
    fn terminate(&mut self) {
        #[cfg(unix)]
        {
            unsafe extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            if self.pid > 0 {
                unsafe {
                    kill(-self.pid, 9);
                }
            }
        }
        let _ = self.child.start_kill();
        self.pid = 0;
    }
}
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}
async fn bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>> {
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|_| anyhow!("WORKER_OUTPUT_READ"))?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(anyhow!("WORKER_TRUNCATED_OUTPUT"))
            };
        }
        let end = available
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(available.len());
        if line.len() + end > 1024 * 1024 {
            return Err(anyhow!("WORKER_LINE_LIMIT"));
        }
        line.extend_from_slice(&available[..end]);
        let complete = available[end - 1] == b'\n';
        reader.consume(end);
        if complete {
            return Ok(Some(std::mem::take(line)));
        }
    }
}
tokio::task_local! {
    /// Queue-scoped cancellation keeps inspect teardown awaitable through the trait.
    pub static INSPECT_CANCEL: CancellationToken;
}

#[async_trait]
impl PlatformDownloader for WorkerDownloader {
    fn name(&self) -> &str {
        "mcp_worker"
    }
    fn can_handle(&self, url: &str) -> bool {
        url == self.url
    }
    async fn get_media_info(&self, url: &str) -> Result<MediaInfo> {
        if url != self.url {
            return Err(anyhow!("WORKER_URL_MISMATCH"));
        }
        let response = self
            .dispatch(
                json!({"operation":"inspect"}),
                INSPECT_CANCEL.try_with(Clone::clone).unwrap_or_default(),
                None,
            )
            .await?;
        let platform = response["platform"]
            .as_str()
            .filter(|p| p.len() <= 64 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            .ok_or_else(|| anyhow!("WORKER_PLATFORM"))?;
        let info: MediaInfo = serde_json::from_value(response["media_info"].clone())
            .map_err(|_| anyhow!("WORKER_MEDIA_INFO"))?;
        *self
            .selected_media
            .lock()
            .map_err(|_| anyhow!("WORKER_STATE"))? = Some((platform.to_owned(), info.clone()));
        // UI/cache metadata cannot trigger an unmediated desktop image fetch.
        // Preserve source URLs only inside this adapter for the next worker.
        let mut display_info = info;
        display_info.thumbnail_url = None;
        for quality in &mut display_info.available_qualities {
            quality.url.clear();
        }
        Ok(display_info)
    }
    async fn download(
        &self,
        _info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> Result<DownloadResult> {
        if opts.filename_template.is_some()
            || opts
                .custom_ytdlp_args
                .as_ref()
                .is_some_and(|a| !a.is_empty())
            || opts.torrent_files.is_some()
        {
            return Err(anyhow!("WORKER_OPTIONS_UNSUPPORTED"));
        }
        let selected = self
            .selected_media
            .lock()
            .map_err(|_| anyhow!("WORKER_STATE"))?
            .clone();
        let quality = opts
            .quality
            .as_deref()
            .filter(|q| !matches!(*q, "best" | "highest"))
            .map(|q| q.trim_end_matches('p').parse::<u32>())
            .transpose()
            .map_err(|_| anyhow!("WORKER_QUALITY"))?;
        let mode = match opts.download_mode.as_deref() {
            None | Some("video") | Some("video_audio") => "video",
            Some("audio") => "audio",
            _ => return Err(anyhow!("WORKER_MODE_UNSUPPORTED")),
        };
        let mut request = json!({"operation":"download","output_dir":opts.output_dir,"quality":quality,"download_mode":mode,"audio_format":opts.audio_format,"format_id":opts.format_id,"referer":opts.referer,"page_url":opts.page_url,"extra_headers":opts.extra_headers,"user_agent":opts.user_agent,"subtitles":opts.download_subtitles,"include_auto_subtitles":opts.include_auto_subtitles,"concurrent_fragments":opts.concurrent_fragments.clamp(1,super::download_intents::WORKER_MAX_FRAGMENTS),"ytdlp_path":opts.ytdlp_path});
        if let Some((platform, info)) = selected {
            request["platform"] = json!(platform);
            request["media_info"] = serde_json::to_value(info)?;
        }
        let response = self
            .dispatch(request, opts.cancel_token.clone(), Some(progress))
            .await?;
        serde_json::from_value(response["result"].clone()).map_err(|_| anyhow!("WORKER_RESULT"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operator_diagnostic_is_bounded_and_redacted() {
        let raw = format!("ERROR: [Reddit] x: Postprocessing: Conversion failed! https://v.redd.it/a.mp4?sig=SYNTHETIC_SECRET&token=SYNTHETIC_SECRET\n\u{1b}[31m{}", "x".repeat(5000));
        let d =
            operator_diagnostic(&serde_json::json!({"type":"diagnostic","detail":raw})).unwrap();
        assert!(d.contains("Conversion failed"));
        assert!(!d.contains("SYNTHETIC_SECRET"), "{d}");
        assert!(!d.chars().any(|c| c.is_control()));
        assert!(d.chars().count() <= 2100);
        assert!(operator_diagnostic(&serde_json::json!({"type":"diagnostic"})).is_none());
    }
    #[test]
    fn worker_progress_with_null_percent_is_indeterminate() {
        let p = progress_from_event(
            &json!({"type":"progress","percent":null,"downloaded_bytes":2_400_000,"total_bytes":null,"phase":"downloading"}),
        );
        assert!(p.indeterminate);
        assert_eq!(p.percent_value(), None);
        assert_eq!(p.downloaded_bytes, Some(2_400_000));
        let p = progress_from_event(
            &json!({"type":"progress","percent":42.5,"downloaded_bytes":10,"total_bytes":20,"phase":"downloading"}),
        );
        assert_eq!(p.percent_value(), Some(42.5));
    }
    fn label(code: &str) -> String {
        WorkerFailure {
            code: code.into(),
            retry_after_seconds: None,
        }
        .to_string()
    }
    #[test]
    fn failure_labels_are_accurate_and_diagnosable() {
        let format = label("FORMAT_UNAVAILABLE");
        assert!(!format.to_ascii_lowercase().contains("not found"));
        assert_eq!(
            crate::core::root_cause::machine_diagnose(&format).code,
            "FORMAT_UNAVAILABLE"
        );
        let blocked = label("BLOCKED_BY_PLATFORM");
        assert!(!blocked.contains("429"));
        let d = crate::core::root_cause::machine_diagnose(&blocked);
        assert_eq!(
            (d.code, d.retryable),
            ("BLOCKED_BY_PLATFORM", "after_cooldown")
        );
        let broken = label("BROKEN_SOURCE");
        assert!(crate::core::root_cause::machine_diagnose(&broken).is_retryable());
        assert!(crate::core::queue::is_retryable_error_message(&broken));
        assert!(!crate::core::queue::is_retryable_error_message(&format));
        // Explicit retry only after the class cooldown; never automatic
        // (external queue items have max_retries=0).
        assert!(crate::core::queue::is_retryable_error_message(&blocked));
        assert_eq!(
            crate::core::root_cause::machine_diagnose(&blocked).cooldown_seconds(),
            900
        );
    }
    #[test]
    fn d15_policy_denial_is_not_a_platform_403() {
        let failure = |code: &str| {
            anyhow!(WorkerFailure {
                code: code.into(),
                retry_after_seconds: None
            })
        };
        let blocked = egress_blocked(failure("ACCESS_DENIED"), 1).to_string();
        assert!(blocked.starts_with("EGRESS_BLOCKED"));
        assert!(!blocked.contains("403"));
        assert_ne!(blocked, label("ACCESS_DENIED"));
        let d = crate::core::root_cause::machine_diagnose(&blocked);
        assert_eq!(
            (d.code, d.retryable),
            ("EGRESS_BLOCKED", "after_user_action")
        );
        assert!(!crate::core::queue::is_retryable_error_message(&blocked));
        // Without a denial, or with a platform statement, nothing changes.
        assert_eq!(
            egress_blocked(failure("ACCESS_DENIED"), 0).to_string(),
            label("ACCESS_DENIED")
        );
        assert_eq!(
            egress_blocked(failure("BLOCKED_BY_PLATFORM"), 2).to_string(),
            label("BLOCKED_BY_PLATFORM")
        );
        assert_eq!(
            egress_blocked(anyhow!("CANCELLED"), 2).to_string(),
            "CANCELLED"
        );
    }
    #[test]
    fn egress_budget_cut_is_its_own_limit_code() {
        let failure = |code: &str| {
            anyhow!(WorkerFailure {
                code: code.into(),
                retry_after_seconds: None
            })
        };
        for (reason, code) in [
            (LimitReason::Bytes, "LIMIT_BYTES"),
            (LimitReason::Lifetime, "LIMIT_TIME"),
            (LimitReason::Connections, "LIMIT_CONNECTIONS"),
        ] {
            for engine in [
                failure("BROKEN_SOURCE"),
                failure("ENGINE_FAILED"),
                anyhow!("EGRESS_REVOKED_OR_BUDGET"),
                anyhow!("WORKER_NO_RESULT"),
            ] {
                let text = egress_limit(engine, Some(reason)).to_string();
                assert!(text.starts_with(code), "{text}");
                let d = crate::core::root_cause::machine_diagnose(&text);
                assert!(
                    !matches!(
                        d.code,
                        "BROKEN_SOURCE" | "NOT_FOUND" | "RATE_LIMITED" | "BLOCKED_BY_PLATFORM"
                    ),
                    "{text} -> {}",
                    d.code
                );
            }
        }
        assert_eq!(
            egress_limit(anyhow!("CANCELLED"), Some(LimitReason::Bytes)).to_string(),
            "CANCELLED"
        );
        assert_eq!(
            egress_limit(failure("BROKEN_SOURCE"), None).to_string(),
            label("BROKEN_SOURCE")
        );
    }
    #[test]
    fn d05_server_error_label_is_transient() {
        let s = label("SERVER_ERROR");
        assert!(!s.to_ascii_lowercase().contains("not found"));
        let d = crate::core::root_cause::machine_diagnose(&s);
        assert_eq!((d.code, d.retryable), ("SERVER_ERROR", "bounded"));
        assert!(crate::core::queue::external_retryable(&s));
        assert!(crate::core::queue::is_retryable_error_message(&s));
    }
    #[tokio::test]
    async fn line_limit_before_unbounded_allocation() {
        let input = vec![b'x'; 1024 * 1024 + 1];
        let mut r = BufReader::new(input.as_slice());
        assert!(bounded_line(&mut r, &mut Vec::new())
            .await
            .unwrap_err()
            .to_string()
            .contains("LINE_LIMIT"));
    }
    #[tokio::test]
    async fn lines_require_complete_frame() {
        let mut r = BufReader::new(&b"{}\npartial"[..]);
        assert_eq!(
            bounded_line(&mut r, &mut Vec::new()).await.unwrap(),
            Some(b"{}\n".to_vec())
        );
        assert!(bounded_line(&mut r, &mut Vec::new()).await.is_err());
    }
    #[test]
    fn onedir_root_only_for_a_real_onedir_layout() {
        let root =
            std::env::temp_dir().join(format!("omniget-onedir-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(root.join("od").join("_internal")).unwrap();
        std::fs::create_dir_all(root.join("onefile")).unwrap();
        std::fs::write(root.join("od").join("yt-dlp_macos"), b"x").unwrap();
        std::fs::write(root.join("onefile").join("yt-dlp"), b"x").unwrap();
        assert_eq!(
            onedir_root(&root.join("od").join("yt-dlp_macos")),
            Some(root.join("od"))
        );
        assert_eq!(onedir_root(&root.join("onefile").join("yt-dlp")), None);
        assert_eq!(onedir_root(&root.join("od").join("missing")), None);
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Real official onedir under the worker's Seatbelt composition (Mach-O
    /// dependency grants + onedir subpath). Opt-in: external 50 MB build.
    /// `OMNIGET_TEST_ONEDIR_YTDLP=/abs/yt-dlp_onedir/yt-dlp_macos`.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn onedir_ytdlp_runs_under_worker_sandbox_only_with_dir_grant() {
        let Some(exe) = std::env::var_os("OMNIGET_TEST_ONEDIR_YTDLP") else {
            eprintln!("skipped: OMNIGET_TEST_ONEDIR_YTDLP unset");
            return;
        };
        let exe = PathBuf::from(exe).canonicalize().unwrap();
        let dir = onedir_root(&exe).expect("onedir layout");
        let files = omniget_core::core::engine_files::dependency_files(&[exe.clone()])
            .await
            .unwrap();
        let runtime = std::env::temp_dir().join(format!(
            "omniget-onedir-run-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&runtime).unwrap();
        let runtime = runtime.canonicalize().unwrap();
        let broker = Broker::start(Policy::default(), CancellationToken::new())
            .await
            .unwrap();
        let run = |reads: Vec<PathBuf>| {
            let mut c = broker
                .confined_command_with_files(&exe, &reads, &[runtime.clone()])
                .unwrap();
            c.current_dir(&runtime)
                .env("TMPDIR", &runtime)
                .args(["--ignore-config", "--version"]);
            async move {
                tokio::time::timeout(Duration::from_secs(120), c.output())
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        let without = run(files.clone()).await;
        let mut granted = files.clone();
        granted.push(dir.clone());
        let started = std::time::Instant::now();
        let with = run(granted.clone()).await;
        let first = started.elapsed();
        let started = std::time::Instant::now();
        let again = run(granted).await;
        let second = started.elapsed();
        broker.shutdown().await;
        let _ = std::fs::remove_dir_all(&runtime);
        eprintln!(
            "onedir in worker sandbox: without_dir exit={:?}; with_dir exit={:?} {:?} then {:?} version={}",
            without.status.code(), with.status.code(), first, second,
            String::from_utf8_lossy(&again.stdout).trim()
        );
        assert!(
            !without.status.success(),
            "file grants alone must not be enough (test would prove nothing)"
        );
        assert!(
            with.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&with.stderr)
        );
        assert!(again.status.success() && !again.stdout.is_empty());
    }
    #[tokio::test]
    async fn polling_tick_preserves_partial_frame() {
        let (mut w, r) = tokio::io::duplex(128);
        let mut r = BufReader::new(r);
        let mut pending = Vec::new();
        w.write_all(b"{\"type\":").await.unwrap();
        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            bounded_line(&mut r, &mut pending)
        )
        .await
        .is_err());
        w.write_all(b"\"result\"}\n").await.unwrap();
        assert_eq!(
            bounded_line(&mut r, &mut pending).await.unwrap(),
            Some(b"{\"type\":\"result\"}\n".to_vec())
        );
    }
}
