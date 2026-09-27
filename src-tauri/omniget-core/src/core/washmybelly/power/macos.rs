//! macOS: `pmset` for Low Power Mode and sleep assertions, `caffeinate`
//! for the hold.

use std::process::{Child, Stdio};

use super::hold::Backend;
use super::parse;

fn pmset(args: &[&str]) -> Result<String, String> {
    let out = crate::core::process::std_command("/usr/bin/pmset")
        .args(args)
        .output()
        .map_err(|e| format!("pmset: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "pmset: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn settings() -> Result<parse::PmsetSettings, String> {
    Ok(parse::pmset_settings(&pmset(&["-g"])?))
}

pub fn assertions() -> Result<Vec<parse::SleepBlocker>, String> {
    Ok(parse::pmset_assertions(&pmset(&["-g", "assertions"])?))
}

/// Runs as root in the helper, after `elevated_setting` validated the value.
pub fn set_low_power_mode(on: bool) -> Result<String, String> {
    pmset(&["-a", "lowpowermode", if on { "1" } else { "0" }])?;
    Ok(format!("Low Power Mode {}", if on { "on" } else { "off" }))
}

/// `caffeinate -i -w <our pid> -t <secs>`: exits on its own when OmniGet
/// quits or the time is up. We keep the child and end only that PID.
pub struct CaffeinateBackend;

struct Caffeinate(Child);

impl Drop for Caffeinate {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Backend for CaffeinateBackend {
    fn name(&self) -> &'static str {
        "caffeinate"
    }

    fn acquire(&self, _reason: &str, max_secs: Option<u64>) -> Result<Box<dyn Send>, String> {
        let pid = std::process::id().to_string();
        let mut cmd = crate::core::process::std_command("/usr/bin/caffeinate");
        cmd.args(["-i", "-w", pid.as_str()]);
        let secs;
        if let Some(s) = max_secs {
            secs = (s + 5).to_string();
            cmd.args(["-t", secs.as_str()]);
        }
        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("caffeinate: {e}"))?;
        Ok(Box::new(Caffeinate(child)))
    }
}
