use std::path::PathBuf;
use std::time::Duration;

use omniget_core::core::canvas::model::{new_id, CanvasNode, NodeKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::portal;

pub const EVENT_DESIGN: &str = "canvas://design-select";
const ARM_WAIT: Duration = Duration::from_secs(300);
const POLL_EVERY: Duration = Duration::from_millis(350);
const HTML_MAX: usize = 4000;

const PICK_LIB: &str = r#"
  var OVERLAY_ATTR = 'data-omniget-design';
  function esc(s) { return (window.CSS && CSS.escape) ? CSS.escape(s) : String(s).replace(/[^a-zA-Z0-9_-]/g, '\\$&'); }
  function uniq(sel) { try { return document.querySelectorAll(sel).length === 1; } catch (e) { return false; } }
  function cssPath(el) {
    if (!(el instanceof Element)) return '';
    var parts = [];
    while (el && el.nodeType === 1 && el !== document.documentElement) {
      if (el.id && uniq('#' + esc(el.id))) { parts.unshift('#' + esc(el.id)); break; }
      var p = el.tagName.toLowerCase();
      var cls = Array.prototype.filter.call(el.classList || [], function (c) { return c && !/^\d/.test(c) && c.length < 40; }).slice(0, 2);
      if (cls.length) p += '.' + cls.map(esc).join('.');
      var par = el.parentElement;
      if (par) {
        var same = Array.prototype.filter.call(par.children, function (c) { return c.tagName === el.tagName; });
        if (same.length > 1) p += ':nth-of-type(' + (same.indexOf(el) + 1) + ')';
      }
      parts.unshift(p);
      if (uniq(parts.join(' > '))) break;
      el = par;
    }
    return parts.join(' > ');
  }
  function under(x, y) {
    var all = document.elementsFromPoint(x, y) || [];
    for (var i = 0; i < all.length; i++) { if (!all[i].closest('[' + OVERLAY_ATTR + ']')) return all[i]; }
    return null;
  }
  function contains(b, r) { return b.left <= r.x + 3 && b.top <= r.y + 3 && b.right >= r.x + r.w - 3 && b.bottom >= r.y + r.h - 3; }
  function pick(a) {
    var el = null, box = null;
    if (a.selector) {
      try { el = document.querySelector(a.selector); } catch (e) { throw new Error('DESIGN_ARGS: bad selector ' + a.selector); }
      if (!el) throw new Error('DESIGN_NOT_FOUND: no element matches ' + a.selector);
      if (el.scrollIntoView) el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    } else if (a.rect) {
      var r = a.rect;
      box = { x: r.x, y: r.y, w: Math.max(1, r.w), h: Math.max(1, r.h) };
      el = under(r.x + r.w / 2, r.y + r.h / 2);
      while (el && el.parentElement && el !== document.body && !contains(el.getBoundingClientRect(), box)) el = el.parentElement;
    } else if (a.point) {
      el = under(a.point.x, a.point.y);
    }
    if (!el) el = document.body;
    var b = el.getBoundingClientRect();
    if (!box) box = { x: Math.max(0, b.left), y: Math.max(0, b.top), w: Math.max(1, Math.min(b.width, innerWidth)), h: Math.max(1, Math.min(b.height, innerHeight)) };
    var html = el.outerHTML || '';
    return {
      selector: cssPath(el),
      tag: el.tagName.toLowerCase(),
      html: html.length > __MAX__ ? html.slice(0, __MAX__) + '…' : html,
      text: String(el.innerText || '').trim().slice(0, 500),
      box: box,
      elementBox: { x: b.left, y: b.top, w: b.width, h: b.height },
      viewport: { w: innerWidth, h: innerHeight, dpr: window.devicePixelRatio || 1 },
      url: location.href,
      title: document.title
    };
  }
"#;

const PICK_BODY: &str = "return pick(a || {});";

const ARM_BODY: &str = r#"
  var old = document.querySelector('[' + OVERLAY_ATTR + ']');
  if (old) old.remove();
  window.__omnigetDesign = null;
  var root = document.createElement('div');
  root.setAttribute(OVERLAY_ATTR, '1');
  root.style.cssText = 'position:fixed;inset:0;z-index:2147483647;cursor:crosshair;background:rgba(0,0,0,0.04)';
  var hover = document.createElement('div');
  hover.style.cssText = 'position:fixed;pointer-events:none;border:2px solid #5b8cff;background:rgba(91,140,255,0.12);border-radius:3px;display:none';
  var band = document.createElement('div');
  band.style.cssText = 'position:fixed;pointer-events:none;border:2px dashed #ff7a45;background:rgba(255,122,69,0.12);display:none';
  var tip = document.createElement('div');
  tip.textContent = a && a.hint ? a.hint : 'Drag a region or click an element · Esc cancels';
  tip.style.cssText = 'position:fixed;left:50%;top:10px;transform:translateX(-50%);pointer-events:none;font:12px -apple-system,system-ui,sans-serif;color:#fff;background:rgba(20,20,24,0.85);padding:5px 10px;border-radius:8px';
  root.appendChild(hover); root.appendChild(band); root.appendChild(tip);
  document.documentElement.appendChild(root);
  var start = null;
  function place(d, r) { d.style.display = 'block'; d.style.left = r.x + 'px'; d.style.top = r.y + 'px'; d.style.width = r.w + 'px'; d.style.height = r.h + 'px'; }
  function finish(v) { root.remove(); document.removeEventListener('keydown', onKey, true); window.__omnigetDesign = v; }
  function onKey(e) { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finish({ done: true, cancelled: true }); } }
  document.addEventListener('keydown', onKey, true);
  root.addEventListener('mousemove', function (e) {
    if (start) {
      place(band, { x: Math.min(start.x, e.clientX), y: Math.min(start.y, e.clientY), w: Math.abs(e.clientX - start.x), h: Math.abs(e.clientY - start.y) });
      return;
    }
    var el = under(e.clientX, e.clientY);
    if (!el) { hover.style.display = 'none'; return; }
    var b = el.getBoundingClientRect();
    place(hover, { x: b.left, y: b.top, w: b.width, h: b.height });
  });
  root.addEventListener('mousedown', function (e) { e.preventDefault(); start = { x: e.clientX, y: e.clientY }; hover.style.display = 'none'; });
  root.addEventListener('mouseup', function (e) {
    if (!start) return;
    var w = Math.abs(e.clientX - start.x), h = Math.abs(e.clientY - start.y);
    var args = (w < 5 && h < 5) ? { point: { x: e.clientX, y: e.clientY } } : { rect: { x: Math.min(start.x, e.clientX), y: Math.min(start.y, e.clientY), w: w, h: h } };
    start = null;
    try { finish({ done: true, pick: pick(args) }); } catch (err) { finish({ done: true, error: String(err && err.message || err) }); }
  });
  return { armed: true };
