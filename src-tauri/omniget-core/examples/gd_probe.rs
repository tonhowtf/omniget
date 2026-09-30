//! Drives the Global Downloader core from the shell, for live tests.
//!
//! OMNIGET_DATA_DIR=/tmp/gd cargo run -p omniget-core --example gd_probe -- open https://www.udemy.com
//!   tabs | sniffed | resolve <url> [connector dir] | download <url> <out dir> [max height]

use omniget_core::core::global_dl::{browser, connector, jobs};

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("tabs");
    match cmd {
        "open" => {
            let b = browser::start(args.get(1).cloned(), true).await?;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            println!("{}", serde_json::to_string_pretty(&b.tabs()).unwrap());
        }
        "tabs" => {
            let b = browser::start(None, false).await?;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            println!("{}", serde_json::to_string_pretty(&b.tabs()).unwrap());
        }
        "cookies" => {
            let b = browser::start(None, false).await?;
            let host = args.get(1).cloned().unwrap_or_default();
            for c in b.cookies_for(&host).await? {
                println!(
                    "{} ({} bytes) domain={}",
                    c["name"],
                    c["value"].as_str().unwrap_or("").len(),
                    c["domain"]
                );
            }
        }
        "tool" => {
            // Any gd_* tool, as an agent calls it: tool <name> [json args]
            let name = args.get(1).ok_or("name")?;
            let a: serde_json::Value = args
                .get(2)
                .map(|j| serde_json::from_str(j).map_err(|e| e.to_string()))
                .transpose()?
                .unwrap_or_else(|| serde_json::json!({}));
            if name != "gd_browser_open"
                && name != "gd_browser_sources"
                && name != "gd_browser_import"
            {
                let _ = browser::start(None, false).await?;
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            }
            let e = omniget_core::core::global_dl::tools::entries()
                .into_iter()
                .find(|e| e.name == name.as_str())
                .ok_or("unknown tool")?;
            let omniget_core::core::llm::tool_table::ToolImpl::Core(f) = e.call else {
                return Err("host tool".into());
            };
            let v = f(a).await?;
            let text = serde_json::to_string_pretty(&v).unwrap();
            println!("{}", text.chars().take(6000).collect::<String>());
        }
        "view" => {
            // Frames and input through the embedded view: view <url>
            let b = browser::start(None, false).await?;
            let mut rx = omniget_core::core::global_dl::events().subscribe();
            let tab = b
                .open_tab(
                    args.get(1)
                        .map(String::as_str)
                        .unwrap_or("https://duckduckgo.com/"),
                )
                .await?;
            omniget_core::core::global_dl::page::wait_loaded(
                &b,
                &tab,
                std::time::Duration::from_secs(15),
            )
            .await;
            let dpr: f64 = std::env::var("GD_DPR")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2.0);
            b.view(&tab, 1000, 700, dpr).await?;
            let counter = tokio::spawn(async move {
                let (mut n, mut bytes, mut size) = (0, 0, (0, 0));
                let end = std::time::Instant::now() + std::time::Duration::from_secs(6);
                while std::time::Instant::now() < end {
                    if let Ok(Ok(ev)) =
                        tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
                    {
                        if ev["kind"] == "frame" {
                            n += 1;
                            bytes += ev["data"].as_str().map(|d| d.len()).unwrap_or(0);
                            size = (
                                ev["width"].as_u64().unwrap_or(0),
                                ev["height"].as_u64().unwrap_or(0),
                            );
                        }
                    }
                }
                (n, bytes, size)
            });
            // Person-like input while frames stream: type in the search box, scroll.
            omniget_core::core::global_dl::page::click_at(&b, &tab, 500.0, 330.0, 1).await?;
            for c in "omniget".chars() {
                b.press(&tab, &c.to_string(), 0).await?;
            }
            for _ in 0..6 {
                b.mouse(&tab, serde_json::json!({ "type": "mouseWheel", "x": 500, "y": 400, "deltaX": 0, "deltaY": 200 })).await?;
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
            let (n, bytes, size) = counter.await.map_err(|e| e.to_string())?;
            let typed = b
                .eval(
                    &tab,
                    "document.activeElement && document.activeElement.value",
                    std::time::Duration::from_secs(5),
                )
                .await?;
            println!("frames in 6s: {n}, avg frame {} KB, frame size {:?}, typed into focused field: {typed}", bytes / n.max(1) / 1024, size);
            b.unview().await;
            b.close_tab(&tab).await?;
        }
        "options" => {
            let id = args.get(1).ok_or("id")?;
            let v = match args.get(2) {
                Some(j) => connector::set_options(
                    id,
                    &serde_json::from_str(j).map_err(|e| e.to_string())?,
                )?,
                None => connector::get_options(id)?,
            };
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        }
        "install" => {
            let m = connector::install_dev(std::path::Path::new(args.get(1).ok_or("dir")?))?;
            println!("installed {} {}", m.id, m.version);
        }
        "eval" => {
            let b = browser::start(None, false).await?;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let tab = b.tabs().last().cloned().ok_or("no tab")?;
            let v = b
                .eval(
                    &tab.id,
                    args.get(1).ok_or("expr")?,
                    std::time::Duration::from_secs(30),
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        }
        "watch" => {
            // Network log of every tab for N seconds (status >= 400 and XHR/Fetch/Document).
            browser::start(None, false).await?;
            let secs: u64 = args.get(1).and_then(|x| x.parse().ok()).unwrap_or(60);
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            let types = [
                "XHR".to_string(),
                "Fetch".to_string(),
                "Document".to_string(),
            ];
            for e in omniget_core::core::global_dl::sniffer::log(None, None, &types, 400) {
                println!(
                    "{} {} {} {}",
                    e.status,
                    e.method,
                    e.resource_type,
                    e.url.chars().take(160).collect::<String>()
                );
            }
        }
        "sniffed" => {
            browser::start(None, false).await?;
            println!("watching for 20 s…");
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            let s = omniget_core::core::global_dl::sniffer::sniffed(None);
            println!("{}", serde_json::to_string_pretty(&s).unwrap());
        }
        "resolve" => {
            browser::start(None, false).await?;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let url = args.get(1).ok_or("url")?;
            let dir = args.get(2).map(std::path::PathBuf::from);
            let r = connector::resolve(url, dir, None, None).await?;
            for l in &r.logs {
                println!("log: {l}");
            }
            println!(
                "connector={:?} items={} drm={} ms={}",
                r.connector, r.items, r.drm, r.ms
            );
            let out = std::env::temp_dir().join("gd_tree.json");
            std::fs::write(&out, serde_json::to_string_pretty(&r.tree).unwrap()).unwrap();
            println!("tree: {}", out.display());
            print_tree(&r.tree, 0, &mut 0);
        }
        "download" => {
            browser::start(None, false).await?;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let url = args.get(1).ok_or("url")?;
            let out = args.get(2).ok_or("out dir")?;
            let h = args.get(3).and_then(|x| x.parse().ok());
            let only: Option<Vec<String>> = std::env::var("GD_ONLY")
                .ok()
                .map(|v| v.split(',').map(String::from).collect());
            let mut r = connector::resolve(url, None, None, None).await?;
            // GD_STALE=1: give every lazy item a dead link, to test the
            // expired-link path (the job must ask resolveItem again).
            if std::env::var("GD_STALE").is_ok() {
                fn stale(n: &mut omniget_core::core::global_dl::Node) {
                    for c in &mut n.children {
                        stale(c);
                    }
                    if n.key.is_some() {
                        if let Some(s) = n.source.as_mut() {
                            s.format = "direct".into();
                            s.url = Some(
                                "https://www.udemy.com/assets/0/files/expired.vtt?token=old".into(),
                            );
                        }
                    }
                }
                stale(&mut r.tree);
            }
            let job = jobs::start(r.tree, only, out.into(), h)?;
            println!("job {} with {} items → {}", job.id, job.total, job.out_dir);
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let j = jobs::list().into_iter().find(|x| x.id == job.id).unwrap();
                println!(
                    "{} done={} skipped={} drm={} failed={} bytes={}",
                    j.status, j.done, j.skipped, j.drm, j.failed, j.bytes
                );
                if j.status != "running" {
                    for it in j.items.iter().filter(|i| i.status == "failed") {
                        println!("FAILED {}: {:?}", it.title, it.error);
                    }
                    break;
                }
            }
        }
        _ => return Err(format!("unknown command {cmd}")),
    }
    Ok(())
}

fn print_tree(n: &omniget_core::core::global_dl::Node, depth: usize, shown: &mut usize) {
    for c in &n.children {
        *shown += 1;
        if *shown > 60 {
            return;
        }
        let src = c
            .source
            .as_ref()
            .map(|s| format!("[{}] {}", s.format, s.filename.clone().unwrap_or_default()))
            .unwrap_or_default();
        println!(
            "{}{} {} {}{}",
            "  ".repeat(depth),
            c.id,
            c.title,
            src,
            if c.drm { " DRM" } else { "" }
        );
        print_tree(c, depth + 1, shown);
    }
}
