//! Maintenance with a real effect: DNS cache, LaunchServices, Spotlight, Quick Look, verify disk, sfc/DISM, fstrim (F7).
//!
//! Each task is a row of a closed table: an id, the operating system it
//! belongs to, whether it needs administrator rights, and the exact argv of
//! every step (programs by absolute path). The screen asks for a plan of
//! kind `maintenance` with the ids it wants; every item is an
//! `Op::Module { module: "maintenance", action: <id> }`. The plan engine
//! runs the ones that need no administrator through [`plan_execute`] and
//! hands the rest to the helper as `ElevatedOp::Maintenance { id }`, where
//! [`elevated_run`] rebuilds the argv from the id alone. Nothing from the
//! plan besides the id reaches a command line.
//!
//! None of these tasks can be undone, and none needs to: they rebuild a
//! cache the system fills again, check a disk, or repair system files from
//! Windows' own store. What is not here on purpose: `purge`, `periodic`,
//! `DISM /ResetBase`, anything sold as a speed-up.

use serde::Serialize;
use serde_json::{json, Value};

use super::elevate::ElevatedOp;
use super::plan::{self, Op, Outcome, PlanItem, Source};
use super::receipt::Undo;
use super::store::Ctx;
use super::{current_os, Os, Risk};

pub const MODULE: &str = "maintenance";
pub const PLAN_KIND: &str = "maintenance";

/// One maintenance task of the closed table.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Task {
    pub id: &'static str,
    #[serde(skip)]
    pub os: Os,
    /// Runs through `omniget-elevate` (one password or UAC prompt per plan).
    pub admin: bool,
    /// Rough duration in minutes, for the card ("about 20 min").
    pub minutes: u32,
    /// Takes long enough that the screen warns before running.
    pub long: bool,
    /// `network`, `apps`, `search`, `disk`, `system`.
    pub group: &'static str,
    pub title: &'static str,
    /// What it does and what changes after it, in plain words.
    pub what: &'static str,
}

const LSREGISTER: &str =
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

pub const TASKS: &[Task] = &[
    // ── macOS ──
    Task {
        id: "dns_flush",
        os: Os::Macos,
        admin: true,
        minutes: 1,
        long: false,
        group: "network",
        title: "Clear the DNS cache",
        what: "Forgets the addresses of websites the Mac remembered, so a site that moved or a changed network setting works right away. Nothing else changes.",
    },
    Task {
        id: "launchservices_rebuild",
        os: Os::Macos,
        admin: false,
        minutes: 2,
        long: false,
        group: "apps",
        title: "Rebuild the \"Open With\" list",
        what: "Cleans out old entries and registers the apps on this Mac again. Fixes duplicate or missing apps in \"Open With\" and wrong icons. Your default apps stay.",
    },
    Task {
        id: "spotlight_reindex",
        os: Os::Macos,
        admin: true,
        minutes: 60,
        long: true,
        group: "search",
        title: "Rebuild the Spotlight index",
        what: "Erases the search index of the startup disk and builds it again. Fixes searches that miss files. Search results stay incomplete while it rebuilds, which can take hours.",
    },
    Task {
        id: "quicklook_reset",
        os: Os::Macos,
        admin: false,
        minutes: 1,
        long: false,
        group: "apps",
        title: "Reset Quick Look previews",
        what: "Clears the cache of file thumbnails and previews. Fixes previews that show an old or broken image; they are made again the next time you look.",
    },
    Task {
        id: "disk_verify",
        os: Os::Macos,
        admin: false,
        minutes: 15,
        long: true,
        group: "disk",
        title: "Check the startup disk",
        what: "Reads the file system of the startup disk and reports problems, like Disk Utility's First Aid. It only checks; nothing is repaired or changed.",
    },
    // ── Windows ──
    Task {
        id: "dns_flush",
        os: Os::Windows,
        admin: false,
        minutes: 1,
        long: false,
        group: "network",
        title: "Clear the DNS cache",
        what: "Forgets the addresses of websites Windows remembered, so a site that moved or a changed network setting works right away. Nothing else changes.",
    },
    Task {
        id: "sfc_scan",
        os: Os::Windows,
        admin: true,
        minutes: 20,
        long: true,
        group: "system",
        title: "Check and repair system files",
        what: "System File Checker (sfc /scannow) compares Windows' own files with the copies Windows keeps and puts back any that are damaged. Your files and apps are not touched.",
    },
    Task {
        id: "dism_restore_health",
        os: Os::Windows,
        admin: true,
        minutes: 20,
        long: true,
        group: "system",
        title: "Repair the Windows component store",
        what: "DISM /RestoreHealth fixes the store sfc repairs from, downloading clean parts through Windows Update. Run it when sfc says it could not fix everything, then run sfc again. Old updates are kept.",
    },
    // ── Linux ──
    Task {
        id: "fstrim",
        os: Os::Linux,
        admin: true,
        minutes: 2,
        long: false,
        group: "disk",
        title: "Trim SSDs",
        what: "Tells every mounted SSD which blocks are free (fstrim -av), which keeps writes fast. Most systems already do this once a week; running it again is harmless.",
    },
    Task {
        id: "dns_flush",
        os: Os::Linux,
        admin: false,
        minutes: 1,
        long: false,
        group: "network",
        title: "Clear the DNS cache",
        what: "Asks systemd-resolved to forget cached addresses, so a site that moved or a changed network setting works right away.",
    },
];