"#;

const POLL_BODY: &str = r#"
  var d = window.__omnigetDesign;
  if (d && d.done) { window.__omnigetDesign = null; return d; }
  return { done: false, armed: !!document.querySelector('[' + OVERLAY_ATTR + ']') };
"#;

const DISARM_BODY: &str = r#"
  var o = document.querySelector('[' + OVERLAY_ATTR + ']');
  if (o) o.remove();
  window.__omnigetDesign = { done: true, cancelled: true };
  return { disarmed: !!o };
"#;

fn func(body: &str) -> String {
    format!(
        "function (a) {{\n{}\n{}\n}}",
        PICK_LIB.replace("__MAX__", &HTML_MAX.to_string()),
        body
    )
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub dpr: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    pub selector: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub html: String,
    #[serde(default)]
    pub text: String,
    #[serde(skip)]
    pub box_: Option<Rect>,
    #[serde(default)]
    pub element_box: Option<Rect>,
    #[serde(default)]
    pub viewport: Option<Viewport>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
}

fn pick_from(v: &Value) -> Result<Pick, String> {
    let mut p: Pick = serde_json::from_value(v.clone()).map_err(|e| format!("DESIGN_PAGE: {e}"))?;
    p.box_ = v
        .get("box")
        .and_then(|b| serde_json::from_value(b.clone()).ok());
    Ok(p)
}

