use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Context};
use serde::Deserialize;

fn unicode_to_byte() -> HashMap<char, u8> {
    let mut bs: Vec<u32> = (b'!' as u32..=b'~' as u32)
        .chain(0xA1..=0xAC)
        .chain(0xAE..=0xFF)
        .collect();
    let mut cs = bs.clone();
    let mut n = 0u32;
    for b in 0..256u32 {
        if !bs.contains(&b) {
            bs.push(b);
            cs.push(256 + n);
            n += 1;
        }
    }
    bs.into_iter()
        .zip(cs)
        .filter_map(|(b, c)| char::from_u32(c).map(|c| (c, b as u8)))
        .collect()
}

#[derive(Debug, Deserialize)]
struct GenerationConfig {
    decoder_start_token_id: u32,
    eos_token_id: u32,
    no_timestamps_token_id: u32,
    #[serde(default)]
    begin_suppress_tokens: Vec<u32>,
    #[serde(default)]
    suppress_tokens: Vec<u32>,
    #[serde(default)]
    lang_to_id: HashMap<String, u32>,
    #[serde(default)]
    task_to_id: HashMap<String, u32>,
    #[serde(default)]
    max_initial_timestamp_index: Option<u32>,
}

pub struct Vocab {
    bytes: Vec<Vec<u8>>,
    pub sot: u32,
    pub eot: u32,
    pub transcribe: u32,
    pub no_timestamps: u32,
    pub no_speech: u32,
    pub timestamp_begin: u32,
    pub max_initial_timestamp: u32,
    pub languages: Vec<(String, u32)>,
    pub suppress: Vec<u32>,
    pub begin_suppress: Vec<u32>,
}

impl Vocab {
    pub fn load(vocab_json: &Path, generation_config: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read(vocab_json)
            .with_context(|| format!("reading {}", vocab_json.display()))?;
        let map: HashMap<String, u32> =
            serde_json::from_slice(&raw).context("parsing vocab.json")?;
        let raw = std::fs::read(generation_config)
            .with_context(|| format!("reading {}", generation_config.display()))?;
        let g: GenerationConfig =
            serde_json::from_slice(&raw).context("parsing generation_config.json")?;

        let table = unicode_to_byte();
        let eot = g.eos_token_id;
        let mut bytes = vec![Vec::new(); eot as usize];
        for (tok, id) in map {
            if (id as usize) < bytes.len() {
                bytes[id as usize] = tok.chars().filter_map(|c| table.get(&c).copied()).collect();
            }
        }
        let mut languages: Vec<(String, u32)> = g
            .lang_to_id
            .into_iter()
            .map(|(k, v)| {
                let code = k
                    .trim_start_matches("<|")
                    .trim_end_matches("|>")
                    .to_string();
                (code, v)
            })
            .collect();
        languages.sort_by_key(|(_, id)| *id);
        if languages.is_empty() {
            return Err(anyhow!(
                "this Whisper model is English-only; use a multilingual one"
            ));
        }
        let transcribe = g
            .task_to_id
            .get("transcribe")
            .copied()
            .unwrap_or(g.no_timestamps_token_id - 4);
        Ok(Self {
            bytes,
            sot: g.decoder_start_token_id,
            eot,
            transcribe,
            no_timestamps: g.no_timestamps_token_id,
            no_speech: g.no_timestamps_token_id - 1,
            timestamp_begin: g.no_timestamps_token_id + 1,
            max_initial_timestamp: g.max_initial_timestamp_index.unwrap_or(50),
            languages,
            suppress: g.suppress_tokens,
            begin_suppress: g.begin_suppress_tokens,
        })
    }

    pub fn language_id(&self, code: &str) -> Option<u32> {
        let code = code.trim().to_ascii_lowercase();
        let base = code.split(['-', '_']).next().unwrap_or("");
        self.languages
            .iter()
            .find(|(c, _)| *c == code || *c == base)
            .map(|(_, id)| *id)
    }

    pub fn language_code(&self, id: u32) -> Option<&str> {
        self.languages
            .iter()
            .find(|(_, i)| *i == id)
            .map(|(c, _)| c.as_str())
    }

    pub fn decode(&self, ids: &[u32]) -> String {
        let mut out = Vec::new();
        for id in ids {
            if let Some(b) = self.bytes.get(*id as usize) {
                out.extend_from_slice(b);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}
