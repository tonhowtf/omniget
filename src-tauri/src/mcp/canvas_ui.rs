use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::oneshot;

pub const EVENT_REPLY: &str = "mcp-ui://reply";

pub const TEST_TOOLS_ENV: &str = "OMNIGET_TEST_TOOLS";

pub fn test_tools_enabled() -> bool {
    std::env::var(TEST_TOOLS_ENV).ok().as_deref() == Some("1")
}

type Pending = HashMap<String, oneshot::Sender<Value>>;

fn pending() -> &'static Mutex<Pending> {
    static P: OnceLock<Mutex<Pending>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn listen_replies(app: &AppHandle) {
    static ON: OnceLock<()> = OnceLock::new();
    ON.get_or_init(|| {
        app.listen_any(EVENT_REPLY, |ev| {
            let Ok(v) = serde_json::from_str::<Value>(ev.payload()) else {
                return;
            };
            let Some(id) = v["id"].as_str() else { return };
            if let Some(tx) = pending()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(id)
            {
                let _ = tx.send(v["value"].clone());
            }
        });
    });
}

fn js_literal(v: &Value) -> String {
    serde_json::to_string(v)
        .unwrap_or_else(|_| "null".into())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

const RUNNER: &str = r#"async (op, a) => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const mac = /Mac|iPhone|iPad/.test(navigator.platform);
  const drv = window.__omnigetCanvasDriver;
  const names = { enter: 'Enter', return: 'Enter', escape: 'Escape', esc: 'Escape', tab: 'Tab', space: ' ',
    backspace: 'Backspace', delete: 'Delete', del: 'Delete', up: 'ArrowUp', down: 'ArrowDown', left: 'ArrowLeft',
    right: 'ArrowRight', home: 'Home', end: 'End', pageup: 'PageUp', pagedown: 'PageDown', plus: '+', minus: '-' };
  const parse = (combo) => {
    const parts = String(combo).split('+').map((p) => p.trim()).filter(Boolean);
    const m = { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false };
    let key = '';
    for (const raw of parts) {
      const p = raw.toLowerCase();
      if (p === 'cmd' || p === 'command' || p === 'primary' || p === 'mod') { if (mac) m.metaKey = true; else m.ctrlKey = true; }
      else if (p === 'meta' || p === 'super' || p === 'win') m.metaKey = true;
      else if (p === 'ctrl' || p === 'control') m.ctrlKey = true;
      else if (p === 'alt' || p === 'option' || p === 'opt') m.altKey = true;
      else if (p === 'shift') m.shiftKey = true;
      else key = names[p] ?? (/^f\d{1,2}$/.test(p) ? p.toUpperCase() : raw.length === 1 ? raw : raw);
    }
    if (!key && parts.length) key = parts[parts.length - 1];
    let code = key;
    if (key.length === 1 && /[a-z]/i.test(key)) code = 'Key' + key.toUpperCase();
    else if (key.length === 1 && /\d/.test(key)) code = 'Digit' + key;
    else if (key === ' ') code = 'Space';
    return { key, code, ...m };
  };
  const target = () => {
    const el = document.activeElement;
    return el && el !== document.body ? el : window;
  };
  const describe = (el) => {
    if (!el || el === document.body) return null;
    const node = el.closest ? el.closest('[data-node-id]') : null;
    return { tag: el.tagName.toLowerCase(), cls: String(el.className || '').slice(0, 80), label: el.getAttribute('aria-label'), nodeId: node ? node.dataset.nodeId : null };
  };
  const mods = (s) => {
    const r = { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false };
    for (const p of String(s || '').toLowerCase().split('+').map((x) => x.trim()).filter(Boolean)) {
      if (p === 'cmd' || p === 'primary' || p === 'mod') { if (mac) r.metaKey = true; else r.ctrlKey = true; }
      else if (p === 'meta') r.metaKey = true;
      else if (p === 'ctrl') r.ctrlKey = true;
      else if (p === 'alt' || p === 'option') r.altKey = true;
      else if (p === 'shift') r.shiftKey = true;
    }
    return r;
  };
  const point = async (v) => {
    if (!v) return null;
    if (typeof v === 'object' && Number.isFinite(v.x) && Number.isFinite(v.y)) return { x: v.x, y: v.y };
    if (Array.isArray(v) && v.length === 2) return { x: +v[0], y: +v[1] };
    const id = typeof v === 'string' ? v : v.nodeId;
    if (id && drv) {
      const r = drv.nodeRect(id);
      if (r) return { x: r.x + r.w / 2, y: r.y + Math.min(r.h / 2, 14) };
    }
    return null;
  };
  const fire = (type, p, extra) => {
    const el = document.elementFromPoint(p.x, p.y) || document.body;
    const base = { bubbles: true, cancelable: true, composed: true, clientX: p.x, clientY: p.y, view: window, ...extra };
    if (type.startsWith('pointer')) el.dispatchEvent(new PointerEvent(type, { pointerId: 1, pointerType: 'mouse', isPrimary: true, ...base }));
    else if (type === 'wheel') el.dispatchEvent(new WheelEvent('wheel', base));
    else el.dispatchEvent(new MouseEvent(type, base));
    return el;
  };
  const guard = () => {
    const want = a.expectWorkspaceId;
    if (!want) return null;
    if (!drv) return { error: 'CANVAS_NOT_OPEN: the window is not on the canvas (expectWorkspaceId ' + want + ')' };
    const w = drv.where();
    if (w.workspaceId !== want) return { error: 'WINDOW_MOVED: the window shows ' + w.workspaceId + ', not ' + want + ' (workspace_activate it again, or drop expectWorkspaceId)' };
    return null;
  };
  const hitAt = (p) => (drv && drv.nodeAt ? drv.nodeAt(p.x, p.y) : null);
  const hitView = (h) => (h ? { nodeId: h.nodeId, title: h.title, kind: h.kind } : null);
  switch (op) {
    case 'ping':
      return { ok: true, route: location.pathname, driver: !!drv };
    case 'state': {
      const base = { route: location.pathname, title: document.title, focusElement: describe(document.activeElement), canvas: !!drv };
      if (!drv) return base;
      const c = await drv.run('state', {});
      return { ...base, ...c };
    }
    case 'key': {
      const blocked = guard();
      if (blocked) return blocked;
      const seq = Array.isArray(a.keys) ? a.keys : String(a.keys || '').split(/\s+/).filter(Boolean);
      const times = Math.max(1, Math.min(50, a.repeat || 1));
      for (let t = 0; t < times; t++) {
        for (const combo of seq) {
          const k = parse(combo);
          const init = { key: k.key, code: k.code, bubbles: true, cancelable: true, composed: true, metaKey: k.metaKey, ctrlKey: k.ctrlKey, altKey: k.altKey, shiftKey: k.shiftKey };
          const el = target();
          const down = new KeyboardEvent('keydown', init);
          el.dispatchEvent(down);
          const printable = k.key.length === 1 && !k.metaKey && !k.ctrlKey;
          if (printable && !down.defaultPrevented && el instanceof HTMLElement && (el.isContentEditable || el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) {
            document.execCommand('insertText', false, k.key);
          }
          el.dispatchEvent(new KeyboardEvent('keyup', init));
          await sleep(a.delayMs || 30);
        }
      }
      if (typeof a.text === 'string' && a.text) {
        const el = target();
        if (el instanceof HTMLElement && (el.isContentEditable || el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) document.execCommand('insertText', false, a.text);
        else for (const ch of a.text) { el.dispatchEvent(new KeyboardEvent('keydown', { key: ch, bubbles: true, cancelable: true })); el.dispatchEvent(new KeyboardEvent('keyup', { key: ch, bubbles: true })); }
      }
      await sleep(60);
      return { focusElement: describe(document.activeElement) };
    }
    case 'pointer': {
      const action = a.action || 'click';
      if (a.file && !a.paths) a.paths = [a.file];
      if ((action === 'drop' || action === 'paste') && a.paths != null && (!Array.isArray(a.paths) || a.paths.some((p) => typeof p !== 'string' || !p.startsWith('/')))) return { error: 'INVALID_ARGUMENT: paths must be absolute file paths' };
      const blocked = guard();
      if (blocked) return blocked;
      const at = await point(a.at ?? a.nodeId ?? a.from);
      if (!at) return { error: 'MISSING_ARGUMENT: at {x,y} (client px) or nodeId' };
      const m = mods(a.mods);
      const hit = hitAt(at);
      if (action === 'hover' || action === 'move') {
        fire('pointerover', at, m); fire('pointerenter', at, m); fire('mouseover', at, m); fire('pointermove', at, m); fire('mousemove', at, m);
      } else if (action === 'click' || action === 'dblclick' || action === 'click-hold') {
        const n = action === 'dblclick' ? 2 : 1;
        let el = null;
        for (let i = 0; i < n; i++) {
          fire('pointerdown', at, { ...m, button: 0, buttons: 1, detail: i + 1 }); fire('mousedown', at, { ...m, button: 0, buttons: 1, detail: i + 1 });
          if (action === 'click-hold') await sleep(a.ms || 700);
          fire('pointerup', at, { ...m, button: 0, detail: i + 1 }); fire('mouseup', at, { ...m, button: 0, detail: i + 1 });
          el = fire('click', at, { ...m, button: 0, detail: i + 1 });
          const focusable = el && el.closest ? el.closest('input,textarea,select,button,a[href],[contenteditable=""],[contenteditable="true"],[tabindex]') : null;
          if (focusable && focusable !== document.activeElement && typeof focusable.focus === 'function') focusable.focus({ preventScroll: true });
          await sleep(40);
        }
        if (n === 2) fire('dblclick', at, { ...m, button: 0, detail: 2 });
        return { at, hit: hitView(hit), target: describe(el) };
      } else if (action === 'rightclick') {
        fire('pointerdown', at, { ...m, button: 2, buttons: 2 }); fire('mousedown', at, { ...m, button: 2, buttons: 2 });
        fire('pointerup', at, { ...m, button: 2 }); fire('mouseup', at, { ...m, button: 2 });
        fire('contextmenu', at, { ...m, button: 2 });
      } else if (action === 'drag') {
        let path = Array.isArray(a.path) ? a.path.map((p) => (Array.isArray(p) ? { x: +p[0], y: +p[1] } : p)) : [];
        const to = await point(a.to);
        if (!path.length && to) {
          const steps = Math.max(2, a.steps || 12);
          for (let i = 1; i <= steps; i++) path.push({ x: at.x + ((to.x - at.x) * i) / steps, y: at.y + ((to.y - at.y) * i) / steps });
        }
        if (!path.length) return { error: 'MISSING_ARGUMENT: to or path' };
        fire('pointerdown', at, { ...m, button: 0, buttons: 1 }); fire('mousedown', at, { ...m, button: 0, buttons: 1 });
        for (const p of path) { await sleep(16); fire('pointermove', p, { ...m, buttons: 1 }); fire('mousemove', p, { ...m, buttons: 1 }); }
        const end = path[path.length - 1];
        fire('pointerup', end, { ...m, button: 0 }); fire('mouseup', end, { ...m, button: 0 });
        await sleep(80);
        let moved = null;
        if (hit && drv) {
          const r = await drv.run('nodePos', { nodeId: hit.nodeId });
          if (r && Number.isFinite(r.x)) {
            const dx = Math.round((r.x - hit.x) * 100) / 100;
            const dy = Math.round((r.y - hit.y) * 100) / 100;
            moved = { nodeId: hit.nodeId, moved: Math.abs(dx) > 0.01 || Math.abs(dy) > 0.01, dx, dy };
          } else moved = { nodeId: hit.nodeId, moved: false, gone: true };
        }
        return { at, to: end, hit: hitView(hit), drag: moved };
      } else if (action === 'drop') {
        if (!a.paths || !a.paths.length) return { error: 'MISSING_ARGUMENT: paths' };
        const dpr = window.devicePixelRatio || 1;
        const position = { x: Math.round(at.x * dpr), y: Math.round(at.y * dpr) };
        const send = (event, payload) => window.__TAURI_INTERNALS__.invoke('plugin:event|emit_to', { target: { kind: 'Webview', label: 'main' }, event, payload });
        await send('tauri://drag-enter', { paths: a.paths, position });
        await send('tauri://drag-over', { position });
        await send('tauri://drag-drop', { paths: a.paths, position });
        await sleep(300);
        return { at, hit: hitView(hit), dropped: a.paths.length };
      } else if (action === 'paste') {
        const el = document.activeElement && document.activeElement !== document.body ? document.activeElement : document.elementFromPoint(at.x, at.y) || document.body;
        const dt = new DataTransfer();
        const paths = a.paths || [];
        if (paths.length) dt.setData('text/uri-list', paths.map((p) => 'file://' + encodeURI(p)).join('\r\n'));
        dt.setData('text/plain', typeof a.text === 'string' ? a.text : paths.join('\n'));
        const ev = new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true, composed: true });
        el.dispatchEvent(ev);
        if (!ev.defaultPrevented && typeof a.text === 'string' && (el.isContentEditable || el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) document.execCommand('insertText', false, a.text);
        await sleep(300);
        return { at, hit: hitView(hit), target: describe(el), handled: ev.defaultPrevented };
      } else if (action === 'scroll') {
        fire('wheel', at, { ...m, deltaX: (a.dx || 0) * 40, deltaY: (a.dy || 0) * 40, deltaMode: 0 });
      } else return { error: 'INVALID_ARGUMENT: action' };
      await sleep(60);
      return { at, hit: hitView(hit) };
    }
    default:
      if (!drv) return { error: 'CANVAS_NOT_OPEN: open the World canvas in the main window (omniget_app_navigate or app_key) and retry' };
      return drv.run(op, a);
  }
}"#;

pub async fn ui(app: &AppHandle, op: &str, args: Value) -> Result<Value, String> {
    ui_timeout(app, op, args, Duration::from_secs(20)).await
}

pub async fn ui_timeout(
    app: &AppHandle,
    op: &str,
    args: Value,
    timeout: Duration,
) -> Result<Value, String> {
    listen_replies(app);
    let window = app
        .get_webview_window("main")
        .ok_or("UI_NO_WINDOW: the main window is not open")?;
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    pending()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), tx);
    let script = String::from("(async()=>{const id=")
        + js_literal(&json!(id)).as_str()
        + ";let r;try{r=await ("
        + RUNNER
        + ")("
        + js_literal(&json!(op)).as_str()
        + ","
        + js_literal(&args).as_str()
        + ");}catch(e){r={error:'UI_FAILED: '+String(e&&e.message||e)};}if(r===undefined)r=null;\
           window.__TAURI_INTERNALS__.invoke('plugin:event|emit',{event:"
        + js_literal(&json!(EVENT_REPLY)).as_str()
        + ",payload:{id,value:r}});})()";
    if let Err(e) = window.eval(&script) {
        pending()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        return Err(format!("UI_FAILED: {e}"));
    }
    let v = match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(v)) => v,
        _ => {
            pending()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err("UI_TIMEOUT: the window did not answer (is it loaded?)".into());
        }
    };
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Err(e.to_string());
    }
    Ok(v)
}

