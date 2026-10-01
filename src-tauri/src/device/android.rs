use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Attached {
    pub serial: String,
    pub state: String,
    pub model: String,
}

pub fn parse_devices(out: &str) -> Vec<Attached> {
    out.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let serial = parts.next()?.to_string();
            let state = parts.next()?.to_string();
            let model = parts
                .find_map(|p| p.strip_prefix("model:"))
                .unwrap_or_default()
                .replace('_', " ");
            Some(Attached {
                serial,
                state,
                model,
            })
        })
        .collect()
}

pub fn parse_wm_size(out: &str) -> Option<(u32, u32)> {
    let pick = |prefix: &str| {
        out.lines().find_map(|l| {
            let (w, h) = l.trim().strip_prefix(prefix)?.trim().split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
    };
    pick("Override size:").or_else(|| pick("Physical size:"))
}

pub struct Raw {
    pub w: u32,
    pub h: u32,
    pub offset: usize,
}

pub fn parse_raw_header(b: &[u8]) -> Result<Raw, String> {
    let word =
        |i: usize| -> Option<u32> { Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?)) };
    let (w, h, format) = match (word(0), word(4), word(8)) {
        (Some(w), Some(h), Some(f)) => (w, h, f),
        _ => return Err("DEVICE_FRAME: screencap reply too short".into()),
    };
    if format != 1 && format != 2 {
        return Err(format!(
            "DEVICE_FRAME: screencap pixel format {format} is not RGBA"
        ));
    }
    let px = (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("DEVICE_FRAME: screencap size overflow")?;
    let offset = match b.len().checked_sub(px) {
        Some(o @ (12 | 16)) => o,
        _ => {
            return Err(format!(
                "DEVICE_FRAME: screencap gave {} bytes for {w}x{h}",
                b.len()
            ))
        }
    };
    Ok(Raw { w, h, offset })
}

pub fn parse_bounds(s: &str) -> Option<(f64, f64, f64, f64)> {
    let nums: Vec<f64> = s
        .split(|c: char| !(c.is_ascii_digit() || c == '-'))
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect();
    match nums[..] {
        [x1, y1, x2, y2] => Some((x1, y1, (x2 - x1).max(0.0), (y2 - y1).max(0.0))),
        _ => None,
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
}

fn attrs(tag: &str) -> Vec<(&str, String)> {
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find("=\"") {
        let name = rest[..eq]
            .trim()
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or("");
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        out.push((name, unescape(&after[..end])));
        rest = &after[end + 1..];
    }
    out
}

fn short_class(c: &str) -> String {
    c.rsplit('.').next().unwrap_or(c).to_string()
}

pub fn parse_ui(xml: &str, limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    for chunk in xml.split("<node ").skip(1) {
        if out.len() >= limit {
            break;
        }
        let tag = chunk.split('>').next().unwrap_or_default();
        let a = attrs(tag);
        let get = |k: &str| {
            a.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.as_str())
                .unwrap_or("")
        };
        let label = [get("text"), get("content-desc")]
            .into_iter()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or("")
            .to_string();
        let clickable = get("clickable") == "true";
        let rid = get("resource-id");
        if label.is_empty() && !(clickable && !rid.is_empty()) {
            continue;
        }
        let Some((x, y, w, h)) = parse_bounds(get("bounds")) else {
            continue;
        };
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let mut item = json!({
            "label": label,
            "role": short_class(get("class")),
            "frame": {"x": x, "y": y, "w": w, "h": h},
        });
        if !rid.is_empty() {
            item["id"] = json!(rid.rsplit('/').next().unwrap_or(rid));
        }
        if clickable {
            item["clickable"] = json!(true);
        }
        if get("enabled") == "false" {
            item["enabled"] = json!(false);
        }
        if get("password") == "true" {
            item["label"] = json!("(password)");
        }
        out.push(item);
    }
    out
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn input_text(s: &str) -> Result<String, String> {
    if let Some(c) = s.chars().find(|c| !c.is_ascii() || c.is_ascii_control()) {
        return Err(format!(
            "CAPABILITY_UNAVAILABLE: adb input text types printable ASCII only, '{}' is not",
            c.escape_default()
        ));
    }
    Ok(shell_quote(&s.replace(' ', "%s")))
}

pub fn keycode(name: &str) -> String {
    let n = name.trim();
    let mapped = match n.to_ascii_lowercase().as_str() {
        "enter" | "return" => "66",
        "back" | "escape" | "esc" => "4",
        "home" => "3",
        "backspace" | "delete" | "del" => "67",
        "forward_delete" => "112",
        "tab" => "61",
        "space" => "62",
        "menu" => "82",
        "recent" | "recents" | "app_switch" => "187",
        "up" | "arrowup" => "19",
        "down" | "arrowdown" => "20",
        "left" | "arrowleft" => "21",
        "right" | "arrowright" => "22",
        "power" => "26",
        "volume_up" => "24",
        "volume_down" => "25",
        _ => "",
    };
    if !mapped.is_empty() {
        return mapped.to_string();
    }
    if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() {
        return n.to_string();
    }
    let up = n.to_ascii_uppercase();
    if up.starts_with("KEYCODE_") {
        up
    } else {
        format!("KEYCODE_{up}")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogLine {
    pub time: String,
    pub level: &'static str,
    pub source: String,
    pub pid: Option<u32>,
    pub message: String,
}

pub fn level_of(c: &str) -> &'static str {
    match c {
        "V" => "verbose",
        "D" => "debug",
        "I" => "info",
        "W" => "warning",
        "E" => "error",
        "F" | "A" => "fault",
        _ => "info",
    }
}

pub fn parse_logcat(line: &str) -> Option<LogLine> {
    let mut it = line.split_whitespace();
    let date = it.next()?;
    let clock = it.next()?;
    if !date.contains('-') || !clock.contains(':') {
        return None;
    }
    let pid: u32 = it.next()?.parse().ok()?;
    let _tid = it.next()?;
    let lvl = it.next()?;
    if lvl.len() != 1 {
        return None;
    }
    let head = format!("{lvl} ");
    let rest = &line[line.find(&head)? + 2..];
    let (tag, msg) = match rest.split_once(": ") {
        Some((t, m)) => (t.trim(), m),
        None => (rest.trim_end_matches(':').trim(), ""),
    };
    Some(LogLine {
        time: format!("{date} {clock}"),
        level: level_of(lvl),
        source: tag.to_string(),
        pid: Some(pid),
        message: msg.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_list_reads_state_and_model() {
        let out = "* daemon started successfully\nList of devices attached\nemulator-5560          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\nR58M offline usb:1-1\n\n";
        let d = parse_devices(out);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].serial, "emulator-5560");
        assert_eq!(d[0].state, "device");
        assert_eq!(d[0].model, "sdk gphone64 arm64");
        assert_eq!(d[1].state, "offline");
    }

    #[test]
    fn wm_size_prefers_override() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\n"),
            Some((1080, 2400))
        );
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600\n"),
            Some((720, 1600))
        );
        assert_eq!(parse_wm_size("error"), None);
    }

    #[test]
    fn raw_header_accepts_12_and_16_byte_headers() {
        let mut b = Vec::new();
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend(std::iter::repeat_n(0u8, 24));
        let r = parse_raw_header(&b).unwrap();
        assert_eq!((r.w, r.h, r.offset), (2, 3, 16));
        let short: Vec<u8> = b[..12].iter().chain(&b[16..]).copied().collect();
        assert_eq!(parse_raw_header(&short).unwrap().offset, 12);
        assert!(parse_raw_header(&b[..30]).is_err());
    }

    #[test]
    fn bounds_become_frames() {
        assert_eq!(
            parse_bounds("[28,947][1052,1453]"),
            Some((28.0, 947.0, 1024.0, 506.0))
        );
        assert_eq!(parse_bounds("[1,2]"), None);
    }

    #[test]
    fn ui_dump_keeps_labelled_or_clickable_ids() {
        let xml = r#"<?xml version='1.0' ?><hierarchy rotation="0"><node index="0" text="" resource-id="" class="android.widget.FrameLayout" content-desc="" clickable="false" enabled="true" password="false" bounds="[0,0][1080,2400]"><node index="1" text="Sign in &amp; go" resource-id="com.app:id/login" class="android.widget.Button" content-desc="" clickable="true" enabled="false" password="false" bounds="[40,900][1040,1030]" /><node index="2" text="" resource-id="com.app:id/fab" class="android.widget.ImageButton" content-desc="" clickable="true" enabled="true" password="false" bounds="[900,2100][1040,2240]" /><node index="3" text="secret" resource-id="" class="android.widget.EditText" content-desc="" clickable="true" enabled="true" password="true" bounds="[40,700][1040,800]" /></node></hierarchy>"#;
        let t = parse_ui(xml, 50);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0]["label"], "Sign in & go");
        assert_eq!(t[0]["role"], "Button");
        assert_eq!(t[0]["id"], "login");
        assert_eq!(t[0]["enabled"], false);
        assert_eq!(t[0]["frame"]["w"], 1000.0);
        assert_eq!(t[1]["label"], "");
        assert_eq!(t[1]["id"], "fab");
        assert_eq!(t[2]["label"], "(password)");
        assert_eq!(parse_ui(xml, 1).len(), 1);
    }

    #[test]
    fn text_is_quoted_for_the_device_shell() {
        assert_eq!(input_text("it's ok").unwrap(), "'it'\\''s%sok'");
        assert!(input_text("ação").is_err());
        assert_eq!(shell_quote("exp://a?b=1&c"), "'exp://a?b=1&c'");
    }

    #[test]
    fn key_names_map_to_keycodes() {
        assert_eq!(keycode("Enter"), "66");
        assert_eq!(keycode("back"), "4");
        assert_eq!(keycode("82"), "82");
        assert_eq!(keycode("dpad_center"), "KEYCODE_DPAD_CENTER");
        assert_eq!(keycode("KEYCODE_CAMERA"), "KEYCODE_CAMERA");
    }

    #[test]
    fn logcat_threadtime_lines_parse() {
        let l = parse_logcat(
            "10-01 10:50:01.123  1234  1250 E ReactNativeJS: TypeError: x is undefined",
        )
        .unwrap();
        assert_eq!(l.level, "error");
        assert_eq!(l.source, "ReactNativeJS");
        assert_eq!(l.pid, Some(1234));
        assert_eq!(l.message, "TypeError: x is undefined");
        assert!(parse_logcat("--------- beginning of main").is_none());
    }
}
