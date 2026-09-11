//! Contagem estimada de tokens e custo por modelo (tool `ai-token-cost`).
//!
//! Contar token exato exige o tokenizador de cada família de modelo (BPE do
//! GPT, o do Claude, SentencePiece do Llama/Gemini) — são megabytes de vocabulário
//! por família e nenhum deles é publicado para todas. O caminho honesto aqui é
//! uma heurística calibrada por segmento (palavra, número, pontuação, espaço,
//! CJK) com um fator por família, e a UI deixando claro que é **estimativa**
//! com margem declarada. Nunca apresente o número como exato.
//!
//! A tabela de preços é a mesma de `super::pricing` (LiteLLM); aqui só entra a
//! contagem e a multiplicação.

use serde::{Deserialize, Serialize};

use super::pricing::{self, ModelPrice};
use super::{report, ProgressFn};

const ID: &str = "ai-token-cost";

/// Margem declarada da estimativa, em porcento, para cima e para baixo.
pub const MARGIN_PCT: f64 = 25.0;

/// Teto padrão por arquivo (5 MiB) — acima disso o arquivo é ignorado.
const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_MAX_FILES: usize = 2000;

const DEFAULT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "rst", "org", "csv", "tsv", "json", "jsonl", "yaml", "yml", "toml",
    "ini", "xml", "html", "htm", "css", "js", "jsx", "ts", "tsx", "svelte", "vue", "rs", "go",
    "py", "rb", "java", "kt", "swift", "c", "h", "cpp", "hpp", "cs", "php", "sh", "bash", "zsh",
    "sql", "tex", "log", "srt", "vtt",
];

/// Modelos mostrados quando a UI não pede nenhum em particular. São chaves
/// estáveis do LiteLLM; o que não existir na tabela do dia volta em `missing`.
pub const DEFAULT_MODELS: &[&str] = &[
    "gpt-4o",
    "gpt-4o-mini",
    "claude-3-5-sonnet-20241022",
    "claude-3-5-haiku-20241022",
    "gemini-1.5-pro",
    "gemini-1.5-flash",
    "deepseek-chat",
];

// ── Famílias de tokenizador ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Family {
    pub key: &'static str,
    pub label: &'static str,
    /// Multiplicador sobre a estimativa base (calibrada no BPE do GPT-4o).
    pub factor: f64,
}

pub const FAMILIES: &[Family] = &[
    Family {
        key: "o200k",
        label: "GPT-4o / GPT-4.1 / o-series (o200k BPE)",
        factor: 1.0,
    },
    Family {
        key: "cl100k",
        label: "GPT-4 / GPT-3.5 (cl100k BPE)",
        factor: 1.05,
    },
    Family {
        key: "claude",
        label: "Claude",
        factor: 1.16,
    },
    Family {
        key: "gemini",
        label: "Gemini / Gemma (SentencePiece)",
        factor: 1.05,
    },
    Family {
        key: "llama",
        label: "Llama (SentencePiece)",
        factor: 1.10,
    },
    Family {
        key: "mistral",
        label: "Mistral / Mixtral",
        factor: 1.12,
    },
    Family {
        key: "qwen",
        label: "Qwen / DeepSeek",
        factor: 1.05,
    },
    Family {
        key: "generic",
        label: "Outro",
        factor: 1.10,
    },
];

fn family(key: &str) -> &'static Family {
    FAMILIES
        .iter()
        .find(|f| f.key == key)
        .unwrap_or(&FAMILIES[FAMILIES.len() - 1])
}

/// Descobre a família pelo nome do modelo. Sem rede e sem tabela.
pub fn family_for(model: &str) -> &'static Family {
    let m = model.to_lowercase();
    let has = |needle: &str| m.contains(needle);
    if has("gpt-4o") || has("gpt-4.1") || has("gpt-5") || has("chatgpt-4o") {
        return family("o200k");
    }
    // o1/o3/o4 são o200k, mas "o1" casaria dentro de outras palavras.
    for p in ["o1", "o3", "o4"] {
        if m == p || m.starts_with(&format!("{}-", p)) || m.contains(&format!("/{}-", p)) {
            return family("o200k");
        }
    }
    if has("gpt-") || has("text-embedding") || has("davinci") || has("babbage") {
        return family("cl100k");
    }
    if has("claude") {
        return family("claude");
    }
    if has("gemini") || has("gemma") {
        return family("gemini");
    }
    if has("llama") {
        return family("llama");
    }
    if has("mistral") || has("mixtral") || has("codestral") {
        return family("mistral");
    }
    if has("qwen") || has("deepseek") || has("glm") || has("kimi") {
        return family("qwen");
    }
    family("generic")
}

