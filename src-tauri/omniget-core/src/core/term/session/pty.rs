//! The PTY behind a terminal, as a trait so the core never depends on a PTY
//! crate. The app provides the `portable-pty` backend (C1); an own
//! `rustix`/ConPTY backend can replace it later without touching the manager.
//!
//! Also here: the kill that `portable-pty` gets wrong for an agent
//! orchestrator. Its `ChildKiller::kill` sends `SIGHUP` to the shell's pid
//! only, and on Windows it kills `cmd.exe`/`pwsh` and leaves `node.exe` or
//! the Codex binary running. [`kill_groups`] signals whole process groups
//! (the shell's own and the foreground job's), escalating to `SIGKILL`, and
//! [`WinJob`] puts the child in a kill-on-close Job Object right after spawn.

use std::io::{Read, Write};
use std::path::PathBuf;

/// What to spawn. `program: None` opens the user's default shell.
#[derive(Debug, Clone, Default)]
pub struct PtySpawn {
    pub program: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    /// Inherited variables removed before `env` is applied.
    pub env_remove: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

/// How the program ended.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PtyExit {
    pub code: Option<i32>,
    pub signal: Option<String>,
}

pub trait PtyBackend: Send + Sync {
    fn spawn(&self, spec: &PtySpawn) -> Result<Box<dyn PtyProcess>, String>;
}

/// A spawned program on a PTY. The reader, writer and waiter are taken once
/// by the session (they move to their own threads).
pub trait PtyProcess: Send {
    fn pid(&self) -> Option<u32>;
    fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, String>;
    fn take_writer(&mut self) -> Result<Box<dyn Write + Send>, String>;
    fn take_waiter(&mut self) -> Option<Box<dyn PtyWaiter>>;
    fn resize(&self, cols: u16, rows: u16) -> Result<(), String>;
    /// Foreground process group of the terminal (`tcgetpgrp`), unix only.
    fn foreground_pgid(&self) -> Option<u32>;
    /// Kills the program and everything it started. Idempotent.
    fn kill(&mut self);
}

pub trait PtyWaiter: Send {
    /// Blocks until the program exits.
    fn wait(&mut self) -> PtyExit;
}

/// Grace between `SIGHUP` and `SIGKILL`.
#[cfg(unix)]
const KILL_GRACE: std::time::Duration = std::time::Duration::from_millis(600);

/// Unix: `SIGHUP` to each process group (the shell leads its own after
/// `setsid`; an interactive shell puts every job in a group of its own, so
/// the foreground job is passed too), then `SIGKILL` to whatever group still
/// exists after a short grace, from a detached thread. Groups `<= 1` and the
/// app's own group are never signalled.
#[cfg(unix)]
pub fn kill_groups(groups: &[u32]) {
    // SAFETY: plain syscall.
    let own = unsafe { libc::getpgrp() } as u32;
    let mut targets: Vec<i32> = Vec::new();
    for &g in groups {
        if g <= 1 || g == own {
            continue;
        }
        let Ok(g) = i32::try_from(g) else { continue };
        if !targets.contains(&g) {
            targets.push(g);
        }
    }
    if targets.is_empty() {
        return;
    }
    for &g in &targets {
        // SAFETY: a group created for this terminal (see above).
        unsafe {
            libc::killpg(g, libc::SIGHUP);
            libc::killpg(g, libc::SIGCONT);
        }
    }
    let _ = std::thread::Builder::new()
        .name("term-kill".into())
        .spawn(move || {
            std::thread::sleep(KILL_GRACE);
            for g in targets {
                // SAFETY: signal 0 only probes; SIGKILL to a group we started.
                unsafe {
                    if libc::killpg(g, 0) == 0 {
                        libc::killpg(g, libc::SIGKILL);
                    }
                }
            }
        });
}

#[cfg(not(unix))]
pub fn kill_groups(_groups: &[u32]) {}

/// Windows: a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` holding
/// the child (assigned by pid right after spawn), so closing it ends the whole
/// tree. A child that already started grandchildren before the assignment
/// leaves those outside the job; the C3 own backend spawns suspended to close
/// that gap.
#[cfg(windows)]
pub struct WinJob {
    job: usize,
}

