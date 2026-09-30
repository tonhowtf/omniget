//! Bringing a person's own browser into the Global Downloader: their logins,
//! saved passwords, extensions and history.
//!
//! - Chromium family (Chrome, Edge, Brave, Chromium, Vivaldi, Opera, Arc):
//!   the chosen profile is copied into OmniGet's browser profile and that
//!   same browser runs it. Cookies and passwords stay readable because the
//!   browser decrypts them with its own key (macOS keychain item, Windows
//!   `Local State` + DPAPI, Linux keyring), which a copy keeps using.
//! - Firefox: its cookies are read from `cookies.sqlite` (not encrypted) and
//!   set into the running browser.
//!
//! The source browser's files are only read, never changed.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use super::{browser, data_dir, emit};

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    /// Folder name inside the browser's data dir (`Default`, `Profile 1`) or
    /// the Firefox profile path.
    pub dir: String,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    /// chromium | firefox
    pub family: String,
    pub exe: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub profiles: Vec<Profile>,
    /// A running browser locks its files on Windows; on macOS and Linux a
    /// copy works while it runs.
    pub running: bool,
}

struct Known {
    id: &'static str,
    name: &'static str,
    mac_data: &'static str,
    mac_app: &'static str,
    win_data: &'static str,
    win_exe: &'static str,
    linux_data: &'static str,
    linux_bin: &'static str,
}

const CHROMIUMS: &[Known] = &[
    Known {
        id: "chrome",
        name: "Google Chrome",
        mac_data: "Google/Chrome",
        mac_app: "Google Chrome.app/Contents/MacOS/Google Chrome",
        win_data: "Google\\Chrome\\User Data",
        win_exe: "Google\\Chrome\\Application\\chrome.exe",
        linux_data: "google-chrome",
        linux_bin: "google-chrome",
    },
    Known {
        id: "edge",
        name: "Microsoft Edge",
        mac_data: "Microsoft Edge",
        mac_app: "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        win_data: "Microsoft\\Edge\\User Data",
        win_exe: "Microsoft\\Edge\\Application\\msedge.exe",
        linux_data: "microsoft-edge",
        linux_bin: "microsoft-edge",
    },
    Known {
        id: "brave",
        name: "Brave",
        mac_data: "BraveSoftware/Brave-Browser",
        mac_app: "Brave Browser.app/Contents/MacOS/Brave Browser",
        win_data: "BraveSoftware\\Brave-Browser\\User Data",
        win_exe: "BraveSoftware\\Brave-Browser\\Application\\brave.exe",
        linux_data: "BraveSoftware/Brave-Browser",
        linux_bin: "brave-browser",
    },
    Known {
        id: "chromium",
        name: "Chromium",
        mac_data: "Chromium",
        mac_app: "Chromium.app/Contents/MacOS/Chromium",
        win_data: "Chromium\\User Data",
        win_exe: "Chromium\\Application\\chrome.exe",
        linux_data: "chromium",
        linux_bin: "chromium",
    },
    Known {
        id: "vivaldi",
        name: "Vivaldi",
        mac_data: "Vivaldi",
        mac_app: "Vivaldi.app/Contents/MacOS/Vivaldi",
        win_data: "Vivaldi\\User Data",
        win_exe: "Vivaldi\\Application\\vivaldi.exe",
        linux_data: "vivaldi",
        linux_bin: "vivaldi",
    },
    Known {
        id: "opera",
        name: "Opera",
        mac_data: "com.operasoftware.Opera",
        mac_app: "Opera.app/Contents/MacOS/Opera",
        win_data: "Opera Software\\Opera Stable",
        win_exe: "Programs\\Opera\\opera.exe",
        linux_data: "opera",
        linux_bin: "opera",
    },
    Known {
        id: "arc",
        name: "Arc",
        mac_data: "Arc/User Data",
        mac_app: "Arc.app/Contents/MacOS/Arc",
        win_data: "",
        win_exe: "",
        linux_data: "",
        linux_bin: "",
    },
];

fn find_exe(k: &Known) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = dirs::home_dir().unwrap_or_default();
        [PathBuf::from("/Applications"), home.join("Applications")]
            .into_iter()
            .map(|b| b.join(k.mac_app))
            .find(|p| p.is_file())
    } else if cfg!(windows) {
        if k.win_exe.is_empty() {
            return None;
        }
        ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .map(|b| PathBuf::from(b).join(k.win_exe))
            .find(|p| p.is_file())
    } else {
        if k.linux_bin.is_empty() {
            return None;
        }
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|d| d.join(k.linux_bin))
            .find(|p| p.is_file())
    }
}

