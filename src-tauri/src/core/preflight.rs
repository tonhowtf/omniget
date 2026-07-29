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

impl Problem {
    /// Problema que o usuario consegue resolver antes de comecar.
    ///
    /// A distincao importa para a copy: "importe seus cookies" e acionavel,
    /// "nenhum downloader cobre este site" nao e.
    pub fn is_actionable(self) -> bool {
        matches!(self, Problem::NeedsAuth)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(url: &str, bytes: u64) -> ItemCheck {
        ItemCheck {
            url: url.into(),
            title: Some("t".into()),
            estimated_bytes: Some(bytes),
            problem: None,
        }
    }

    fn bad(url: &str, p: Problem) -> ItemCheck {
        ItemCheck {
            url: url.into(),
            title: None,
            estimated_bytes: None,
            problem: Some(p),
        }
    }

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn lote_limpo_que_cabe_libera() {
        let r = build_report(vec![ok("a", GB), ok("b", GB)], Some(10 * GB));
        assert_eq!(r.verdict, Verdict::Go);
        assert_eq!(r.ready, 2);
        assert_eq!(r.estimated_bytes, 2 * GB);
        assert!(r.problems.is_empty());
    }

    #[test]
    fn alguns_com_problema_ainda_libera_o_resto() {
        // O lote nao para inteiro por causa de um link privado: o usuario
        // baixa o que da e resolve o resto depois.
        let r = build_report(
            vec![ok("a", GB), bad("b", Problem::NeedsAuth), ok("c", GB)],
            Some(10 * GB),
        );
        assert_eq!(r.verdict, Verdict::GoWithSkips);
        assert_eq!(r.ready, 2);
        assert_eq!(r.problems.len(), 1);
        // Tamanho estimado conta so o que vai baixar de verdade.
        assert_eq!(r.estimated_bytes, 2 * GB);
    }

    #[test]
    fn nada_resolvendo_para_o_lote() {
        let r = build_report(
            vec![bad("a", Problem::Unsupported), bad("b", Problem::NotFound)],
            Some(100 * GB),
        );
        assert_eq!(r.verdict, Verdict::Stop);
        assert_eq!(r.ready, 0);
    }

    #[test]
    fn nao_cabe_no_disco_para_antes_de_comecar() {
        // O caso que originou o item: o lote nao pode comecar para descobrir
        // no meio que o disco acabou.
        // 10 GB estimados exigem 11 GB com a margem, e nao cabem em 10 GB.
        let r = build_report(vec![ok("a", 10 * GB)], Some(10 * GB));
        assert_eq!(r.verdict, Verdict::Stop, "10GB + 10% nao cabe em 10GB");

        // 9 GB exigem 9,9 GB e cabem — a margem nao pode ser tao conservadora
        // a ponto de recusar lote que cabe.
        let r2 = build_report(vec![ok("a", 9 * GB)], Some(10 * GB));
        assert_eq!(r2.verdict, Verdict::Go, "9GB + 10% cabe em 10GB");
    }

    #[test]
    fn espaco_ilegivel_nao_bloqueia() {
        // Impedir o download porque a leitura de disco falhou trocaria um
        // problema possivel por um problema certo.
        let r = build_report(vec![ok("a", 500 * GB)], None);
        assert_eq!(r.verdict, Verdict::Go);
        assert_eq!(r.available_bytes, None);
    }

    #[test]
    fn item_sem_tamanho_estimado_nao_zera_a_conta() {
        // Live e alguns extractors nao informam tamanho. Nao pode virar 0 e
        // fazer um lote de 50 GB parecer caber em qualquer lugar — mas tambem
        // nao pode inventar numero. Conta o que sabe, e segue.
        let sem_tamanho = ItemCheck {
            url: "live".into(),
            title: None,
            estimated_bytes: None,
            problem: None,
        };
        let r = build_report(vec![ok("a", GB), sem_tamanho], Some(10 * GB));
        assert_eq!(r.ready, 2);
        assert_eq!(r.estimated_bytes, GB);
        assert_eq!(r.verdict, Verdict::Go);
    }

    #[test]
    fn ja_baixado_conta_como_problema_e_nao_como_pronto() {
        let r = build_report(
            vec![ok("a", GB), bad("b", Problem::AlreadyHave)],
            Some(10 * GB),
        );
        assert_eq!(r.ready, 1);
        assert_eq!(r.verdict, Verdict::GoWithSkips);
    }

    #[test]
    fn so_falta_de_auth_e_acionavel_pelo_usuario() {
        // Guia a copy: "importe seus cookies" resolve, "nenhum downloader
        // cobre este site" nao.
        assert!(Problem::NeedsAuth.is_actionable());
        for p in [
            Problem::Unsupported,
            Problem::NotFound,
            Problem::AlreadyHave,
        ] {
            assert!(!p.is_actionable(), "{p:?}");
        }
    }

    #[test]
    fn lote_vazio_para_em_vez_de_liberar() {
        let r = build_report(vec![], Some(GB));
        assert_eq!(r.verdict, Verdict::Stop);
        assert_eq!(r.total, 0);
    }

    #[test]
    fn espaco_livre_do_volume_atual_e_legivel() {
        // Nao afirma um numero — so que a leitura funciona e nao devolve zero
        // no diretorio temporario, o que indicaria erro silencioso.
        let livre = available_space(&std::env::temp_dir());
        assert!(livre.is_some(), "nao conseguiu ler espaco livre");
        assert!(livre.unwrap() > 0);
    }
}
