use flate2::{Decompress, FlushDecompress, Status};

const SUFFIX: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];
const MAX_FRAME: usize = 64 * 1024 * 1024;

pub struct ZlibStream {
    inflater: Decompress,
    pending: Vec<u8>,
}

impl Default for ZlibStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ZlibStream {
    pub fn new() -> Self {
        Self {
            inflater: Decompress::new(true),
            pending: Vec::new(),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Option<String>, String> {
        self.pending.extend_from_slice(chunk);
        if self.pending.len() > MAX_FRAME {
            self.pending.clear();
            return Err("gateway frame too large".into());
        }
        if !ends_with_flush(&self.pending) {
            return Ok(None);
        }
        let input = std::mem::take(&mut self.pending);
        let mut out: Vec<u8> = Vec::new();
        let mut offset = 0usize;
        loop {
            out.reserve((input.len() - offset).max(1) * 4 + 1024);
            let before_in = self.inflater.total_in();
            let status = self
                .inflater
                .decompress_vec(&input[offset..], &mut out, FlushDecompress::Sync)
                .map_err(|e| format!("zlib: {e}"))?;
            offset += (self.inflater.total_in() - before_in) as usize;
            if out.len() > MAX_FRAME {
                return Err("gateway payload too large".into());
            }
            let spare = out.capacity() - out.len();
            if matches!(status, Status::StreamEnd) || (offset >= input.len() && spare > 0) {
                break;
            }
        }
        let text =
            String::from_utf8(out).map_err(|_| "gateway payload is not UTF-8".to_string())?;
        Ok(Some(text))
    }
}

pub fn ends_with_flush(buf: &[u8]) -> bool {
    buf.len() >= 4 && buf[buf.len() - 4..] == SUFFIX
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    fn stream_frames(messages: &[&str]) -> Vec<Vec<u8>> {
        let mut enc = Compress::new(Compression::default(), true);
        let mut frames = Vec::new();
        for m in messages {
            let start = enc.total_in();
            let mut frame = Vec::new();
            let bytes = m.as_bytes();
            loop {
                let done = (enc.total_in() - start) as usize;
                let mut chunk = Vec::with_capacity(bytes.len() + 1024);
                enc.compress_vec(&bytes[done..], &mut chunk, FlushCompress::Sync)
                    .unwrap();
                frame.extend_from_slice(&chunk);
                if (enc.total_in() - start) as usize >= bytes.len() && ends_with_flush(&frame) {
                    break;
                }
            }
            frames.push(frame);
        }
        frames
    }

    #[test]
    fn sync_flush_frames_end_with_the_suffix() {
        let frames = stream_frames(&["{\"op\":10}"]);
        assert!(ends_with_flush(&frames[0]));
        assert!(!ends_with_flush(&[0x00, 0xFF, 0xFF]));
        assert!(!ends_with_flush(&[]));
    }

    #[test]
    fn decodes_consecutive_messages_sharing_one_context() {
        let a = r#"{"op":10,"d":{"heartbeat_interval":41250}}"#;
        let b = r#"{"op":0,"t":"MESSAGE_CREATE","s":2,"d":{"content":"heartbeat_interval again"}}"#;
        let frames = stream_frames(&[a, b, a]);
        let mut z = ZlibStream::new();
        assert_eq!(z.push(&frames[0]).unwrap().as_deref(), Some(a));
        assert_eq!(z.push(&frames[1]).unwrap().as_deref(), Some(b));
        assert_eq!(z.push(&frames[2]).unwrap().as_deref(), Some(a));
    }

    #[test]
    fn waits_for_the_suffix_when_a_frame_is_split() {
        let big: String = (0..5000).map(|i| format!("{{\"n\":{i}}},")).collect();
        let msg = format!("{{\"op\":0,\"d\":[{}0]}}", big);
        let frames = stream_frames(&[&msg]);
        let frame = &frames[0];
        let mut z = ZlibStream::new();
        let cut = frame.len() / 2;
        assert_eq!(z.push(&frame[..cut]).unwrap(), None);
        assert_eq!(
            z.push(&frame[cut..]).unwrap().as_deref(),
            Some(msg.as_str())
        );
    }

    #[test]
    fn rejects_garbage() {
        let mut z = ZlibStream::new();
        assert!(z.push(&[1, 2, 3, 4, 0, 0, 0xFF, 0xFF]).is_err());
    }
}
