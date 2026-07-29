//! Ajusta `-N` por host medindo o throughput real, em vez de confiar num
//! numero fixo em Config.
//!
//! O numero em Config e um chute que o usuario tem que dar sem informacao: alto
//! demais toma 429, baixo demais desperdiça banda, e o valor certo depende do
//! host e do momento. O limitador por host e o contador de 429 ja existem — o
//! que faltava era fechar o laco.
//!
//! Ficou possivel agora porque o B23 fez o freio de 429 funcionar de verdade:
//! antes, `-N` e `--concurrent-fragments` disputavam e o valor efetivo nao era
//! o que o codigo pensava, entao nao havia sinal confiavel para realimentar.
//!
//! A decisao e pura: dadas amostras de throughput e o historico de 429, qual
//! deve ser o proximo `-N`. Medir e I/O.
//!
//! Origem: controle de congestionamento de CDN — subir devagar, cair rapido.

use serde::Serialize;
use std::collections::HashMap;

/// Limites do ajuste. Um host nunca sai desta faixa, por mais que as medicoes
/// sugiram: o teto protege o servidor e o piso garante que o download anda.
pub const MIN_N: u32 = 1;
pub const MAX_N: u32 = 16;

/// Ganho minimo para justificar subir mais.
///
/// Dez por cento: abaixo disso a diferenca se confunde com variacao normal de
/// rede, e subir `-N` atras de ruido so aumenta a chance de 429.
pub const MEANINGFUL_GAIN: f64 = 1.10;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Sample {
    pub concurrency: u32,
    /// Bytes por segundo observados com essa concorrencia.
    pub throughput_bps: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HostStats {
    samples: Vec<Sample>,
    rate_limit_hits: u32,
}

impl HostStats {
    pub fn record(&mut self, concurrency: u32, throughput_bps: f64) {
        if throughput_bps.is_finite() && throughput_bps > 0.0 {
            self.samples.push(Sample {
                concurrency,
                throughput_bps,
            });
        }
    }

    pub fn record_rate_limit(&mut self) {
        self.rate_limit_hits = self.rate_limit_hits.saturating_add(1);
    }

    /// Melhor throughput observado para uma dada concorrencia.
    ///
    /// Melhor e nao media: uma amostra ruim costuma ser interferencia local
    /// (outro download, wifi), nao a capacidade real daquele nivel.
    fn best_at(&self, concurrency: u32) -> Option<f64> {
        self.samples
            .iter()
            .filter(|s| s.concurrency == concurrency)
            .map(|s| s.throughput_bps)
            .fold(None, |acc: Option<f64>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            })
    }
}

/// Proximo `-N` para o host.
///
/// Sobe devagar e cai rapido, que e a assimetria certa: subir errado custa um
/// 429 e um retry; descer errado custa um pouco de velocidade.
pub fn next_concurrency(stats: &HostStats, current: u32) -> u32 {
    let current = current.clamp(MIN_N, MAX_N);

    // 429 domina qualquer medicao de velocidade. Nao adianta o throughput
    // parecer bom se o host esta recusando.
    if stats.rate_limit_hits > 0 {
        let penalizado = current / 2u32.pow(stats.rate_limit_hits.min(3));
        return penalizado.clamp(MIN_N, MAX_N);
    }

    let Some(atual) = stats.best_at(current) else {
        return current; // sem medicao ainda: nao mexe
    };

    match stats.best_at(current.saturating_sub(1)) {
        // Se um nivel abaixo era melhor, subir foi erro — desce.
        Some(anterior) if anterior > atual * MEANINGFUL_GAIN => (current - 1).clamp(MIN_N, MAX_N),
        _ => {
            // Só sobe se o degrau anterior mostrou ganho real.
            let vale_subir = match stats.best_at(current.saturating_sub(1)) {
                Some(anterior) => atual > anterior * MEANINGFUL_GAIN,
                None => true, // primeiro degrau medido: tenta subir uma vez
            };
            if vale_subir {
                (current + 1).clamp(MIN_N, MAX_N)
            } else {
                current
            }
        }
    }
}

/// Estado por host, para o ajuste sobreviver entre downloads.
#[derive(Debug, Default)]
pub struct ConcurrencyTuner {
    hosts: HashMap<String, HostStats>,
}

impl ConcurrencyTuner {
    pub fn record(&mut self, host: &str, concurrency: u32, throughput_bps: f64) {
        self.hosts
            .entry(host.to_string())
            .or_default()
            .record(concurrency, throughput_bps);
    }

    pub fn record_rate_limit(&mut self, host: &str) {
        self.hosts
            .entry(host.to_string())
            .or_default()
            .record_rate_limit();
    }

