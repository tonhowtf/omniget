use std::path::PathBuf;

use serde_json::{json, Value};

use super::harness::{self, SnapOpts};
use super::{browser, connector, jobs, market, page, profiles, sniffer, Node, Source};
use crate::core::llm::tool_table::{
    list, need_str, num, s, to_json, BoxFut, CostHint, ToolEntry, ToolImpl,
};

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

fn e(
    name: &'static str,
    description: &'static str,
    schema: Value,
    call: fn(Value) -> BoxFut,
    cost: CostHint,
) -> ToolEntry {
    ToolEntry {
        name,
        description,
        input_schema: schema,
        call: ToolImpl::Core(call),
        category: "downloads",
        cost_hint: cost,
        platforms: &[],
        feature: None,
        risk: None,
        always_ask: false,
    }
}

pub fn entries() -> Vec<ToolEntry> {
    use CostHint::*;
    vec![
        e("gd_browser_open",
          "Open the Global Downloader browser (Chromium on OmniGet's own profile) and optionally a URL in a new tab; every tab's network traffic is recorded. By default it is embedded: pages only show inside OmniGet's Global Downloader screen. Pass window=true when the person has to see the page or log in (sites like Google refuse embedded logins): it becomes a separate window on their screen and stays so. Returns the tabs and the mode.",
          obj(json!({ "url": { "type": "string" }, "window": { "type": "boolean", "description": "true: a visible browser window the person can use" } }), &[]),
          |a| Box::pin(async move {
              let url = Some(s(&a, "url")).filter(|u| !u.is_empty());
              let attached = browser::current().is_some_and(|b| b.attached);
              if a["window"].as_bool() == Some(true) && browser::mode() != "window" && !attached {
                  browser::ensure_no_running_job(false)?;
                  browser::switch_mode("window").await?;
              }
              let b = browser::start(url, true).await?;
              let mode = if b.headless { "embedded" } else { "window" };
              to_json(json!({ "tabs": b.tabs(), "mode": mode }))
          }), Local),
        e("gd_tabs",
          "List, open, close, focus or navigate tabs of the Global Downloader browser. action: list (default) | open (url) | close (tab_id) | focus (tab_id) | navigate (tab_id, url).",
          obj(json!({ "action": { "type": "string", "enum": ["list", "open", "open_private", "close", "focus", "navigate"] }, "tab_id": { "type": "string" }, "url": { "type": "string" } }), &[]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              match s(&a, "action").as_str() {
                  "open" => { b.open_tab(&browser::address_to_url(&need_str(&a, "url")?)).await?; }
                  "open_private" => { b.open_private_tab(&browser::address_to_url(&need_str(&a, "url")?)).await?; }
                  "close" => b.close_tab(&need_str(&a, "tab_id")?).await?,
                  "focus" => b.activate_tab(&need_str(&a, "tab_id")?).await?,
                  "navigate" => b.navigate(&need_str(&a, "tab_id")?, &browser::address_to_url(&need_str(&a, "url")?)).await?,
                  _ => {}
              }
              tokio::time::sleep(std::time::Duration::from_millis(300)).await;
              to_json(json!({ "tabs": b.tabs() }))
          }), Cheap),
        e("gd_snapshot",
          "Read the page in a browser tab (default: the tab shown in OmniGet) as an indented role/name tree whose refs (e12) work in gd_click, gd_type and gd_act, plus the visible text, URL and title; needs_human reports a captcha, bot check or login wall. Use it to find elements before acting, and keep it small with maxTokens, role, text or scope, or diff=true to get only what changed since the last snapshot of that tab.",
          obj(json!({ "tab_id": { "type": "string" }, "maxTokens": { "type": "integer", "minimum": 1, "description": "budget for the tree and text (chars/4); the result says truncated, total and returned" }, "role": { "type": "string", "description": "keep only lines with this role (button, link, textbox, heading...)" }, "text": { "type": "string", "description": "keep only lines containing this text" }, "scope": { "type": "string", "description": "a ref (e12) or CSS selector: snapshot only that subtree" }, "diff": { "type": "boolean", "description": "return only added, changed and removed lines since the previous snapshot of this tab" }, "max_nodes": { "type": "integer", "maximum": 10000 }, "max_chars": { "type": "integer", "maximum": 200000 } }), &[]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(tab_arg(&a)))?;
              let text = |k: &str| Some(s(&a, k)).filter(|v| !v.trim().is_empty());
              let opts = SnapOpts {
                  max_tokens: num(&a, "maxTokens"),
                  role: text("role"),
                  text: text("text"),
                  diff: a["diff"].as_bool().unwrap_or(false),
              };
              let mut v = page::snapshot(&b, &tab, num(&a, "max_nodes"), num(&a, "max_chars"), text("scope").as_deref(), &opts).await?;
              v["tab_id"] = json!(tab);
              Ok(v)
          }), Cheap),
        e("gd_act",
          "Run several browser steps in order in one call (click, type, press, scroll, navigate, wait, eval; each step takes the arguments of gd_click, gd_type, gd_press, gd_scroll, gd_navigate, gd_wait and gd_page_eval) and return, per step, the result plus outcome evidence: confirmed, no_effect or unknown, and what changed (url, dom, focus, value, scroll). Use it instead of single-step calls for any flow of two or more steps, such as filling and submitting a form; it stops at the first failed step unless stopOnError=false, and at a captcha or login wall (needs_human).",
          obj(json!({ "steps": { "type": "array", "minItems": 1, "maxItems": 30, "items": { "type": "object", "properties": { "op": { "type": "string", "enum": ["click", "type", "press", "scroll", "navigate", "wait", "eval"] }, "selector": { "type": "string" }, "text": { "type": "string" }, "key": { "type": "string" }, "url": { "type": "string" }, "expression": { "type": "string" } }, "required": ["op"] } }, "stopOnError": { "type": "boolean", "description": "default true" }, "tab_id": { "type": "string" } }), &["steps"]),
          |a| Box::pin(act(a)), Local),
        e("gd_cdp",
          "Send one raw Chrome DevTools Protocol command (method, params) to a tab, or to the browser for Browser.* and Target.* methods, and return its result; results over about 8000 characters come back as a preview with truncated=true and the path of a file holding the full JSON. Use it only for what no gd_* tool covers (emulation, tracing, coverage, PDF, accessibility tree); methods that close or crash the browser, wipe all cookies or read system info are refused.",
          obj(json!({ "method": { "type": "string", "description": "Domain.method, e.g. Page.printToPDF" }, "params": { "type": "object" }, "tabId": { "type": "string" } }), &["method"]),
          |a| Box::pin(async move {
              let method = need_str(&a, "method")?;
              if !harness::cdp_method_allowed(&method) {
                  return Err(json!({
                      "code": "CDP_METHOD_DENIED",
                      "message": format!("{method} is not allowed through gd_cdp"),
                      "retryable": false,
                      "side_effect": "none",
                      "next_step": "Use Domain.method names. Browser.close, Browser.crash, Target.disposeBrowserContext, Storage.clearCookies, Network.clearBrowserCookies and SystemInfo.* are refused; close tabs with gd_tabs and remove single cookies with Network.deleteCookies.",
                  }).to_string());
              }
              let b = browser::require()?;
              let params = a.get("params").filter(|p| p.is_object()).cloned().unwrap_or_else(|| json!({}));
              let tab = tab_arg(&a);
              let session = if tab.is_empty() && harness::cdp_browser_level(&method) {
                  None
              } else {
                  Some(b.session_for(&page::pick_tab(&b, Some(tab))?).await?)
              };
              let r = b.cdp.call(&method, params, session.as_deref()).await?;
              let (out, full) = harness::cap_result(r, harness::RESULT_CAP_CHARS);
              match full {
                  Some(full) => with_file(out, full),
                  None => Ok(json!({ "result": out, "truncated": false })),
              }
          }), Local),
        e("gd_browser_attach",
          "Attach the Global Downloader to a Chrome the person already runs, with their logins, instead of launching one: from then on every gd_* tool works on that browser, and it is never closed by OmniGet (detach=true lets go of it). Use it when a site needs the person's real session; if Chrome is not debuggable the error has code CHROME_NOT_DEBUGGABLE and a next_step telling how to turn remote debugging on.",
          obj(json!({ "port": { "type": "integer", "minimum": 1, "maximum": 65535, "description": "remote debugging port (default 9222)" }, "endpoint": { "type": "string", "description": "ws://... browser endpoint, or http://host:port to discover it" }, "detach": { "type": "boolean" } }), &[]),
          |a| Box::pin(async move {
              if a["detach"].as_bool() == Some(true) {
                  let was = browser::current().is_some_and(|b| b.attached);
                  if was {
                      browser::stop().await?;
                  }
                  return Ok(json!({ "attached": false, "detached": was }));
              }
              let b = browser::attach_external(harness::attach_target(&a)?).await?;
              let product = b.cdp.call("Browser.getVersion", json!({}), None).await.map(|v| v["product"].clone()).unwrap_or(Value::Null);
              let tabs = b.tabs();
              to_json(json!({ "attached": true, "product": product, "tabs_total": tabs.len(), "tabs": tabs.into_iter().take(30).collect::<Vec<_>>() }))
          }), Local),
        e("gd_click",
          "Click an element with the real mouse (the page sees a trusted click). selector: a ref from gd_snapshot (e12), text=Label, or a CSS selector. Or x/y in CSS pixels.",
          obj(json!({ "selector": { "type": "string" }, "x": { "type": "number" }, "y": { "type": "number" }, "double": { "type": "boolean" }, "tab_id": { "type": "string" } }), &[]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(s(&a, "tab_id")))?;
              let double = a.get("double").and_then(Value::as_bool).unwrap_or(false);
              let sel = s(&a, "selector");
              let r = if sel.is_empty() {
                  let (x, y) = (a.get("x").and_then(Value::as_f64).ok_or("selector or x/y is required")?, a.get("y").and_then(Value::as_f64).ok_or("y is required")?);
                  page::click_at(&b, &tab, x, y, if double { 2 } else { 1 }).await?;
                  json!({ "clicked": [x, y] })
              } else {
                  page::click(&b, &tab, &sel, double).await?
              };
              page::wait_loaded(&b, &tab, std::time::Duration::from_secs(8)).await;
              Ok(r)
          }), Local),
        e("gd_type",
          "Type into a field as keyboard input: clicks it, optionally clears it (default true), types the text, and presses Enter with submit.",
          obj(json!({ "selector": { "type": "string" }, "text": { "type": "string" }, "clear": { "type": "boolean" }, "submit": { "type": "boolean" }, "tab_id": { "type": "string" } }), &["selector", "text"]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(s(&a, "tab_id")))?;
              let clear = a.get("clear").and_then(Value::as_bool).unwrap_or(true);
              let submit = a.get("submit").and_then(Value::as_bool).unwrap_or(false);
              let r = page::type_text(&b, &tab, &need_str(&a, "selector")?, &s(&a, "text"), clear, submit).await?;
              if submit { page::wait_loaded(&b, &tab, std::time::Duration::from_secs(10)).await; }
              Ok(r)
          }), Local),
        e("gd_press",
          "Press a key or combination in the tab: Enter, Tab, Escape, ArrowDown, PageDown, a, Control+a, Meta+Shift+z.",
          obj(json!({ "key": { "type": "string" }, "tab_id": { "type": "string" } }), &["key"]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(s(&a, "tab_id")))?;
              page::press_combo(&b, &tab, &need_str(&a, "key")?).await?;
              Ok(json!({ "pressed": s(&a, "key") }))
          }), Local),
        e("gd_scroll",
          "Scroll the page with the mouse wheel: dy positive goes down (pixels, default 600).",
          obj(json!({ "dy": { "type": "number" }, "dx": { "type": "number" }, "tab_id": { "type": "string" } }), &[]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(s(&a, "tab_id")))?;
              page::scroll(&b, &tab, a.get("dy").and_then(Value::as_f64).unwrap_or(600.0), a.get("dx").and_then(Value::as_f64).unwrap_or(0.0)).await
          }), Cheap),
        e("gd_navigate",
          "Open a URL (or search words) in a tab, or go back/forward/reload. Waits for the page to load and returns its URL and title.",
          obj(json!({ "url": { "type": "string" }, "action": { "type": "string", "enum": ["back", "forward", "reload"] }, "tab_id": { "type": "string" } }), &[]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(s(&a, "tab_id")))?;
              match s(&a, "action").as_str() {
                  "" => b.navigate(&tab, &browser::address_to_url(&need_str(&a, "url")?)).await?,
                  act => b.history(&tab, act).await?,
              }
              page::wait_loaded(&b, &tab, std::time::Duration::from_secs(20)).await;
              let t = b.tab(&tab).unwrap_or_default();
              Ok(json!({ "tab_id": tab, "url": t.url, "title": t.title }))
          }), Network),
        e("gd_wait",
          "Wait until text appears on the page or a selector matches (timeout_ms, default 10000).",
          obj(json!({ "text": { "type": "string" }, "selector": { "type": "string" }, "timeout_ms": { "type": "integer" }, "tab_id": { "type": "string" } }), &[]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(s(&a, "tab_id")))?;
              let text = Some(s(&a, "text")).filter(|t| !t.is_empty());
              let sel = Some(s(&a, "selector")).filter(|t| !t.is_empty());
              page::wait_for(&b, &tab, text.as_deref(), sel.as_deref(), num(&a, "timeout_ms").unwrap_or(10_000)).await
          }), Cheap),
        e("gd_screenshot",
          "PNG screenshot of a browser tab's viewport; returns {path, bytes}.",
          obj(json!({ "tab_id": { "type": "string" } }), &[]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(s(&a, "tab_id")))?;
              let png = b.screenshot(&tab).await?;
              let dir = super::data_dir().join("screenshots");
              std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
              let path = dir.join(format!("{}.png", chrono::Utc::now().format("%Y%m%d-%H%M%S%3f")));
              std::fs::write(&path, &png).map_err(|e| e.to_string())?;
              Ok(json!({ "path": path, "bytes": png.len() }))
          }), Local),
        e("gd_browser_sources",
          "Browsers on this computer whose logins the Global Downloader can take (Chrome, Edge, Brave, Chromium, Vivaldi, Opera, Arc, Firefox), with their profiles, and what was imported last.",
          obj(json!({}), &[]),
          |_| Box::pin(async move { to_json(json!({ "sources": profiles::sources(), "imported": profiles::imported() })) }), Cheap),
        e("gd_browser_import",
          "Bring a browser profile's logins into the Global Downloader browser (restarts it). Chromium browsers: the whole profile (cookies, saved passwords, extensions, history); Firefox: cookies. The source browser is only read.",
          obj(json!({ "source": { "type": "string", "description": "id from gd_browser_sources (chrome, edge, brave, firefox...)" }, "profile": { "type": "string", "description": "profile dir or name; default the first" } }), &["source"]),
          |a| Box::pin(async move { profiles::import(&need_str(&a, "source")?, &s(&a, "profile")).await }), Local),
        e("gd_network_log",
          "Requests the browser tabs made, newest last: url, method, status, mime, resource type, request headers and post data. Filter by tab_id, a substring of the URL (contains) and resource types (XHR, Fetch, Document, Media, Script...). Use it to find a site's API before writing a connector.",
          obj(json!({ "tab_id": { "type": "string" }, "contains": { "type": "string" }, "types": { "type": "array", "items": { "type": "string" } }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 }, "headers": { "type": "boolean", "description": "include request headers (default false)" } }), &[]),
          |a| Box::pin(async move {
              let tab = Some(s(&a, "tab_id")).filter(|t| !t.is_empty());
              let contains = Some(s(&a, "contains")).filter(|t| !t.is_empty());
              let limit = num(&a, "limit").unwrap_or(100) as usize;
              let with_headers = a.get("headers").and_then(Value::as_bool).unwrap_or(false);
              let mut rows = sniffer::log(tab.as_deref(), contains.as_deref(), &list(&a, "types"), limit);
              if !with_headers {
                  for r in &mut rows { r.request_headers.clear(); }
              }
              to_json(json!({ "requests": rows }))
          }), Cheap),
        e("gd_response_body",
          "The body of one response from gd_network_log (by request_id), while the tab that made it is still open. Truncated to max_chars (default 20000).",
          obj(json!({ "request_id": { "type": "string" }, "max_chars": { "type": "integer" } }), &["request_id"]),
          |a| Box::pin(async move {
              let rid = need_str(&a, "request_id")?;
              let entry = sniffer::log_entry(&rid).ok_or("unknown request_id")?;
              let body = browser::require()?.response_body(&entry.session, &rid).await?;
              let max = num(&a, "max_chars").unwrap_or(20_000) as usize;
              let truncated = body.chars().count() > max;
              let body: String = body.chars().take(max).collect();
              to_json(json!({ "url": entry.url, "mime": entry.mime, "body": body, "truncated": truncated }))
          }), Cheap),
        e("gd_page_eval",
          "Evaluate a JavaScript expression in a browser tab (promises are awaited) and return its JSON value. Use it to read state a page already has (window.__NEXT_DATA__, player config).",
          obj(json!({ "tab_id": { "type": "string" }, "expression": { "type": "string" } }), &["tab_id", "expression"]),
          |a| Box::pin(async move {
              let v = browser::require()?
                  .eval(&need_str(&a, "tab_id")?, &need_str(&a, "expression")?, std::time::Duration::from_secs(30))
                  .await?;
              Ok(json!({ "value": v }))
          }), Local),
        e("gd_sniffed",
          "Media the browser tabs loaded (HLS and DASH playlists, video, audio, subtitles, documents), with the headers the page sent. Optional tab_id.",
          obj(json!({ "tab_id": { "type": "string" } }), &[]),
          |a| Box::pin(async move {
              let tab = Some(s(&a, "tab_id")).filter(|t| !t.is_empty());
              to_json(json!({ "media": sniffer::sniffed(tab.as_deref()) }))
          }), Cheap),
        e("gd_connector_spec",
          "The OmniGet connector contract: manifest fields, the ctx API available to connector.js, the tree format, and the workflow for writing one. Read it before writing a connector.",
          obj(json!({}), &[]),
          |_| Box::pin(async move { Ok(json!({ "spec": connector::SPEC })) }), Cheap),
        e("gd_connector_scaffold",
          "Create a connector skeleton (manifest.json, connector.js, GUIDE.md) in a folder. Existing files are kept.",
          obj(json!({ "dir": { "type": "string", "description": "absolute folder" }, "id": { "type": "string" }, "name": { "type": "string" }, "domains": { "type": "array", "items": { "type": "string" } }, "match": { "type": "array", "items": { "type": "string" } } }), &["dir", "id", "name", "domains", "match"]),
          |a| Box::pin(async move {
              let dir = PathBuf::from(need_str(&a, "dir")?);
              connector::scaffold(&dir, &need_str(&a, "id")?, &need_str(&a, "name")?, &list(&a, "domains"), &list(&a, "match"))?;
              Ok(json!({ "dir": dir, "files": ["manifest.json", "connector.js", "GUIDE.md"] }))
          }), Local),
        e("gd_connector_validate",
          "Check a connector folder: manifest fields, match patterns, entry file, opted-out domains, and that the script parses.",
          obj(json!({ "dir": { "type": "string" } }), &["dir"]),
          |a| Box::pin(async move {
              let dir = PathBuf::from(need_str(&a, "dir")?);
              let m = connector::load(&dir)?;
              let src = std::fs::read_to_string(dir.join(&m.entry)).map_err(|e| e.to_string())?;
              syntax_check(src).await?;
              to_json(json!({ "ok": true, "manifest": m }))
          }), Cheap),
        e("gd_connector_run",
          "Dry run: resolve a page URL with a connector (the one matching the URL, or the folder in dir) and return the tree it produces, the connector's logs and counts. Downloads nothing.",
          obj(json!({ "url": { "type": "string" }, "dir": { "type": "string" }, "tab_id": { "type": "string" }, "options": { "type": "object", "description": "overrides for the connector's options" }, "item_key": { "type": "string", "description": "resolve one lazy item of the tree instead (resolveItem)" } }), &["url"]),
          |a| Box::pin(async move {
              let dir = Some(s(&a, "dir")).filter(|d| !d.is_empty()).map(PathBuf::from);
              let tab = Some(s(&a, "tab_id")).filter(|t| !t.is_empty());
              let key = s(&a, "item_key");
              if !key.is_empty() {
                  let id = match &dir { Some(d) => connector::load(d)?.id, None => connector::find_for(&need_str(&a, "url")?).ok_or("no connector matches this URL")?.manifest.id };
                  let d = dir.as_ref().map(|d| d.display().to_string());
                  return to_json(connector::resolve_item(&id, d.as_deref(), &need_str(&a, "url")?, &key).await?);
              }
              to_json(connector::resolve(&need_str(&a, "url")?, dir, tab, a.get("options").cloned()).await?)
          }), Network),
        e("gd_connector_install_dev",
          "Link a local connector folder in developer mode, so the Global Downloader uses it for matching URLs. Edits apply on the next run.",
          obj(json!({ "dir": { "type": "string" } }), &["dir"]),
          |a| Box::pin(async move {
              let m = connector::install_dev(&PathBuf::from(need_str(&a, "dir")?))?;
              to_json(json!({ "installed": m }))
          }), Local),
        e("gd_connector_options",
          "Read or change a connector's options (declared in its manifest, passed to the script as ctx.options). With values, saves them.",
          obj(json!({ "id": { "type": "string" }, "values": { "type": "object" } }), &["id"]),
          |a| Box::pin(async move {
              let id = need_str(&a, "id")?;
              match a.get("values") {
                  Some(v) if v.is_object() => connector::set_options(&id, v),
                  _ => connector::get_options(&id),
              }
          }), Cheap),
        e("gd_connectors",
          "Installed connectors (id, name, domains, match patterns, folder, developer mode).",
          obj(json!({}), &[]),
          |_| Box::pin(async move { to_json(json!({ "connectors": connector::list() })) }), Cheap),
        e("gd_market",
          "The connector marketplace: every connector in the signed catalog with its state (available, installed, update). With url, tells which installed or catalog connector covers that page. refresh=true fetches the catalog again.",
          obj(json!({ "url": { "type": "string" }, "refresh": { "type": "boolean" } }), &[]),
          |a| Box::pin(async move {
              let url = s(&a, "url");
              if !url.is_empty() {
                  return to_json(json!(market::lookup(&url).await));
              }
              to_json(json!(market::view(a.get("refresh").and_then(Value::as_bool).unwrap_or(false)).await))
          }), Network),
        e("gd_market_install",
          "Install a connector from the marketplace by id, or update it when it is already installed. The package is checked against the signed catalog before it is unpacked.",
          obj(json!({ "id": { "type": "string" } }), &["id"]),
          |a| Box::pin(async move {
              let m = market::install_from_catalog(&need_str(&a, "id")?).await?;
              to_json(json!({ "installed": m }))
          }), Network),
        e("gd_download",
          "Resolve a URL (connector or sniffer) and download the whole tree, or the node ids in select, into out_dir. DRM items are skipped. Returns the job.",
          obj(json!({ "url": { "type": "string" }, "out_dir": { "type": "string" }, "select": { "type": "array", "items": { "type": "string" } }, "max_height": { "type": "integer" } }), &["url"]),
          |a| Box::pin(async move {
              let r = connector::resolve(&need_str(&a, "url")?, None, None, None).await?;
              let out = Some(s(&a, "out_dir")).filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(default_out_dir);
              let sel = Some(list(&a, "select")).filter(|v| !v.is_empty());
              to_json(jobs::start(r.tree, sel, out, num(&a, "max_height").map(|h| h as u32))?)
          }), Network),
        e("gd_jobs",
          "Global Downloader jobs with per-item status (done, skipped, drm, handed_off, failed) and bytes.",
          obj(json!({}), &[]),
          |_| Box::pin(async move { to_json(json!({ "jobs": jobs::list() })) }), Cheap),
    ]
}

