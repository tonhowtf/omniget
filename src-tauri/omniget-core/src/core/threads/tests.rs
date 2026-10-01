//! Decider and projector tests, plus the acceptance test: a simulated turn
//! (delta, tool, pending approval) survives closing and reopening the
//! database with the same snapshot.

use serde_json::json;

use super::engine::ThreadsEngine;

// ── Engine + SQLite ─────────────────────────────────────────────────────

fn temp_db(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "omniget-threads-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("threads.db")
}

#[test]
fn migration_imports_once() {
    use crate::core::llm::coordinator::append_message;
    use crate::core::llm::types::{ContentPart, Message, Role};
    let path = temp_db("migrate");
    let conv_dir = path.parent().unwrap().join("conversations");
    append_message(
        &conv_dir,
        "chat-1",
        Some("coder"),
        &Message::text(Role::User, "hello"),
    );
    append_message(
        &conv_dir,
        "chat-1",
        Some("coder"),
        &Message {
            role: Role::Assistant,
            parts: vec![
                ContentPart::Text { text: "hi".into() },
                ContentPart::ToolUse {
                    id: "c1".into(),
                    name: "fs_read".into(),
                    input: json!({"path": "a"}),
                },
            ],
        },
    );
    append_message(
        &conv_dir,
        "chat-1",
        Some("coder"),
        &Message {
            role: Role::Tool,
            parts: vec![ContentPart::ToolResult {
                tool_use_id: "c1".into(),
                content: "file".into(),
                is_error: false,
            }],
        },
    );
    append_message(
        &conv_dir,
        "help-x",
        None,
        &Message::text(Role::User, "help"),
    );
    let e = ThreadsEngine::open(&path).unwrap();
    let r1 = super::migrate::migrate_conversations(&e, &conv_dir, |_| None);
    assert_eq!(r1.imported, 1, "{r1:?}");
    let r2 = super::migrate::migrate_conversations(&e, &conv_dir, |_| None);
    assert_eq!(r2.imported, 0);
    assert_eq!(r2.skipped, 1);
    let page = e.turns_page_blocking("chat-1", None, 10).unwrap();
    assert_eq!(page.turns.len(), 1);
    assert_eq!(
        page.turns[0].activities[0].payload["data"]["output"],
        "file"
    );
    drop(e);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
