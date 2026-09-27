//! Administrator work, through the small `omniget-elevate` helper.
//!
//! The app never runs as admin. When a plan has admin items, their typed
//! operations are written to a file, signed with an HMAC key only this user
//! can read, and the helper is started elevated (macOS `osascript ... with
//! administrator privileges`, Windows UAC `runas`, Linux `pkexec`). The
//! helper accepts nothing but this closed enum: no command, no script, no
//! free path. It checks the signature, the expiry, the protected paths and
//! the fingerprints again before touching anything.
//!
//! Without elevation the screen offers [`copy_command`]: the same work as
//! commands the person can read and run in a terminal.

use std::path::{Path, PathBuf};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::fsops::Fingerprint;
use super::protect::Guard;
use super::store::{now, Ctx};

/// Every operation the helper knows. Adding one means a new variant here and
/// its executor in [`execute_local`]; nothing else reaches root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ElevatedOp {
    RemovePath {
        path: PathBuf,
        #[serde(default)]
        root: Option<PathBuf>,
        guard: Guard,
        fp: Fingerprint,
        #[serde(default)]
        contents_only: bool,
    },
    /// Windows registry (HKLM and other hives the user cannot write).
    RegSet {
        key: String,
        name: String,
        /// `REG_DWORD`, `REG_QWORD`, `REG_SZ`, `REG_EXPAND_SZ`, `REG_MULTI_SZ`, `REG_BINARY`.
        kind: String,
        data: String,
    },
    RegDelete {
        key: String,
        #[serde(default)]
        name: Option<String>,
    },
    /// Windows service or Linux systemd unit start type.
    ServiceStartup {
        name: String,
        /// `automatic`, `manual`, `disabled` (Windows); `enable`, `disable`, `mask` (systemd).
        mode: String,
    },
    /// Windows scheduled task state.
    TaskState { path: String, enabled: bool },
    /// Remove packages through the system manager (apt, dnf, pacman, zypper,
    /// snap, flatpak --system, winget machine scope).
    PackageRemove { manager: String, names: Vec<String> },
    /// Upgrade packages through the system manager (apt, dnf, pacman whole
    /// system, zypper, snap, flatpak --system).
    PackageUpgrade { manager: String, names: Vec<String> },
    /// Windows: run an app's own uninstaller as administrator. The helper
    /// rebuilds the command from the registry entry named by `app_id` and
    /// refuses when `program`/`args`/`kind` differ.
    WinUninstall {
        app_id: String,
        kind: String,
        program: String,
        args: Vec<String>,
    },
    /// macOS: forget a `.pkg` receipt after its files are gone.
    PkgForget { id: String },
    /// macOS: stop a LaunchDaemon before its plist leaves.
    LaunchctlBootout { domain: String, label: String },
    /// macOS: `launchctl disable|enable <domain>/<label>` (survives restarts).
    LaunchctlState {
        domain: String,
        label: String,
        enabled: bool,
    },
    /// macOS: thin local Time Machine snapshots.
    TmutilThin { bytes: u64 },
    /// Linux: `journalctl --vacuum-size`.
    JournalVacuum { max_mb: u64 },
    /// Replace the hosts file with the content of a file we staged.
    WriteHosts { staged: PathBuf },
    /// Windows power setting, macOS `pmset` key, Linux sysfs knob.
    PowerSetting { key: String, value: String },
    /// macOS/Linux maintenance with a fixed argv chosen by the module
    /// (`id` picks it from the module's own table; the helper re-derives
    /// the argv from the id and never trusts one from the plan).
    Maintenance { id: String },
    /// Linux package-cache and snap-revision cleanup; the helper rebuilds
    /// the argv from `id` with `clean::native_argv` (closed table).
    CleanNative { id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpResult {
    pub ok: bool,
    pub detail: String,
    #[serde(default)]
    pub bytes: u64,
}

/// What the app writes and the helper reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedBatch {
    pub ops: Vec<ElevatedOp>,
    pub expires: i64,
    pub nonce: String,
    /// Hex HMAC-SHA256 of `ops|expires|nonce` with the user's key.
    pub mac: String,
}

const KEY_FILE: &str = "elevate.key";

