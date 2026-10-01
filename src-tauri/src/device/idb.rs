use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use serde_json::{json, Value};

pub const SERVICE: &str = "idb.CompanionService";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hid {
    Touch {
        x: f64,
        y: f64,
        up: bool,
    },
    Button {
        code: u64,
        up: bool,
    },
    Key {
        code: u64,
        up: bool,
    },
    Swipe {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        secs: f64,
    },
    Delay(f64),
}

pub const BUTTON_HOME: u64 = 1;
pub const BUTTON_LOCK: u64 = 2;
pub const KEY_SHIFT: u64 = 225;

pub mod pb {
    pub fn varint(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn key(out: &mut Vec<u8>, field: u32, wire: u8) {
        varint(out, (u64::from(field) << 3) | u64::from(wire));
    }

    pub fn double(out: &mut Vec<u8>, field: u32, v: f64) {
        key(out, field, 1);
        out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn uint(out: &mut Vec<u8>, field: u32, v: u64) {
        if v == 0 {
            return;
        }
        key(out, field, 0);
        varint(out, v);
    }

    pub fn bytes(out: &mut Vec<u8>, field: u32, m: &[u8]) {
        key(out, field, 2);
        varint(out, m.len() as u64);
        out.extend_from_slice(m);
    }

    #[derive(Debug, PartialEq)]
    pub enum Field<'a> {
        Varint(u64),
        Fixed64([u8; 8]),
        Bytes(&'a [u8]),
        Fixed32([u8; 4]),
    }

    fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *buf.get(*pos)?;
            *pos += 1;
            v |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return Some(v);
            }
        }
        None
    }

    pub fn fields(buf: &[u8]) -> Option<Vec<(u32, Field<'_>)>> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos < buf.len() {
            let k = read_varint(buf, &mut pos)?;
            let field = u32::try_from(k >> 3).ok()?;
            let f = match k & 7 {
                0 => Field::Varint(read_varint(buf, &mut pos)?),
                1 => {
                    let b: [u8; 8] = buf.get(pos..pos + 8)?.try_into().ok()?;
                    pos += 8;
                    Field::Fixed64(b)
                }
                2 => {
                    let len = usize::try_from(read_varint(buf, &mut pos)?).ok()?;
                    let b = buf.get(pos..pos.checked_add(len)?)?;
                    pos += len;
                    Field::Bytes(b)
                }
                5 => {
                    let b: [u8; 4] = buf.get(pos..pos + 4)?.try_into().ok()?;
                    pos += 4;
                    Field::Fixed32(b)
                }
                _ => return None,
            };
            out.push((field, f));
        }
        Some(out)
    }

    pub fn string(buf: &[u8], field: u32) -> Option<String> {
        fields(buf)?.into_iter().find_map(|(n, f)| match f {
            Field::Bytes(b) if n == field => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        })
    }
}

fn point(x: f64, y: f64) -> Vec<u8> {
    let mut p = Vec::new();
    pb::double(&mut p, 1, x);
    pb::double(&mut p, 2, y);
    p
}

fn press(action_field: u32, action: &[u8], up: bool) -> Vec<u8> {
    let mut pa = Vec::new();
    pb::bytes(&mut pa, action_field, action);
    let mut pr = Vec::new();
    pb::bytes(&mut pr, 1, &pa);
    pb::uint(&mut pr, 2, u64::from(up));
    let mut ev = Vec::new();
    pb::bytes(&mut ev, 1, &pr);
    ev
}

pub fn encode(h: &Hid) -> Vec<u8> {
    match *h {
        Hid::Touch { x, y, up } => {
            let mut t = Vec::new();
            pb::bytes(&mut t, 1, &point(x, y));
            press(1, &t, up)
        }
        Hid::Button { code, up } => {
            let mut b = Vec::new();
            pb::uint(&mut b, 1, code);
            press(2, &b, up)
        }
        Hid::Key { code, up } => {
            let mut k = Vec::new();
            pb::uint(&mut k, 1, code);
            press(3, &k, up)
        }
        Hid::Swipe {
            x1,
            y1,
            x2,
            y2,
            secs,
        } => {
            let mut s = Vec::new();
            pb::bytes(&mut s, 1, &point(x1, y1));
            pb::bytes(&mut s, 2, &point(x2, y2));
            pb::double(&mut s, 5, 10.0);
            pb::double(&mut s, 6, secs);
            let mut ev = Vec::new();
            pb::bytes(&mut ev, 2, &s);
            ev
        }
        Hid::Delay(secs) => {
            let mut d = Vec::new();
            pb::double(&mut d, 1, secs);
            let mut ev = Vec::new();
            pb::bytes(&mut ev, 3, &d);
            ev
        }
    }
}