// ── Heurística de contagem ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Letter,
    Digit,
    Cjk,
    Space,
    Other,
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF      // kana
        | 0x3400..=0x4DBF    // extensão A
        | 0x4E00..=0x9FFF    // unificado
        | 0xF900..=0xFAFF    // compatibilidade
        | 0xAC00..=0xD7AF) // hangul
}

fn class_of(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if is_cjk(c) {
        Class::Cjk
    } else if c.is_ascii_digit() {
        Class::Digit
    } else if c.is_alphabetic() {
        Class::Letter
    } else {
        Class::Other
    }
}

fn ceil_div(n: usize, d: usize) -> f64 {
    (n.div_ceil(d.max(1))) as f64
}

/// Custo de uma palavra: até seis letras costuma ser um token só; a partir daí
/// o BPE quebra devagar, porque o vocabulário guarda os pedaços comuns.
fn word_tokens(n: usize) -> f64 {
    1.0 + (n.saturating_sub(6) as f64) * 0.12
}

/// Estimativa base, calibrada no BPE do GPT-4o. Devolve fracionário de
/// propósito: arredondar só no fim evita empilhar erro em texto grande.
pub fn estimate_tokens(text: &str) -> f64 {
    let mut total = 0.0f64;
    let mut run_class: Option<Class> = None;
    let mut run_len = 0usize;
    let mut run_newline = false;

    let flush = |class: Option<Class>, len: usize, newline: bool, total: &mut f64| {
        let Some(class) = class else { return };
        if len == 0 {
            return;
        }
        *total += match class {
            Class::Letter => word_tokens(len),
            Class::Digit => ceil_div(len, 3),
            Class::Cjk => len as f64 * 0.9,
            Class::Space => {
                if len == 1 && !newline {
                    // Um espaço simples entra no token da próxima palavra.
                    0.0
                } else {
                    ceil_div(len, 4).max(1.0)
                }
            }
            Class::Other => ceil_div(len * 2, 3).max(1.0),
        };
    };

    for c in text.chars() {
        let cls = class_of(c);
        if Some(cls) == run_class {
            run_len += 1;
            run_newline |= c == '\n';
        } else {
            flush(run_class, run_len, run_newline, &mut total);
            run_class = Some(cls);
            run_len = 1;
            run_newline = c == '\n';
        }
    }
    flush(run_class, run_len, run_newline, &mut total);
    total
}

/// Estimativa já com o fator da família (`o200k`, `claude`, `llama`…).
pub fn tokens_for_family(text: &str, family_key: &str) -> u64 {
    (estimate_tokens(text) * family(family_key).factor).round() as u64
}

/// Estimativa para um modelo pelo nome (usa `family_for`).
pub fn tokens_for_model(text: &str, model: &str) -> u64 {
    (estimate_tokens(text) * family_for(model).factor).round() as u64
}

