//! Checagem de lote antes de baixar qualquer coisa.
//!
//! Hoje um lote de 80 links comeca a baixar e voce descobre de manha que 12
//! falharam por falta de espaco ou por exigirem login. O pre-flight resolve
//! tudo antes, mostra o que vai falhar, e deixa o usuario decidir com a
//! informacao na mao.
//!
//! A decisao e separada da coleta de proposito: o que **fazer** com N URLs, um
//! tamanho estimado e o espaco livre e logica pura e testavel; buscar metadado
//! e checar disco e I/O que so a integracao exercita.
//!
//! Origem: checklist pre-voo. Nao decola com problema conhecido em terra.

use serde::Serialize;

/// O que se descobriu sobre uma URL do lote.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ItemCheck {
    pub url: String,
    pub title: Option<String>,
    /// Tamanho estimado em bytes, quando o extractor informou.
    pub estimated_bytes: Option<u64>,
    pub problem: Option<Problem>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    /// Nenhum downloader reconhece a URL.
    Unsupported,
    /// O conteudo existe mas exige sessao — curso pago, video privado.
    NeedsAuth,
    /// O extractor respondeu que nao existe.
    NotFound,
    /// Ja esta na fila ou no archive.
    AlreadyHave,
}

impl Problem {}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PreflightReport {
    pub total: usize,
    pub ready: usize,
    pub estimated_bytes: u64,
    /// `None` quando nao foi possivel ler o espaco livre.
    pub available_bytes: Option<u64>,
    pub problems: Vec<ItemCheck>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Tudo resolvido e cabe no disco.
    Go,
    /// Alguns itens tem problema, mas o resto pode baixar.
    GoWithSkips,
    /// Nada resolveu, ou nao cabe no disco.
    Stop,
}

/// Margem exigida acima do tamanho estimado.
///
/// Dez por cento: a estimativa do extractor erra para baixo com frequencia
/// (bitrate variavel), e encher o disco ate o ultimo byte quebra o proprio
/// sistema do usuario, nao so o download.
pub const HEADROOM: f64 = 1.10;

/// Monta o relatorio a partir do que ja foi coletado.
///
/// `available_bytes = None` significa "nao consegui medir" e **nao** bloqueia:
/// impedir o download porque a leitura de disco falhou trocaria um problema
/// possivel por um problema certo.
pub fn build_report(checks: Vec<ItemCheck>, available_bytes: Option<u64>) -> PreflightReport {
    let total = checks.len();
    let ready = checks.iter().filter(|c| c.problem.is_none()).count();

    let estimated_bytes: u64 = checks
        .iter()
        .filter(|c| c.problem.is_none())
        .filter_map(|c| c.estimated_bytes)
        .sum();

    let fits = match available_bytes {
        Some(free) => (estimated_bytes as f64 * HEADROOM) <= free as f64,
        None => true,
    };

    // Dois motivos distintos para parar: nada resolveu, ou o que resolveu nao
    // cabe. A copy da tela precisa distinguir os dois, mas o veredito e o mesmo.
    let verdict = if ready == 0 || !fits {
        Verdict::Stop
    } else if ready < total {
        Verdict::GoWithSkips
    } else {
        Verdict::Go
    };

    PreflightReport {
        total,
        ready,
        estimated_bytes,
        available_bytes,
        problems: checks.into_iter().filter(|c| c.problem.is_some()).collect(),
        verdict,
    }
}

/// Espaco livre no volume de `path`, em bytes.
pub fn available_space(path: &std::path::Path) -> Option<u64> {
    fs4::available_space(path).ok()
}
