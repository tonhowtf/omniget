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
