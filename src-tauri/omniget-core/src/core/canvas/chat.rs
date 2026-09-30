//! Chat face of a terminal node (Maestri Chat, estudo 95 I / 02 §7).
//!
//! The transcript belongs to the app, not to the agent: it survives
//! compaction, restarts and swapping the CLI. One JSON line per entry in
//! `<canvas>/<ws>/chat/<nodeId>.jsonl`, seven fixed threads (`0..=6`) with
//! stable colours. Attachments are copied into
//! `<canvas>/<ws>/chat/attachments/<nodeId>/` so they outlive the source file.
//!
//! Direction user → agent: [`envelope`] builds the fixed header the agent
//! sees in its PTY. Direction agent → user: `omniget canvas say` becomes
//! [`say`]; `omniget canvas recall` becomes [`recall`].

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::is_safe_id;
use super::store::CanvasStore;

/// The seven threads, in slot order (Maestri: blue, purple, pink, red,
/// orange, yellow, green).
pub const THREADS: [(&str, &str); 7] = [
    ("blue", "#3B82F6"),
    ("purple", "#8B5CF6"),
    ("pink", "#EC4899"),
    ("red", "#EF4444"),
    ("orange", "#F97316"),
    ("yellow", "#EAB308"),
    ("green", "#22C55E"),
];

/// Live window the UI gets first, and the page size going back.
pub const WINDOW: usize = 80;
pub const PAGE: usize = 60;
/// Default `recall` depth (turns).
pub const RECALL_TURNS: usize = 10;
/// Largest attachment (Maestri: 25 MB).
pub const MAX_ATTACHMENT: u64 = 25 * 1024 * 1024;
/// A user message above this goes to a file and the envelope carries the path.
pub const INLINE_MAX_BYTES: usize = 4 * 1024;
pub const INLINE_MAX_LINES: usize = 40;
/// Largest message text kept in one entry.
pub const MAX_TEXT: usize = 512 * 1024;

pub fn thread_key(i: u8) -> &'static str {
    THREADS.get(i as usize).map(|t| t.0).unwrap_or("blue")
}

pub fn thread_color(i: u8) -> &'static str {
    THREADS.get(i as usize).map(|t| t.1).unwrap_or("#3B82F6")
}

