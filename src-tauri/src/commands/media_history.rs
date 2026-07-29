//! Guarda o que cada URL era da ultima vez, para saber quando ela mudou.
//!
//! B39. O `core::media_diff` compara dois retratos e diz o que mudou; o que
//! faltava era alguem guardar o retrato anterior. Sem isso, re-baixar um video
//! reeditado sobrescreve em silencio e o usuario descobre assistindo.
//!
//! Um arquivo proprio, chaveado por URL. Nao entra no historico de downloads
//! porque a pergunta e outra: o historico diz o que voce baixou, isto diz o que
//! a fonte era naquele momento.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::core::media_diff::{diff, summarize, MediaSnapshot};

const SNAPSHOTS_FILE: &str = "media-snapshots.json";

/// Quantos retratos guardar. Um por URL ja baixada acumula sem limite num
/// usuario de anos; este teto corta os mais antigos por ordem de insercao.
const MAX_SNAPSHOTS: usize = 2000;

fn file_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(SNAPSHOTS_FILE))
}

fn load() -> BTreeMap<String, MediaSnapshot> {
    let Some(path) = file_path() else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("[media-diff] {} ilegivel, ignorando: {}", path.display(), e);
        BTreeMap::new()
    })
}

fn save(map: &BTreeMap<String, MediaSnapshot>) {
    let Some(path) = file_path() else { return };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(json) = serde_json::to_string(map) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Corta os retratos mais antigos quando passa do teto.
///
/// Separado para poder ser testado: um teto que nunca e exercitado e um vazamento
/// de disco esperando a hora.
pub fn podar(
    mut map: BTreeMap<String, MediaSnapshot>,
    teto: usize,
) -> BTreeMap<String, MediaSnapshot> {
    while map.len() > teto {
        let Some(primeira) = map.keys().next().cloned() else {
            break;
        };
        map.remove(&primeira);
    }
    map
}

/// Compara o retrato novo com o guardado e devolve o resumo, se algo mudou.
///
/// Chamada **antes** de baixar: e o unico momento em que dizer "isto nao e mais
/// o mesmo video" ainda serve para alguma coisa.
#[tauri::command]
pub fn check_media_changed(url: String, current: MediaSnapshot) -> Option<String> {
    let mapa = load();
    let anterior = mapa.get(&url)?;
    let mudancas = diff(anterior, &current);
    summarize(&mudancas)
}

/// Guarda o retrato atual desta URL.
#[tauri::command]
pub fn record_media_snapshot(url: String, snapshot: MediaSnapshot) {
    let mut mapa = load();
    mapa.insert(url, snapshot);
    let mapa = podar(mapa, MAX_SNAPSHOTS);
    save(&mapa);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retrato(dur: f64, sha: &str, titulo: &str) -> MediaSnapshot {
        MediaSnapshot {
            duration_secs: Some(dur),
            chapters: Vec::new(),
            sha256: Some(sha.to_string()),
            title: Some(titulo.to_string()),
        }
    }

    #[test]
    fn video_igual_nao_gera_ruido() {
        // O caso comum: re-baixar o mesmo video. Avisar aqui seria alarme falso,
        // e alarme falso treina o usuario a ignorar o aviso de verdade.
        let a = retrato(120.0, "abc", "Aula 1");
        let mudancas = diff(&a, &a);
        assert!(summarize(&mudancas).is_none());
    }

    #[test]
    fn video_reeditado_e_detectado() {
        let antes = retrato(600.0, "abc", "Aula 1");
        let depois = retrato(540.0, "def", "Aula 1");
        let resumo = summarize(&diff(&antes, &depois)).expect("mudou");
        assert!(!resumo.is_empty());
    }

    #[test]
    fn o_teto_corta_os_mais_antigos() {
        let mut mapa = BTreeMap::new();
        for i in 0..10 {
            mapa.insert(format!("url-{i:02}"), retrato(1.0, "h", "x"));
        }
        let podado = podar(mapa, 4);
        assert_eq!(podado.len(), 4, "o teto tem que valer");
        assert!(
            !podado.contains_key("url-00"),
            "o mais antigo tem que sair primeiro"
        );
        assert!(podado.contains_key("url-09"), "o mais novo tem que ficar");
    }

    #[test]
    fn abaixo_do_teto_nada_e_perdido() {
        let mut mapa = BTreeMap::new();
        mapa.insert("a".to_string(), retrato(1.0, "h", "x"));
        assert_eq!(podar(mapa, 100).len(), 1);
    }
}
