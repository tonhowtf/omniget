fn enhanced_path() -> Option<String> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let bin_dir = crate::core::paths::app_data_dir()?.join("bin");
            let sep = if cfg!(windows) { ";" } else { ":" };
            let current = std::env::var("PATH").unwrap_or_default();

            #[allow(unused_mut)]
            let mut extra_dirs: Vec<String> = vec![bin_dir.display().to_string()];

            #[cfg(target_os = "macos")]
            {
                extra_dirs.push("/opt/homebrew/bin".into());
                extra_dirs.push("/usr/local/bin".into());
            }

            #[cfg(target_os = "linux")]
            {
                if let Some(home) = dirs::home_dir() {
                    extra_dirs.push(home.join(".local").join("bin").display().to_string());
                }
                extra_dirs.push("/usr/local/bin".into());
            }

            Some(format!("{}{}{}", extra_dirs.join(sep), sep, current))
        })
        .clone()
}

pub fn command<S: AsRef<std::ffi::OsStr>>(program: S) -> tokio::process::Command {
    let pinned = if crate::core::dependencies::worker_mode() {
        program
            .as_ref()
            .to_str()
            .and_then(crate::core::dependencies::worker_tool)
    } else {
        None
    };
    let mut cmd = tokio::process::Command::new(
        pinned
            .as_deref()
            .map(|p| p.as_os_str())
            .unwrap_or(program.as_ref()),
    );
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    if let Some(path) = enhanced_path() {
        cmd.env("PATH", path);
    }
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.env("PYTHONLEGACYWINDOWSSTDIO", "0");
    cmd
}

pub fn std_command<S: AsRef<std::ffi::OsStr>>(program: S) -> std::process::Command {
    let pinned = if crate::core::dependencies::worker_mode() {
        program
            .as_ref()
            .to_str()
            .and_then(crate::core::dependencies::worker_tool)
    } else {
        None
    };
    let mut cmd = std::process::Command::new(
        pinned
            .as_deref()
            .map(|p| p.as_os_str())
            .unwrap_or(program.as_ref()),
    );
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    if let Some(path) = enhanced_path() {
        cmd.env("PATH", path);
    }
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.env("PYTHONUTF8", "1");
    cmd.env("PYTHONLEGACYWINDOWSSTDIO", "0");
    cmd
}

/// `ETXTBSY`: on Linux a file that was just written cannot be executed while
/// any process still holds it open for writing, and a `fork` on another thread
/// keeps that descriptor alive until its own `exec`. The window is a few
/// milliseconds; a launcher script written and started in the same breath
/// (ours, or a test's fake CLI) lands in it now and then.
pub fn is_text_file_busy(err: &std::io::Error) -> bool {
    cfg!(unix) && err.raw_os_error() == Some(26)
}

/// Runs `spawn`, trying again for a moment when the program is "text file busy".
pub fn spawn_retrying_busy<T>(mut spawn: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut tries = 0;
    loop {
        match spawn() {
            Err(e) if is_text_file_busy(&e) && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            other => return other,
        }
    }
}