/// `"green"`, `"6"` → 6.
pub fn parse_thread(s: &str) -> Option<u8> {
    let s = s.trim().to_ascii_lowercase();
    if let Ok(n) = s.parse::<u8>() {
        return (n < THREADS.len() as u8).then_some(n);
    }
    THREADS.iter().position(|t| t.0 == s).map(|i| i as u8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Author {
    User,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Message,
    Progress,
    Final,
    Divider,
    /// The user interrupted the turn (Esc); the agent's reply never came.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    /// Absolute path of the kept copy.
    pub path: String,
    #[serde(default)]
    pub is_image: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEntry {
    pub id: String,
    pub thread: u8,
    pub role: Author,
    pub kind: EntryKind,
    /// RFC 3339.
    pub ts: String,
    /// Text without the delivery envelope (markdown).
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfo {
    pub index: u8,
    pub key: &'static str,
    pub color: &'static str,
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ts: Option<String>,
}

/// Per-node chat state, next to the transcript (`<nodeId>.state.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatState {
    /// Thread of the last message delivered to the agent: the thread of the
    /// open turn, where its `say` lands whatever the user selects meanwhile.
    #[serde(default)]
    pub thread: u8,
    /// Thread the user has selected (the UI and `canvas_chat_thread`); the
    /// next message goes there. `None` = `thread`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<u8>,
    /// A user message is waiting for the agent's reply.
    #[serde(default)]
    pub turn_open: bool,
    /// When the open turn started (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_started: Option<String>,
    /// The last turn ended by an interrupt (Esc) or without a reply.
    #[serde(default)]
    pub turn_interrupted: bool,
    /// Agent replies not read yet, per thread.
    #[serde(default)]
    pub unread: [u32; 7],
}

impl ChatState {
    /// The thread the user sees and writes to.
    pub fn active(&self) -> u8 {
        self.selected
            .filter(|t| (*t as usize) < THREADS.len())
            .unwrap_or(self.thread)
    }

    pub fn unread_total(&self) -> u32 {
        self.unread.iter().sum()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub node_id: String,
    pub thread: u8,
    pub entries: Vec<ChatEntry>,
    pub has_older: bool,
    pub threads: Vec<ThreadInfo>,
    pub active_thread: u8,
    pub turn_open: bool,
    /// Thread of the open (or last) turn.
    pub turn_thread: u8,
    pub turn_interrupted: bool,
    /// Unread agent replies per thread, and their sum (before this read
    /// marked `thread` as read, when it did).
    pub unread: [u32; 7],
    pub unread_count: u32,
}

// ---------------------------------------------------------------------------
// Paths

pub fn chat_dir(store: &CanvasStore, ws: &str) -> PathBuf {
    store.workspace_dir(ws).join("chat")
}

fn check_ids(ws: &str, node: &str) -> Result<(), String> {
    if !is_safe_id(ws) || !is_safe_id(node) {
        return Err("CANVAS_INVALID: id".into());
    }
    Ok(())
}

pub fn transcript_path(store: &CanvasStore, ws: &str, node: &str) -> Result<PathBuf, String> {
    check_ids(ws, node)?;
    Ok(chat_dir(store, ws).join(format!("{node}.jsonl")))
}

fn state_path(store: &CanvasStore, ws: &str, node: &str) -> Result<PathBuf, String> {
    check_ids(ws, node)?;
    Ok(chat_dir(store, ws).join(format!("{node}.state.json")))
}

pub fn attachments_dir(store: &CanvasStore, ws: &str, node: &str) -> Result<PathBuf, String> {
    check_ids(ws, node)?;
    Ok(chat_dir(store, ws).join("attachments").join(node))
}

/// Where the composer keeps pasted screenshots (`<ws>/inbox/attachments`).
pub fn inbox_attachments_dir(store: &CanvasStore, ws: &str) -> Result<PathBuf, String> {
    if !is_safe_id(ws) {
        return Err("CANVAS_INVALID: id".into());
    }
    Ok(store.workspace_dir(ws).join("inbox").join("attachments"))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Transcript

/// Every entry of the node, oldest first. Broken lines are skipped.
pub fn read_all(store: &CanvasStore, ws: &str, node: &str) -> Result<Vec<ChatEntry>, String> {
    let path = transcript_path(store, ws, node)?;
    let Ok(f) = fs::File::open(&path) else {
        return Ok(Vec::new());
    };
    Ok(BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<ChatEntry>(&l).ok())
        .collect())
}

pub fn append(store: &CanvasStore, ws: &str, node: &str, entry: &ChatEntry) -> Result<(), String> {
    let path = transcript_path(store, ws, node)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    }
    let mut line = serde_json::to_string(entry).map_err(|e| format!("CANVAS_IO: {e}"))?;
    line.push('\n');
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("CANVAS_IO: {e}"))?;
    f.write_all(line.as_bytes())
        .map_err(|e| format!("CANVAS_IO: {e}"))
}

pub fn load_state(store: &CanvasStore, ws: &str, node: &str) -> ChatState {
    state_path(store, ws, node)
        .ok()
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_state(store: &CanvasStore, ws: &str, node: &str, st: &ChatState) -> Result<(), String> {
    let path = state_path(store, ws, node)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    }
    let body = serde_json::to_vec_pretty(st).map_err(|e| format!("CANVAS_IO: {e}"))?;
    fs::write(path, body).map_err(|e| format!("CANVAS_IO: {e}"))
}

pub fn threads_of(entries: &[ChatEntry]) -> Vec<ThreadInfo> {
    (0..THREADS.len() as u8)
        .map(|i| {
            let mine: Vec<&ChatEntry> = entries
                .iter()
                .filter(|e| e.thread == i && e.kind != EntryKind::Divider)
                .collect();
            ThreadInfo {
                index: i,
                key: thread_key(i),
                color: thread_color(i),
                count: mine.len(),
                last_ts: mine.last().map(|e| e.ts.clone()),
            }
        })
        .collect()
}

/// One page of a thread: the newest `limit` entries (default [`WINDOW`]),
/// or the `limit` (default [`PAGE`]) before the entry `before`.
pub fn history(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    thread: Option<u8>,
    before: Option<&str>,
    limit: Option<usize>,
) -> Result<HistoryPage, String> {
    history_opts(store, ws, node, thread, before, limit, false)
}

/// [`history`]; `peek` leaves the unread counts alone, otherwise reading a
/// thread marks it read.
pub fn history_opts(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    thread: Option<u8>,
    before: Option<&str>,
    limit: Option<usize>,
    peek: bool,
) -> Result<HistoryPage, String> {
    let all = read_all(store, ws, node)?;
    let mut st = load_state(store, ws, node);
    let thread = thread.unwrap_or(st.active()).min(THREADS.len() as u8 - 1);
    let unread = st.unread;
    if !peek && st.unread[thread as usize] > 0 {
        st.unread[thread as usize] = 0;
        save_state(store, ws, node, &st)?;
    }
    let mine: Vec<&ChatEntry> = all.iter().filter(|e| e.thread == thread).collect();
    let end = match before {
        Some(id) => mine.iter().position(|e| e.id == id).unwrap_or(mine.len()),
        None => mine.len(),
    };
    let limit = limit
        .unwrap_or(if before.is_some() { PAGE } else { WINDOW })
        .clamp(1, 500);
    let start = end.saturating_sub(limit);
    Ok(HistoryPage {
        node_id: node.to_string(),
        thread,
        entries: mine[start..end].iter().map(|e| (*e).clone()).collect(),
        has_older: start > 0,
        threads: threads_of(&all),
        active_thread: st.active(),
        turn_open: st.turn_open,
        turn_thread: st.thread,
        turn_interrupted: st.turn_interrupted,
        unread,
        unread_count: unread.iter().sum(),
    })
}

/// Selects the thread the next message goes to. Refused while a turn is
/// open on another thread (Maestri locks the threads during a turn); the
/// caller says whether the turn still counts (`turn_locked`).
pub fn select_thread(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    thread: u8,
    turn_locked: bool,
) -> Result<ChatState, String> {
    if thread as usize >= THREADS.len() {
        return Err(format!(
            "CANVAS_INVALID: thread {thread} (0-6: blue, purple, pink, red, orange, yellow, green)"
        ));
    }
    let mut st = load_state(store, ws, node);
    if turn_locked && thread != st.thread {
        return Err(format!(
            "CHAT_TURN_ACTIVE: the agent is answering on thread {} ({}); interrupt it or wait",
            st.thread,
            thread_key(st.thread)
        ));
    }
    st.selected = Some(thread);
    save_state(store, ws, node, &st)?;
    Ok(st)
}

/// Marks one thread (or every thread) as read.
pub fn mark_read(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    thread: Option<u8>,
) -> Result<ChatState, String> {
    let mut st = load_state(store, ws, node);
    match thread {
        Some(t) if (t as usize) < THREADS.len() => st.unread[t as usize] = 0,
        Some(_) => return Err("CANVAS_INVALID: thread".into()),
        None => st.unread = [0; 7],
    }
    save_state(store, ws, node, &st)?;
    Ok(st)
}

/// Closes the open turn without a reply: `interrupted` (Esc) records an
/// entry on the turn's thread; a turn that just ended silently records one
/// too, so the transcript says why nothing came. `None` = no open turn.
pub fn end_turn(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    text: &str,
) -> Result<Option<ChatEntry>, String> {
    let mut st = load_state(store, ws, node);
    if !st.turn_open {
        return Ok(None);
    }
    let e = ChatEntry {
        id: new_id(),
        thread: st.thread,
        role: Author::User,
        kind: EntryKind::Interrupted,
        ts: now(),
        text: text.to_string(),
        attachments: Vec::new(),
        author_name: None,
        agent_type: None,
        client_message_id: None,
    };
    append(store, ws, node, &e)?;
    st.turn_open = false;
    st.turn_started = None;
    st.turn_interrupted = true;
    save_state(store, ws, node, &st)?;
    Ok(Some(e))
}

/// Appends a divider ("Clear"): `recall` stops there unless `--all`.
pub fn clear(store: &CanvasStore, ws: &str, node: &str, thread: u8) -> Result<ChatEntry, String> {
    let e = ChatEntry {
        id: new_id(),
        thread,
        role: Author::User,
        kind: EntryKind::Divider,
        ts: now(),
        text: String::new(),
        attachments: Vec::new(),
        author_name: None,
        agent_type: None,
        client_message_id: None,
    };
    append(store, ws, node, &e)?;
    Ok(e)
}

/// Rewrites the transcript with `keep` (atomically: temp file + rename).
fn rewrite(store: &CanvasStore, ws: &str, node: &str, keep: &[ChatEntry]) -> Result<(), String> {
    let path = transcript_path(store, ws, node)?;
    if keep.is_empty() {
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("CANVAS_IO: {e}"))?;
        }
        return Ok(());
    }
    let mut body = String::new();
    for e in keep {
        body.push_str(&serde_json::to_string(e).map_err(|e| format!("CANVAS_IO: {e}"))?);
        body.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, body).map_err(|e| format!("CANVAS_IO: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("CANVAS_IO: {e}"))
}

/// Deletes the kept copies of `gone` that no entry of `keep` still uses.
/// Only files inside the node's attachment folder are touched.
fn drop_attachments(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    gone: &[ChatEntry],
    keep: &[ChatEntry],
) -> usize {
    let Ok(dir) = attachments_dir(store, ws, node) else {
        return 0;
    };
    let used: std::collections::HashSet<&str> = keep
        .iter()
        .flat_map(|e| e.attachments.iter().map(|a| a.path.as_str()))
        .collect();
    let mut n = 0;
    for a in gone.iter().flat_map(|e| e.attachments.iter()) {
        let p = Path::new(&a.path);
        if used.contains(a.path.as_str()) || !p.starts_with(&dir) {
            continue;
        }
        if fs::remove_file(p).is_ok() {
            n += 1;
        }
    }
    n
}

/// Result of [`delete_entry`] / [`wipe`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub removed: Vec<String>,
    pub attachments_removed: usize,
    /// Entries left in the transcript (all threads).
    pub remaining: usize,
}

/// Removes one entry (Maestri: Delete Message) and its attachment files when
/// no other entry references them. `thread` narrows the lookup when given.
pub fn delete_entry(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    id: &str,
    thread: Option<u8>,
) -> Result<Removed, String> {
    let all = read_all(store, ws, node)?;
    let pos = all
        .iter()
        .position(|e| {
            (e.id == id || (id.len() >= 6 && e.id.starts_with(id)))
                && thread.is_none_or(|t| e.thread == t)
        })
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: chat entry {id}"))?;
    let mut keep = all;
    let gone = keep.remove(pos);
    rewrite(store, ws, node, &keep)?;
    let attachments_removed = drop_attachments(store, ws, node, std::slice::from_ref(&gone), &keep);
    Ok(Removed {
        removed: vec![gone.id],
        attachments_removed,
        remaining: keep.len(),
    })
}

/// Clear Chat History: drops the transcript of one thread, or of every
/// thread (plus the attachment folder and the turn state) when `thread` is
/// None. Only the app's record: the provider's own memory is not touched.
/// The caller refuses while a turn runs.
pub fn wipe(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    thread: Option<u8>,
) -> Result<Removed, String> {
    let all = read_all(store, ws, node)?;
    let (gone, keep): (Vec<ChatEntry>, Vec<ChatEntry>) = all
        .into_iter()
        .partition(|e| thread.is_none_or(|t| e.thread == t));
    rewrite(store, ws, node, &keep)?;
    let mut attachments_removed = drop_attachments(store, ws, node, &gone, &keep);
    let mut st = load_state(store, ws, node);
    st.turn_open = false;
    st.turn_started = None;
    match thread {
        Some(t) if (t as usize) < THREADS.len() => st.unread[t as usize] = 0,
        _ => st.unread = [0; 7],
    }
    if thread.is_none() {
        if let Ok(dir) = attachments_dir(store, ws, node) {
            if dir.is_dir() {
                attachments_removed += fs::read_dir(&dir).map(|r| r.count()).unwrap_or(0);
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }
    save_state(store, ws, node, &st)?;
    Ok(Removed {
        removed: gone.into_iter().map(|e| e.id).collect(),
        attachments_removed,
        remaining: keep.len(),
    })
}

// ---------------------------------------------------------------------------
// Attachments

fn content_type_of(path: &Path) -> (Option<&'static str>, bool) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => (Some("image/png"), true),
        "jpg" | "jpeg" => (Some("image/jpeg"), true),
        "gif" => (Some("image/gif"), true),
        "webp" => (Some("image/webp"), true),
        "heic" => (Some("image/heic"), true),
        "svg" => (Some("image/svg+xml"), false),
        "md" | "markdown" => (Some("text/markdown"), false),
        "txt" | "log" => (Some("text/plain"), false),
        "json" => (Some("application/json"), false),
        "pdf" => (Some("application/pdf"), false),
        "csv" => (Some("text/csv"), false),
        "html" | "htm" => (Some("text/html"), false),
        _ => (None, false),
    }
}

pub fn is_image_path(path: &Path) -> bool {
    content_type_of(path).1
}

fn clean_file_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let s: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() {
        "file".into()
    } else {
        s.chars().take(120).collect()
    }
}

/// Copies `src` into the node's attachment folder. Local regular files only,
/// at most [`MAX_ATTACHMENT`] bytes.
pub fn keep_attachment(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    src: &Path,
) -> Result<Attachment, String> {
    let meta =
        fs::metadata(src).map_err(|e| format!("CANVAS_NOT_FOUND: {}: {e}", src.display()))?;
    if !meta.is_file() {
        return Err(format!("CANVAS_INVALID: not a file: {}", src.display()));
    }
    if meta.len() > MAX_ATTACHMENT {
        return Err(format!(
            "CANVAS_INVALID: {} is larger than 25 MB",
            src.display()
        ));
    }
    let id = new_id();
    let filename = clean_file_name(&src.to_string_lossy());
    let dir = attachments_dir(store, ws, node)?;
    fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    let dest = dir.join(format!("{}-{filename}", &id[..8]));
    fs::copy(src, &dest).map_err(|e| format!("CANVAS_IO: {e}"))?;
    let (ct, is_image) = content_type_of(src);
    Ok(Attachment {
        id,
        filename,
        path: dest.to_string_lossy().into_owned(),
        is_image,
        content_type: ct.map(str::to_string),
        size: meta.len(),
    })
}

/// Saves bytes the composer pasted (screenshots) into `<ws>/inbox/attachments`.
pub fn save_inbox_attachment(
    store: &CanvasStore,
    ws: &str,
    name: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    if bytes.len() as u64 > MAX_ATTACHMENT {
        return Err("CANVAS_INVALID: attachment larger than 25 MB".into());
    }
    let dir = inbox_attachments_dir(store, ws)?;
    fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let short = &new_id()[..6];
    let path = dir.join(format!("{stamp}-{short}-{}", clean_file_name(name)));
    fs::write(&path, bytes).map_err(|e| format!("CANVAS_IO: {e}"))?;
    Ok(path)
}

/// Finds an attachment by id in the node's transcript.
pub fn attachment(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    id: &str,
) -> Result<Attachment, String> {
    read_all(store, ws, node)?
        .into_iter()
        .flat_map(|e| e.attachments)
        .find(|a| a.id == id || a.id.starts_with(id))
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: attachment {id}"))
}

// ---------------------------------------------------------------------------
// User → agent

/// A user message ready for the transcript and for delivery.
pub struct Outgoing {
    pub entry: ChatEntry,
    /// Exact text pasted into the agent's terminal.
    pub envelope: String,
}

/// The fixed header of OmniGet Chat (Maestri's, with our CLI). When the
/// thread changed since the last delivery, the agent is told to `recall`.
pub fn envelope(thread: u8, previous: Option<u8>, body: &str, attachments: &[String]) -> String {
    let mut s = String::from("---\nBelow is a message from the user through OmniGet Chat.\n");
    s.push_str("It was typed by the user in the chat panel of your canvas node and pasted here by OmniGet. Reply only via `omniget canvas say \"...\"` (see the omniget-canvas skill).\n");
    s.push_str(&format!("thread: {}\n", thread_key(thread)));
    if let Some(p) = previous.filter(|p| *p != thread) {
        s.push_str(&format!(
            "note: the thread changed from {} to {}; run `omniget canvas recall {}` before replying.\n",
            thread_key(p),
            thread_key(thread),
            thread_key(thread)
        ));
    }
    s.push_str("---\n");
    s.push_str(body.trim_end());
    if !attachments.is_empty() {
        s.push_str("\n\nAttached files:\n");
        for a in attachments {
            s.push_str(&format!("- {a}\n"));
        }
    }
    s
}

fn too_long(text: &str) -> bool {
    text.len() > INLINE_MAX_BYTES || text.lines().count() > INLINE_MAX_LINES
}

/// Records a user message on `thread` and builds the envelope. A long
/// message goes to `<ws>/inbox/<node>/chat-<id>.md` and the envelope points
/// at it (avoids the TUIs' paste collapse). `files` are paths the user
/// attached (kept as copies; the envelope lists the originals so image-aware
/// CLIs can read them).
pub fn user_message(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    thread: u8,
    text: &str,
    files: &[String],
    client_message_id: Option<String>,
) -> Result<Outgoing, String> {
    if thread as usize >= THREADS.len() {
        return Err("CANVAS_INVALID: thread".into());
    }
    if text.trim().is_empty() && files.is_empty() {
        return Err("CANVAS_INVALID: empty message".into());
    }
    if text.len() > MAX_TEXT {
        return Err("CANVAS_INVALID: message too long".into());
    }
    let mut attachments = Vec::new();
    for f in files {
        attachments.push(keep_attachment(store, ws, node, Path::new(f))?);
    }
    let entry = ChatEntry {
        id: new_id(),
        thread,
        role: Author::User,
        kind: EntryKind::Message,
        ts: now(),
        text: text.to_string(),
        attachments,
        author_name: None,
        agent_type: None,
        client_message_id,
    };
    let mut st = load_state(store, ws, node);
    let has_history = read_all(store, ws, node)?
        .iter()
        .any(|e| e.kind != EntryKind::Divider);
    let previous = has_history.then_some(st.thread);
    let body = if too_long(text) {
        let dir = store.inbox_dir(ws, node);
        fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
        let path = dir.join(format!("chat-{}.md", &entry.id[..8]));
        fs::write(&path, text).map_err(|e| format!("CANVAS_IO: {e}"))?;
        format!(
            "The message is long ({} lines); read it in full from:\n{}",
            text.lines().count(),
            path.display()
        )
    } else {
        text.to_string()
    };
    let paths: Vec<String> = entry.attachments.iter().map(|a| a.path.clone()).collect();
    let envelope = envelope(thread, previous, &body, &paths);
    append(store, ws, node, &entry)?;
    st.thread = thread;
    st.selected = Some(thread);
    st.turn_open = true;
    st.turn_started = Some(entry.ts.clone());
    st.turn_interrupted = false;
    save_state(store, ws, node, &st)?;
    Ok(Outgoing { entry, envelope })
}

// ---------------------------------------------------------------------------
// Agent → user

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SayArgs {
    #[serde(default)]
    pub text: Option<String>,
    /// Intermediate update; the turn goes on.
    #[serde(default)]
    pub progress: bool,
    /// `--file`: this file goes with the message as an attachment (like
    /// `--attach`), whatever its type.
    #[serde(default)]
    pub file: Option<String>,
    /// `--text-from`: the message text is this UTF-8 file's content.
    #[serde(default, alias = "text-from")]
    pub text_from: Option<String>,
    /// `--attach` (repeatable).
    #[serde(default)]
    pub attach: Vec<String>,
    /// Override the thread (default: the thread of the last delivery).
    #[serde(default)]
    pub thread: Option<String>,
    /// Working directory of the CLI, to resolve relative paths.
    #[serde(default)]
    pub cwd: Option<String>,
}

fn resolve(path: &str, cwd: Option<&str>) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    match cwd {
        Some(c) => Path::new(c).join(p),
        None => p.to_path_buf(),
    }
}

