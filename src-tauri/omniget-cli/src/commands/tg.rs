//! `omniget tg`: the /telegram superpower from the terminal. Reads, local
//! downloads, auto-save rules and exports only: nothing here sends, forwards
//! or deletes anything on the account.
//!
//! Two backends, same command names as the app's `telegram_call`:
//! - the running app, through the local bridge (`POST /v1/telegram/<command>`),
//!   so the app's own session does the work and its downloads show up there;
//! - this process (`--local`, or when the app is not running): the engine in
//!   `omniget_core::core::telegram` with the same data folder and session,
//!   progress drawn here, and the process waits for its downloads.
//!
//! Two processes on one session at once is avoided on purpose: when the app
//! answers, the CLI never opens the session itself unless told to.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use omniget_core::core::telegram::{commands, host, net};

use super::agent::Bridge;

#[derive(clap::Subcommand)]
pub enum TgCommand {
    /// Session, connection route and which backend answers
    Status,
    /// List chats (optionally only those whose title contains QUERY)
    Chats {
        query: Option<String>,
        #[arg(long, default_value = "100")]
        limit: usize,
    },
    /// List media of a chat (id, title, t.me/c/<id> or t.me/<user>/<msg> link)
    Ls {
        chat: String,
        #[arg(
            short = 't',
            long = "type",
            help = "video, photo, document, audio, voice, gif"
        )]
        media_type: Option<String>,
        #[arg(short, long, help = "Search captions and file names")]
        query: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
        #[arg(long, default_value = "0")]
        offset: i32,
        #[command(flatten)]
        chat_opts: ChatOpts,
    },
    /// Download the media of t.me links (message, album, range t.me/x/10-20)
    Dl {
        #[arg(required = true)]
        links: Vec<String>,
        #[arg(short, long, help = "Output folder")]
        output: Option<String>,
        #[arg(
            long,
            help = "Name template: {chat} {chat_id} {msg_id} {date} {time} {file_name} {stem} {ext} {type} {caption} {album}"
        )]
        template: Option<String>,
    },
    /// Download a whole chat with filters (like `tdl dl`)
    DlChat {
        chat: String,
        #[command(flatten)]
        filter: FilterArgs,
        #[arg(short, long, help = "Output folder")]
        output: Option<String>,
        #[arg(long, help = "Name template (see `omniget tg dl --help`)")]
        template: Option<String>,
        #[arg(
            long,
            help = "Download again files that already exist with the same size"
        )]
        no_skip_same: bool,
        #[arg(
            long,
            help = "Read the history inside a takeout session (fewer flood waits)"
        )]
        takeout: bool,
        #[arg(long, help = "Only show what would be downloaded")]
        dry_run: bool,
        #[command(flatten)]
        chat_opts: ChatOpts,
    },
    /// Auto-save rules: save new media of a chat as it arrives
    Watch {
        #[command(subcommand)]
        command: WatchCommand,
    },
    /// Export a chat as Telegram Desktop's result.json (or HTML)
    Export {
        chat: String,
        #[arg(long, default_value = "json", value_parser = ["json", "html"])]
        format: String,
        #[arg(long, help = "Also download the media into photos/ and files/")]
        media: bool,
        #[arg(long, help = "At most N messages (0 = all)", default_value = "0")]
        limit: u64,
        #[arg(short, long, help = "Output folder")]
        output: Option<String>,
        #[command(flatten)]
        chat_opts: ChatOpts,
    },
}

// clap parses this once per run; boxing the big variant buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(clap::Subcommand)]
pub enum WatchCommand {
    /// The rules and what they saved
    List,
    /// Add a rule for a chat
    Add {
        chat: String,
        #[arg(
            short = 't',
            long = "type",
            value_delimiter = ',',
            help = "video,photo,document,audio,gif,round_video,voice (none = all)"
        )]
        media_types: Vec<String>,
        #[arg(long, help = "Minimum size (e.g. 10MB)")]
        min_size: Option<String>,
        #[arg(long, help = "Maximum size (e.g. 2GB)")]
        max_size: Option<String>,
        #[arg(long, help = "Only captions or file names containing this")]
        keyword: Option<String>,
        #[arg(long = "ext", value_delimiter = ',', help = "Only these extensions")]
        ext_include: Vec<String>,
        #[arg(long = "exclude-ext", value_delimiter = ',')]
        ext_exclude: Vec<String>,
        #[arg(short, long, help = "Output folder")]
        output: Option<String>,
        #[arg(long)]
        template: Option<String>,
        #[arg(
            long,
            default_value = "0",
            help = "Also save the N newest matching media now"
        )]
        backfill: u64,
        #[arg(long, help = "Create the rule switched off")]
        disabled: bool,
        #[command(flatten)]
        chat_opts: ChatOpts,
    },
    /// Remove a rule
    Rm { id: String },
    /// Run one rule now (all enabled rules without ID)
    Run { id: Option<String> },
}

