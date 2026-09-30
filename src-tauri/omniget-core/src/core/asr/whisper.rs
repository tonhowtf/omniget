//! The Whisper decoder loop on two ONNX sessions: the encoder turns a 30 s
//! log-mel window into hidden states, the merged decoder (one graph with and
//! without KV cache, switched by `use_cache_branch`) predicts one token per
//! step. Greedy search with Whisper's own logit rules (suppressed tokens,
//! paired timestamps that never go back, the initial-timestamp cap), language
//! detection from the most likely language token, and the seek logic of
//! `whisper/transcribe.py` to walk long audio window by window.

use std::borrow::Cow;
use std::collections::HashMap;

use anyhow::{anyhow, Context};
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};

use super::mel::{self, N_MELS, SAMPLE_RATE, WINDOW_FRAMES, WINDOW_SAMPLES};
use super::models::{self, ModelSpec};
use super::tokens::Vocab;
use super::Segment;

/// Most tokens sampled in one window (`n_text_ctx / 2`, like Whisper).
const MAX_SAMPLE: usize = 224;
/// Decoder context.
const MAX_CTX: usize = 448;
/// One timestamp token step: 20 ms.
const TS_SAMPLES: usize = SAMPLE_RATE / 50;
const NO_SPEECH_THRESHOLD: f32 = 0.6;
const LOGPROB_THRESHOLD: f32 = -1.0;

pub struct Model {
    pub id: &'static str,
    spec: &'static ModelSpec,
    encoder: Session,
    decoder: Session,
    pub vocab: Vocab,
    decoder_inputs: Vec<String>,
}

impl Model {
    /// Blocking: loads both graphs. The files must already be on disk.
    pub fn load(spec: &'static ModelSpec) -> anyhow::Result<Self> {
        if !models::is_downloaded(spec) {
            return Err(anyhow!("the {} model has not been downloaded yet", spec.id));
        }
        let path = |f: &str| models::file_path(spec, f).ok_or_else(|| anyhow!("no model dir"));
        crate::core::onnxrt::init()?;
        let encoder = crate::core::tools::onnx::session_from_file(&path(models::ENCODER)?)?;
        let decoder = crate::core::tools::onnx::session_from_file(&path(models::DECODER)?)?;
        let vocab = Vocab::load(&path(models::VOCAB)?, &path(models::GEN_CONFIG)?)?;
        let decoder_inputs = decoder
            .inputs()
            .iter()
            .map(|i| i.name().to_string())
            .collect();
        Ok(Self {
            id: spec.id,
            spec,
            encoder,
            decoder,
            vocab,
            decoder_inputs,
        })
    }

