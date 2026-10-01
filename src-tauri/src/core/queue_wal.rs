use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WalRecord {
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
        #[serde(default)]
        referer: Option<String>,
        #[serde(default)]
        position: u32,
    },
    Started {
        id: u64,
    },
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

pub const PROGRESS_STEP_PERCENT: f64 = 5.0;

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

pub fn encode(record: &WalRecord) -> Option<String> {
    serde_json::to_string(record).ok()
}

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

pub fn compactable(records: &[WalRecord]) -> bool {
    let live = replay(records);
    records.len() > COMPACT_THRESHOLD && live.len() * 4 < records.len()
}

pub const COMPACT_THRESHOLD: usize = 500;

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
        let mut texto = String::new();
        texto.push_str(&encode(&enq(1, 0)).unwrap());
        texto.push('\n');
        texto.push_str(&encode(&enq(2, 1)).unwrap());
        texto.push('\n');
        texto.push_str("{\"op\":\"enqueued\",\"id\":3,\"url\":\"https://exa");

        let (records, dropped) = decode_log(&texto);
        assert_eq!(dropped, 1);
        assert_eq!(records.len(), 2);
        assert_eq!(replay(&records).len(), 2);
    }

    #[test]
    fn descarte_e_contado_e_nao_silencioso() {
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
        // WHY: a parse error must never replace the file: the same class of bug once wiped settings.json.
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
        assert_eq!(fila.iter().map(|i| i.id).collect::<Vec<_>>(), vec![0, 5]);
        let cinco = fila.iter().find(|i| i.id == 5).unwrap();
        assert!(cinco.was_running);
        assert_eq!(cinco.url, "https://example.com/5");
        assert_eq!(cinco.output_dir, "/downloads");
    }
}