fn tab_arg(a: &Value) -> String {
    Some(s(a, "tab_id"))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| s(a, "tabId"))
}

fn with_file(mut out: Value, full: String) -> Result<Value, String> {
    let path = std::env::temp_dir().join(format!(
        "omniget-gd-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S%3f")
    ));
    std::fs::write(&path, full).map_err(|e| e.to_string())?;
    out["path"] = json!(path);
    Ok(out)
}

async fn call_tool(name: &str, args: Value) -> Result<Value, String> {
    let entry = entries()
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| format!("unknown tool {name}"))?;
    match entry.call {
        ToolImpl::Core(f) => f(args).await,
        ToolImpl::Host => Err(format!("{name} cannot run in a batch")),
    }
}

const MAX_STEPS: usize = 30;
const STEP_RESULT_CAP_CHARS: usize = 2000;

async fn act(a: Value) -> Result<Value, String> {
    let b = browser::require()?;
    let batch_tab = page::pick_tab(&b, Some(tab_arg(&a)))?;
    let steps = a["steps"]
        .as_array()
        .filter(|s| !s.is_empty())
        .ok_or("steps is required: a non-empty array of {op, ...}")?;
    if steps.len() > MAX_STEPS {
        return Err(format!("at most {MAX_STEPS} steps per call"));
    }
    let stop_on_error = a["stopOnError"].as_bool().unwrap_or(true);
    let mut rows = Vec::new();
    let mut stopped = None;
    let mut needs_human = None;
    let mut last = None;
    let mut last_tab = batch_tab.clone();
    for step in steps {
        let op = s(step, "op");
        let Some(tool) = harness::tool_for_op(&op) else {
            rows.push(json!({ "op": op, "ok": false, "outcome": "no_effect", "error": "op must be one of click, type, press, scroll, navigate, wait, eval" }));
            if stop_on_error {
                stopped = Some("error");
                break;
            }
            continue;
        };
        let tab = Some(tab_arg(step))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| batch_tab.clone());
        let mut args = step.clone();
        args["tab_id"] = json!(tab);
        let before = page::probe(&b, &tab).await;
        let result = call_tool(tool, args).await;
        let mut after = page::probe(&b, &tab).await;
        let idle = match (&before, &after) {
            (Some(x), Some(y)) => harness::changes(x, y).is_empty(),
            _ => false,
        };
        if result.is_ok() && op != "wait" && idle {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            after = page::probe(&b, &tab).await;
        }
        let outcome = harness::derive_outcome(&op, before.as_ref(), after.as_ref(), &result);
        let changed = match (&before, &after) {
            (Some(x), Some(y)) => harness::changes(x, y),
            _ => Vec::new(),
        };
        let ok = result.is_ok();
        let mut row = json!({ "op": op, "ok": ok, "outcome": outcome, "changed": changed });
        match result {
            Ok(v) => {
                let (out, full) = harness::cap_result(v, STEP_RESULT_CAP_CHARS);
                row["result"] = match full {
                    Some(full) => with_file(out, full)?,
                    None => out,
                };
            }
            Err(e) => row["error"] = json!(e),
        }
        rows.push(row);
        let was = before
            .as_ref()
            .and_then(|p| harness::detect_blocker(p, false));
        let now = after.as_ref().and_then(|y| {
            let moved = before.as_ref().is_some_and(|x| {
                harness::redirected(&op, &browser::address_to_url(&s(step, "url")), x, y)
            });
            harness::detect_blocker(y, moved)
        });
        last = after;
        last_tab = tab;
        if let Some(n) = now {
            if was.as_ref().map(|w| &w["kind"]) != Some(&n["kind"]) {
                needs_human = Some(n);
                stopped = Some("needs_human");
                break;
            }
        }
        if !ok && stop_on_error {
            stopped = Some("error");
            break;
        }
    }
    let t = b.tab(&last_tab).unwrap_or_default();
    let (url, title) = match &last {
        Some(p) => (p.url.clone(), p.title.clone()),
        None => (t.url, t.title),
    };
    let needs_human = needs_human.or_else(|| {
        last.as_ref()
            .and_then(|p| harness::detect_blocker(p, false))
    });
    let mut out = json!({ "tab_id": last_tab, "steps": rows, "completed": rows.len(), "total": steps.len(), "url": url, "title": title });
    if let Some(reason) = stopped {
        out["stopped"] = json!(reason);
    }
    if let Some(n) = needs_human {
        out["needs_human"] = n;
    }
    Ok(out)
}

