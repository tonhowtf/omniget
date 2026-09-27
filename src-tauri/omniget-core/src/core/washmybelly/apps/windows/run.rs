//! How an uninstaller is started and watched.
//!
//! Ported design of Bulk Crap Uninstaller (Copyright (c) 2017 Marcin
//! Szeniak, Apache License 2.0, see `data/washmybelly/bcu/NOTICE`): quiet
//! strings per installer kind (`MsiUninstallStringGenerator.cs`,
//! `InnoSetupQuietUninstallStringGenerator.cs`), the NSIS `_?=` trick to
//! keep the exit code (`UninstallManager.cs`), one Windows Installer job and
//! one windowed uninstaller at a time, and the stall check by CPU and disk
//! (`BulkUninstallTask.cs`, `BulkUninstallEntry.cs`). Rewritten in Rust and
//! modified for OmniGet: no UI automation (NSIS gets `/S`), no `cmd /C`,
//! never a shell; the program must exist and be an `.exe`.

use serde_json::Value;

use super::parse::{
    self, detect_kind, program_refused, split_command, win_file_name, win_parent, CmdLine, Kind,
    Probe, RegEntry,
};

/// What will run for one uninstall, built from the registry entry (never
/// from the caller's words).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// `winget`, `msi`, `inno`, `nsis`, `quiet`, `loud`, `steam`, `scoop`, `appx`.
    pub method: &'static str,
    pub kind: Kind,
    pub program: String,
    /// Arguments quoted by Rust one by one.
    pub args: Vec<String>,
    /// Vendor text appended as it is (`raw_arg`), after `args`.
    pub raw: Option<String>,
    pub env: Vec<(String, String)>,
    /// Runs without a window the person has to answer.
    pub silent: bool,
    /// Goes through the Windows Installer: one at a time, system-wide.
    pub msi_like: bool,
    /// NSIS: run a temporary copy of this file so `_?=` can wait for it.
    pub nsis_copy_from: Option<String>,
}

impl Planned {
    /// The command as a person would type it (display and "copy command").
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

/// `msiexec /qb /X{GUID} REBOOT=ReallySuppress /norestart` (BCU; `/qb` shows
/// a progress bar and no questions).
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

/// `winget uninstall --id <id> --exact --silent ...`.
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

/// The uninstall command for a registry entry: silent when the installer
/// kind allows it, the vendor's own command (with its window) otherwise.
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
                // `_?=` must be last and unquoted: NSIS reads the rest of the line.
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
                // A Windows Installer entry without WindowsInstaller=1.
                let code = parse::guid_in(&cmd.raw_args).ok_or("msiexec without a product code")?;
                return msi_uninstall(&code, system_root);
            }
            checked(&cmd, probe)?;
            Ok(base("loud", &cmd, false))
        }
    }
}

/// Steam asks the person itself (`steam://uninstall/<id>`).
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

/// Fixed script: the app name only ever travels in an environment variable.
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

/// Fixed script: removes the package for the current user; the name comes
/// from an environment variable and is never part of the script text.
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

/// Reinstall from the Store (the undo of an Appx removal).
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

/// A registry key the uninstaller may remove: under `Software` with at
/// least a publisher and a product part, or an Uninstall entry. Never the
/// shared roots (`Microsoft`, `Classes`, `Policies`...).
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
    // Uninstall entries: exactly one level below the Uninstall key.
    let un = ["microsoft", "windows", "currentversion", "uninstall"];
    if lower.len() == 5 && lower[..4] == un {
        return Ok(());
    }
    if lower.is_empty() || super::leftovers::KEY_BLACKLIST.contains(&lower[0].as_str()) {
        return Err(format!("PROTECTED: {k} is a shared key"));
    }
    Ok(())
}

/// Stall tracker: a silent uninstaller that shows no CPU and no disk
/// activity for `limit` checks in a row is stuck (BCU: CPU ≤ 1 %, I/O ≤ 10 KB/s).
#[derive(Debug, Clone)]
pub struct IdleTracker {
    limit: u32,
    idle: u32,
}

impl IdleTracker {
    pub fn new(limit: u32) -> IdleTracker {
        IdleTracker { limit, idle: 0 }
    }

    /// One sample (summed CPU percent, bytes read and written since the
    /// last sample, one second apart). True when stuck.
    pub fn feed(&mut self, cpu: f32, io_bytes: u64) -> bool {
        if cpu <= 1.0 && io_bytes <= 10 * 1024 {
            self.idle += 1;
        } else {
            self.idle = 0;
        }
        self.idle >= self.limit
    }
}

