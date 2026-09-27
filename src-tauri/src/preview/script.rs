//! JavaScript injected into the preview webview. The init script runs on every
//! page load (main frame) and keeps a console ring and the element refs in
//! page memory; nothing is sent to the host until a tool asks, except the URL
//! of single-page navigations. Answers come back through the app command
//! `preview_report` (Tauri IPC), keyed by a one-shot request id.

use serde_json::Value;

pub const INIT: &str = r#"(function () {
  if (window.__omnigetPreview) return;
  var T = window.__TAURI_INTERNALS__;
  function send(kind, id, data) {
    try {
      if (T && T.invoke) {
        var p = T.invoke('preview_report', { kind: kind, id: id || null, data: data === undefined ? null : data });
        if (p && p.catch) p.catch(function () {});
      }
    } catch (e) {}
  }
  var logs = [];
  var MAX = 500;
  function fmt(v) {
    try {
      if (typeof v === 'string') return v;
      if (v instanceof Error) return v.stack || (v.name + ': ' + v.message);
      var s = JSON.stringify(v);
      return s === undefined ? String(v) : s;
    } catch (e) { return String(v); }
  }
  function push(level, args) {
    var text = Array.prototype.map.call(args, fmt).join(' ');
    if (text.length > 4000) text = text.slice(0, 4000) + '…';
    logs.push({ level: level, text: text, at: Date.now(), url: location.href });
    if (logs.length > MAX) logs.splice(0, logs.length - MAX);
  }
  ['log', 'info', 'warn', 'error', 'debug'].forEach(function (level) {
    var orig = console[level];
    if (typeof orig !== 'function') return;
    console[level] = function () {
      try { push(level, arguments); } catch (e) {}
      return orig.apply(this, arguments);
    };
  });
  window.addEventListener('error', function (e) {
    var t = e.target;
    if (t && t !== window && t.tagName) {
      push('error', ['Failed to load ' + t.tagName.toLowerCase() + ': ' + (t.currentSrc || t.src || t.href || '')]);
    } else {
      push('error', [(e.message || 'Error') + (e.filename ? ' (' + e.filename + ':' + e.lineno + ':' + e.colno + ')' : '')]);
    }
  }, true);
  window.addEventListener('unhandledrejection', function (e) {
    push('error', ['Unhandled promise rejection: ' + fmt(e.reason)]);
  });
  var last = '';
  function status() {
    var key = location.href + '\n' + document.title;
    if (key === last) return;
    last = key;
    send('status', null, { url: location.href, title: document.title });
  }
  ['pushState', 'replaceState'].forEach(function (m) {
    var o = history[m];
    history[m] = function () {
      var r = o.apply(this, arguments);
      setTimeout(status, 0);
      return r;
    };
  });
  window.addEventListener('popstate', status);
  window.addEventListener('hashchange', status);
  Object.defineProperty(window, '__omnigetPreview', {
    value: { send: send, logs: logs, refs: new Map() },
    enumerable: false, configurable: false, writable: false
  });
})();"#;

/// Wraps a JS function expression so its (possibly async) result is sent back
/// as `{ok, value}` / `{ok:false, error}` under `id`.
pub fn wrap(id: &str, func: &str, args: &Value) -> String {
    let id = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into());
    let args = serde_json::to_string(args).unwrap_or_else(|_| "null".into());
    format!(
        r#"(function () {{
  var ID = {id};
  var P = window.__omnigetPreview;
  var T = window.__TAURI_INTERNALS__;
  function done(d) {{
    if (P) {{ P.send('result', ID, d); return; }}
    try {{ if (T && T.invoke) T.invoke('preview_report', {{ kind: 'result', id: ID, data: d }}); }} catch (e) {{}}
  }}
  Promise.resolve().then(function () {{ return ({func})({args}); }}).then(
    function (v) {{ done({{ ok: true, value: v === undefined ? null : v }}); }},
    function (e) {{ done({{ ok: false, error: String((e && e.message) || e) }}); }}
  );
}})();"#
    )
}

