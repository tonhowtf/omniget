//! Cifras clássicas: César/ROT, Atbash, Vigenère e Rail Fence. Todas
//! preservam pontuação e caixa, e todas têm o caminho de volta.

use serde::{Deserialize, Serialize};

use super::analysis::english_score;

#[derive(Debug, Clone, Deserialize)]
pub struct CipherOptions {
    pub text: String,
    /// "caesar" | "atbash" | "vigenere" | "railfence"
    pub cipher: String,
    #[serde(default)]
    pub decrypt: bool,
    /// Deslocamento do César ou número de trilhos do Rail Fence.
    #[serde(default)]
    pub shift: i32,
    /// Chave do Vigenère.
    #[serde(default)]
    pub key: String,
}

pub fn caesar(text: &str, shift: i32) -> String {
    let s = shift.rem_euclid(26) as u8;
    text.chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                (b'a' + (c as u8 - b'a' + s) % 26) as char
            } else if c.is_ascii_uppercase() {
                (b'A' + (c as u8 - b'A' + s) % 26) as char
            } else {
                c
            }
        })
        .collect()
}

pub fn atbash(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                (b'z' - (c as u8 - b'a')) as char
            } else if c.is_ascii_uppercase() {
                (b'Z' - (c as u8 - b'A')) as char
            } else {
                c
            }
        })
        .collect()
}

pub fn vigenere(text: &str, key: &str, decrypt: bool) -> String {
    let k: Vec<u8> = key
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase() as u8 - b'a')
        .collect();
    if k.is_empty() {
        return text.to_string();
    }
    let mut i = 0usize;
    text.chars()
        .map(|c| {
            if !c.is_ascii_alphabetic() {
                return c;
            }
            let base = if c.is_ascii_lowercase() { b'a' } else { b'A' };
            let shift = k[i % k.len()] as i32;
            i += 1;
            let shift = if decrypt { -shift } else { shift };
            (base + ((c as u8 - base) as i32 + shift).rem_euclid(26) as u8) as char
        })
        .collect()
}

pub fn rail_fence(text: &str, rails: usize, decrypt: bool) -> String {
    let rails = rails.clamp(2, text.chars().count().max(2));
    let chars: Vec<char> = text.chars().collect();
    // Padrão de zigue-zague: qual trilho cada posição ocupa.
    let mut pattern = Vec::with_capacity(chars.len());
    let (mut r, mut dir) = (0isize, 1isize);
    for _ in 0..chars.len() {
        pattern.push(r as usize);
        if r == 0 {
            dir = 1;
        } else if r as usize == rails - 1 {
            dir = -1;
        }
        r += dir;
    }
    if !decrypt {
        let mut out = String::with_capacity(chars.len());
        for rail in 0..rails {
            for (i, c) in chars.iter().enumerate() {
                if pattern[i] == rail {
                    out.push(*c);
                }
            }
        }
        out
    } else {
        let mut out = vec![' '; chars.len()];
        let mut it = chars.iter();
        for rail in 0..rails {
            for (i, slot) in out.iter_mut().enumerate() {
                if pattern[i] == rail {
                    if let Some(c) = it.next() {
                        *slot = *c;
                    }
                }
            }
        }
        out.into_iter().collect()
    }
}

pub fn apply(opts: &CipherOptions) -> String {
    match opts.cipher.as_str() {
        "atbash" => atbash(&opts.text),
        "vigenere" => vigenere(&opts.text, &opts.key, opts.decrypt),
        "railfence" => rail_fence(&opts.text, opts.shift.max(2) as usize, opts.decrypt),
        _ => caesar(
            &opts.text,
            if opts.decrypt {
                -opts.shift
            } else {
                opts.shift
            },
        ),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CaesarCandidate {
    pub shift: i32,
    pub text: String,
    pub score: f64,
}

/// As 26 rotações, ordenadas pela que mais parece texto de verdade.
pub fn caesar_bruteforce(text: &str) -> Vec<CaesarCandidate> {
    let mut out: Vec<CaesarCandidate> = (0..26)
        .map(|shift| {
            let text = caesar(text, -shift);
            CaesarCandidate {
                score: english_score(&text),
                shift,
                text,
            }
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caesar_round_trips_and_keeps_punctuation() {
        let enc = caesar("Ataque ao amanhecer, as 5h!", 3);
        assert_eq!(enc, "Dwdtxh dr dpdqkhfhu, dv 5k!");
        assert_eq!(caesar(&enc, -3), "Ataque ao amanhecer, as 5h!");
    }

    #[test]
    fn rot13_is_its_own_inverse() {
        let t = "Hello, World!";
        assert_eq!(caesar(&caesar(t, 13), 13), t);
    }

    #[test]
    fn atbash_is_its_own_inverse() {
        let t = "Attack at dawn";
        assert_eq!(atbash("abc"), "zyx");
        assert_eq!(atbash(&atbash(t)), t);
    }

    #[test]
    fn vigenere_round_trips() {
        let enc = vigenere("ATTACKATDAWN", "LEMON", false);
        assert_eq!(enc, "LXFOPVEFRNHR", "vetor clássico do Vigenère");
        assert_eq!(vigenere(&enc, "LEMON", true), "ATTACKATDAWN");
    }

    #[test]
    fn vigenere_skips_non_letters_without_burning_the_key() {
        let enc = vigenere("AT TACK", "LEMON", false);
        assert_eq!(enc, "LX FOPV", "o espaço não pode consumir letra da chave");
    }

    #[test]
    fn vigenere_without_a_key_is_a_no_op() {
        assert_eq!(vigenere("teste", "123!", false), "teste");
    }

    #[test]
    fn rail_fence_round_trips() {
        let enc = rail_fence("WEAREDISCOVEREDFLEEATONCE", 3, false);
        assert_eq!(enc, "WECRLTEERDSOEEFEAOCAIVDEN");
        assert_eq!(rail_fence(&enc, 3, true), "WEAREDISCOVEREDFLEEATONCE");
    }

    #[test]
    fn bruteforce_puts_the_real_text_on_top() {
        let secret = "the quick brown fox jumps over the lazy dog and then some more";
        let enc = caesar(secret, 7);
        let best = &caesar_bruteforce(&enc)[0];
        assert_eq!(best.shift, 7, "achou o deslocamento");
        assert_eq!(best.text, secret);
    }
}
