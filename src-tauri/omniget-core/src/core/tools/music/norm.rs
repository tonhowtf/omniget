//! Normalização de artista/título e distância de edição. É o miolo do
//! casamento entre uma playlist exportada e os arquivos que estão no disco:
//! o mesmo álbum aparece como "Song (Remastered 2011)" no Spotify e como
//! "01 - Artist - Song.mp3" na pasta, e os dois precisam virar a mesma chave.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

/// Trechos entre parênteses/colchetes ou depois de " - " que só descrevem a
/// edição da faixa, não a faixa. "live" fica de fora de propósito: descartar
/// isso casaria a versão de estúdio com a ao vivo, que é um erro visível.
const NOISE: &[&str] = &[
    "remaster",
    "remastered",
    "remasterizado",
    "remasterizada",
    "remastering",
    "radio edit",
    "radio version",
    "radio mix",
    "single version",
    "single edit",
    "album version",
    "original version",
    "original mix",
    "extended mix",
    "extended version",
    "club mix",
    "deluxe",
    "bonus track",
    "bonus",
    "mono version",
    "stereo version",
    "explicit",
    "clean version",
    "digital remaster",
    "re recorded",
    "rerecorded",
    "anniversary edition",
    "edit",
];

static PAREN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*[\(\[]([^\(\)\[\]]*)[\)\]]").unwrap());
static FEAT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s+(feat\.?|ft\.?|featuring)\s+.*$").unwrap());
static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static TRACK_NUM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\d{1,3}\s*[-._)\]]\s*").unwrap());
static TRACK_NUM_LOOSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\d{1,3}\s+").unwrap());

/// Tira acentos decompondo em NFD e jogando fora as marcas combinantes.
pub fn strip_accents(s: &str) -> String {
    s.nfd()
        .filter(|c| !matches!(*c as u32, 0x0300..=0x036F))
        .collect()
}

fn contains_word(hay: &str, needle: &str) -> bool {
    let hay_tokens: Vec<&str> = hay.split_whitespace().collect();
    let needle_tokens: Vec<&str> = needle.split_whitespace().collect();
    if needle_tokens.is_empty() || needle_tokens.len() > hay_tokens.len() {
        return false;
    }
    hay_tokens
        .windows(needle_tokens.len())
        .any(|w| w == needle_tokens.as_slice())
}

/// Um trecho é descartável quando é crédito de participação ou fala só da
/// edição da faixa.
pub fn is_noise_chunk(chunk: &str) -> bool {
    let c = simplify(chunk);
    if c.is_empty() {
        return true;
    }
    if c.starts_with("feat ") || c.starts_with("ft ") || c.starts_with("featuring ") {
        return true;
    }
    NOISE.iter().any(|n| contains_word(&c, n))
}

/// Minúsculas, sem acento, sem pontuação, espaços colapsados. Não mexe na
/// estrutura do título — quem faz isso é `clean_title`.
pub fn simplify(s: &str) -> String {
    let s = strip_accents(s).to_lowercase();
    let s = s.replace('&', " and ").replace('’', "'");
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
        } else {
            out.push(' ');
        }
    }
    WS_RE.replace_all(out.trim(), " ").to_string()
}

/// Tira "(Remastered 2011)", "- Radio Edit" e "feat. Fulano" de um título,
/// preservando o texto original (ainda com maiúsculas e acento).
pub fn clean_title(title: &str) -> String {
    // 1. parênteses e colchetes descartáveis, quantos houver.
    let mut cur = title.to_string();
    loop {
        let mut changed = false;
        let mut next = String::new();
        let mut last = 0usize;
        for m in PAREN_RE.captures_iter(&cur) {
            let whole = m.get(0).map(|g| (g.start(), g.end()));
            let inner = m.get(1).map(|g| g.as_str()).unwrap_or("");
            if let Some((s, e)) = whole {
                if is_noise_chunk(inner) {
                    next.push_str(&cur[last..s]);
                    last = e;
                    changed = true;
                }
            }
        }
        next.push_str(&cur[last..]);
        cur = next;
        if !changed {
            break;
        }
    }

    // 2. sufixos depois de traço.
    let normalized_dash = cur.replace(" – ", " - ").replace(" — ", " - ");
    let parts: Vec<&str> = normalized_dash.split(" - ").collect();
    let mut kept: Vec<&str> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 || !is_noise_chunk(part) {
            kept.push(part);
        }
    }
    cur = kept.join(" - ");

    // 3. "feat. X" solto, sem parênteses.
    cur = FEAT_RE.replace(&cur, "").to_string();

    WS_RE.replace_all(cur.trim(), " ").to_string()
}

/// Chave de comparação de um título: limpa e simplifica.
pub fn norm_title(title: &str) -> String {
    simplify(&clean_title(title))
}

/// Chave de comparação de um artista. "Artist feat. Outro" e
/// "Artist, Outro" viram só o principal, que é o que aparece no nome do
/// arquivo na maior parte das vezes.
pub fn norm_artist(artist: &str) -> String {
    let a = FEAT_RE.replace(artist, "").to_string();
    let a = PAREN_RE.replace_all(&a, "").to_string();
    simplify(&a)
}