/// The task `id` of `os`.
pub fn task(os: Os, id: &str) -> Option<&'static Task> {
    TASKS.iter().find(|t| t.os == os && t.id == id)
}

/// The argv of every step of task `id` on `os`, programs by absolute path.
/// `sysroot` is `%SystemRoot%` on Windows; `exists` picks among the fixed
/// places a Linux distribution keeps a program. The closed table: an id
/// that is not here gets `None`.
pub fn steps_for(
    os: Os,
    id: &str,
    sysroot: &str,
    exists: &dyn Fn(&str) -> bool,
) -> Option<Vec<Vec<String>>> {
    let v = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let first = |cands: &[&str]| cands.iter().find(|c| exists(c)).map(|c| c.to_string());
    let sys32 = |exe: &str| format!("{}\\System32\\{exe}", sysroot.trim_end_matches('\\'));
    Some(match (os, id) {
        (Os::Macos, "dns_flush") => vec![
            v(&["/usr/bin/dscacheutil", "-flushcache"]),
            v(&["/usr/bin/killall", "-HUP", "mDNSResponder"]),
        ],
        // `lsregister -kill` is gone from current macOS; garbage-collect the
        // database, then force a rescan of the local, system and user domains.
        (Os::Macos, "launchservices_rebuild") => vec![
            v(&[LSREGISTER, "-gc"]),
            v(&[LSREGISTER, "-r", "-f", "-all", "local,system,user"]),
        ],
        (Os::Macos, "spotlight_reindex") => vec![v(&["/usr/bin/mdutil", "-E", "/"])],
        (Os::Macos, "quicklook_reset") => vec![v(&["/usr/bin/qlmanage", "-r", "cache"])],
        (Os::Macos, "disk_verify") => vec![v(&["/usr/sbin/diskutil", "verifyVolume", "/"])],
        (Os::Windows, "dns_flush") => vec![vec![sys32("ipconfig.exe"), "/flushdns".into()]],
        (Os::Windows, "sfc_scan") => vec![vec![sys32("sfc.exe"), "/scannow".into()]],
        // Never `/ResetBase`: it makes installed updates impossible to remove.
        (Os::Windows, "dism_restore_health") => vec![vec![
            sys32("Dism.exe"),
            "/Online".into(),
            "/Cleanup-Image".into(),
            "/RestoreHealth".into(),
        ]],
        (Os::Linux, "fstrim") => {
            vec![vec![
                first(&[
                    "/usr/sbin/fstrim",
                    "/sbin/fstrim",
                    "/usr/bin/fstrim",
                    "/bin/fstrim",
                ])?,
                "-av".into(),
            ]]
        }
        (Os::Linux, "dns_flush") => vec![vec![
            first(&["/usr/bin/resolvectl", "/bin/resolvectl"])?,
            "flush-caches".into(),
        ]],
        _ => return None,
    })
}