// ── Process tree containment ──────────────────────────────────────────────
//
// Adapted from herdrdev/herdr (Apache-2.0), src/platform/windows.rs:993-1128
// @ c33fd40eab3d92b564d7dd25bbfdd462e4b7aba2 (`configure_status_command`,
// `StatusCommandGuard`, `resume_suspended_process`). Changes: the guard also
// knows how to let go without killing (`release`), and it has a Unix half
// (process group plus a walk of the descendants, below) that is not from
// herdr. Licensed under the Apache License, Version 2.0; a copy is at
// http://www.apache.org/licenses/LICENSE-2.0.

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// Windows: the child is created suspended (and without a console window),
/// so not one instruction of it runs before [`ProcessTreeGuard::new`] has put
/// it in a kill-on-close Job Object and resumed it. Without this a `claude`
/// that forks `bash` or an MCP server in its first milliseconds leaves that
/// grandchild outside the job. Unix: nothing (the caller uses
/// `process_group(0)`).
pub fn suspend_on_spawn(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Owns the whole process tree of one child.
///
/// Windows: a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; every
/// process the child starts inherits the job, so closing it (`terminate`, or
/// dropping the guard) ends all of them at once — the only reliable tree kill
/// there, where a plain `TerminateProcess` of the child leaves `git`, `bash`
/// and MCP servers running.
///
/// Unix: the child leads a process group of its own (`process_group(0)`),
/// and `killpg` is the fast path, but a descendant that calls `setsid()` or
/// `setpgid()` (an MCP server, `nohup`, a daemonizing tool, some `node`
/// launchers) leaves the group and used to survive every cancel. So the
/// guard also walks the parent links of the process table (`/proc` on Linux,
/// libproc on macOS) from the child down and signals every descendant it
/// finds, each one checked by its start time so a recycled pid is never hit.
/// What the walk cannot see is a process whose whole chain of parents
/// already exited before the kill (a double fork): the kernel handed it to
/// `init`/`launchd` and nothing links it to the turn any more.
///
/// Deliberately not a `PR_SET_CHILD_SUBREAPER`: that would keep those
/// orphans linked, but they would become children of the whole app, and
/// every one of them that exits later (from any turn, any tool) would stay a
/// zombie until something here reaps it — a reaper thread for the life of the
/// app, racing tokio's own `waitpid` on the children it spawns.
pub struct ProcessTreeGuard {
    #[cfg(windows)]
    job: usize,
    /// The child as (pid, start time); `None` when it was gone before the
    /// guard could read it (a zombie has no children left to find).
    #[cfg(unix)]
    root: Option<(i32, u64)>,
    /// The group `killpg` may hit: only when the child leads a group of its
    /// own, so a caller that forgot `process_group(0)` never gets the app's
    /// own group killed. The id cannot be recycled while any member lives.
    #[cfg(unix)]
    pgid: Option<i32>,
    /// Every member seen so far, as (pid, start time): a descendant whose
    /// parent died during the kill is still reached through this.
    #[cfg(unix)]
    known: std::collections::HashMap<i32, u64>,
    /// Killed or released: nothing more to do, `Drop` included.
    #[cfg(unix)]
    done: bool,
}

impl ProcessTreeGuard {
    /// Puts a child spawned through [`suspend_on_spawn`] in a fresh
    /// kill-on-close job and only then resumes it. On any failure the job is
    /// closed (which kills the child if it was already assigned); a child
    /// that never got assigned stays suspended and the caller must kill it.
    #[cfg(windows)]
    pub fn new(child: &tokio::process::Child) -> std::io::Result<Self> {
        use std::mem::size_of;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let close = |error: std::io::Error| {
            unsafe {
                CloseHandle(job);
            }
            Err(error)
        };

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let Ok(limits_size) = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
        else {
            return close(std::io::Error::other("job limits size exceeds u32"));
        };
        if unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                limits_size,
            )
        } == 0
        {
            return close(std::io::Error::last_os_error());
        }
        let Some(process) = child.raw_handle() else {
            return close(std::io::Error::other("the child has no process handle"));
        };
        if unsafe { AssignProcessToJobObject(job, process.cast()) } == 0 {
            return close(std::io::Error::last_os_error());
        }
        // Assigned: from here closing the job kills the child, which is
        // exactly what a failed resume needs (never leave it suspended).
        if let Err(error) = resume_suspended_process(child.id()) {
            return close(error);
        }
        Ok(Self { job: job as usize })
    }

    /// Unix: records the child (pid and start time) and its group. Never
    /// fails; the `Result` is the Windows one.
    ///
    /// No pidfd for the child (Linux ≥ 5.3): tokio holds it unreaped until
    /// its `wait`, so its pid cannot be recycled while we signal it, and the
    /// descendants are not our children, so no pidfd could be opened for
    /// them without the same pid-then-open race the start time already
    /// settles.
    #[cfg(unix)]
    pub fn new(child: &tokio::process::Child) -> std::io::Result<Self> {
        let pid = child.id().and_then(|p| i32::try_from(p).ok());
        let root = pid.and_then(|p| start_time(p).map(|start| (p, start)));
        // SAFETY: plain syscall; a pid that is gone just returns -1.
        let pgid = pid.filter(|&p| unsafe { libc::getpgid(p) } == p);
        Ok(Self {
            root,
            pgid,
            known: root.into_iter().collect(),
            done: false,
        })
    }

    #[cfg(not(any(windows, unix)))]
    pub fn new(_child: &tokio::process::Child) -> std::io::Result<Self> {
        Ok(Self {})
    }

    /// The polite first half of a kill: `SIGTERM` to the group and to every
    /// descendant outside it, so a CLI can stop its MCP servers itself.
    /// Returns whether anything was asked, i.e. whether a grace period is
    /// worth waiting before [`terminate`](Self::terminate). Windows has no
    /// such signal: the job close is the only step, so `false`.
    pub fn interrupt(&mut self) -> bool {
        #[cfg(unix)]
        {
            if self.done {
                return false;
            }
            // The walk comes before any signal: once the group's members
            // die, their escaped children are handed to init and the parent
            // links that lead to them are gone.
            let tree = self.walk();
            self.signal(&tree, libc::SIGTERM);
            true
        }
        #[cfg(not(unix))]
        false
    }

    /// Kills every process still in the tree. Idempotent.
    ///
    /// Unix: freeze, then kill. Everything found gets `SIGSTOP` (a stopped
    /// process can neither fork nor exit, so the parent links hold still),
    /// and the walk repeats until a pass finds nobody new and everyone
    /// signalled is really stopped — that catches a child forked while the
    /// kill was under way. Bounded by [`SETTLE_LIMIT`]: a process stuck in
    /// uninterruptible I/O never reports stopped, and a cancel must not
    /// hang on it. Then `SIGKILL` to the group and to every member.
    pub fn terminate(&mut self) {
        #[cfg(windows)]
        if self.job != 0 {
            // KILL_ON_JOB_CLOSE ends the child and every descendant still in
            // the job.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(
                    self.job as windows_sys::Win32::Foundation::HANDLE,
                );
            }
            self.job = 0;
        }
        #[cfg(unix)]
        {
            if self.done {
                return;
            }
            self.done = true;
            let since = std::time::Instant::now();
            let mut stopping: std::collections::HashSet<i32> = std::collections::HashSet::new();
            let mut tree = self.walk();
            if let Some(pg) = self.pgid {
                // SAFETY: a group we created (see `pgid`).
                unsafe {
                    libc::killpg(pg, libc::SIGSTOP);
                }
            }
            loop {
                let mut fresh = false;
                for row in &tree {
                    if stopping.insert(row.pid) {
                        fresh = true;
                        // SAFETY: a member of the tree, its start time just read.
                        if unsafe { libc::kill(row.pid, libc::SIGSTOP) } != 0 {
                            // Not ours to stop (another user): not waited on.
                            stopping.remove(&row.pid);
                        }
                    }
                }
                let still_running = tree
                    .iter()
                    .any(|row| stopping.contains(&row.pid) && !row.frozen);
                if (!fresh && !still_running) || since.elapsed() >= SETTLE_LIMIT {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
                tree = self.walk();
            }
            self.signal(&tree, libc::SIGKILL);
        }
    }

    /// Lets go without killing: after a normal exit, whatever the agent left
    /// running on purpose (a dev server it started) keeps running, exactly as
    /// on Unix, where a clean exit sends no `killpg`. The kill-on-close limit
    /// is cleared first, so closing the last handle no longer kills.
    pub fn release(mut self) {
        #[cfg(windows)]
        if self.job != 0 {
            use windows_sys::Win32::System::JobObjects::{
                JobObjectExtendedLimitInformation, SetInformationJobObject,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            };
            let limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
            let job = self.job as windows_sys::Win32::Foundation::HANDLE;
            let cleared = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    size,
                )
            } != 0;
            if !cleared {
                // Could not disarm it: better to leave the handle open for
                // the life of the app than to kill what the agent left.
                self.job = 0;
                return;
            }
        }
        #[cfg(unix)]
        {
            self.done = true;
        }
        self.terminate();
    }

    /// The tree as it is now: the child, every member seen before (even one
    /// whose parent is gone) and every group member are the roots, and each
    /// root is taken only if its start time still matches.
    #[cfg(unix)]
    fn walk(&mut self) -> Vec<ProcRow> {
        let table = process_table();
        let mut roots: Vec<(i32, u64)> = self.known.iter().map(|(&p, &s)| (p, s)).collect();
        if let Some(pg) = self.pgid {
            // Our group only ever holds our own descendants; one whose
            // parent already exited still leads to whatever it started.
            roots.extend(table.iter().filter(|r| r.pgid == pg).map(|r| (r.pid, r.start)));
        }
        roots.extend(self.root);
        let tree = tree_of(&table, &roots);
        for row in &tree {
            self.known.insert(row.pid, row.start);
        }
        tree
    }

    /// `signal` to the group and to each member that left it. A member still
    /// in the group is not signalled twice: a second `SIGTERM` is "force
    /// quit" to some CLIs.
    #[cfg(unix)]
    fn signal(&self, tree: &[ProcRow], signal: libc::c_int) {
        if let Some(pg) = self.pgid {
            // SAFETY: a group we created (see `pgid`).
            unsafe {
                libc::killpg(pg, signal);
            }
        }
        for row in tree.iter().filter(|r| Some(r.pgid) != self.pgid) {
            // SAFETY: a member of the tree whose start time was just read.
            unsafe {
                libc::kill(row.pid, signal);
            }
        }
    }
}

