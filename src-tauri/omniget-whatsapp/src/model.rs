use serde::{Deserialize, Serialize};
use whatsapp_rust::prelude::{wa, MessageExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Text,
    Image,
    Video,
    Audio,
    Voice,
    Document,
    Sticker,
    Location,
    Contact,
    Poll,
}

impl MessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Voice => "voice",
            Self::Document => "document",
            Self::Sticker => "sticker",
            Self::Location => "location",
            Self::Contact => "contact",
            Self::Poll => "poll",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => Self::Text,
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "voice" => Self::Voice,
            "document" => Self::Document,
            "sticker" => Self::Sticker,
            "location" => Self::Location,
            "contact" => Self::Contact,
            "poll" => Self::Poll,
            _ => return None,
        })
    }

    pub fn media(self) -> Option<MediaKind> {
        match self {
            Self::Image => Some(MediaKind::Image),
            Self::Video => Some(MediaKind::Video),
            Self::Audio | Self::Voice => Some(MediaKind::Audio),
            Self::Document => Some(MediaKind::Document),
            Self::Sticker => Some(MediaKind::Sticker),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Document,
    Sticker,
}

impl MediaKind {
    pub fn message_kinds(self) -> &'static [MessageKind] {
        match self {
            Self::Image => &[MessageKind::Image],
            Self::Video => &[MessageKind::Video],
            Self::Audio => &[MessageKind::Audio, MessageKind::Voice],
            Self::Document => &[MessageKind::Document],
            Self::Sticker => &[MessageKind::Sticker],
        }
    }
}