fn sysroot() -> String {
    std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())
}

fn file_exists(p: &str) -> bool {
    std::path::Path::new(p).is_file()
}

/// Steps of `id` on this machine.
fn steps(id: &str) -> Option<Vec<Vec<String>>> {
    steps_for(current_os(), id, &sysroot(), &file_exists)
}

/// The tasks of this OS with whether they can run here.
pub fn list() -> Value {
    let os = current_os();
    let tasks: Vec<Value> = TASKS
        .iter()
        .filter(|t| t.os == os)
        .map(|t| {
            let s = steps(t.id);
            let available = s
                .as_ref()
                .is_some_and(|s| s.iter().all(|a| file_exists(&a[0])));
            json!({
                "id": t.id,
                "title": t.title,
                "what": t.what,
                "admin": t.admin,
                "minutes": t.minutes,
                "long": t.long,
                "group": t.group,
                "available": available,
                "reason": if available { Value::Null } else { json!("program_missing") },
                "command": command_line(t.id),
                "undo": "none",
            })
        })
        .collect();
    json!({ "os": os.as_str(), "tasks": tasks })
}

/// Plan items for the chosen ids (unknown ids are an error).
pub fn items(os: Os, ids: &[String]) -> Result<Vec<PlanItem>, String> {
    let mut out = Vec::new();
    for id in ids {
        let t = task(os, id).ok_or_else(|| format!("maintenance: unknown task {id}"))?;
        out.push(PlanItem {
            id: format!("maintenance:{}", t.id),
            label: t.title.to_string(),
            detail: Some(format!(
                "{} Nothing to undo: this task only rebuilds, checks or repairs.",
                t.what
            )),
            group: t.group.to_string(),
            bytes: 0,
            risk: Risk::Reversible,
            admin: t.admin,
            op: Op::Module {
                module: MODULE.into(),
                action: t.id.into(),
                args: Value::Null,
            },
        });
    }
    Ok(out)
}

/// Windows tools like sfc write UTF-16LE to a pipe; everything else is UTF-8
/// (or the OEM code page, read lossily).
pub fn decode_output(bytes: &[u8]) -> String {
    let zeros = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    if bytes.len() >= 4 && zeros * 2 >= bytes.len() / 2 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
            .trim_start_matches('\u{feff}')
            .to_string()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// The last meaningful lines of a tool's output, for the receipt.
pub fn summarize(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| {
            !l.is_empty()
                && !l.chars().all(|c| {
                    c == '=' || c == '-' || c == '.' || c == '%' || c.is_ascii_digit() || c == ' '
                })
        })
        .collect();
    let tail = &lines[lines.len().saturating_sub(3)..];
    let s = tail.join(" · ");
    if s.chars().count() > 400 {
        let cut: String = s
            .chars()
            .rev()
            .take(400)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{cut}")
    } else {
        s
    }
}

