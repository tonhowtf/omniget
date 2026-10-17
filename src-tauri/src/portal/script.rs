use serde_json::Value;

use crate::preview::script as base;

pub const REPORT_COMMAND: &str = "canvas_portal_report";

fn repoint(js: &str) -> String {
    js.replace("preview_report", REPORT_COMMAND)
}

pub fn init() -> String {
    format!("{}\n{}", repoint(base::INIT), repoint(POPUP_INIT))
}

const POPUP_INIT: &str = r#"(function () {
  // Every portal page: the app only acts for popup portals.
  if (window.__omnigetPopupClose) return;
  window.__omnigetPopupClose = true;
  var T = window.__TAURI_INTERNALS__;
  var orig = window.close;
  window.close = function () {
    try {
      if (T && T.invoke) {
        var p = T.invoke('preview_report', { kind: 'close', id: null, data: { popup: !!window.opener, url: location.href } });
        if (p && p.catch) p.catch(function () {});
      }
    } catch (e) {}
    try { return orig.apply(window, arguments); } catch (e) {}
  };
})();"#;

pub fn mute_js(muted: bool) -> String {
    format!(
        r#"(function (m) {{
  window.__omnigetMuted = m;
  function apply(root) {{
    try {{ (root.querySelectorAll ? root.querySelectorAll('audio,video') : []).forEach(function (el) {{ el.muted = m; }}); }} catch (e) {{}}
  }}
  apply(document);
  if (!window.__omnigetMuteObs) {{
    window.__omnigetMuteObs = new MutationObserver(function () {{ if (window.__omnigetMuted) apply(document); }});
    window.__omnigetMuteObs.observe(document.documentElement, {{ childList: true, subtree: true }});
    document.addEventListener('play', function (e) {{ if (window.__omnigetMuted && e.target) e.target.muted = true; }}, true);
  }}
}})({muted});"#
    )
}

pub fn wrap(id: &str, func: &str, args: &Value) -> String {
    repoint(&base::wrap(id, func, args))
}

pub fn snapshot_fn() -> String {
    base::snapshot_fn()
}

pub fn type_fn() -> String {
    base::type_fn()
}

pub const CONSOLE_FN: &str = base::CONSOLE_FN;

pub fn normalize_selector(raw: &str) -> String {
    let s = raw.trim();
    s.strip_prefix('@').unwrap_or(s).to_string()
}

pub fn is_point(sel: &str) -> bool {
    let mut parts = sel.split(',');
    let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    a.trim().parse::<f64>().is_ok() && b.trim().parse::<f64>().is_ok()
}

