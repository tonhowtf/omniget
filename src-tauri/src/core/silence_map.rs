//! Mapa de silencio de uma aula, para o player pular em reproducao.
//!
//! O B36 corta silencio transformando o arquivo, e por isso paga dois precos:
//! `silenceremove` preserva `stop_duration` de cada trecho para nao decepar o
//! ataque da fala, o que deixou 1,4% de residuo na medicao do B53; e como ele e
//! filtro de audio, a saida tem que ser audio, senao o video dessincroniza.
//!
//! Pulando na reproducao, os dois problemas somem. O padding vira decisao de
//! reproducao — ajustavel e reversivel — em vez de corte irreversivel, e o video
//! continua video. **Isto nao e o B36 noutra camada: e a versao que alcanca o
//! numero que o B36 nao alcanca.**
//!
//! O mapa e computado uma vez pela sonda que ja existe (`silence_probe_args`) e
//! guardado como metadado da aula. Nada e reprocessado ao assistir de novo.
//!
//! Origem: Smart Speed do Overcast, que tambem age na reproducao.

use serde::{Deserialize, Serialize};

/// Um trecho de silencio, em segundos desde o inicio.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct SilenceSpan {
    pub start: f64,
    pub end: f64,
}

impl SilenceSpan {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// Mapa persistido junto da aula.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SilenceMap {
    /// Versao do algoritmo. Se os parametros da sonda mudarem, um mapa antigo
    /// descreve outra coisa e precisa ser recomputado em vez de reaproveitado.
    #[serde(default)]
    pub version: u32,
    pub media_duration_secs: f64,
    #[serde(default)]
    pub spans: Vec<SilenceSpan>,
}

pub const CURRENT_VERSION: u32 = 1;

/// Quanto de cada trecho fica preservado na reproducao.
///
/// Menor que o `stop_duration` de 0,35 s do B36 porque aqui nao ha risco de
/// decepar audio: nada e cortado, so pulado, e um salto pode ser ajustado ou
/// desligado a qualquer momento. 0,15 s evita que a fala comece abruptamente
/// sem devolver o silencio inteiro.
pub const PLAYBACK_PADDING_SECS: f64 = 0.15;

/// Trecho curto demais nao vale um salto: o seek custa mais que o silencio.
pub const MIN_SKIPPABLE_SECS: f64 = 0.4;

/// Le `silence_start` / `silence_end` do stderr do ffmpeg em pares.
///
/// Um `silence_start` sem `silence_end` significa que o arquivo terminou em
/// silencio; sem a duracao total nao da para fechar o par, entao e descartado.
pub fn parse_spans(stderr: &str) -> Vec<SilenceSpan> {
    let mut spans = Vec::new();
    let mut open: Option<f64> = None;

    for line in stderr.lines() {
        if let Some(v) = field_after(line, "silence_start:") {
            open = Some(v);
        } else if let Some(end) = field_after(line, "silence_end:") {
            if let Some(start) = open.take() {
                if end > start {
                    spans.push(SilenceSpan { start, end });
                }
            }
        }
    }
    spans
}

