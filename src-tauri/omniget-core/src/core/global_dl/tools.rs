use std::path::PathBuf;

use serde_json::{json, Value};

use super::{browser, connector, jobs, page, profiles, sniffer, Node, Source};
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
              if a["window"].as_bool() == Some(true) && browser::mode() != "window" {
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
          "Read the page in a browser tab (default: the tab shown in OmniGet): an indented role/name tree whose refs (e12) work in gd_click and gd_type, plus the visible text, URL and title.",
          obj(json!({ "tab_id": { "type": "string" }, "max_nodes": { "type": "integer", "maximum": 10000 }, "max_chars": { "type": "integer", "maximum": 200000 } }), &[]),
          |a| Box::pin(async move {
              let b = browser::require()?;
              let tab = page::pick_tab(&b, Some(s(&a, "tab_id")))?;
              let mut v = page::snapshot(&b, &tab, num(&a, "max_nodes"), num(&a, "max_chars")).await?;
              v["tab_id"] = json!(tab);
              Ok(v)
          }), Cheap),
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