pub fn element_fn() -> String {
    format!(
        r#"function (a) {{
{helpers}
  var act = a.action;
  if (act === 'probe') {{
    try {{
      if (a.selector) return !!find(a.selector);
      if (a.text) return (document.body.innerText || '').indexOf(a.text) >= 0;
      return document.readyState === 'complete';
    }} catch (e) {{ return false; }}
  }}
  var el = a.selector ? find(a.selector) : null;
  if (act === 'scroll') {{
    if (el && a.dy == null && a.dx == null && !a.direction) {{
      el.scrollIntoView({{ block: 'center' }});
      return {{ scrolled: describe(el), scrollY: window.scrollY }};
    }}
    var dy = a.dy != null ? Number(a.dy) : (a.direction === 'up' ? -600 : a.direction === 'top' ? -1e9 : a.direction === 'bottom' ? 1e9 : 600);
    (el || window).scrollBy(Number(a.dx || 0), dy);
    return {{ scrollY: window.scrollY }};
  }}
  if (act === 'text' || act === 'html') {{
    var root = el || document.body;
    var v = act === 'text' ? (root.innerText || '') : root.outerHTML;
    var max = a.maxChars || 20000;
    return {{ value: v.slice(0, max), truncated: v.length > max, untrusted: true }};
  }}
  if (!el) throw new Error('selector is required');
  if (act === 'hover') {{
    el.scrollIntoView({{ block: 'center' }});
    var r = el.getBoundingClientRect();
    var o = {{ bubbles: true, composed: true, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, view: window }};
    ['pointerover', 'pointerenter', 'mouseover', 'mouseenter', 'pointermove', 'mousemove'].forEach(function (n) {{
      try {{ el.dispatchEvent(n.indexOf('pointer') === 0 ? new PointerEvent(n, o) : new MouseEvent(n, o)); }} catch (e) {{}}
    }});
    return {{ hovered: describe(el) }};
  }}
  if (act === 'check' || act === 'uncheck') {{
    var want = act === 'check';
    if (!!el.checked !== want) el.click();
    return {{ checked: !!el.checked, element: describe(el) }};
  }}
  if (act === 'select') {{
    if (el.tagName !== 'SELECT') throw new Error(describe(el) + ' is not a <select>');
    var val = String(a.value == null ? '' : a.value).trim();
    var opt = [].slice.call(el.options).find(function (x) {{ return x.value === val || x.text.trim() === val; }});
    if (!opt) throw new Error('no option ' + JSON.stringify(val));
    el.value = opt.value;
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
    return {{ selected: opt.value, text: opt.text.trim() }};
  }}
  throw new Error('unknown action ' + act);
}}"#,
        helpers = base::HELPERS
    )
}

pub fn click_fn(selector: &str) -> String {
    if is_point(selector) {
        POINT_CLICK_FN.to_string()
    } else {
        base::click_fn()
    }
}

const POINT_CLICK_FN: &str = r#"function (a) {
  var p = String(a.selector).split(',');
  var x = parseFloat(p[0]), y = parseFloat(p[1]);
  var el = document.elementFromPoint(x, y);
  if (!el) throw new Error('nothing at ' + x + ',' + y);
  var o = { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, button: 0, view: window };
  var po = Object.assign({ pointerId: 1, pointerType: 'mouse', isPrimary: true }, o);
  try { el.dispatchEvent(new PointerEvent('pointerdown', po)); } catch (e) {}
  el.dispatchEvent(new MouseEvent('mousedown', o));
  if (el.focus) el.focus({ preventScroll: true });
  try { el.dispatchEvent(new PointerEvent('pointerup', po)); } catch (e) {}
  el.dispatchEvent(new MouseEvent('mouseup', o));
  el.dispatchEvent(new MouseEvent('click', o));
  if (a.double) el.dispatchEvent(new MouseEvent('dblclick', o));
  return { clicked: el.tagName.toLowerCase() + (el.id ? '#' + el.id : ''), url: location.href };
}"#;

