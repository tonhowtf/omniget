use std::ffi::OsStr;
use std::io::Read;
use std::process::{Child, ChildStdout, Stdio};

use anyhow::{anyhow, Context};

use super::mel::SAMPLE_RATE;

pub struct PcmStream {
    child: Child,
    stdout: ChildStdout,
    buf: Vec<f32>,
    offset: usize,
    eof: bool,
    carry: Vec<u8>,
}

impl PcmStream {
    pub fn open(input: &OsStr) -> anyhow::Result<Self> {
        let mut child = crate::core::process::std_command("ffmpeg")
            .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i"])
            .arg(input)
            .args(["-map", "0:a:0", "-vn", "-ac", "1", "-ar"])
            .arg(SAMPLE_RATE.to_string())
            .args(["-f", "f32le", "-acodec", "pcm_f32le", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("ffmpeg is not available")?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("ffmpeg without stdout"))?;
        Ok(Self {
            child,
            stdout,
            buf: Vec::new(),
            offset: 0,
            eof: false,
            carry: Vec::new(),
        })
    }

    fn fill_to(&mut self, end: usize) -> anyhow::Result<()> {
        let mut chunk = vec![0u8; 1 << 16];
        while !self.eof && self.offset + self.buf.len() < end {
            let n = self.stdout.read(&mut chunk).context("reading ffmpeg")?;
            if n == 0 {
                self.eof = true;
                break;
            }
            self.carry.extend_from_slice(&chunk[..n]);
            let whole = self.carry.len() / 4 * 4;
            self.buf.extend(
                self.carry[..whole]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            );
            self.carry.drain(..whole);
        }
        Ok(())
    }

    pub fn window(&mut self, start: usize, len: usize) -> anyhow::Result<Vec<f32>> {
        if start > self.offset {
            let drop = (start - self.offset).min(self.buf.len());
            self.buf.drain(..drop);
            self.offset += drop;
        }
        self.fill_to(start + len)?;
        let from = start.saturating_sub(self.offset).min(self.buf.len());
        let to = (from + len).min(self.buf.len());
        Ok(self.buf[from..to].to_vec())
    }

    pub fn finish(mut self, got_audio: bool) -> anyhow::Result<()> {
        if got_audio {
            let _ = self.child.kill();
            let _ = self.child.wait();
            return Ok(());
        }
        let ok = self.child.wait().map(|s| s.success()).unwrap_or(false);
        Err(if ok {
            anyhow!("this file has no audio")
        } else {
            anyhow!("this file has no audio track, or ffmpeg could not read it")
        })
    }
}

impl Drop for PcmStream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.try_wait();
    }
}