#[derive(clap::Args, Clone, Default)]
pub struct ChatOpts {
    #[arg(
        long = "chat-type",
        value_parser = ["private", "group", "channel"],
        help = "Chat type, when CHAT is a numeric id that is not in the chat list"
    )]
    chat_type: Option<String>,
}

#[derive(clap::Args, Clone, Default)]
pub struct FilterArgs {
    #[arg(
        short = 't',
        long = "type",
        value_delimiter = ',',
        help = "video,photo,document,audio,gif,round_video,voice"
    )]
    media_types: Vec<String>,
    #[arg(long, help = "From this date (YYYY-MM-DD or unix seconds)")]
    from: Option<String>,
    #[arg(long, help = "Until this date, inclusive (YYYY-MM-DD or unix seconds)")]
    to: Option<String>,
    #[arg(long, help = "Minimum size (e.g. 10MB)")]
    min_size: Option<String>,
    #[arg(long, help = "Maximum size (e.g. 2GB)")]
    max_size: Option<String>,
    #[arg(long, help = "Only captions or file names containing this")]
    keyword: Option<String>,
    #[arg(
        long = "ext",
        value_delimiter = ',',
        help = "Only these extensions (mp4,mkv)"
    )]
    ext_include: Vec<String>,
    #[arg(long = "exclude-ext", value_delimiter = ',')]
    ext_exclude: Vec<String>,
    #[arg(long, help = "From this message id")]
    from_id: Option<i64>,
    #[arg(long, help = "Until this message id")]
    to_id: Option<i64>,
    #[arg(long, help = "At most N files")]
    limit: Option<u64>,
}

// ---- backend ----------------------------------------------------------------

const NO_ROUTE: &str = "the running OmniGet app does not expose Telegram to the CLI yet \
(no /v1/telegram route). Close the app, or pass --local to use the session from this process \
while the app runs (not recommended)";

enum Backend {
    App(Bridge),
    Local(Arc<Tracker>),
}

impl Backend {
    async fn call(&self, command: &str, args: Value) -> anyhow::Result<Value> {
        match self {
            Backend::App(bridge) => bridge
                .post(&format!("/v1/telegram/{command}"), args)
                .await
                .map_err(|e| {
                    if e.to_string().starts_with("404") {
                        anyhow!(NO_ROUTE)
                    } else {
                        e
                    }
                }),
            Backend::Local(_) => commands::dispatch(command.to_string(), args)
                .await
                .map_err(|e| anyhow!(e)),
        }
    }

    async fn net_status(&self) -> anyhow::Result<Value> {
        match self {
            Backend::App(_) => self.call("telegram_net_status", json!({})).await,
            Backend::Local(_) => Ok(serde_json::to_value(net::manager::get().status())?),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Backend::App(_) => "app",
            Backend::Local(_) => "local",
        }
    }
}

fn app_settings() -> Value {
    omniget_core::core::paths::app_data_dir()
        .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|v| v["app_settings"].clone())
        .unwrap_or(Value::Null)
}

