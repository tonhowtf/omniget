use crate::proto::prelude::{wa, InboundMessage};
use crate::proto::waproto::codec::message_to_vec;
use crate::proto::waproto::whatsapp::web_message_info::StubType;

use crate::model::{protocol_op, reaction_of, summarize, MessageKind, Op, Summary};
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
                for r in &web.reactions {
                    let Some(rk) = r.key.as_option() else {
                        continue;
                    };
                    let who = if rk.from_me.unwrap_or(false) {
                        None
                    } else {
                        Some(tx.canonical(rk.participant.as_deref().unwrap_or(&chat)))
                    };
                    tx.set_reaction(
                        &chat,
                        id,
                        who.as_deref(),
                        r.text.as_deref().unwrap_or_default(),
                    )?;
                }
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
            if let Some((id, emoji)) = reaction_of(&item.message) {
                let who = if src.is_from_me {
                    None
                } else {
                    Some(tx.canonical(&src.sender.to_non_ad_string()))
                };
                if tx.set_reaction(&chat, &id, who.as_deref(), &emoji)? && !touched.contains(&chat)
                {
                    touched.push(chat);
                }
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
                tx.mark_live(&chat, item.info.id.as_str())?;
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
    use crate::proto::prelude::MessageField;

    #[test]
    fn status_broadcast_is_skipped() {
        assert!(skip_chat("status@broadcast"));
        assert!(!skip_chat("123@g.us"));
        assert!(!skip_chat("5511@s.whatsapp.net"));
    }

    fn key(chat: &str, from_me: bool, id: &str) -> MessageField<wa::MessageKey> {
        let mut key = wa::MessageKey::default();
        key.remote_jid = Some(chat.into());
        key.from_me = Some(from_me);
        key.id = Some(id.into());
        MessageField::some(key)
    }

    fn text(text: &str) -> MessageField<wa::Message> {
        let mut message = wa::Message::default();
        message.conversation = Some(text.into());
        MessageField::some(message)
    }

    fn synced(info: wa::WebMessageInfo) -> wa::HistorySyncMsg {
        let mut msg = wa::HistorySyncMsg::default();
        msg.message = MessageField::some(info);
        msg
    }

    fn conversation(id: &str, messages: Vec<wa::HistorySyncMsg>) -> wa::Conversation {
        let mut conversation = wa::Conversation::default();
        conversation.id = id.into();
        conversation.messages = messages;
        conversation
    }

    #[test]
    fn history_sync_fills_chats_messages_and_names() {
        let store = Store::open_memory().unwrap();
        let web = |id: &str, ts: u64, body: &str, from_me: bool| {
            let mut info = wa::WebMessageInfo::default();
            info.key = key("99@lid", from_me, id);
            info.message = text(body);
            info.message_timestamp = Some(ts);
            synced(info)
        };
        let mut chat = conversation(
            "99@lid",
            vec![
                web("m1", 100, "oi", false),
                web("m2", 160, "tudo bem?", true),
            ],
        );
        chat.pn_jid = Some("5511988887777@s.whatsapp.net".into());
        chat.unread_count = Some(3);
        let mut pushname = wa::Pushname::default();
        pushname.id = Some("5511988887777@s.whatsapp.net".into());
        pushname.pushname = Some("Carla".into());
        let mut hs = wa::HistorySync::default();
        hs.conversations = vec![chat, conversation("status@broadcast", vec![])];
        hs.pushnames = vec![pushname];
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
        let chat = "5511@s.whatsapp.net";
        let mut edited = wa::message::ProtocolMessage::default();
        edited.key = key(chat, false, "m1");
        edited.r#type = Some(Type::MESSAGE_EDIT);
        edited.edited_message = text("texto novo");
        let mut edit_message = wa::Message::default();
        edit_message.protocol_message = MessageField::some(edited);
        let mut edit = wa::WebMessageInfo::default();
        edit.key = key(chat, false, "e1");
        edit.message = MessageField::some(edit_message);
        edit.message_timestamp = Some(300);
        let mut stub = wa::WebMessageInfo::default();
        stub.key = key(chat, false, "m2");
        stub.message_stub_type = Some(StubType::REVOKE);
        stub.message_timestamp = Some(250);
        let mut original = wa::WebMessageInfo::default();
        original.key = key(chat, false, "m1");
        original.message = text("texto velho");
        original.message_timestamp = Some(200);
        let mut hs = wa::HistorySync::default();
        hs.conversations = vec![conversation(
            chat,
            vec![synced(edit), synced(stub), synced(original)],
        )];
        history(&store, &hs).unwrap();
        let rows = store.messages("5511@s.whatsapp.net", None, 10).unwrap();
        assert_eq!(rows.len(), 2);
        let m1 = rows.iter().find(|m| m.id == "m1").unwrap();
        assert_eq!(m1.text.as_deref(), Some("texto novo"));
        assert!(m1.edited);
        let m2 = rows.iter().find(|m| m.id == "m2").unwrap();
        assert!(m2.deleted);
    }
    #[test]
    fn history_keeps_reactions_and_who_reacted() {
        let store = Store::open_memory().unwrap();
        let chat = "5511@s.whatsapp.net";
        let reaction = |from_me: bool, emoji: &str| {
            let mut reaction = wa::Reaction::default();
            reaction.key = key(chat, from_me, "r");
            reaction.text = Some(emoji.into());
            reaction
        };
        let mut info = wa::WebMessageInfo::default();
        info.key = key(chat, true, "m1");
        info.message = text("oi");
        info.message_timestamp = Some(100);
        info.reactions = vec![reaction(false, "👍"), reaction(true, "❤️")];
        let mut hs = wa::HistorySync::default();
        hs.conversations = vec![conversation(chat, vec![synced(info)])];
        history(&store, &hs).unwrap();
        let rows = store.messages("5511@s.whatsapp.net", None, 10).unwrap();
        let got: Vec<(&str, u32, bool)> = rows[0]
            .reactions
            .iter()
            .map(|r| (r.emoji.as_str(), r.count, r.mine))
            .collect();
        assert_eq!(got, vec![("👍", 1, false), ("❤️", 1, true)]);
    }
}
