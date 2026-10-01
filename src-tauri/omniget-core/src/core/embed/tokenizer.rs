use std::collections::HashMap;

pub const MODEL_MAX_LEN: usize = 512;

pub const DEFAULT_MAX_LEN: usize = 256;

const UNK: &str = "[UNK]";
const CLS: &str = "[CLS]";
const SEP: &str = "[SEP]";
const PAD: &str = "[PAD]";

const MAX_CHARS_PER_WORD: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoding {
    pub ids: Vec<u32>,
    pub attention_mask: Vec<u32>,
    pub type_ids: Vec<u32>,
}

impl Encoding {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct Tokenizer {
    vocab: HashMap<String, u32>,
    unk: u32,
    cls: u32,
    sep: u32,
    pad: u32,
    max_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VocabError {
    Empty,
    MissingSpecial(&'static str),
}

impl std::fmt::Display for VocabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VocabError::Empty => write!(f, "the vocabulary file is empty"),
            VocabError::MissingSpecial(t) => write!(f, "the vocabulary has no {t} token"),
        }
    }
}

impl std::error::Error for VocabError {}

impl Tokenizer {
    pub fn from_vocab_text(text: &str) -> Result<Self, VocabError> {
        let mut vocab: HashMap<String, u32> = HashMap::with_capacity(32_000);
        for (i, token) in text.lines().enumerate() {
            vocab.entry(token.to_string()).or_insert(i as u32);
        }
        if vocab.is_empty() {
            return Err(VocabError::Empty);
        }
        let get = |t: &'static str| vocab.get(t).copied().ok_or(VocabError::MissingSpecial(t));
        Ok(Self {
            unk: get(UNK)?,
            cls: get(CLS)?,
            sep: get(SEP)?,
            pad: get(PAD)?,
            vocab,
            max_len: DEFAULT_MAX_LEN,
        })
    }

    pub fn with_max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len.clamp(2, MODEL_MAX_LEN);
        self
    }

    pub fn max_len(&self) -> usize {
        self.max_len
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    pub fn pad_id(&self) -> u32 {
        self.pad
    }

    pub fn token_id(&self, token: &str) -> Option<u32> {
        self.vocab.get(token).copied()
    }

    pub fn pieces(&self, text: &str) -> Vec<String> {
        basic_tokenize(text)
            .iter()
            .flat_map(|w| self.wordpiece(w))
            .collect()
    }

    pub fn encode(&self, text: &str) -> Encoding {
        let mut ids = Vec::with_capacity(16);
        ids.push(self.cls);
        let room = self.max_len.saturating_sub(2);
        for piece in self.pieces(text) {
            if ids.len() > room {
                break;
            }
            ids.push(self.vocab.get(&piece).copied().unwrap_or(self.unk));
        }
        ids.push(self.sep);
        let n = ids.len();
        Encoding {
            ids,
            attention_mask: vec![1; n],
            type_ids: vec![0; n],
        }
    }

    pub fn encode_batch(&self, texts: &[&str]) -> Vec<Encoding> {
        let mut out: Vec<Encoding> = texts.iter().map(|t| self.encode(t)).collect();
        let width = out.iter().map(|e| e.len()).max().unwrap_or(0);
        for e in &mut out {
            while e.ids.len() < width {
                e.ids.push(self.pad);
                e.attention_mask.push(0);
                e.type_ids.push(0);
            }
        }
        out
    }

    fn wordpiece(&self, word: &str) -> Vec<String> {
        let chars: Vec<char> = word.chars().collect();
        if chars.is_empty() {
            return vec![];
        }
        if chars.len() > MAX_CHARS_PER_WORD {
            return vec![UNK.to_string()];
        }
        let mut out = Vec::new();
        let mut start = 0usize;
        while start < chars.len() {
            let mut end = chars.len();
            let mut found: Option<String> = None;
            while start < end {
                let mut piece: String = chars[start..end].iter().collect();
                if start > 0 {
                    piece.insert_str(0, "##");
                }
                if self.vocab.contains_key(&piece) {
                    found = Some(piece);
                    break;
                }
                end -= 1;
            }
            match found {
                Some(piece) => {
                    out.push(piece);
                    start = end;
                }
                None => return vec![UNK.to_string()],
            }
        }
        out
    }
}

fn is_control(c: char) -> bool {
    if c == '\t' || c == '\n' || c == '\r' {
        return false;
    }
    c.is_control() || c == '\u{0}' || c == '\u{fffd}'
}

fn is_whitespace(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n' || c == '\r' || c.is_whitespace()
}

pub(crate) fn is_cjk(c: char) -> bool {
    let cp = c as u32;
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0x20000..=0x2A6DF).contains(&cp)
        || (0x2A700..=0x2B73F).contains(&cp)
        || (0x2B740..=0x2B81F).contains(&cp)
        || (0x2B820..=0x2CEAF).contains(&cp)
        || (0x2F800..=0x2FA1F).contains(&cp)
}

fn is_splittable_punct(c: char) -> bool {
    !c.is_alphanumeric() && !is_whitespace(c) && !is_cjk(c)
}