/// How long [`ProcessTreeGuard::terminate`] waits for the tree to hold still
/// before it kills whatever it has found.
#[cfg(unix)]
const SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);

/// One process as the tree walk needs it. `start` tells a process from a
/// later one that got the same pid (Linux: clock ticks since boot; macOS:
/// microseconds since the epoch); `frozen` = stopped or already dead.
#[cfg(any(unix, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcRow {
    pid: i32,
    ppid: i32,
    pgid: i32,
    start: u64,
    frozen: bool,
}

/// Every process reachable from `roots` by parent links, roots included.
/// A root counts only while a process with its pid *and* its start time
/// exists: a pid recycled since it was recorded is someone else, and so are
/// that someone's children.
#[cfg(any(unix, test))]
fn tree_of(table: &[ProcRow], roots: &[(i32, u64)]) -> Vec<ProcRow> {
    use std::collections::{HashMap, HashSet};
    let by_pid: HashMap<i32, &ProcRow> = table.iter().map(|r| (r.pid, r)).collect();
    let mut children: HashMap<i32, Vec<&ProcRow>> = HashMap::new();
    for row in table {
        // pid 0 (the swapper/kernel_task) lists itself as its own parent.
        if row.ppid != row.pid {
            children.entry(row.ppid).or_default().push(row);
        }
    }
    let mut seen: HashSet<i32> = HashSet::new();
    let mut out = Vec::new();
    let mut queue: Vec<&ProcRow> = roots
        .iter()
        .filter_map(|&(pid, start)| by_pid.get(&pid).copied().filter(|r| r.start == start))
        .collect();
    while let Some(row) = queue.pop() {
        // Never the kernel, init/launchd or ourselves, whatever the table says.
        if row.pid <= 1 || row.pid == std::process::id() as i32 || !seen.insert(row.pid) {
            continue;
        }
        out.push(*row);
        if let Some(kids) = children.get(&row.pid) {
            queue.extend(kids.iter().copied());
        }
    }
    out
}