/// `omniget canvas say`: one bubble from the agent. Without `progress` the
/// bubble closes the turn.
pub fn say(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    args: SayArgs,
    author_name: Option<String>,
    agent_type: Option<String>,
) -> Result<ChatEntry, String> {
    let mut st = load_state(store, ws, node);
    let thread = match args.thread.as_deref() {
        Some(t) => {
            parse_thread(t).ok_or_else(|| format!("CANVAS_INVALID: unknown thread '{t}'"))?
        }
        None => st.thread,
    };
    let mut text = args.text.unwrap_or_default();
    let mut attachments = Vec::new();
    if let Some(f) = args.file.as_deref() {
        attachments.push(keep_attachment(
            store,
            ws,
            node,
            &resolve(f, args.cwd.as_deref()),
        )?);
    }
    if let Some(f) = args.text_from.as_deref() {
        let path = resolve(f, args.cwd.as_deref());
        let meta = fs::metadata(&path)
            .map_err(|e| format!("CANVAS_NOT_FOUND: {}: {e}", path.display()))?;
        if meta.len() as usize > MAX_TEXT {
            return Err("CANVAS_INVALID: file too large for a message; use --file".into());
        }
        let body = fs::read_to_string(&path)
            .map_err(|_| format!("CANVAS_INVALID: {} is not UTF-8 text", path.display()))?;
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&body);
    }
    for a in &args.attach {
        attachments.push(keep_attachment(
            store,
            ws,
            node,
            &resolve(a, args.cwd.as_deref()),
        )?);
    }
    if text.trim().is_empty() && attachments.is_empty() {
        return Err("CANVAS_INVALID: nothing to say".into());
    }
    if text.len() > MAX_TEXT {
        return Err("CANVAS_INVALID: message too long".into());
    }
    let entry = ChatEntry {
        id: new_id(),
        thread,
        role: Author::Agent,
        kind: if args.progress {
            EntryKind::Progress
        } else {
            EntryKind::Message
        },
        ts: now(),
        text,
        attachments,
        author_name,
        agent_type,
        client_message_id: None,
    };
    append(store, ws, node, &entry)?;
    if !args.progress {
        st.unread[thread as usize] = st.unread[thread as usize].saturating_add(1);
        if st.turn_open {
            st.turn_open = false;
            st.turn_started = None;
            st.turn_interrupted = false;
        }
    }
    save_state(store, ws, node, &st)?;
    Ok(entry)
}