fn data_of(k: &Known) -> Option<PathBuf> {
    let rel = if cfg!(target_os = "macos") {
        k.mac_data
    } else if cfg!(windows) {
        k.win_data
    } else {
        k.linux_data
    };
    if rel.is_empty() {
        return None;
    }
    let base = if cfg!(windows) {
        // Opera keeps its profile in Roaming; the rest in Local.
        if k.id == "opera" {
            dirs::data_dir()?
        } else {
            dirs::data_local_dir()?
        }
    } else if cfg!(target_os = "macos") {
        dirs::data_dir()?
    } else {
        dirs::config_dir()?
    };
    Some(base.join(rel)).filter(|p| p.is_dir())
}

fn chromium_profiles(data: &Path, id: &str) -> Vec<Profile> {
    let state: Value = std::fs::read_to_string(data.join("Local State"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut out: Vec<Profile> = state["profile"]["info_cache"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(dir, _)| data.join(dir).is_dir())
                .map(|(dir, v)| Profile {
                    dir: dir.clone(),
                    name: v["name"].as_str().unwrap_or(dir).to_string(),
                    email: v["user_name"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(String::from),
                })
                .collect()
        })
        .unwrap_or_default();
    // Opera keeps a single profile in the data dir itself.
    if out.is_empty()
        && id == "opera"
        && (data.join("Cookies").exists() || data.join("Network").is_dir())
    {
        out.push(Profile {
            dir: ".".into(),
            name: "Default".into(),
            email: None,
        });
    }
    if out.is_empty() && data.join("Default").is_dir() {
        out.push(Profile {
            dir: "Default".into(),
            name: "Default".into(),
            email: None,
        });
    }
    out.sort_by(|a, b| {
        (a.dir != "Default")
            .cmp(&(b.dir != "Default"))
            .then(a.dir.cmp(&b.dir))
    });
    out
}

fn firefox_root() -> Option<PathBuf> {
    let p = if cfg!(target_os = "macos") {
        dirs::data_dir()?.join("Firefox")
    } else if cfg!(windows) {
        dirs::data_dir()?.join("Mozilla").join("Firefox")
    } else {
        dirs::home_dir()?.join(".mozilla").join("firefox")
    };
    Some(p).filter(|p| p.is_dir())
}

fn firefox_profiles(root: &Path) -> Vec<Profile> {
    let ini = std::fs::read_to_string(root.join("profiles.ini")).unwrap_or_default();
    let mut out = Vec::new();
    let (mut name, mut path, mut rel) = (String::new(), String::new(), true);
    let flush = |name: &mut String, path: &mut String, rel: bool, out: &mut Vec<Profile>| {
        if !path.is_empty() {
            let full = if rel {
                root.join(&*path)
            } else {
                PathBuf::from(&*path)
            };
            if full.join("cookies.sqlite").is_file() {
                out.push(Profile {
                    dir: full.display().to_string(),
                    name: if name.is_empty() {
                        path.clone()
                    } else {
                        name.clone()
                    },
                    email: None,
                });
            }
        }
        name.clear();
        path.clear();
    };
    for line in ini.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            flush(&mut name, &mut path, rel, &mut out);
            rel = true;
        } else if let Some(v) = l.strip_prefix("Name=") {
            name = v.to_string();
        } else if let Some(v) = l.strip_prefix("Path=") {
            path = v.to_string();
        } else if let Some(v) = l.strip_prefix("IsRelative=") {
            rel = v == "1";
        }
    }
    flush(&mut name, &mut path, rel, &mut out);
    out
}

fn is_running(exe: &Path) -> bool {
    let name = exe
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return false;
    }
    let out = if cfg!(windows) {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}")])
            .output()
    } else {
        std::process::Command::new("pgrep")
            .args(["-x", &name])
            .output()
    };
    out.map(|o| {
        String::from_utf8_lossy(&o.stdout).contains(&name) || (!cfg!(windows) && o.status.success())
    })
    .unwrap_or(false)
}

/// Browsers on this computer that OmniGet can take logins from.
pub fn sources() -> Vec<Source> {
    let mut out = Vec::new();
    for k in CHROMIUMS {
        let Some(data) = data_of(k) else { continue };
        let exe = find_exe(k);
        let profiles = chromium_profiles(&data, k.id);
        if profiles.is_empty() {
            continue;
        }
        out.push(Source {
            id: k.id.into(),
            name: k.name.into(),
            family: "chromium".into(),
            running: exe.as_deref().is_some_and(is_running),
            exe,
            data_dir: data,
            profiles,
        });
    }
    if let Some(root) = firefox_root() {
        let profiles = firefox_profiles(&root);
        if !profiles.is_empty() {
            out.push(Source {
                id: "firefox".into(),
                name: "Firefox".into(),
                family: "firefox".into(),
                exe: None,
                data_dir: root,
                profiles,
                running: false,
            });
        }
    }
    out
}