/// One line of `/proc/<pid>/stat`. The second field is the command name in
/// parentheses, and that name may itself hold spaces and `)` (`(sd-pam)`,
/// `(my (odd) name)`), so the fields are counted from the *last* `)`:
/// state, ppid, pgrp, … and the start time is field 22 of the man page.
#[cfg(any(target_os = "linux", target_os = "android", test))]
fn parse_proc_stat(stat: &str) -> Option<ProcRow> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let pid: i32 = stat[..open].trim().parse().ok()?;
    let rest: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    // `rest[0]` is field 3 (state), so field N sits at `rest[N - 3]`.
    let state = rest.first()?.chars().next()?;
    Some(ProcRow {
        pid,
        ppid: rest.get(1)?.parse().ok()?,
        pgid: rest.get(2)?.parse().ok()?,
        start: rest.get(19)?.parse().ok()?,
        frozen: matches!(state, 'T' | 't' | 'Z' | 'X' | 'x'),
    })
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn process_table() -> Vec<ProcRow> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|entry| {
        let pid: i32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
        // Bytes, not a String: the command name need not be UTF-8.
        let stat = std::fs::read(format!("/proc/{pid}/stat")).ok()?;
        parse_proc_stat(&String::from_utf8_lossy(&stat))
    })
    .collect()
}