pub fn tap(x: f64, y: f64) -> Vec<Hid> {
    vec![
        Hid::Touch { x, y, up: false },
        Hid::Delay(0.05),
        Hid::Touch { x, y, up: true },
    ]
}

pub fn button(code: u64) -> Vec<Hid> {
    vec![
        Hid::Button { code, up: false },
        Hid::Delay(0.05),
        Hid::Button { code, up: true },
    ]
}

pub fn keycode(c: char) -> Option<(u64, bool)> {
    const PLAIN: &str = "-=[]\\;'`,./";
    const PLAIN_CODES: [u64; 11] = [45, 46, 47, 48, 49, 51, 52, 53, 54, 55, 56];
    const SHIFTED: &str = "_+{}|:\"~<>?";
    const SHIFTED_CODES: [u64; 11] = [45, 46, 47, 48, 49, 51, 52, 53, 54, 55, 56];
    const DIGIT_SHIFT: &str = "!@#$%^&*()";
    Some(match c {
        'a'..='z' => (4 + (c as u64 - 'a' as u64), false),
        'A'..='Z' => (4 + (c as u64 - 'A' as u64), true),
        '1'..='9' => (30 + (c as u64 - '1' as u64), false),
        '0' => (39, false),
        '\n' => (40, false),
        '\t' => (43, false),
        ' ' => (44, false),
        _ => {
            if let Some(i) = DIGIT_SHIFT.find(c) {
                (30 + i as u64, true)
            } else if let Some(i) = SHIFTED.find(c) {
                (SHIFTED_CODES[i], true)
            } else {
                (PLAIN_CODES[PLAIN.find(c)?], false)
            }
        }
    })
}

pub fn text(s: &str) -> Result<Vec<Hid>, String> {
    let mut out = Vec::new();
    for c in s.chars() {
        let (code, shift) = keycode(c).ok_or_else(|| {
            format!("CAPABILITY_UNAVAILABLE: iOS text input types US keyboard characters only, '{c}' is not one")
        })?;
        if shift {
            out.push(Hid::Key {
                code: KEY_SHIFT,
                up: false,
            });
        }
        out.push(Hid::Key { code, up: false });
        out.push(Hid::Key { code, up: true });
        if shift {
            out.push(Hid::Key {
                code: KEY_SHIFT,
                up: true,
            });
        }
    }
    Ok(out)
}

pub fn named_key(name: &str) -> Option<u64> {
    Some(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => 40,
        "escape" | "esc" => 41,
        "backspace" | "delete" | "del" => 42,
        "tab" => 43,
        "space" => 44,
        "right" | "arrowright" => 79,
        "left" | "arrowleft" => 80,
        "down" | "arrowdown" => 81,
        "up" | "arrowup" => 82,
        n => return n.parse().ok(),
    })
}

fn frame_messages(buf: &mut Vec<u8>) -> Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::new();
    loop {
        if buf.len() < 5 {
            return Ok(out);
        }
        if buf[0] != 0 {
            return Err("IDB: compressed gRPC replies are not supported".into());
        }
        let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        if buf.len() < 5 + len {
            return Ok(out);
        }
        out.push(buf[5..5 + len].to_vec());
        buf.drain(..5 + len);
    }
}

fn grpc_frame(msg: &[u8]) -> Bytes {
    let mut f = Vec::with_capacity(msg.len() + 5);
    f.push(0);
    f.extend_from_slice(&(msg.len() as u32).to_be_bytes());
    f.extend_from_slice(msg);
    Bytes::from(f)
}

fn status_of(h: &http::HeaderMap) -> Option<Result<(), String>> {
    let code = h.get("grpc-status")?.to_str().ok()?.to_string();
    if code == "0" {
        return Some(Ok(()));
    }
    let msg = h
        .get("grpc-message")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    Some(Err(format!("IDB: status {code} {msg}")))
}

#[cfg(unix)]
async fn connect(sock: &Path) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(sock).await
}

#[cfg(not(unix))]
async fn connect(_sock: &Path) -> std::io::Result<tokio::io::DuplexStream> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "idb_companion runs on macOS only",
    ))
}

pub async fn alive(sock: &Path) -> bool {
    connect(sock).await.is_ok()
}