pub fn kinds_for_filter(filter: &[MediaKind]) -> Vec<MessageKind> {
    let mut out: Vec<MessageKind> = Vec::new();
    for m in filter {
        for k in m.message_kinds() {
            if !out.contains(k) {
                out.push(*k);
            }
        }
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub kind: Option<MessageKind>,
    pub text: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub file_name: Option<String>,
    pub duration: Option<u32>,
    pub thumb: Option<Vec<u8>>,
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn summarize(message: &wa::Message) -> Option<Summary> {
    let base = message.get_base_message();
    if let Some(img) = base.image_message.as_option() {
        return Some(Summary {
            kind: Some(MessageKind::Image),
            text: non_empty(img.caption.as_deref()),
            mime: img.mimetype.clone(),
            size: img.file_length,
            thumb: img.jpeg_thumbnail.clone(),
            ..Default::default()
        });
    }
    if let Some(vid) = base
        .video_message
        .as_option()
        .or_else(|| base.ptv_message.as_option())
    {
        return Some(Summary {
            kind: Some(MessageKind::Video),
            text: non_empty(vid.caption.as_deref()),
            mime: vid.mimetype.clone(),
            size: vid.file_length,
            duration: vid.seconds,
            thumb: vid.jpeg_thumbnail.clone(),
            ..Default::default()
        });
    }
    if let Some(aud) = base.audio_message.as_option() {
        let kind = if aud.ptt.unwrap_or(false) {
            MessageKind::Voice
        } else {
            MessageKind::Audio
        };
        return Some(Summary {
            kind: Some(kind),
            mime: aud.mimetype.clone(),
            size: aud.file_length,
            duration: aud.seconds,
            ..Default::default()
        });
    }
    if let Some(doc) = base.document_message.as_option() {
        return Some(Summary {
            kind: Some(MessageKind::Document),
            text: non_empty(doc.caption.as_deref()),
            mime: doc.mimetype.clone(),
            size: doc.file_length,
            file_name: non_empty(doc.file_name.as_deref())
                .or_else(|| non_empty(doc.title.as_deref())),
            thumb: doc.jpeg_thumbnail.clone(),
            ..Default::default()
        });
    }
    if let Some(st) = base.sticker_message.as_option() {
        return Some(Summary {
            kind: Some(MessageKind::Sticker),
            mime: st.mimetype.clone(),
            size: st.file_length,
            ..Default::default()
        });
    }
    if let Some(loc) = base.location_message.as_option() {
        let label = non_empty(loc.name.as_deref())
            .or_else(|| non_empty(loc.address.as_deref()))
            .unwrap_or_else(|| {
                format!(
                    "{:.5}, {:.5}",
                    loc.degrees_latitude.unwrap_or_default(),
                    loc.degrees_longitude.unwrap_or_default()
                )
            });
        return Some(Summary {
            kind: Some(MessageKind::Location),
            text: Some(label),
            ..Default::default()
        });
    }
    if let Some(c) = base.contact_message.as_option() {
        return Some(Summary {
            kind: Some(MessageKind::Contact),
            text: non_empty(c.display_name.as_deref()),
            ..Default::default()
        });
    }
    if let Some(p) = base.poll_creation_message.as_option() {
        return Some(Summary {
            kind: Some(MessageKind::Poll),
            text: non_empty(p.name.as_deref()),
            ..Default::default()
        });
    }
    let text = non_empty(base.text_content())?;
    Some(Summary {
        kind: Some(MessageKind::Text),
        text: Some(text),
        ..Default::default()
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Revoke { id: String },
    Edit { id: String, text: Option<String> },
}

pub fn protocol_op(message: &wa::Message) -> Option<Op> {
    use wa::message::protocol_message::Type;
    let base = message.get_base_message();
    let pm = base.protocol_message.as_option()?;
    let id = pm.key.as_option()?.id.clone().filter(|i| !i.is_empty())?;
    match pm.r#type {
        Some(Type::REVOKE) => Some(Op::Revoke { id }),
        Some(Type::MESSAGE_EDIT) => {
            let edited = pm.edited_message.as_option()?;
            let text = summarize(edited).and_then(|s| s.text);
            Some(Op::Edit { id, text })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::prelude::MessageField;

    #[test]
    fn text_message_is_text() {
        let m = wa::Message {
            conversation: Some("oi".into()),
            ..Default::default()
        };
        let s = summarize(&m).unwrap();
        assert_eq!(s.kind, Some(MessageKind::Text));
        assert_eq!(s.text.as_deref(), Some("oi"));
    }

    #[test]
    fn empty_message_is_skipped() {
        assert!(summarize(&wa::Message::default()).is_none());
    }

    #[test]
    fn voice_note_counts_as_audio_media() {
        let m = wa::Message {
            audio_message: MessageField::some(wa::message::AudioMessage {
                ptt: Some(true),
                seconds: Some(7),
                file_length: Some(1200),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarize(&m).unwrap();
        assert_eq!(s.kind, Some(MessageKind::Voice));
        assert_eq!(s.duration, Some(7));
        assert_eq!(MessageKind::Voice.media(), Some(MediaKind::Audio));
    }

    #[test]
    fn filter_expands_audio_to_voice() {
        let k = kinds_for_filter(&[MediaKind::Audio, MediaKind::Image, MediaKind::Audio]);
        assert_eq!(
            k,
            vec![MessageKind::Audio, MessageKind::Voice, MessageKind::Image]
        );
        assert!(!k.contains(&MessageKind::Text));
    }

    #[test]
    fn kind_round_trips() {
        for k in [
            MessageKind::Text,
            MessageKind::Image,
            MessageKind::Video,
            MessageKind::Audio,
            MessageKind::Voice,
            MessageKind::Document,
            MessageKind::Sticker,
            MessageKind::Location,
            MessageKind::Contact,
            MessageKind::Poll,
        ] {
            assert_eq!(MessageKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(MessageKind::parse("nope"), None);
    }

    fn protocol(
        t: wa::message::protocol_message::Type,
        edited: Option<wa::Message>,
    ) -> wa::Message {
        wa::Message {
            protocol_message: MessageField::some(wa::message::ProtocolMessage {
                key: MessageField::some(wa::MessageKey {
                    id: Some("ORIG".into()),
                    ..Default::default()
                }),
                r#type: Some(t),
                edited_message: match edited {
                    Some(m) => MessageField::some(m),
                    None => MessageField::none(),
                },
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn revoke_and_edit_are_recognized() {
        use wa::message::protocol_message::Type;
        assert_eq!(
            protocol_op(&protocol(Type::REVOKE, None)),
            Some(Op::Revoke { id: "ORIG".into() })
        );
        let new = wa::Message {
            conversation: Some("corrigido".into()),
            ..Default::default()
        };
        let edit = protocol(Type::MESSAGE_EDIT, Some(new));
        assert_eq!(
            protocol_op(&edit),
            Some(Op::Edit {
                id: "ORIG".into(),
                text: Some("corrigido".into())
            })
        );
        assert!(summarize(&edit).is_none());
        assert_eq!(protocol_op(&wa::Message::default()), None);
    }
}
