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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_the_whole_tree_and_nothing_else() {
        let table = [
            (1, None),
            (10, Some(1)),     // OmniGet
            (11, Some(10)),    // yt-dlp
            (12, Some(11)),    // ffmpeg under yt-dlp
            (13, Some(10)),    // agent
            (20, Some(1)),     // someone else's
            (21, Some(20)),    // their child
            (30, Some(30)),    // self-parented garbage
        ];
        assert_eq!(descendants(10, &table), vec![11, 12, 13]);
        assert!(descendants(21, &table).is_empty());
    }

    #[test]
    fn a_cycle_does_not_hang() {
        let table = [(2, Some(3)), (3, Some(2))];
        assert_eq!(descendants(2, &table), vec![3]);
    }

    #[test]
    fn a_spawned_child_is_found() {
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sleep" })
            .args(if cfg!(windows) { vec!["/C", "timeout", "/T", "3"] } else { vec!["3"] })
            .spawn()
            .expect("spawn test child");
        let pid = child.id();
        let found = own_children(0).iter().any(|c| c.pid == pid);
        let skipped = own_children(pid).iter().any(|c| c.pid == pid);
        let _ = child.kill();
        let _ = child.wait();
        assert!(found);
        assert!(!skipped);
    }
}
