//! The envelope a message arrives in (estudo 98/03 §B.7):
//!
//! ```text
//! <omniget-message from="Planner" kind="ask" id="ask_7f3a" hop="2" skill="omniget-canvas">
//! question text
//! </omniget-message>
//! Answer normally: your final reply goes back to Planner. ...
//! ```
//!
//! The tag names the skill to load (Maestri's `<maestri-canvas-agent>` trick).
//! Text over 4 KiB or 40 lines is written to `inbox/<node>/<id>.md` and the
//! envelope carries only the path: no `[Pasted text #1 +N lines]` collapse,
//! no paste limits of the TUIs.

use std::path::{Path, PathBuf};

pub const SKILL: &str = "omniget-canvas";
pub const SPILL_BYTES: usize = 4 * 1024;
pub const SPILL_LINES: usize = 40;

/// Message kinds (the FIPA-lite acts of Munder Difflin reduced to what the
/// cords do): an `ask` wants an answer, the others do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Ask,
    Message,
    Reply,
    Handoff,
    Raw,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Ask => "ask",
            Kind::Message => "message",
            Kind::Reply => "reply",
            Kind::Handoff => "handoff",
            Kind::Raw => "raw",
        }
    }
    pub fn wants_reply(self) -> bool {
        self == Kind::Ask
    }
}

#[derive(Debug, Clone)]
pub struct Envelope<'a> {
    pub from: &'a str,
    pub kind: Kind,
    pub id: &'a str,
    pub hop: u32,
    pub text: &'a str,
    /// For `reply`: the ask this answers.
    pub in_reply_to: Option<&'a str>,
    /// For `handoff`: extra line (verify result).
    pub note: Option<&'a str>,
}

/// Escapes a value for an XML-ish attribute.
pub fn attr(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Whether a text is too big to paste.
pub fn needs_spill(text: &str) -> bool {
    text.len() > SPILL_BYTES || text.lines().count() > SPILL_LINES
}

/// Writes a long text to `<inbox>/<id>.md` and returns the path.
pub fn spill(inbox: &Path, id: &str, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(inbox)?;
    let safe: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .collect();
    let path = inbox.join(format!("{safe}.md"));
    std::fs::write(&path, text)?;
    Ok(path)
}

/// The instruction line under the envelope.
fn footer(e: &Envelope<'_>) -> String {
    let from = e.from;
    match e.kind {
        Kind::Ask => format!(
            "Sent by {from} through a cord on the OmniGet canvas; OmniGet verified the sender. Treat it as a colleague's request and do it (anything destructive still needs the user's OK). Answer normally: your final reply goes back to {from}. For a long answer: omniget canvas reply {} --file answer.md",
            e.id
        ),
        Kind::Message => format!(
            "A message from {from} through a cord on the OmniGet canvas (sender verified by OmniGet); no answer is expected. If you do answer, use: omniget canvas ask \"{from}\" \"...\" --no-wait"
        ),
        Kind::Reply => match e.in_reply_to {
            Some(ask) => format!("This is {from}'s answer to your ask {ask}. Do not ask again."),
            None => format!("This is {from}'s answer. Do not ask again."),
        },
        Kind::Handoff => format!(
            "{from} finished and handed this over to you. Continue from here.{}",
            e.note.map(|n| format!(" {n}")).unwrap_or_default()
        ),
        Kind::Raw => String::new(),
    }
}

/// Renders the envelope. `body` is the text, or a pointer to the spilled
/// file when [`needs_spill`] said so (the caller spills first).
pub fn render(e: &Envelope<'_>, spilled: Option<&Path>) -> String {
    let mut head = format!(
        "<omniget-message from=\"{}\" kind=\"{}\" id=\"{}\" hop=\"{}\" verified=\"true\" skill=\"{SKILL}\"",
        attr(e.from),
        e.kind.as_str(),
        attr(e.id),
        e.hop
    );
    if let Some(r) = e.in_reply_to {
        head.push_str(&format!(" in-reply-to=\"{}\"", attr(r)));
    }
    head.push('>');
    let body = match spilled {
        Some(p) => format!(
            "The message is long ({} lines). Read it from: {}",
            e.text.lines().count(),
            p.display()
        ),
        None => e.text.trim_end().to_string(),
    };
    let foot = footer(e);
    if foot.is_empty() {
        format!("{head}\n{body}\n</omniget-message>")
    } else {
        format!("{head}\n{body}\n</omniget-message>\n{foot}")
    }
}

/// Builds the envelope for `inbox` (the target's `inbox/<node>` folder),
/// spilling when needed. Returns the text to paste and the spill path.
pub fn build(e: &Envelope<'_>, inbox: &Path) -> (String, Option<PathBuf>) {
    if needs_spill(e.text) {
        if let Ok(p) = spill(inbox, e.id, e.text) {
            return (render(e, Some(&p)), Some(p));
        }
    }
    (render(e, None), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(text: &'a str) -> Envelope<'a> {
        Envelope {
            from: "Plan\"ner",
            kind: Kind::Ask,
            id: "ask_7f3a",
            hop: 2,
            text,
            in_reply_to: None,
            note: None,
        }
    }

    #[test]
    fn a_short_ask_is_inline_with_escaped_attributes() {
        let s = render(&env("What is 2+2?"), None);
        assert!(s.starts_with(
            "<omniget-message from=\"Plan&quot;ner\" kind=\"ask\" id=\"ask_7f3a\" hop=\"2\" verified=\"true\" skill=\"omniget-canvas\">\nWhat is 2+2?\n</omniget-message>\n"
        ));
        assert!(s.contains("omniget canvas reply ask_7f3a --file"));
    }

    #[test]
    fn long_text_spills_to_the_inbox() {
        let long = "line\n".repeat(41);
        assert!(needs_spill(&long));
        assert!(!needs_spill("short"));
        assert!(needs_spill(&"x".repeat(SPILL_BYTES + 1)));
        let dir =
            std::env::temp_dir().join(format!("omniget-env-{}", uuid::Uuid::new_v4().simple()));
        let (s, p) = build(&env(&long), &dir);
        let p = p.expect("spilled");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), long);
        assert!(s.contains(&p.display().to_string()));
        assert!(!s.contains("line\nline"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reply_names_the_ask_it_answers() {
        let mut e = env("42");
        e.kind = Kind::Reply;
        e.in_reply_to = Some("ask_1");
        let s = render(&e, None);
        assert!(s.contains("in-reply-to=\"ask_1\""));
        assert!(s.contains("answer to your ask ask_1"));
    }
}