// ---------------------------------------------------------------------------
// Recall

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallArgs {
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default)]
    pub turns: Option<usize>,
    /// `day` | `week` | `month`.
    #[serde(default)]
    pub last: Option<String>,
    /// RFC 3339 or `YYYY-MM-DD`.
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub all: bool,
}

fn parse_date(s: &str, end_of_day: bool) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s.trim()) {
        return Some(d.with_timezone(&chrono::Utc));
    }
    let d = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    let t = if end_of_day {
        d.and_hms_opt(23, 59, 59)?
    } else {
        d.and_hms_opt(0, 0, 0)?
    };
    Some(t.and_utc())
}

fn ts_of(e: &ChatEntry) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(&e.ts)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// Splits entries into turns: each user message opens one; agent entries
/// before the first user message form their own turn.
pub fn turns(entries: &[ChatEntry]) -> Vec<Vec<ChatEntry>> {
    let mut out: Vec<Vec<ChatEntry>> = Vec::new();
    for e in entries {
        if e.kind == EntryKind::Divider {
            continue;
        }
        if (e.role == Author::User && e.kind != EntryKind::Interrupted) || out.is_empty() {
            out.push(Vec::new());
        }
        if let Some(last) = out.last_mut() {
            last.push(e.clone());
        }
    }
    out
}