/// Caches and other folders a copy does not need.
const SKIP: &[&str] = &[
    "Cache",
    "Code Cache",
    "GPUCache",
    "DawnCache",
    "DawnGraphiteCache",
    "DawnWebGPUCache",
    "GrShaderCache",
    "ShaderCache",
    "GraphiteDawnCache",
    "Media Cache",
    "CacheStorage",
    "ScriptCache",
    "blob_storage",
    "optimization_guide_model_store",
    "optimization_guide_hint_cache_store",
    "Crashpad",
    "BrowserMetrics",
    "component_crx_cache",
    "Safe Browsing",
    "SingletonLock",
    "SingletonCookie",
    "SingletonSocket",
    "LOCK",
    "lockfile",
];

fn copy_tree(from: &Path, to: &Path, bytes: &mut u64) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        if SKIP.iter().any(|s| *s == n) {
            continue;
        }
        let ft = e.file_type()?;
        let dst = to.join(&name);
        if ft.is_dir() {
            copy_tree(&e.path(), &dst, bytes)?;
        } else if ft.is_file() {
            // A file locked by the running browser is skipped, not fatal.
            if let Ok(n) = std::fs::copy(e.path(), &dst) {
                *bytes += n;
            }
        }
    }
    Ok(())
}

/// Imports `profile` of `source_id` and restarts the browser with it.
pub async fn import(source_id: &str, profile: &str) -> Result<Value, String> {
    let src = sources()
        .into_iter()
        .find(|s| s.id == source_id)
        .ok_or_else(|| format!("{source_id} was not found on this computer"))?;
    let prof = src
        .profiles
        .iter()
        .find(|p| p.dir == profile || p.name == profile)
        .cloned()
        .or_else(|| src.profiles.first().cloned())
        .ok_or("that browser has no profile")?;
    emit(json!({ "kind": "import", "state": "started", "from": src.name, "profile": prof.name }));
    let r = if src.family == "firefox" {
        import_firefox(&src, &prof).await
    } else {
        import_chromium(&src, &prof).await
    };
    match &r {
        Ok(v) => emit(json!({ "kind": "import", "state": "done", "result": v })),
        Err(e) => emit(json!({ "kind": "import", "state": "failed", "error": e })),
    }
    r
}

async fn import_chromium(src: &Source, prof: &Profile) -> Result<Value, String> {
    let exe = src
        .exe
        .clone()
        .ok_or_else(|| format!("{} is not installed, only its data is", src.name))?;
    if cfg!(windows) && src.running {
        return Err(format!(
            "close {} first: Windows locks its files while it runs",
            src.name
        ));
    }
    let was_open = browser::current().is_some();
    browser::stop().await?;
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let target = data_dir().join("browser-profile");
    let from = if prof.dir == "." {
        src.data_dir.clone()
    } else {
        src.data_dir.join(&prof.dir)
    };
    let started = std::time::Instant::now();
    let (src_dir, dst_root) = (from.clone(), target.clone());
    let data = src.data_dir.clone();
    let bytes = tokio::task::spawn_blocking(move || -> Result<u64, String> {
        // Keep what OmniGet itself wrote (runtime file), replace the rest.
        let _ = std::fs::remove_dir_all(&dst_root);
        std::fs::create_dir_all(&dst_root).map_err(|e| e.to_string())?;
        let mut bytes = 0u64;
        copy_tree(&src_dir, &dst_root.join("Default"), &mut bytes).map_err(|e| e.to_string())?;
        // Local State holds the cookie key on Windows and Linux.
        if let Ok(t) = std::fs::read_to_string(data.join("Local State")) {
            let mut st: Value = serde_json::from_str(&t).unwrap_or_default();
            if let Some(cache) = st["profile"]["info_cache"].as_object().cloned() {
                let mine = cache.values().next().cloned().unwrap_or_default();
                st["profile"]["info_cache"] = json!({ "Default": mine });
            }
            st["profile"]["last_used"] = json!("Default");
            st["profile"]["last_active_profiles"] = json!(["Default"]);
            let _ = std::fs::write(dst_root.join("Local State"), st.to_string());
        }
        // The copy must not reopen the source's windows.
        let prefs = dst_root.join("Default").join("Preferences");
        if let Ok(t) = std::fs::read_to_string(&prefs) {
            if let Ok(mut p) = serde_json::from_str::<Value>(&t) {
                p["session"]["restore_on_startup"] = json!(5);
                p["profile"]["exit_type"] = json!("Normal");
                let _ = std::fs::write(&prefs, p.to_string());
            }
        }
        for f in [
            "Current Session",
            "Current Tabs",
            "Last Session",
            "Last Tabs",
        ] {
            let _ = std::fs::remove_file(dst_root.join("Default").join(f));
        }
        let _ = std::fs::remove_dir_all(dst_root.join("Default").join("Sessions"));
        Ok(bytes)
    })
    .await
    .map_err(|e| e.to_string())??;

    browser::set_browser_path(Some(exe.display().to_string()))?;
    browser::set_config(
        "imported",
        json!({ "from": src.name, "id": src.id, "profile": prof.name, "at": chrono::Utc::now().to_rfc3339() }),
    )?;
    let b = browser::start(None, true).await?;
    let cookies = b
        .cdp
        .call("Storage.getCookies", json!({}), None)
        .await
        .ok()
        .and_then(|r| r["cookies"].as_array().map(|a| a.len()))
        .unwrap_or(0);
    let _ = was_open;
    Ok(json!({
        "from": src.name,
        "profile": prof.name,
        "copied_mb": bytes / (1024 * 1024),
        "seconds": started.elapsed().as_secs(),
        "cookies": cookies,
        "browser": exe,
    }))
}

