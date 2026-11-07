use serde_json::Value;

use super::model::{LyricLine, Lyrics};
use crate::core::music::lrc as music_lrc;

pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let parsed = music_lrc::parse(text);
    if parsed.is_synced() {
        return parsed
            .lines
            .into_iter()
            .map(|l| LyricLine {
                start_ms: Some(l.time_ms.clamp(0, u32::MAX as i64) as u32),
                text: l.text,
            })
            .collect();
    }
    parsed
        .plain
        .as_deref()
        .unwrap_or(text)
        .lines()
        .map(|l| LyricLine {
            start_ms: None,
            text: l.trim().to_string(),
        })
        .collect()
}

fn lyrics_from(text: &str, synced: bool) -> Option<Lyrics> {
    let lines = parse_lrc(text);
    let synced = synced && lines.iter().any(|l| l.start_ms.is_some());
    if lines.iter().all(|l| l.text.is_empty()) {
        return None;
    }
    Some(Lyrics {
        source: "lrclib".into(),
        synced,
        lines,
        footer: Some("LRCLIB".into()),
    })
}

fn non_empty(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.trim().is_empty())
}

pub fn lrclib_pick(json: &Value, duration_s: u32) -> Option<Lyrics> {
    let list: Vec<&Value> = match json {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![json],
        _ => return None,
    };
    let best = list
        .into_iter()
        .filter(|e| !e["instrumental"].as_bool().unwrap_or(false))
        .filter(|e| {
            non_empty(&e["syncedLyrics"]).is_some() || non_empty(&e["plainLyrics"]).is_some()
        })
        .filter_map(|e| {
            let diff = match (duration_s, e["duration"].as_f64()) {
                (0, _) => 0.0,
                (want, Some(d)) => (d - want as f64).abs(),
                (_, None) => return None,
            };
            (diff <= 2.0).then_some((e, diff))
        })
        .min_by(|(a, da), (b, db)| {
            let sa = non_empty(&a["syncedLyrics"]).is_none();
            let sb = non_empty(&b["syncedLyrics"]).is_none();
            sa.cmp(&sb).then(da.total_cmp(db))
        })?
        .0;
    match non_empty(&best["syncedLyrics"]) {
        Some(s) => lyrics_from(s, true),
        None => lyrics_from(non_empty(&best["plainLyrics"])?, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lrc_synced_and_plain() {
        let l = parse_lrc("[ar:X]\n[00:12.34]First\n[00:05.00][00:20.00]Chorus\n");
        let got: Vec<(Option<u32>, &str)> =
            l.iter().map(|x| (x.start_ms, x.text.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (Some(5000), "Chorus"),
                (Some(12340), "First"),
                (Some(20000), "Chorus")
            ]
        );
        let plain = parse_lrc("line one\nline two");
        assert_eq!(plain.len(), 2);
        assert!(plain.iter().all(|x| x.start_ms.is_none()));
        assert_eq!(plain[1].text, "line two");
    }

    #[test]
    fn lrclib_prefers_synced_within_two_seconds() {
        let body = json!([
            {"id": 1, "duration": 200.0, "instrumental": false, "plainLyrics": "far off", "syncedLyrics": "[00:01.00]far"},
            {"id": 2, "duration": 181.0, "instrumental": false, "plainLyrics": "plain only", "syncedLyrics": null},
            {"id": 3, "duration": 179.5, "instrumental": false, "plainLyrics": "a", "syncedLyrics": "[00:01.00]synced line"},
            {"id": 4, "duration": 180.0, "instrumental": true, "plainLyrics": null, "syncedLyrics": null}
        ]);
        let l = lrclib_pick(&body, 180).unwrap();
        assert!(l.synced);
        assert_eq!(l.source, "lrclib");
        assert_eq!(l.lines[0].text, "synced line");

        let only_plain = json!([{"duration": 181.0, "plainLyrics": "p1\np2", "syncedLyrics": ""}]);
        let l = lrclib_pick(&only_plain, 180).unwrap();
        assert!(!l.synced);
        assert_eq!(l.lines.len(), 2);

        assert!(lrclib_pick(
            &json!([{"duration": 300.0, "syncedLyrics": "[00:01.00]x"}]),
            180
        )
        .is_none());
        assert!(lrclib_pick(
            &json!({"duration": 999.0, "syncedLyrics": "[00:01.00]x"}),
            0
        )
        .is_some());
        assert!(lrclib_pick(&json!("nope"), 10).is_none());
    }
}