/// The selected turns of one thread (default: the active thread's last 10,
/// stopping at the last divider unless `all`).
pub fn recall(
    store: &CanvasStore,
    ws: &str,
    node: &str,
    args: &RecallArgs,
) -> Result<(u8, Vec<Vec<ChatEntry>>), String> {
    let st = load_state(store, ws, node);
    let thread = match args.thread.as_deref() {
        Some(t) => {
            parse_thread(t).ok_or_else(|| format!("CANVAS_INVALID: unknown thread '{t}'"))?
        }
        None => st.thread,
    };
    let mine: Vec<ChatEntry> = read_all(store, ws, node)?
        .into_iter()
        .filter(|e| e.thread == thread)
        .collect();
    let dated = args.last.is_some() || args.from.is_some() || args.to.is_some();
    let scoped: Vec<ChatEntry> = if args.all || dated {
        mine
    } else {
        let cut = mine
            .iter()
            .rposition(|e| e.kind == EntryKind::Divider)
            .map(|i| i + 1)
            .unwrap_or(0);
        mine[cut..].to_vec()
    };
    let now = chrono::Utc::now();
    let from = match args.last.as_deref() {
        Some("day") => Some(now - chrono::Duration::days(1)),
        Some("week") => Some(now - chrono::Duration::weeks(1)),
        Some("month") => Some(now - chrono::Duration::days(30)),
        Some(other) => return Err(format!("CANVAS_INVALID: --last {other} (day|week|month)")),
        None => args.from.as_deref().and_then(|s| parse_date(s, false)),
    };
    let to = args.to.as_deref().and_then(|s| parse_date(s, true));
    let filtered: Vec<ChatEntry> = scoped
        .into_iter()
        .filter(|e| {
            let Some(t) = ts_of(e) else { return !dated };
            from.is_none_or(|f| t >= f) && to.is_none_or(|x| t <= x)
        })
        .collect();
    let mut all_turns = turns(&filtered);
    if !(args.all || dated) {
        let n = args.turns.unwrap_or(RECALL_TURNS).max(1);
        let skip = all_turns.len().saturating_sub(n);
        all_turns.drain(..skip);
    } else if let Some(n) = args.turns {
        let skip = all_turns.len().saturating_sub(n.max(1));
        all_turns.drain(..skip);
    }
    Ok((thread, all_turns))
}