#[cfg(windows)]
impl WinJob {
    pub fn assign(pid: u32) -> Option<Self> {
        use std::mem::size_of;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };
        // SAFETY: FFI with checked handles; every handle opened is closed.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(job);
                return None;
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                CloseHandle(job);
                return None;
            }
            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                CloseHandle(job);
                return None;
            }
            Some(Self { job: job as usize })
        }
    }

    /// Ends every process in the job. Idempotent.
    pub fn kill(&mut self) {
        if self.job == 0 {
            return;
        }
        // SAFETY: the handle is ours and closed exactly once.
        unsafe {
            let h = self.job as windows_sys::Win32::Foundation::HANDLE;
            windows_sys::Win32::System::JobObjects::TerminateJobObject(h, 1);
            windows_sys::Win32::Foundation::CloseHandle(h);
        }
        self.job = 0;
    }
}

#[cfg(windows)]
impl Drop for WinJob {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Builds the spawn spec for an argv: empty = the default shell; on unix a
/// command runs through the login shell (`exec "$0" "$@"`, no quoting) so it
/// sees the user's PATH. Returns the spec and the label of the program that
/// "idles" in the terminal.
pub fn spec_for(argv: &[String]) -> (PtySpawn, String) {
    use super::foreground::label_for;
    if argv.is_empty() {
        #[cfg(unix)]
        if !shell_env_ok() {
            // $SHELL points at a shell that is gone: open one that exists.
            let sh = available_shell();
            let label = label_for(&sh);
            return (
                PtySpawn {
                    program: Some(sh),
                    args: vec!["-l".into()],
                    ..Default::default()
                },
                label,
            );
        }
        return (PtySpawn::default(), default_shell_label());
    }
    let label = label_for(&argv[0]);
    #[cfg(unix)]
    {
        // An absolute zsh or fish is the shell integration itself (it passes
        // its own `-l`): no wrapper, which would source the profile twice
        // (and fish cannot run the POSIX wrapper at all).
        if std::path::Path::new(&argv[0]).is_absolute() && matches!(label.as_str(), "zsh" | "fish")
        {
            return (
                PtySpawn {
                    program: Some(argv[0].clone()),
                    args: argv[1..].to_vec(),
                    ..Default::default()
                },
                label,
            );
        }
        let shell = available_shell();
        // The login profile may put another `omniget` first on PATH: the
        // app's CLI (`$OMNIGET_CLI`) goes back in front before the exec.
        let script = format!(
            "{}; exec \"$0\" \"$@\"",
            crate::core::term::agent::launch::POSIX_PATH_FIX
        );
        let mut args = vec!["-l".to_string(), "-c".into(), script];
        args.extend(argv.iter().cloned());
        (
            PtySpawn {
                program: Some(shell),
                args,
                ..Default::default()
            },
            label,
        )
    }
    #[cfg(not(unix))]
    {
        (
            PtySpawn {
                program: Some(argv[0].clone()),
                args: argv[1..].to_vec(),
                ..Default::default()
            },
            label,
        )
    }
}

/// `$SHELL` is set and the file exists.
#[cfg(unix)]
fn shell_env_ok() -> bool {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some_and(|s| std::path::Path::new(&s).exists())
}

/// A shell that exists: `$SHELL`, else zsh, bash, sh (`%ComSpec%` on Windows).
pub fn available_shell() -> String {
    #[cfg(unix)]
    {
        if shell_env_ok() {
            return std::env::var("SHELL").unwrap_or_default();
        }
        for c in ["/bin/zsh", "/bin/bash", "/usr/bin/bash", "/bin/sh"] {
            if std::path::Path::new(c).exists() {
                return c.to_string();
            }
        }
        "/bin/sh".to_string()
    }
    #[cfg(not(unix))]
    {
        std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())
    }
}

/// The program of `argv` is a path (not a bare name looked up on PATH) that
/// does not exist: the terminal would only print "not found" and exit.
pub fn missing_program(argv: &[String]) -> bool {
    let Some(p) = argv.first() else { return false };
    (p.contains('/') || p.contains('\\')) && !std::path::Path::new(p).exists()
}

pub fn default_shell_label() -> String {
    #[cfg(unix)]
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    #[cfg(not(unix))]
    let shell = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
    super::foreground::label_for(&shell)
}