    /// Hidden states of one window's log-mel (`[N_MELS][WINDOW_FRAMES]`).
    fn encode(&mut self, mel: Vec<f32>) -> anyhow::Result<DynValue> {
        let input = Tensor::from_array((vec![1i64, N_MELS as i64, WINDOW_FRAMES as i64], mel))
            .context("building input_features")?;
        let name = self
            .encoder
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "input_features".into());
        let out_name = self
            .encoder
            .outputs()
            .first()
            .map(|o| o.name().to_string())
            .unwrap_or_else(|| "last_hidden_state".into());
        let inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> =
            vec![(Cow::Owned(name), input.into())];
        let mut out = self.encoder.run(inputs).context("running the encoder")?;
        out.remove(&out_name)
            .ok_or_else(|| anyhow!("the encoder has no `{out_name}` output"))
    }

    /// One decoder call. `cache` empty means the first step (no-cache
    /// branch); it is filled with the new KV values. Returns the logits of
    /// every input position (`rows × vocab`) and the vocab width.
    fn step(
        &mut self,
        ids: &[u32],
        enc: &DynValue,
        cache: &mut HashMap<String, DynValue>,
    ) -> anyhow::Result<(Vec<f32>, usize)> {
        let first = cache.is_empty();
        let ids_t = Tensor::from_array((
            vec![1i64, ids.len() as i64],
            ids.iter().map(|t| *t as i64).collect::<Vec<_>>(),
        ))?;
        let use_cache = Tensor::from_array((vec![1i64], vec![!first]))?;
        let (heads, head_dim) = (self.spec.heads as i64, self.spec.head_dim as i64);
        let mut inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> =
            Vec::with_capacity(self.decoder_inputs.len());
        let mut ids_t = Some(ids_t);
        let mut use_cache = Some(use_cache);
        for name in &self.decoder_inputs {
            let value: SessionInputValue<'_> = match name.as_str() {
                "input_ids" => ids_t
                    .take()
                    .ok_or_else(|| anyhow!("input_ids twice"))?
                    .into(),
                "encoder_hidden_states" => enc.into(),
                "use_cache_branch" => use_cache
                    .take()
                    .ok_or_else(|| anyhow!("use_cache_branch twice"))?
                    .into(),
                n if n.starts_with("past_key_values.") => {
                    let present = n.replacen("past_key_values", "present", 1);
                    match cache.get(&present) {
                        Some(v) => v.into(),
                        // First step: the no-cache branch ignores these,
                        // they only need a valid shape.
                        None => Tensor::from_array((
                            vec![1i64, heads, 1, head_dim],
                            vec![0f32; (heads * head_dim) as usize],
                        ))?
                        .into(),
                    }
                }
                other => return Err(anyhow!("the decoder asks for an unknown input `{other}`")),
            };
            inputs.push((Cow::Owned(name.clone()), value));
        }
        let mut out = self.decoder.run(inputs).context("running the decoder")?;
        let (shape, logits) = out["logits"]
            .try_extract_tensor::<f32>()
            .context("reading the logits")?;
        let width = *shape
            .last()
            .ok_or_else(|| anyhow!("logits without shape"))? as usize;
        let logits = logits.to_vec();
        let names: Vec<String> = out
            .keys()
            .filter(|k| k.starts_with("present."))
            .map(|k| k.to_string())
            .collect();
        for n in names {
            // The encoder half of the cache is computed once, on the first
            // step; later steps only pass it through.
            if !first && n.contains(".encoder.") {
                continue;
            }
            if let Some(v) = out.remove(&n) {
                cache.insert(n, v);
            }
        }
        Ok((logits, width))
    }

    /// The most likely language token for this window.
    fn detect_language(&mut self, enc: &DynValue) -> anyhow::Result<u32> {
        let mut cache = HashMap::new();
        let (logits, _) = self.step(&[self.vocab.sot], enc, &mut cache)?;
        let (_, id) = self
            .vocab
            .languages
            .iter()
            .filter_map(|(_, id)| logits.get(*id as usize).map(|l| (*l, *id)))
            .fold((f32::MIN, self.vocab.languages[0].1), |best, cur| {
                if cur.0 > best.0 {
                    cur
                } else {
                    best
                }
            });
        Ok(id)
    }

    fn apply_rules(&self, logits: &mut [f32], sampled: &[u32]) {
        let v = &self.vocab;
        let neg = f32::NEG_INFINITY;
        let tsb = v.timestamp_begin as usize;
        let eot = v.eot as usize;
        let n = logits.len();
        for t in &v.suppress {
            if let Some(l) = logits.get_mut(*t as usize) {
                *l = neg;
            }
        }
        // Every special token (sot, languages, tasks, no-speech,
        // no-timestamps) except end-of-text.
        for l in logits.iter_mut().take(tsb.min(n)).skip(eot + 1) {
            *l = neg;
        }
        if sampled.is_empty() {
            for t in &v.begin_suppress {
                if let Some(l) = logits.get_mut(*t as usize) {
                    *l = neg;
                }
            }
        }

        let is_ts = |t: u32| t as usize >= tsb;
        let len = sampled.len();
        let last_ts = len >= 1 && is_ts(sampled[len - 1]);
        let penult_ts = len < 2 || is_ts(sampled[len - 2]);
        if last_ts {
            if penult_ts {
                logits[tsb.min(n)..].fill(neg);
            } else {
                logits[..eot.min(n)].fill(neg);
            }
        }
        if let Some(prev) = sampled.iter().rev().find(|t| is_ts(**t)) {
            let upto = if last_ts && !penult_ts {
                *prev as usize
            } else {
                *prev as usize + 1
            };
            logits[tsb.min(n)..upto.min(n)].fill(neg);
        }
        if sampled.is_empty() {
            logits[..tsb.min(n)].fill(neg);
            let cap = tsb + v.max_initial_timestamp as usize + 1;
            if cap < n {
                logits[cap..].fill(neg);
            }
        }

        // If timestamps together are likelier than any single text token,
        // a timestamp it is.
        if tsb < n {
            let max = logits.iter().copied().fold(neg, f32::max);
            if max.is_finite() {
                let lse = |s: &[f32]| {
                    let m = s.iter().copied().fold(neg, f32::max);
                    if !m.is_finite() {
                        return neg;
                    }
                    m + s.iter().map(|x| (x - m).exp()).sum::<f32>().ln()
                };
                let ts_mass = lse(&logits[tsb..]);
                let max_text = logits[..tsb].iter().copied().fold(neg, f32::max);
                if ts_mass > max_text {
                    logits[..tsb].fill(neg);
                }
            }
        }
    }

    /// Greedy decode of one window. Returns the sampled tokens (without
    /// end-of-text), the no-speech probability and the mean log-probability.
    fn decode_window(&mut self, enc: &DynValue, lang: u32) -> anyhow::Result<(Vec<u32>, f32, f32)> {
        let prompt = [self.vocab.sot, lang, self.vocab.transcribe];
        let mut cache = HashMap::new();
        let (logits, width) = self.step(&prompt, enc, &mut cache)?;
        let no_speech = softmax_at(&logits[..width], self.vocab.no_speech as usize);
        let mut last = logits[(prompt.len() - 1) * width..prompt.len() * width].to_vec();
        let mut sampled: Vec<u32> = Vec::new();
        let mut sum_logprob = 0f32;
        while sampled.len() < MAX_SAMPLE && prompt.len() + sampled.len() < MAX_CTX {
            self.apply_rules(&mut last, &sampled);
            let (tok, _) =
                last.iter()
                    .enumerate()
                    .fold((0usize, f32::NEG_INFINITY), |b, (i, l)| {
                        if *l > b.1 {
                            (i, *l)
                        } else {
                            b
                        }
                    });
            sum_logprob += log_softmax_at(&last, tok);
            let tok = tok as u32;
            if tok == self.vocab.eot {
                break;
            }
            sampled.push(tok);
            if looping(&sampled) {
                break;
            }
            let (l, _) = self.step(&[tok], enc, &mut cache)?;
            last = l;
        }
        let avg = sum_logprob / (sampled.len() + 1) as f32;
        Ok((sampled, no_speech, avg))
    }

    /// Transcribes a PCM stream from the start. `progress` gets the number
    /// of samples consumed so far.
    pub fn transcribe(
        &mut self,
        pcm: &mut super::audio::PcmStream,
        lang: Option<&str>,
        progress: &dyn Fn(usize),
    ) -> anyhow::Result<(String, Vec<Segment>, bool)> {
        let mut lang_id = match lang
            .map(str::trim)
            .filter(|l| !l.is_empty() && *l != "auto")
        {
            Some(code) => Some(
                self.vocab
                    .language_id(code)
                    .ok_or_else(|| anyhow!("Whisper doesn't know the language `{code}`"))?,
            ),
            None => None,
        };
        let tsb = self.vocab.timestamp_begin;
        let mut seek = 0usize;
        let mut segments = Vec::new();
        let mut got_audio = false;
        loop {
            let samples = pcm.window(seek, WINDOW_SAMPLES)?;
            if samples.is_empty() {
                break;
            }
            got_audio = true;
            let win_len = samples.len();
            let final_window = win_len < WINDOW_SAMPLES;
            // A sliver under 0.2 s at the very end carries no words.
            if final_window && win_len < SAMPLE_RATE / 5 && seek > 0 {
                break;
            }
            let enc = self.encode(mel::log_mel_window(&samples))?;
            let lid = match lang_id {
                Some(l) => l,
                None => {
                    let l = self.detect_language(&enc)?;
                    lang_id = Some(l);
                    l
                }
            };
            let (tokens, no_speech, avg_logprob) = self.decode_window(&enc, lid)?;
            let offset_ms = (seek * 1000 / SAMPLE_RATE) as i64;
            let win_ms = (win_len * 1000 / SAMPLE_RATE) as i64;
            let silent = no_speech > NO_SPEECH_THRESHOLD && avg_logprob < LOGPROB_THRESHOLD;
            let mut advance = win_len;
            if !silent {
                let (segs, adv) = split_segments(&tokens, tsb, win_len);
                for (start, end, text_tokens) in segs {
                    let text = self.vocab.decode(&text_tokens);
                    if text.trim().is_empty() {
                        continue;
                    }
                    let end_ms = (end as i64 * 20).min(win_ms).max(start as i64 * 20);
                    segments.push(Segment {
                        start_ms: offset_ms + start as i64 * 20,
                        end_ms: offset_ms + end_ms,
                        text,
                    });
                }
                advance = adv;
            }
            seek += advance.max(TS_SAMPLES);
            progress(seek);
        }
        let code = lang_id
            .and_then(|id| self.vocab.language_code(id))
            .unwrap_or("")
            .to_string();
        Ok((code, segments, got_audio))
    }
}