/// Plain-text rendering of recalled turns for the agent.
pub fn render_recall(thread: u8, turns: &[Vec<ChatEntry>]) -> String {
    if turns.is_empty() {
        return format!("No messages in thread {}.", thread_key(thread));
    }
    let mut s = format!(
        "Thread {} ({} turns, oldest first)\n",
        thread_key(thread),
        turns.len()
    );
    for turn in turns {
        s.push('\n');
        for e in turn {
            let when = ts_of(e)
                .map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            let who = match (e.role, e.kind) {
                (_, EntryKind::Interrupted) => "(turn interrupted)".to_string(),
                (Author::User, _) => "user".to_string(),
                (Author::Agent, EntryKind::Progress) => "you (progress)".to_string(),
                (Author::Agent, _) => "you".to_string(),
            };
            s.push_str(&format!("[{when}] {who}:\n{}\n", e.text.trim_end()));
            for a in &e.attachments {
                s.push_str(&format!(
                    "  attachment {} {} ({})\n",
                    &a.id[..8.min(a.id.len())],
                    a.filename,
                    a.path
                ));
            }
        }
    }
    s
}

/// `recall list`: one line per thread with messages.
pub fn render_thread_list(entries: &[ChatEntry], active: u8) -> String {
    let mut s = String::new();
    for t in threads_of(entries) {
        if t.count == 0 && t.index != active {
            continue;
        }
        s.push_str(&format!(
            "{}{} {} messages{}\n",
            if t.index == active { "* " } else { "  " },
            t.key,
            t.count,
            t.last_ts
                .map(|ts| format!(", last {ts}"))
                .unwrap_or_default()
        ));
    }
    if s.is_empty() {
        s.push_str("No chat messages yet.\n");
    }
    s
}

