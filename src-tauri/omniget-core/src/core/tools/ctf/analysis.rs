//! Análise de frequência: histograma, índice de coincidência e a pontuação
//! que diz "isso aqui parece texto" — é o que faz a força bruta escolher
//! sozinha a resposta certa em vez de despejar 256 alternativas.

use serde::Serialize;

/// Frequência das letras em inglês, em porcentagem, na ordem A-Z.
const ENGLISH_FREQ: [f64; 26] = [
    8.17, 1.49, 2.78, 4.25, 12.70, 2.23, 2.02, 6.09, 6.97, 0.15, 0.77, 4.03, 2.41, 6.75, 7.51,
    1.93, 0.10, 5.99, 6.33, 9.06, 2.76, 0.98, 2.36, 0.15, 1.97, 0.07,
];

/// Quanto o texto se parece com linguagem natural. Quanto maior, melhor.
///
/// Qui-quadrado invertido contra a frequência do inglês, com desconto pesado
/// para byte não imprimível — sem isso a força bruta de XOR escolhe lixo
/// binário que por acaso tem letras.
pub fn english_score(text: &str) -> f64 {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return f64::MIN;
    }
    let mut counts = [0f64; 26];
    let (mut letters, mut printable, mut total) = (0f64, 0f64, 0f64);
    for b in bytes {
        total += 1.0;
        if b.is_ascii_alphabetic() {
            counts[(b.to_ascii_lowercase() - b'a') as usize] += 1.0;
            letters += 1.0;
        }
        if (0x20..0x7F).contains(b) || *b == b'\n' || *b == b'\t' || *b == b'\r' {
            printable += 1.0;
        }
    }
    if letters == 0.0 {
        return f64::MIN;
    }
    let chi: f64 = (0..26)
        .map(|i| {
            let expected = ENGLISH_FREQ[i] / 100.0 * letters;
            let diff = counts[i] - expected;
            diff * diff / expected.max(0.5)
        })
        .sum();
    let printable_ratio = printable / total;
    let space_ratio = bytes.iter().filter(|b| **b == b' ').count() as f64 / total;
    // Texto de verdade tem ~15% de espaço; lixo binário não tem nenhum.
    -chi / letters + printable_ratio * 20.0 + (space_ratio * 10.0).min(2.0)
}

#[derive(Debug, Clone, Serialize)]
pub struct FreqEntry {
    pub byte: u8,
    pub display: String,
    pub count: u64,
    pub percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FreqReport {
    pub total: u64,
    pub distinct: usize,
    pub entropy: f64,
    /// Índice de coincidência. ~0,067 = inglês (ou cifra de substituição
    /// simples); ~0,038 = aleatório ou Vigenère com chave longa.
    pub index_of_coincidence: f64,
    pub top: Vec<FreqEntry>,
    pub looks_like: String,
}

pub fn index_of_coincidence(text: &str) -> f64 {
    let letters: Vec<u8> = text
        .bytes()
        .filter(|b| b.is_ascii_alphabetic())
        .map(|b| b.to_ascii_lowercase())
        .collect();
    let n = letters.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mut counts = [0f64; 26];
    for b in &letters {
        counts[(*b - b'a') as usize] += 1.0;
    }
    counts.iter().map(|c| c * (c - 1.0)).sum::<f64>() / (n * (n - 1.0))
}

pub fn analyze(data: &[u8], top_n: usize) -> FreqReport {
    let mut counts = [0u64; 256];
    for b in data {
        counts[*b as usize] += 1;
    }
    let total = data.len() as u64;
    let mut top: Vec<FreqEntry> = counts
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(b, c)| FreqEntry {
            byte: b as u8,
            display: if (0x20..0x7F).contains(&(b as u8)) {
                (b as u8 as char).to_string()
            } else {
                format!("\\x{:02x}", b)
            },
            count: *c,
            percent: if total > 0 {
                *c as f64 / total as f64 * 100.0
            } else {
                0.0
            },
        })
        .collect();
    let distinct = top.len();
    top.sort_by_key(|e| std::cmp::Reverse(e.count));
    top.truncate(top_n.max(1));

    let text = String::from_utf8_lossy(data);
    let ic = index_of_coincidence(&text);
    let entropy = super::magic::entropy(data);
    let looks_like = if entropy > 7.5 {
        "comprimido ou cifrado".into()
    } else if ic > 0.06 {
        "texto natural ou substituição simples".into()
    } else if ic > 0.045 {
        "texto com alfabeto misturado".into()
    } else if distinct <= 20 {
        "alfabeto pequeno — talvez codificação (base32/64, hex)".into()
    } else {
        "aleatório, Vigenère de chave longa ou binário".into()
    };

    FreqReport {
        total,
        distinct,
        entropy,
        index_of_coincidence: ic,
        top,
        looks_like,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_text_scores_above_noise() {
        let real = english_score("the quick brown fox jumps over the lazy dog");
        let noise = english_score("zzqxjkvwzzqxjkvwzzqxjkvw");
        assert!(real > noise, "real {} vs ruído {}", real, noise);
    }

    #[test]
    fn binary_junk_scores_worse_than_text() {
        let text = english_score("hello world this is a normal sentence");
        let junk = english_score("h\u{1}e\u{2}l\u{3}l\u{4}o\u{5}w\u{6}o\u{7}r\u{8}l\u{9}d");
        assert!(text > junk, "texto {} vs lixo {}", text, junk);
    }

    #[test]
    fn empty_and_letterless_are_worst() {
        assert_eq!(english_score(""), f64::MIN);
        assert_eq!(english_score("12345 67890"), f64::MIN);
    }

    #[test]
    fn ioc_separates_english_from_random() {
        let english = "the index of coincidence is a statistic used in cryptanalysis of text";
        let ic = index_of_coincidence(english);
        assert!((0.055..0.085).contains(&ic), "IC do inglês foi {}", ic);
        let flat = "abcdefghijklmnopqrstuvwxyz".repeat(8);
        assert!(index_of_coincidence(&flat) < 0.045, "alfabeto plano");
    }

    #[test]
    fn ioc_needs_two_letters() {
        assert_eq!(index_of_coincidence("a"), 0.0);
        assert_eq!(index_of_coincidence("!!!"), 0.0);
    }

    #[test]
    fn report_calls_out_what_it_sees() {
        // Frase comum, não pangrama: pangrama tem distribuição plana de
        // propósito e derruba o índice de coincidência.
        let text = analyze(
            b"the index of coincidence is a statistic used in the analysis of text",
            5,
        );
        assert!(
            text.index_of_coincidence > 0.05,
            "IC ficou {}",
            text.index_of_coincidence
        );
        assert_eq!(text.top[0].display, " ", "espaço é o byte mais comum");
        assert!(text.looks_like.contains("texto"));

        let random: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let r = analyze(&random, 5);
        assert_eq!(r.distinct, 256);
        assert!(r.entropy > 7.9);
        assert!(r.looks_like.contains("cifrado"));
    }
}