fn pick_json(p: &Pick) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("box_");
        o.insert("box".into(), json!(p.box_));
    }
    v
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub node_id: String,
    pub name: String,
    pub agent: String,
}

fn store() -> std::sync::Arc<omniget_core::core::canvas::store::CanvasStore> {
    super::doc::host().store().clone()
}

fn portal_node(portal_id: &str) -> Result<(String, String, CanvasNode), String> {
    let s = store();
    let (ws, floor) = s
        .locate_node(portal_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {portal_id}"))?;
    let n = s
        .read(&ws, &floor, |d| d.node(portal_id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {portal_id}"))?;
    if n.kind != NodeKind::Portal {
        return Err("DESIGN_ARGS: design-select works on portal nodes".into());
    }
    if n.data_str("portalKind").unwrap_or("web") != "web" {
        return Err(
            "PORTAL_UNSUPPORTED: design-select works on web portals, not device nodes".into(),
        );
    }
    Ok((ws, floor, n))
}

pub fn targets(portal_id: &str) -> Result<Vec<Target>, String> {
    let (ws, floor, _) = portal_node(portal_id)?;
    store().read(&ws, &floor, |d| {
        d.connections
            .iter()
            .filter(|c| c.touches(portal_id))
            .filter_map(|c| c.other_end(portal_id))
            .filter_map(|id| d.node(id))
            .filter(|n| n.is_terminal())
            .map(|n| Target {
                node_id: n.id.clone(),
                name: super::cords::node_name(n),
                agent: n.data_str("agent").unwrap_or("shell").to_string(),
            })
            .collect()
    })
}

fn resolve_target(portal_id: &str, want: Option<&str>) -> Result<Target, String> {
    let list = targets(portal_id)?;
    let names = || {
        list.iter()
            .map(|t| format!("\"{}\"", t.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match want.map(str::trim).filter(|w| !w.is_empty()) {
        Some(w) => list
            .iter()
            .find(|t| t.node_id == w)
            .or_else(|| list.iter().find(|t| t.name.eq_ignore_ascii_case(w)))
            .cloned()
            .ok_or_else(|| {
                if list.is_empty() {
                    "CORD_REQUIRED: connect an agent to this portal with a cord first".to_string()
                } else {
                    format!(
                        "CORD_REQUIRED: \"{w}\" is not corded to this portal; corded agents: {}",
                        names()
                    )
                }
            }),
        None => match list.as_slice() {
            [] => Err("CORD_REQUIRED: connect an agent to this portal with a cord first".into()),
            [one] => Ok(one.clone()),
            _ => Err(format!(
                "DESIGN_AMBIGUOUS: several agents are corded, pass `target`: {}",
                names()
            )),
        },
    }
}

fn app_handle() -> Result<AppHandle, String> {
    portal::app().ok_or_else(|| {
        "PORTAL_NOT_READY: the canvas has not started yet; open the World canvas once".into()
    })
}

async fn crop(app: &AppHandle, ws: &str, portal_id: &str, p: &Pick) -> Result<String, String> {
    let bytes = portal::snapshot_png(app, portal_id).await?;
    let dir: PathBuf = store().workspace_dir(ws).join("files").join("design");
    let (Some(b), Some(vp)) = (p.box_.clone(), p.viewport.clone()) else {
        return Err("DESIGN_PAGE: the page gave no box".into());
    };
    tauri::async_runtime::spawn_blocking(move || {
        let img = image::load_from_memory(&bytes).map_err(|e| format!("DESIGN_CROP: {e}"))?;
        let (iw, ih) = (img.width() as f64, img.height() as f64);
        let scale = if vp.w > 0.0 { iw / vp.w } else { 1.0 };
        let x = (b.x * scale).floor().clamp(0.0, iw - 1.0);
        let y = (b.y * scale).floor().clamp(0.0, ih - 1.0);
        let w = (b.w * scale).ceil().clamp(1.0, iw - x);
        let h = (b.h * scale).ceil().clamp(1.0, ih - y);
        let out = img.crop_imm(x as u32, y as u32, w as u32, h as u32);
        std::fs::create_dir_all(&dir).map_err(|e| format!("DESIGN_IO: {e}"))?;
        let path = dir.join(format!("design-{}.png", new_id()));
        out.save_with_format(&path, image::ImageFormat::Png)
            .map_err(|e| format!("DESIGN_IO: {e}"))?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("DESIGN_TASK: {e}"))?
}

pub fn message(
    comment: &str,
    p: &Pick,
    screenshot: Option<&str>,
    attach: &[String],
    portal_name: &str,
) -> String {
    let mut m = String::new();
    m.push_str(&format!(
        "Design selection from the portal \"{portal_name}\" ({})\n",
        if p.url.is_empty() { "page" } else { &p.url }
    ));
    let c = comment.trim();
    if !c.is_empty() {
        m.push_str(&format!("Request: {c}\n"));
    }
    if !p.selector.is_empty() {
        m.push_str(&format!(
            "Element: {} (CSS selector: {})\n",
            p.tag, p.selector
        ));
    }
    if let Some(b) = &p.box_ {
        m.push_str(&format!(
            "Region: x={:.0} y={:.0} w={:.0} h={:.0} (viewport px)\n",
            b.x, b.y, b.w, b.h
        ));
    }
    if let Some(s) = screenshot {
        m.push_str(&format!("Screenshot of the region: {s}\n"));
    }
    for a in attach {
        m.push_str(&format!("Attachment: {a}\n"));
    }
    if !p.html.is_empty() {
        m.push_str("HTML:\n```html\n");
        m.push_str(&p.html);
        m.push_str("\n```\n");
    }
    m
}

async fn page_call(
    app: &AppHandle,
    portal_id: &str,
    body: &str,
    args: Value,
) -> Result<Value, String> {
    portal::automation::ensure_open(app, portal_id)?;
    portal::eval_fn(app, portal_id, &func(body), args).await
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectArgs {
    pub rect: Option<Rect>,
    pub selector: Option<String>,
    pub point: Option<Value>,
    #[serde(default)]
    pub comment: String,
    pub target: Option<String>,
    #[serde(default)]
    pub attach: Vec<String>,
    pub send: Option<bool>,
}

pub async fn select(portal_id: &str, a: SelectArgs) -> Result<Value, String> {
    let app = app_handle()?;
    let (ws, _, node) = portal_node(portal_id)?;
    let send = a.send.unwrap_or(true);
    let target = if send {
        Some(resolve_target(portal_id, a.target.as_deref())?)
    } else {
        None
    };
    for f in &a.attach {
        if !std::path::Path::new(f).exists() {
            return Err(format!("DESIGN_ARGS: attachment {f} does not exist"));
        }
    }
    let mut args = json!({});
    if let Some(r) = &a.rect {
        args["rect"] = json!(r);
    }
    if let Some(s) = &a.selector {
        args["selector"] = json!(s);
    }
    if let Some(p) = &a.point {
        args["point"] = p.clone();
    }
    let v = page_call(&app, portal_id, PICK_BODY, args).await?;
    let pick = pick_from(&v)?;
    let shot = crop(&app, &ws, portal_id, &pick).await;
    let shot_path = shot.as_ref().ok().cloned();
    let mut out = pick_json(&pick);
    out["screenshotPath"] = json!(shot_path);
    if let Err(e) = &shot {
        out["screenshotError"] = json!(e);
    }
    if let Some(t) = target {
        let name = super::cords::node_name(&node);
        let text = message(&a.comment, &pick, shot_path.as_deref(), &a.attach, &name);
        deliver(&t, text).await?;
        out["deliveredTo"] = json!(t);
    }
    Ok(out)
}

async fn deliver(t: &Target, text: String) -> Result<(), String> {
    let marker = delivery_marker(&text);
    let before = marker_count(&t.node_id, &marker);
    if super::cords::host().deliver_chat(&t.node_id, text).await {
        return Ok(());
    }
    if marker_count(&t.node_id, &marker) > before {
        return Ok(());
    }
    Err(format!(
        "DESIGN_NOT_DELIVERED: \"{}\" did not take the message (busy or not running)",
        t.name
    ))
}

fn delivery_marker(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let quoted = first.split('"').nth(1).unwrap_or("");
    let base = format!("Design selection from the portal \"{quoted}");
    base.chars().take(48).collect()
}

fn marker_count(node_id: &str, marker: &str) -> usize {
    let Some(tid) = store()
        .locate_node(node_id)
        .and_then(|(ws, floor)| {
            store()
                .read(&ws, &floor, |d| {
                    d.node(node_id)
                        .and_then(|n| n.data_str("terminalId").map(String::from))
                })
                .ok()
                .flatten()
        })
        .filter(|t| !t.is_empty())
    else {
        return 0;
    };
    let Ok(snap) = crate::pty::term().screen_text(&tid, 400) else {
        return 0;
    };
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let m = squash(marker);
    if m.is_empty() {
        return 0;
    }
    squash(&snap.text).matches(&m).count()
}

pub async fn arm(portal_id: &str, hint: Option<String>) -> Result<Value, String> {
    let app = app_handle()?;
    let (ws, _, _) = portal_node(portal_id)?;
    let targets = targets(portal_id)?;
    page_call(&app, portal_id, ARM_BODY, json!({ "hint": hint })).await?;
    let id = portal_id.to_string();
    let app2 = app.clone();
    let reply = json!({ "armed": true, "targets": targets });
    tauri::async_runtime::spawn(async move {
        let t0 = std::time::Instant::now();
        let done = loop {
            tokio::time::sleep(POLL_EVERY).await;
            if t0.elapsed() > ARM_WAIT {
                let _ = portal::eval_fn(&app2, &id, &func(DISARM_BODY), json!({})).await;
                break json!({ "cancelled": true, "reason": "timeout" });
            }
            match portal::eval_fn(&app2, &id, &func(POLL_BODY), json!({})).await {
                Ok(v) if v["done"].as_bool() == Some(true) => break v,
                Ok(v) if v["armed"].as_bool() == Some(false) => {
                    break json!({ "cancelled": true, "reason": "navigated" });
                }
                Ok(_) => {}
                Err(_) => break json!({ "cancelled": true, "reason": "closed" }),
            }
        };
        let mut ev = json!({
            "workspaceId": ws,
            "nodeId": id,
            "targets": targets,
        });
        if done["cancelled"].as_bool() == Some(true) || done.get("error").is_some() {
            ev["cancelled"] = json!(true);
            ev["error"] = done.get("error").cloned().unwrap_or(Value::Null);
        } else if let Ok(p) = pick_from(&done["pick"]) {
            ev["pick"] = pick_json(&p);
            match crop(&app2, &ws, &id, &p).await {
                Ok(path) => ev["screenshotPath"] = json!(path),
                Err(e) => ev["screenshotError"] = json!(e),
            }
        } else {
            ev["cancelled"] = json!(true);
        }
        let _ = app2.emit(EVENT_DESIGN, ev);
    });
    Ok(reply)
}

pub async fn disarm(portal_id: &str) -> Result<Value, String> {
    let app = app_handle()?;
    portal::eval_fn(&app, portal_id, &func(DISARM_BODY), json!({})).await
}

pub async fn send(
    portal_id: &str,
    target: &str,
    comment: &str,
    pick: Value,
    screenshot: Option<String>,
    attach: Vec<String>,
) -> Result<Value, String> {
    let (_, _, node) = portal_node(portal_id)?;
    let t = resolve_target(portal_id, Some(target))?;
    let p = pick_from(&pick)?;
    let text = message(
        comment,
        &p,
        screenshot.as_deref(),
        &attach,
        &super::cords::node_name(&node),
    );
    deliver(&t, text).await?;
    Ok(json!({ "deliveredTo": t }))
}
