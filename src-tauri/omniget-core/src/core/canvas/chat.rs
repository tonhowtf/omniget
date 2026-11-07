use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::is_safe_id;
use super::store::CanvasStore;

pub const THREADS: [(&str, &str); 7] = [
    ("blue", "#3B82F6"),
    ("purple", "#8B5CF6"),
    ("pink", "#EC4899"),
    ("red", "#EF4444"),
    ("orange", "#F97316"),
    ("yellow", "#EAB308"),
    ("green", "#22C55E"),
];

pub const WINDOW: usize = 80;
pub const PAGE: usize = 60;
pub const RECALL_TURNS: usize = 10;
pub const MAX_ATTACHMENT: u64 = 25 * 1024 * 1024;
pub const INLINE_MAX_BYTES: usize = 4 * 1024;
pub const INLINE_MAX_LINES: usize = 40;
pub const MAX_TEXT: usize = 512 * 1024;

pub fn thread_key(i: u8) -> &'static str {
    THREADS.get(i as usize).map(|t| t.0).unwrap_or("blue")
}

pub fn thread_color(i: u8) -> &'static str {
    THREADS.get(i as usize).map(|t| t.1).unwrap_or("#3B82F6")
}

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
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub filename: String,
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
    pub ts: String,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatState {
    #[serde(default)]
    pub thread: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<u8>,
    #[serde(default)]
    pub turn_open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_started: Option<String>,
    #[serde(default)]
    pub turn_interrupted: bool,
    #[serde(default)]
    pub unread: [u32; 7],
}

impl ChatState {
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
    pub turn_thread: u8,
    pub turn_interrupted: bool,
    pub unread: [u32; 7],
    pub unread_count: u32,
}

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

pub fn inbox_attachments_dir(store: &CanvasStore, ws: &str) -> Result<PathBuf, String> {
    if !is_safe_id(ws) {
        return Err("CANVAS_INVALID: id".into());
    }
    Ok(store.workspace_dir(ws).join("inbox").join("attachments"))
}

pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub removed: Vec<String>,
    pub attachments_removed: usize,
    pub remaining: usize,
}

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

pub struct Outgoing {
    pub entry: ChatEntry,
    pub envelope: String,
}

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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SayArgs {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub progress: bool,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, alias = "text-from")]
    pub text_from: Option<String>,
    #[serde(default)]
    pub attach: Vec<String>,
    #[serde(default)]
    pub thread: Option<String>,
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallArgs {
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default)]
    pub turns: Option<usize>,
    #[serde(default)]
    pub last: Option<String>,
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
    pub snippet: String,
}

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
