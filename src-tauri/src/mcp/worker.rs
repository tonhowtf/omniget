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
            "FORMAT_UNAVAILABLE" => "FORMAT_UNAVAILABLE: requested format is not available within the requested options",
            "BROKEN_SOURCE" => "BROKEN_SOURCE: the source stopped serving part of the media (fragment failure)",
            "BLOCKED_BY_PLATFORM" => "BLOCKED_BY_PLATFORM: the platform blocked access from this network",
            "BOT_CHALLENGE" => "BOT_CHALLENGE: the platform requires a bot challenge",
            "EXTRACTOR_FAILURE" => "EXTRACTOR_FAILURE: unable to extract this page; the site may have changed",
            "SERVER_ERROR" => "SERVER_ERROR: the server had a temporary error (HTTP 5xx); retry later",
            "EGRESS_FAILED" => "EGRESS_FAILED: network egress failed (proxy, TLS or DNS) before the platform answered; retry later",
            "EGRESS_BLOCKED" => "EGRESS_BLOCKED: blocked by OmniGet network policy, not by the platform; grant this local address to the connection in OmniGet or use a public URL",
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
            let ytdlp=omniget_core::core::ytdlp::managed_ytdlp_path().filter(|p|p.is_file());
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
        let policy = Policy {
            local_allowances: self.allowed_sockets.clone(),
            upstream: omniget_core::core::egress::Upstream::from_settings(),
            ..Policy::default()
        };
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
            drop(input);
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
