use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::super::elevate::ElevatedOp;
use super::super::super::fsops::{self, Fingerprint};
use super::super::super::plan::Outcome;
use super::super::super::protect::{self, Guard};
use super::super::super::receipt::Undo;
use super::super::super::store::Ctx;
use super::parse;

fn arg_str<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    a.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("apps.mac: missing {k}"))
}

fn arg_fp(a: &Value) -> Result<Fingerprint, String> {
    serde_json::from_value(a.get("fp").cloned().ok_or("apps.mac: missing fp")?)
        .map_err(|e| e.to_string())
}

fn bundle_arg(a: &Value) -> Result<String, String> {
    let b = arg_str(a, "bundle_id")?;
    if !parse::valid_bundle_id(b) || parse::is_apple(b) {
        return Err(format!("apps.mac: refused bundle id {b}"));
    }
    Ok(b.to_string())
}

fn label_arg(a: &Value) -> Result<String, String> {
    let l = arg_str(a, "label")?;
    if !parse::valid_label(l) || parse::is_apple(l) {
        return Err(format!("apps.mac: refused label {l}"));
    }
    Ok(l.to_string())
}

fn launch_plist(a: &Value, allowed: &[&str]) -> Result<PathBuf, String> {
    let plist = PathBuf::from(arg_str(a, "plist")?);
    let parent = plist.parent().ok_or("apps.mac: bad plist path")?;
    let dir = parent.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if !allowed.contains(&dir) || plist.extension().is_none_or(|e| e != "plist") {
        return Err(format!(
            "apps.mac: {} is not a launchd job file",
            plist.display()
        ));
    }
    protect::check(&plist, Guard::Picked, Some(parent))?;
    fsops::unchanged(&plist, &arg_fp(a)?)?;
    Ok(plist)
}

