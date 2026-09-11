//! XOR: aplicar uma chave, quebrar chave de um byte pela frequência das
//! letras, e achar o tamanho da chave de vários bytes pela distância de
//! Hamming — o caminho clássico do "repeating-key XOR".

use serde::{Deserialize, Serialize};

use super::analysis::english_score;

pub fn xor(data: &[u8], key: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct XorCandidate {
    pub key: String,
    pub key_hex: String,
    pub text: String,
    pub score: f64,
}

/// Testa as 256 chaves de um byte e ordena pela que mais parece texto.
pub fn brute_single(data: &[u8], top_n: usize) -> Vec<XorCandidate> {
    let mut out: Vec<XorCandidate> = (0..=255u8)
        .map(|k| {
            let plain = xor(data, &[k]);
            let text = String::from_utf8_lossy(&plain).to_string();
            XorCandidate {
                score: english_score(&text),
                key: if (0x20..0x7F).contains(&k) {
                    (k as char).to_string()
                } else {
                    format!("\\x{:02x}", k)
                },
                key_hex: format!("{:02x}", k),
                text,
            }
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(top_n.max(1));
    out
}

pub fn hamming(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

/// Tamanhos de chave prováveis, do mais provável para o menos.
///
/// A ideia: com o tamanho certo, dois blocos consecutivos foram cifrados com
/// a mesma chave, então a distância de Hamming normalizada entre eles cai
/// para a distância entre dois textos em inglês (~2-3 bits/byte) em vez da
/// de dois blocos aleatórios (~4).
pub fn guess_key_sizes(data: &[u8], max_size: usize, top_n: usize) -> Vec<(usize, f64)> {
    let mut scores: Vec<(usize, f64)> = Vec::new();
    for size in 2..=max_size.min(data.len() / 4).max(2) {
        let blocks: Vec<&[u8]> = data
            .chunks(size)
            .take(6)
            .filter(|c| c.len() == size)
            .collect();
        if blocks.len() < 2 {
            continue;
        }
        let mut total = 0f64;
        let mut pairs = 0f64;
        for i in 0..blocks.len() {
            for j in (i + 1)..blocks.len() {
                total += hamming(blocks[i], blocks[j]) as f64 / size as f64;
                pairs += 1.0;
            }
        }
        if pairs > 0.0 {
            scores.push((size, total / pairs));
        }
    }
    scores.sort_by(|a, b| a.1.total_cmp(&b.1));
    scores.truncate(top_n.max(1));
    scores
}

/// Quebra a chave inteira: descobre o tamanho, fatia por posição e roda a
/// força bruta de um byte em cada fatia.
pub fn break_repeating(data: &[u8], max_size: usize) -> Option<XorCandidate> {
    let sizes = guess_key_sizes(data, max_size, 3);
    let mut best: Option<XorCandidate> = None;
    for (size, _) in sizes {
        let mut key = Vec::with_capacity(size);
        for pos in 0..size {
            let slice: Vec<u8> = data.iter().skip(pos).step_by(size).copied().collect();
            let top = brute_single(&slice, 1);
            key.push(u8::from_str_radix(&top[0].key_hex, 16).unwrap_or(0));
        }
        let plain = xor(data, &key);
        let text = String::from_utf8_lossy(&plain).to_string();
        let score = english_score(&text);
        if best.as_ref().map(|b| score > b.score).unwrap_or(true) {
            best = Some(XorCandidate {
                key: String::from_utf8_lossy(&key).to_string(),
                key_hex: hex::encode(&key),
                text,
                score,
            });
        }
    }
    best
}

#[derive(Debug, Clone, Deserialize)]
pub struct XorOptions {
    /// Texto de entrada; hex quando `input_hex` for verdadeiro.
    pub input: String,
    #[serde(default)]
    pub input_hex: bool,
    /// "apply" | "single" | "repeating"
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Chave para o modo "apply" (texto, ou hex se `key_hex`).
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub key_hex: bool,
    #[serde(default = "default_max")]
    pub max_key_size: usize,
}

fn default_mode() -> String {
    "single".into()
}
fn default_max() -> usize {
    16
}

#[derive(Debug, Clone, Serialize)]
pub struct XorResult {
    pub candidates: Vec<XorCandidate>,
    pub key_sizes: Vec<(usize, f64)>,
}

pub fn run(opts: &XorOptions) -> anyhow::Result<XorResult> {
    let data = if opts.input_hex {
        hex::decode(opts.input.trim().replace([' ', '\n', ':'], ""))
            .map_err(|e| anyhow::anyhow!("hex inválido: {}", e))?
    } else {
        opts.input.as_bytes().to_vec()
    };
    Ok(match opts.mode.as_str() {
        "apply" => {
            let key = if opts.key_hex {
                hex::decode(opts.key.trim()).map_err(|e| anyhow::anyhow!("chave hex: {}", e))?
            } else {
                opts.key.as_bytes().to_vec()
            };
            let out = xor(&data, &key);
            XorResult {
                candidates: vec![XorCandidate {
                    key: opts.key.clone(),
                    key_hex: hex::encode(&key),
                    text: String::from_utf8_lossy(&out).to_string(),
                    score: 0.0,
                }],
                key_sizes: vec![],
            }
        }
        "repeating" => XorResult {
            key_sizes: guess_key_sizes(&data, opts.max_key_size, 5),
            candidates: break_repeating(&data, opts.max_key_size)
                .into_iter()
                .collect(),
        },
        _ => XorResult {
            candidates: brute_single(&data, 8),
            key_sizes: vec![],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_is_its_own_inverse() {
        let data = b"mensagem secreta";
        let key = b"chave";
        assert_eq!(xor(&xor(data, key), key), data);
        assert_eq!(xor(data, b""), data, "sem chave, nada muda");
    }

    #[test]
    fn hamming_matches_the_cryptopals_example() {
        assert_eq!(hamming(b"this is a test", b"wokka wokka!!!"), 37);
    }

    #[test]
    fn single_byte_key_is_recovered() {
        let plain = "the quick brown fox jumps over the lazy dog, again and again";
        let enc = xor(plain.as_bytes(), &[0x42]);
        let top = brute_single(&enc, 3);
        assert_eq!(top[0].key_hex, "42");
        assert_eq!(top[0].text, plain);
    }

    #[test]
    fn repeating_key_is_recovered() {
        let plain = "the quick brown fox jumps over the lazy dog while the cat sleeps \
                     under the warm sun and nothing else happens in this small town today";
        let enc = xor(plain.as_bytes(), b"omni");
        let best = break_repeating(&enc, 12).unwrap();
        assert_eq!(best.key, "omni", "chave achada: {:?}", best.key);
        assert_eq!(best.text, plain);
    }

    #[test]
    fn key_size_guess_ranks_the_real_one_high() {
        let plain = "attack at dawn from the north side of the river when the fog lifts \
                     and the guards change shift at exactly six in the morning sharp";
        let enc = xor(plain.as_bytes(), b"key12");
        let sizes = guess_key_sizes(&enc, 12, 5);
        assert!(
            sizes.iter().take(3).any(|(s, _)| *s == 5),
            "5 tinha que estar no topo: {:?}",
            sizes
        );
    }

    #[test]
    fn hex_input_is_accepted() {
        let opts = XorOptions {
            input: "48 65 6c 6c 6f".into(),
            input_hex: true,
            mode: "apply".into(),
            key: "00".into(),
            key_hex: true,
            max_key_size: 8,
        };
        let r = run(&opts).unwrap();
        assert_eq!(r.candidates[0].text, "Hello");
    }

    #[test]
    fn bad_hex_is_an_error_not_a_panic() {
        let opts = XorOptions {
            input: "zznotahex".into(),
            input_hex: true,
            mode: "single".into(),
            key: String::new(),
            key_hex: false,
            max_key_size: 8,
        };
        assert!(run(&opts).is_err());
    }
}
