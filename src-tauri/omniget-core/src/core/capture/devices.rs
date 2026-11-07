use std::process::Stdio;

use super::{Platform, Source, SourceKind, Sources};

fn strip_tag(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with('[') {
        if let Some(end) = t.find(']') {
            return &t[end + 1..];
        }
    }
    t
}

pub fn parse_avfoundation(stderr: &str) -> (Vec<Source>, Vec<Source>) {
    let mut screens = Vec::new();
    let mut audio = Vec::new();
    let mut section = 0u8;
    for raw in stderr.lines() {
        let line = strip_tag(raw).trim();
        if line.contains("AVFoundation video devices") {
            section = 1;
            continue;
        }
        if line.contains("AVFoundation audio devices") {
            section = 2;
            continue;
        }
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some(close) = rest.find(']') else {
            continue;
        };
        let Ok(index) = rest[..close].trim().parse::<u32>() else {
            continue;
        };
        let name = rest[close + 1..].trim().to_string();
        if name.is_empty() {
            continue;
        }
        match section {
            1 if name.to_ascii_lowercase().starts_with("capture screen") => screens.push(Source {
                id: index.to_string(),
                name,
                kind: SourceKind::Screen,
            }),
            2 => audio.push(Source {
                id: index.to_string(),
                name,
                kind: SourceKind::Audio,
            }),
            _ => {}
        }
    }
    (screens, audio)
}

fn quoted(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

pub fn parse_dshow_audio(stderr: &str) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    let mut legacy_audio = false;
    for raw in stderr.lines() {
        let line = strip_tag(raw).trim();
        if line.starts_with("Alternative name") {
            continue;
        }
        if line.starts_with("DirectShow audio devices") {
            legacy_audio = true;
            continue;
        }
        if line.starts_with("DirectShow video devices") {
            legacy_audio = false;
            continue;
        }
        let Some(name) = quoted(line) else {
            continue;
        };
        let tail = line.rsplit('"').next().unwrap_or("").trim();
        let is_audio = if tail.is_empty() {
            legacy_audio
        } else {
            tail.contains("(audio")
        };
        if is_audio && !name.is_empty() && !out.iter().any(|s| s.id == name) {
            out.push(Source {
                id: name.to_string(),
                name: name.to_string(),
                kind: SourceKind::Audio,
            });
        }
    }
    out
}

fn csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_q && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_q = !in_q,
            ',' if !in_q => fields.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

pub fn parse_tasklist_windows(csv: &str) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    for line in csv.lines() {
        let f = csv_fields(line.trim());
        let Some(title) = f.last().map(|s| s.trim()) else {
            continue;
        };
        if f.len() < 9 || title.is_empty() || title == "N/A" || title == "OleMainThreadWndName" {
            continue;
        }
        if out.iter().any(|s| s.name == title) {
            continue;
        }
        out.push(Source {
            id: format!("title={title}"),
            name: title.to_string(),
            kind: SourceKind::Window,
        });
    }
    out
}

pub fn parse_pactl_sources(text: &str) -> Vec<Source> {
    text.lines()
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let _index = cols.next()?;
            let name = cols.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(Source {
                id: format!("pulse:{name}"),
                name: name.to_string(),
                kind: SourceKind::Audio,
            })
        })
        .collect()
}

pub fn linux_screen(display: Option<&str>) -> Option<Source> {
    let d = display.map(str::trim).filter(|d| !d.is_empty())?;
    Some(Source {
        id: d.to_string(),
        name: format!("Display {d}"),
        kind: SourceKind::Screen,
    })
}

