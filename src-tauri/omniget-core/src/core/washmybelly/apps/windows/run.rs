//! Ported design of Bulk Crap Uninstaller (Copyright (c) 2017 Marcin
//! Szeniak, Apache License 2.0, see `data/washmybelly/bcu/NOTICE`): quiet

use serde_json::Value;

use super::parse::{
    self, detect_kind, program_refused, split_command, win_file_name, win_parent, CmdLine, Kind,
    Probe, RegEntry,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub method: &'static str,
    pub kind: Kind,
    pub program: String,
    pub args: Vec<String>,
    pub raw: Option<String>,
    pub env: Vec<(String, String)>,
    pub silent: bool,
    pub msi_like: bool,
    pub nsis_copy_from: Option<String>,
}

impl Planned {
    pub fn display(&self) -> String {
        let q = |s: &str| {
            if s.is_empty() || s.contains([' ', '\t', '"']) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.to_string()
            }
        };
        let mut parts = vec![q(&self.program)];
        parts.extend(self.args.iter().map(|a| q(a)));
        if let Some(r) = &self.raw {
            if !r.is_empty() {
                parts.push(r.clone());
            }
        }
        let mut s = parts.join(" ");
        for (k, v) in &self.env {
            s = format!("$env:{k}={}; {s}", q(v));
        }
        s
    }
}

pub fn system_root() -> String {
    std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())
}

pub fn msiexec(system_root: &str) -> String {
    format!(r"{system_root}\System32\msiexec.exe")
}

pub fn powershell(system_root: &str) -> String {
    format!(r"{system_root}\System32\WindowsPowerShell\v1.0\powershell.exe")
}

pub fn msi_uninstall(code: &str, system_root: &str) -> Result<Planned, String> {
    if !parse::is_guid(code) {
        return Err(format!("not a product code: {code}"));
    }
    Ok(Planned {
        method: "msi",
        kind: Kind::Msi,
        program: msiexec(system_root),
        args: vec![
            "/qb".into(),
            format!("/X{}", code.to_ascii_uppercase()),
            "REBOOT=ReallySuppress".into(),
            "/norestart".into(),
        ],
        raw: None,
        env: vec![],
        silent: true,
        msi_like: true,
        nsis_copy_from: None,
    })
}

