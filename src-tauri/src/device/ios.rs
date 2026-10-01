use serde_json::Value;

use super::android::LogLine;

#[derive(Debug, Clone, PartialEq)]
pub struct Sim {
    pub udid: String,
    pub name: String,
    pub state: String,
    pub os: String,
    pub type_id: String,
}

pub fn runtime_label(key: &str) -> String {
    let tail = key.rsplit('.').next().unwrap_or(key);
    match tail.split_once('-') {
        Some((os, ver)) => format!("{os} {}", ver.replace('-', ".")),
        None => tail.to_string(),
    }
}

fn version_key(os: &str) -> Vec<u32> {
    os.split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .collect()
}

pub fn parse_list(v: &Value) -> Vec<Sim> {
    let mut out = Vec::new();
    let Some(map) = v["devices"].as_object() else {
        return out;
    };
    for (runtime, list) in map {
        let os = runtime_label(runtime);
        for d in list.as_array().into_iter().flatten() {
            if d["isAvailable"] == Value::Bool(false) {
                continue;
            }
            let (Some(udid), Some(name)) = (d["udid"].as_str(), d["name"].as_str()) else {
                continue;
            };
            out.push(Sim {
                udid: udid.to_string(),
                name: name.to_string(),
                state: d["state"].as_str().unwrap_or("Shutdown").to_string(),
                os: os.clone(),
                type_id: d["deviceTypeIdentifier"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    out.sort_by(|a, b| {
        let booted = |s: &Sim| s.state == "Booted";
        let iphone = |s: &Sim| s.name.starts_with("iPhone");
        booted(b)
            .cmp(&booted(a))
            .then(iphone(b).cmp(&iphone(a)))
            .then(version_key(&b.os).cmp(&version_key(&a.os)))
            .then(a.name.cmp(&b.name))
    });
    out
}

pub fn pick<'a>(sims: &'a [Sim], want: Option<&str>) -> Option<&'a Sim> {
    match want.map(str::trim).filter(|w| !w.is_empty()) {
        None => sims
            .iter()
            .find(|s| s.os.starts_with("iOS"))
            .or(sims.first()),
        Some(w) => sims
            .iter()
            .find(|s| s.udid.eq_ignore_ascii_case(w))
            .or_else(|| sims.iter().find(|s| s.name.eq_ignore_ascii_case(w))),
    }
}

pub fn bundle_path(devicetypes: &Value, type_id: &str) -> Option<String> {
    devicetypes["devicetypes"]
        .as_array()?
        .iter()
        .find(|t| t["identifier"] == type_id)?["bundlePath"]
        .as_str()
        .map(str::to_string)
}

fn level_of(code: &str) -> &'static str {
    match code {
        "Db" => "debug",
        "In" => "info",
        "Df" | "A" => "info",
        "E" => "error",
        "F" => "fault",
        _ => "info",
    }
}

pub fn parse_log(line: &str) -> Option<LogLine> {
    let mut it = line.splitn(4, ' ');
    let date = it.next()?;
    let clock = it.next()?;
    if !date.contains('-') || !clock.contains(':') {
        return None;
    }
    let rest = line[date.len() + clock.len() + 2..].trim_start();
    let (code, rest) = rest.split_once(' ')?;
    if code.len() > 2 {
        return None;
    }
    let rest = rest.trim_start();
    let (proc_part, msg) = rest.split_once(' ').unwrap_or((rest, ""));
    let (source, pid) = match proc_part.split_once('[') {
        Some((p, ids)) => (
            p.to_string(),
            ids.split([':', ']']).next().and_then(|n| n.parse().ok()),
        ),
        None => (proc_part.to_string(), None),
    };
    let msg = msg.trim_start();
    let msg = match msg.strip_prefix('[') {
        Some(m) if m.contains("] ") => m.split_once("] ").map(|(_, r)| r).unwrap_or(msg),
        _ => msg,
    };
    Some(LogLine {
        time: format!("{date} {clock}"),
        level: level_of(code),
        source,
        pid,
        message: msg.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtimes_read_as_os_versions() {
        assert_eq!(
            runtime_label("com.apple.CoreSimulator.SimRuntime.iOS-26-5"),
            "iOS 26.5"
        );
        assert_eq!(
            runtime_label("com.apple.CoreSimulator.SimRuntime.watchOS-11-0"),
            "watchOS 11.0"
        );
    }

    #[test]
    fn list_puts_booted_iphones_on_newest_os_first() {
        let v = json!({"devices": {
            "com.apple.CoreSimulator.SimRuntime.iOS-18-0": [
                {"udid": "OLD", "name": "iPhone 16", "state": "Shutdown", "isAvailable": true, "deviceTypeIdentifier": "t16"}
            ],
            "com.apple.CoreSimulator.SimRuntime.iOS-26-5": [
                {"udid": "PAD", "name": "iPad Air", "state": "Shutdown", "isAvailable": true},
                {"udid": "NEW", "name": "iPhone 17", "state": "Shutdown", "isAvailable": true, "deviceTypeIdentifier": "t17"},
                {"udid": "GONE", "name": "iPhone 15", "state": "Shutdown", "isAvailable": false},
                {"udid": "ON", "name": "iPhone Air", "state": "Booted", "isAvailable": true}
            ]
        }});
        let l = parse_list(&v);
        let ids: Vec<&str> = l.iter().map(|s| s.udid.as_str()).collect();
        assert_eq!(ids, vec!["ON", "NEW", "OLD", "PAD"]);
        assert_eq!(l[1].os, "iOS 26.5");
        assert_eq!(pick(&l, None).unwrap().udid, "ON");
        assert_eq!(pick(&l, Some("iphone 17")).unwrap().udid, "NEW");
        assert_eq!(pick(&l, Some("old")).unwrap().udid, "OLD");
        assert!(pick(&l, Some("Pixel")).is_none());
    }

    #[test]
    fn device_type_bundle_is_found() {
        let v = json!({"devicetypes": [{"identifier": "t17", "bundlePath": "/x/iPhone 17.simdevicetype"}]});
        assert_eq!(
            bundle_path(&v, "t17").as_deref(),
            Some("/x/iPhone 17.simdevicetype")
        );
        assert_eq!(bundle_path(&v, "nope"), None);
    }

    #[test]
    fn compact_log_lines_parse() {
        let l =
            parse_log("2026-10-01 10:50:01.123 E  MyApp[812:9031] [com.app:net] request failed")
                .unwrap();
        assert_eq!(l.level, "error");
        assert_eq!(l.source, "MyApp");
        assert_eq!(l.pid, Some(812));
        assert_eq!(l.message, "request failed");
        let d = parse_log("2026-10-01 10:50:02.000 Df SpringBoard[60:61] hello").unwrap();
        assert_eq!(d.level, "info");
        assert_eq!(d.message, "hello");
        assert!(parse_log("Filtering the log data using \"process == x\"").is_none());
        assert!(parse_log("Timestamp               Ty Process[PID:TID]").is_none());
    }
}