pub async fn screenshot(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let window = app
        .get_webview_window("main")
        .ok_or("UI_NO_WINDOW: the main window is not open")?;
    let png = crate::native_capture::snapshot(&window).await?;
    let region = if let Some(n) = a.get("nodeId").and_then(Value::as_str) {
        let r = ui(app, "nodeRect", json!({ "nodeId": n })).await?;
        if r.is_null() {
            return Err(format!(
                "CANVAS_NOT_FOUND: node {n} is not on the open floor"
            ));
        }
        Some(r)
    } else {
        a.get("region").filter(|r| r.is_object()).cloned()
    };
    let mut img = image::load_from_memory(&png).map_err(|e| format!("UI_CAPTURE: {e}"))?;
    let factor = window.scale_factor().unwrap_or(1.0);
    let css_w = window
        .inner_size()
        .ok()
        .map(|s| s.width as f64 / factor.max(0.1))
        .filter(|w| *w > 0.0);
    let scale = match css_w {
        Some(w) => img.width() as f64 / w,
        None => factor,
    };
    if let Some(r) = region {
        let f = |k: &str| r[k].as_f64().unwrap_or(0.0) * scale;
        let (rx, ry, rw, rh) = (f("x"), f("y"), f("w").max(1.0), f("h").max(1.0));
        if rx >= img.width() as f64 || ry >= img.height() as f64 || rx + rw <= 0.0 || ry + rh <= 0.0
        {
            return Err("UI_OFFSCREEN: the node or region is outside the window (canvas_node_focus or canvas_camera_set first)".into());
        }
        let x0 = rx.max(0.0);
        let y0 = ry.max(0.0);
        let x1 = (rx + rw).min(img.width() as f64);
        let y1 = (ry + rh).min(img.height() as f64);
        let (x, y) = (x0 as u32, y0 as u32);
        let w = ((x1 - x0).round() as u32).max(1);
        let h = ((y1 - y0).round() as u32).max(1);
        img = img.crop_imm(x, y, w, h);
    }
    let dir = omniget_core::core::tools::temp_dir().join("mcp-shots");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "shot-{}.png",
        chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f")
    ));
    let path = match a.get("path").and_then(Value::as_str) {
        Some(p) if test_tools_enabled() => std::path::PathBuf::from(p),
        _ => path,
    };
    img.save_with_format(&path, image::ImageFormat::Png)
        .map_err(|e| format!("UI_CAPTURE: {e}"))?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "width": img.width(),
        "height": img.height(),
        "scale": scale,
    }))
}

