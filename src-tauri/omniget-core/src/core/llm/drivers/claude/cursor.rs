use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnMark {
    pub turn_id: String,
    pub prompt_uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_uuid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCursor {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turns: Vec<TurnMark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_at: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fork: bool,
}

impl ClaudeCursor {
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

    const SID: &str = "00000000-0000-4000-8000-000000000002";

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
        assert!(ClaudeCursor::from_value(&json!("claude-thread-1"))
            .unwrap()
            .session_id
            .is_empty());
    }
}
