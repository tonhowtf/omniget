use serde::Serialize;
use serde_json::{json, Value};

use super::elevate::ElevatedOp;
use super::plan::{self, Op, Outcome, PlanItem, Source};
use super::receipt::Undo;
use super::store::Ctx;
use super::{current_os, Os, Risk};

pub const MODULE: &str = "maintenance";
pub const PLAN_KIND: &str = "maintenance";

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Task {
    pub id: &'static str,
    #[serde(skip)]
    pub os: Os,
    pub admin: bool,
    pub minutes: u32,
    pub long: bool,
    pub group: &'static str,
    pub title: &'static str,
    pub what: &'static str,
}

const LSREGISTER: &str =
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

pub const TASKS: &[Task] = &[
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

pub fn task(os: Os, id: &str) -> Option<&'static Task> {
    TASKS.iter().find(|t| t.os == os && t.id == id)
}

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
        (Os::Macos, "launchservices_rebuild") => vec![
            v(&[LSREGISTER, "-gc"]),
            v(&[LSREGISTER, "-r", "-f", "-all", "local,system,user"]),
        ],
        (Os::Macos, "spotlight_reindex") => vec![v(&["/usr/bin/mdutil", "-E", "/"])],
        (Os::Macos, "quicklook_reset") => vec![v(&["/usr/bin/qlmanage", "-r", "cache"])],
        (Os::Macos, "disk_verify") => vec![v(&["/usr/sbin/diskutil", "verifyVolume", "/"])],
        (Os::Windows, "dns_flush") => vec![vec![sys32("ipconfig.exe"), "/flushdns".into()]],
        (Os::Windows, "sfc_scan") => vec![vec![sys32("sfc.exe"), "/scannow".into()]],
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

fn steps(id: &str) -> Option<Vec<Vec<String>>> {
    steps_for(current_os(), id, &sysroot(), &file_exists)
}

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

pub fn elevated_run(id: &str) -> Result<String, String> {
    elevated_steps(current_os(), id, &sysroot(), &file_exists)
        .and_then(|steps| run_steps(id, &steps))
}

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
