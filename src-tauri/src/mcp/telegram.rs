use serde_json::{json, Value};

use super::policy::Principal;
use omniget_core::core::telegram::tools::{self, Caller};

pub async fn call(p: &Principal, name: &str, a: Value) -> Result<Value, String> {
    if !a.is_object() {
        return Err("INVALID_ARGUMENTS".into());
    }
    tools::call_as(Caller::External(p.name.clone()), name, a).await
}

fn o(props: Value) -> Value {
    json!({ "type": "object", "properties": props })
}

fn t(kind: &str) -> Value {
    json!({ "type": kind })
}

fn chat() -> Value {
    o(json!({ "id": t("integer"), "type": t("string"), "title": t("string") }))
}

pub fn output_schema(name: &str) -> Option<Value> {
    if !name.starts_with("tg_") {
        return None;
    }
    Some(match name {
        "tg_status" => o(
            json!({ "logged_in": t("boolean"), "account": {"type": ["object", "null"]}, "connection": t("object") }),
        ),
        "tg_list_chats" => o(json!({ "total": t("integer"), "chats": t("array") })),
        "tg_read_messages" => o(
            json!({ "chat": chat(), "count": t("integer"), "messages": t("array"), "next_offset_id": {"type": ["integer", "null"]} }),
        ),
        "tg_search" => o(json!({ "count": t("integer"), "hits": t("array") })),
        "tg_chat_info" => {
            o(json!({ "id": t("integer"), "title": t("string"), "type": t("string") }))
        }
        "tg_list_media" => o(json!({ "chat": chat(), "count": t("integer"), "items": t("array") })),
        "tg_resolve_links" => o(json!({ "items": t("array"), "errors": t("array") })),
        "tg_stream_url" => o(json!({ "url": t("string"), "mime": t("string") })),
        "tg_download_status" => o(json!({ "count": t("integer"), "active": t("array") })),
        "tg_watch_list" => o(json!({ "rules": t("array") })),
        "tg_digest" => o(
            json!({ "since": t("integer"), "chats": t("integer"), "messages": t("integer"), "digest": t("string"), "errors": t("array") }),
        ),
        "tg_export" => o(
            json!({ "chat": chat(), "format": t("string"), "count": t("integer"), "truncated": t("boolean") }),
        ),
        "tg_download" => {
            o(json!({ "batch_id": {"type": ["integer", "null"]}, "count": t("integer") }))
        }
        "tg_watch_add" | "tg_watch_update" => o(json!({ "rule": t("object") })),
        "tg_watch_remove" => o(json!({ "ok": t("boolean") })),
        "tg_watch_run" => o(json!({ "result": {} })),
        "tg_send_message" => o(
            json!({ "ok": t("boolean"), "chat": chat(), "message_id": {"type": ["integer", "null"]} }),
        ),
        "tg_send_files" => o(json!({ "ok": t("boolean"), "chat": chat(), "count": t("integer") })),
        "tg_forward" => o(json!({ "ok": t("boolean"), "count": t("integer") })),
        "tg_delete_messages" => {
            o(json!({ "ok": t("boolean"), "chat": chat(), "count": t("integer") }))
        }
        _ => o(json!({})),
    })
}