pub fn basic_tokenize(text: &str) -> Vec<String> {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;

    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let flush = |cur: &mut String, words: &mut Vec<String>| {
        if !cur.is_empty() {
            words.push(std::mem::take(cur));
        }
    };

    for raw in text.chars() {
        if is_control(raw) {
            continue;
        }
        if is_whitespace(raw) {
            flush(&mut current, &mut words);
            continue;
        }
        if is_cjk(raw) {
            flush(&mut current, &mut words);
            words.push(raw.to_string());
            continue;
        }
        if is_splittable_punct(raw) {
            flush(&mut current, &mut words);
            words.push(raw.to_string());
            continue;
        }
        for lower in raw.to_lowercase() {
            for d in lower.nfd() {
                if !is_combining_mark(d) {
                    current.push(d);
                }
            }
        }
    }
    flush(&mut current, &mut words);
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOCAB: &str = include_str!("../../../tests/embed_fixtures/vocab.txt");

    fn tk() -> Tokenizer {
        Tokenizer::from_vocab_text(VOCAB).expect("vocab")
    }

    #[test]
    fn o_vocab_da_fixture_tem_o_tamanho_do_bert_uncased() {
        let t = tk();
        assert_eq!(t.vocab_size(), 30_522);
        assert_eq!(t.pad_id(), 0);
        assert_eq!(t.token_id(UNK), Some(100));
        assert_eq!(t.token_id(CLS), Some(101));
        assert_eq!(t.token_id(SEP), Some(102));
    }

    #[test]
    fn vocab_vazio_ou_sem_especiais_da_erro() {
        assert_eq!(
            Tokenizer::from_vocab_text("").err(),
            Some(VocabError::Empty)
        );
        assert_eq!(
            Tokenizer::from_vocab_text("olá\nmundo\n").err(),
            Some(VocabError::MissingSpecial(UNK))
        );
    }

    #[test]
    fn hello_world_da_os_ids_conhecidos_do_bert() {
        let e = tk().encode("Hello, world!");
        assert_eq!(e.ids, vec![101, 7592, 1010, 2088, 999, 102]);
        assert_eq!(e.attention_mask, vec![1; 6]);
        assert_eq!(e.type_ids, vec![0; 6]);
    }

    #[test]
    fn os_ids_batem_com_a_linha_do_vocab() {
        let t = tk();
        let linhas: Vec<&str> = VOCAB.lines().collect();
        for id in t.encode("the quick brown fox jumps over the lazy dog").ids {
            assert!(!linhas[id as usize].is_empty());
        }
        assert_eq!(linhas[7592], "hello");
        assert_eq!(linhas[2088], "world");
    }

    #[test]
    fn palavra_desconhecida_vira_pecas_com_hashtag() {
        assert_eq!(
            tk().pieces("unaffable"),
            vec!["una", "##ffa", "##ble"],
            "WordPiece guloso da esquerda para a direita"
        );
        assert_eq!(tk().pieces("OmniGet"), vec!["om", "##nig", "##et"]);
    }

    #[test]
    fn acento_e_caixa_somem_na_normalizacao() {
        assert_eq!(basic_tokenize("Café ORAÇÃO"), vec!["cafe", "oracao"]);
        assert_eq!(tk().pieces("Café"), tk().pieces("cafe"));
    }

    #[test]
    fn pontuacao_vira_token_proprio() {
        assert_eq!(
            basic_tokenize("a,b. c-d!"),
            vec!["a", ",", "b", ".", "c", "-", "d", "!"]
        );
    }

    #[test]
    fn ideograma_cjk_e_um_token_por_caractere() {
        assert_eq!(basic_tokenize("下载视频"), vec!["下", "载", "视", "频"]);
        assert!(is_cjk('下') && !is_cjk('a') && !is_cjk('ã'));
    }

    #[test]
    fn controle_some_e_espaco_qualquer_separa() {
        assert_eq!(basic_tokenize("a\u{0}b\tc\u{a0}d"), vec!["ab", "c", "d"]);
    }

    #[test]
    fn emoji_vira_unk_por_caractere() {
        let p = tk().pieces("hello 🚀🚀");
        assert_eq!(p, vec!["hello", UNK, UNK]);
    }

    #[test]
    fn palavra_gigante_vira_unk_inteira() {
        let longa = "a".repeat(MAX_CHARS_PER_WORD + 1);
        assert_eq!(tk().pieces(&longa), vec![UNK]);
        let no_limite = "a".repeat(MAX_CHARS_PER_WORD);
        assert_ne!(tk().pieces(&no_limite), vec![UNK]);
    }

    #[test]
    fn texto_vazio_e_so_cls_sep() {
        let e = tk().encode("   \n\t ");
        assert_eq!(e.ids, vec![101, 102]);
    }

    #[test]
    fn trunca_no_max_len_com_cls_e_sep() {
        let t = tk().with_max_len(8);
        let e = t.encode(&"palavra ".repeat(50));
        assert_eq!(e.len(), 8);
        assert_eq!(e.ids[0], 101);
        assert_eq!(*e.ids.last().unwrap(), 102);
    }

    #[test]
    fn max_len_fica_dentro_do_modelo() {
        assert_eq!(tk().with_max_len(99_999).max_len(), MODEL_MAX_LEN);
        assert_eq!(tk().with_max_len(0).max_len(), 2);
        assert_eq!(tk().max_len(), DEFAULT_MAX_LEN);
    }

    #[test]
    fn lote_sai_retangular_com_mascara_zerada_no_padding() {
        let t = tk();
        let batch = t.encode_batch(&["hello", "uma frase bem mais longa que a outra"]);
        let width = batch[0].len();
        assert!(batch.iter().all(|e| e.len() == width));
        assert_eq!(batch[0].attention_mask.iter().sum::<u32>(), 3);
        assert!(batch[0]
            .ids
            .iter()
            .zip(&batch[0].attention_mask)
            .all(|(id, m)| *m == 1 || *id == t.pad_id()));
    }

    #[test]
    fn lote_vazio_nao_quebra() {
        assert!(tk().encode_batch(&[]).is_empty());
    }
}
