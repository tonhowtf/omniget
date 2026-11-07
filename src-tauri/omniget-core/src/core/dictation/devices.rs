use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub index: Option<u32>,
}

fn after_prefix(line: &str) -> &str {
    match line.find("] ") {
        Some(i) if line.trim_start().starts_with('[') => &line[i + 2..],
        _ => line,
    }
}

pub fn parse_avfoundation(stderr: &str) -> Vec<Device> {
    let mut in_audio = false;
    let mut out = Vec::new();
    for raw in stderr.lines() {
        let line = after_prefix(raw).trim();
        if line.contains("AVFoundation audio devices") {
            in_audio = true;
            continue;
        }
        if line.contains("AVFoundation video devices") {
            in_audio = false;
            continue;
        }
        if !in_audio {
            continue;
        }
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((num, name)) = rest.split_once(']') else {
            continue;
        };
        let Ok(index) = num.trim().parse::<u32>() else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        out.push(Device {
            id: name.to_string(),
            name: name.to_string(),
            index: Some(index),
        });
    }
    out
}

fn quoted(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let end = line[start + 1..].find('"')? + start + 1;
    Some(&line[start + 1..end])
}

pub fn parse_dshow(stderr: &str) -> Vec<Device> {
    let mut section_audio = false;
    let mut out: Vec<Device> = Vec::new();
    for raw in stderr.lines() {
        let line = after_prefix(raw).trim();
        if line.starts_with("DirectShow audio devices") {
            section_audio = true;
            continue;
        }
        if line.starts_with("DirectShow video devices") {
            section_audio = false;
            continue;
        }
        if line.starts_with("Alternative name") {
            continue;
        }
        let Some(name) = quoted(line) else {
            continue;
        };
        let tail = line.rsplit('"').next().unwrap_or("").trim();
        let audio = match tail {
            "(audio)" => true,
            "(video)" | "(audio, video)" | "(video, audio)" => tail.contains("audio"),
            "" => section_audio,
            _ => false,
        };
        if audio && !name.is_empty() && !out.iter().any(|d| d.name == name) {
            out.push(Device {
                id: name.to_string(),
                name: name.to_string(),
                index: None,
            });
        }
    }
    out
}

pub fn parse_pactl_sources(stdout: &str) -> Vec<Device> {
    stdout
        .lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            let index = cols.first()?.trim().parse::<u32>().ok()?;
            let id = cols.get(1)?.trim();
            if id.is_empty() || id.ends_with(".monitor") {
                return None;
            }
            Some(Device {
                id: id.to_string(),
                name: pretty_pulse_name(id),
                index: Some(index),
            })
        })
        .collect()
}

fn pretty_pulse_name(id: &str) -> String {
    let core = id
        .strip_prefix("alsa_input.")
        .or_else(|| id.strip_prefix("bluez_input."))
        .or_else(|| id.strip_prefix("bluez_source."))
        .unwrap_or(id);
    core.replace(['_', '-'], " ")
        .split('.')
        .next()
        .unwrap_or(core)
        .trim()
        .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Platform::Mac => "macos",
            Platform::Windows => "windows",
            Platform::Linux => "linux",
        }
    }
}

pub fn list_args(platform: Platform) -> Vec<String> {
    let args: &[&str] = match platform {
        Platform::Mac => &[
            "-hide_banner",
            "-nostdin",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ],
        Platform::Windows => &[
            "-hide_banner",
            "-nostdin",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ],
        Platform::Linux => &[],
    };
    args.iter().map(|s| s.to_string()).collect()
}

