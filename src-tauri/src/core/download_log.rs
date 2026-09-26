use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const MAX_LINES_PER_DOWNLOAD: usize = 200;
const EMIT_THROTTLE_MS: u64 = 200;

struct Entry {
    lines: VecDeque<String>,
    last_emit: Option<Instant>,
    pending_emit: bool,
}

impl Entry {
    fn new() -> Self {
        Self {
            lines: VecDeque::with_capacity(MAX_LINES_PER_DOWNLOAD),
            last_emit: None,
            pending_emit: false,
        }
    }
}

static STORE: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<u64, Entry>> {
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn push_line(id: u64, line: &str) -> bool {
    let line = super::flight_recorder::redact(line);
    super::flight_recorder::record(&line);
    super::download_journal::record(id, &line);
    let mut map = match store().lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if map.len() >= 1024 && !map.contains_key(&id) {
        if let Some(old) = map.keys().next().copied() {
            map.remove(&old);
        }
    }
    let entry = map.entry(id).or_insert_with(Entry::new);
    if entry.lines.len() >= MAX_LINES_PER_DOWNLOAD {
        entry.lines.pop_front();
    }
    entry.lines.push_back(line);

    let now = Instant::now();
    let should_emit = match entry.last_emit {
        Some(t) => now.duration_since(t) >= Duration::from_millis(EMIT_THROTTLE_MS),
        None => true,
    };
    if should_emit {
        entry.last_emit = Some(now);
        entry.pending_emit = false;
        true
    } else {
        entry.pending_emit = true;
        false
    }
}

pub fn get(id: u64) -> Vec<String> {
    match store().lock() {
        Ok(g) => g
            .get(&id)
            .map(|e| e.lines.iter().cloned().collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn clear(id: u64) {
    if let Ok(mut g) = store().lock() {
        g.remove(&id);
    }
}

pub fn clear_all() {
    if let Ok(mut g) = store().lock() {
        g.clear();
    }
}
