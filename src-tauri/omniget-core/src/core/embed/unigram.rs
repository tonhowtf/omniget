//! SentencePiece Unigram tokenizer, read from a HuggingFace `tokenizer.json`
//! (the XLM-RoBERTa one that `multilingual-e5-small` and the mMiniLM
//! reranker share). Written here, like the WordPiece one, instead of pulling
//! the `tokenizers` crate: the pipeline is short and the fixtures in
//! `tests/embed_fixtures/xlmr_ids.json` (recorded with `tokenizers` 0.23)
//! pin it id for id.
//!
//! Pipeline, as `tokenizers` runs it for XLM-R:
//! 1. `Precompiled` normalizer: SentencePiece's `nmt_nfkc` rules, stored as a
//!    darts-clone double-array trie plus a blob of replacement strings; each
//!    grapheme (or, failing that, each char) is looked up and replaced;
//! 2. runs of two or more spaces become one;
//! 3. `Metaspace`: spaces become `▁`, a `▁` is prepended, and the text is
//!    split before every `▁`;
//! 4. Viterbi over the vocabulary's log-probabilities per piece; a char no
//!    piece covers becomes `<unk>` (consecutive unknowns fuse into one);
//! 5. `<s> A </s>` or `<s> A </s></s> B </s>`.

use std::collections::HashMap;

use base64::Engine;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

use super::tokenizer::Encoding;

pub const BOS: u32 = 0;
pub const PAD: u32 = 1;
pub const EOS: u32 = 2;
/// Penalty `tokenizers` gives an unknown piece below the lowest score.
const UNK_PENALTY: f64 = 10.0;

#[derive(Debug)]
pub struct UnigramError(pub String);

impl std::fmt::Display for UnigramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Precompiled charsmap (darts-clone double array) ──────────────────────

struct Charsmap {
    units: Vec<u32>,
    normalized: Vec<u8>,
}

impl Charsmap {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let size = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?) as usize;
        let trie = bytes.get(4..4 + size)?;
        let units = trie
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Some(Self {
            units,
            normalized: bytes.get(4 + size..)?.to_vec(),
        })
    }

    fn prefixes(&self, key: &[u8]) -> Vec<u32> {
        let has_leaf = |u: u32| (u >> 8) & 1 == 1;
        let value = |u: u32| u & ((1u32 << 31) - 1);
        let label = |u: u32| u & ((1u32 << 31) | 0xFF);
        let offset = |u: u32| ((u >> 10) << ((u & (1u32 << 9)) >> 6)) as usize;
        let mut out = Vec::new();
        let Some(&root) = self.units.first() else {
            return out;
        };
        let mut pos = offset(root);
        for &c in key {
            if c == 0 {
                break;
            }
            pos ^= c as usize;
            let Some(&unit) = self.units.get(pos) else {
                return out;
            };
            if label(unit) != c as u32 {
                return out;
            }
            pos ^= offset(unit);
            if has_leaf(unit) {
                if let Some(&leaf) = self.units.get(pos) {
                    out.push(value(leaf));
                }
            }
        }
        out
    }

    /// The replacement for `chunk`, if the rules have one. Like
    /// `spm_precompiled`, the first (shortest) prefix match wins.
    fn transform(&self, chunk: &str) -> Option<&str> {
        let first = *self.prefixes(chunk.as_bytes()).first()? as usize;
        let end = self.normalized[first..]
            .iter()
            .position(|b| *b == 0)
            .map(|p| first + p)
            .unwrap_or(self.normalized.len());
        std::str::from_utf8(&self.normalized[first..end]).ok()
    }

    fn normalize(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for g in text.graphemes(true) {
            if g.len() < 6 {
                if let Some(n) = self.transform(g) {
                    out.push_str(n);
                    continue;
                }
            }
            for (i, c) in g.char_indices() {
                let part = &g[i..i + c.len_utf8()];
                match self.transform(part) {
                    Some(n) => out.push_str(n),
                    None => out.push_str(part),
                }
            }
        }
        out
    }
}

// ── Vocabulary trie (byte-keyed) ─────────────────────────────────────────

#[derive(Default)]
struct Node {
    next: HashMap<u8, usize>,
    id: Option<u32>,
}