/// Plain-English summary for the receipt.
pub fn summary(meaning: &parse::ExitMeaning, name: &str) -> String {
    format!("{name}: {}", meaning.text)
}

/// Arguments of a module item, read with defaults.
pub fn arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

// ── Windows: starting and watching ──────────────────────────────────

#[cfg(windows)]
pub mod sys {
    use std::process::{Child, Command, Output, Stdio};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    use super::{IdleTracker, Planned};

    static MSI_LOCK: Mutex<()> = Mutex::new(());
    static LOUD_LOCK: Mutex<()> = Mutex::new(());

    /// Silent uninstaller stuck this many seconds: stopped.
    const STALL_SECS: u32 = 60;
    /// Windows Installer jobs are never killed; this long idle is reported.
    const MSI_STALL_SECS: u32 = 180;
    const SILENT_LIMIT: Duration = Duration::from_secs(30 * 60);
    const LOUD_LIMIT: Duration = Duration::from_secs(60 * 60);

    /// Wait while another Windows Installer job holds `_MSIExecute`.
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

    /// How a run ended.
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
            // A window waiting for the person is idle on purpose.
            if p.silent && idle.feed(cpu, io) {
                if !p.msi_like {
                    // Only the processes we started, never msiexec's service.
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

    /// Run a planned uninstall with the queue rules and the stall check.
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
                // No console window flashing for silent runs.
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

    /// Run a short command and collect its output, killing it after `secs`
    /// (winget can hang on a source prompt the first time).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::washmybelly::apps::windows::parse::Root;

    struct Disk(Vec<&'static str>, bool);
    impl Probe for Disk {
        fn exists(&self, p: &str) -> bool {
            self.0.iter().any(|x| x.eq_ignore_ascii_case(p))
        }
        fn nullsoft(&self, _: &str) -> bool {
            self.1
        }
    }

    fn entry(pairs: &[(&str, &str)]) -> RegEntry {
        RegEntry {
            root: Root::Hklm,
            key_name: "K".into(),
            values: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn msi_is_rebuilt_from_the_product_code() {
        let e = entry(&[
            ("WindowsInstaller", "0x1"),
            (
                "UninstallString",
                "MsiExec.exe /I{AC76BA86-7AD7-1033-7B44-AC0F074E4100}",
            ),
        ]);
        let p = for_entry(&e, &Disk(vec![], false), r"C:\Windows").unwrap();
        assert_eq!(p.program, r"C:\Windows\System32\msiexec.exe");
        assert_eq!(
            p.args,
            vec![
                "/qb",
                "/X{AC76BA86-7AD7-1033-7B44-AC0F074E4100}",
                "REBOOT=ReallySuppress",
                "/norestart"
            ]
        );
        assert!(p.silent && p.msi_like);
        assert!(msi_uninstall("{bad}", r"C:\Windows").is_err());
    }

    #[test]
    fn inno_gets_its_silent_switches() {
        let e = entry(&[
            ("Inno Setup: App Path", r"C:\P\Foo"),
            ("UninstallString", r#""C:\P\Foo\unins000.exe""#),
        ]);
        let p = for_entry(
            &e,
            &Disk(
                vec![r"C:\P\Foo\unins000.exe", r"C:\P\Foo\unins000.dat"],
                false,
            ),
            r"C:\Windows",
        )
        .unwrap();
        assert_eq!(p.method, "inno");
        assert_eq!(
            p.args,
            vec!["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]
        );
        assert_eq!(p.raw, None);
        assert!(!p.msi_like);
    }

    #[test]
    fn nsis_runs_a_copy_with_the_folder_last() {
        let e = entry(&[("UninstallString", r"C:\Program Files\Foo\uninst.exe")]);
        let p = for_entry(
            &e,
            &Disk(vec![r"C:\Program Files\Foo\uninst.exe"], true),
            r"C:\Windows",
        )
        .unwrap();
        assert_eq!(p.method, "nsis");
        assert_eq!(p.args, vec!["/S"]);
        assert_eq!(p.raw.as_deref(), Some(r"_?=C:\Program Files\Foo"));
        assert_eq!(
            p.nsis_copy_from.as_deref(),
            Some(r"C:\Program Files\Foo\uninst.exe")
        );
        assert!(p.display().ends_with(r#"/S _?=C:\Program Files\Foo"#));
    }

    #[test]
    fn other_kinds_prefer_quiet_then_loud_and_refuse_shells() {
        let disk = Disk(vec![r"C:\P\x\setup.exe", r"C:\P\x\q.exe"], false);
        let e = entry(&[
            ("UninstallString", r#""C:\P\x\setup.exe" --uninstall"#),
            ("QuietUninstallString", r#""C:\P\x\q.exe" /quiet"#),
        ]);
        let p = for_entry(&e, &disk, r"C:\Windows").unwrap();
        assert_eq!(
            (p.method, p.program.as_str(), p.raw.as_deref()),
            ("quiet", r"C:\P\x\q.exe", Some("/quiet"))
        );
        let e = entry(&[(
            "UninstallString",
            r#""C:\P\x\setup.exe" --uninstall --x="a b""#,
        )]);
        let p = for_entry(&e, &disk, r"C:\Windows").unwrap();
        assert_eq!(p.method, "loud");
        assert!(!p.silent);
        assert_eq!(
            p.raw.as_deref(),
            Some(r#"--uninstall --x="a b""#),
            "vendor quoting kept"
        );
        let e = entry(&[("UninstallString", r#"cmd.exe /c del C:\x"#)]);
        assert!(for_entry(&e, &disk, r"C:\Windows").is_err());
        let e = entry(&[("UninstallString", r#""C:\P\missing\u.exe""#)]);
        assert!(for_entry(&e, &disk, r"C:\Windows")
            .unwrap_err()
            .contains("not there"));
        let e = entry(&[("UninstallString", r#"C:\P\x\remove.bat"#)]);
        assert!(for_entry(&e, &disk, r"C:\Windows").is_err());
    }

    #[test]
    fn winget_scoop_appx_store_argv() {
        let w = winget_uninstall("Mozilla.Firefox").unwrap();
        assert_eq!(
            w.args[..4],
            ["uninstall", "--id", "Mozilla.Firefox", "--exact"]
        );
        assert!(w.args.contains(&"--disable-interactivity".to_string()));
        assert!(winget_uninstall("--all").is_err());
        let s = scoop_uninstall("git", false, r"C:\Windows").unwrap();
        assert!(
            !s.args.iter().any(|a| a.contains("git")),
            "name only in the environment"
        );
        assert_eq!(s.env[0], ("WMB_SCOOP_APP".to_string(), "git".to_string()));
        let a = appx_remove("Microsoft.BingNews", r"C:\Windows").unwrap();
        assert!(!a.args.iter().any(|x| x.contains("BingNews")));
        assert!(appx_remove("x'; Remove-Item C:\\", r"C:\Windows").is_err());
        let r = store_install("9WZDNCRFHVFW").unwrap();
        assert!(r.args.contains(&"msstore".to_string()));
        assert!(steam_uninstall(r"C:\Steam\steam.exe", "730").is_ok());
        assert!(steam_uninstall(r"C:\Steam\steam.exe", "7 30").is_err());
    }

    #[test]
    fn registry_keys_allowed_for_removal() {
        assert!(reg_key_ok(r"HKCU\Software\Foo Software\Foo Tool").is_ok());
        assert!(reg_key_ok(r"HKLM\SOFTWARE\WOW6432Node\Foo").is_ok());
        assert!(reg_key_ok(
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Foo Tool_is1"
        )
        .is_ok());
        assert!(reg_key_ok(r"HKLM\SOFTWARE\Microsoft").is_err());
        assert!(reg_key_ok(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall").is_err());
        assert!(reg_key_ok(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run").is_err());
        assert!(reg_key_ok(r"HKCU\Software\Classes\foo").is_err());
        assert!(reg_key_ok(r"HKLM\SYSTEM\CurrentControlSet\Services\x").is_err());
        assert!(reg_key_ok(r"HKCU\Software").is_err());
        assert!(reg_key_ok(r"HKU\S-1-5\Software\Foo").is_err());
        assert!(reg_key_ok(r"HKCU\Software\Foo\").is_err());
    }

    #[test]
    fn idle_tracker_needs_consecutive_quiet_seconds() {
        let mut t = IdleTracker::new(3);
        assert!(!t.feed(0.0, 0));
        assert!(!t.feed(0.5, 100));
        assert!(!t.feed(20.0, 0), "activity resets");
        assert!(!t.feed(0.0, 0));
        assert!(!t.feed(0.0, 50_000), "disk counts");
        assert!(!t.feed(0.0, 0));
        assert!(!t.feed(0.0, 0));
        assert!(t.feed(0.0, 0));
    }
}
