//! `portable-pty` backend of `core::term` (C1). The kill is ours, not the
//! crate's: `ChildKiller::kill` only sends `SIGHUP` to the shell's pid, so a
//! Claude or Codex started from that shell (a job in a process group of its
//! own) survived closing the terminal. Here the kill hits the shell's group
//! and the foreground job's group, escalating to `SIGKILL`; on Windows the
//! child goes into a kill-on-close Job Object right after spawn.

use std::io::{Read, Write};

use omniget_core::core::term::session::pty::{
    kill_groups, PtyBackend, PtyExit, PtyProcess, PtySpawn, PtyWaiter,
};
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};

pub struct PortableBackend;

struct PortableProcess {
    master: Box<dyn MasterPty + Send>,
    child: Option<Box<dyn Child + Send + Sync>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
    killed: bool,
    #[cfg(windows)]
    job: Option<omniget_core::core::term::session::pty::WinJob>,
}

struct PortableWaiter(Box<dyn Child + Send + Sync>);

impl PtyWaiter for PortableWaiter {
    fn wait(&mut self) -> PtyExit {
        match self.0.wait() {
            Ok(st) => {
                let signal = st.signal().map(|s| s.to_string());
                let code = if signal.is_some() {
                    None
                } else {
                    Some(st.exit_code() as i32)
                };
                PtyExit { code, signal }
            }
            Err(_) => PtyExit::default(),
        }
    }
}

impl PtyBackend for PortableBackend {
    fn spawn(&self, spec: &PtySpawn) -> Result<Box<dyn PtyProcess>, String> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: spec.rows.max(1),
                cols: spec.cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
        let mut cmd = match &spec.program {
            None => CommandBuilder::new_default_prog(),
            Some(p) => {
                let mut c = CommandBuilder::new(p);
                c.args(&spec.args);
                c
            }
        };
        if let Some(dir) = &spec.cwd {
            cmd.cwd(dir);
        }
        for k in &spec.env_remove {
            cmd.env_remove(k);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        // The child holds the slave now; keeping ours would hide EOF.
        drop(pair.slave);
        let pid = child.process_id();
        #[cfg(windows)]
        let job = pid.and_then(omniget_core::core::term::session::pty::WinJob::assign);
        let killer = child.clone_killer();
        Ok(Box::new(PortableProcess {
            master: pair.master,
            child: Some(child),
            killer,
            pid,
            killed: false,
            #[cfg(windows)]
            job,
        }))
    }
}

impl PtyProcess for PortableProcess {
    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, String> {
        self.master.try_clone_reader().map_err(|e| e.to_string())
    }

    fn take_writer(&mut self) -> Result<Box<dyn Write + Send>, String> {
        self.master.take_writer().map_err(|e| e.to_string())
    }

    fn take_waiter(&mut self) -> Option<Box<dyn PtyWaiter>> {
        self.child
            .take()
            .map(|c| Box::new(PortableWaiter(c)) as Box<dyn PtyWaiter>)
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())
    }

    fn foreground_pgid(&self) -> Option<u32> {
        #[cfg(unix)]
        {
            self.master
                .process_group_leader()
                .filter(|p| *p > 0)
                .map(|p| p as u32)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    fn kill(&mut self) {
        if self.killed {
            return;
        }
        self.killed = true;
        let mut groups: Vec<u32> = Vec::new();
        if let Some(fg) = self.foreground_pgid() {
            groups.push(fg);
        }
        if let Some(pid) = self.pid {
            groups.push(pid);
        }
        kill_groups(&groups);
        #[cfg(windows)]
        if let Some(job) = self.job.as_mut() {
            job.kill();
        }
        let _ = self.killer.kill();
    }
}