const NOTIFY_KEEP: usize = 200;

fn notifications() -> &'static Mutex<VecDeque<Value>> {
    static N: OnceLock<Mutex<VecDeque<Value>>> = OnceLock::new();
    N.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub fn record_notification(source: &str, title: &str, body: &str, system: bool) {
    let mut n = notifications().lock().unwrap_or_else(|e| e.into_inner());
    n.push_back(json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "source": source,
        "title": title,
        "body": body,
        "system": system,
    }));
    while n.len() > NOTIFY_KEEP {
        n.pop_front();
    }
}

pub fn notifications_list(limit: usize, clear: bool) -> Value {
    let mut n = notifications().lock().unwrap_or_else(|e| e.into_inner());
    let items: Vec<Value> = n.iter().rev().take(limit).cloned().collect();
    if clear {
        n.clear();
    }
    json!({ "notifications": items })
}

fn sensitive(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.starts_with("bridge")
        || p.starts_with("mcp")
        || [
            "token",
            "secret",
            "password",
            "api_key",
            "apikey",
            "cookie",
            "credential",
        ]
        .iter()
        .any(|w| p.contains(w))
}

fn redact(v: Value, path: &str) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let p = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    let v = if sensitive(&p) && !v.is_object() {
                        json!("[REDACTED]")
                    } else {
                        redact(v, &p)
                    };
                    (k, v)
                })
                .collect(),
        ),
        v => v,
    }
}