/// Segments `(start_ts, end_ts, text tokens)` from one window's tokens, in
/// timestamp units (20 ms), and how many samples the next window starts
/// after — Whisper's `transcribe.py` rules.
fn split_segments(tokens: &[u32], tsb: u32, win_len: usize) -> (Vec<(u32, u32, Vec<u32>)>, usize) {
    let is_ts = |t: u32| t >= tsb;
    let single_ending =
        tokens.len() >= 2 && !is_ts(tokens[tokens.len() - 2]) && is_ts(tokens[tokens.len() - 1]);
    let mut consecutive: Vec<usize> = (1..tokens.len())
        .filter(|&i| is_ts(tokens[i]) && is_ts(tokens[i - 1]))
        .collect();
    let text_of = |s: &[u32]| s.iter().copied().filter(|t| !is_ts(*t)).collect::<Vec<_>>();
    let win_ts = (win_len / TS_SAMPLES) as u32;
    if !consecutive.is_empty() {
        if single_ending {
            consecutive.push(tokens.len());
        }
        let mut out = Vec::new();
        let mut last = 0usize;
        for cur in consecutive {
            let sliced = &tokens[last..cur];
            if let (Some(first), Some(end)) = (sliced.first(), sliced.last()) {
                let start = if is_ts(*first) { first - tsb } else { 0 };
                let end = if is_ts(*end) { end - tsb } else { win_ts };
                out.push((start, end, text_of(sliced)));
            }
            last = cur;
        }
        let advance = if single_ending {
            win_len
        } else {
            // Resume from the last complete timestamp.
            let t = tokens[last - 1].saturating_sub(tsb) as usize;
            (t * TS_SAMPLES).min(win_len)
        };
        (out, advance)
    } else {
        let start = match tokens.first() {
            Some(t) if is_ts(*t) => t - tsb,
            _ => 0,
        };
        let end = tokens
            .iter()
            .rev()
            .find(|t| is_ts(**t))
            .map(|t| t - tsb)
            .filter(|t| *t > start)
            .unwrap_or(win_ts);
        (vec![(start, end.max(start), text_of(tokens))], win_len)
    }
}