pub async fn call(sock: &Path, method: &str, msgs: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, String> {
    let io = connect(sock)
        .await
        .map_err(|e| format!("IDB: connect: {e}"))?;
    let (send, conn) = h2::client::handshake(io)
        .await
        .map_err(|e| format!("IDB: handshake: {e}"))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut send = send.ready().await.map_err(|e| format!("IDB: {e}"))?;
    let req = http::Request::builder()
        .method("POST")
        .uri(format!("http://localhost/{SERVICE}/{method}"))
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(())
        .map_err(|e| format!("IDB: {e}"))?;
    let (resp, mut stream) = send
        .send_request(req, false)
        .map_err(|e| format!("IDB: send: {e}"))?;
    for m in msgs {
        stream
            .send_data(grpc_frame(m), false)
            .map_err(|e| format!("IDB: send: {e}"))?;
    }
    stream
        .send_data(Bytes::new(), true)
        .map_err(|e| format!("IDB: send: {e}"))?;
    let resp = resp.await.map_err(|e| format!("IDB: reply: {e}"))?;
    if let Some(Err(e)) = status_of(resp.headers()) {
        return Err(e);
    }
    let mut body = resp.into_body();
    let mut buf = Vec::new();
    let mut out = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(|e| format!("IDB: read: {e}"))?;
        let _ = body.flow_control().release_capacity(chunk.len());
        buf.extend_from_slice(&chunk);
        out.extend(frame_messages(&mut buf)?);
    }
    if let Ok(Some(t)) = body.trailers().await {
        if let Some(Err(e)) = status_of(&t) {
            return Err(e);
        }
    }
    Ok(out)
}

pub async fn hid(sock: &Path, events: &[Hid]) -> Result<(), String> {
    let msgs: Vec<Vec<u8>> = events.iter().map(encode).collect();
    let secs: f64 = events
        .iter()
        .map(|e| match e {
            Hid::Delay(s) => *s,
            Hid::Swipe { secs, .. } => *secs,
            _ => 0.01,
        })
        .sum();
    let limit = Duration::from_secs_f64(secs.max(0.0) + 10.0);
    tokio::time::timeout(limit, call(sock, "hid", &msgs))
        .await
        .map_err(|_| "IDB: hid timed out".to_string())??;
    Ok(())
}

pub async fn accessibility(sock: &Path) -> Result<Value, String> {
    let r = tokio::time::timeout(
        Duration::from_secs(15),
        call(sock, "accessibility_info", &[Vec::new()]),
    )
    .await
    .map_err(|_| "IDB: accessibility_info timed out".to_string())??;
    let raw = r
        .first()
        .and_then(|m| pb::string(m, 1))
        .ok_or("IDB: empty accessibility reply")?;
    serde_json::from_str(&raw).map_err(|e| format!("IDB: accessibility json: {e}"))
}

fn num(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or(0.0)
}

pub fn trim_tree(v: &Value, limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    let mut stack: Vec<&Value> = match v {
        Value::Array(a) => a.iter().rev().collect(),
        other => vec![other],
    };
    while let Some(e) = stack.pop() {
        if out.len() >= limit {
            break;
        }
        if let Some(kids) = e["children"].as_array() {
            stack.extend(kids.iter().rev());
        }
        let label = ["AXLabel", "AXValue", "title", "AXUniqueId"]
            .iter()
            .find_map(|k| e[*k].as_str().filter(|s| !s.trim().is_empty()))
            .unwrap_or("")
            .trim()
            .to_string();
        let role = e["role_description"]
            .as_str()
            .or(e["type"].as_str())
            .or(e["role"].as_str())
            .unwrap_or("")
            .to_string();
        let f = &e["frame"];
        let (w, h) = (num(f, "width"), num(f, "height"));
        if label.is_empty() || w <= 0.0 || h <= 0.0 {
            continue;
        }
        let mut item = json!({
            "label": label,
            "role": role,
            "frame": {"x": num(f, "x").round(), "y": num(f, "y").round(), "w": w.round(), "h": h.round()},
        });
        if let Some(id) = e["AXUniqueId"].as_str().filter(|s| !s.is_empty()) {
            item["id"] = json!(id);
        }
        if e["enabled"] == json!(false) {
            item["enabled"] = json!(false);
        }
        out.push(item);
    }
    out
}

pub fn socket_path(key: &str) -> PathBuf {
    let short: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(16)
        .collect();
    std::env::temp_dir().join(format!("og-idb-{short}.sock"))
}

pub struct Companion {
    pub sock: PathBuf,
    child: tokio::process::Child,
}

impl Drop for Companion {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_file(&self.sock);
    }
}