fn at_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .filter(|s| !s.is_empty())
        .try_fold(v, |cur, k| cur.get(k))
}

pub fn settings_get(app: &AppHandle, key: Option<&str>) -> Result<Value, String> {
    let all = serde_json::to_value(crate::storage::config::load_settings(app))
        .map_err(|e| e.to_string())?;
    let all = redact(all, "");
    match key.filter(|k| !k.trim().is_empty()) {
        None => Ok(json!({ "settings": all })),
        Some(k) => {
            let v = at_path(&all, k)
                .cloned()
                .ok_or_else(|| format!("SETTING_UNKNOWN: {k}"))?;
            Ok(json!({ "key": k, "value": v }))
        }
    }
}

pub fn settings_set(app: &AppHandle, key: &str, value: Value) -> Result<Value, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("MISSING_ARGUMENT: key".into());
    }
    if sensitive(key) {
        return Err(format!(
            "SETTING_PROTECTED: {key} cannot be changed over MCP"
        ));
    }
    let current = serde_json::to_value(crate::storage::config::load_settings(app))
        .map_err(|e| e.to_string())?;
    if at_path(&current, key).is_none() {
        return Err(format!(
            "SETTING_UNKNOWN: {key} is not an OmniGet setting (app_settings_get lists them)"
        ));
    }
    let mut patch = value.clone();
    for k in key.rsplit('.') {
        patch = json!({ k: patch });
    }
    crate::commands::settings::update_settings(app.clone(), patch.to_string())
        .map_err(|e| serde_json::to_string(&e).unwrap_or_else(|_| "SETTING_INVALID".into()))?;
    let after = serde_json::to_value(crate::storage::config::load_settings(app))
        .map_err(|e| e.to_string())?;
    let now = at_path(&after, key).cloned().unwrap_or(Value::Null);
    Ok(json!({ "key": key, "value": now, "applied": same_value(&now, &value) }))
}

