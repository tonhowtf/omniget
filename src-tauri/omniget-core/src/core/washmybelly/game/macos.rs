//! macOS: Game Mode is the system's (automatic when a game is in full
//! screen, no public way to force it). What OmniGet can do without admin:
//! take the game out of the background band if something put it there, and
//! put its own children (yt-dlp, ffmpeg, agents) in the background band
//! (`PRIO_DARWIN_BG`, the same thing `taskpolicy -b` / `-B -p` does), which
//! is fully reversible by the same user. A `kqueue` `NOTE_EXIT` watcher
//! undoes it when the game exits.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use super::children::own_children;
use super::{Active, Applied, ApplyFn, Exec, ExitWaiter, PollWaiter, Target, UndoStep, WaiterFn};

// <sys/resource.h>
const PRIO_DARWIN_PROCESS: libc::c_int = 4;
const PRIO_DARWIN_BG: libc::c_int = 0x1000;

/// Whether `pid` is in the background band. `None` when it cannot be read.
pub fn get_bg(pid: u32) -> Option<bool> {
    unsafe {
        *libc::__error() = 0;
        let r = libc::getpriority(PRIO_DARWIN_PROCESS, pid as libc::id_t);
        if r == -1 && *libc::__error() != 0 {
            return None;
        }
        Some(r != 0)
    }
}

pub fn set_bg(pid: u32, on: bool) -> Result<(), String> {
    let v = if on { PRIO_DARWIN_BG } else { 0 };
    let r = unsafe { libc::setpriority(PRIO_DARWIN_PROCESS, pid as libc::id_t, v) };
    if r == 0 {
        Ok(())
    } else {
        Err(format!("setpriority {pid}: {}", std::io::Error::last_os_error()))
    }
}

pub struct Sys;

fn lower_children(game: u32, known: &[u32]) -> Vec<UndoStep> {
    own_children(game)
        .into_iter()
        .filter(|c| !known.contains(&c.pid))
        .filter(|c| get_bg(c.pid) == Some(false))
        .filter_map(|c| {
            set_bg(c.pid, true).ok()?;
            Some(UndoStep::DarwinBg {
                pid: c.pid,
                started: c.started,
                restore: false,
            })
        })
        .collect()
}

impl Exec for Sys {
    fn undo(&self, step: &UndoStep) -> Result<String, String> {
        match step {
            UndoStep::DarwinBg { pid, restore, .. } => set_bg(*pid, *restore).map(|_| if *restore { "back in the background band" } else { "normal priority again" }.to_string()),
            other => Err(format!("not a macOS step: {other:?}")),
        }
    }

    fn tick(&self, active: &Active) -> Vec<UndoStep> {
        let known: Vec<u32> = active.undo.iter().filter_map(UndoStep::child_pid).collect();
        lower_children(active.pid, &known)
    }
}

fn apply(target: &Target) -> Result<Applied, String> {
    let mut out = Applied {
        method: "macos".into(),
        notes: vec!["game_mode_automatic".into()],
        ..Default::default()
    };
    match get_bg(target.pid) {
        Some(true) => {
            set_bg(target.pid, false)?;
            out.applied.push("game_out_of_background".into());
            out.undo.push(UndoStep::DarwinBg {
                pid: target.pid,
                started: target.started,
                restore: true,
            });
        }
        Some(false) => out.notes.push("game_not_throttled".into()),
        None => out.notes.push("game_priority_unreadable".into()),
    }
    let kids = lower_children(target.pid, &[]);
    if !kids.is_empty() {
        out.applied.push("children_background".into());
    }
    out.undo.extend(kids);
    out.applied.push("watch_children".into());
    Ok(out)
}

/// `kqueue` with `EVFILT_PROC`/`NOTE_EXIT` on the game's pid.
pub struct KqWaiter {
    kq: libc::c_int,
    exited: bool,
    fallback: PollWaiter,
}

