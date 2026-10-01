pub mod format;
pub mod mangohud;
pub mod metal;
pub mod presentmon;
pub mod session;
pub mod stats;

use serde_json::{json, Value};

use super::store::Ctx;
use super::tools::blocking;
use super::{current_os, Os};
use format::Machine;
pub use session::SessionSummary;

const DEFAULT_SECONDS: u64 = 60;

fn caps() -> Value {
    super::caps::cached(false)
}

pub fn capabilities_for(os: Os, c: &Value, presentmon_installed: bool) -> Value {
    let flag = |p: &str| c.pointer(p).and_then(Value::as_bool).unwrap_or(false);
    let mut v = match os {
        Os::Windows => {
            let group = flag("/windows/performance_log_users");
            let admin = flag("/is_admin");
            let ok = group || admin;
            json!({
                "available": ok,
                "method": "presentmon",
                "missing": if ok { json!([]) } else { json!(["performance_log_users"]) },
                "frames": ok,
                "reason": if ok { Value::Null } else { json!("needs_performance_log_users") },
                "windows": {
                    "performance_log_users": group,
                    "is_admin": admin,
                    "setup_action": if ok { Value::Null } else { json!("perf.setup_access") },
                    "setup_needs_sign_out": true,
                    "per_capture_uac": false,
                },
                "presentmon": {
                    "installed": presentmon_installed,
                    "version": presentmon::VERSION,
                    "sha256": presentmon::SHA256,
                    "download_on_first_use": !presentmon_installed,
                    "license": "MIT",
                },
                "anti_cheat_safe": true,
            })
        }
        Os::Linux => {
            let mango = flag("/linux/mangohud");
            let flatpak = flag("/linux/flatpak_sandbox");
            let ok = mango && !flatpak;
            let mut missing = vec![];
            if !mango {
                missing.push("mangohud");
            }
            if flatpak {
                missing.push("flatpak_sandbox");
            }
            json!({
                "available": ok,
                "method": "mangohud",
                "missing": missing,
                "frames": ok,
                "reason": if flatpak { json!("flatpak_sandbox") } else if !mango { json!("install_mangohud") } else { json!("start_game_with_command") },
                "linux": { "mangohud": mango, "flatpak": flatpak, "needs_launch": true, "running_game": "unavailable" },
            })
        }
        Os::Macos => json!({
            "available": true,
            "method": "metal_hud",
            "missing": [],
            "frames": "launched_only",
            "reason": "mac_hud_log_private",
            "macos": {
                "metal_hud": true,
                "hud_log_readable": false,
                "frames_when_started_by_omniget": true,
                "running_game": "telemetry_only",
                "checked_on": "macOS 26.5.2",
            },
        }),
    };
    v["os"] = json!(os.as_str());
    v["telemetry"] = json!(true);
    v["active"] = json!(session::active_id());
    v
}

pub async fn tool_capabilities(_a: Value) -> Result<Value, String> {
    blocking(|| {
        #[cfg(windows)]
        let installed = presentmon::installed().is_some();
        #[cfg(not(windows))]
        let installed = false;
        Ok(capabilities_for(current_os(), &caps(), installed))
    })
    .await
}

fn clean_process_name(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty()
        || s.len() > 200
        || s.chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '"' | '*' | '?' | '<' | '>' | '|'))
    {
        return Err("process_name must be a program name such as \"League of Legends.exe\"".into());
    }
    Ok(s.to_string())
}