#[cfg(target_os = "macos")]
fn process_table() -> Vec<ProcRow> {
    // With no buffer libproc answers how many pids there are; the slack
    // covers the ones started between the two calls.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let Ok(count) = usize::try_from(count) else {
        return Vec::new();
    };
    let mut pids: Vec<libc::pid_t> = vec![0; count + 64];
    let Ok(bytes) = libc::c_int::try_from(std::mem::size_of_val(pids.as_slice())) else {
        return Vec::new();
    };
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    let Ok(got) = usize::try_from(got) else {
        return Vec::new();
    };
    pids.truncate(got.min(pids.len()));
    pids.into_iter().filter_map(bsd_row).collect()
}

/// A process as libproc's `PROC_PIDTBSDINFO` describes it; `None` for one
/// that is gone or that we may not look at.
#[cfg(target_os = "macos")]
fn bsd_row(pid: libc::pid_t) -> Option<ProcRow> {
    if pid <= 0 {
        return None;
    }
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
    let got = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            size,
        )
    };
    if got != size {
        return None;
    }
    Some(ProcRow {
        pid,
        ppid: i32::try_from(info.pbi_ppid).ok()?,
        pgid: i32::try_from(info.pbi_pgid).ok()?,
        start: info
            .pbi_start_tvsec
            .saturating_mul(1_000_000)
            .saturating_add(info.pbi_start_tvusec),
        frozen: matches!(info.pbi_status, libc::SSTOP | libc::SZOMB),
    })
}

/// Other Unixes: no table, so the group is the whole tree (`killpg` only).
#[cfg(all(unix, not(any(target_os = "linux", target_os = "android", target_os = "macos"))))]
fn process_table() -> Vec<ProcRow> {
    Vec::new()
}

/// The start time of one process, to recognise it later.
#[cfg(unix)]
fn start_time(pid: i32) -> Option<u64> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return std::fs::read(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| parse_proc_stat(&String::from_utf8_lossy(&stat)))
        .map(|row| row.start);
    #[cfg(target_os = "macos")]
    return bsd_row(pid).map(|row| row.start);
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    {
        // No table to match against: any constant does.
        let _ = pid;
        Some(0)
    }
}