pub struct Unigram {
    charsmap: Option<Charsmap>,
    nodes: Vec<Node>,
    scores: Vec<f64>,
    unk_id: u32,
    unk_score: f64,
}

impl Unigram {
    /// Reads `tokenizer.json`. Only the Unigram model, the `Precompiled`
    /// normalizer and `Metaspace` are supported, which is what XLM-R uses.
    pub fn from_json(text: &str) -> Result<Self, UnigramError> {
        let v: Value =
            serde_json::from_str(text).map_err(|e| UnigramError(format!("tokenizer.json: {e}")))?;
        let model = &v["model"];
        if model["type"] != "Unigram" {
            return Err(UnigramError("not a Unigram tokenizer".into()));
        }
        let vocab = model["vocab"]
            .as_array()
            .ok_or_else(|| UnigramError("no vocab".into()))?;
        let unk_id = model["unk_id"].as_u64().unwrap_or(3) as u32;
        let mut nodes = vec![Node::default()];
        let mut scores = Vec::with_capacity(vocab.len());
        let mut min = f64::INFINITY;
        for (id, entry) in vocab.iter().enumerate() {
            let piece = entry[0].as_str().unwrap_or("");
            let score = entry[1].as_f64().unwrap_or(0.0);
            scores.push(score);
            if score < min {
                min = score;
            }
            // Special pieces (<s>, </s>…) are never produced from text.
            if piece.is_empty() || (piece.starts_with('<') && piece.ends_with('>')) {
                continue;
            }
            let mut at = 0usize;
            for b in piece.bytes() {
                at = match nodes[at].next.get(&b) {
                    Some(&n) => n,
                    None => {
                        nodes.push(Node::default());
                        let n = nodes.len() - 1;
                        nodes[at].next.insert(b, n);
                        n
                    }
                };
            }
            nodes[at].id = Some(id as u32);
        }
        let charsmap = find_charsmap(&v["normalizer"]).and_then(|b64| {
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .ok()
                .and_then(|b| Charsmap::parse(&b))
        });
        Ok(Self {
            charsmap,
            nodes,
            scores,
            unk_id,
            unk_score: min - UNK_PENALTY,
        })
    }

    fn normalize(&self, text: &str) -> String {
        let n = match &self.charsmap {
            Some(c) => c.normalize(text),
            None => text.to_string(),
        };
        // " {2,}" → " "
        let mut out = String::with_capacity(n.len());
        let mut last_space = false;
        for ch in n.chars() {
            if ch == ' ' {
                if last_space {
                    continue;
                }
                last_space = true;
            } else {
                last_space = false;
            }
            out.push(ch);
        }
        out
    }

    /// Viterbi over one `▁`-prefixed word.
    fn word(&self, w: &str, out: &mut Vec<u32>) {
        let bytes = w.as_bytes();
        let n = bytes.len();
        // best[i] = (score, start, id) of the best path ending at byte i.
        let mut best: Vec<Option<(f64, usize, u32)>> = vec![None; n + 1];
        best[0] = Some((0.0, 0, 0));
        let starts: Vec<usize> = w.char_indices().map(|(i, _)| i).collect();
        for &s in &starts {
            let Some((base, _, _)) = best[s] else {
                continue;
            };
            let mut at = 0usize;
            let mut single_char_found = false;
            let char_end = s + w[s..].chars().next().map(char::len_utf8).unwrap_or(1);
            for (k, b) in bytes[s..].iter().enumerate() {
                let Some(&next) = self.nodes[at].next.get(b) else {
                    break;
                };
                at = next;
                if let Some(id) = self.nodes[at].id {
                    let e = s + k + 1;
                    if e == char_end {
                        single_char_found = true;
                    }
                    let score = base + self.scores[id as usize];
                    if best[e].map(|x| score > x.0).unwrap_or(true) {
                        best[e] = Some((score, s, id));
                    }
                }
            }
            if !single_char_found {
                let score = base + self.unk_score;
                if best[char_end].map(|x| score > x.0).unwrap_or(true) {
                    best[char_end] = Some((score, s, self.unk_id));
                }
            }
        }
        let mut ids = Vec::new();
        let mut e = n;
        while e > 0 {
            let Some((_, s, id)) = best[e] else { break };
            ids.push(id);
            e = s;
        }
        ids.reverse();
        // Fuse runs of <unk>.
        for id in ids {
            if id == self.unk_id && out.last() == Some(&self.unk_id) {
                continue;
            }
            out.push(id);
        }
    }