async fn import_firefox(src: &Source, prof: &Profile) -> Result<Value, String> {
    let db = PathBuf::from(&prof.dir).join("cookies.sqlite");
    let tmp = std::env::temp_dir().join(format!("omniget-ff-{}.sqlite", std::process::id()));
    std::fs::copy(&db, &tmp).map_err(|e| format!("could not read Firefox cookies: {e}"))?;
    for ext in ["-wal", "-shm"] {
        let w = PathBuf::from(format!("{}{}", db.display(), ext));
        if w.is_file() {
            let _ = std::fs::copy(&w, format!("{}{}", tmp.display(), ext));
        }
    }
    let rows = tokio::task::spawn_blocking(move || -> Result<Vec<Value>, String> {
        let c = rusqlite::Connection::open(&tmp).map_err(|e| e.to_string())?;
        let mut st = c
            .prepare("SELECT host, name, value, path, expiry, isSecure, isHttpOnly, sameSite FROM moz_cookies")
            .map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        let rows = st
            .query_map([], |r| {
                let mut expiry: i64 = r.get(4)?;
                // Newer Firefox stores milliseconds.
                if expiry > 100_000_000_000 {
                    expiry /= 1000;
                }
                let same = match r.get::<_, i64>(7)? {
                    1 => "Lax",
                    2 => "Strict",
                    _ => "None",
                };
                Ok((expiry, json!({
                    "domain": r.get::<_, String>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "value": r.get::<_, String>(2)?,
                    "path": r.get::<_, String>(3)?,
                    "expires": expiry as f64,
                    "secure": r.get::<_, i64>(5)? == 1,
                    "httpOnly": r.get::<_, i64>(6)? == 1,
                    "sameSite": same,
                })))
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter(|(exp, c)| *exp > now && !c["domain"].as_str().unwrap_or("").is_empty())
            .map(|(_, mut c)| {
                // SameSite=None needs Secure in Chromium.
                if c["sameSite"] == "None" && c["secure"] == false {
                    c["sameSite"] = json!("Lax");
                }
                c
            })
            .collect::<Vec<_>>();
        drop(st);
        drop(c);
        let _ = std::fs::remove_file(&tmp);
        Ok(rows)
    })
    .await
    .map_err(|e| e.to_string())??;

    let b = browser::start(None, true).await?;
    let mut set = 0usize;
    for chunk in rows.chunks(200) {
        if b.cdp
            .call("Storage.setCookies", json!({ "cookies": chunk }), None)
            .await
            .is_ok()
        {
            set += chunk.len();
        } else {
            for c in chunk {
                if b.cdp
                    .call("Storage.setCookies", json!({ "cookies": [c] }), None)
                    .await
                    .is_ok()
                {
                    set += 1;
                }
            }
        }
    }
    browser::set_config(
        "imported",
        json!({ "from": src.name, "id": src.id, "profile": prof.name, "at": chrono::Utc::now().to_rfc3339() }),
    )?;
    Ok(json!({ "from": src.name, "profile": prof.name, "cookies": set, "found": rows.len() }))
}

/// What was imported last, if anything.
pub fn imported() -> Value {
    std::fs::read_to_string(data_dir().join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|c| c["imported"].clone())
        .unwrap_or(Value::Null)
}