/// Run the steps of a task, streaming each output line as progress.
fn run_steps(id: &str, steps: &[Vec<String>]) -> Result<String, String> {
    use std::io::{BufRead, Read};
    let total = steps.len() as u64;
    let mut all = String::new();
    for (i, argv) in steps.iter().enumerate() {
        let (prog, args) = argv.split_first().ok_or("empty step")?;
        super::progress(
            MODULE,
            id,
            i as u64,
            total,
            prog.rsplit(['/', '\\']).next().unwrap_or(prog).to_string(),
        );
        let mut child = crate::core::process::std_command(prog)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("{prog}: {e}"))?;
        let mut err = child.stderr.take();
        let err_reader = std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_end(&mut b);
            }
            b
        });
        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            // sfc and DISM write UTF-16 on Windows: a byte split on `\n`
            // would cut it, so Windows reads the whole output at once.
            if cfg!(windows) {
                let mut b = Vec::new();
                let _ = stdout.read_to_end(&mut b);
                out = decode_output(&b).replace('\r', "");
            } else {
                for line in std::io::BufReader::new(stdout)
                    .split(b'\n')
                    .map_while(Result::ok)
                {
                    let line = decode_output(&line);
                    let line = line.trim_end_matches('\r');
                    if !line.trim().is_empty() {
                        super::progress(MODULE, id, i as u64, total, line.trim().to_string());
                    }
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        let stderr = decode_output(&err_reader.join().unwrap_or_default());
        all.push_str(&out);
        if !status.success() {
            let msg = summarize(&format!("{out}\n{stderr}"));
            return Err(if msg.is_empty() {
                format!("{prog} exited with {status}")
            } else {
                msg
            });
        }
    }
    super::progress(MODULE, id, total, total, "done");
    let s = summarize(&all);
    Ok(if s.is_empty() { "done".into() } else { s })
}

// ── Plan hooks (called by `plan::execute` for `Op::Module { module: "maintenance" }`) ──

pub fn plan_execute(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    _args: &Value,
) -> Result<Outcome, String> {
    let t = task(current_os(), action)
        .ok_or_else(|| format!("maintenance: unknown action {action}"))?;
    if t.admin {
        return Err(format!("maintenance: {action} needs administrator rights"));
    }
    let steps =
        steps(t.id).ok_or_else(|| format!("maintenance: {action} is not available here"))?;
    let detail = run_steps(t.id, &steps)?;
    Ok(Outcome {
        detail,
        bytes: 0,
        undo: None,
    })
}

pub fn plan_elevated(
    _ctx: &Ctx,
    _receipt_id: &str,
    action: &str,
    _args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    match task(current_os(), action) {
        Some(t) if t.admin => Ok((
            vec![ElevatedOp::Maintenance {
                id: t.id.to_string(),
            }],
            None,
        )),
        _ => Err(format!(
            "maintenance: action {action} has no administrator form"
        )),
    }
}

pub fn plan_undo(_ctx: &Ctx, action: &str, _args: &Value) -> Result<String, String> {
    Err(format!(
        "maintenance: {action} has nothing to undo (it only rebuilds, checks or repairs)"
    ))
}

/// Runs as root inside `omniget-elevate`. Only the administrator tasks of
/// this OS; the argv comes from the closed table, never from the plan.
pub fn elevated_run(id: &str) -> Result<String, String> {
    elevated_steps(current_os(), id, &sysroot(), &file_exists)
        .and_then(|steps| run_steps(id, &steps))
}

/// The steps `elevated_run` would take: refuses unknown ids and tasks that
/// do not need administrator rights.
pub fn elevated_steps(
    os: Os,
    id: &str,
    sysroot: &str,
    exists: &dyn Fn(&str) -> bool,
) -> Result<Vec<Vec<String>>, String> {
    match task(os, id) {
        Some(t) if t.admin => steps_for(os, id, sysroot, exists)
            .ok_or_else(|| format!("maintenance: {id} is not available here")),
        Some(_) => Err(format!("maintenance: {id} does not run as administrator")),
        None => Err(format!("maintenance: unknown task {id}")),
    }
}

/// The command line of maintenance task `id` (display only).
pub fn command_line(id: &str) -> String {
    let os = current_os();
    command_line_for(os, id, &sysroot(), &file_exists)
}