/// Shared helpers: `find(selector)` (CSS, `e12` / `ref=e12` from the last
/// snapshot, or `text=Label`) and `describe(el)`.
const HELPERS: &str = r#"
  var P = window.__omnigetPreview || { refs: new Map() };
  function clean(s, n) { s = (s || '').replace(/\s+/g, ' ').trim(); return s.length > n ? s.slice(0, n) + '…' : s; }
  function find(sel) {
    sel = String(sel || '').trim();
    if (!sel) throw new Error('selector is empty');
    var m = sel.match(/^(?:ref=)?(e\d+)$/);
    if (m) {
      var hit = P.refs && P.refs.get(m[1]);
      if (hit && hit.isConnected) return hit;
      throw new Error('ref ' + m[1] + ' is gone; take a new preview_snapshot');
    }
    if (sel.indexOf('text=') === 0) {
      var want = sel.slice(5).trim().replace(/^["']|["']$/g, '').toLowerCase();
      var all = document.querySelectorAll('a,button,input[type=button],input[type=submit],[role=button],[role=link],[role=tab],[role=menuitem],[role=option],label,summary,option,li,td,th,h1,h2,h3,h4,h5,h6,p,span,div');
      var best = null;
      for (var i = 0; i < all.length; i++) {
        var t = clean(all[i].innerText || all[i].value || '', 400).toLowerCase();
        if (t === want) return all[i];
        if (!best && t.indexOf(want) >= 0 && t.length < want.length + 40) best = all[i];
      }
      if (best) return best;
      throw new Error('No element with text ' + JSON.stringify(want));
    }
    var el = document.querySelector(sel);
    if (!el) throw new Error('No element matches ' + sel);
    return el;
  }
  function describe(el) {
    var d = el.tagName.toLowerCase();
    if (el.id) d += '#' + el.id;
    var txt = clean(el.innerText || el.value || el.getAttribute('aria-label') || '', 60);
    return txt ? d + ' "' + txt + '"' : d;
  }
"#;

pub fn snapshot_fn() -> String {
    format!(
        r#"function (a) {{
{HELPERS}
  var refs = new Map();
  P.refs = refs;
  var max = a.maxNodes || 1500;
  var count = 0;
  var lines = [];
  var LEAF = /^(link|button|heading|textbox|searchbox|checkbox|radio|combobox|slider|img|option|tab|menuitem|switch|spinbutton|progressbar|meter)$/;
  var IMPLICIT = {{ BUTTON: 'button', SELECT: 'combobox', TEXTAREA: 'textbox', IMG: 'img', NAV: 'navigation', MAIN: 'main', HEADER: 'banner', FOOTER: 'contentinfo', ASIDE: 'complementary', FORM: 'form', UL: 'list', OL: 'list', LI: 'listitem', TABLE: 'table', TR: 'row', TH: 'columnheader', TD: 'cell', DIALOG: 'dialog', SUMMARY: 'button', LABEL: 'label', OPTION: 'option', P: 'paragraph', ARTICLE: 'article', PROGRESS: 'progressbar', METER: 'meter' }};
  function role(el) {{
    var r = el.getAttribute('role');
    if (r) return r.split(' ')[0];
    var t = el.tagName;
    if (t === 'A') return el.hasAttribute('href') ? 'link' : null;
    if (t === 'INPUT') {{
      var ty = (el.type || 'text').toLowerCase();
      var map = {{ checkbox: 'checkbox', radio: 'radio', button: 'button', submit: 'button', reset: 'button', image: 'button', range: 'slider', search: 'searchbox', number: 'spinbutton', hidden: '' }};
      return ty in map ? (map[ty] || null) : 'textbox';
    }}
    if (/^H[1-6]$/.test(t)) return 'heading';
    if (t === 'SECTION') return el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby') ? 'region' : null;
    if (el.isContentEditable && el.getAttribute('contenteditable') !== null) return 'textbox';
    return IMPLICIT[t] || null;
  }}
  function visible(el) {{
    if (el.getAttribute('aria-hidden') === 'true') return false;
    var s = getComputedStyle(el);
    if (s.display === 'none' || s.visibility === 'hidden' || s.visibility === 'collapse') return false;
    if (s.display !== 'contents' && s.position !== 'fixed' && el.getClientRects().length === 0) return false;
    return true;
  }}
  function labelOf(el) {{
    if (el.id) {{
      var lab = document.querySelector('label[for="' + (window.CSS && CSS.escape ? CSS.escape(el.id) : el.id) + '"]');
      if (lab) return lab.innerText;
    }}
    var pl = el.closest('label');
    return pl ? pl.innerText : '';
  }}
  function name(el, r) {{
    var l = el.getAttribute('aria-label');
    if (l) return clean(l, 100);
    var lb = el.getAttribute('aria-labelledby');
    if (lb) {{
      var txt = lb.split(/\s+/).map(function (id) {{ var x = document.getElementById(id); return x ? (x.innerText || x.textContent) : ''; }}).join(' ');
      if (txt.trim()) return clean(txt, 100);
    }}
    if (el.tagName === 'IMG') return clean(el.alt || el.title, 100);
    if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT') {{
      if (el.type === 'submit' || el.type === 'button' || el.type === 'reset') return clean(el.value || el.title, 100);
      return clean(labelOf(el) || el.getAttribute('placeholder') || el.title || el.name, 100);
    }}
    if (LEAF.test(r)) return clean(el.innerText || el.textContent || el.title, 100);
    return '';
  }}
  function walk(node, depth) {{
    for (var c = node.firstChild; c; c = c.nextSibling) {{
      if (count >= max) return;
      if (c.nodeType === 3) {{
        var tx = clean(c.textContent, 200);
        if (tx.length > 1) {{ count++; lines.push('  '.repeat(depth) + '- text: ' + JSON.stringify(tx)); }}
        continue;
      }}
      if (c.nodeType !== 1) continue;
      var tag = c.tagName;
      if (tag === 'SCRIPT' || tag === 'STYLE' || tag === 'NOSCRIPT' || tag === 'TEMPLATE' || tag === 'HEAD') continue;
      if (!visible(c)) continue;
      var r = role(c);
      var next = depth;
      if (r) {{
        count++;
        var ref = 'e' + count;
        refs.set(ref, c);
        var parts = ['- ' + r];
        var n = name(c, r);
        if (n) parts.push(JSON.stringify(n));
        if (r === 'heading') {{
          var lv = /^H([1-6])$/.exec(tag);
          parts.push('[level=' + (lv ? lv[1] : (c.getAttribute('aria-level') || '?')) + ']');
        }}
        if (tag === 'INPUT' && (c.type === 'checkbox' || c.type === 'radio')) parts.push(c.checked ? '[checked]' : '[unchecked]');
        else if ((tag === 'INPUT' || tag === 'TEXTAREA') && c.type !== 'password' && c.value) parts.push('value=' + JSON.stringify(clean(c.value, 80)));
        if (tag === 'SELECT' && c.selectedOptions && c.selectedOptions[0]) parts.push('value=' + JSON.stringify(clean(c.selectedOptions[0].text, 60)));
        if (c.disabled || c.getAttribute('aria-disabled') === 'true') parts.push('[disabled]');
        if (c.getAttribute('aria-expanded')) parts.push('[expanded=' + c.getAttribute('aria-expanded') + ']');
        if (r === 'link' && c.getAttribute('href')) parts.push('href=' + JSON.stringify(clean(c.getAttribute('href'), 120)));
        parts.push('[ref=' + ref + ']');
        lines.push('  '.repeat(depth) + parts.join(' '));
        if (LEAF.test(r)) continue;
        next = depth + 1;
      }}
      walk(c.shadowRoot || c, next);
    }}
  }}
  if (document.body) walk(document.body, 0);
  var maxChars = a.maxChars || 20000;
  var text = document.body ? (document.body.innerText || '') : '';
  var out = {{
    url: location.href,
    title: document.title,
    tree: lines.join('\n'),
    nodes: count,
    truncated: count >= max,
    text: text.length > maxChars ? text.slice(0, maxChars) + '…' : text,
    viewport: {{ width: innerWidth, height: innerHeight, scrollY: Math.round(scrollY), scrollHeight: document.documentElement.scrollHeight }}
  }};
  if (a.includeHtml) {{
    var html = document.documentElement.outerHTML;
    out.html = html.length > maxChars ? html.slice(0, maxChars) + '…' : html;
  }}
  return out;
}}"#
    )
}

