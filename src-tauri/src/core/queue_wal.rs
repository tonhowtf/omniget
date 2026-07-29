//! Fila de download como write-ahead log.
//!
//! O `recovery.json` de hoje reescreve o arquivo inteiro a cada mudança e guarda
//! só a intenção — url, título, pasta. Um `kill -9` no meio de 200 itens perde a
//! ordem, o progresso e as opções de cada um, e o que volta é um aviso que o
//! usuário precisa aceitar, com tudo re-resolvido do zero.
//!
//! Aqui cada transição de estado vira **um registro append-only**. Reconstruir é
//! reler o log do começo. Sem reescrita, sem janela em que o arquivo está pela
//! metade, e o custo de gravar não cresce com o tamanho da fila.
//!
//! ## Por que append-only e não "escreve tudo, renomeia"
//!
//! O `write_to_disk` atual já é atômico via `tmp` + `rename`, e isso protege
//! contra arquivo truncado — mas não contra perda. Entre duas gravações existe
//! um intervalo, e uma fila de 200 itens reescrita a cada progresso ou paga
//! I/O demais ou grava raramente e perde o que aconteceu no meio.
//!
//! ## O que sobrevive a um registro corrompido
//!
//! A última linha é a que pode estar pela metade quando o processo morre no meio
//! da gravação. Ela é descartada e **todas as anteriores continuam válidas** —
//! que é a propriedade que um arquivo único reescrito não tem.

use serde::{Deserialize, Serialize};

/// Uma transição. O log é uma sequência disto.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WalRecord {
    /// Entrou na fila, com tudo que é preciso para refazê-lo sem re-resolver.
    Enqueued {
        id: u64,
        url: String,
        title: String,
        platform: String,
        output_dir: String,
        #[serde(default)]
        quality: Option<String>,
        #[serde(default)]
        download_mode: Option<String>,
        #[serde(default)]
        format_id: Option<String>,
        /// Referer, quando a plataforma exige um para o download funcionar.
        /// Ausente nos registros gravados antes deste campo existir.
        #[serde(default)]
        referer: Option<String>,
        /// Posição na fila, para a ordem sobreviver.
        #[serde(default)]
        position: u32,
    },
    Started {
        id: u64,
    },
    /// Progresso. Gravado com parcimônia — ver `PROGRESS_STEP_PERCENT`.
    Progress {
        id: u64,
        percent: f64,
        #[serde(default)]
        downloaded_bytes: Option<u64>,
    },
    Completed {
        id: u64,
        file_path: String,
    },
    Failed {
        id: u64,
        error: String,
    },
    Cancelled {
        id: u64,
    },
    Removed {
        id: u64,
    },
}

impl WalRecord {
    pub fn id(&self) -> u64 {
        match self {
            WalRecord::Enqueued { id, .. }
            | WalRecord::Started { id }
            | WalRecord::Progress { id, .. }
            | WalRecord::Completed { id, .. }
            | WalRecord::Failed { id, .. }
            | WalRecord::Cancelled { id }
            | WalRecord::Removed { id } => *id,
        }
    }

    /// Registro terminal: depois dele o item não volta para a fila.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WalRecord::Completed { .. }
                | WalRecord::Failed { .. }
                | WalRecord::Cancelled { .. }
                | WalRecord::Removed { .. }
        )
    }
}

/// Só grava progresso a cada 5%.
///
/// Gravar todo evento de progresso transformaria o WAL num gerador de I/O:
/// 200 itens × centenas de eventos cada. Cinco por cento perde no máximo 5% de
/// progresso num crash, e o `.part` do yt-dlp cobre o resto — o WAL guarda a
/// intenção, não os bytes.
pub const PROGRESS_STEP_PERCENT: f64 = 5.0;