    /// Piece ids of `text`, without special tokens.
    pub fn pieces(&self, text: &str) -> Vec<u32> {
        let norm = self.normalize(text);
        if norm.is_empty() {
            return Vec::new();
        }
        let mut s = norm.replace(' ', "▁");
        if !s.starts_with('▁') {
            s.insert(0, '▁');
        }
        let mut out = Vec::new();
        let mut word_start = 0usize;
        for (i, _) in s.match_indices('▁') {
            if i > word_start {
                self.word(&s[word_start..i], &mut out);
            }
            word_start = i;
        }
        if word_start < s.len() {
            self.word(&s[word_start..], &mut out);
        }
        out
    }

    fn finish(ids: Vec<u32>) -> Encoding {
        let n = ids.len();
        Encoding {
            ids,
            attention_mask: vec![1; n],
            type_ids: vec![0; n],
        }
    }

    /// `<s> text </s>`, truncated to `max_len` tokens.
    pub fn encode(&self, text: &str, max_len: usize) -> Encoding {
        let mut p = self.pieces(text);
        p.truncate(max_len.saturating_sub(2));
        let mut ids = Vec::with_capacity(p.len() + 2);
        ids.push(BOS);
        ids.extend(p);
        ids.push(EOS);
        Self::finish(ids)
    }

    /// `<s> a </s></s> b </s>`, truncating the longer side first.
    pub fn encode_pair(&self, a: &str, b: &str, max_len: usize) -> Encoding {
        let mut pa = self.pieces(a);
        let mut pb = self.pieces(b);
        let budget = max_len.saturating_sub(4);
        while pa.len() + pb.len() > budget {
            if pa.len() > pb.len() {
                pa.pop();
            } else {
                pb.pop();
            }
        }
        let mut ids = Vec::with_capacity(pa.len() + pb.len() + 4);
        ids.push(BOS);
        ids.extend(pa);
        ids.push(EOS);
        ids.push(EOS);
        ids.extend(pb);
        ids.push(EOS);
        Self::finish(ids)
    }
}

/// Pads a batch to its longest row with `<pad>` and a zero mask.
pub fn pad(mut batch: Vec<Encoding>) -> Vec<Encoding> {
    let width = batch.iter().map(|e| e.ids.len()).max().unwrap_or(0);
    for e in &mut batch {
        while e.ids.len() < width {
            e.ids.push(PAD);
            e.attention_mask.push(0);
            e.type_ids.push(0);
        }
    }
    batch
}

fn find_charsmap(n: &Value) -> Option<&str> {
    match n["type"].as_str()? {
        "Precompiled" => n["precompiled_charsmap"].as_str(),
        "Sequence" => n["normalizers"].as_array()?.iter().find_map(find_charsmap),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the real `tokenizer.json` (17 MB, not in the repo). Run with
    /// `OMNIGET_XLMR_TOKENIZER=<path> cargo test ... unigram -- --ignored`.
    #[test]
    #[ignore = "needs the XLM-R tokenizer.json"]
    fn matches_huggingface_tokenizers() {
        let path = std::env::var("OMNIGET_XLMR_TOKENIZER").expect("OMNIGET_XLMR_TOKENIZER");
        let t = Unigram::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let fx: Value =
            serde_json::from_str(include_str!("../../../tests/embed_fixtures/xlmr_ids.json"))
                .unwrap();
        let mut bad = 0;
        for case in fx["single"].as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            let want: Vec<u32> = serde_json::from_value(case["ids"].clone()).unwrap();
            let got = t.encode(text, 512).ids;
            if got != want {
                bad += 1;
                eprintln!("{text:?}\n  want {want:?}\n  got  {got:?}");
            }
        }
        for case in fx["pair"].as_array().unwrap() {
            let want: Vec<u32> = serde_json::from_value(case["ids"].clone()).unwrap();
            let got = t
                .encode_pair(
                    case["a"].as_str().unwrap(),
                    case["b"].as_str().unwrap(),
                    512,
                )
                .ids;
            if got != want {
                bad += 1;
                eprintln!("pair\n  want {want:?}\n  got  {got:?}");
            }
        }
        assert_eq!(bad, 0);
    }
}