fn uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::getuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn system(prog: &str, args: &[&str]) -> Result<String, String> {
    if cfg!(test) || !cfg!(target_os = "macos") {
        return Ok(String::new());
    }
    let out = crate::core::process::std_command(prog)
        .args(args)
        .output()
        .map_err(|e| format!("{prog}: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text.trim().to_string())
    } else {
        Err(text.trim().to_string())
    }
}

fn stash(ctx: &Ctx, receipt_id: &str, path: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().ok_or("apps.mac: no file name")?;
    let slot = ctx
        .quarantine_dir()
        .join(receipt_id)
        .join(format!("apps-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&slot).map_err(|e| e.to_string())?;
    let dest = slot.join(name);
    std::fs::rename(path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(dest)
}

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    match action {
        "bootout_agent" => {
            let label = label_arg(args)?;
            let plist = launch_plist(args, &["LaunchAgents"])?;
            let bytes = fsops::measure(&plist).0;
            let stopped = system(
                "/bin/launchctl",
                &["bootout", &format!("gui/{}/{label}", uid())],
            );
            let dest = stash(ctx, receipt_id, &plist)?;
            Ok(Outcome {
                detail: match stopped {
                    Ok(_) => format!("stopped {label}; its file is in quarantine"),
                    Err(e) => format!("{label} was not running ({e}); its file is in quarantine"),
                },
                bytes,
                undo: Some(Undo::Module {
                    module: "apps".into(),
                    action: "mac.restore_agent".into(),
                    args: json!({ "from": dest, "to": plist, "label": label }),
                }),
            })
        }
        "remove_prefs" => {
            let bid = bundle_arg(args)?;
            if let Some(app) = args.get("app_path").and_then(Value::as_str) {
                if super::running(
                    Path::new(app),
                    args.get("executable").and_then(Value::as_str),
                ) {
                    return Err("APP_OPEN: quit the app first".into());
                }
            }
            let plist = PathBuf::from(arg_str(args, "plist")?);
            let root = PathBuf::from(arg_str(args, "root")?);
            if plist.file_name().and_then(|n| n.to_str()) != Some(&format!("{bid}.plist"))
                || root.file_name().and_then(|n| n.to_str()) != Some("Preferences")
            {
                return Err(format!(
                    "apps.mac: {} is not the preferences file of {bid}",
                    plist.display()
                ));
            }
            protect::check(&plist, Guard::Picked, Some(&root))?;
            fsops::unchanged(&plist, &arg_fp(args)?)?;
            let bytes = fsops::measure(&plist).0;
            let dest = stash(ctx, receipt_id, &plist)?;
            let _ = system("/usr/bin/defaults", &["delete", &bid]);
            Ok(Outcome {
                detail: "preferences cleared; a copy is in quarantine".into(),
                bytes,
                undo: Some(Undo::Module {
                    module: "apps".into(),
                    action: "mac.restore_prefs".into(),
                    args: json!({ "from": dest, "to": plist, "bundle_id": bid }),
                }),
            })
        }
        _ => Err(format!("apps.mac: unknown action {action}")),
    }
}

pub fn plan_elevated(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    match action {
        "remove_daemon" => {
            let label = label_arg(args)?;
            let domain = arg_str(args, "domain")?;
            let ok_domain = domain == "system"
                || domain
                    .strip_prefix("gui/")
                    .is_some_and(|u| !u.is_empty() && u.chars().all(|c| c.is_ascii_digit()));
            if !ok_domain {
                return Err(format!("apps.mac: refused launchd domain {domain}"));
            }
            let plist = launch_plist(args, &["LaunchAgents", "LaunchDaemons"])?;
            let fp = arg_fp(args)?;
            Ok((
                vec![
                    ElevatedOp::LaunchctlBootout {
                        domain: domain.to_string(),
                        label,
                    },
                    ElevatedOp::RemovePath {
                        root: plist.parent().map(Path::to_path_buf),
                        path: plist,
                        guard: Guard::Picked,
                        fp,
                        contents_only: false,
                    },
                ],
                None,
            ))
        }
        "pkg_forget" => {
            let id = arg_str(args, "id")?;
            check_pkg_id(id)?;
            Ok((vec![ElevatedOp::PkgForget { id: id.to_string() }], None))
        }
        _ => Err(format!(
            "apps.mac: action {action} has no administrator form"
        )),
    }
}

pub fn plan_undo(_ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    match action {
        "restore_agent" => {
            let from = PathBuf::from(arg_str(args, "from")?);
            let to = PathBuf::from(arg_str(args, "to")?);
            let label = label_arg(args)?;
            super::super::super::quarantine::restore(&from, &to)?;
            let to_s = to.to_string_lossy().to_string();
            match system(
                "/bin/launchctl",
                &["bootstrap", &format!("gui/{}", uid()), &to_s],
            ) {
                Ok(_) => Ok(format!("{label} is back and running")),
                Err(e) => Ok(format!(
                    "{label} is back; it starts at the next login ({e})"
                )),
            }
        }
        "restore_prefs" => {
            let bid = bundle_arg(args)?;
            let from = PathBuf::from(arg_str(args, "from")?);
            let to = PathBuf::from(arg_str(args, "to")?);
            if cfg!(target_os = "macos") && !cfg!(test) {
                let from_s = from.to_string_lossy().to_string();
                if system("/usr/bin/defaults", &["import", &bid, &from_s]).is_ok() {
                    return Ok(format!("preferences of {bid} are back"));
                }
            }
            super::super::super::quarantine::restore(&from, &to)?;
            Ok(format!("preferences of {bid} are back"))
        }
        _ => Err(format!("apps.mac: cannot undo {action}")),
    }
}

fn check_pkg_id(id: &str) -> Result<(), String> {
    if !parse::valid_label(id) {
        return Err(format!("pkg_forget: refused id {id}"));
    }
    if parse::is_apple(id) {
        return Err(format!("pkg_forget: {id} belongs to macOS"));
    }
    Ok(())
}

pub fn elevated_pkg_forget(id: &str) -> Result<String, String> {
    check_pkg_id(id)?;
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/sbin/pkgutil")
            .arg("--forget")
            .arg(id)
            .env_clear()
            .output()
            .map_err(|e| format!("pkgutil: {e}"))?;
        if out.status.success() {
            Ok(format!("forgot the receipt {id}"))
        } else {
            Err(format!(
                "pkgutil --forget {id}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(format!("pkg_forget {id}: macOS only"))
    }
}