pub const PRESS_FN: &str = r#"function (a) {
  var P = window.__omnigetPreview || { refs: new Map() };
  var el = document.activeElement || document.body;
  if (a.selector) {
    var m = String(a.selector).match(/^(?:ref=)?(e\d+)$/);
    if (m) {
      el = P.refs && P.refs.get(m[1]);
      if (!el || !el.isConnected) throw new Error('ref ' + m[1] + ' is gone; take a new snapshot');
    } else {
      el = document.querySelector(a.selector);
      if (!el) throw new Error('No element matches ' + a.selector);
    }
    if (el.focus) el.focus();
  }
  var parts = String(a.key || '').split('+').map(function (s) { return s.trim(); }).filter(Boolean);
  if (!parts.length) throw new Error('key is empty');
  var key = parts.pop();
  var mods = {};
  parts.forEach(function (m) { mods[m.toLowerCase()] = true; });
  var NAMES = { enter: 'Enter', return: 'Enter', tab: 'Tab', esc: 'Escape', escape: 'Escape', space: ' ', backspace: 'Backspace', delete: 'Delete', up: 'ArrowUp', down: 'ArrowDown', left: 'ArrowLeft', right: 'ArrowRight', arrowup: 'ArrowUp', arrowdown: 'ArrowDown', arrowleft: 'ArrowLeft', arrowright: 'ArrowRight', home: 'Home', end: 'End', pageup: 'PageUp', pagedown: 'PageDown' };
  var k = NAMES[key.toLowerCase()] || key;
  var code = k.length === 1 ? (/[a-z]/i.test(k) ? 'Key' + k.toUpperCase() : /[0-9]/.test(k) ? 'Digit' + k : '') : k;
  var KC = { Enter: 13, Tab: 9, Escape: 27, ' ': 32, Backspace: 8, Delete: 46, ArrowUp: 38, ArrowDown: 40, ArrowLeft: 37, ArrowRight: 39, Home: 36, End: 35, PageUp: 33, PageDown: 34 };
  var kc = KC[k] || (k.length === 1 ? k.toUpperCase().charCodeAt(0) : 0);
  var init = { key: k, code: code, keyCode: kc, which: kc, bubbles: true, cancelable: true, composed: true,
    ctrlKey: !!(mods.ctrl || mods.control), metaKey: !!(mods.cmd || mods.meta || mods.command), shiftKey: !!mods.shift, altKey: !!(mods.alt || mods.option) };
  var go = el.dispatchEvent(new KeyboardEvent('keydown', init));
  if (go && k.length === 1) el.dispatchEvent(new KeyboardEvent('keypress', init));
  el.dispatchEvent(new KeyboardEvent('keyup', init));
  if (go && k === 'Enter' && el.form && !init.ctrlKey && !init.metaKey) {
    if (el.form.requestSubmit) el.form.requestSubmit(); else el.form.submit();
  }
  if (go && (init.ctrlKey || init.metaKey) && k.toLowerCase() === 'a' && el.select) el.select();
  return { pressed: String(a.key), target: el.tagName ? el.tagName.toLowerCase() : 'document', url: location.href };
}"#;

pub const EVALUATE_FN: &str = r#"function (a) {
  var code = String(a.code || '');
  var v;
  try { v = (0, eval)('(' + code + '\n)'); }
  catch (e) { if (e instanceof SyntaxError) v = (0, eval)(code); else throw e; }
  return Promise.resolve(v).then(function (r) {
    if (r === undefined) return null;
    if (typeof r === 'function') return String(r);
    if (r instanceof Element) return r.outerHTML.slice(0, 20000);
    try {
      var s = JSON.stringify(r);
      if (s === undefined) return String(r);
      if (s.length > 20000) return s.slice(0, 20000) + '…';
      return JSON.parse(s);
    } catch (e) { return String(r); }
  });
}"#;

pub const UPLOAD_CHUNK_FN: &str = r#"function (a) {
  var U = window.__omnigetUpload || (window.__omnigetUpload = {});
  var f = U[a.id] || (U[a.id] = []);
  (f[a.i] || (f[a.i] = [])).push(a.data);
  return f[a.i].length;
}"#;

pub const UPLOAD_DROP_FN: &str = r#"function (a) {
  if (window.__omnigetUpload) delete window.__omnigetUpload[a.id];
  return true;
}"#;