/// Só o primeiro artista de uma lista ("A, B & C" -> "A").
pub fn primary_artist(artist: &str) -> String {
    let a = FEAT_RE.replace(artist, "").to_string();
    let a = PAREN_RE.replace_all(&a, "").to_string();
    // O corte vem antes de `simplify`, senão a vírgula já teria sumido.
    let lower = a.to_lowercase();
    let mut cut = lower.chars().count();
    for sep in [",", ";", "/", " & ", " and ", " x ", " vs "] {
        if let Some(i) = lower.find(sep) {
            cut = cut.min(lower[..i].chars().count());
        }
    }
    let head: String = a.chars().take(cut).collect();
    simplify(&head)
}

/// Tira "01 - ", "03. " e afins do começo do nome de arquivo.
pub fn strip_track_number(stem: &str) -> String {
    TRACK_NUM_RE.replace(stem, "").to_string()
}

/// Igual ao anterior, mas aceita "01 Song" sem separador. Usar só como
/// candidato extra: um título como "99 Problems" perderia o número aqui.
pub fn strip_track_number_loose(stem: &str) -> String {
    TRACK_NUM_LOOSE_RE.replace(stem, "").to_string()
}

/// Distância de Levenshtein em caracteres (não em bytes).
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Semelhança de 0 a 100. Igual dá 100; string contida na outra ganha um
/// piso proporcional, para "song" achar "song extended".
pub fn similarity(a: &str, b: &str) -> u32 {
    if a.is_empty() || b.is_empty() {
        return u32::from(a == b) * 100;
    }
    if a == b {
        return 100;
    }
    let la = a.chars().count();
    let lb = b.chars().count();
    let max = la.max(lb);
    let d = levenshtein(a, b);
    let mut score = (((max - d) as f64 / max as f64) * 100.0).round() as u32;
    if a.contains(b) || b.contains(a) {
        let floor = ((la.min(lb) as f64 / max as f64) * 100.0).round() as u32;
        score = score.max(floor);
    }
    score.min(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limpa_remastered_entre_parenteses() {
        assert_eq!(
            clean_title("Come Together (Remastered 2009)"),
            "Come Together"
        );
        assert_eq!(
            clean_title("Bohemian Rhapsody [2011 Remaster]"),
            "Bohemian Rhapsody"
        );
    }

    #[test]
    fn limpa_sufixo_depois_de_traco() {
        assert_eq!(clean_title("Bad Guy - Radio Edit"), "Bad Guy");
        assert_eq!(
            clean_title("Californication - Remastered 2011"),
            "Californication"
        );
        // sufixo que não é ruído continua no título.
        assert_eq!(
            clean_title("Wish You Were Here - Part 2"),
            "Wish You Were Here - Part 2"
        );
    }

    #[test]
    fn limpa_feat_com_e_sem_parenteses() {
        assert_eq!(clean_title("Stay (feat. Justin Bieber)"), "Stay");
        assert_eq!(clean_title("Sunflower feat. Swae Lee"), "Sunflower");
        assert_eq!(clean_title("Lean On ft. MØ"), "Lean On");
    }

    #[test]
    fn ao_vivo_nao_e_ruido() {
        assert_eq!(
            clean_title("Hey Jude - Live at Wembley"),
            "Hey Jude - Live at Wembley"
        );
    }

    #[test]
    fn simplify_tira_acento_e_pontuacao() {
        assert_eq!(simplify("Não Vou Me Adaptar!"), "nao vou me adaptar");
        assert_eq!(simplify("AC/DC"), "ac dc");
        assert_eq!(simplify("Simon & Garfunkel"), "simon and garfunkel");
    }

    #[test]
    fn norm_title_junta_tudo() {
        assert_eq!(
            norm_title("Águas de Março (Remastered 2011)"),
            "aguas de marco"
        );
    }

    #[test]
    fn artista_principal() {
        assert_eq!(primary_artist("Post Malone, Swae Lee"), "post malone");
        assert_eq!(primary_artist("Drake feat. Rihanna"), "drake");
        assert_eq!(primary_artist("Simon & Garfunkel"), "simon");
        assert_eq!(primary_artist("Beyoncé"), "beyonce");
        assert_eq!(norm_artist("Beyoncé"), "beyonce");
    }

    #[test]
    fn tira_numero_de_faixa() {
        assert_eq!(strip_track_number("01 - Song"), "Song");
        assert_eq!(strip_track_number("03. Song"), "Song");
        assert_eq!(strip_track_number("2001 A Space"), "2001 A Space");
        assert_eq!(strip_track_number("01 Song"), "01 Song");
        assert_eq!(strip_track_number_loose("01 Song"), "Song");
    }

    #[test]
    fn levenshtein_conhecido() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("coração", "coracao"), 2);
    }

    #[test]
    fn similaridade_e_limiar() {
        assert_eq!(similarity("bad guy", "bad guy"), 100);
        assert!(similarity("bad guy", "bad guy ") >= 87);
        assert!(similarity("imagine dragons believer", "imagine dragons beleiver") >= 90);
        assert!(similarity("bad guy", "good girl") < 60);
        // contida: piso proporcional.
        assert!(similarity("song", "song extended") >= 30);
    }
}