/// Something listens on the bridge port of `settings.json`: the app runs.
async fn app_alive(settings: &Value) -> bool {
    let port = std::env::var("OMNIGET_BRIDGE_URL")
        .ok()
        .and_then(|u| {
            u.rsplit(':')
                .next()
                .and_then(|p| p.trim_end_matches('/').parse::<u64>().ok())
        })
        .or_else(|| settings["bridge"]["port"].as_u64())
        .filter(|p| *p > 0 && *p <= u16::MAX as u64);
    let Some(port) = port else { return false };
    tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(("127.0.0.1", port as u16)),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

async fn backend(local: bool, json_out: bool) -> anyhow::Result<Backend> {
    let settings = app_settings();
    if settings["telegram"]["enabled"].as_bool() == Some(false) {
        bail!("Telegram is switched off in OmniGet's settings");
    }
    if !local && app_alive(&settings).await {
        return Ok(Backend::App(Bridge::open()?));
    }
    let tracker = Arc::new(Tracker::new(json_out));
    host::install(Arc::new(CliHost::new(settings, tracker.clone())));
    Ok(Backend::Local(tracker))
}

// ---- local host ------------------------------------------------------------

struct CliHost {
    prefs: Mutex<Value>,
    default_out: PathBuf,
    tracker: Arc<Tracker>,
}

impl CliHost {
    fn new(settings: Value, tracker: Arc<Tracker>) -> Self {
        let tg = &settings["telegram"];
        let mut prefs = match tg["prefs"].clone() {
            Value::Object(m) => m,
            _ => Default::default(),
        };
        prefs.entry("concurrent_downloads").or_insert_with(|| {
            tg["concurrent_downloads"]
                .as_u64()
                .map(Value::from)
                .unwrap_or(json!(3))
        });
        prefs.entry("fix_file_extensions").or_insert_with(|| {
            tg["fix_file_extensions"]
                .as_bool()
                .map(Value::from)
                .unwrap_or(json!(true))
        });
        let default_out = settings["download"]["default_output_dir"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(crate::reporter::default_output_dir);
        Self {
            prefs: Mutex::new(Value::Object(prefs)),
            default_out,
            tracker,
        }
    }
}

impl host::TelegramHost for CliHost {
    fn emit(&self, event: &str, payload: Value) {
        self.tracker.send(event, payload);
    }

    fn data_dir(&self) -> PathBuf {
        let dir = omniget_core::core::paths::app_data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("telegram");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn settings(&self) -> Value {
        self.prefs.lock().map(|p| p.clone()).unwrap_or(Value::Null)
    }

    /// The app owns `settings.json`: a change made here lasts this process.
    fn save_settings(&self, settings: Value) {
        if let Ok(mut p) = self.prefs.lock() {
            *p = settings;
        }
    }

    fn default_output_dir(&self) -> PathBuf {
        self.default_out.clone()
    }

    fn toast(&self, _kind: &str, message: &str) {
        self.tracker.note(message);
    }
}

// ---- progress of local work --------------------------------------------------

/// Follows the engine's events: one bar per download (single file or batch),
/// export and chat scan, and which of them are still open.
struct Tracker {
    tx: mpsc::UnboundedSender<(String, Value)>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<(String, Value)>>>,
    sent: AtomicU64,
    state: Mutex<TrackState>,
    json_out: bool,
    multi: MultiProgress,
}

#[derive(Default)]
struct TrackState {
    processed: u64,
    open: HashSet<String>,
    finished: HashSet<String>,
    bars: HashMap<String, ProgressBar>,
    failed: u32,
    last_event: Option<Instant>,
}

const BAR_STYLE: &str = "{spinner:.cyan} [{elapsed_precise}] [{bar:30.cyan/blue}] {pos:>3}% {msg}";
const SPIN_STYLE: &str = "{spinner:.cyan} {msg}";

impl Tracker {
    fn new(json_out: bool) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
            sent: AtomicU64::new(0),
            state: Mutex::new(TrackState::default()),
            json_out,
            multi: MultiProgress::new(),
        }
    }

    fn send(&self, event: &str, payload: Value) {
        self.sent.fetch_add(1, Ordering::SeqCst);
        let _ = self.tx.send((event.to_string(), payload));
    }

    fn note(&self, line: &str) {
        if self.json_out {
            eprintln!("{line}");
        } else {
            let _ = self.multi.println(line);
        }
    }

    /// Starts the task that reads the events (once).
    fn start(self: &Arc<Self>) {
        let Some(mut rx) = self.rx.lock().ok().and_then(|mut r| r.take()) else {
            return;
        };
        let me = self.clone();
        tokio::spawn(async move {
            while let Some((event, payload)) = rx.recv().await {
                me.handle(&event, &payload);
                if let Ok(mut s) = me.state.lock() {
                    s.processed += 1;
                }
            }
        });
    }

    /// A download or export the command said it started: wait for it even
    /// when its first event is still on the way.
    fn expect(&self, key: String) {
        if let Ok(mut s) = self.state.lock() {
            if !s.finished.contains(&key) {
                s.open.insert(key);
            }
        }
    }

    fn bar(
        &self,
        s: &mut TrackState,
        key: &str,
        spinner: bool,
        title: &str,
    ) -> Option<ProgressBar> {
        if self.json_out {
            return None;
        }
        let pb = s.bars.entry(key.to_string()).or_insert_with(|| {
            let pb = self.multi.add(if spinner {
                ProgressBar::new_spinner()
            } else {
                ProgressBar::new(100)
            });
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(if spinner { SPIN_STYLE } else { BAR_STYLE })
                    .unwrap(),
            );
            pb.enable_steady_tick(Duration::from_millis(120));
            pb.set_message(title.to_string());
            pb
        });
        Some(pb.clone())
    }

    fn handle(&self, event: &str, p: &Value) {
        let Ok(mut s) = self.state.lock() else { return };
        s.last_event = Some(Instant::now());
        let kind = p["kind"].as_str().unwrap_or("");
        let top = matches!(kind, "single" | "batch");
        let title = p["title"].as_str().unwrap_or("").to_string();
        match event {
            "telegram-download-queued" if top => {
                let key = format!("dl:{}", p["id"]);
                if !s.finished.contains(&key) {
                    s.open.insert(key.clone());
                    self.bar(&mut s, &key, false, &title);
                }
            }
            "telegram-download-progress" if top => {
                let key = format!("dl:{}", p["id"]);
                if s.finished.contains(&key) {
                    return;
                }
                let pct = p["percent"].as_f64().unwrap_or(0.0).clamp(0.0, 100.0);
                let speed = p["speed_bytes_per_sec"].as_f64().unwrap_or(0.0);
                if let Some(pb) = self.bar(&mut s, &key, false, &title) {
                    pb.set_position(pct as u64);
                    if speed > 0.0 {
                        pb.set_message(format!("{title}  {}/s", bytes(speed as u64)));
                    }
                } else if self.json_out {
                    println!(
                        "{}",
                        json!({ "type": "progress", "event": event, "data": p })
                    );
                }
            }
            "telegram-download-complete" if top => {
                let key = format!("dl:{}", p["id"]);
                s.open.remove(&key);
                s.finished.insert(key.clone());
                let ok = p["success"].as_bool().unwrap_or(false);
                if !ok {
                    s.failed += 1;
                }
                let line = if ok {
                    let files = p["file_count"].as_u64().unwrap_or(0);
                    let size = p["file_size_bytes"].as_u64().unwrap_or(0);
                    let mut l = format!("done: {title}");
                    if files > 1 {
                        l.push_str(&format!(" ({files} files"));
                        if size > 0 {
                            l.push_str(&format!(", {}", bytes(size)));
                        }
                        l.push(')');
                    }
                    if let Some(path) = p["file_path"].as_str() {
                        l.push_str(&format!(" -> {path}"));
                    }
                    l
                } else {
                    format!(
                        "failed: {title}: {}",
                        p["error"].as_str().unwrap_or("unknown error")
                    )
                };
                if let Some(pb) = s.bars.remove(&key) {
                    pb.finish_and_clear();
                }
                if self.json_out {
                    println!("{}", json!({ "type": "complete", "data": p }));
                } else {
                    let _ = self.multi.println(line);
                }
            }
            "telegram-batch-file-status" => {
                if p["status"].as_str() == Some("error") && !self.json_out {
                    let _ = self.multi.println(format!(
                        "  message {}: {}",
                        p["message_id"],
                        p["error"].as_str().unwrap_or("error")
                    ));
                }
            }
            "telegram-export-progress" => {
                let key = format!("export:{}", p["export_id"]);
                if s.finished.contains(&key) {
                    return;
                }
                s.open.insert(key.clone());
                let msg = format!(
                    "export: {} messages, media {}/{}",
                    p["messages"].as_u64().unwrap_or(0),
                    p["media_done"].as_u64().unwrap_or(0),
                    p["media_total"].as_u64().unwrap_or(0)
                );
                if let Some(pb) = self.bar(&mut s, &key, true, &msg) {
                    pb.set_message(msg);
                }
            }
            "telegram-export-complete" => {
                let key = format!("export:{}", p["export_id"]);
                s.open.remove(&key);
                s.finished.insert(key.clone());
                if let Some(pb) = s.bars.remove(&key) {
                    pb.finish_and_clear();
                }
                let ok = p["success"].as_bool().unwrap_or(false);
                if !ok {
                    s.failed += 1;
                }
                if self.json_out {
                    println!("{}", json!({ "type": "export", "data": p }));
                } else if ok {
                    let _ = self.multi.println(format!(
                        "exported {} messages -> {}",
                        p["message_count"].as_u64().unwrap_or(0),
                        p["path"].as_str().unwrap_or("")
                    ));
                } else {
                    let _ = self.multi.println(format!(
                        "export failed: {}",
                        p["error"].as_str().unwrap_or("unknown error")
                    ));
                }
            }
            "telegram-chat-scan" => {
                let key = format!("scan:{}", p["chat_id"]);
                let msg = format!(
                    "reading history: {} scanned, {} match",
                    p["scanned"].as_u64().unwrap_or(0),
                    p["matched"].as_u64().unwrap_or(0)
                );
                if p["done"].as_bool() == Some(true) {
                    if let Some(pb) = s.bars.remove(&key) {
                        pb.finish_and_clear();
                    }
                } else if let Some(pb) = self.bar(&mut s, &key, true, &msg) {
                    pb.set_message(msg);
                }
            }
            "telegram-watch-run" => {
                let line = match p["error"].as_str() {
                    Some(e) => format!("rule {}: {e}", p["id"].as_str().unwrap_or("")),
                    None => format!(
                        "rule {}: {} new, {} queued",
                        p["id"].as_str().unwrap_or(""),
                        p["found"].as_u64().unwrap_or(0),
                        p["queued"].as_u64().unwrap_or(0)
                    ),
                };
                self.note(&line);
            }
            _ => {}
        }
    }

    /// Scan bars are only useful while the command reads; clear them.
    fn clear_scans(&self) {
        if let Ok(mut s) = self.state.lock() {
            let keys: Vec<String> = s
                .bars
                .keys()
                .filter(|k| k.starts_with("scan:"))
                .cloned()
                .collect();
            for k in keys {
                if let Some(pb) = s.bars.remove(&k) {
                    pb.finish_and_clear();
                }
            }
        }
    }

    /// Waits until every download and export this process started is over.
    /// Work that starts on its own task (auto-save runs) gets `grace` to show
    /// its first event. Returns how many failed.
    async fn wait(&self, grace: Duration) -> u32 {
        self.clear_scans();
        let t0 = Instant::now();
        loop {
            let sent = self.sent.load(Ordering::SeqCst);
            let (idle, failed) = match self.state.lock() {
                Ok(s) => (s.processed >= sent && s.open.is_empty(), s.failed),
                Err(_) => (true, 0),
            };
            if idle && t0.elapsed() >= grace {
                return failed;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

// ---- helpers -----------------------------------------------------------------

fn bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

/// `10MB`, `1.5 GB`, `700k`, `123` (bytes). Decimal units, like the app.
fn parse_size(raw: &str) -> anyhow::Result<u64> {
    let t = raw.trim().to_ascii_lowercase().replace(' ', "");
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().with_context(|| format!("bad size: {raw}"))?;
    let mul = match unit.trim_end_matches('b').trim_end_matches('i') {
        "" => 1.0,
        "k" => 1e3,
        "m" => 1e6,
        "g" => 1e9,
        "t" => 1e12,
        _ => bail!("bad size unit in {raw}"),
    };
    Ok((n * mul) as u64)
}

/// `YYYY-MM-DD` (local day; `end` = its last second) or unix seconds.
fn parse_date(raw: &str, end: bool) -> anyhow::Result<i64> {
    use chrono::{Local, NaiveDate, TimeZone};
    if let Ok(n) = raw.trim().parse::<i64>() {
        return Ok(n);
    }
    let d = NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .with_context(|| format!("bad date {raw}: use YYYY-MM-DD or unix seconds"))?;
    let t = if end {
        d.and_hms_opt(23, 59, 59)
    } else {
        d.and_hms_opt(0, 0, 0)
    }
    .ok_or_else(|| anyhow!("bad date {raw}"))?;
    Local
        .from_local_datetime(&t)
        .earliest()
        .map(|t| t.timestamp())
        .ok_or_else(|| anyhow!("bad date {raw}"))
}

fn date(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn exts(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

fn filter_json(f: &FilterArgs) -> anyhow::Result<Value> {
    let mut m = serde_json::Map::new();
    if !f.media_types.is_empty() {
        m.insert("mediaTypes".into(), json!(f.media_types));
    }
    if let Some(v) = &f.from {
        m.insert("fromDate".into(), json!(parse_date(v, false)?));
    }
    if let Some(v) = &f.to {
        m.insert("toDate".into(), json!(parse_date(v, true)?));
    }
    if let Some(v) = &f.min_size {
        m.insert("minSize".into(), json!(parse_size(v)?));
    }
    if let Some(v) = &f.max_size {
        m.insert("maxSize".into(), json!(parse_size(v)?));
    }
    if let Some(v) = &f.keyword {
        m.insert("keyword".into(), json!(v));
    }
    if !f.ext_include.is_empty() {
        m.insert("extInclude".into(), json!(exts(&f.ext_include)));
    }
    if !f.ext_exclude.is_empty() {
        m.insert("extExclude".into(), json!(exts(&f.ext_exclude)));
    }
    if let Some(v) = f.from_id {
        m.insert("fromMessageId".into(), json!(v));
    }
    if let Some(v) = f.to_id {
        m.insert("toMessageId".into(), json!(v));
    }
    if let Some(v) = f.limit {
        m.insert("limit".into(), json!(v));
    }
    Ok(Value::Object(m))
}

fn abs(dir: Option<String>) -> Option<String> {
    dir.map(|d| {
        let p = PathBuf::from(&d);
        if p.is_absolute() {
            d
        } else {
            std::env::current_dir()
                .map(|c| c.join(p).to_string_lossy().to_string())
                .unwrap_or(d)
        }
    })
}

// ---- chat resolution --------------------------------------------------------

struct Chat {
    id: i64,
    chat_type: String,
    title: String,
}

impl Chat {
    fn args(&self) -> Value {
        json!({ "chatId": self.id, "chatType": self.chat_type, "chatTitle": self.title })
    }
}

fn chat_of(v: &Value) -> Option<Chat> {
    Some(Chat {
        id: v["id"].as_i64()?,
        chat_type: v["chat_type"].as_str()?.to_string(),
        title: v["title"].as_str().unwrap_or("").to_string(),
    })
}

/// A chat from its id, a t.me link, or its title (exact, then a unique
/// part of it). Listing the chats also teaches the engine their access
/// hashes, which the media calls need.
async fn resolve_chat(b: &Backend, spec: &str, opts: &ChatOpts) -> anyhow::Result<Chat> {
    let spec = spec.trim();
    let lower = spec.to_ascii_lowercase();
    let link = lower
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    let is_link = ["t.me/", "telegram.me/", "telegram.dog/"]
        .iter()
        .any(|p| link.starts_with(p));

    // t.me/<user>/<msg>: the engine resolves the message and names its chat.
    if is_link && !link.contains("/c/") {
        let r = b
            .call("telegram_resolve_links", json!({ "text": spec }))
            .await?;
        if let Some(it) = r["items"].as_array().and_then(|a| a.first()) {
            return Ok(Chat {
                id: it["chat_id"].as_i64().unwrap_or(0),
                chat_type: it["chat_type"].as_str().unwrap_or("channel").to_string(),
                title: it["chat_title"].as_str().unwrap_or("").to_string(),
            });
        }
        bail!("could not resolve {spec}: give a link to one message (t.me/<name>/<id>), the chat id or its title");
    }
    if let Some(name) = spec.strip_prefix('@') {
        bail!("{spec}: usernames are not resolved yet; give a link to one of its messages (t.me/{name}/<id>), the chat id or its title");
    }

    let wanted_id: Option<i64> = if is_link {
        link.split("/c/")
            .nth(1)
            .and_then(|r| r.split('/').next())
            .and_then(|n| n.parse().ok())
    } else {
        spec.parse().ok()
    };

    let list = b.call("telegram_list_chats", json!({})).await?;
    let chats: Vec<Chat> = list
        .as_array()
        .map(|a| a.iter().filter_map(chat_of).collect())
        .unwrap_or_default();

    if let Some(id) = wanted_id {
        // Bot API style ids (-100123…, -123…) name the same chat.
        let bare = {
            let s = id.to_string();
            match s.strip_prefix("-100") {
                Some(rest) if !rest.is_empty() => rest.parse().unwrap_or(id),
                _ => id.abs(),
            }
        };
        if let Some(c) = chats.into_iter().find(|c| c.id == id || c.id == bare) {
            return Ok(c);
        }
        if let Some(t) = &opts.chat_type {
            return Ok(Chat {
                id: bare,
                chat_type: t.clone(),
                title: spec.to_string(),
            });
        }
        bail!("chat {spec} is not in your chat list; pass --chat-type to use the id anyway");
    }

    let exact: Vec<&Chat> = chats
        .iter()
        .filter(|c| c.title.to_lowercase() == spec.to_lowercase())
        .collect();
    let pick = if exact.len() == 1 {
        exact
    } else {
        let needle = spec.to_lowercase();
        chats
            .iter()
            .filter(|c| c.title.to_lowercase().contains(&needle))
            .collect()
    };
    match pick.as_slice() {
        [one] => Ok(Chat {
            id: one.id,
            chat_type: one.chat_type.clone(),
            title: one.title.clone(),
        }),
        [] => bail!("no chat matches {spec:?}; `omniget tg chats` lists them"),
        many => {
            let names: Vec<String> = many
                .iter()
                .take(10)
                .map(|c| format!("  {}  {}  {}", c.id, c.chat_type, c.title))
                .collect();
            bail!(
                "{} chats match {spec:?}; use the id:\n{}",
                many.len(),
                names.join("\n")
            )
        }
    }
}

// ---- printing -------------------------------------------------------------------

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn print_media(items: &Value) {
    let Some(items) = items.as_array() else {
        return;
    };
    for it in items {
        println!(
            "{:>8}  {}  {:>10}  {:<9} {}",
            it["message_id"].as_i64().unwrap_or(0),
            date(it["date"].as_i64().unwrap_or(0)),
            bytes(it["file_size"].as_u64().unwrap_or(0)),
            it["media_type"].as_str().unwrap_or(""),
            it["file_name"].as_str().unwrap_or("")
        );
    }
    if items.is_empty() {
        println!("(nothing)");
    }
}

fn print_rules(rules: &Value) {
    let Some(rules) = rules.as_array() else {
        return;
    };
    if rules.is_empty() {
        println!("(no rules)");
    }
    for r in rules {
        let types = r["mediaTypes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "all".into());
        println!(
            "{}  {}  {}  [{}]  saved {}{}",
            r["id"].as_str().unwrap_or(""),
            if r["enabled"].as_bool().unwrap_or(false) {
                "on "
            } else {
                "off"
            },
            r["chatTitle"].as_str().unwrap_or(""),
            types,
            r["savedCount"].as_u64().unwrap_or(0),
            r["lastError"]
                .as_str()
                .map(|e| format!("  error: {e}"))
                .unwrap_or_default()
        );
    }
}

// ---- entry --------------------------------------------------------------------

pub async fn execute(cmd: TgCommand, local: bool, json_out: bool) -> anyhow::Result<()> {
    let b = backend(local, json_out).await?;
    if let Backend::Local(t) = &b {
        t.start();
    }
    // Downloads started by this call (their batch/export ids) and a grace
    // for work that starts on its own task.
    let mut started: Vec<String> = Vec::new();
    let mut grace = Duration::ZERO;
    let mut queued_msg = false;

    match cmd {
        TgCommand::Status => {
            let session = match b.call("telegram_check_session", json!({})).await {
                Ok(v) => json!({ "logged_in": true, "phone": v }),
                Err(e) if e.to_string().contains("not_authenticated") => {
                    json!({ "logged_in": false })
                }
                Err(e) => json!({ "logged_in": false, "error": e.to_string() }),
            };
            let net = b.net_status().await.unwrap_or(Value::Null);
            let out = json!({ "backend": b.name(), "session": session, "network": net });
            if json_out {
                print_json(&out);
            } else {
                println!(
                    "backend  {}",
                    match b {
                        Backend::App(_) => "the running OmniGet app",
                        Backend::Local(_) => "this process (the app is not running)",
                    }
                );
                match session["logged_in"].as_bool() {
                    Some(true) => println!(
                        "account  logged in ({})",
                        session["phone"].as_str().unwrap_or("")
                    ),
                    _ => println!(
                        "account  not logged in{}; log in from OmniGet → Telegram",
                        session["error"]
                            .as_str()
                            .map(|e| format!(" ({e})"))
                            .unwrap_or_default()
                    ),
                }
                let active = &net["active"];
                if active.is_object() {
                    println!(
                        "route    {} ({}){}",
                        active["route"]["name"].as_str().unwrap_or(""),
                        active["route"]["kind"].as_str().unwrap_or(""),
                        active["health"]["rtt_ms"]
                            .as_u64()
                            .map(|ms| format!(", {ms} ms"))
                            .unwrap_or_default()
                    );
                } else {
                    println!(
                        "route    none yet (phase {})",
                        net["phase"].as_str().unwrap_or("unknown")
                    );
                }
                if let Some(routes) = net["routes"].as_array() {
                    let ok = routes
                        .iter()
                        .filter(|r| r["health"]["ok"].as_bool() == Some(true))
                        .count();
                    println!("routes   {} known, {} working", routes.len(), ok);
                }
            }
        }

        TgCommand::Chats { query, limit } => {
            let list = b.call("telegram_list_chats", json!({})).await?;
            let needle = query.as_deref().map(str::to_lowercase);
            let chats: Vec<Value> = list
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|c| match &needle {
                    Some(n) => c["title"].as_str().unwrap_or("").to_lowercase().contains(n),
                    None => true,
                })
                .take(limit)
                .collect();
            if json_out {
                print_json(&Value::Array(chats));
            } else {
                for c in &chats {
                    println!(
                        "{:>14}  {:<8} {:>5}  {}",
                        c["id"].as_i64().unwrap_or(0),
                        c["chat_type"].as_str().unwrap_or(""),
                        match c["unread_count"].as_i64().unwrap_or(0) {
                            0 => String::new(),
                            n => n.to_string(),
                        },
                        c["title"].as_str().unwrap_or("")
                    );
                }
                if chats.is_empty() {
                    println!("(no chats)");
                }
            }
        }

        TgCommand::Ls {
            chat,
            media_type,
            query,
            limit,
            offset,
            chat_opts,
        } => {
            let c = resolve_chat(&b, &chat, &chat_opts).await?;
            let mut args = c.args();
            args["limit"] = json!(limit);
            if let Some(t) = media_type {
                args["mediaType"] = json!(t);
            }
            let items = match query {
                Some(q) => {
                    args["query"] = json!(q);
                    b.call("telegram_search_media", args).await?
                }
                None => {
                    args["offset"] = json!(offset);
                    b.call("telegram_list_media", args).await?
                }
            };
            if json_out {
                print_json(&items);
            } else {
                print_media(&items);
            }
        }

        TgCommand::Dl {
            links,
            output,
            template,
        } => {
            let mut args = json!({ "text": links.join("\n") });
            if let Some(o) = abs(output) {
                args["outputDir"] = json!(o);
            }
            if let Some(t) = template {
                args["template"] = json!(t);
            }
            let r = b.call("telegram_download_links", args).await?;
            if let Some(id) = r["batch_id"].as_u64() {
                started.push(format!("dl:{id}"));
            }
            if json_out {
                print_json(&r);
            } else {
                println!(
                    "{} file(s) -> {}",
                    r["count"].as_u64().unwrap_or(0),
                    r["output_dir"].as_str().unwrap_or("")
                );
                if let Some(errs) = r["errors"].as_array() {
                    for e in errs {
                        eprintln!(
                            "  {}: {}",
                            e["url"].as_str().unwrap_or(""),
                            e["error"].as_str().unwrap_or("")
                        );
                    }
                }
            }
            queued_msg = true;
        }

        TgCommand::DlChat {
            chat,
            filter,
            output,
            template,
            no_skip_same,
            takeout,
            dry_run,
            chat_opts,
        } => {
            let c = resolve_chat(&b, &chat, &chat_opts).await?;
            let mut args = c.args();
            args["filter"] = filter_json(&filter)?;
            args["skipSame"] = json!(!no_skip_same);
            args["takeout"] = json!(takeout);
            if let Some(o) = abs(output) {
                args["outputDir"] = json!(o);
            }
            if let Some(t) = template {
                args["template"] = json!(t);
            }
            if dry_run {
                let r = b.call("telegram_preview_chat_download", args).await?;
                if let Backend::Local(t) = &b {
                    t.clear_scans();
                }
                if json_out {
                    print_json(&r);
                } else {
                    println!(
                        "{} file(s), {} (read {} messages) -> {}",
                        r["count"].as_u64().unwrap_or(0),
                        bytes(r["total_bytes"].as_u64().unwrap_or(0)),
                        r["scanned"].as_u64().unwrap_or(0),
                        r["output_dir"].as_str().unwrap_or("")
                    );
                    for it in r["sample"].as_array().cloned().unwrap_or_default() {
                        println!(
                            "  {:>8}  {:>10}  {}{}",
                            it["message_id"].as_i64().unwrap_or(0),
                            bytes(it["file_size"].as_u64().unwrap_or(0)),
                            it["path"].as_str().unwrap_or(""),
                            if it["exists"].as_bool() == Some(true) {
                                "  (exists)"
                            } else {
                                ""
                            }
                        );
                    }
                }
            } else {
                let r = b.call("telegram_download_chat", args).await?;
                if let Some(id) = r["batch_id"].as_u64() {
                    started.push(format!("dl:{id}"));
                    queued_msg = true;
                }
                if json_out {
                    print_json(&r);
                } else {
                    println!(
                        "{} file(s) from {}",
                        r["count"].as_u64().unwrap_or(0),
                        c.title
                    );
                }
            }
        }

        TgCommand::Watch { command } => match command {
            WatchCommand::List => {
                let r = b.call("telegram_watch_list", json!({})).await?;
                if json_out {
                    print_json(&r);
                } else {
                    print_rules(&r);
                }
            }
            WatchCommand::Add {
                chat,
                media_types,
                min_size,
                max_size,
                keyword,
                ext_include,
                ext_exclude,
                output,
                template,
                backfill,
                disabled,
                chat_opts,
            } => {
                let c = resolve_chat(&b, &chat, &chat_opts).await?;
                let mut rule = c.args();
                rule["enabled"] = json!(!disabled);
                rule["mediaTypes"] = json!(media_types);
                rule["extInclude"] = json!(exts(&ext_include));
                rule["extExclude"] = json!(exts(&ext_exclude));
                if let Some(v) = min_size {
                    rule["minSize"] = json!(parse_size(&v)?);
                }
                if let Some(v) = max_size {
                    rule["maxSize"] = json!(parse_size(&v)?);
                }
                if let Some(v) = keyword {
                    rule["keyword"] = json!(v);
                }
                if let Some(v) = abs(output) {
                    rule["outputDir"] = json!(v);
                }
                if let Some(v) = template {
                    rule["template"] = json!(v);
                }
                let r = b
                    .call(
                        "telegram_watch_add",
                        json!({ "rule": rule, "backfill": backfill }),
                    )
                    .await?;
                if json_out {
                    print_json(&r);
                } else {
                    print_rules(&json!([r]));
                }
                if backfill > 0 {
                    grace = Duration::from_secs(3);
                    queued_msg = true;
                }
            }
            WatchCommand::Rm { id } => {
                let r = b.call("telegram_watch_remove", json!({ "id": id })).await?;
                if json_out {
                    print_json(&r);
                } else if r["ok"].as_bool() == Some(true) {
                    println!("removed {id}");
                } else {
                    bail!("no rule {id}");
                }
            }
            WatchCommand::Run { id } => {
                let args = match &id {
                    Some(id) => json!({ "id": id }),
                    None => json!({}),
                };
                let r = b.call("telegram_watch_run", args).await?;
                if json_out {
                    print_json(&r);
                } else {
                    let runs = match &r {
                        Value::Array(a) => a.clone(),
                        other => vec![other.clone()],
                    };
                    for run in runs {
                        println!(
                            "rule {}: {} new, {} queued{}",
                            run["id"].as_str().unwrap_or(""),
                            run["found"].as_u64().unwrap_or(0),
                            run["queued"].as_u64().unwrap_or(0),
                            if run["busy"].as_bool() == Some(true) {
                                " (already running)"
                            } else {
                                ""
                            }
                        );
                    }
                }
                grace = Duration::from_secs(3);
                queued_msg = true;
            }
        },

        TgCommand::Export {
            chat,
            format,
            media,
            limit,
            output,
            chat_opts,
        } => {
            let c = resolve_chat(&b, &chat, &chat_opts).await?;
            let mut args = c.args();
            args["format"] = json!(format);
            args["includeMedia"] = json!(media);
            args["limit"] = json!(limit);
            if let Some(o) = abs(output) {
                args["outputDir"] = json!(o);
            }
            let r = b.call("telegram_export_chat", args).await?;
            if let Some(id) = r["export_id"].as_u64() {
                started.push(format!("export:{id}"));
            }
            if json_out {
                print_json(&r);
            } else {
                println!("exporting {}", c.title);
            }
            queued_msg = true;
        }
    }

    match &b {
        Backend::App(_) => {
            if queued_msg && !json_out {
                eprintln!("Running in the OmniGet app; follow it in its Downloads.");
            }
        }
        Backend::Local(t) => {
            for key in started {
                t.expect(key);
            }
            let failed = tokio::select! {
                n = t.wait(grace) => n,
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\nInterrupted. Run the same command again to resume the partial files.");
                    std::process::exit(130);
                }
            };
            if failed > 0 {
                bail!("{failed} download(s) or export(s) failed");
            }
        }
    }
    Ok(())
}