pub fn same_value(now: &Value, asked: &Value) -> bool {
    match (now, asked) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-9,
            _ => a == b,
        },
        (Value::Object(a), Value::Object(b)) => b
            .iter()
            .all(|(k, v)| a.get(k).is_some_and(|x| same_value(x, v))),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_value(x, y))
        }
        _ => now == asked,
    }
}

fn need_test_tools() -> Result<(), String> {
    if test_tools_enabled() {
        Ok(())
    } else {
        Err(format!(
            "TOOL_NOT_AUTHORIZED: host tools need the app started with {TEST_TOOLS_ENV}=1"
        ))
    }
}

pub async fn host_shell(a: &Value) -> Result<Value, String> {
    need_test_tools()?;
    let command = a["command"].as_str().unwrap_or("").to_string();
    if command.trim().is_empty() {
        return Err("MISSING_ARGUMENT: command".into());
    }
    let timeout = Duration::from_millis(a["timeoutMs"].as_u64().unwrap_or(60_000).min(600_000));
    let mut cmd = tokio::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
    if cfg!(windows) {
        cmd.args(["/C", &command]);
    } else {
        cmd.args(["-c", &command]);
    }
    if let Some(cwd) = a["cwd"].as_str() {
        cmd.current_dir(cwd);
    }
    cmd.kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| "HOST_TIMEOUT".to_string())?
        .map_err(|e| format!("HOST_FAILED: {e}"))?;
    let clip = |b: &[u8]| {
        let s = String::from_utf8_lossy(b);
        let n = s.chars().count();
        if n > 200_000 {
            s.chars().skip(n - 200_000).collect()
        } else {
            s.into_owned()
        }
    };
    Ok(json!({
        "code": out.status.code(),
        "stdout": clip(&out.stdout),
        "stderr": clip(&out.stderr),
    }))
}