pub fn command_line_for(os: Os, id: &str, sysroot: &str, exists: &dyn Fn(&str) -> bool) -> String {
    let Some(t) = task(os, id) else {
        return format!("# unknown maintenance task {id}");
    };
    let Some(steps) = steps_for(os, id, sysroot, exists) else {
        return format!("# {id} is not available here");
    };
    let sudo = if t.admin && os != Os::Windows {
        "sudo "
    } else {
        ""
    };
    let quote = |a: &String| {
        if a.contains(' ') {
            format!("\"{a}\"")
        } else {
            a.clone()
        }
    };
    let lines: Vec<String> = steps
        .iter()
        .map(|s| {
            format!(
                "{sudo}{}",
                s.iter().map(quote).collect::<Vec<_>>().join(" ")
            )
        })
        .collect();
    let joined = lines.join(if os == Os::Windows { " ; " } else { " && " });
    if t.admin && os == Os::Windows {
        format!("{joined}   # in an administrator terminal")
    } else {
        joined
    }
}

/// Actions only the screen may run (`washmybelly_ui` with `maintenance.<action>`):
/// never exposed as MCP tools.
pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    match action {
        "list" => super::tools::blocking(|| Ok(list())).await,
        "plan" => {
            let ids = crate::core::llm::tool_table::list(&a, "ids");
            if ids.is_empty() {
                return Err("maintenance: ids is empty".into());
            }
            super::tools::blocking(move || {
                let ctx = Ctx::app()?;
                let items = items(current_os(), &ids)?;
                let p = plan::offer(&ctx, PLAN_KIND, items, Source::Ui)?;
                Ok(json!({ "plan_id": p.id, "plan": p.preview() }))
            })
            .await
        }
        _ => Err(format!("maintenance: unknown screen action {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any(_: &str) -> bool {
        true
    }
    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn every_task_has_fixed_absolute_steps() {
        for t in TASKS {
            let steps =
                steps_for(t.os, t.id, "C:\\Windows", &any).unwrap_or_else(|| panic!("{}", t.id));
            assert!(!steps.is_empty());
            for s in &steps {
                let p = &s[0];
                assert!(
                    p.starts_with('/') || p.starts_with("C:\\Windows\\System32\\"),
                    "{p}"
                );
                assert!(
                    !s.iter().any(|a| a.contains("ResetBase")),
                    "never /ResetBase"
                );
            }
            assert!(!t.id.starts_with("win_"), "win_ ids belong to elevate.rs");
        }
        assert_eq!(
            steps_for(Os::Macos, "dns_flush", "", &any).unwrap(),
            vec![
                vec!["/usr/bin/dscacheutil", "-flushcache"],
                vec!["/usr/bin/killall", "-HUP", "mDNSResponder"]
            ]
        );
        assert_eq!(
            steps_for(Os::Windows, "dism_restore_health", "C:\\Windows\\", &any).unwrap(),
            vec![vec![
                "C:\\Windows\\System32\\Dism.exe",
                "/Online",
                "/Cleanup-Image",
                "/RestoreHealth"
            ]]
        );
        assert_eq!(
            steps_for(Os::Linux, "fstrim", "", &|p| p == "/sbin/fstrim").unwrap(),
            vec![vec!["/sbin/fstrim", "-av"]]
        );
        assert!(
            steps_for(Os::Linux, "fstrim", "", &none).is_none(),
            "no program, no task"
        );
    }

    #[test]
    fn elevated_table_is_closed() {
        assert!(elevated_steps(Os::Macos, "rm_rf", "", &any)
            .unwrap_err()
            .contains("unknown task"));
        assert!(elevated_steps(Os::Macos, "dns_flush; rm -rf /", "", &any).is_err());
        assert!(elevated_steps(Os::Macos, "quicklook_reset", "", &any)
            .unwrap_err()
            .contains("does not run as administrator"));
        assert!(
            elevated_steps(Os::Macos, "sfc_scan", "", &any).is_err(),
            "a Windows id on macOS"
        );
        assert_eq!(
            elevated_steps(Os::Macos, "spotlight_reindex", "", &any).unwrap(),
            vec![vec!["/usr/bin/mdutil", "-E", "/"]]
        );
        assert_eq!(
            elevated_steps(Os::Windows, "sfc_scan", "D:\\Win", &any).unwrap(),
            vec![vec!["D:\\Win\\System32\\sfc.exe", "/scannow"]]
        );
        // The real entry point refuses before running anything.
        assert!(elevated_run("nope").is_err());
        assert!(elevated_run("").is_err());
    }

    #[test]
    fn plans_are_module_items() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(dir.path());
        let ids = vec![
            "dns_flush".to_string(),
            "quicklook_reset".to_string(),
            "disk_verify".to_string(),
        ];
        let items = items(Os::Macos, &ids).unwrap();
        assert_eq!(items.len(), 3);
        assert!(items[0].admin && !items[1].admin);
        assert_eq!(items[0].id, "maintenance:dns_flush");
        assert!(items
            .iter()
            .all(|i| i.detail.as_deref().unwrap().contains("Nothing to undo")));
        match &items[1].op {
            Op::Module { module, action, .. } => assert_eq!(
                (module.as_str(), action.as_str()),
                ("maintenance", "quicklook_reset")
            ),
            other => panic!("{other:?}"),
        }
        let p = plan::offer(&ctx, PLAN_KIND, items, Source::Ui).unwrap();
        assert!(p.needs_admin);
        assert_eq!(p.kind, "maintenance");
        assert!(items_err());
        // The admin half hands the helper nothing but the id.
        if current_os() == Os::Macos {
            let (ops, undo) = plan_elevated(&ctx, "r", "dns_flush", &Value::Null).unwrap();
            assert!(undo.is_none());
            assert!(matches!(&ops[..], [ElevatedOp::Maintenance { id }] if id == "dns_flush"));
            assert!(plan_elevated(&ctx, "r", "quicklook_reset", &Value::Null).is_err());
            assert!(plan_execute(&ctx, "r", "dns_flush", &Value::Null)
                .unwrap_err()
                .contains("administrator"));
        }
        assert!(plan_execute(&ctx, "r", "rm_rf", &Value::Null).is_err());
        assert!(plan_undo(&ctx, "dns_flush", &Value::Null).is_err());
    }

    fn items_err() -> bool {
        items(Os::Macos, &["sfc_scan".to_string()]).is_err()
            && items(Os::Linux, &["bogus".to_string()]).is_err()
    }

    #[test]
    fn command_lines_for_copying() {
        assert_eq!(
            command_line_for(Os::Macos, "dns_flush", "", &any),
            "sudo /usr/bin/dscacheutil -flushcache && sudo /usr/bin/killall -HUP mDNSResponder"
        );
        assert_eq!(
            command_line_for(Os::Macos, "quicklook_reset", "", &any),
            "/usr/bin/qlmanage -r cache"
        );
        assert!(
            command_line_for(Os::Windows, "sfc_scan", "C:\\Windows", &any)
                .contains("administrator terminal")
        );
        assert!(command_line_for(Os::Macos, "nope", "", &any).starts_with('#'));
    }

    #[test]
    fn output_decoding_and_summary() {
        let text = "Beginning system scan.\r\nVerification 100% complete.\r\nWindows Resource Protection did not find any integrity violations.\r\n";
        let utf16: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode_output(&utf16), text);
        assert_eq!(decode_output(b"plain"), "plain");
        let fx = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/washmybelly_fixtures/maintenance/diskutil_verify.txt"),
        )
        .unwrap();
        let s = summarize(&fx);
        assert!(
            s.ends_with("Finished file system verification on disk3s1s1 (Macintosh HD)"),
            "{s}"
        );
        assert!(summarize(&"x".repeat(2000)).chars().count() <= 401);
    }
}
