//! Resume cursor of a Claude thread and the small file that keeps it.
//!
//! The cursor is what goes out in `session.started.resume` (the engine stores
//! it in `provider_sessions` and hands it back in `SessionStart`). It carries
//! the CLI session id plus one mark per engine turn: the `uuid` we stamped on
//! the prompt and the uuid of the last transcript message of that turn. The
//! last one is the `--resume-session-at` boundary that rollback and fork use
//! (verified on 2.1.280: `--resume <sid> --resume-session-at <uuid>
//! --fork-session` answers from the truncated history under a new id).
//!
//! The driver also keeps its own copy per thread in `claude-threads.json`
//! (under `<app_data>/llm`): `rollback`/`fork` arrive with only a thread id,
//! possibly after a restart, before any `start_session`. The file is written
//! only when a cursor changes; nothing reads it in the background.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnMark {
    /// Engine turn id (informational: fork copies marks under new turn ids,
    /// so boundaries are always looked up by position).
    pub turn_id: String,
    /// The `uuid` of our stream-json user message (= transcript uuid).
    pub prompt_uuid: String,
    /// Last transcript message (assistant or tool result) of the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_uuid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCursor {
    /// CLI session id; empty = start fresh.
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turns: Vec<TurnMark>,
    /// Set by rollback/fork: the next launch resumes at this message…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_at: Option<String>,
    /// …under a new session id (`--fork-session`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fork: bool,
}

impl ClaudeCursor {
    /// Accepts our own object, a bare session id string, or T3's legacy
    /// `{resume: <id>}` / `{sessionId}` shapes. Rejects non-UUID ids: a
    /// synthetic id would make `--resume` fail at every launch.
    pub fn from_value(value: &Value) -> Option<ClaudeCursor> {
        let mut cursor = match value {
            Value::String(s) => ClaudeCursor {
                session_id: s.clone(),
                ..Default::default()
            },
            Value::Object(map) => {
                let mut c: ClaudeCursor = serde_json::from_value(value.clone()).unwrap_or_default();
                if c.session_id.is_empty() {
                    if let Some(s) = map.get("resume").and_then(Value::as_str) {
                        c.session_id = s.to_string();
                    }
                }
                c
            }
            _ => return None,
        };
        if !cursor.session_id.is_empty() && uuid::Uuid::parse_str(&cursor.session_id).is_err() {
            cursor.session_id.clear();
            cursor.turns.clear();
        }
        Some(cursor)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Keep the first `turn_count` turns. `Ok(true)` when something changed,
    /// `Err` when the boundary is unknown (older cursor, interrupted turn with
    /// no transcript message).
    pub fn truncate(&mut self, turn_count: usize) -> Result<bool, String> {
        if turn_count == 0 {
            let changed = !self.session_id.is_empty() || !self.turns.is_empty();
            *self = ClaudeCursor::default();
            return Ok(changed);
        }
        if self.session_id.is_empty() || turn_count >= self.turns.len() {
            return Ok(false);
        }
        let boundary = self.turns[turn_count - 1].last_uuid.clone().ok_or_else(|| {
            format!(
                "the transcript boundary of turn {turn_count} is unknown; start a new thread instead"
            )
        })?;
        self.turns.truncate(turn_count);
        self.resume_at = Some(boundary);
        self.fork = true;
        Ok(true)
    }
}

/// Per-thread cursors on disk (`{threadId: cursor}`), plus an in-memory copy.
pub struct CursorStore {
    path: Option<PathBuf>,
    map: Mutex<Option<BTreeMap<String, ClaudeCursor>>>,
}

impl CursorStore {
    pub fn at(path: Option<PathBuf>) -> Self {
        Self {
            path,
            map: Mutex::new(None),
        }
    }

    /// `<app_data>/llm/claude-threads.json`.
    pub fn default_path() -> Option<PathBuf> {
        crate::core::llm::roster_store::llm_dir().map(|d| d.join("claude-threads.json"))
    }