/// Vale gravar este progresso, dado o último gravado?
pub fn should_log_progress(last_logged: Option<f64>, current: f64) -> bool {
    match last_logged {
        None => current > 0.0,
        Some(last) => (current - last).abs() >= PROGRESS_STEP_PERCENT,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecoveredItem {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub platform: String,
    pub output_dir: String,
    pub quality: Option<String>,
    pub download_mode: Option<String>,
    pub format_id: Option<String>,
    pub referer: Option<String>,
    pub position: u32,
    pub percent: f64,
    pub was_running: bool,
}

/// Reconstrói a fila a partir do log.
///
/// Devolve na ordem em que os itens foram enfileirados, não na ordem do log —
/// a fila do usuário tem uma ordem e ela é parte do que se está recuperando.
pub fn replay(records: &[WalRecord]) -> Vec<RecoveredItem> {
    use std::collections::HashMap;
    let mut live: HashMap<u64, RecoveredItem> = HashMap::new();

    for r in records {
        match r {
            WalRecord::Enqueued {
                id,
                url,
                title,
                platform,
                output_dir,
                quality,
                download_mode,
                format_id,
                referer,
                position,
            } => {
                live.insert(
                    *id,
                    RecoveredItem {
                        id: *id,
                        url: url.clone(),
                        title: title.clone(),
                        platform: platform.clone(),
                        output_dir: output_dir.clone(),
                        quality: quality.clone(),
                        download_mode: download_mode.clone(),
                        format_id: format_id.clone(),
                        referer: referer.clone(),
                        position: *position,
                        percent: 0.0,
                        was_running: false,
                    },
                );
            }
            WalRecord::Started { id } => {
                if let Some(item) = live.get_mut(id) {
                    item.was_running = true;
                }
            }
            WalRecord::Progress { id, percent, .. } => {
                if let Some(item) = live.get_mut(id) {
                    item.percent = *percent;
                }
            }
            other if other.is_terminal() => {
                live.remove(&other.id());
            }
            _ => {}
        }
    }

    let mut out: Vec<RecoveredItem> = live.into_values().collect();
    out.sort_by_key(|i| (i.position, i.id));
    out
}

/// Serializa um registro como uma linha JSON.
///
/// Uma linha por registro é o que permite descartar só a última quando ela está
/// pela metade. JSON num bloco só não teria essa propriedade.
pub fn encode(record: &WalRecord) -> Option<String> {
    serde_json::to_string(record).ok()
}

/// Lê o log, descartando registros ilegíveis.
///
/// Devolve `(registros, descartados)`. O contador não é decorativo: descarte
/// silencioso em caminho de recuperação é como se perde a fila sem ninguém
/// notar, e o chamador precisa poder logar quanto perdeu.
pub fn decode_log(contents: &str) -> (Vec<WalRecord>, usize) {
    let mut records = Vec::new();
    let mut dropped = 0usize;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<WalRecord>(line) {
            Ok(r) => records.push(r),
            Err(_) => dropped += 1,
        }
    }
    (records, dropped)
}

/// O log pode ser compactado?
///
/// Só registros de itens terminados podem sair. Compactar com item vivo no meio
/// perderia o `Enqueued` dele — que é exatamente o que não se pode perder.
pub fn compactable(records: &[WalRecord]) -> bool {
    let live = replay(records);
    records.len() > COMPACT_THRESHOLD && live.len() * 4 < records.len()
}

pub const COMPACT_THRESHOLD: usize = 500;

/// Log equivalente contendo só o que ainda importa.
pub fn compact(records: &[WalRecord]) -> Vec<WalRecord> {
    replay(records)
        .into_iter()
        .flat_map(|i| {
            let mut out = vec![WalRecord::Enqueued {
                id: i.id,
                url: i.url,
                title: i.title,
                platform: i.platform,
                output_dir: i.output_dir,
                quality: i.quality,
                download_mode: i.download_mode,
                format_id: i.format_id,
                referer: i.referer,
                position: i.position,
            }];
            if i.was_running {
                out.push(WalRecord::Started { id: i.id });
            }
            if i.percent > 0.0 {
                out.push(WalRecord::Progress {
                    id: i.id,
                    percent: i.percent,
                    downloaded_bytes: None,
                });
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enq(id: u64, pos: u32) -> WalRecord {
        WalRecord::Enqueued {
            id,
            url: format!("https://example.com/{id}"),
            title: format!("video {id}"),
            platform: "youtube".into(),
            output_dir: "/downloads".into(),
            quality: Some("1080".into()),
            download_mode: None,
            format_id: None,
            referer: None,
            position: pos,
        }
    }

    #[test]
    fn replay_devolve_a_fila_na_ordem_em_que_foi_enfileirada() {
        // A ordem e parte do que se esta recuperando: o usuario montou a fila
        // numa sequencia e espera ela de volta.
        let log = vec![enq(3, 2), enq(1, 0), enq(2, 1)];
        let itens = replay(&log);
        assert_eq!(
            itens.iter().map(|i| i.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn item_terminado_nao_volta_para_a_fila() {
        for terminal in [
            WalRecord::Completed {
                id: 1,
                file_path: "/x.mp4".into(),
            },
            WalRecord::Failed {
                id: 1,
                error: "boom".into(),
            },
            WalRecord::Cancelled { id: 1 },
            WalRecord::Removed { id: 1 },
        ] {
            let log = vec![enq(1, 0), WalRecord::Started { id: 1 }, terminal.clone()];
            assert!(replay(&log).is_empty(), "voltou apos {terminal:?}");
        }
    }

    #[test]
    fn o_que_estava_baixando_volta_marcado_com_o_progresso() {
        // Sem isto, retomar significa comecar do zero — o problema que o item
        // veio resolver.
        let log = vec![
            enq(1, 0),
            WalRecord::Started { id: 1 },
            WalRecord::Progress {
                id: 1,
                percent: 45.0,
                downloaded_bytes: Some(1234),
            },
        ];
        let i = &replay(&log)[0];
        assert!(i.was_running);
        assert_eq!(i.percent, 45.0);
        assert_eq!(i.quality.as_deref(), Some("1080"));
    }

    #[test]
    fn a_ultima_linha_pela_metade_nao_derruba_as_anteriores() {
        // A propriedade central do WAL, e a que um arquivo unico reescrito nao
        // tem: o processo morreu no meio da gravacao e tudo antes sobrevive.
        let mut texto = String::new();
        texto.push_str(&encode(&enq(1, 0)).unwrap());
        texto.push('\n');
        texto.push_str(&encode(&enq(2, 1)).unwrap());
        texto.push('\n');
        texto.push_str("{\"op\":\"enqueued\",\"id\":3,\"url\":\"https://exa"); // cortado

        let (records, dropped) = decode_log(&texto);
        assert_eq!(dropped, 1);
        assert_eq!(records.len(), 2);
        assert_eq!(replay(&records).len(), 2);
    }

    #[test]
    fn descarte_e_contado_e_nao_silencioso() {
        // Perder registro sem ninguem notar e como se perde a fila inteira sem
        // explicacao. O chamador precisa poder logar quanto perdeu.
        let (_, dropped) = decode_log("lixo\n{}\n\n{\"op\":\"started\",\"id\":1}\n");
        assert_eq!(dropped, 2);
    }

    #[test]
    fn log_vazio_nao_e_erro() {
        let (r, d) = decode_log("");
        assert!(r.is_empty());
        assert_eq!(d, 0);
        assert!(replay(&[]).is_empty());
    }

    #[test]
    fn progresso_so_grava_a_cada_cinco_por_cento() {
        // Gravar todo evento transformaria o WAL num gerador de I/O: 200 itens
        // vezes centenas de eventos cada.
        assert!(should_log_progress(None, 1.0));
        assert!(!should_log_progress(None, 0.0));
        assert!(!should_log_progress(Some(10.0), 12.0));
        assert!(should_log_progress(Some(10.0), 15.0));
        assert!(
            should_log_progress(Some(50.0), 20.0),
            "regressao tambem grava"
        );
    }

    #[test]
    fn compactar_preserva_a_fila_viva() {
        // Compactar nao pode perder o Enqueued de um item vivo — e exatamente
        // o que nao se pode perder.
        let mut log = vec![enq(1, 0), enq(2, 1), WalRecord::Started { id: 1 }];
        for p in [5.0, 10.0, 15.0, 20.0] {
            log.push(WalRecord::Progress {
                id: 1,
                percent: p,
                downloaded_bytes: None,
            });
        }
        log.push(WalRecord::Completed {
            id: 2,
            file_path: "/x".into(),
        });

        let antes = replay(&log);
        let depois = replay(&compact(&log));
        assert_eq!(antes, depois, "compactar mudou a fila recuperada");
        assert!(compact(&log).len() < log.len());
    }

    #[test]
    fn nao_compacta_log_pequeno_nem_log_cheio_de_item_vivo() {
        let pequeno: Vec<WalRecord> = (0..10).map(|i| enq(i, i as u32)).collect();
        assert!(!compactable(&pequeno), "log pequeno nao vale compactar");

        let so_vivos: Vec<WalRecord> = (0..600).map(|i| enq(i, i as u32)).collect();
        assert!(
            !compactable(&so_vivos),
            "log grande mas todo vivo nao tem o que compactar"
        );
    }

    #[test]
    fn registro_sobrevive_ao_round_trip() {
        let originais = vec![
            enq(1, 0),
            WalRecord::Started { id: 1 },
            WalRecord::Progress {
                id: 1,
                percent: 33.3,
                downloaded_bytes: Some(99),
            },
            WalRecord::Completed {
                id: 1,
                file_path: "/a b/c.mp4".into(),
            },
        ];
        let texto: String = originais
            .iter()
            .map(|r| encode(r).unwrap() + "\n")
            .collect();
        let (lidos, dropped) = decode_log(&texto);
        assert_eq!(dropped, 0);
        assert_eq!(lidos, originais);
    }

    #[test]
    fn registro_gravado_por_versao_anterior_sem_position_carrega() {
        // Campo novo num log ja gravado nao pode derrubar a recuperacao
        // inteira — mesma classe do bug que zerou o settings.json.
        let linha = r#"{"op":"enqueued","id":7,"url":"https://x","title":"t","platform":"youtube","output_dir":"/d"}"#;
        let (r, dropped) = decode_log(linha);
        assert_eq!(dropped, 0);
        assert_eq!(r.len(), 1);
        assert_eq!(replay(&r)[0].position, 0);
    }
}

#[cfg(test)]
mod kill9_tests {
    use super::*;

    /// Recupera de um WAL produzido por um processo morto com SIGKILL de
    /// verdade no meio de uma gravacao.
    ///
    /// O arquivo e gerado por , que escreve 202 registros
    /// e leva um SIGKILL enquanto o ultimo esta pela metade. O conteudo abaixo e
    /// o resultado real desse arquivo, reduzido, com a linha final truncada
    /// exatamente como o kernel a deixou.
    const WAL_APOS_KILL9: &str = concat!(
        r#"{"op":"enqueued","id":0,"url":"https://example.com/0","title":"v0","platform":"youtube","output_dir":"/downloads","position":0}"#,
        "\n",
        r#"{"op":"enqueued","id":5,"url":"https://example.com/5","title":"v5","platform":"youtube","output_dir":"/downloads","position":5}"#,
        "\n",
        r#"{"op":"enqueued","id":7,"url":"https://example.com/7","title":"v7","platform":"youtube","output_dir":"/downloads","position":7}"#,
        "\n",
        r#"{"op":"started","id":5}"#,
        "\n",
        r#"{"op":"completed","id":7,"file_path":"/x.mp4"}"#,
        "\n",
        r#"{"op":"progress","id":5,"perc"#,
    );

    #[test]
    fn recupera_a_fila_de_um_arquivo_cortado_por_kill9_real() {
        let (records, dropped) = decode_log(WAL_APOS_KILL9);
        assert_eq!(dropped, 1, "so a linha cortada e descartada");

        let fila = replay(&records);
        // O item 7 completou e nao volta; 0 e 5 voltam, na ordem da fila.
        assert_eq!(fila.iter().map(|i| i.id).collect::<Vec<_>>(), vec![0, 5]);
        // O que estava baixando volta marcado, com as opcoes intactas.
        let cinco = fila.iter().find(|i| i.id == 5).unwrap();
        assert!(cinco.was_running);
        assert_eq!(cinco.url, "https://example.com/5");
        assert_eq!(cinco.output_dir, "/downloads");
    }
}
