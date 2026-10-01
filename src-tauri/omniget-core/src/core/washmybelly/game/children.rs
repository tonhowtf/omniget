//! OmniGet's own descendants (yt-dlp, ffmpeg, agents): the only processes
//! the game mode ever slows down. Never a process OmniGet did not start.

use std::collections::{HashMap, HashSet};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Every pid below `root` in the `(pid, parent)` table, `root` excluded.
/// Walks breadth first; a loop in the table (pid reuse) cannot hang it.
pub fn descendants(root: u32, table: &[(u32, Option<u32>)]) -> Vec<u32> {
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(pid, parent) in table {
        if let Some(pp) = parent {
            if pp != pid {
                kids.entry(pp).or_default().push(pid);
            }
        }
    }
    let mut seen = HashSet::from([root]);
    let mut out = Vec::new();
    let mut queue = vec![root];
    while let Some(p) = queue.pop() {
        for &c in kids.get(&p).map(Vec::as_slice).unwrap_or(&[]) {
            if seen.insert(c) {
                out.push(c);
                queue.push(c);
            }
        }
    }
    out.sort_unstable();
    out
}

/// A running descendant: pid, start time (to spot pid reuse) and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Child {
    pub pid: u32,
    pub started: u64,
    pub name: String,
}

/// OmniGet's descendants right now, except `skip` (the game, if the game
/// happens to have been launched by OmniGet).
pub fn own_children(skip: u32) -> Vec<Child> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let table: Vec<(u32, Option<u32>)> = sys
        .processes()
        .values()
        .filter(|p| p.thread_kind().is_none())
        .map(|p| (p.pid().as_u32(), p.parent().map(|x| x.as_u32())))
        .collect();
    descendants(std::process::id(), &table)
        .into_iter()
        .filter(|&pid| pid != skip)
        .filter_map(|pid| {
            let p = sys.process(Pid::from_u32(pid))?;
            Some(Child {
                pid,
                started: p.start_time(),
                name: p.name().to_string_lossy().to_string(),
            })
        })
        .collect()
}