pub fn winget_uninstall(id: &str) -> Result<Planned, String> {
    if !parse::winget_id_ok(id) {
        return Err(format!("not a winget id: {id}"));
    }
    Ok(Planned {
        method: "winget",
        kind: Kind::Other,
        program: "winget".into(),
        args: [
            "uninstall",
            "--id",
            id,
            "--exact",
            "--silent",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        raw: None,
        env: vec![],
        silent: true,
        msi_like: true,
        nsis_copy_from: None,
    })
}

fn checked(cmd: &CmdLine, probe: &dyn Probe) -> Result<(), String> {
    if let Some(why) = program_refused(&cmd.program) {
        return Err(why.into());
    }
    if !cmd.program.contains(['\\', '/']) || !probe.exists(&cmd.program) {
        return Err(format!("the uninstaller {} is not there", cmd.program));
    }
    Ok(())
}

pub fn for_entry(e: &RegEntry, probe: &dyn Probe, system_root: &str) -> Result<Planned, String> {
    let kind = detect_kind(e, probe);
    let us = e.get_prefixed("UninstallString");
    let quiet = e.get("QuietUninstallString");
    if kind == Kind::Msi {
        let code = e.product_code().ok_or("no product code in the entry")?;
        return msi_uninstall(&code, system_root);
    }
    let base = |method, cmd: &CmdLine, silent: bool| Planned {
        method,
        kind,
        program: cmd.program.clone(),
        args: vec![],
        raw: Some(cmd.raw_args.clone()).filter(|r| !r.is_empty()),
        env: vec![],
        silent,
        msi_like: kind.msi_like(),
        nsis_copy_from: None,
    };
    let exists = |p: &str| probe.exists(p);
    match kind {
        Kind::Inno => {
            let cmd = split_command(us.or(quiet).ok_or("no uninstall command")?, &exists)
                .ok_or("unreadable uninstall command")?;
            checked(&cmd, probe)?;
            if !parse::is_inno_uninstaller(win_file_name(&cmd.program)) {
                return Ok(base("loud", &cmd, false));
            }
            Ok(Planned {
                args: vec![
                    "/VERYSILENT".into(),
                    "/SUPPRESSMSGBOXES".into(),
                    "/NORESTART".into(),
                ],
                raw: None,
                ..base("inno", &cmd, true)
            })
        }
        Kind::Nsis => {
            let cmd = split_command(us.ok_or("no uninstall command")?, &exists)
                .ok_or("unreadable uninstall command")?;
            checked(&cmd, probe)?;
            let file = win_file_name(&cmd.program).to_ascii_lowercase();
            if file.contains("uninst") && cmd.args.is_empty() {
                let dir = win_parent(&cmd.program).to_string();
                return Ok(Planned {
                    args: vec!["/S".into()],
                    raw: Some(format!("_?={dir}")),
                    nsis_copy_from: Some(cmd.program.clone()),
                    ..base("nsis", &cmd, true)
                });
            }
            let raw = format!("{} /S", cmd.raw_args).trim().to_string();
            Ok(Planned {
                raw: Some(raw),
                ..base("nsis", &cmd, true)
            })
        }
        _ => {
            if let Some(q) = quiet {
                if let Some(cmd) = split_command(q, &exists) {
                    if checked(&cmd, probe).is_ok() {
                        return Ok(base("quiet", &cmd, true));
                    }
                }
            }
            let cmd = split_command(us.ok_or("no uninstall command")?, &exists)
                .ok_or("unreadable uninstall command")?;
            let msiexec_like = win_file_name(&cmd.program)
                .to_ascii_lowercase()
                .trim_end_matches(".exe")
                == "msiexec";
            if msiexec_like {
                let code = parse::guid_in(&cmd.raw_args).ok_or("msiexec without a product code")?;
                return msi_uninstall(&code, system_root);
            }
            checked(&cmd, probe)?;
            Ok(base("loud", &cmd, false))
        }
    }
}

pub fn steam_uninstall(steam_exe: &str, appid: &str) -> Result<Planned, String> {
    if appid.is_empty() || !appid.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("not a Steam app id: {appid}"));
    }
    if !steam_exe.to_ascii_lowercase().ends_with("steam.exe") {
        return Err("Steam was not found".into());
    }
    Ok(Planned {
        method: "steam",
        kind: Kind::Other,
        program: steam_exe.to_string(),
        args: vec![format!("steam://uninstall/{appid}")],
        raw: None,
        env: vec![],
        silent: false,
        msi_like: false,
        nsis_copy_from: None,
    })
}

pub const SCOOP_SCRIPT: &str = "$ErrorActionPreference='Stop'; if ($env:WMB_SCOOP_GLOBAL -eq '1') { & scoop uninstall $env:WMB_SCOOP_APP --global } else { & scoop uninstall $env:WMB_SCOOP_APP }; exit $LASTEXITCODE";