pub async fn list(ffmpeg: &std::path::Path) -> anyhow::Result<Vec<Device>> {
    let platform = Platform::current();
    if platform == Platform::Linux {
        let out = crate::core::process::command("pactl")
            .args(["list", "short", "sources"])
            .stdin(std::process::Stdio::null())
            .output()
            .await;
        return Ok(match out {
            Ok(o) if o.status.success() => parse_pactl_sources(&String::from_utf8_lossy(&o.stdout)),
            _ => Vec::new(),
        });
    }
    let out = crate::core::process::command(ffmpeg)
        .args(list_args(platform))
        .stdin(std::process::Stdio::null())
        .output()
        .await?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    Ok(match platform {
        Platform::Mac => parse_avfoundation(&stderr),
        _ => parse_dshow(&stderr),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "[AVFoundation indev @ 0xa29010140] AVFoundation video devices:
[AVFoundation indev @ 0xa29010140] [0] MacBook Air Camera
[AVFoundation indev @ 0xa29010140] [1] MacBook Air Desk View Camera
[AVFoundation indev @ 0xa29010140] [2] Capture screen 0
[AVFoundation indev @ 0xa29010140] AVFoundation audio devices:
[AVFoundation indev @ 0xa29010140] [0] Microsoft Teams Audio
[AVFoundation indev @ 0xa29010140] [1] MacBook Air Microphone
[AVFoundation indev @ 0xa29010140] [2] CORSAIR VIRTUOSO XT Wireless Gaming Receiver
[in#0 @ 0xa29010000] Error opening input: Input/output error
Error opening input file .";

    const DSHOW_NEW: &str = r#"[dshow @ 000001d7b8f2e1c0] "Integrated Camera" (video)
[dshow @ 000001d7b8f2e1c0]   Alternative name "@device_pnp_\\?\usb#vid_04f2"
[dshow @ 000001d7b8f2e1c0] "Microphone Array (Realtek(R) Audio)" (audio)
[dshow @ 000001d7b8f2e1c0]   Alternative name "@device_cm_{33D9A762-90C8-11D0-BD43-00A0C911CE86}\wave_{1}"
[dshow @ 000001d7b8f2e1c0] "Headset (Jabra Evolve 65)" (audio)
dummy: Immediate exit requested"#;

    const DSHOW_OLD: &str = r#"[dshow @ 0000000000123] DirectShow video devices (some may be both video and audio devices)
[dshow @ 0000000000123]  "Integrated Webcam"
[dshow @ 0000000000123]     Alternative name "@device_pnp_x"
[dshow @ 0000000000123] DirectShow audio devices
[dshow @ 0000000000123]  "Microphone (Realtek High Definition Audio)"
[dshow @ 0000000000123]     Alternative name "@device_cm_y""#;

    const PACTL: &str = "0\talsa_output.pci-0000_00_1f.3.analog-stereo.monitor\tmodule-alsa-card.c\ts16le 2ch 44100Hz\tSUSPENDED
1\talsa_input.pci-0000_00_1f.3.analog-stereo\tmodule-alsa-card.c\ts16le 2ch 44100Hz\tRUNNING
2\tbluez_input.AA_BB_CC.headset\tmodule-bluez5-device.c\tfloat32le 1ch 16000Hz\tIDLE
";

    #[test]
    fn mac_lists_only_audio_devices() {
        let d = parse_avfoundation(MAC);
        assert_eq!(d.len(), 3);
        assert_eq!(d[1].name, "MacBook Air Microphone");
        assert_eq!(d[1].index, Some(1));
        assert_eq!(d[1].id, "MacBook Air Microphone");
    }

    #[test]
    fn dshow_new_format() {
        let d = parse_dshow(DSHOW_NEW);
        let names: Vec<_> = d.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Microphone Array (Realtek(R) Audio)",
                "Headset (Jabra Evolve 65)"
            ]
        );
    }

    #[test]
    fn dshow_old_format() {
        let d = parse_dshow(DSHOW_OLD);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "Microphone (Realtek High Definition Audio)");
    }

    #[test]
    fn pactl_skips_monitors() {
        let d = parse_pactl_sources(PACTL);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].id, "alsa_input.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(d[0].name, "pci 0000 00 1f");
        assert_eq!(d[1].index, Some(2));
    }

    #[test]
    fn garbage_gives_nothing() {
        assert!(parse_avfoundation("ffmpeg: not found").is_empty());
        assert!(parse_dshow("").is_empty());
        assert!(parse_pactl_sources("hello\tworld").is_empty());
    }
}
