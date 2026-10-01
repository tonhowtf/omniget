#[derive(Debug, Clone, PartialEq, Default)]
pub struct IcyMeta {
    pub stream_title: Option<String>,
    pub stream_url: Option<String>,
}

#[derive(Debug)]
pub struct IcyDemuxer {
    metaint: usize,
    until_meta: usize,
    meta_left: usize,
    meta_buf: Vec<u8>,
}

impl IcyDemuxer {
    pub fn new(metaint: usize) -> Self {
        Self {
            metaint,
            until_meta: metaint,
            meta_left: 0,
            meta_buf: Vec::new(),
        }
    }

    pub fn push(&mut self, mut chunk: &[u8], audio: &mut Vec<u8>) -> Vec<IcyMeta> {
        let mut metas = Vec::new();
        if self.metaint == 0 {
            audio.extend_from_slice(chunk);
            return metas;
        }
        while !chunk.is_empty() {
            if self.meta_left > 0 {
                let n = self.meta_left.min(chunk.len());
                self.meta_buf.extend_from_slice(&chunk[..n]);
                self.meta_left -= n;
                chunk = &chunk[n..];
                if self.meta_left == 0 {
                    if let Some(m) = parse_block(&self.meta_buf) {
                        metas.push(m);
                    }
                    self.meta_buf.clear();
                    self.until_meta = self.metaint;
                }
            } else if self.until_meta > 0 {
                let n = self.until_meta.min(chunk.len());
                audio.extend_from_slice(&chunk[..n]);
                self.until_meta -= n;
                chunk = &chunk[n..];
            } else {
                let len = chunk[0] as usize * 16;
                chunk = &chunk[1..];
                if len == 0 {
                    self.until_meta = self.metaint;
                } else {
                    self.meta_left = len;
                }
            }
        }
        metas
    }
}

fn field(s: &str, key: &str) -> Option<String> {
    let needle = format!("{key}='");
    let start = s.find(&needle)? + needle.len();
    let rest = &s[start..];
    let end = rest.find("';").unwrap_or(rest.len());
    let v = rest[..end].trim_end_matches(['\'', ';']).trim().to_string();
    (!v.is_empty()).then_some(v)
}

pub fn parse_block(raw: &[u8]) -> Option<IcyMeta> {
    let trimmed: Vec<u8> = raw.iter().copied().take_while(|b| *b != 0).collect();
    if trimmed.is_empty() {
        return None;
    }
    let text = match String::from_utf8(trimmed.clone()) {
        Ok(s) => s,
        Err(_) => trimmed.iter().map(|&b| b as char).collect(),
    };
    Some(IcyMeta {
        stream_title: field(&text, "StreamTitle"),
        stream_url: field(&text, "StreamUrl"),
    })
}

pub fn split_title(stream_title: &str) -> (Option<String>, String) {
    match stream_title.split_once(" - ") {
        Some((a, t)) if !a.trim().is_empty() && !t.trim().is_empty() => {
            (Some(a.trim().to_string()), t.trim().to_string())
        }
        _ => (None, stream_title.trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(metaint: usize, title: &str) -> Vec<u8> {
        let mut v = vec![0xAAu8; metaint];
        let meta = format!("StreamTitle='{title}';");
        let blocks = meta.len().div_ceil(16);
        v.push(blocks as u8);
        let mut m = meta.into_bytes();
        m.resize(blocks * 16, 0);
        v.extend(m);
        v
    }

    #[test]
    fn splits_audio_and_titles_across_chunk_boundaries() {
        let mut stream = frame(8, "Band - It's Song");
        stream.extend(vec![0xAA; 8]);
        stream.push(0);
        stream.extend(vec![0xAA; 3]);
        for split in 1..stream.len() {
            let mut d = IcyDemuxer::new(8);
            let mut audio = Vec::new();
            let mut metas = d.push(&stream[..split], &mut audio);
            metas.extend(d.push(&stream[split..], &mut audio));
            assert_eq!(audio.len(), 19, "split {split}");
            assert!(audio.iter().all(|b| *b == 0xAA));
            assert_eq!(metas.len(), 1);
            assert_eq!(metas[0].stream_title.as_deref(), Some("Band - It's Song"));
        }
    }

    #[test]
    fn splits_artist_and_title() {
        assert_eq!(
            split_title("A - B"),
            (Some("A".to_string()), "B".to_string())
        );
        assert_eq!(
            split_title("Just a show"),
            (None, "Just a show".to_string())
        );
    }

    #[test]
    fn latin1_fallback() {
        let raw = b"StreamTitle='Caf\xe9';\0\0";
        let m = parse_block(raw).unwrap();
        assert_eq!(m.stream_title.as_deref(), Some("Café"));
    }
}