pub async fn start(bin: &Path, udid: &str, key: &str) -> Result<Companion, String> {
    let sock = socket_path(key);
    let _ = std::fs::remove_file(&sock);
    let child = omniget_core::core::process::command(bin)
        .arg("--udid")
        .arg(udid)
        .arg("--grpc-domain-sock")
        .arg(&sock)
        .arg("--log-level")
        .arg("info")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("IDB: start idb_companion: {e}"))?;
    let comp = Companion { sock, child };
    for _ in 0..100 {
        if alive(&comp.sock).await {
            return Ok(comp);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("IDB: idb_companion did not open its socket in 10 s".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trip() {
        let mut b = Vec::new();
        pb::uint(&mut b, 1, 300);
        assert_eq!(b, vec![0x08, 0xac, 0x02]);
        assert_eq!(pb::fields(&b).unwrap(), vec![(1, pb::Field::Varint(300))]);
    }

    #[test]
    fn touch_up_nests_press_action_and_direction() {
        let ev = encode(&Hid::Touch {
            x: 1.0,
            y: 2.0,
            up: true,
        });
        let top = pb::fields(&ev).unwrap();
        let pb::Field::Bytes(press) = top[0].1 else {
            panic!()
        };
        assert_eq!(top[0].0, 1);
        let pf = pb::fields(press).unwrap();
        assert_eq!(pf[1], (2, pb::Field::Varint(1)));
        let pb::Field::Bytes(action) = pf[0].1 else {
            panic!()
        };
        let af = pb::fields(action).unwrap();
        assert_eq!(af[0].0, 1);
        let pb::Field::Bytes(touch) = af[0].1 else {
            panic!()
        };
        let pb::Field::Bytes(p) = pb::fields(touch).unwrap()[0].1 else {
            panic!()
        };
        assert_eq!(
            pb::fields(p).unwrap(),
            vec![
                (1, pb::Field::Fixed64(1.0f64.to_le_bytes())),
                (2, pb::Field::Fixed64(2.0f64.to_le_bytes()))
            ]
        );
    }

    #[test]
    fn touch_down_omits_default_direction() {
        let ev = encode(&Hid::Touch {
            x: 0.0,
            y: 0.0,
            up: false,
        });
        let pb::Field::Bytes(press) = pb::fields(&ev).unwrap()[0].1 else {
            panic!()
        };
        assert_eq!(pb::fields(press).unwrap().len(), 1);
    }

    #[test]
    fn keycodes_follow_the_us_layout() {
        assert_eq!(keycode('a'), Some((4, false)));
        assert_eq!(keycode('Z'), Some((29, true)));
        assert_eq!(keycode('0'), Some((39, false)));
        assert_eq!(keycode('!'), Some((30, true)));
        assert_eq!(keycode('@'), Some((31, true)));
        assert_eq!(keycode('_'), Some((45, true)));
        assert_eq!(keycode('.'), Some((55, false)));
        assert_eq!(keycode('?'), Some((56, true)));
        assert_eq!(keycode('é'), None);
        assert_eq!(text("Hi").unwrap().len(), 6);
    }

    #[test]
    fn grpc_frames_split_across_chunks() {
        let mut buf = grpc_frame(b"abc").to_vec();
        let tail = buf.split_off(4);
        assert!(frame_messages(&mut buf).unwrap().is_empty());
        buf.extend(tail);
        buf.extend_from_slice(&grpc_frame(b""));
        assert_eq!(
            frame_messages(&mut buf).unwrap(),
            vec![b"abc".to_vec(), Vec::new()]
        );
        assert!(buf.is_empty());
    }

    #[test]
    fn accessibility_tree_is_trimmed_to_labelled_frames() {
        let v = json!([
            {"AXLabel": "Settings", "type": "Application", "frame": {"x": 0, "y": 0, "width": 402, "height": 874}},
            {"AXLabel": null, "type": "Group", "frame": {"x": 0, "y": 0, "width": 10, "height": 10}},
            {"AXLabel": "Sign in", "AXUniqueId": "login", "type": "Button", "role_description": "button", "enabled": false,
             "frame": {"x": 20.4, "y": 300, "width": 100, "height": 44}}
        ]);
        let t = trim_tree(&v, 10);
        assert_eq!(t.len(), 2);
        assert_eq!(t[1]["label"], "Sign in");
        assert_eq!(t[1]["role"], "button");
        assert_eq!(t[1]["id"], "login");
        assert_eq!(t[1]["frame"]["x"], 20.0);
        assert_eq!(t[1]["enabled"], false);
    }
}