/// The tail repeats one short pattern four times: the decoder is stuck.
fn looping(tokens: &[u32]) -> bool {
    for k in 1..=16 {
        if tokens.len() < k * 4 {
            break;
        }
        let tail = &tokens[tokens.len() - k..];
        if (1..4).all(|r| {
            let s = tokens.len() - k * (r + 1);
            &tokens[s..s + k] == tail
        }) {
            return true;
        }
    }
    false
}

fn softmax_at(logits: &[f32], i: usize) -> f32 {
    log_softmax_at(logits, i).exp()
}

fn log_softmax_at(logits: &[f32], i: usize) -> f32 {
    let m = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if !m.is_finite() {
        return f32::NEG_INFINITY;
    }
    let sum: f32 = logits.iter().map(|x| (x - m).exp()).sum();
    logits.get(i).copied().unwrap_or(f32::NEG_INFINITY) - m - sum.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSB: u32 = 50364;

    #[test]
    fn paired_timestamps_become_segments_and_seek_to_the_last_one() {
        // <0.00> hi <1.00><1.00> there <2.00>
        let toks = [TSB, 10, TSB + 50, TSB + 50, 11, TSB + 100];
        let (segs, adv) = split_segments(&toks, TSB, WINDOW_SAMPLES);
        assert_eq!(segs.len(), 2);
        assert_eq!((segs[0].0, segs[0].1, segs[0].2.clone()), (0, 50, vec![10]));
        assert_eq!(
            (segs[1].0, segs[1].1, segs[1].2.clone()),
            (50, 100, vec![11])
        );
        // Single timestamp ending: the whole window is done.
        assert_eq!(adv, WINDOW_SAMPLES);

        // Ends on a pair: resume from it.
        let toks = [TSB, 10, TSB + 50, TSB + 50, 11];
        let (segs, adv) = split_segments(&toks, TSB, WINDOW_SAMPLES);
        assert_eq!(segs.len(), 1);
        assert_eq!(adv, 50 * TS_SAMPLES);
    }

    #[test]
    fn no_pairs_is_one_segment_for_the_window() {
        let toks = [TSB, 10, 11, TSB + 40];
        let (segs, adv) = split_segments(&toks, TSB, WINDOW_SAMPLES);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].0, 0);
        assert_eq!(segs[0].1, 40);
        assert_eq!(adv, WINDOW_SAMPLES);
    }

    #[test]
    fn a_stuck_decoder_is_caught() {
        assert!(looping(&[1, 2, 1, 2, 1, 2, 1, 2]));
        assert!(!looping(&[1, 2, 3, 4, 5, 6, 7, 8]));
    }
}