pub fn host_fs_read(a: &Value) -> Result<Value, String> {
    need_test_tools()?;
    let path = a["path"].as_str().ok_or("MISSING_ARGUMENT: path")?;
    let bytes = std::fs::read(expand(path)).map_err(|e| format!("HOST_FS: {e}"))?;
    let max = a["maxBytes"].as_u64().unwrap_or(1 << 20) as usize;
    let size = bytes.len();
    let text = String::from_utf8_lossy(&bytes[..size.min(max)]).into_owned();
    Ok(json!({ "path": path, "size": size, "text": text, "truncated": size > max }))
}

pub fn host_fs_write(a: &Value) -> Result<Value, String> {
    need_test_tools()?;
    let path = expand(a["path"].as_str().ok_or("MISSING_ARGUMENT: path")?);
    let text = a["text"].as_str().unwrap_or("");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("HOST_FS: {e}"))?;
    }
    if a["append"].as_bool() == Some(true) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("HOST_FS: {e}"))?;
        f.write_all(text.as_bytes())
            .map_err(|e| format!("HOST_FS: {e}"))?;
    } else {
        std::fs::write(&path, text).map_err(|e| format!("HOST_FS: {e}"))?;
    }
    #[cfg(unix)]
    if a["executable"].as_bool() == Some(true) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(json!({ "path": path.to_string_lossy(), "bytes": text.len() }))
}

fn expand(p: &str) -> std::path::PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => std::path::PathBuf::from(p),
    }
}

fn remote_base() -> Result<String, String> {
    let h = crate::local_bridge_remote::hub();
    let addr = h.bound_addr().ok_or(
        "REMOTE_NOT_RUNNING: turn phone access on first (remote_pair start=true, or Share > Phone)",
    )?;
    let ip = if addr.ip().is_unspecified() {
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    } else {
        addr.ip()
    };
    Ok(crate::local_bridge_remote::base_for(ip, addr.port()))
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_default()
}