fn key_path(dir: &Path) -> PathBuf {
    dir.join(KEY_FILE)
}

/// The HMAC key, created 0600 on first use.
pub fn key(dir: &Path) -> Result<Vec<u8>, String> {
    let path = key_path(dir);
    if let Ok(k) = std::fs::read(&path) {
        if k.len() >= 32 {
            return Ok(k);
        }
    }
    let mut k = vec![0u8; 32];
    for chunk in k.chunks_mut(16) {
        chunk.copy_from_slice(&uuid::Uuid::new_v4().into_bytes()[..chunk.len()]);
    }
    std::fs::write(&path, &k).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(k)
}

fn mac_of(key: &[u8], ops: &[ElevatedOp], expires: i64, nonce: &str) -> Result<String, String> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).map_err(|e| e.to_string())?;
    m.update(
        serde_json::to_string(ops)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    );
    m.update(format!("|{expires}|{nonce}").as_bytes());
    Ok(hex::encode(m.finalize().into_bytes()))
}

pub fn sign(key: &[u8], ops: Vec<ElevatedOp>) -> Result<SignedBatch, String> {
    let expires = now() + 300;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let mac = mac_of(key, &ops, expires, &nonce)?;
    Ok(SignedBatch {
        ops,
        expires,
        nonce,
        mac,
    })
}

pub fn verify(key: &[u8], b: &SignedBatch) -> Result<(), String> {
    if now() > b.expires {
        return Err("EXPIRED: the plan is too old".into());
    }
    let mut m = Hmac::<Sha256>::new_from_slice(key).map_err(|e| e.to_string())?;
    m.update(
        serde_json::to_string(&b.ops)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    );
    m.update(format!("|{}|{}", b.expires, b.nonce).as_bytes());
    let want = hex::decode(&b.mac).map_err(|_| "BAD_SIGNATURE")?;
    m.verify_slice(&want)
        .map_err(|_| "BAD_SIGNATURE".to_string())
}

/// Path of the helper next to the running binary (bundle or `target/`).
pub fn helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        "omniget-elevate.exe"
    } else {
        "omniget-elevate"
    };
    let p = dir.join(name);
    p.is_file().then_some(p)
}

/// Run a batch as administrator: one prompt for the whole batch. Returns one
/// result per operation, in order.
pub fn run(ctx: &Ctx, ops: &[ElevatedOp]) -> Result<Vec<OpResult>, String> {
    if crate::core::dependencies::is_flatpak() {
        return Err(
            "NO_ELEVATION: administrator actions are not available inside the Flatpak sandbox"
                .into(),
        );
    }
    let helper =
        helper_path().ok_or("NO_HELPER: omniget-elevate is not installed next to OmniGet")?;
    let batch = sign(&key(&ctx.dir)?, ops.to_vec())?;
    let dir = ctx.dir.join("elevate");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let plan_file = dir.join(format!("{}.json", batch.nonce));
    let out_file = dir.join(format!("{}.out.json", batch.nonce));
    std::fs::write(
        &plan_file,
        serde_json::to_vec(&batch).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let status = launch_elevated(&helper, &plan_file, &out_file);
    let _ = std::fs::remove_file(&plan_file);
    status?;
    let text = std::fs::read_to_string(&out_file)
        .map_err(|_| "CANCELLED: the administrator prompt was closed".to_string())?;
    let _ = std::fs::remove_file(&out_file);
    let results: Vec<OpResult> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if results.len() != ops.len() {
        return Err("the helper answered a different number of operations".into());
    }
    Ok(results)
}

#[cfg(target_os = "macos")]
fn launch_elevated(helper: &Path, plan: &Path, out: &Path) -> Result<(), String> {
    // Paths travel as argv of osascript and are quoted by AppleScript's own
    // `quoted form of`; no path is ever spliced into the script text.
    let status = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg("on run argv")
        .arg("-e")
        .arg("do shell script (quoted form of item 1 of argv) & \" --plan \" & (quoted form of item 2 of argv) & \" --out \" & (quoted form of item 3 of argv) with administrator privileges")
        .arg("-e")
        .arg("end run")
        .arg(helper)
        .arg(plan)
        .arg(out)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("CANCELLED: the administrator prompt was closed".into())
    }
}