async fn machine() -> Machine {
    let c = caps();
    let gpu = super::monitor::tool_sensors(json!({}))
        .await
        .ok()
        .and_then(|v| {
            v.pointer("/gpu/0/name")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .unwrap_or_default();
    let (cpu, ram) = blocking(|| {
        use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        let cpu = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();
        Ok((
            cpu,
            stats::round(sys.total_memory() as f64 / 1_073_741_824.0, 1),
        ))
    })
    .await
    .unwrap_or_default();
    Machine {
        os: current_os().as_str().into(),
        os_version: c["os_version"].as_str().unwrap_or("").into(),
        arch: std::env::consts::ARCH.into(),
        cpu,
        gpu,
        ram_gb: ram,
    }
}

fn start_args(a: &Value) -> Result<session::StartArgs, String> {
    let pid = a["pid"]
        .as_u64()
        .filter(|p| *p > 0 && *p <= u32::MAX as u64)
        .map(|p| p as u32);
    let process_name = match a["process_name"].as_str() {
        Some(s) if !s.trim().is_empty() => Some(clean_process_name(s)?),
        _ => None,
    };
    let max_seconds = a["max_seconds"]
        .as_u64()
        .unwrap_or(DEFAULT_SECONDS)
        .clamp(5, 3600);
    let label = a["label"]
        .as_str()
        .map(|s| s.trim().chars().take(120).collect::<String>())
        .filter(|s| !s.is_empty())
        .or_else(|| process_name.clone())
        .unwrap_or_else(|| "Capture".into());
    Ok(session::StartArgs {
        pid,
        process_name,
        max_seconds,
        label,
        machine: Machine::default(),
    })
}

fn started(s: &format::Session) -> Value {
    json!({
        "session_id": s.id,
        "status": s.status,
        "method": s.method,
        "max_seconds": s.max_seconds,
        "launch": s.launch,
        "reason": s.reason,
    })
}

pub async fn tool_capture_start(a: Value) -> Result<Value, String> {
    if let Some(id) = session::active_id() {
        return Err(format!(
            "BUSY: capture {id} is already running; stop it first"
        ));
    }
    let mut args = start_args(&a)?;
    args.machine = machine().await;
    let ctx = Ctx::app()?;
    match current_os() {
        Os::Windows => {
            #[cfg(windows)]
            {
                let c = capabilities_for(Os::Windows, &caps(), true);
                if c["available"] != json!(true) {
                    return Err("UNAVAILABLE: needs_performance_log_users — PresentMon needs the one-time setup on the Washmybelly screen (join Performance Log Users, then sign out)".into());
                }
                if args.pid.is_none() && args.process_name.is_none() {
                    return Err("pid or process_name is required".into());
                }
                let exe = presentmon::ensure().await?;
                let s = blocking(move || session::start_presentmon(&ctx, args, exe)).await?;
                Ok(started(&s))
            }
            #[cfg(not(windows))]
            unreachable!()
        }
        Os::Linux => {
            let c = capabilities_for(Os::Linux, &caps(), false);
            if c["available"] != json!(true) {
                return Err(format!(
                    "UNAVAILABLE: {}",
                    c["reason"].as_str().unwrap_or("mangohud")
                ));
            }
            if args.pid.is_some() {
                return Err("UNAVAILABLE: already_running — MangoHud only measures a game started with it. Call without pid (optionally with process_name) to get the launch command, then start the game with it".into());
            }
            let s = blocking(move || session::start_mangohud(&ctx, args)).await?;
            Ok(started(&s))
        }
        Os::Macos => {
            let s = blocking(move || {
                session::start_telemetry(&ctx, args, "mac_running_game_no_frames")
            })
            .await?;
            Ok(started(&s))
        }
    }
}

pub async fn tool_capture_stop(a: Value) -> Result<Value, String> {
    if let (Some(want), Some(cur)) = (a["session_id"].as_str(), session::active_id()) {
        if want != cur {
            return Err(format!(
                "NOT_RUNNING: {want} is not the running capture ({cur})"
            ));
        }
    }
    let s = blocking(|| session::finish("stopped")).await?;
    let sum = Ctx::app()
        .ok()
        .and_then(|ctx| session::summary_of(&ctx, &s.id).ok());
    Ok(json!({
        "session_id": s.id,
        "status": s.status,
        "frames": s.frames,
        "reason": s.reason,
        "fps_avg": sum.as_ref().and_then(|x| x.stats.fps_avg),
        "low_1": sum.as_ref().and_then(|x| x.stats.low_1),
        "low_01": sum.as_ref().and_then(|x| x.stats.low_01),
        "stutters": sum.as_ref().map(|x| x.stats.stutters),
    }))
}

pub async fn tool_sessions(a: Value) -> Result<Value, String> {
    let limit = a["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
    blocking(move || {
        let ctx = Ctx::app()?;
        let mut list = session::list(&ctx);
        list.truncate(limit);
        Ok(json!({ "sessions": list, "active": session::active_id() }))
    })
    .await
}

pub async fn tool_summary(a: Value) -> Result<Value, String> {
    let id = a["session_id"]
        .as_str()
        .ok_or("session_id is required")?
        .to_string();
    blocking(move || {
        let ctx = Ctx::app()?;
        summary_value(&ctx, &id)
    })
    .await
}

fn summary_value(ctx: &Ctx, id: &str) -> Result<Value, String> {
    let sum = session::summary_of(ctx, id)?;
    let mut v = serde_json::to_value(&sum).map_err(|e| e.to_string())?;
    for k in ["stutter_events", "freeze_events"] {
        if let Some(arr) = v[k].as_array_mut() {
            arr.truncate(50);
        }
    }
    Ok(v)
}

pub fn compare_value(ctx: &Ctx, before: &str, after: &str) -> Result<Value, String> {
    let b = session::summary_of(ctx, before)?;
    let a = session::summary_of(ctx, after)?;
    let bd = session::session_dir(ctx, before)?;
    let ad = session::session_dir(ctx, after)?;
    let sb = session::load_session(&bd)?;
    let sa = session::load_session(&ad)?;
    let noise = |sum: &format::Summary| -> Vec<(&'static str, Option<f64>)> {
        stats::COMPARED
            .iter()
            .map(|(m, _)| (*m, sum.noise.get(*m).copied().flatten()))
            .collect()
    };
    let deltas = stats::compare(&b.stats, &a.stats, &noise(&b), &noise(&a));
    let mut warnings = vec![];
    if b.source != a.source {
        warnings.push("different_frame_source");
    }
    if sb.method != sa.method {
        warnings.push("different_method");
    }
    if sb.machine.gpu != sa.machine.gpu || sb.machine.cpu != sa.machine.cpu {
        warnings.push("different_hardware");
    }
    if sb.machine.os_version != sa.machine.os_version {
        warnings.push("different_os_version");
    }
    if b.stats.frames < 1000 || a.stats.frames < 1000 {
        warnings.push("short_capture");
    }
    let better = deltas.iter().filter(|d| d.verdict == "better").count();
    let worse = deltas.iter().filter(|d| d.verdict == "worse").count();
    let verdict = match (better, worse) {
        (0, 0) => "same",
        (b, 0) if b > 0 => "better",
        (0, w) if w > 0 => "worse",
        _ => "mixed",
    };
    let pick = |s: &format::Summary, meta: &format::Session| {
        json!({
            "session_id": s.session_id, "label": s.label, "started_ms": meta.started_ms,
            "fps_avg": s.stats.fps_avg, "low_1": s.stats.low_1, "low_01": s.stats.low_01,
            "stutters": s.stats.stutters, "freezes": s.stats.freezes, "frames": s.stats.frames,
        })
    };
    Ok(json!({
        "before": pick(&b, &sb),
        "after": pick(&a, &sa),
        "delta": {
            "verdict": verdict,
            "metrics": deltas,
            "stutters": a.stats.stutters as i64 - b.stats.stutters as i64,
            "freezes": a.stats.freezes as i64 - b.stats.freezes as i64,
        },
        "warnings": warnings,
        "noise_rule": "a change counts only when it is larger than how much the metric moves between the two halves of either session (at least 1%, or 0.5 points for time percentages)",
    }))
}

pub async fn tool_compare(a: Value) -> Result<Value, String> {
    let before = a["before"]
        .as_str()
        .ok_or("before is required")?
        .to_string();
    let after = a["after"].as_str().ok_or("after is required")?.to_string();
    blocking(move || compare_value(&Ctx::app()?, &before, &after)).await
}

fn chart(ctx: &Ctx, id: &str, points: usize) -> Result<Value, String> {
    let dir = session::session_dir(ctx, id)?;
    let s = session::load_session(&dir).map_err(|_| "NOT_FOUND: unknown session".to_string())?;
    let frames = session::read_frames(&dir);
    let ((source, ft, t), _, _) = format::pick_source(&frames);
    let sum = session::summary_of(ctx, id).ok();
    Ok(json!({
        "session_id": s.id,
        "source": source,
        "series": stats::downsample(&t, &ft, points),
        "histogram": stats::histogram(&ft, 40),
        "sensors": session::read_sensors(&dir),
        "stutter_events": sum.as_ref().map(|x| x.stutter_events.clone()).unwrap_or_default(),
        "freeze_events": sum.as_ref().map(|x| x.freeze_events.clone()).unwrap_or_default(),
        "duration_ms": t.last().copied().unwrap_or(0.0),
    }))
}

fn delete_session(ctx: &Ctx, id: &str) -> Result<Value, String> {
    if session::active_id().as_deref() == Some(id) {
        return Err("BUSY: stop the capture first".into());
    }
    let dir = session::session_dir(ctx, id)?;
    if !dir.join("session.json").is_file() {
        return Err("NOT_FOUND: unknown session".into());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(json!({ "deleted": id }))
}

#[cfg(windows)]
fn setup_access() -> Result<Value, String> {
    let sid = presentmon::current_user_sid()?;
    let user = presentmon::account_of(&sid).unwrap_or_else(|_| sid.clone());
    let group = presentmon::account_of(presentmon::PERF_LOG_USERS_SID)
        .map(|g| presentmon::bare_group(&g).to_string())
        .unwrap_or_else(|_| "Performance Log Users".into());
    let joined = super::elevate::run(
        &Ctx::app()?,
        &[super::elevate::ElevatedOp::PerfLogUsers {
            user_sid: sid.clone(),
            join: true,
        }],
    );
    let ready = matches!(&joined, Ok(r) if r.iter().all(|x| x.ok));
    Ok(json!({
        "ready": ready,
        "error": joined.err(),
        "user_sid": sid,
        "group_sid": presentmon::PERF_LOG_USERS_SID,
        "group": group,
        "sign_out_needed": true,
        "commands": [format!("net localgroup \"{group}\" \"{user}\" /add")],
        "undo_commands": [format!("net localgroup \"{group}\" \"{user}\" /delete")],
    }))
}

pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    match action {
        "status" => Ok(session::status()),
        "chart" => {
            let id = a["session_id"]
                .as_str()
                .ok_or("session_id is required")?
                .to_string();
            let points = a["points"].as_u64().unwrap_or(600).clamp(50, 4000) as usize;
            blocking(move || chart(&Ctx::app()?, &id, points)).await
        }
        "delete_session" => {
            let id = a["session_id"]
                .as_str()
                .ok_or("session_id is required")?
                .to_string();
            blocking(move || delete_session(&Ctx::app()?, &id)).await
        }
        "setup_access" => {
            #[cfg(windows)]
            {
                blocking(setup_access).await
            }
            #[cfg(not(windows))]
            {
                Err("setup_access exists only on Windows".into())
            }
        }
        "launch" => {
            #[cfg(target_os = "macos")]
            {
                if let Some(id) = session::active_id() {
                    return Err(format!("BUSY: capture {id} is already running"));
                }
                let path = std::path::PathBuf::from(a["path"].as_str().ok_or("path is required")?);
                let mut args = start_args(&a)?;
                args.pid = None;
                args.machine = machine().await;
                blocking(move || {
                    let exe = metal::executable_of(&path)?;
                    if args.label == "Capture" {
                        args.label = path
                            .file_stem()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                    }
                    let s = session::start_metal(&Ctx::app()?, args, &exe)?;
                    Ok(started(&s))
                })
                .await
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = a;
                Err("launch exists only on macOS; on Linux start the game with the MangoHud command".into())
            }
        }
        _ => Err(format!("perf: unknown screen action {action}")),
    }
}