pub fn click_fn() -> String {
    format!(
        r#"function (a) {{
{HELPERS}
  var el = find(a.selector);
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  var r = el.getBoundingClientRect();
  var o = {{ bubbles: true, cancelable: true, composed: true, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, button: 0, view: window }};
  var po = Object.assign({{ pointerId: 1, pointerType: 'mouse', isPrimary: true }}, o);
  try {{ el.dispatchEvent(new PointerEvent('pointerdown', po)); }} catch (e) {{}}
  el.dispatchEvent(new MouseEvent('mousedown', o));
  if (el.focus) el.focus({{ preventScroll: true }});
  try {{ el.dispatchEvent(new PointerEvent('pointerup', po)); }} catch (e) {{}}
  el.dispatchEvent(new MouseEvent('mouseup', o));
  el.click();
  if (a.double) el.dispatchEvent(new MouseEvent('dblclick', o));
  return {{ clicked: describe(el), url: location.href }};
}}"#
    )
}

pub fn type_fn() -> String {
    format!(
        r#"function (a) {{
{HELPERS}
  var el = find(a.selector);
  el.scrollIntoView({{ block: 'center' }});
  if (el.focus) el.focus();
  var text = a.text == null ? '' : String(a.text);
  if (el.isContentEditable) {{
    if (a.clear !== false) document.execCommand('selectAll', false);
    document.execCommand('insertText', false, text);
  }} else if ('value' in el && el.tagName !== 'BUTTON') {{
    var proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : el.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    var desc = Object.getOwnPropertyDescriptor(proto, 'value');
    var v = a.clear === false ? el.value + text : text;
    if (desc && desc.set) desc.set.call(el, v); else el.value = v;
    try {{ el.dispatchEvent(new InputEvent('input', {{ bubbles: true, composed: true, data: text, inputType: 'insertText' }})); }}
    catch (e) {{ el.dispatchEvent(new Event('input', {{ bubbles: true }})); }}
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  }} else {{
    throw new Error(describe(el) + ' is not editable');
  }}
  if (a.submit) {{
    var kd = {{ key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true }};
    var go = el.dispatchEvent(new KeyboardEvent('keydown', kd));
    el.dispatchEvent(new KeyboardEvent('keypress', kd));
    el.dispatchEvent(new KeyboardEvent('keyup', kd));
    if (go && el.form) {{ if (el.form.requestSubmit) el.form.requestSubmit(); else el.form.submit(); }}
  }}
  var secret = el.type === 'password';
  return {{ typed: text.length, into: describe(el), value: secret || !('value' in el) ? null : el.value }};
}}"#
    )
}

pub const CONSOLE_FN: &str = r#"function (a) {
  var P = window.__omnigetPreview;
  if (!P) return { entries: [], total: 0, note: 'console capture is not installed on this page' };
  var list = P.logs.slice();
  if (a.clear !== false) P.logs.splice(0);
  var lv = a.level;
  if (lv === 'error') list = list.filter(function (e) { return e.level === 'error'; });
  else if (lv === 'warn') list = list.filter(function (e) { return e.level === 'error' || e.level === 'warn'; });
  else if (lv) list = list.filter(function (e) { return e.level === lv; });
  var lim = a.limit || 200;
  return { entries: list.slice(-lim), total: list.length, url: location.href };
}"#;

pub const HISTORY_FN: &str = r#"function (a) {
  if (a.action === 'back') history.back();
  else if (a.action === 'forward') history.forward();
  else location.reload();
  return { ok: true };
}"#;