pub fn scoop_uninstall(name: &str, global: bool, system_root: &str) -> Result<Planned, String> {
    if !parse::scoop_name_ok(name) {
        return Err(format!("not a Scoop app: {name}"));
    }
    Ok(Planned {
        method: "scoop",
        kind: Kind::Other,
        program: powershell(system_root),
        args: [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SCOOP_SCRIPT,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        raw: None,
        env: vec![
            ("WMB_SCOOP_APP".into(), name.into()),
            (
                "WMB_SCOOP_GLOBAL".into(),
                if global { "1" } else { "0" }.into(),
            ),
        ],
        silent: true,
        msi_like: false,
        nsis_copy_from: None,
    })
}

pub const APPX_REMOVE_SCRIPT: &str = "$ErrorActionPreference='Stop'; $p = Get-AppxPackage -Name $env:WMB_APPX; if (-not $p) { exit 3 }; $p | Remove-AppxPackage; if (Get-AppxPackage -Name $env:WMB_APPX) { exit 4 }; exit 0";

pub fn appx_remove(name: &str, system_root: &str) -> Result<Planned, String> {
    if !parse::appx_name_ok(name) {
        return Err(format!("not an Appx package name: {name}"));
    }
    Ok(Planned {
        method: "appx",
        kind: Kind::Other,
        program: powershell(system_root),
        args: [
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            APPX_REMOVE_SCRIPT,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        raw: None,
        env: vec![("WMB_APPX".into(), name.into())],
        silent: true,
        msi_like: false,
        nsis_copy_from: None,
    })
}

pub fn store_install(store_id: &str) -> Result<Planned, String> {
    if !parse::store_id_ok(store_id) {
        return Err(format!("not a Store id: {store_id}"));
    }
    Ok(Planned {
        method: "winget",
        kind: Kind::Other,
        program: "winget".into(),
        args: [
            "install",
            "--id",
            store_id,
            "--source",
            "msstore",
            "--exact",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        raw: None,
        env: vec![],
        silent: true,
        msi_like: false,
        nsis_copy_from: None,
    })
}

pub fn reg_key_ok(key: &str) -> Result<(), String> {
    let k = key.trim();
    if k.contains(['"', '\0', '\n', '/']) || k.ends_with('\\') || k.contains(r"\\") {
        return Err(format!("PROTECTED: {k} is not a plain key"));
    }
    let parts: Vec<&str> = k.split('\\').collect();
    let hive = parts
        .first()
        .map(|h| h.to_ascii_uppercase())
        .unwrap_or_default();
    if hive != "HKCU" && hive != "HKLM" {
        return Err(format!("PROTECTED: {k} is outside HKCU and HKLM"));
    }
    if parts.len() < 3 || !parts[1].eq_ignore_ascii_case("software") {
        return Err(format!("PROTECTED: {k} is not under Software"));
    }
    let mut rest: Vec<&str> = parts[2..].to_vec();
    if rest
        .first()
        .is_some_and(|p| p.eq_ignore_ascii_case("wow6432node"))
    {
        rest.remove(0);
    }
    let lower: Vec<String> = rest.iter().map(|p| p.to_ascii_lowercase()).collect();
    let un = ["microsoft", "windows", "currentversion", "uninstall"];
    if lower.len() == 5 && lower[..4] == un {
        return Ok(());
    }
    if lower.is_empty() || super::leftovers::KEY_BLACKLIST.contains(&lower[0].as_str()) {
        return Err(format!("PROTECTED: {k} is a shared key"));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct IdleTracker {
    limit: u32,
    idle: u32,
}

impl IdleTracker {
    pub fn new(limit: u32) -> IdleTracker {
        IdleTracker { limit, idle: 0 }
    }

    pub fn feed(&mut self, cpu: f32, io_bytes: u64) -> bool {
        if cpu <= 1.0 && io_bytes <= 10 * 1024 {
            self.idle += 1;
        } else {
            self.idle = 0;
        }
        self.idle >= self.limit
    }
}

pub fn summary(meaning: &parse::ExitMeaning, name: &str) -> String {
    format!("{name}: {}", meaning.text)
}

pub fn arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

#[cfg(windows)]
pub mod sys {
    use std::process::{Child, Command, Output, Stdio};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    use super::{IdleTracker, Planned};

    static MSI_LOCK: Mutex<()> = Mutex::new(());
    static LOUD_LOCK: Mutex<()> = Mutex::new(());

    const STALL_SECS: u32 = 60;
    const MSI_STALL_SECS: u32 = 180;
    const SILENT_LIMIT: Duration = Duration::from_secs(30 * 60);
    const LOUD_LIMIT: Duration = Duration::from_secs(60 * 60);

    fn wait_windows_installer(max: Duration) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::OpenMutexW;
        let name: Vec<u16> = r"Global\_MSIExecute"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let start = Instant::now();
        while start.elapsed() < max {
            let h = unsafe { OpenMutexW(0x0010_0000, 0, name.as_ptr()) };
            if h.is_null() {
                return;
            }
            unsafe {
                CloseHandle(h);
            }
            crate::core::washmybelly::progress(
                "uninstall",
                "wait",
                0,
                0,
                "waiting for another installation to finish",
            );
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    fn tree(sys: &System, root: Pid, with_msiexec: bool) -> Vec<Pid> {
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            let parent = out[i];
            for (pid, p) in sys.processes() {
                if p.parent() == Some(parent) && !out.contains(pid) {
                    out.push(*pid);
                }
            }
            i += 1;
        }
        if with_msiexec {
            for (pid, p) in sys.processes() {
                if p.name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("msiexec.exe")
                    && !out.contains(pid)
                {
                    out.push(*pid);
                }
            }
        }
        out
    }

    pub enum Ended {
        Exit(i32),
        Stalled,
        TimedOut,
    }

    fn watch(child: &mut Child, p: &Planned) -> Result<Ended, String> {
        let root = Pid::from_u32(child.id());
        let mut sys = System::new();
        let kind = ProcessRefreshKind::nothing().with_cpu().with_disk_usage();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let mut idle = IdleTracker::new(if p.msi_like {
            MSI_STALL_SECS
        } else {
            STALL_SECS
        });
        let limit = if p.silent { SILENT_LIMIT } else { LOUD_LIMIT };
        let start = Instant::now();
        loop {
            if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
                return Ok(Ended::Exit(st.code().unwrap_or(-1)));
            }
            std::thread::sleep(Duration::from_secs(1));
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
            let pids = tree(&sys, root, p.msi_like);
            let (mut cpu, mut io) = (0f32, 0u64);
            for pid in &pids {
                if let Some(pr) = sys.process(*pid) {
                    cpu += pr.cpu_usage();
                    let d = pr.disk_usage();
                    io += d.read_bytes + d.written_bytes;
                }
            }
            if p.silent && idle.feed(cpu, io) {
                if !p.msi_like {
                    for pid in tree(&sys, root, false).iter().rev() {
                        if let Some(pr) = sys.process(*pid) {
                            pr.kill();
                        }
                    }
                    let _ = child.kill();
                }
                return Ok(Ended::Stalled);
            }
            if start.elapsed() > limit {
                if p.silent && !p.msi_like {
                    let _ = child.kill();
                }
                return Ok(Ended::TimedOut);
            }
        }
    }

    pub fn run(p: &Planned, label: &str) -> Result<Ended, String> {
        let _msi = if p.msi_like {
            wait_windows_installer(Duration::from_secs(600));
            Some(MSI_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
        } else {
            None
        };
        let _loud = if !p.silent {
            Some(LOUD_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
        } else {
            None
        };
        let mut temp: Option<std::path::PathBuf> = None;
        let program = match &p.nsis_copy_from {
            Some(src) => {
                let dir = std::env::temp_dir()
                    .join(format!("omniget-wmb-{}", uuid::Uuid::new_v4().simple()));
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let dst = dir.join(super::win_file_name(src));
                std::fs::copy(src, &dst).map_err(|e| format!("copy the uninstaller: {e}"))?;
                temp = Some(dir);
                dst.to_string_lossy().to_string()
            }
            None => p.program.clone(),
        };
        let mut cmd = if program == "winget" {
            crate::core::process::std_command("winget")
        } else {
            Command::new(&program)
        };
        cmd.args(&p.args);
        {
            use std::os::windows::process::CommandExt;
            if let Some(raw) = &p.raw {
                cmd.raw_arg(raw);
            }
            if p.silent {
                cmd.creation_flags(0x0800_0000);
            }
        }
        for (k, v) in &p.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::core::washmybelly::progress(
            "uninstall",
            "run",
            0,
            0,
            format!("running the uninstaller of {label}"),
        );
        let spawned = cmd.spawn();
        let result = match spawned {
            Ok(mut child) => watch(&mut child, p),
            Err(e) if e.raw_os_error() == Some(740) => {
                Err("the uninstaller needs administrator rights".into())
            }
            Err(e) => Err(format!("{}: {e}", p.program)),
        };
        if let Some(dir) = temp {
            for _ in 0..10 {
                if std::fs::remove_dir_all(&dir).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        result
    }

    pub fn output_with_timeout(mut cmd: Command, secs: u64) -> Result<Output, String> {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        let t_out = std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(o) = out.as_mut() {
                let _ = std::io::Read::read_to_end(o, &mut b);
            }
            b
        });
        let t_err = std::thread::spawn(move || {
            let mut b = Vec::new();
            if let Some(o) = err.as_mut() {
                let _ = std::io::Read::read_to_end(o, &mut b);
            }
            b
        });
        let start = Instant::now();
        let status = loop {
            if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
                break st;
            }
            if start.elapsed() > Duration::from_secs(secs) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        Ok(Output {
            status,
            stdout: t_out.join().unwrap_or_default(),
            stderr: t_err.join().unwrap_or_default(),
        })
    }
}
