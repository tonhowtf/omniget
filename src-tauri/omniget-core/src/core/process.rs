#[cfg(not(windows))]
fn user_bin_dirs() -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        ".local/bin",
        ".npm-global/bin",
        ".bun/bin",
        ".cargo/bin",
        ".volta/bin",
        ".opencode/bin",
    ]
    .iter()
    .map(|d| home.join(d).display().to_string())
    .collect()
}

pub fn enhanced_path() -> Option<String> {
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let bin_dir = crate::core::paths::app_data_dir()?.join("bin");
            let sep = if cfg!(windows) { ";" } else { ":" };
            let current = std::env::var("PATH").unwrap_or_default();

            #[allow(unused_mut)]
            let mut extra_dirs: Vec<String> = vec![bin_dir.display().to_string()];

            #[cfg(not(windows))]
            extra_dirs.extend(user_bin_dirs());

            #[cfg(target_os = "macos")]
            {
                extra_dirs.push("/opt/homebrew/bin".into());
                extra_dirs.push("/usr/local/bin".into());
            }

            #[cfg(target_os = "linux")]
            extra_dirs.push("/usr/local/bin".into());

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

pub fn is_text_file_busy(err: &std::io::Error) -> bool {
    cfg!(unix) && err.raw_os_error() == Some(26)
}

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

// Adapted from herdrdev/herdr (Apache-2.0), src/platform/windows.rs:993-1128
// herdr. Licensed under the Apache License, Version 2.0; a copy is at

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

pub fn suspend_on_spawn(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    #[cfg(not(windows))]
    let _ = cmd;
}

pub struct ProcessTreeGuard {
    #[cfg(windows)]
    job: usize,
    #[cfg(unix)]
    root: Option<(i32, u64)>,
    #[cfg(unix)]
    pgid: Option<i32>,
    #[cfg(unix)]
    known: std::collections::HashMap<i32, u64>,
    #[cfg(unix)]
    done: bool,
}

impl ProcessTreeGuard {
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
        if let Err(error) = resume_suspended_process(child.id()) {
            return close(error);
        }
        Ok(Self { job: job as usize })
    }

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

    pub fn interrupt(&mut self) -> bool {
        #[cfg(unix)]
        {
            if self.done {
                return false;
            }
            let tree = self.walk();
            self.signal(&tree, libc::SIGTERM);
            true
        }
        #[cfg(not(unix))]
        false
    }

    pub fn terminate(&mut self) {
        #[cfg(windows)]
        if self.job != 0 {
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

    #[cfg(unix)]
    fn walk(&mut self) -> Vec<ProcRow> {
        let table = process_table();
        let mut roots: Vec<(i32, u64)> = self.known.iter().map(|(&p, &s)| (p, s)).collect();
        if let Some(pg) = self.pgid {
            roots.extend(
                table
                    .iter()
                    .filter(|r| r.pgid == pg)
                    .map(|r| (r.pid, r.start)),
            );
        }
        roots.extend(self.root);
        let tree = tree_of(&table, &roots);
        for row in &tree {
            self.known.insert(row.pid, row.start);
        }
        tree
    }

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

#[cfg(unix)]
const SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);

#[cfg(any(unix, test))]
#[cfg_attr(not(unix), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcRow {
    pid: i32,
    ppid: i32,
    pgid: i32,
    start: u64,
    frozen: bool,
}

#[cfg(any(unix, test))]
#[cfg_attr(not(unix), allow(dead_code))]
fn tree_of(table: &[ProcRow], roots: &[(i32, u64)]) -> Vec<ProcRow> {
    use std::collections::{HashMap, HashSet};
    let by_pid: HashMap<i32, &ProcRow> = table.iter().map(|r| (r.pid, r)).collect();
    let mut children: HashMap<i32, Vec<&ProcRow>> = HashMap::new();
    for row in table {
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

#[cfg(any(target_os = "linux", target_os = "android"))]
fn parse_proc_stat(stat: &str) -> Option<ProcRow> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let pid: i32 = stat[..open].trim().parse().ok()?;
    let rest: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
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
        let stat = std::fs::read(format!("/proc/{pid}/stat")).ok()?;
        parse_proc_stat(&String::from_utf8_lossy(&stat))
    })
    .collect()
}

#[cfg(target_os = "macos")]
fn process_table() -> Vec<ProcRow> {
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

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn process_table() -> Vec<ProcRow> {
    Vec::new()
}

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
        let _ = pid;
        Some(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleProcess {
    Gone,
    NotOurs,
    Killed,
    Survived,
}

pub fn start_token(pid: u32) -> Option<u64> {
    #[cfg(unix)]
    {
        start_time(i32::try_from(pid).ok()?)
    }
    #[cfg(windows)]
    {
        windows_start_token(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

#[cfg(windows)]
fn windows_start_token(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(pid).map(|p| p.start_time())
}

pub fn stop_stale(pid: u32, start: u64) -> StaleProcess {
    match start_token(pid) {
        None => return StaleProcess::Gone,
        Some(now) if now != start => return StaleProcess::NotOurs,
        Some(_) => {}
    }
    #[cfg(unix)]
    {
        let Ok(p) = i32::try_from(pid) else {
            return StaleProcess::NotOurs;
        };
        if p <= 1 || p == std::process::id() as i32 {
            return StaleProcess::NotOurs;
        }
        // SAFETY: plain syscall; a pid that is gone just returns -1.
        let pgid = (unsafe { libc::getpgid(p) } == p).then_some(p);
        let mut guard = ProcessTreeGuard {
            root: Some((p, start)),
            pgid,
            known: std::iter::once((p, start)).collect(),
            done: false,
        };
        guard.terminate();
    }
    #[cfg(windows)]
    {
        let _ = std_command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    for _ in 0..50 {
        if start_token(pid) != Some(start) {
            return StaleProcess::Killed;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    StaleProcess::Survived
}

impl Drop for ProcessTreeGuard {
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

#[cfg(all(test, unix))]
pub(crate) mod test_exec {
    use std::path::{Path, PathBuf};

    const LAUNCHER: &str = "#!/bin/sh\nexec /bin/sh \"$0.body\" \"$@\"\n";

    fn launcher() -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join("omniget-test-exec");
        let path = dir.join("launcher-v1");
        if std::fs::read(&path).is_ok_and(|b| b == LAUNCHER.as_bytes()) {
            return path;
        }
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join(format!("launcher-{}.tmp", uuid::Uuid::new_v4().simple()));
        std::fs::write(&tmp, LAUNCHER).unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        path
    }

    pub fn script(path: &Path, body: &str) {
        let mut body_path = path.as_os_str().to_owned();
        body_path.push(".body");
        std::fs::write(PathBuf::from(body_path), body).unwrap();
        let _ = std::fs::remove_file(path);
        let launcher = launcher();
        if std::fs::hard_link(&launcher, path).is_err() {
            std::fs::copy(&launcher, path).unwrap();
        }
    }
}