#[cfg(target_os = "linux")]
fn launch_elevated(helper: &Path, plan: &Path, out: &Path) -> Result<(), String> {
    let status = std::process::Command::new("pkexec")
        .arg(helper)
        .arg("--plan")
        .arg(plan)
        .arg("--out")
        .arg(out)
        .status()
        .map_err(|e| format!("pkexec: {e}"))?;
    match status.code() {
        Some(0) => Ok(()),
        Some(126) | Some(127) => Err("CANCELLED: the administrator prompt was closed".into()),
        _ => Err("the helper failed".into()),
    }
}

#[cfg(windows)]
fn launch_elevated(helper: &Path, plan: &Path, out: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }
    // Our own data paths have no quotes; each is wrapped in quotes as one
    // argument for CommandLineToArgvW.
    let params = format!("--plan \"{}\" --out \"{}\"", plan.display(), out.display());
    if params.matches('"').count() != 4 {
        return Err("unexpected quote in a data path".into());
    }
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(helper.as_os_str());
    let params = wide(std::ffi::OsStr::new(&params));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = 0;
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        return Err("CANCELLED: the UAC prompt was closed".into());
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
    }
    if code == 0 {
        Ok(())
    } else {
        Err("the helper failed".into())
    }
}

/// Run one operation in the current process. Only `omniget-elevate` calls
/// this (as admin); tests call it on temporary folders.
pub fn execute_local(op: &ElevatedOp) -> OpResult {
    let r: Result<(String, u64), String> = match op {
        ElevatedOp::RemovePath {
            path,
            root,
            guard,
            fp,
            contents_only,
        } => (|| {
            super::protect::check(path, *guard, root.as_deref())?;
            super::fsops::unchanged(path, fp)?;
            let freed = super::fsops::remove_tree(path, *contents_only)?;
            Ok(("removed".to_string(), freed))
        })(),
        ElevatedOp::RegSet {
            key,
            name,
            kind,
            data,
        } => super::tweaks::elevated_reg_set(key, name, kind, data).map(|d| (d, 0)),
        ElevatedOp::RegDelete { key, name } => {
            super::tweaks::elevated_reg_delete(key, name.as_deref()).map(|d| (d, 0))
        }
        ElevatedOp::ServiceStartup { name, mode } => {
            super::startup::elevated_service_startup(name, mode).map(|d| (d, 0))
        }
        ElevatedOp::TaskState { path, enabled } => {
            super::startup::elevated_task_state(path, *enabled).map(|d| (d, 0))
        }
        ElevatedOp::PackageRemove { manager, names } => {
            super::packages::elevated_remove(manager, names).map(|d| (d, 0))
        }
        ElevatedOp::PackageUpgrade { manager, names } => {
            super::packages::elevated_upgrade(manager, names).map(|d| (d, 0))
        }
        ElevatedOp::WinUninstall {
            app_id,
            kind,
            program,
            args,
        } => super::apps::windows::elevated_win_uninstall(app_id, kind, program, args)
            .map(|d| (d, 0)),
        ElevatedOp::PkgForget { id } => super::apps::elevated_pkg_forget(id).map(|d| (d, 0)),
        ElevatedOp::LaunchctlBootout { domain, label } => {
            super::startup::elevated_bootout(domain, label).map(|d| (d, 0))
        }
        ElevatedOp::LaunchctlState {
            domain,
            label,
            enabled,
        } => super::startup::elevated_launchctl_state(domain, label, *enabled).map(|d| (d, 0)),
        ElevatedOp::TmutilThin { bytes } => super::clean::elevated_tmutil_thin(*bytes),
        ElevatedOp::JournalVacuum { max_mb } => super::clean::elevated_journal_vacuum(*max_mb),
        ElevatedOp::WriteHosts { staged } => super::hosts::elevated_write(staged).map(|d| (d, 0)),
        ElevatedOp::PowerSetting { key, value } => {
            super::power::elevated_setting(key, value).map(|d| (d, 0))
        }
        ElevatedOp::Maintenance { id } if id.starts_with("win_") => windows_maintenance(id),
        ElevatedOp::Maintenance { id } => super::maintenance::elevated_run(id).map(|d| (d, 0)),
        ElevatedOp::CleanNative { id } => super::clean::elevated_native(id),
    };
    match r {
        Ok((detail, bytes)) => OpResult {
            ok: true,
            detail,
            bytes,
        },
        Err(detail) => OpResult {
            ok: false,
            detail,
            bytes: 0,
        },
    }
}

