//! `meeting.md`: YAML frontmatter (the same fields `minutes` uses, so tools
//! that read those notes read these) plus summary, action items and the
//! transcript. Plain markdown for Obsidian, grep and any agent.

use super::transcript::{fmt_clock, merge_turns, Segment};
use super::{Meeting, MeetingSource};

fn yaml_str(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
    )
}

fn source_label(m: &Meeting) -> String {
    match &m.source {
        MeetingSource::Capture { .. } => "capture".into(),
        MeetingSource::File { .. } => "file".into(),
        MeetingSource::Teams { .. } => "teams-recording".into(),
        MeetingSource::Bot { platform, .. } => format!("bot:{platform}"),
    }
}

pub fn render(m: &Meeting, segments: &[Segment]) -> String {
    let date = chrono::DateTime::from_timestamp_millis(m.created_at_ms as i64)
        .map(|d| d.with_timezone(&chrono::Local).to_rfc3339())
        .unwrap_or_default();
    let mut out = String::from("---\n");
    out.push_str(&format!("title: {}\n", yaml_str(&m.title)));
    out.push_str("type: meeting\n");
    out.push_str(&format!("id: {}\n", m.id));
    out.push_str(&format!("date: {date}\n"));
    if m.duration_ms > 0 {
        out.push_str(&format!(
            "duration: {}\n",
            yaml_str(&fmt_clock(m.duration_ms))
        ));
    }
    out.push_str(&format!("source: {}\n", source_label(m)));
    if let Some(l) = &m.language {
        out.push_str(&format!("language: {l}\n"));
    }
    if !m.speakers.is_empty() {
        out.push_str("attendees:\n");
        for s in &m.speakers {
            out.push_str(&format!("  - {}\n", yaml_str(s)));
        }
    }
    if let Some(s) = &m.summary {
        if !s.action_items.is_empty() {
            out.push_str("action_items:\n");
            for a in &s.action_items {
                out.push_str(&format!("  - task: {}\n", yaml_str(&a.task)));
                if let Some(o) = &a.owner {
                    out.push_str(&format!("    assignee: {}\n", yaml_str(o)));
                }
                if let Some(d) = &a.due {
                    out.push_str(&format!("    due: {}\n", yaml_str(d)));
                }
                out.push_str(&format!(
                    "    status: {}\n",
                    if a.done { "done" } else { "open" }
                ));
            }
        }
        if !s.decisions.is_empty() {
            out.push_str("decisions:\n");
            for d in &s.decisions {
                out.push_str(&format!("  - text: {}\n", yaml_str(d)));
            }
        }
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", m.title));

    if let Some(s) = &m.summary {
        if !s.overview.is_empty() {
            out.push_str("## Summary\n\n");
            out.push_str(s.overview.trim());
            out.push_str("\n\n");
        }
        bullets(&mut out, "Key points", &s.key_points);
        bullets(&mut out, "Decisions", &s.decisions);
        if !s.action_items.is_empty() {
            out.push_str("## Action items\n\n");
            for a in &s.action_items {
                out.push_str(if a.done { "- [x] " } else { "- [ ] " });
                out.push_str(a.task.trim());
                let mut meta = Vec::new();
                if let Some(o) = &a.owner {
                    meta.push(format!("@{o}"));
                }
                if let Some(d) = &a.due {
                    meta.push(format!("due {d}"));
                }
                if !meta.is_empty() {
                    out.push_str(&format!(" ({})", meta.join(", ")));
                }
                out.push('\n');
            }
            out.push('\n');
        }
        bullets(&mut out, "Open questions", &s.open_questions);
    }
    if !m.notes.trim().is_empty() {
        out.push_str("## Notes\n\n");
        out.push_str(m.notes.trim());
        out.push_str("\n\n");
    }
    if !segments.is_empty() {
        out.push_str("## Transcript\n\n");
        for s in merge_turns(segments, 2000) {
            out.push_str(&format!("**[{}]", fmt_clock(s.start_ms)));
            if let Some(sp) = &s.speaker {
                out.push_str(&format!(" {sp}"));
            }
            out.push_str(":** ");
            out.push_str(s.text.trim());
            out.push_str("\n\n");
        }
    }
    out
}

fn bullets(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("## {title}\n\n"));
    for i in items {
        out.push_str(&format!("- {}\n", i.trim()));
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::meetings::{ActionItem, MeetingStatus, Summary};

    #[test]
    fn renders_frontmatter_and_sections() {
        let m = Meeting {
            id: "20260929-100000-abcdef".into(),
            title: "Sprint \"review\"".into(),
            created_at_ms: 1_790_000_000_000,
            duration_ms: 125_000,
            source: MeetingSource::Teams {
                url: "https://x".into(),
            },
            status: MeetingStatus::Ready,
            error: None,
            audio: None,
            tracks: vec![],
            language: Some("pt".into()),
            transcript_source: Some("teams".into()),
            segment_count: 1,
            speakers: vec!["Ana".into()],
            summary: Some(Summary {
                overview: "Revisamos o sprint.".into(),
                decisions: vec!["Adiar o deploy".into()],
                action_items: vec![ActionItem {
                    task: "Abrir PR".into(),
                    owner: Some("Ana".into()),
                    due: Some("sexta".into()),
                    done: false,
                }],
                ..Default::default()
            }),
            notes: String::new(),
            updated_at_ms: 0,
        };
        let md = render(
            &m,
            &[Segment {
                start_ms: 0,
                end_ms: 1000,
                speaker: Some("Ana".into()),
                text: "Oi".into(),
            }],
        );
        assert!(md.starts_with("---\ntitle: \"Sprint \\\"review\\\"\"\n"));
        assert!(md.contains("source: teams-recording\n"));
        assert!(md.contains("duration: \"02:05\"\n"));
        assert!(md.contains(
            "  - task: \"Abrir PR\"\n    assignee: \"Ana\"\n    due: \"sexta\"\n    status: open\n"
        ));
        assert!(md.contains("- [ ] Abrir PR (@Ana, due sexta)\n"));
        assert!(md.contains("**[00:00] Ana:** Oi\n"));
    }
}