// ── Entrada ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Options {
    /// Texto colado.
    #[serde(default)]
    pub text: String,
    /// Arquivos e/ou pastas. Pasta é varrida recursivamente.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Filtro de extensões, separado por vírgula. Vazio = lista padrão de texto.
    #[serde(default)]
    pub extensions: String,
    #[serde(default)]
    pub max_files: usize,
    #[serde(default)]
    pub max_bytes: u64,
    /// Modelos a comparar. Vazio = `DEFAULT_MODELS`.
    #[serde(default)]
    pub models: Vec<String>,
    /// Tokens de saída assumidos para o cálculo de custo.
    #[serde(default)]
    pub output_tokens: u64,
    /// Quantas vezes o mesmo prompt seria enviado.
    #[serde(default)]
    pub calls: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCount {
    pub path: String,
    pub bytes: u64,
    pub chars: u64,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCost {
    /// O que o usuário pediu.
    pub model: String,
    /// A chave que casou na tabela do LiteLLM.
    pub matched: String,
    pub provider: String,
    pub family: String,
    pub family_label: String,
    pub input_tokens: u64,
    pub input_low: u64,
    pub input_high: u64,
    pub output_tokens: u64,
    pub input_usd: Option<f64>,
    pub output_usd: Option<f64>,
    pub total_usd: Option<f64>,
    pub total_low_usd: Option<f64>,
    pub total_high_usd: Option<f64>,
    /// Custo da entrada se ela estivesse toda em cache lido (quando o preço distingue).
    pub cached_input_usd: Option<f64>,
    /// Custo de gravar essa entrada no cache (quando o preço distingue).
    pub cache_write_usd: Option<f64>,
    pub max_input_tokens: Option<u64>,
    pub fits_context: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub chars: u64,
    pub bytes: u64,
    pub words: u64,
    pub lines: u64,
    pub files: Vec<FileCount>,
    pub files_read: u64,
    pub files_skipped: u64,
    /// Estimativa na base GPT-4o, antes do fator de família.
    pub tokens: u64,
    pub tokens_low: u64,
    pub tokens_high: u64,
    pub margin_pct: f64,
    /// Sempre `true`: nunca apresente estes números como contagem exata.
    pub estimated: bool,
    pub calls: u64,
    pub models: Vec<ModelCost>,
    /// Modelos que o usuário pediu e a tabela de preços não conhece.
    pub missing: Vec<String>,
}

fn wanted_exts(spec: &str) -> Vec<String> {
    let list: Vec<String> = spec
        .split([',', ';', ' '])
        .map(|s| s.trim().trim_start_matches('.').to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if list.is_empty() {
        DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect()
    } else {
        list
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// Junta o texto colado com o conteúdo dos arquivos/pastas pedidos.
/// Bloqueante de propósito: quem chama roda em `spawn_blocking`.
pub fn collect(opts: &Options, p: &ProgressFn) -> anyhow::Result<(String, Vec<FileCount>, u64)> {
    let mut parts: Vec<String> = Vec::new();
    let mut files: Vec<FileCount> = Vec::new();
    let mut skipped = 0u64;
    if !opts.text.is_empty() {
        parts.push(opts.text.clone());
    }
    if opts.paths.is_empty() {
        return Ok((parts.join("\n"), files, skipped));
    }

    let exts = wanted_exts(&opts.extensions);
    let max_files = if opts.max_files == 0 {
        DEFAULT_MAX_FILES
    } else {
        opts.max_files
    };
    let max_bytes = if opts.max_bytes == 0 {
        DEFAULT_MAX_BYTES
    } else {
        opts.max_bytes
    };

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    for raw in &opts.paths {
        let path = std::path::PathBuf::from(raw);
        if path.is_dir() {
            for entry in walkdir::WalkDir::new(&path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let ok = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| exts.iter().any(|w| w == &e.to_lowercase()))
                    .unwrap_or(false);
                if ok {
                    candidates.push(entry.into_path());
                } else {
                    skipped += 1;
                }
                if candidates.len() >= max_files {
                    break;
                }
            }
        } else if path.is_file() {
            candidates.push(path);
        } else {
            skipped += 1;
        }
        if candidates.len() >= max_files {
            break;
        }
    }

    let total = candidates.len() as u64;
    for (i, path) in candidates.iter().enumerate() {
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if meta.len() > max_bytes {
            skipped += 1;
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if looks_binary(&bytes) {
            skipped += 1;
            continue;
        }
        let text = String::from_utf8_lossy(&bytes).to_string();
        files.push(FileCount {
            path: path.to_string_lossy().to_string(),
            bytes: meta.len(),
            chars: text.chars().count() as u64,
            tokens: estimate_tokens(&text).round() as u64,
        });
        parts.push(text);
        if i % 25 == 0 {
            report(p, ID, "progress", i as u64, Some(total), None);
        }
    }
    report(p, ID, "progress", total, Some(total), None);
    Ok((parts.join("\n"), files, skipped))
}

// ── Custo ──────────────────────────────────────────────────────────────

fn per_token(per_m: Option<f64>) -> Option<f64> {
    per_m.map(|v| v / 1_000_000.0)
}

/// Custo de um prompt num modelo, com a tabela de preços já resolvida.
/// Puro: dá para testar sem rede.
pub fn cost_for(
    asked: &str,
    price: &ModelPrice,
    base_tokens: f64,
    output_tokens: u64,
    calls: u64,
) -> ModelCost {
    let fam = family_for(asked);
    let calls = calls.max(1) as f64;
    let exact = base_tokens * fam.factor;
    let low = exact * (1.0 - MARGIN_PCT / 100.0);
    let high = exact * (1.0 + MARGIN_PCT / 100.0);
    let input_tokens = exact.round() as u64;

    let in_rate = per_token(price.input_per_m);
    let out_rate = per_token(price.output_per_m);
    let input_usd = in_rate.map(|r| r * exact * calls);
    let output_usd = out_rate.map(|r| r * output_tokens as f64 * calls);
    // O total sai do mesmo caminho que o resto do app usa (`pricing::cost`),
    // para não haver duas contas de custo divergindo.
    let total_usd = pricing::cost(price, input_tokens, output_tokens).map(|c| c * calls);
    let total_low_usd = in_rate.map(|r| r * low * calls + output_usd.unwrap_or(0.0));
    let total_high_usd = in_rate.map(|r| r * high * calls + output_usd.unwrap_or(0.0));

    ModelCost {
        model: asked.to_string(),
        matched: price.key.clone(),
        provider: price.provider.clone(),
        family: fam.key.to_string(),
        family_label: fam.label.to_string(),
        input_tokens,
        input_low: low.round() as u64,
        input_high: high.round() as u64,
        output_tokens,
        input_usd,
        output_usd,
        total_usd,
        total_low_usd,
        total_high_usd,
        cached_input_usd: per_token(price.cache_read_per_m).map(|r| r * exact * calls),
        cache_write_usd: per_token(price.cache_write_per_m).map(|r| r * exact * calls),
        max_input_tokens: price.max_input_tokens,
        fits_context: price.max_input_tokens.map(|m| input_tokens <= m),
    }
}

/// Só a contagem, sem tocar na tabela de preços (nem na rede).
pub fn count(opts: &Options, p: &ProgressFn) -> anyhow::Result<Report> {
    report(p, ID, "started", 0, None, None);
    let (text, files, skipped) = collect(opts, p)?;
    let base = estimate_tokens(&text);
    let low = base * (1.0 - MARGIN_PCT / 100.0);
    let high = base * (1.0 + MARGIN_PCT / 100.0);
    let files_read = files.len() as u64;
    let r = Report {
        chars: text.chars().count() as u64,
        bytes: text.len() as u64,
        words: text.split_whitespace().count() as u64,
        lines: if text.is_empty() {
            0
        } else {
            text.lines().count() as u64
        },
        files,
        files_read,
        files_skipped: skipped,
        tokens: base.round() as u64,
        tokens_low: low.round() as u64,
        tokens_high: high.round() as u64,
        margin_pct: MARGIN_PCT,
        estimated: true,
        calls: opts.calls.max(1),
        models: Vec::new(),
        missing: Vec::new(),
    };
    report(p, ID, "done", r.tokens, None, None);
    Ok(r)
}

/// Contagem + custo por modelo. A parte de preço usa `super::pricing`, que
/// pode ir à rede uma vez por dia para atualizar a tabela do LiteLLM.
pub async fn run(opts: Options, p: ProgressFn) -> anyhow::Result<Report> {
    let opts_for_count = opts.clone();
    let p2 = p.clone();
    let mut rep = tokio::task::spawn_blocking(move || count(&opts_for_count, &p2)).await??;

    let asked: Vec<String> = if opts.models.is_empty() {
        DEFAULT_MODELS.iter().map(|s| s.to_string()).collect()
    } else {
        opts.models.clone()
    };
    let base = rep.tokens as f64;
    for m in asked {
        match pricing::price_for(&m).await {
            Some(price) => {
                rep.models
                    .push(cost_for(&m, &price, base, opts.output_tokens, opts.calls))
            }
            None => rep.missing.push(m),
        }
    }
    rep.models.sort_by(|a, b| {
        a.total_usd
            .unwrap_or(f64::MAX)
            .partial_cmp(&b.total_usd.unwrap_or(f64::MAX))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(key: &str, input: f64, output: f64) -> ModelPrice {
        ModelPrice {
            key: key.to_string(),
            provider: "openai".to_string(),
            mode: "chat".to_string(),
            input_per_m: Some(input),
            output_per_m: Some(output),
            cache_read_per_m: Some(input / 2.0),
            cache_write_per_m: Some(input * 1.25),
            max_input_tokens: Some(128_000),
            max_output_tokens: Some(16_384),
            input_per_second: None,
            input_per_character: None,
            supports_vision: true,
            supports_tools: true,
            supports_reasoning: false,
            supports_caching: true,
            deprecation_date: None,
        }
    }

    // Casos conhecidos: o BPE do GPT dá 4 e 9 tokens nestas duas frases.
    #[test]
    fn casos_conhecidos_de_contagem() {
        assert_eq!(estimate_tokens("Hello, world!").round() as u64, 4);
        assert_eq!(
            estimate_tokens("The quick brown fox jumps over the lazy dog").round() as u64,
            9
        );
    }

    #[test]
    fn vazio_e_zero() {
        assert_eq!(estimate_tokens("").round() as u64, 0);
        assert_eq!(tokens_for_family("", "claude"), 0);
    }

    #[test]
    fn palavra_longa_custa_mais_que_curta() {
        assert!(estimate_tokens("internationalization") > estimate_tokens("dog"));
        assert!(word_tokens(6) < word_tokens(20));
        assert_eq!(word_tokens(3), 1.0);
    }

    #[test]
    fn numeros_agrupam_de_tres() {
        // "1234567" vira ~3 pedaços no BPE moderno.
        assert_eq!(estimate_tokens("1234567").round() as u64, 3);
    }

    #[test]
    fn cjk_conta_por_caractere() {
        let t = estimate_tokens("你好世界");
        assert!(t > 3.0 && t < 5.0, "{}", t);
    }

    #[test]
    fn quebra_de_linha_conta() {
        assert!(estimate_tokens("a\n\nb") > estimate_tokens("a b"));
    }

    #[test]
    fn familia_por_nome_do_modelo() {
        assert_eq!(family_for("gpt-4o-mini").key, "o200k");
        assert_eq!(family_for("openai/gpt-4-turbo").key, "cl100k");
        assert_eq!(family_for("claude-3-5-sonnet-20241022").key, "claude");
        assert_eq!(family_for("gemini-1.5-pro").key, "gemini");
        assert_eq!(family_for("groq/llama-3.3-70b-versatile").key, "llama");
        assert_eq!(family_for("mistral-large-latest").key, "mistral");
        assert_eq!(family_for("deepseek-chat").key, "qwen");
        assert_eq!(family_for("coisa-nova-2030").key, "generic");
        assert_eq!(family_for("o3-mini").key, "o200k");
    }

    #[test]
    fn claude_estima_mais_token_que_gpt() {
        let s = "Um texto qualquer para comparar as duas familias de tokenizador.";
        assert!(tokens_for_family(s, "claude") > tokens_for_family(s, "o200k"));
    }

    #[test]
    fn custo_bate_com_a_tabela() {
        let p = price("gpt-4o", 2.5, 10.0);
        // 1M tokens de entrada e 1M de saida: 2.50 + 10.00.
        let c = cost_for("gpt-4o", &p, 1_000_000.0, 1_000_000, 1);
        assert_eq!(c.input_tokens, 1_000_000);
        let total = c.total_usd.unwrap_or(0.0);
        assert!((total - 12.5).abs() < 1e-6, "{}", total);
        assert!((c.input_usd.unwrap_or(0.0) - 2.5).abs() < 1e-6);
        assert!((c.output_usd.unwrap_or(0.0) - 10.0).abs() < 1e-6);
        // Cache lido custa metade neste preço de teste.
        assert!((c.cached_input_usd.unwrap_or(0.0) - 1.25).abs() < 1e-6);
        assert!((c.cache_write_usd.unwrap_or(0.0) - 3.125).abs() < 1e-6);
        assert_eq!(c.fits_context, Some(false));
    }

    #[test]
    fn margem_envolve_a_estimativa() {
        let p = price("gpt-4o", 2.5, 10.0);
        let c = cost_for("gpt-4o", &p, 100_000.0, 0, 1);
        assert!(c.input_low < c.input_tokens && c.input_tokens < c.input_high);
        let (lo, hi) = (
            c.total_low_usd.unwrap_or(0.0),
            c.total_high_usd.unwrap_or(0.0),
        );
        let mid = c.total_usd.unwrap_or(0.0);
        assert!(lo < mid && mid < hi, "{} {} {}", lo, mid, hi);
    }

    #[test]
    fn varias_chamadas_multiplicam() {
        let p = price("gpt-4o", 2.5, 10.0);
        let um = cost_for("gpt-4o", &p, 1_000_000.0, 0, 1);
        let dez = cost_for("gpt-4o", &p, 1_000_000.0, 0, 10);
        let a = um.total_usd.unwrap_or(0.0);
        let b = dez.total_usd.unwrap_or(0.0);
        assert!((b - a * 10.0).abs() < 1e-6, "{} {}", a, b);
    }

    #[test]
    fn conta_texto_colado_sem_arquivo() {
        let opts = Options {
            text: "Hello, world!".to_string(),
            ..Default::default()
        };
        let Ok(r) = count(&opts, &super::super::noop_progress()) else {
            panic!("contagem falhou");
        };
        assert_eq!(r.tokens, 4);
        assert_eq!(r.words, 2);
        assert_eq!(r.chars, 13);
        assert!(r.estimated);
        assert_eq!(r.files_read, 0);
    }

    #[test]
    fn extensoes_padrao_quando_vazio() {
        assert!(wanted_exts("").len() > 10);
        assert_eq!(wanted_exts(".md, rs").len(), 2);
        assert!(wanted_exts("MD").contains(&"md".to_string()));
    }

    #[test]
    fn binario_e_ignorado() {
        assert!(looks_binary(&[0x00, 0x01, 0x02]));
        assert!(!looks_binary(b"texto normal"));
    }
}