fn remote_client(a: &Value) -> Result<(String, reqwest::Client), String> {
    let base = remote_base()?;
    if a["tls"].as_bool() != Some(true) {
        return Ok((base, http()));
    }
    let pin = match a["pin"].as_str() {
        Some(p) => p.to_string(),
        None => crate::local_bridge_remote::tls_info()["pin"]
            .as_str()
            .ok_or("REMOTE_TLS: no certificate")?
            .to_string(),
    };
    let cfg = crate::local_bridge_remote::tls::pinned_client(&pin);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .use_preconfigured_tls(cfg)
        .build()
        .map_err(|e| format!("REMOTE_TLS: {e}"))?;
    Ok((base.replacen("http://", "https://", 1), client))
}

pub async fn remote_pair(app: &AppHandle, a: &Value) -> Result<Value, String> {
    let h = crate::local_bridge_remote::hub();
    h.set_app(app);
    if h.bound_addr().is_none() && a["start"].as_bool() == Some(true) {
        crate::commands::central::remote::remote_start(
            app.clone(),
            a["bindIp"].as_str().unwrap_or("127.0.0.1").to_string(),
            None,
        )
        .await?;
    }
    let base = remote_base()?;
    let scope = match a["scope"]
        .as_str()
        .or_else(|| a["role"].as_str())
        .unwrap_or("read")
    {
        "guest" | "read" | "viewer" => "read",
        "owner" | "drive" | "control" => "drive",
        other => return Err(format!("INVALID_ARGUMENT: scope '{other}' (read or drive)")),
    };
    let code = h.create_code_pairing(
        crate::local_bridge_remote::Scope::parse(scope)?,
        a["label"].as_str().map(str::to_string),
    );
    let name = a["name"].as_str().unwrap_or("MCP test device");
    let resp = http()
        .post(format!("{base}/remote/api/pair"))
        .json(&json!({ "secret": code.code, "name": name }))
        .send()
        .await
        .map_err(|e| format!("REMOTE_HTTP: {e}"))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if status >= 300 {
        return Err(format!("REMOTE_PAIR: HTTP {status} {body}"));
    }
    Ok(
        json!({ "base": base, "scope": scope, "role": if scope == "drive" { "owner" } else { "guest" }, "result": body }),
    )
}

pub async fn remote_call(a: &Value) -> Result<Value, String> {
    let (base, client) = remote_client(a)?;
    let path = a["path"].as_str().ok_or("MISSING_ARGUMENT: path")?;
    if !path.starts_with('/') {
        return Err("INVALID_ARGUMENT: path starts with /".into());
    }
    let method = reqwest::Method::from_bytes(
        a["method"]
            .as_str()
            .unwrap_or("GET")
            .to_ascii_uppercase()
            .as_bytes(),
    )
    .map_err(|_| "INVALID_ARGUMENT: method".to_string())?;
    let mut req = client.request(method, format!("{base}{path}"));
    if let Some(t) = a["token"].as_str() {
        req = req.bearer_auth(t);
    }
    if let Some(hs) = a["headers"].as_object() {
        for (k, v) in hs {
            if let Some(v) = v.as_str() {
                req = req.header(k.as_str(), v);
            }
        }
    }
    if let Some(b) = a.get("body").filter(|b| !b.is_null()) {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| format!("REMOTE_HTTP: {e}"))?;
    let status = resp.status().as_u16();
    let headers: serde_json::Map<String, Value> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap_or(""))))
        .collect();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("REMOTE_HTTP: {e}"))?;
    let body = serde_json::from_slice::<Value>(&bytes)
        .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes[..bytes.len().min(200_000)])));
    Ok(json!({ "status": status, "headers": headers, "body": body }))
}

const CANVAS_STREAM: &str = "/remote/api/canvas/stream";