/// Windows maintenance steps of the clean rules (stop Windows Update before
/// its cache goes, Delivery Optimization's own cleanup). The argv comes from
/// a fixed table under System32; nothing from the plan reaches it.
fn windows_maintenance(id: &str) -> Result<(String, u64), String> {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let steps = super::clean::rules_windows::maintenance_argv(id, &root)
        .ok_or_else(|| format!("unknown maintenance step {id}"))?;
    for argv in steps {
        let (prog, args) = argv.split_first().ok_or("empty step")?;
        let out = std::process::Command::new(prog)
            .args(args)
            .output()
            .map_err(|e| format!("{prog}: {e}"))?;
        // `net stop` of a service that is not running is not a failure.
        if !out.status.success() && !String::from_utf8_lossy(&out.stdout).contains("not started") {
            return Err(format!(
                "{id}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok((format!("{id} done"), 0))
}

/// Entry point of the helper binary: read, verify, run, write results.
pub fn helper_main(plan: &Path, out: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(plan).map_err(|e| e.to_string())?;
    let batch: SignedBatch = serde_json::from_str(&text).map_err(|e| format!("BAD_PLAN: {e}"))?;
    // The key sits next to the plan folder: <data>/washmybelly/elevate.key.
    let data = plan
        .parent()
        .and_then(Path::parent)
        .ok_or("BAD_PLAN: unexpected location")?;
    let k = std::fs::read(key_path(data)).map_err(|_| "NO_KEY")?;
    verify(&k, &batch)?;
    let results: Vec<OpResult> = batch.ops.iter().map(execute_local).collect();
    std::fs::write(
        out,
        serde_json::to_vec(&results).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // The result file must stay readable by the user that asked.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o644));
    }
    Ok(())
}

/// The same work as terminal commands, for people who would rather run them
/// themselves. Display only: nothing here is ever executed by OmniGet.
pub fn copy_command(ops: &[ElevatedOp]) -> Vec<String> {
    let q = |s: &str| {
        if cfg!(windows) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            format!("'{}'", s.replace('\'', "'\\''"))
        }
    };
    let sudo = if cfg!(windows) { "" } else { "sudo " };
    ops.iter()
        .map(|op| match op {
            ElevatedOp::RemovePath { path, contents_only, .. } => {
                let p = path.to_string_lossy();
                if cfg!(windows) {
                    format!("Remove-Item -LiteralPath {} -Recurse -Force", q(&p))
                } else if *contents_only {
                    format!("{sudo}find {} -mindepth 1 -delete", q(&p))
                } else {
                    format!("{sudo}rm -rf -- {}", q(&p))
                }
            }
            ElevatedOp::RegSet { key, name, kind, data } => format!("reg add {} /v {} /t {} /d {} /f", q(key), q(name), kind, q(data)),
            ElevatedOp::RegDelete { key, name: Some(n) } => format!("reg delete {} /v {} /f", q(key), q(n)),
            ElevatedOp::RegDelete { key, name: None } => format!("reg delete {} /f", q(key)),
            ElevatedOp::ServiceStartup { name, mode } if cfg!(windows) => format!("sc.exe config {} start= {}", q(name), match mode.as_str() { "automatic" => "auto", "manual" => "demand", other => other }),
            ElevatedOp::ServiceStartup { name, mode } => format!("{sudo}systemctl {mode} {}", q(name)),
            ElevatedOp::TaskState { path, enabled } => format!("schtasks /Change /TN {} /{}", q(path), if *enabled { "ENABLE" } else { "DISABLE" }),
            ElevatedOp::PackageRemove { manager, names } => format!("{sudo}{} {}", super::packages::remove_command(manager), names.iter().map(|n| q(n)).collect::<Vec<_>>().join(" ")),
            ElevatedOp::PackageUpgrade { manager, .. } if manager == "pacman" => format!("{sudo}{}", super::packages::upgrade_command(manager)),
            ElevatedOp::PackageUpgrade { manager, names } => format!("{sudo}{} {}", super::packages::upgrade_command(manager), names.iter().map(|n| q(n)).collect::<Vec<_>>().join(" ")),
            ElevatedOp::WinUninstall { program, args, .. } => std::iter::once(q(program)).chain(args.iter().map(|a| q(a))).collect::<Vec<_>>().join(" "),
            ElevatedOp::PkgForget { id } => format!("{sudo}pkgutil --forget {}", q(id)),
            ElevatedOp::LaunchctlBootout { domain, label } => format!("{sudo}launchctl bootout {}/{}", domain, q(label)),
            ElevatedOp::LaunchctlState { domain, label, enabled } => format!("{sudo}launchctl {} {}/{}", if *enabled { "enable" } else { "disable" }, domain, q(label)),
            ElevatedOp::TmutilThin { bytes } => format!("{sudo}tmutil thinlocalsnapshots / {bytes} 4"),
            ElevatedOp::JournalVacuum { max_mb } => format!("{sudo}journalctl --vacuum-size={max_mb}M"),
            ElevatedOp::WriteHosts { staged } => {
                if cfg!(windows) {
                    format!("Copy-Item -LiteralPath {} -Destination \"$env:SystemRoot\\System32\\drivers\\etc\\hosts\"", q(&staged.to_string_lossy()))
                } else {
                    format!("{sudo}cp {} /etc/hosts", q(&staged.to_string_lossy()))
                }
            }
            ElevatedOp::PowerSetting { key, value } => format!("# power setting {key} = {value}"),
            ElevatedOp::Maintenance { id } if id.starts_with("win_") => {
                let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
                super::clean::rules_windows::maintenance_argv(id, &root)
                    .map(|steps| steps.iter().map(|a| a.join(" ")).collect::<Vec<_>>().join(" ; "))
                    .unwrap_or_else(|| format!("# {id}"))
            }
            ElevatedOp::Maintenance { id } => super::maintenance::command_line(id),
            ElevatedOp::CleanNative { id } => super::clean::native_command_line(id),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tampered_batch_is_refused() {
        let k = vec![7u8; 32];
        let mut b = sign(&k, vec![ElevatedOp::JournalVacuum { max_mb: 100 }]).unwrap();
        verify(&k, &b).unwrap();
        b.ops = vec![ElevatedOp::JournalVacuum { max_mb: 1 }];
        assert!(verify(&k, &b).is_err());
    }

    #[test]
    fn an_expired_batch_is_refused() {
        let k = vec![7u8; 32];
        let mut b = sign(&k, vec![]).unwrap();
        b.expires = 1;
        b.mac = mac_of(&k, &b.ops, b.expires, &b.nonce).unwrap();
        assert!(verify(&k, &b).unwrap_err().starts_with("EXPIRED"));
    }

    #[test]
    fn remove_path_revalidates_in_the_helper() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let f = root.join("x.tmp");
        std::fs::write(&f, "1").unwrap();
        let fp = super::super::fsops::fingerprint(&f).unwrap();
        let op = ElevatedOp::RemovePath {
            path: f.clone(),
            root: Some(root.clone()),
            guard: Guard::Picked,
            fp,
            contents_only: false,
        };
        std::fs::write(&f, "changed!").unwrap();
        assert!(!execute_local(&op).ok, "a changed file is not removed");
        let fp = super::super::fsops::fingerprint(&f).unwrap();
        let op = ElevatedOp::RemovePath {
            path: f.clone(),
            root: Some(root),
            guard: Guard::Picked,
            fp,
            contents_only: false,
        };
        assert!(execute_local(&op).ok);
        assert!(!f.exists());
    }

    #[test]
    fn copy_command_quotes_paths() {
        let cmds = copy_command(&[ElevatedOp::RemovePath {
            path: PathBuf::from("/tmp/a b'c"),
            root: None,
            guard: Guard::Picked,
            fp: Fingerprint::default(),
            contents_only: false,
        }]);
        if !cfg!(windows) {
            assert_eq!(cmds[0], "sudo rm -rf -- '/tmp/a b'\\''c'");
        }
    }
}