// ---------------------------------------------------------------------------
// Search (palette: "search inside every chat")

/// One chat entry matching a palette search.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub workspace_id: String,
    pub floor_id: Option<String>,
    pub node_id: String,
    pub thread: u8,
    pub entry_id: String,
    pub role: Author,
    pub ts: String,
    /// ~120 characters around the first match.
    pub snippet: String,
}

/// Case- and accent-insensitive form of one char (1:1, so positions in the
/// folded text are positions in the original).
fn fold_char(c: char) -> char {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    let base = std::iter::once(c)
        .nfd()
        .find(|x| !is_combining_mark(*x))
        .unwrap_or(c);
    base.to_lowercase().next().unwrap_or(base)
}

fn fold_chars(s: &str) -> Vec<char> {
    s.chars().map(fold_char).collect()
}

fn find_chars(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// ~`width` chars of `text` around char `at`, on one line.
fn snippet_at(text: &str, at: usize, len: usize, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let pad = width.saturating_sub(len) / 2;
    let start = at.saturating_sub(pad);
    let end = (at + len + pad).min(chars.len());
    let mut out: String = chars[start..end]
        .iter()
        .map(|c| if c.is_whitespace() { ' ' } else { *c })
        .collect();
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        out.insert(0, '…');
    }
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// Every thread of every terminal of the workspaces `workspaces`: entries
/// holding all the words of `query` (no case, no accents), newest first.
pub fn search(
    store: &CanvasStore,
    workspaces: &[String],
    query: &str,
    limit: usize,
) -> Vec<SearchHit> {
    let words: Vec<Vec<char>> = query
        .split_whitespace()
        .map(fold_chars)
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for ws in workspaces {
        if !is_safe_id(ws) {
            continue;
        }
        let Ok(dir) = fs::read_dir(chat_dir(store, ws)) else {
            continue;
        };
        for f in dir.flatten() {
            let path = f.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(node) = path.file_stem().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some((w, floor)) = store.locate_node(node) else {
                continue;
            };
            if &w != ws {
                continue;
            }
            for e in read_all(store, ws, node).unwrap_or_default() {
                if matches!(e.kind, EntryKind::Divider | EntryKind::Interrupted) {
                    continue;
                }
                let mut hay = e.text.clone();
                for a in &e.attachments {
                    hay.push(' ');
                    hay.push_str(&a.filename);
                }
                let folded = fold_chars(&hay);
                let mut first: Option<(usize, usize)> = None;
                let all = words.iter().all(|w| match find_chars(&folded, w) {
                    Some(i) => {
                        first.get_or_insert((i, w.len()));
                        true
                    }
                    None => false,
                });
                if !all {
                    continue;
                }
                let (at, len) = first.unwrap_or((0, 0));
                hits.push(SearchHit {
                    workspace_id: ws.clone(),
                    floor_id: Some(floor.clone()),
                    node_id: node.to_string(),
                    thread: e.thread,
                    entry_id: e.id.clone(),
                    role: e.role,
                    ts: e.ts.clone(),
                    snippet: snippet_at(&hay, at, len, 120),
                });
            }
        }
    }
    hits.sort_by(|a, b| b.ts.cmp(&a.ts));
    hits.truncate(limit.max(1));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(role: Author, kind: EntryKind, text: &str) -> ChatEntry {
        ChatEntry {
            id: new_id(),
            thread: 0,
            role,
            kind,
            ts: now(),
            text: text.into(),
            attachments: vec![],
            author_name: None,
            agent_type: None,
            client_message_id: None,
        }
    }

    #[test]
    fn threads_parse() {
        assert_eq!(parse_thread("green"), Some(6));
        assert_eq!(parse_thread("0"), Some(0));
        assert_eq!(parse_thread("7"), None);
        assert_eq!(parse_thread("graphite"), None);
    }

    #[test]
    fn envelope_header_is_fixed() {
        let s = envelope(1, Some(1), "hello", &[]);
        assert!(s.starts_with("---\nBelow is a message from the user through OmniGet Chat.\n"));
        assert!(s.contains("thread: purple\n---\nhello"));
        assert!(!s.contains("recall"));
        let s = envelope(2, Some(0), "x", &["/a.png".into()]);
        assert!(s.contains("recall pink"));
        assert!(s.ends_with("- /a.png\n"));
    }

    #[test]
    fn turns_split_on_user() {
        let v = vec![
            e(Author::Agent, EntryKind::Message, "boot"),
            e(Author::User, EntryKind::Message, "q1"),
            e(Author::Agent, EntryKind::Progress, "p"),
            e(Author::Agent, EntryKind::Message, "a1"),
            e(Author::User, EntryKind::Divider, ""),
            e(Author::User, EntryKind::Message, "q2"),
        ];
        let t = turns(&v);
        assert_eq!(t.len(), 3);
        assert_eq!(t[1].len(), 3);
        assert_eq!(t[2][0].text, "q2");
    }

    #[test]
    fn turn_thread_holds_while_selection_moves() {
        let dir = tempfile::tempdir().unwrap();
        let store = CanvasStore::open(dir.path().to_path_buf());
        let ws = super::super::model::DEFAULT_WORKSPACE_ID;
        user_message(&store, ws, "n1", 1, "q", &[], None).unwrap();
        // Locked while the turn is open on another thread.
        assert!(select_thread(&store, ws, "n1", 0, true)
            .unwrap_err()
            .starts_with("CHAT_TURN_ACTIVE"));
        assert!(select_thread(&store, ws, "n1", 7, false).is_err());
        select_thread(&store, ws, "n1", 0, false).unwrap();
        let st = load_state(&store, ws, "n1");
        assert_eq!((st.active(), st.thread), (0, 1));
        let a = say(
            &store,
            ws,
            "n1",
            SayArgs {
                text: Some("a".into()),
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
        assert_eq!(a.thread, 1);
        let p = history_opts(&store, ws, "n1", Some(1), None, None, true).unwrap();
        assert_eq!(p.unread_count, 1);
        let p = history_opts(&store, ws, "n1", Some(1), None, None, false).unwrap();
        assert_eq!(p.unread[1], 1);
        assert_eq!(load_state(&store, ws, "n1").unread_total(), 0);
        user_message(&store, ws, "n1", 1, "q2", &[], None).unwrap();
        let e = end_turn(&store, ws, "n1", "Interrupted").unwrap().unwrap();
        assert_eq!(e.kind, EntryKind::Interrupted);
        assert!(!load_state(&store, ws, "n1").turn_open);
        assert!(end_turn(&store, ws, "n1", "x").unwrap().is_none());
    }

    #[test]
    fn search_folds_case_and_accents() {
        assert_eq!(fold_chars("Café"), fold_chars("cafe"));
        let h = fold_chars("o termo-raro-77 aqui");
        assert_eq!(find_chars(&h, &fold_chars("TERMO-raro")), Some(2));
        assert_eq!(snippet_at("abc def", 4, 3, 120), "abc def");
    }

    #[test]
    fn delete_and_wipe_keep_other_threads() {
        let dir = tempfile::tempdir().unwrap();
        let store = CanvasStore::open(dir.path().to_path_buf());
        let ws = super::super::model::DEFAULT_WORKSPACE_ID;
        let src = dir.path().join("a.txt");
        fs::write(&src, "x").unwrap();
        let att = keep_attachment(&store, ws, "n1", &src).unwrap();
        let mut a = e(Author::Agent, EntryKind::Message, "a");
        a.attachments.push(att.clone());
        let mut b = e(Author::User, EntryKind::Message, "b");
        b.thread = 2;
        let c = e(Author::User, EntryKind::Message, "c");
        for x in [&a, &b, &c] {
            append(&store, ws, "n1", x).unwrap();
        }
        let r = delete_entry(&store, ws, "n1", &a.id, None).unwrap();
        assert_eq!(r.remaining, 2);
        assert_eq!(r.attachments_removed, 1);
        assert!(!Path::new(&att.path).exists());
        assert!(delete_entry(&store, ws, "n1", &a.id, None).is_err());
        let r = wipe(&store, ws, "n1", Some(2)).unwrap();
        assert_eq!(r.removed, vec![b.id.clone()]);
        assert_eq!(read_all(&store, ws, "n1").unwrap().len(), 1);
        wipe(&store, ws, "n1", None).unwrap();
        assert!(read_all(&store, ws, "n1").unwrap().is_empty());
    }
}