pub fn upload_fn() -> String {
    format!(
        r#"function (a) {{
{helpers}
  var U = window.__omnigetUpload || {{}};
  var parts = U[a.id] || [];
  delete U[a.id];
  function bytes(b64) {{
    var s = atob(b64);
    var u = new Uint8Array(s.length);
    for (var i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
    return u;
  }}
  var dt = new DataTransfer();
  var names = [];
  (a.files || []).forEach(function (f, i) {{
    dt.items.add(new File((parts[i] || []).map(bytes), f.name, {{ type: f.type || '', lastModified: f.lastModified || Date.now() }}));
    names.push(f.name);
  }});
  function isFile(el) {{ return !!el && el.tagName === 'INPUT' && String(el.type).toLowerCase() === 'file'; }}
  function inputFor(el) {{
    if (isFile(el)) return el;
    if (el.tagName === 'LABEL') {{
      if (isFile(el.control)) return el.control;
    }}
    var lab = el.closest && el.closest('label');
    if (lab && isFile(lab.control)) return lab.control;
    var inner = el.querySelector && el.querySelector('input[type=file]');
    if (inner) return inner;
    var ids = ((el.getAttribute && (el.getAttribute('aria-controls') || el.getAttribute('for'))) || '').split(/\s+/);
    for (var k = 0; k < ids.length; k++) {{
      var c = ids[k] && document.getElementById(ids[k]);
      if (isFile(c)) return c;
    }}
    for (var p = el.parentElement, d = 0; p && d < 4; p = p.parentElement, d++) {{
      var near = p.querySelectorAll('input[type=file]');
      if (near.length === 1) return near[0];
      if (near.length > 1) break;
    }}
    return null;
  }}
  function opens(el) {{
    var proto = HTMLInputElement.prototype;
    var oc = proto.click, osp = proto.showPicker, caught = null;
    proto.click = function () {{ if (isFile(this)) {{ caught = this; return; }} return oc.apply(this, arguments); }};
    if (osp) proto.showPicker = function () {{ if (isFile(this)) {{ caught = this; return; }} return osp.apply(this, arguments); }};
    try {{ el.click(); }} finally {{
      proto.click = oc;
      if (osp) proto.showPicker = osp;
    }}
    return caught;
  }}
  function give(input) {{
    if (input.disabled) throw new Error(describe(input) + ' is disabled');
    if (!input.multiple && dt.files.length > 1) throw new Error(describe(input) + ' takes a single file; upload them one at a time');
    input.files = dt.files;
    input.dispatchEvent(new Event('input', {{ bubbles: true, composed: true }}));
    input.dispatchEvent(new Event('change', {{ bubbles: true, composed: true }}));
    return {{ uploaded: names, via: 'input', element: describe(input), accept: input.accept || null, files: input.files ? input.files.length : 0 }};
  }}
  var el = a.selector ? find(a.selector) : null;
  if (!el) {{
    var all = document.querySelectorAll('input[type=file]');
    if (all.length === 1) return give(all[0]);
    if (!all.length) throw new Error('no file input on this page; pass the @ref of the upload button or drop zone');
    throw new Error(all.length + ' file inputs on this page; pass the @ref of the one to use');
  }}
  var input = inputFor(el);
  if (input) return give(input);
  var t = el.tagName;
  var role = (el.getAttribute('role') || '').toLowerCase();
  if (t === 'BUTTON' || t === 'A' || t === 'LABEL' || t === 'SUMMARY' || role === 'button' || role === 'link') {{
    var caught = opens(el);
    if (caught) return give(caught);
  }}
  el.scrollIntoView({{ block: 'center' }});
  var r = el.getBoundingClientRect();
  var o = {{ bubbles: true, cancelable: true, composed: true, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, view: window }};
  function drag(n) {{
    var ev = null;
    try {{ ev = new DragEvent(n, Object.assign({{ dataTransfer: dt }}, o)); }} catch (e) {{}}
    if (!ev || ev.dataTransfer !== dt) {{
      ev = new Event(n, o);
      Object.defineProperty(ev, 'dataTransfer', {{ value: dt }});
    }}
    return ev;
  }}
  var taken = false;
  ['dragenter', 'dragover', 'drop'].forEach(function (n) {{
    if (!el.dispatchEvent(drag(n)) && n !== 'dragenter') taken = true;
  }});
  return {{ uploaded: names, via: 'drop', element: describe(el), accepted: taken, hint: taken ? null : 'the element did not take the drop; snapshot again and target the file input or the button that opens it' }};
}}"#,
        helpers = base::HELPERS
    )
}