    fn with<R>(&self, f: impl FnOnce(&mut BTreeMap<String, ClaudeCursor>) -> R) -> R {
        let mut guard = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let loaded = self
                .path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            *guard = Some(loaded);
        }
        f(guard.as_mut().expect("loaded above"))
    }

    pub fn get(&self, thread_id: &str) -> Option<ClaudeCursor> {
        self.with(|m| m.get(thread_id).cloned())
    }

    pub fn put(&self, thread_id: &str, cursor: &ClaudeCursor) {
        let snapshot = self.with(|m| {
            if m.get(thread_id) == Some(cursor) {
                return None;
            }
            m.insert(thread_id.to_string(), cursor.clone());
            Some(m.clone())
        });
        if let Some(all) = snapshot {
            self.write(&all);
        }
    }

    pub fn remove(&self, thread_id: &str) {
        let snapshot = self.with(|m| m.remove(thread_id).map(|_| m.clone()));
        if let Some(all) = snapshot {
            self.write(&all);
        }
    }

    fn write(&self, all: &BTreeMap<String, ClaudeCursor>) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        match serde_json::to_vec_pretty(all) {
            Ok(bytes) => {
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, path);
                }
            }
            Err(e) => tracing::warn!("[claude] cursor store: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SID: &str = "a00fe044-a480-48c4-ac8b-b3d6bdd759ed";

    fn cursor() -> ClaudeCursor {
        ClaudeCursor {
            session_id: SID.into(),
            turns: vec![
                TurnMark {
                    turn_id: "t1".into(),
                    prompt_uuid: "p1".into(),
                    last_uuid: Some("bef506b5-1936-436c-807f-a1286c3f12ce".into()),
                },
                TurnMark {
                    turn_id: "t2".into(),
                    prompt_uuid: "p2".into(),
                    last_uuid: Some("70661c9e-e6bf-484b-91e9-ae66f60b5b34".into()),
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn cursor_round_trips_and_accepts_legacy_shapes() {
        let c = cursor();
        assert_eq!(ClaudeCursor::from_value(&c.to_value()), Some(c));
        assert_eq!(
            ClaudeCursor::from_value(&json!(SID)).unwrap().session_id,
            SID
        );
        assert_eq!(
            ClaudeCursor::from_value(&json!({"resume": SID}))
                .unwrap()
                .session_id,
            SID
        );
        // A synthetic id is never resumed.
        assert!(ClaudeCursor::from_value(&json!("claude-thread-1"))
            .unwrap()
            .session_id
            .is_empty());
    }

    #[test]
    fn truncate_keeps_the_boundary_of_the_last_kept_turn() {
        let mut c = cursor();
        assert_eq!(c.truncate(1), Ok(true));
        assert_eq!(c.turns.len(), 1);
        assert_eq!(
            c.resume_at.as_deref(),
            Some("bef506b5-1936-436c-807f-a1286c3f12ce")
        );
        assert!(c.fork);
        // Keeping everything changes nothing.
        let mut c = cursor();
        assert_eq!(c.truncate(2), Ok(false));
        assert_eq!(c.truncate(9), Ok(false));
        // Zero = start over.
        let mut c = cursor();
        assert_eq!(c.truncate(0), Ok(true));
        assert!(c.session_id.is_empty());
        // Unknown boundary is an error, not a silent full resume.
        let mut c = cursor();
        c.turns[0].last_uuid = None;
        assert!(c.truncate(1).is_err());
    }

    #[test]
    fn the_store_persists_per_thread() {
        let dir =
            std::env::temp_dir().join(format!("omniget-claude-cursor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("claude-threads.json");
        let store = CursorStore::at(Some(path.clone()));
        store.put("thr_a", &cursor());
        let again = CursorStore::at(Some(path));
        assert_eq!(again.get("thr_a"), Some(cursor()));
        again.remove("thr_a");
        assert_eq!(again.get("thr_a"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
