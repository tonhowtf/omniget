use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Foreground {
    pub running: bool,
    pub label: String,
    pub pid: Option<u32>,
}

#[derive(Default)]
pub struct NameCache {
    pid: Option<u32>,
    name: String,
}

fn base_name(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('-').unwrap_or(s);
    let s = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let s = s.strip_suffix(".exe").unwrap_or(s);
    s.chars().take(64).collect()
}

#[cfg(target_os = "linux")]
fn name_of(pid: u32) -> Option<String> {
    let p = std::path::Path::new("/proc")
        .join(pid.to_string())
        .join("comm");
    std::fs::read_to_string(p)
        .ok()
        .map(|s| base_name(&s))
        .filter(|s| !s.is_empty())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn name_of(pid: u32) -> Option<String> {
    let out = crate::core::process::std_command("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = base_name(&String::from_utf8_lossy(&out.stdout));
    (!s.is_empty()).then_some(s)
}

#[cfg(unix)]
pub fn resolve(
    fg_pgid: Option<u32>,
    shell_pid: Option<u32>,
    shell_label: &str,
    cache: &mut NameCache,
) -> Foreground {
    let Some(pgid) = fg_pgid.filter(|p| *p > 0) else {
        return Foreground {
            running: false,
            label: shell_label.to_string(),
            pid: shell_pid,
        };
    };
    if Some(pgid) == shell_pid {
        return Foreground {
            running: false,
            label: shell_label.to_string(),
            pid: shell_pid,
        };
    }
    if cache.pid != Some(pgid) {
        cache.pid = Some(pgid);
        cache.name = name_of(pgid).unwrap_or_default();
    }
    let label = if cache.name.is_empty() {
        shell_label.to_string()
    } else {
        cache.name.clone()
    };
    Foreground {
        running: label != shell_label,
        label,
        pid: Some(pgid),
    }
}

#[cfg(windows)]
pub fn resolve(
    _fg_pgid: Option<u32>,
    shell_pid: Option<u32>,
    shell_label: &str,
    cache: &mut NameCache,
) -> Foreground {
    let idle = Foreground {
        running: false,
        label: shell_label.to_string(),
        pid: shell_pid,
    };
    let Some(root) = shell_pid else { return idle };
    let Some(table) = windows_process_table() else {
        return idle;
    };
    let mut cur = root;
    let mut name = String::new();
    loop {
        let next = table
            .iter()
            .filter(|(_, ppid, n)| {
                *ppid == cur
                    && !n.eq_ignore_ascii_case("conhost.exe")
                    && !n.eq_ignore_ascii_case("OpenConsole.exe")
            })
            .max_by_key(|(pid, _, _)| *pid);
        match next {
            Some((pid, _, n)) => {
                cur = *pid;
                name = base_name(n);
            }
            None => break,
        }
    }
    if cur == root {
        return idle;
    }
    cache.pid = Some(cur);
    cache.name = name.clone();
    Foreground {
        running: true,
        label: name,
        pid: Some(cur),
    }
}

#[cfg(windows)]
fn windows_process_table() -> Option<Vec<(u32, u32, String)>> {
    let out = crate::core::process::std_command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId)`t$($_.ParentProcessId)`t$($_.Name)\" }",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let rows = text
        .lines()
        .filter_map(|l| {
            let mut it = l.trim().splitn(3, '\t');
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let name = it.next()?.to_string();
            Some((pid, ppid, name))
        })
        .collect::<Vec<_>>();
    (!rows.is_empty()).then_some(rows)
}

pub fn label_for(program: &str) -> String {
    base_name(program)
}
