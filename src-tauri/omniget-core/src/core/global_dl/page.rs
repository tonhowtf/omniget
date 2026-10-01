//! Driving a page like a person, for agents: an accessibility-style snapshot
//! with refs, then real (trusted) mouse and keyboard input through CDP on the
//! element a ref names. Same ref grammar as the app's Browser preview
//! (`e12`, a CSS selector, or `text=Label`).

use std::time::Duration;

use serde_json::{json, Value};

use super::browser::{self, Browser};

/// Shared by every page function: `find(sel)` resolves a ref, `text=` or a
/// CSS selector.
const HELPERS: &str = r#"
  var G = window.__gdPage || (window.__gdPage = { refs: new Map() });
  function clean(s, n) { s = (s || '').replace(/\s+/g, ' ').trim(); return s.length > n ? s.slice(0, n) + '…' : s; }
  function find(sel) {
    sel = String(sel || '').trim();
    if (!sel) throw new Error('selector is empty');
    var m = sel.match(/^(?:ref=)?(e\d+)$/);
    if (m) {
      var hit = G.refs.get(m[1]);
      if (hit && hit.isConnected) return hit;
      throw new Error('ref ' + m[1] + ' is gone; take a new gd_snapshot');
    }
    if (sel.indexOf('text=') === 0) {
      var want = sel.slice(5).trim().replace(/^["']|["']$/g, '').toLowerCase();
      var all = document.querySelectorAll('a,button,input[type=button],input[type=submit],[role=button],[role=link],[role=tab],[role=menuitem],[role=option],[role=checkbox],label,summary,option,li,td,th,h1,h2,h3,h4,h5,h6,p,span,div');
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

const SNAPSHOT: &str = r#"
  var refs = new Map(); G.refs = refs;
  var max = A.maxNodes || 1500, count = 0, lines = [];
  var LEAF = /^(link|button|heading|textbox|searchbox|checkbox|radio|combobox|slider|img|option|tab|menuitem|switch|spinbutton|progressbar|meter)$/;
  var IMPLICIT = { BUTTON: 'button', SELECT: 'combobox', TEXTAREA: 'textbox', IMG: 'img', NAV: 'navigation', MAIN: 'main', HEADER: 'banner', FOOTER: 'contentinfo', ASIDE: 'complementary', FORM: 'form', UL: 'list', OL: 'list', LI: 'listitem', TABLE: 'table', TR: 'row', TH: 'columnheader', TD: 'cell', DIALOG: 'dialog', SUMMARY: 'button', LABEL: 'label', OPTION: 'option', P: 'paragraph', ARTICLE: 'article', VIDEO: 'video', AUDIO: 'audio', IFRAME: 'iframe' };
  function role(el) {
    var r = el.getAttribute('role'); if (r) return r.split(' ')[0];
    var t = el.tagName;
    if (t === 'A') return el.hasAttribute('href') ? 'link' : null;
    if (t === 'INPUT') {
      var ty = (el.type || 'text').toLowerCase();
      var map = { checkbox: 'checkbox', radio: 'radio', button: 'button', submit: 'button', reset: 'button', image: 'button', range: 'slider', search: 'searchbox', number: 'spinbutton', hidden: '', file: 'button' };
      return ty in map ? (map[ty] || null) : 'textbox';
    }
    if (/^H[1-6]$/.test(t)) return 'heading';
    if (el.isContentEditable && el.getAttribute('contenteditable') !== null) return 'textbox';
    return IMPLICIT[t] || null;
  }
  function visible(el) {
    if (el.getAttribute('aria-hidden') === 'true') return false;
    var s = getComputedStyle(el);
    if (s.display === 'none' || s.visibility === 'hidden') return false;
    if (s.display !== 'contents' && s.position !== 'fixed' && el.getClientRects().length === 0) return false;
    return true;
  }
  function labelOf(el) {
    if (el.id) { var lab = document.querySelector('label[for="' + CSS.escape(el.id) + '"]'); if (lab) return lab.innerText; }
    var pl = el.closest('label'); return pl ? pl.innerText : '';
  }
  function name(el, r) {
    var l = el.getAttribute('aria-label'); if (l) return clean(l, 100);
    if (el.tagName === 'IMG') return clean(el.alt || el.title, 100);
    if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT') {
      if (el.type === 'submit' || el.type === 'button' || el.type === 'reset') return clean(el.value || el.title, 100);
      return clean(labelOf(el) || el.getAttribute('placeholder') || el.title || el.name, 100);
    }
    if (LEAF.test(r)) return clean(el.innerText || el.textContent || el.title, 100);
    return '';
  }
  function walk(node, depth) {
    for (var c = node.firstChild; c; c = c.nextSibling) {
      if (count >= max) return;
      if (c.nodeType === 3) { var tx = clean(c.textContent, 200); if (tx.length > 1) { count++; lines.push('  '.repeat(depth) + '- text: ' + JSON.stringify(tx)); } continue; }
      if (c.nodeType !== 1) continue;
      var tag = c.tagName;
      if (tag === 'SCRIPT' || tag === 'STYLE' || tag === 'NOSCRIPT' || tag === 'TEMPLATE' || tag === 'HEAD') continue;
      if (!visible(c)) continue;
      var r = role(c), next = depth;
      if (r) {
        count++; var ref = 'e' + count; refs.set(ref, c);
        var parts = ['- ' + r], n = name(c, r);
        if (n) parts.push(JSON.stringify(n));
        if (tag === 'INPUT' && (c.type === 'checkbox' || c.type === 'radio')) parts.push(c.checked ? '[checked]' : '[unchecked]');
        else if ((tag === 'INPUT' || tag === 'TEXTAREA') && c.type !== 'password' && c.value) parts.push('value=' + JSON.stringify(clean(c.value, 80)));
        if (tag === 'INPUT' && c.type === 'password') parts.push('[password]');
        if (c.disabled || c.getAttribute('aria-disabled') === 'true') parts.push('[disabled]');
        if (r === 'link' && c.getAttribute('href')) parts.push('href=' + JSON.stringify(clean(c.getAttribute('href'), 120)));
        parts.push('[ref=' + ref + ']');
        lines.push('  '.repeat(depth) + parts.join(' '));
        if (LEAF.test(r)) continue;
        next = depth + 1;
      }
      walk(c.shadowRoot || c, next);
    }
  }
  if (document.body) walk(document.body, 0);
  var maxChars = A.maxChars || 20000;
  var text = document.body ? (document.body.innerText || '') : '';
  return { url: location.href, title: document.title, tree: lines.join('\n'), nodes: count, truncated: count >= max,
    text: text.length > maxChars ? text.slice(0, maxChars) + '…' : text,
    viewport: { width: innerWidth, height: innerHeight, scrollY: Math.round(scrollY), scrollHeight: document.documentElement.scrollHeight } };
"#;

/// Scrolls the element into view and returns its center in CSS pixels.
const LOCATE: &str = r#"
  var el = find(A.selector);
  el.scrollIntoView({ block: 'center', inline: 'center' });
  var r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, h: r.height, what: describe(el),
    editable: el.isContentEditable || ('value' in el && el.tagName !== 'BUTTON'), tag: el.tagName };
"#;

const SELECT_ALL: &str = r#"
  var el = document.activeElement;
  if (el && typeof el.select === 'function') el.select();
  else document.execCommand('selectAll', false);
  return true;
"#;

fn wrap(body: &str, args: &Value) -> String {
    format!("(function(A){{ {HELPERS}\n{body} }})({args})")
}

async fn run(b: &Browser, tab: &str, body: &str, args: Value) -> Result<Value, String> {
    b.eval(tab, &wrap(body, &args), Duration::from_secs(30))
        .await
}

/// The tab an agent means: the one given, else the one shown in OmniGet,
/// else the most recent page.
pub fn pick_tab(b: &Browser, tab: Option<String>) -> Result<String, String> {
    if let Some(t) = tab.filter(|t| !t.is_empty()) {
        return Ok(t);
    }
    if let Some(v) = browser::status()["view"]["tab"].as_str() {
        return Ok(v.to_string());
    }
    b.tabs()
        .into_iter()
        .rev()
        .find(|t| !t.url.starts_with("chrome") && !t.url.starts_with("devtools"))
        .map(|t| t.id)
        .ok_or_else(|| "no tab is open".to_string())
}

pub async fn snapshot(
    b: &Browser,
    tab: &str,
    max_nodes: Option<u64>,
    max_chars: Option<u64>,
) -> Result<Value, String> {
    run(
        b,
        tab,
        SNAPSHOT,
        json!({ "maxNodes": max_nodes, "maxChars": max_chars }),
    )
    .await
}

/// Clicks with the real mouse at the element's center.
pub async fn click(b: &Browser, tab: &str, selector: &str, double: bool) -> Result<Value, String> {
    let at = run(b, tab, LOCATE, json!({ "selector": selector })).await?;
    let (x, y) = (
        at["x"].as_f64().unwrap_or(0.0),
        at["y"].as_f64().unwrap_or(0.0),
    );
    click_at(b, tab, x, y, if double { 2 } else { 1 }).await?;
    Ok(json!({ "clicked": at["what"], "x": x, "y": y }))
}

pub async fn click_at(b: &Browser, tab: &str, x: f64, y: f64, count: u32) -> Result<(), String> {
    b.mouse(tab, json!({ "type": "mouseMoved", "x": x, "y": y }))
        .await?;
    for n in 1..=count {
        b.mouse(tab, json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": n }))
            .await?;
        b.mouse(tab, json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": n }))
            .await?;
    }
    Ok(())
}

/// Focuses the field with a click and types as keyboard input.
pub async fn type_text(
    b: &Browser,
    tab: &str,
    selector: &str,
    text: &str,
    clear: bool,
    submit: bool,
) -> Result<Value, String> {
    let at = run(b, tab, LOCATE, json!({ "selector": selector })).await?;
    let (x, y) = (
        at["x"].as_f64().unwrap_or(0.0),
        at["y"].as_f64().unwrap_or(0.0),
    );
    click_at(b, tab, x, y, 1).await?;
    if clear {
        run(b, tab, SELECT_ALL, json!({})).await?;
        b.press(tab, "Backspace", 0).await?;
    }
    b.insert_text(tab, text).await?;
    if submit {
        b.press(tab, "Enter", 0).await?;
    }
    Ok(json!({ "typed": text.chars().count(), "into": at["what"] }))
}

/// `Enter`, `Tab`, `Escape`, `ArrowDown`, `a`, with `+` modifiers:
/// `Control+a`, `Meta+Shift+z`.
pub async fn press_combo(b: &Browser, tab: &str, combo: &str) -> Result<(), String> {
    let mut mods = 0u32;
    let mut key = combo.to_string();
    let parts: Vec<&str> = combo.split('+').collect();
    if parts.len() > 1 {
        for m in &parts[..parts.len() - 1] {
            mods |= match m.to_ascii_lowercase().as_str() {
                "alt" | "option" => 1,
                "control" | "ctrl" => 2,
                "meta" | "cmd" | "command" => 4,
                "shift" => 8,
                other => return Err(format!("unknown modifier {other}")),
            };
        }
        key = parts[parts.len() - 1].to_string();
    }
    b.press(tab, &key, mods).await
}

pub async fn scroll(b: &Browser, tab: &str, dy: f64, dx: f64) -> Result<Value, String> {
    let vp = run(
        b,
        tab,
        "return { w: innerWidth, h: innerHeight };",
        json!({}),
    )
    .await?;
    let (x, y) = (
        vp["w"].as_f64().unwrap_or(800.0) / 2.0,
        vp["h"].as_f64().unwrap_or(600.0) / 2.0,
    );
    b.mouse(
        tab,
        json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy }),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    run(b, tab, "return { scrollY: Math.round(scrollY), scrollHeight: document.documentElement.scrollHeight };", json!({})).await
}

/// Waits until `text` shows on the page or `selector` matches, up to
/// `timeout_ms`.
pub async fn wait_for(
    b: &Browser,
    tab: &str,
    text: Option<&str>,
    selector: Option<&str>,
    timeout_ms: u64,
) -> Result<Value, String> {
    let deadline =
        std::time::Instant::now() + Duration::from_millis(timeout_ms.clamp(100, 120_000));
    loop {
        let found = run(
            b,
            tab,
            "if (A.selector) { try { find(A.selector); return true; } catch (e) { return false; } } return !!(document.body && A.text && document.body.innerText.indexOf(A.text) >= 0);",
            json!({ "text": text, "selector": selector }),
        )
        .await
        .unwrap_or(Value::Bool(false));
        if found == Value::Bool(true) {
            return Ok(json!({ "found": true }));
        }
        if std::time::Instant::now() > deadline {
            return Ok(json!({ "found": false }));
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

/// Starts what the page has not played yet. Every `<video>` and `<audio>`,
/// in the page and in its same-origin iframes (the frames cat-catch's
/// content script walks, js/content-script.js), is muted and `play()`ed.
/// When nothing was playing, the first visible play button of a known player
/// (or any control labelled "Play") is located for a real click, or clicked
/// from script when something covers it.
const KICK: &str = r#"
  var docs = [document];
  document.querySelectorAll('iframe').forEach(function (f) {
    try { if (f.contentDocument) docs.push(f.contentDocument); } catch (e) {}
  });
  var media = 0, playing = 0;
  docs.forEach(function (d) {
    d.querySelectorAll('video,audio').forEach(function (m) {
      media++;
      try {
        if (!m.paused) { playing++; return; }
        m.muted = true;
        var p = m.play();
        if (p && p.catch) p.catch(function () {});
      } catch (e) {}
    });
  });
  if (playing > 0 || !A.click) return { media: media, playing: playing, click: null };
  // Known players first: their play controls are often links that a click
  // handler turns into playback (MediaWiki's .mw-tmh-play, SoundCloud's
  // a.sc-button-play). The generic ones after GENERIC must not navigate.
  var SEL = [
    '.vjs-big-play-button', '.jw-display-icon-display', '.plyr__control--overlaid',
    '.ytp-large-play-button', '.mw-tmh-play', '.playbutton', '.sc-button-play',
    '.audio-module-controls-play', '.mejs__overlay-button', '.fp-play',
    '.shaka-play-button', '[data-testid="play-button"]',
    'button[aria-label="Play" i]', '[role=button][aria-label="Play" i]',
    'button[aria-label^="Play " i]', 'button[title="Play" i]', 'button[class*="play" i]'
  ];
  var GENERIC = 12;
  var covered = null;
  var vw = innerWidth, vh = innerHeight;
  function seen(r) { return !(r.bottom < 0 || r.top > vh || r.right < 0 || r.left > vw); }
  for (var i = 0; i < SEL.length; i++) {
    var els = document.querySelectorAll(SEL[i]);
    for (var j = 0; j < els.length; j++) {
      var el = els[j];
      if (i >= GENERIC) {
        var a = el.closest('a[href]');
        if (a && !/^(#|javascript:|$)/i.test(a.getAttribute('href') || '')) continue;
      }
      var st = getComputedStyle(el);
      if (st.visibility === 'hidden' || st.display === 'none' || +st.opacity === 0) continue;
      var r = el.getBoundingClientRect();
      if (r.width < 8 || r.height < 8) continue;
      if (!seen(r)) {
        el.scrollIntoView({ block: 'center' });
        r = el.getBoundingClientRect();
        if (!seen(r)) continue;
      }
      var x = r.left + r.width / 2, y = r.top + r.height / 2;
      // A sign-in modal or a fixed footer on top would take a real click
      // (SoundCloud, Bandcamp): the control is then clicked from script.
      var hit = document.elementFromPoint(x, y);
      if (!hit || !(hit === el || el.contains(hit) || hit.contains(el))) {
        if (!covered) covered = { el: el, what: SEL[i] };
        continue;
      }
      return { media: media, playing: playing, click: { x: x, y: y, what: SEL[i] } };
    }
  }
  if (covered) {
    covered.el.click();
    return { media: media, playing: playing, click: null, scripted: covered.what };
  }
  return { media: media, playing: playing, click: null };
"#;

/// Gets a page's player going so the sniffer sees its media: muted `play()`
/// on every media element, then, when nothing plays, one trusted click on
/// the play button (a click from script does not count as a gesture where
/// autoplay needs one), then `play()` again for the element the click
/// created.
pub async fn start_playback(b: &Browser, tab: &str) -> Result<Value, String> {
    let r = run(b, tab, KICK, json!({ "click": true })).await?;
    if let Some(c) = r.get("click").filter(|c| c.is_object()) {
        let (x, y) = (
            c["x"].as_f64().unwrap_or(0.0),
            c["y"].as_f64().unwrap_or(0.0),
        );
        click_at(b, tab, x, y, 1).await?;
    }
    if r.get("click").is_some_and(Value::is_object) || r.get("scripted").is_some() {
        tokio::time::sleep(Duration::from_millis(800)).await;
        let _ = run(b, tab, KICK, json!({ "click": false })).await;
    }
    Ok(r)
}

/// Waits for the tab to stop loading (or `timeout`).
pub async fn wait_loaded(b: &Browser, tab: &str, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    tokio::time::sleep(Duration::from_millis(300)).await;
    while std::time::Instant::now() < deadline {
        let ready = b
            .eval(tab, "document.readyState", Duration::from_secs(5))
            .await
            .ok()
            .and_then(|v| v.as_str().map(|s| s == "complete"))
            .unwrap_or(false);
        if ready && !b.tab(tab).map(|t| t.loading).unwrap_or(false) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