pub fn default_out_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("OmniGet")
}

pub fn tree_for_sniffed(id: u64) -> Result<Node, String> {
    let m = sniffer::sniffed_by_id(id).ok_or("unknown media id")?;
    let format = match m.kind.as_str() {
        "hls" => "hls",
        "dash" => "dash",
        _ => "direct",
    };
    let title = super::connector::media_title(&m.url, &m.page_title);
    let mut tree = Node {
        title: "Sniffed".into(),
        kind: "folder".into(),
        children: vec![Node {
            title,
            kind: match m.kind.as_str() {
                "hls" | "dash" => "video".into(),
                "document" => "file".into(),
                k => k.into(),
            },
            source: Some(Source {
                format: format.into(),
                url: Some(m.url),
                headers: m.headers,
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    tree.assign_ids("0");
    Ok(tree)
}

async fn syntax_check(src: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        use rquickjs::{CatchResultExt, Context, Runtime};
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        let src = src
            .replace("export async function", "async function")
            .replace("export function", "function");
        ctx.with(|ctx| {
            let f: rquickjs::Function = ctx
                .eval(format!(
                    "(function(){{ return function(){{ {src}\n; return typeof resolve; }}; }})()"
                ))
                .catch(&ctx)
                .map_err(|e| format!("syntax: {e}"))?;
            let kind: String = f
                .call(())
                .catch(&ctx)
                .map_err(|e| format!("top level: {e}"))?;
            if kind != "function" {
                return Err("the script defines no resolve(url, ctx)".to_string());
            }
            Ok(())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod smoke_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PAGE: &str = r#"<!doctype html><title>Shop</title><input id="q" aria-label="Query"><button id="go" onclick="var d=document.createElement('p');d.textContent='hello '+document.getElementById('q').value;document.body.appendChild(d)">Go</button><div id="dead">dead</div>"#;
    const CHALLENGE: &str = "<!doctype html><title>Just a moment...</title><p>wait</p>";

    async fn serve() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let body = if String::from_utf8_lossy(&buf[..n]).starts_with("GET /challenge") {
                    CHALLENGE
                } else {
                    PAGE
                };
                let _ = sock
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            }
        });
        port
    }

    #[tokio::test]
    #[ignore = "launches a headless Chromium; run by hand: cargo test -p omniget-core global_dl::tools::smoke -- --ignored"]
    async fn attach_act_snapshot_and_cdp_against_a_real_browser() {
        let (_, exe) = browser::installed_browsers()
            .into_iter()
            .next()
            .expect("a Chromium browser is installed");
        let profile = std::env::temp_dir().join(format!("omniget-gd-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&profile).unwrap();
        let mut chrome = tokio::process::Command::new(&exe)
            .arg("--headless=new")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--remote-debugging-port=0")
            .arg("--no-first-run")
            .arg("about:blank")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut debug_port = None;
        for _ in 0..100 {
            if let Ok(t) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
                debug_port = t.lines().next().and_then(|l| l.trim().parse::<u16>().ok());
                if debug_port.is_some() {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let debug_port = debug_port.expect("the browser opened its debugging port");
        let site = serve().await;

        let refused = call_tool("gd_browser_attach", json!({ "port": 1 }))
            .await
            .unwrap_err();
        let refused: Value = serde_json::from_str(&refused).unwrap();
        assert_eq!(refused["code"], "CHROME_NOT_DEBUGGABLE");

        let attached = call_tool("gd_browser_attach", json!({ "port": debug_port }))
            .await
            .unwrap();
        assert_eq!(attached["attached"], true);
        assert_eq!(browser::status()["attached"], true);

        let r = call_tool(
            "gd_act",
            json!({ "steps": [
                { "op": "navigate", "url": format!("http://127.0.0.1:{site}/") },
                { "op": "type", "selector": "#q", "text": "abc" },
                { "op": "click", "selector": "#go" },
                { "op": "wait", "text": "hello abc", "timeout_ms": 3000 },
                { "op": "click", "selector": "#dead" },
                { "op": "click", "selector": "#dead" },
                { "op": "eval", "expression": "document.querySelectorAll('p').length" },
                { "op": "click", "selector": "#missing" },
                { "op": "press", "key": "Enter" }
            ] }),
        )
        .await
        .unwrap();
        let steps = r["steps"].as_array().unwrap();
        assert_eq!(r["total"], 9);
        assert_eq!(r["completed"], 8);
        assert_eq!(r["stopped"], "error");
        assert_eq!(r["title"], "Shop");
        for step in &steps[..4] {
            assert_eq!(step["outcome"], "confirmed");
        }
        assert_eq!(steps[5]["outcome"], "no_effect");
        assert_eq!(steps[6]["result"]["value"], 1);
        assert_eq!(steps[7]["ok"], false);
        assert_eq!(steps[7]["outcome"], "no_effect");
        assert!(r.get("needs_human").is_none());

        let buttons = call_tool("gd_snapshot", json!({ "role": "button" }))
            .await
            .unwrap();
        assert_eq!(buttons["total"], 1);
        assert!(buttons["tree"]
            .as_str()
            .unwrap()
            .starts_with("- button \"Go\""));
        let small = call_tool("gd_snapshot", json!({ "maxTokens": 6 }))
            .await
            .unwrap();
        assert_eq!(small["truncated"], true);
        let scoped = call_tool("gd_snapshot", json!({ "scope": "#dead" }))
            .await
            .unwrap();
        assert_eq!(scoped["nodes"], 1);
        call_tool("gd_snapshot", json!({})).await.unwrap();
        call_tool("gd_click", json!({ "selector": "#go" }))
            .await
            .unwrap();
        let diff = call_tool("gd_snapshot", json!({ "diff": true }))
            .await
            .unwrap();
        assert_eq!(diff["diff"]["added"].as_array().unwrap().len(), 2);
        assert!(diff.get("tree").is_none());

        let two = call_tool(
            "gd_cdp",
            json!({ "method": "Runtime.evaluate", "params": { "expression": "1+1" } }),
        )
        .await
        .unwrap();
        assert_eq!(two["result"]["result"]["value"], 2);
        let version = call_tool("gd_cdp", json!({ "method": "Browser.getVersion" }))
            .await
            .unwrap();
        assert!(version["result"]["product"].is_string());
        let big = call_tool(
            "gd_cdp",
            json!({ "method": "Runtime.evaluate", "params": { "expression": "'x'.repeat(20000)" } }),
        )
        .await
        .unwrap();
        assert_eq!(big["truncated"], true);
        let path = big["path"].as_str().unwrap();
        assert!(std::fs::metadata(path).unwrap().len() > 20000);
        let _ = std::fs::remove_file(path);
        let denied = call_tool("gd_cdp", json!({ "method": "Browser.close" }))
            .await
            .unwrap_err();
        assert!(denied.contains("CDP_METHOD_DENIED"));

        let blocked = call_tool(
            "gd_act",
            json!({ "steps": [
                { "op": "navigate", "url": format!("http://127.0.0.1:{site}/challenge") },
                { "op": "click", "selector": "text=wait" }
            ] }),
        )
        .await
        .unwrap();
        assert_eq!(blocked["completed"], 1);
        assert_eq!(blocked["stopped"], "needs_human");
        assert_eq!(blocked["needs_human"]["kind"], "bot_challenge");

        let detached = call_tool("gd_browser_attach", json!({ "detach": true }))
            .await
            .unwrap();
        assert_eq!(detached["detached"], true);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            chrome.try_wait().unwrap().is_none(),
            "detach must not close the browser"
        );
        assert!(browser::current().is_none());

        let _ = chrome.kill().await;
        let _ = std::fs::remove_dir_all(&profile);
    }
}