impl Drop for ProcessTreeGuard {
    /// A guard dropped without `release` (a panic, an early return) takes
    /// the tree with it: nothing it started may outlive the turn unnoticed.
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(windows)]
fn resume_suspended_process(process_id: Option<u32>) -> std::io::Result<()> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    let process_id =
        process_id.ok_or_else(|| std::io::Error::other("the child has no process id"))?;
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }

    let result = (|| {
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = u32::try_from(size_of::<THREADENTRY32>())
            .map_err(|_| std::io::Error::other("thread entry size exceeds u32"))?;
        if unsafe { Thread32First(snapshot, &mut entry) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        loop {
            // A process created suspended has exactly one thread: resume it.
            if entry.th32OwnerProcessID == process_id {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let resumed = unsafe { ResumeThread(thread) };
                let error = (resumed == u32::MAX).then(std::io::Error::last_os_error);
                unsafe {
                    CloseHandle(thread);
                }
                return match error {
                    Some(error) => Err(error),
                    None => Ok(()),
                };
            }
            if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
                return Err(std::io::Error::other(
                    "the primary thread of the child was not found",
                ));
            }
        }
    })();

    unsafe {
        CloseHandle(snapshot);
    }
    result
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    const STAT: &str = "4242 (sleep) S 4200 4200 4100 34817 4200 4194304 101 0 0 0 0 0 0 0 \
                        20 0 1 0 987654 5373952 196 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3";

    #[test]
    fn a_proc_stat_line_gives_parent_group_and_start() {
        let row = parse_proc_stat(STAT).expect("a valid line");
        assert_eq!(
            row,
            ProcRow { pid: 4242, ppid: 4200, pgid: 4200, start: 987654, frozen: false }
        );
        let stopped = STAT.replacen(") S ", ") T ", 1);
        assert!(parse_proc_stat(&stopped).unwrap().frozen);
        let zombie = STAT.replacen(") S ", ") Z ", 1);
        assert!(parse_proc_stat(&zombie).unwrap().frozen);
    }

    /// The command name is free text: spaces and `)` inside it must not
    /// shift the fields (a name like `a) R 1 1` would otherwise read as
    /// state R, parent 1).
    #[test]
    fn a_command_name_with_spaces_and_parentheses_is_skipped_whole() {
        for comm in ["my (odd) name", "a) R 1 1", "(sd-pam)", " ", ")"] {
            let line = STAT.replacen("(sleep)", &format!("({comm})"), 1);
            let row = parse_proc_stat(&line).unwrap_or_else(|| panic!("{comm:?}"));
            assert_eq!((row.pid, row.ppid, row.pgid, row.start), (4242, 4200, 4200, 987654), "{comm:?}");
        }
    }

    #[test]
    fn a_broken_proc_stat_line_is_none() {
        for line in ["", "4242", "4242 (sleep)", "x (sleep) S 1 1", "4242 (sleep) S 4200 4200 4100"] {
            assert_eq!(parse_proc_stat(line), None, "{line:?}");
        }
    }

    fn row(pid: i32, ppid: i32, pgid: i32, start: u64) -> ProcRow {
        ProcRow { pid, ppid, pgid, start, frozen: false }
    }

    fn pids(rows: &[ProcRow]) -> Vec<i32> {
        let mut out: Vec<i32> = rows.iter().map(|r| r.pid).collect();
        out.sort_unstable();
        out
    }

    /// The tree follows parent links whatever the group: 102 left the group
    /// (`setsid`) and still counts, and so does its child; the stranger 200
    /// and its child do not.
    #[test]
    fn the_tree_follows_parents_across_sessions_and_nothing_else() {
        let table = [
            row(1, 0, 1, 1),
            row(100, 50, 100, 10),
            row(101, 100, 100, 11),
            row(102, 101, 102, 12),
            row(103, 102, 102, 13),
            row(200, 1, 200, 14),
            row(201, 200, 200, 15),
        ];
        assert_eq!(pids(&tree_of(&table, &[(100, 10)])), vec![100, 101, 102, 103]);
        // A member whose parent already died is still reached as a root.
        assert_eq!(pids(&tree_of(&table, &[(100, 10), (201, 15)])), vec![100, 101, 102, 103, 201]);
    }

    /// A pid recycled since it was recorded is another process: neither it
    /// nor its children are touched.
    #[test]
    fn a_recycled_pid_is_not_a_root() {
        let table = [row(100, 1, 100, 99), row(101, 100, 100, 100)];
        assert!(tree_of(&table, &[(100, 10)]).is_empty());
    }

    /// Never init, never a loop: a table read mid-change can be odd.
    #[test]
    fn the_walk_skips_init_and_survives_a_cycle() {
        let table = [row(1, 1, 1, 1), row(50, 51, 50, 5), row(51, 50, 50, 6), row(52, 1, 52, 7)];
        assert_eq!(pids(&tree_of(&table, &[(50, 5)])), vec![50, 51]);
        assert!(tree_of(&table, &[(1, 1)]).is_empty());
    }

    /// The real table has this very process in it, with its parent.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn the_process_table_sees_this_process() {
        let me = std::process::id() as i32;
        let table = process_table();
        let row = table.iter().find(|r| r.pid == me).expect("this process");
        assert_eq!(row.ppid, unsafe { libc::getppid() });
        assert_eq!(start_time(me), Some(row.start));
    }

    #[cfg(unix)]
    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks existence.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// A child in a group of its own (as `spawn_turn_with` makes it) that
    /// starts a grandchild in a new session, writes its pid and waits.
    #[cfg(unix)]
    async fn escaping_tree() -> (tokio::process::Child, i32, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("omniget-tree-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("escapee.pid");
        let code = "import os, subprocess, sys, time\n\
                    p = subprocess.Popen(['sleep', '120'], start_new_session=True)\n\
                    open(sys.argv[1] + '.t', 'w').write(str(p.pid))\n\
                    os.rename(sys.argv[1] + '.t', sys.argv[1])\n\
                    time.sleep(120)\n";
        let child = tokio::process::Command::new("python3")
            .args(["-c", code])
            .arg(&pidfile)
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let since = std::time::Instant::now();
        while !pidfile.exists() && since.elapsed() < std::time::Duration::from_secs(10) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let escapee = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        (child, escapee, dir)
    }

    #[cfg(unix)]
    async fn gone_within_1s(pid: i32) -> bool {
        let since = std::time::Instant::now();
        while alive(pid) && since.elapsed() < std::time::Duration::from_secs(1) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        !alive(pid)
    }

    /// `killpg` alone misses the grandchild that called `setsid()`; the
    /// walk does not.
    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_kills_a_descendant_that_left_the_group() {
        let (mut child, escapee, dir) = escaping_tree().await;
        let mut guard = ProcessTreeGuard::new(&child).unwrap();
        assert!(guard.interrupt(), "Unix asks politely first");
        guard.terminate();
        let dead = gone_within_1s(escapee).await;
        let exited = tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await;
        if !dead {
            unsafe { libc::kill(escapee, libc::SIGKILL) };
        }
        let _ = std::fs::remove_dir_all(dir);
        assert!(dead, "the escaped grandchild survived");
        assert!(exited.is_ok(), "the child survived");
        // Idempotent, and nothing left to signal.
        guard.terminate();
        assert!(!guard.interrupt());
    }

    /// A guard dropped without `release` takes the tree with it (a panic,
    /// an early return), escapees included.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dropped_guard_takes_the_escapee_with_it() {
        let (_child, escapee, dir) = escaping_tree().await;
        drop(ProcessTreeGuard::new(&_child).unwrap());
        let dead = gone_within_1s(escapee).await;
        if !dead {
            unsafe { libc::kill(escapee, libc::SIGKILL) };
        }
        let _ = std::fs::remove_dir_all(dir);
        assert!(dead, "the escaped grandchild survived the drop");
    }

    /// After a clean end nothing is killed: what the agent left running on
    /// purpose keeps running, like on Windows.
    #[cfg(unix)]
    #[tokio::test]
    async fn release_leaves_the_tree_running() {
        let (mut child, escapee, dir) = escaping_tree().await;
        ProcessTreeGuard::new(&child).unwrap().release();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let running = alive(escapee) && child.try_wait().unwrap().is_none();
        unsafe { libc::kill(escapee, libc::SIGKILL) };
        let _ = child.kill().await;
        let _ = std::fs::remove_dir_all(dir);
        assert!(running, "release must not kill");
    }

    /// Windows: the child runs only after it is inside the job, and closing
    /// the job ends its grandchild too (`ping -t` never ends by itself).
    #[cfg(windows)]
    #[tokio::test]
    async fn terminate_kills_the_grandchild_on_windows() {
        // `start /b` leaves a grandchild ping; the second ping keeps cmd up.
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.args([
            "/c", "start", "/b", "ping", "-t", "127.0.0.1", "&", "ping", "-t", "127.0.0.1",
        ]);
        suspend_on_spawn(&mut cmd);
        cmd.kill_on_drop(true);
        let mut child = cmd.spawn().unwrap();
        let mut guard = ProcessTreeGuard::new(&child).expect("assign + resume");
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let count = || {
            let out = std::process::Command::new("tasklist")
                .args(["/FI", "IMAGENAME eq PING.EXE", "/NH"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.to_ascii_lowercase().contains("ping.exe"))
                .count()
        };
        assert!(count() >= 1, "the fake tree did not start");
        let started = std::time::Instant::now();
        guard.terminate();
        let _ = child.wait().await;
        while count() > 0 && started.elapsed() < std::time::Duration::from_secs(1) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(count(), 0, "a ping survived the job close");
    }
}
