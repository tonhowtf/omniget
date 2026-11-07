use whatsapp_rust::prelude::{wa, InboundMessage};
use whatsapp_rust::waproto::codec::message_to_vec;
use whatsapp_rust::waproto::whatsapp::web_message_info::StubType;

use crate::model::{protocol_op, summarize, MessageKind, Op, Summary};
use crate::store::{NewMessage, Store, Tx};

pub fn apply_op(tx: &Tx<'_>, chat: &str, op: &Op) -> rusqlite::Result<bool> {
    match op {
        Op::Revoke { id } => tx.apply_revoke(chat, id),
        Op::Edit { id, text } => tx.apply_edit(chat, id, text.as_deref()),
    }
}

pub fn skip_chat(jid: &str) -> bool {
    jid.is_empty() || jid.ends_with("@broadcast")
}

pub fn history(store: &Store, hs: &wa::HistorySync) -> rusqlite::Result<(usize, usize)> {
    store.with_tx(|tx| {
        for m in &hs.phone_number_to_lid_mappings {
            if let (Some(pn), Some(lid)) = (m.pn_jid.as_deref(), m.lid_jid.as_deref()) {
                tx.alias(lid, pn)?;
            }
        }
        for p in &hs.pushnames {
            if let Some(id) = p.id.as_deref() {
                let id = tx.canonical(id);
                tx.contact(&id, None, p.pushname.as_deref())?;
            }
        }
        let mut chats = 0usize;
        let mut messages = 0usize;
        for conv in &hs.conversations {
            if let (Some(lid), Some(pn)) = (conv.lid_jid.as_deref(), conv.pn_jid.as_deref()) {
                tx.alias(lid, pn)?;
            }
            if conv.id.ends_with("@lid") {
                if let Some(pn) = conv.pn_jid.as_deref() {
                    tx.alias(&conv.id, pn)?;
                }
            }
            let chat = tx.canonical(&conv.id);
            if skip_chat(&chat) {
                continue;
            }
            let last_ts = conv
                .conversation_timestamp
                .or(conv.last_msg_timestamp)
                .map(|t| t.min(i64::MAX as u64) as i64);
            let name = conv
                .name
                .as_deref()
                .or(conv.display_name.as_deref())
                .filter(|n| !n.trim().is_empty());
            tx.upsert_chat(&chat, name, last_ts, conv.unread_count, conv.archived)?;
            chats += 1;
            for hm in &conv.messages {
                let Some(web) = hm.message.as_option() else {
                    continue;
                };
                let Some(key) = web.key.as_option() else {
                    continue;
                };
                let Some(id) = key.id.as_deref() else {
                    continue;
                };
                let from_me = key.from_me.unwrap_or(false);
                let wire_sender = web.participant.as_deref().or(key.participant.as_deref());
                let sender = if from_me {
                    None
                } else {
                    wire_sender.map(|s| tx.canonical(s))
                };
                let ts = web.message_timestamp.unwrap_or(0).min(i64::MAX as u64) as i64;
                let revoked_stub = web.message_stub_type == Some(StubType::REVOKE);
                let (summary, raw) = match web.message.as_option() {
                    Some(msg) => {
                        if let Some(op) = protocol_op(msg) {
                            apply_op(tx, &chat, &op)?;
                            continue;
                        }
                        let Some(summary) = summarize(msg) else {
                            continue;
                        };
                        (summary, Some(message_to_vec(msg)))
                    }
                    None if revoked_stub => (
                        Summary {
                            kind: Some(MessageKind::Text),
                            ..Default::default()
                        },
                        None,
                    ),
                    None => continue,
                };
                if tx.insert_message(&NewMessage {
                    chat: &chat,
                    id,
                    from_me,
                    sender: sender.as_deref(),
                    push_name: web.push_name.as_deref(),
                    ts,
                    summary: &summary,
                    raw: raw.as_deref(),
                    wire_chat: Some(&conv.id),
                    wire_sender,
                })? {
                    messages += 1;
                }
                if revoked_stub {
                    tx.apply_revoke(&chat, id)?;
                }
                if ts > 0 {
                    tx.upsert_chat(&chat, None, Some(ts), None, None)?;
                }
            }
        }
        Ok((chats, messages))
    })
}