async fn remote_sse(a: &Value, path: &str) -> Result<Value, String> {
    let (base, client) = remote_client(a)?;
    let secs = a["seconds"].as_u64().unwrap_or(5).clamp(1, 60);
    let max = a["max"].as_u64().unwrap_or(500).clamp(1, 5_000) as usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut req = client
        .get(format!("{base}{path}"))
        .header("accept", "text/event-stream");
    if let Some(t) = a["token"].as_str() {
        req = req.bearer_auth(t);
    }
    let mut resp = tokio::time::timeout(Duration::from_secs(secs), req.send())
        .await
        .map_err(|_| "REMOTE_TIMEOUT".to_string())?
        .map_err(|e| format!("REMOTE_HTTP: {e}"))?;
    let status = resp.status().as_u16();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if status >= 300 || !ctype.starts_with("text/event-stream") {
        let bytes = resp.bytes().await.unwrap_or_default();
        let body = serde_json::from_slice::<Value>(&bytes)
            .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
        return Ok(
            json!({ "messages": [{ "status": status, "body": body }], "transport": "sse", "status": status }),
        );
    }
    if let Some(list) = a["send"].as_array() {
        for m in list {
            let mut call = json!({ "path": "/remote/api/canvas/feed", "method": "POST", "body": m, "tls": a["tls"], "pin": a["pin"] });
            if let Some(node) = path
                .strip_prefix("/remote/api/canvas/nodes/")
                .and_then(|r| r.split('/').next())
            {
                if call["body"]["nodeId"].is_null() {
                    call["body"]["nodeId"] = json!(node);
                }
            }
            if let Some(t) = a["token"].as_str() {
                call["token"] = json!(t);
            }
            let _ = remote_call(&call).await;
        }
    }
    let mut buf = String::new();
    let mut messages = Vec::new();
    let mut keepalives = 0u64;
    'read: while messages.len() < max {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, resp.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(i) = buf.find("\n\n") {
                    let frame: String = buf.drain(..i + 2).collect();
                    let data: Vec<&str> = frame
                        .lines()
                        .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
                        .collect();
                    if data.is_empty() {
                        keepalives += 1;
                        continue;
                    }
                    let text = data.join("\n");
                    messages.push(serde_json::from_str::<Value>(&text).unwrap_or(json!(text)));
                    if messages.len() >= max {
                        break 'read;
                    }
                }
            }
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => break,
        }
    }
    Ok(
        json!({ "messages": messages, "transport": "sse", "status": status, "keepalives": keepalives }),
    )
}

pub async fn remote_stream(a: &Value) -> Result<Value, String> {
    let sse_path = a["path"].as_str().unwrap_or(CANVAS_STREAM);
    let terminal_stream = sse_path.starts_with("/remote/api/canvas/nodes/")
        && sse_path
            .split('?')
            .next()
            .is_some_and(|p| p.ends_with("/stream"));
    if sse_path.starts_with(CANVAS_STREAM) || terminal_stream {
        return remote_sse(a, sse_path).await;
    }
    let secs = a["seconds"].as_u64().unwrap_or(5).clamp(1, 60);
    let every = Duration::from_millis(a["intervalMs"].as_u64().unwrap_or(1000).clamp(200, 10_000));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let path = a["path"]
        .as_str()
        .unwrap_or("/remote/api/events")
        .to_string();
    let mut after = a["after"].as_u64().unwrap_or(0);
    let mut messages = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let sep = if path.contains('?') { '&' } else { '?' };
        let mut call = a.clone();
        call["path"] = json!(format!("{path}{sep}after={after}"));
        call["method"] = json!("GET");
        let r = remote_call(&call).await?;
        if r["status"].as_u64().unwrap_or(0) >= 300 {
            messages.push(r);
            break;
        }
        let body = r["body"].clone();
        let events = body
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| body.as_array().cloned());
        match events {
            Some(list) => {
                for e in list {
                    if let Some(s) = e.get("seq").or_else(|| e.get("id")).and_then(Value::as_u64) {
                        after = after.max(s);
                    }
                    messages.push(e);
                }
            }
            None => messages.push(body),
        }
        tokio::time::sleep(every).await;
    }
    Ok(json!({ "messages": messages, "after": after }))
}

pub fn restart_later(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        crate::canvas::doc::flush_on_exit();
        app.restart();
    });
}