    /// `ceiling` e o numero de Config: vira **teto**, nao valor fixo.
    pub fn suggest(&self, host: &str, current: u32, ceiling: u32) -> u32 {
        let teto = ceiling.clamp(MIN_N, MAX_N);
        match self.hosts.get(host) {
            Some(stats) => next_concurrency(stats, current).min(teto),
            None => current.min(teto),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: f64 = 1_000_000.0;

    #[test]
    fn sem_medicao_nenhuma_nao_mexe() {
        // Ajustar sem dado seria adivinhar, que e o que a feature veio evitar.
        let s = HostStats::default();
        assert_eq!(next_concurrency(&s, 4), 4);
    }

    #[test]
    fn sobe_quando_o_degrau_anterior_mostrou_ganho_real() {
        let mut s = HostStats::default();
        s.record(3, 3.0 * MB);
        s.record(4, 5.0 * MB); // +66%, bem acima do ruido
        assert_eq!(next_concurrency(&s, 4), 5);
    }

    #[test]
    fn nao_sobe_atras_de_ruido() {
        // 5% de diferenca se confunde com variacao normal de rede; subir por
        // isso so aumenta a chance de 429 sem ganhar velocidade.
        let mut s = HostStats::default();
        s.record(3, 5.0 * MB);
        s.record(4, 5.25 * MB);
        assert_eq!(next_concurrency(&s, 4), 4);
    }

    #[test]
    fn desce_quando_subir_piorou() {
        let mut s = HostStats::default();
        s.record(3, 8.0 * MB);
        s.record(4, 5.0 * MB); // subir custou velocidade
        assert_eq!(next_concurrency(&s, 4), 3);
    }

    #[test]
    fn rate_limit_derruba_rapido_e_ignora_o_throughput() {
        // Assimetria deliberada: subir errado custa 429 e retry, descer errado
        // custa um pouco de velocidade. E throughput bom nao significa nada se
        // o host esta recusando.
        let mut s = HostStats::default();
        s.record(8, 50.0 * MB);
        s.record_rate_limit();
        assert_eq!(next_concurrency(&s, 8), 4);

        s.record_rate_limit();
        assert_eq!(next_concurrency(&s, 8), 2);

        s.record_rate_limit();
        assert_eq!(next_concurrency(&s, 8), 1);
    }

    #[test]
    fn nunca_sai_da_faixa() {
        let mut s = HostStats::default();
        s.record(16, 100.0 * MB);
        s.record(15, 10.0 * MB);
        assert_eq!(next_concurrency(&s, 16), MAX_N, "nao passa do teto");

        let mut baixo = HostStats::default();
        for _ in 0..10 {
            baixo.record_rate_limit();
        }
        assert_eq!(next_concurrency(&baixo, 1), MIN_N, "nao vai abaixo de 1");
    }

    #[test]
    fn amostra_ruim_nao_apaga_a_boa() {
        // Interferencia local (outro download, wifi) nao e a capacidade real do
        // nivel. Por isso o melhor, e nao a media.
        let mut s = HostStats::default();
        s.record(3, 3.0 * MB);
        s.record(4, 5.0 * MB);
        s.record(4, 0.1 * MB); // pico de interferencia
        assert_eq!(next_concurrency(&s, 4), 5, "a amostra ruim nao deve mandar");
    }

    #[test]
    fn medicao_invalida_e_descartada() {
        let mut s = HostStats::default();
        s.record(4, f64::NAN);
        s.record(4, -1.0);
        s.record(4, 0.0);
        assert_eq!(next_concurrency(&s, 4), 4, "sem amostra valida, nao mexe");
    }

    #[test]
    fn o_numero_de_config_vira_teto_e_nao_valor_fixo() {
        // O ponto da feature: o usuario deixa de adivinhar um numero e passa a
        // declarar um limite.
        let mut t = ConcurrencyTuner::default();
        t.record("youtube.com", 3, 3.0 * MB);
        t.record("youtube.com", 4, 6.0 * MB);
        assert_eq!(t.suggest("youtube.com", 4, 16), 5, "abaixo do teto, sobe");
        assert_eq!(t.suggest("youtube.com", 4, 4), 4, "o teto segura");
    }

    #[test]
    fn hosts_nao_contaminam_uns_aos_outros() {
        // 429 do YouTube nao pode reduzir a concorrencia do Bilibili.
        let mut t = ConcurrencyTuner::default();
        t.record_rate_limit("youtube.com");
        assert_eq!(t.suggest("youtube.com", 8, 16), 4);
        assert_eq!(t.suggest("bilibili.com", 8, 16), 8);
    }
}