async fn run_text(program: &str, args: &[&str]) -> Option<(String, String)> {
    let out = crate::core::process::command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .ok()?;
    Some((
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

pub async fn list_sources() -> Sources {
    let platform = Platform::current();
    let ffmpeg = crate::core::ffmpeg::is_ffmpeg_available().await;
    let mut sources = Sources {
        platform,
        ffmpeg,
        screens: Vec::new(),
        audio: Vec::new(),
        error: None,
    };
    if !ffmpeg {
        sources.error = Some(super::errors::code_message(
            super::errors::NO_FFMPEG,
            "FFmpeg was not found.",
        ));
        return sources;
    }
    match platform {
        Platform::Macos => {
            let probe = run_text(
                "ffmpeg",
                &[
                    "-hide_banner",
                    "-f",
                    "avfoundation",
                    "-list_devices",
                    "true",
                    "-i",
                    "",
                ],
            )
            .await;
            if let Some((_, err)) = probe {
                let (screens, audio) = parse_avfoundation(&err);
                sources.screens = screens;
                sources.audio = audio;
            }
        }
        Platform::Windows => {
            sources.screens.push(Source {
                id: "desktop".to_string(),
                name: "Desktop".to_string(),
                kind: SourceKind::Screen,
            });
            if let Some((out, _)) = run_text("tasklist", &["/v", "/fo", "csv", "/nh"]).await {
                sources.screens.extend(parse_tasklist_windows(&out));
            }
            if let Some((_, err)) = run_text(
                "ffmpeg",
                &[
                    "-hide_banner",
                    "-list_devices",
                    "true",
                    "-f",
                    "dshow",
                    "-i",
                    "dummy",
                ],
            )
            .await
            {
                sources.audio = parse_dshow_audio(&err);
            }
        }
        Platform::Linux => {
            let display = std::env::var("DISPLAY").ok();
            sources.screens.extend(linux_screen(display.as_deref()));
            if let Some((out, _)) = run_text("pactl", &["list", "short", "sources"]).await {
                sources.audio = parse_pactl_sources(&out);
            }
            if sources.audio.is_empty() {
                sources.audio.push(Source {
                    id: "alsa:default".to_string(),
                    name: "default".to_string(),
                    kind: SourceKind::Audio,
                });
            }
        }
        Platform::Other => {}
    }
    if sources.screens.is_empty() && sources.error.is_none() {
        let code = if platform == Platform::Linux {
            super::errors::NO_DISPLAY
        } else {
            super::errors::NO_SCREENS
        };
        sources.error = Some(super::errors::code_message(
            code,
            "No screen could be listed for recording.",
        ));
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "[AVFoundation indev @ 0xa3b010140] AVFoundation video devices:
[AVFoundation indev @ 0xa3b010140] [0] MacBook Air Camera
[AVFoundation indev @ 0xa3b010140] [1] MacBook Air Desk View Camera
[AVFoundation indev @ 0xa3b010140] [2] Capture screen 0
[AVFoundation indev @ 0xa3b010140] [3] Capture screen 1
[AVFoundation indev @ 0xa3b010140] AVFoundation audio devices:
[AVFoundation indev @ 0xa3b010140] [0] Microsoft Teams Audio
[AVFoundation indev @ 0xa3b010140] [1] MacBook Air Microphone
[AVFoundation indev @ 0xa3b010140] [2] CORSAIR VIRTUOSO XT Wireless Gaming Receiver
[in#0 @ 0xa3b010000] Error opening input: Input/output error
Error opening input file .";

    #[test]
    fn parses_mac_screens_and_mics() {
        let (screens, audio) = parse_avfoundation(MAC);
        let ids: Vec<&str> = screens.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["2", "3"]);
        assert_eq!(screens[0].name, "Capture screen 0");
        assert_eq!(audio.len(), 3);
        assert_eq!(audio[1].id, "1");
        assert_eq!(audio[1].name, "MacBook Air Microphone");
        assert!(audio.iter().all(|a| a.kind == SourceKind::Audio));
    }

    #[test]
    fn parses_dshow_new_format() {
        let err = r#"[dshow @ 000001] "Integrated Camera" (video)
[dshow @ 000001]   Alternative name "@device_pnp_\\?\usb#vid"
[dshow @ 000001] "Microphone (Realtek(R) Audio)" (audio)
[dshow @ 000001]   Alternative name "@device_cm_{33D9A762}\wave_{1}"
[dshow @ 000001] "OBS Virtual Camera" (none)
dummy: Immediate exit requested"#;
        let audio = parse_dshow_audio(err);
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].id, "Microphone (Realtek(R) Audio)");
    }

    #[test]
    fn parses_dshow_legacy_format() {
        let err = r#"[dshow @ 0x1] DirectShow video devices (some may be both video and audio devices)
[dshow @ 0x1]  "Integrated Camera"
[dshow @ 0x1]     Alternative name "@device_pnp_x"
[dshow @ 0x1] DirectShow audio devices
[dshow @ 0x1]  "Microphone Array (Intel)"
[dshow @ 0x1]     Alternative name "@device_cm_y""#;
        let audio = parse_dshow_audio(err);
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].name, "Microphone Array (Intel)");
    }

    #[test]
    fn parses_window_titles_from_tasklist() {
        let csv = r#""System Idle Process","0","Services","0","8 K","Unknown","NT AUTHORITY\SYSTEM","10:00:00","N/A"
"chrome.exe","1234","Console","1","200,000 K","Running","PC\me","0:01:00","Docs - ""Plan"" - Chrome"
"chrome.exe","1240","Console","1","90,000 K","Running","PC\me","0:00:10","N/A"
"Code.exe","999","Console","1","300,000 K","Running","PC\me","0:05:00","main.rs - omniget""#;
        let w = parse_tasklist_windows(csv);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].name, "Docs - \"Plan\" - Chrome");
        assert_eq!(w[0].id, "title=Docs - \"Plan\" - Chrome");
        assert_eq!(w[1].kind, SourceKind::Window);
    }

    #[test]
    fn parses_pactl_and_display() {
        let text = "0\talsa_output.pci.analog-stereo.monitor\tmodule-alsa-card.c\ts16le 2ch 44100Hz\tSUSPENDED\n1\talsa_input.usb-mic\tmodule-alsa-card.c\ts16le 1ch 48000Hz\tRUNNING\n";
        let s = parse_pactl_sources(text);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].id, "pulse:alsa_input.usb-mic");
        assert_eq!(linux_screen(Some(":0")).unwrap().id, ":0");
        assert!(linux_screen(Some(" ")).is_none());
        assert!(linux_screen(None).is_none());
    }
}