pub fn live(store: &Store, items: &[InboundMessage]) -> rusqlite::Result<Vec<String>> {
    store.with_tx(|tx| {
        let mut touched: Vec<String> = Vec::new();
        for item in items {
            let src = &item.info.source;
            let raw_chat = src.chat.to_non_ad_string();
            if !src.is_group && !src.is_from_me && raw_chat.ends_with("@lid") {
                if let Some(alt) = src.sender_alt.as_ref() {
                    let alt = alt.to_non_ad_string();
                    if alt.ends_with("@s.whatsapp.net") {
                        tx.alias(&raw_chat, &alt)?;
                    }
                }
            }
            let chat = tx.canonical(&raw_chat);
            if skip_chat(&chat) {
                continue;
            }
            if let Some(op) = protocol_op(&item.message) {
                if apply_op(tx, &chat, &op)? && !touched.contains(&chat) {
                    touched.push(chat);
                }
                continue;
            }
            let Some(summary) = summarize(&item.message) else {
                continue;
            };
            let sender = if src.is_from_me {
                None
            } else {
                Some(tx.canonical(&src.sender.to_non_ad_string()))
            };
            let push = item.info.push_name.as_str();
            if let Some(s) = sender.as_deref() {
                tx.contact(s, None, Some(push))?;
            }
            let ts = item.info.timestamp.timestamp();
            let raw = message_to_vec(&item.message);
            let wire_sender = if src.is_group && !src.is_from_me {
                Some(src.sender.to_non_ad_string())
            } else {
                None
            };
            tx.upsert_chat(&chat, None, Some(ts), None, None)?;
            let inserted = tx.insert_message(&NewMessage {
                chat: &chat,
                id: item.info.id.as_str(),
                from_me: src.is_from_me,
                sender: sender.as_deref(),
                push_name: Some(push),
                ts,
                summary: &summary,
                raw: Some(&raw),
                wire_chat: Some(&raw_chat),
                wire_sender: wire_sender.as_deref(),
            })?;
            if inserted {
                if !src.is_from_me {
                    tx.bump_unread(&chat)?;
                }
                if !touched.contains(&chat) {
                    touched.push(chat);
                }
            }
        }
        Ok(touched)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::prelude::MessageField;

    #[test]
    fn status_broadcast_is_skipped() {
        assert!(skip_chat("status@broadcast"));
        assert!(!skip_chat("123@g.us"));
        assert!(!skip_chat("5511@s.whatsapp.net"));
    }

    #[test]
    fn history_sync_fills_chats_messages_and_names() {
        let store = Store::open_memory().unwrap();
        let web = |id: &str, ts: u64, text: &str, from_me: bool| wa::HistorySyncMsg {
            message: MessageField::some(wa::WebMessageInfo {
                key: MessageField::some(wa::MessageKey {
                    remote_jid: Some("99@lid".into()),
                    from_me: Some(from_me),
                    id: Some(id.into()),
                    participant: None,
                }),
                message: MessageField::some(wa::Message {
                    conversation: Some(text.into()),
                    ..Default::default()
                }),
                message_timestamp: Some(ts),
                ..Default::default()
            }),
            ..Default::default()
        };
        let hs = wa::HistorySync {
            conversations: vec![
                wa::Conversation {
                    id: "99@lid".into(),
                    pn_jid: Some("5511988887777@s.whatsapp.net".into()),
                    unread_count: Some(3),
                    messages: vec![
                        web("m1", 100, "oi", false),
                        web("m2", 160, "tudo bem?", true),
                    ],
                    ..Default::default()
                },
                wa::Conversation {
                    id: "status@broadcast".into(),
                    ..Default::default()
                },
            ],
            pushnames: vec![wa::Pushname {
                id: Some("5511988887777@s.whatsapp.net".into()),
                pushname: Some("Carla".into()),
            }],
            ..Default::default()
        };
        let (chats, msgs) = history(&store, &hs).unwrap();
        assert_eq!((chats, msgs), (1, 2));
        let list = store.chats(None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].jid, "5511988887777@s.whatsapp.net");
        assert_eq!(list[0].name, "Carla");
        assert_eq!(list[0].unread, 3);
        assert_eq!(list[0].last_ts, Some(160));
        assert_eq!(list[0].last_text.as_deref(), Some("tudo bem?"));
        let again = history(&store, &hs).unwrap();
        assert_eq!(again.1, 0);
    }

    #[test]
    fn history_applies_edits_and_revoke_stubs() {
        use wa::message::protocol_message::Type;
        let store = Store::open_memory().unwrap();
        let key = |id: &str| {
            MessageField::some(wa::MessageKey {
                remote_jid: Some("5511@s.whatsapp.net".into()),
                from_me: Some(false),
                id: Some(id.into()),
                participant: None,
            })
        };
        let edit = wa::HistorySyncMsg {
            message: MessageField::some(wa::WebMessageInfo {
                key: key("e1"),
                message: MessageField::some(wa::Message {
                    protocol_message: MessageField::some(wa::message::ProtocolMessage {
                        key: key("m1"),
                        r#type: Some(Type::MESSAGE_EDIT),
                        edited_message: MessageField::some(wa::Message {
                            conversation: Some("texto novo".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                message_timestamp: Some(300),
                ..Default::default()
            }),
            ..Default::default()
        };
        let stub = wa::HistorySyncMsg {
            message: MessageField::some(wa::WebMessageInfo {
                key: key("m2"),
                message_stub_type: Some(StubType::REVOKE),
                message_timestamp: Some(250),
                ..Default::default()
            }),
            ..Default::default()
        };
        let original = wa::HistorySyncMsg {
            message: MessageField::some(wa::WebMessageInfo {
                key: key("m1"),
                message: MessageField::some(wa::Message {
                    conversation: Some("texto velho".into()),
                    ..Default::default()
                }),
                message_timestamp: Some(200),
                ..Default::default()
            }),
            ..Default::default()
        };
        let hs = wa::HistorySync {
            conversations: vec![wa::Conversation {
                id: "5511@s.whatsapp.net".into(),
                messages: vec![edit, stub, original],
                ..Default::default()
            }],
            ..Default::default()
        };
        history(&store, &hs).unwrap();
        let rows = store.messages("5511@s.whatsapp.net", None, 10).unwrap();
        assert_eq!(rows.len(), 2);
        let m1 = rows.iter().find(|m| m.id == "m1").unwrap();
        assert_eq!(m1.text.as_deref(), Some("texto novo"));
        assert!(m1.edited);
        let m2 = rows.iter().find(|m| m.id == "m2").unwrap();
        assert!(m2.deleted);
    }
}