fn field_after(line: &str, key: &str) -> Option<f64> {
    let idx = line.find(key)?;
    line[idx + key.len()..]
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Para onde saltar quando a reproducao chega em `position`.
///
/// `None` significa "siga tocando". Devolve o destino em vez de um booleano
/// para o player nao ter que reimplementar a aritmetica do padding.
pub fn skip_target(map: &SilenceMap, position: f64) -> Option<f64> {
    map.spans
        .iter()
        .filter(|s| s.duration() >= MIN_SKIPPABLE_SECS)
        .find(|s| position >= s.start && position < s.end - PLAYBACK_PADDING_SECS)
        .map(|s| s.end - PLAYBACK_PADDING_SECS)
}

/// Quanto tempo a reproducao economiza com o mapa, em segundos.
///
/// Este e o numero que o B36 nao alcanca: aqui so o padding de reproducao e
/// descontado, e ele e menos da metade do `stop_duration` que o corte exige.
pub fn savings_secs(map: &SilenceMap) -> f64 {
    map.spans
        .iter()
        .filter(|s| s.duration() >= MIN_SKIPPABLE_SECS)
        .map(|s| (s.duration() - PLAYBACK_PADDING_SECS).max(0.0))
        .sum()
}

/// Um mapa gravado por uma versao anterior do algoritmo nao serve.
pub fn needs_recompute(map: &SilenceMap) -> bool {
    map.version != CURRENT_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(spans: &[(f64, f64)]) -> SilenceMap {
        SilenceMap {
            version: CURRENT_VERSION,
            media_duration_secs: 300.0,
            spans: spans
                .iter()
                .map(|(a, b)| SilenceSpan { start: *a, end: *b })
                .collect(),
        }
    }

    const STDERR: &str = "\
[silencedetect @ 0x1] silence_start: 10.0\n\
[silencedetect @ 0x1] silence_end: 15.0 | silence_duration: 5.0\n\
[silencedetect @ 0x1] silence_start: 100.0\n\
[silencedetect @ 0x1] silence_end: 103.0 | silence_duration: 3.0\n\
[silencedetect @ 0x1] silence_start: 290.0\n";

    #[test]
    fn le_pares_e_descarta_o_trecho_aberto() {
        let s = parse_spans(STDERR);
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(
            s[0],
            SilenceSpan {
                start: 10.0,
                end: 15.0
            }
        );
        assert_eq!(
            s[1],
            SilenceSpan {
                start: 100.0,
                end: 103.0
            }
        );
    }

    #[test]
    fn stderr_sem_silencio_da_mapa_vazio() {
        assert!(parse_spans("").is_empty());
        assert!(parse_spans("size=N/A time=00:05:00.00").is_empty());
        // `silence_end` sem `start` correspondente nao pode virar trecho.
        assert!(parse_spans("silence_end: 5.0 | silence_duration: 1.0").is_empty());
    }

    #[test]
    fn salta_do_inicio_do_silencio_para_pouco_antes_do_fim() {
        let m = map(&[(10.0, 15.0)]);
        let alvo = skip_target(&m, 10.0).unwrap();
        assert!(
            (alvo - (15.0 - PLAYBACK_PADDING_SECS)).abs() < 1e-9,
            "{alvo}"
        );
        // Dentro do trecho tambem salta, nao so na borda exata.
        assert!(skip_target(&m, 12.0).is_some());
    }

    #[test]
    fn nao_salta_fora_do_silencio() {
        let m = map(&[(10.0, 15.0)]);
        assert_eq!(skip_target(&m, 5.0), None);
        assert_eq!(skip_target(&m, 20.0), None);
    }

    #[test]
    fn nao_salta_dentro_do_padding_final() {
        // Saltar aqui daria um seek de milissegundos e cortaria o inicio da
        // fala — o defeito que o padding existe para evitar.
        let m = map(&[(10.0, 15.0)]);
        assert_eq!(skip_target(&m, 14.95), None);
    }

    #[test]
    fn trecho_curto_demais_nao_vale_o_seek() {
        let m = map(&[(10.0, 10.2)]);
        assert_eq!(skip_target(&m, 10.0), None);
        assert_eq!(savings_secs(&m), 0.0);
    }

    #[test]
    fn a_economia_supera_a_do_corte_em_arquivo() {
        // O ponto do item. Doze trechos de 5 s: cortando o arquivo, o B36
        // preserva 0,35 s de cada um e entrega 55,8 s. Pulando na reproducao o
        // padding e 0,15 s, entao sobra mais.
        let spans: Vec<(f64, f64)> = (0..12)
            .map(|i| (i as f64 * 25.0, i as f64 * 25.0 + 5.0))
            .collect();
        let m = map(&spans);

        let economia = savings_secs(&m);
        let corte_em_arquivo = 12.0 * (5.0 - 0.35);
        assert!(
            economia > corte_em_arquivo,
            "reproducao {economia} deveria superar corte {corte_em_arquivo}"
        );
        assert!((economia - 12.0 * (5.0 - 0.15)).abs() < 1e-9, "{economia}");
    }

    #[test]
    fn mapa_de_versao_antiga_e_recomputado() {
        // Se os parametros da sonda mudarem, um mapa velho descreve outra
        // coisa. Reaproveitar faria o player pular nos lugares errados.
        let mut m = map(&[(1.0, 2.0)]);
        assert!(!needs_recompute(&m));
        m.version = 0;
        assert!(needs_recompute(&m));
        assert!(needs_recompute(&SilenceMap::default()));
    }

    #[test]
    fn mapa_sobrevive_ao_round_trip_json() {
        let m = map(&[(10.0, 15.0), (100.0, 103.0)]);
        let json = serde_json::to_string(&m).unwrap();
        let back: SilenceMap = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn mapa_antigo_sem_version_e_tratado_como_desatualizado() {
        // Campo novo num metadado ja gravado: ausente vira 0, que nao e a
        // versao atual, entao recomputa em vez de confiar em mapa de origem
        // desconhecida.
        let json = r#"{"media_duration_secs":300.0,"spans":[]}"#;
        let m: SilenceMap = serde_json::from_str(json).unwrap();
        assert_eq!(m.version, 0);
        assert!(needs_recompute(&m));
    }
}