impl KqWaiter {
    pub fn new(pid: u32, started: u64) -> KqWaiter {
        let fallback = PollWaiter { pid, started };
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return KqWaiter { kq: -1, exited: false, fallback };
        }
        let ev = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let r = unsafe { libc::kevent(kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        // ESRCH: the process is already gone.
        let exited = r < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if r < 0 && !exited {
            unsafe { libc::close(kq) };
            return KqWaiter { kq: -1, exited: false, fallback };
        }
        KqWaiter { kq, exited, fallback }
    }
}

impl ExitWaiter for KqWaiter {
    fn wait(&mut self, timeout: Duration) -> bool {
        if self.exited {
            return true;
        }
        if self.kq < 0 {
            return self.fallback.wait(timeout);
        }
        let ts = libc::timespec {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_nsec: timeout.subsec_nanos() as libc::c_long,
        };
        let mut out = libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let n = unsafe { libc::kevent(self.kq, std::ptr::null(), 0, &mut out, 1, &ts) };
        if n > 0 && out.fflags & libc::NOTE_EXIT != 0 {
            self.exited = true;
        }
        self.exited
    }
}

impl Drop for KqWaiter {
    fn drop(&mut self) {
        if self.kq >= 0 {
            unsafe { libc::close(self.kq) };
        }
    }
}

pub fn plan(target: &Target) -> (ApplyFn, WaiterFn) {
    let t = target.clone();
    let (pid, started) = (target.pid, target.started);
    (Box::new(move || apply(&t)), Box::new(move || Box::new(KqWaiter::new(pid, started)) as Box<dyn ExitWaiter>))
}

/// Xcode's `gamepolicyctl`, the only reader of the Game Mode state. Found
/// through `xcode-select -p` (never `xcrun`, which may open an installer).
fn gamepolicyctl() -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(o) = crate::core::process::std_command("xcode-select").arg("-p").output() {
        if o.status.success() {
            dirs.push(PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
        }
    }
    dirs.push(PathBuf::from("/Applications/Xcode.app/Contents/Developer"));
    dirs.into_iter().map(|d| d.join("usr/bin/gamepolicyctl")).find(|p| p.is_file())
}

pub fn status(pid: Option<u32>) -> Value {
    let (detectable, on, policy) = match gamepolicyctl() {
        Some(bin) => match crate::core::process::std_command(&bin).args(["game-mode", "status"]).output() {
            Ok(o) if o.status.success() => {
                let s = super::parse::gamepolicyctl_status(&String::from_utf8_lossy(&o.stdout));
                (s.on.is_some(), s.on, s.policy)
            }
            _ => (false, None, None),
        },
        None => (false, None, None),
    };
    let mut v = json!({
        "backend": "macos",
        "available": true,
        "can_force": false,
        "reason": "macos_automatic",
        "system": {
            "game_mode_detectable": detectable,
            "game_mode_on": on,
            "policy": policy,
        },
        "offers": ["game_out_of_background", "children_background"],
        "install": null,
    });
    if let Some(p) = pid {
        v["pid_background"] = json!(get_bg(p));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn kqueue_sees_the_exit_of_our_test_child() {
        let mut child = std::process::Command::new("sleep").arg("1").spawn().unwrap();
        let mut w = KqWaiter::new(child.id(), 0);
        assert!(w.kq >= 0);
        let t0 = Instant::now();
        let mut exited = false;
        while t0.elapsed() < Duration::from_secs(5) {
            if w.wait(Duration::from_millis(500)) {
                exited = true;
                break;
            }
        }
        let _ = child.wait();
        assert!(exited);
        assert!(t0.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn background_band_round_trip_on_our_test_child() {
        let mut child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        let pid = child.id();
        let before = get_bg(pid);
        set_bg(pid, true).unwrap();
        let during = get_bg(pid);
        set_bg(pid, false).unwrap();
        let after = get_bg(pid);
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(before, Some(false));
        assert_eq!(during, Some(true));
        assert_eq!(after, Some(false));
    }

    #[test]
    fn status_never_claims_it_can_force() {
        let s = status(None);
        assert_eq!(s["can_force"], false);
        assert_eq!(s["backend"], "macos");
    }
}
