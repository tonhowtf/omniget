//! Leitura de arquivo de estado do usuário sem perda silenciosa.
//!
//! O padrão que este módulo substitui é
//! `read_to_string().ok().and_then(parse).unwrap_or_default()`: quando o
//! arquivo existe mas não desserializa, o chamador recebe "vazio", e a próxima
//! gravação sobrescreve o original. O usuário perde tudo sem ver uma linha de
//! log. Foi assim que o `settings.json` zerou preferências, e o mesmo formato
//! estava replicado no registro de cookies, nos templates do YouTube e nas
//! settings de plugin.
//!
//! Aqui o arquivo ilegível é **posto em quarentena** antes de qualquer default
//! ser devolvido: ele é renomeado para `<nome>.corrupt-<epoch_ms>`, o que
//! impede a gravação seguinte de destruí-lo e deixa uma cópia recuperável.

use std::path::{Path, PathBuf};

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Caminho de quarentena para `path`. Público para os testes e para quem
/// precisar apontar o usuário ao arquivo preservado.
pub fn quarantine_path(path: &Path, stamp_ms: u128) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state.json");
    path.with_file_name(format!("{}.corrupt-{}", name, stamp_ms))
}

/// Renomeia o arquivo ilegível para o caminho de quarentena. Devolve o destino
/// quando conseguiu; `None` quando nem isso foi possível (aí o log é o que
/// resta, e o chamador segue com o default).
pub fn quarantine(path: &Path, label: &str) -> Option<PathBuf> {
    let dest = quarantine_path(path, now_ms());
    match std::fs::rename(path, &dest) {
        Ok(()) => {
            tracing::error!(
                "[state] {} ilegível em {} — preservado como {}. O app seguiu com os valores padrão; \
                 o arquivo original NÃO foi sobrescrito.",
                label,
                path.display(),
                dest.display()
            );
            Some(dest)
        }
        Err(e) => {
            tracing::error!(
                "[state] {} ilegível em {} e não foi possível preservá-lo ({}). \
                 O app seguiu com os valores padrão.",
                label,
                path.display(),
                e
            );
            None
        }
    }
}

/// Lê e desserializa um arquivo de estado.
///
/// - arquivo ausente → `None`, sem ruído (é o caso normal na primeira execução)
/// - arquivo ilegível ou corrompido → quarentena + log de erro + `None`
///
/// Nunca devolve default silenciosamente: o `None` aqui sempre significa
/// "não havia estado utilizável", e o motivo está no log quando não for a
/// primeira execução.
pub fn load_or_quarantine<T: serde::de::DeserializeOwned>(path: &Path, label: &str) -> Option<T> {
    if !path.exists() {
        return None;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[state] não foi possível ler {}: {}", path.display(), e);
            return None;
        }
    };
    // Arquivo vazio (queda no meio de uma gravação não atômica) conta como
    // ausente, não como corrompido — não há o que preservar.
    if content.trim().is_empty() {
        tracing::warn!("[state] {} vazio em {}", label, path.display());
        return None;
    }
    match serde_json::from_str::<T>(&content) {
        Ok(value) => Some(value),
        Err(e) => {
            // O erro do serde nomeia o campo e a posição, nunca o conteúdo —
            // seguro de logar mesmo para arquivo de cookie.
            tracing::error!("[state] {} não desserializa: {}", label, e);
            quarantine(path, label);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Estado {
        contas: Vec<String>,
    }

    fn tmp_dir(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omniget-state-{}-{}", nome, now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn arquivo_ausente_devolve_none_sem_quarentena() {
        let dir = tmp_dir("ausente");
        let path = dir.join("estado.json");
        let lido: Option<Estado> = load_or_quarantine(&path, "estado");
        assert!(lido.is_none());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arquivo_valido_e_lido() {
        let dir = tmp_dir("valido");
        let path = dir.join("estado.json");
        std::fs::write(&path, r#"{"contas":["a","b"]}"#).unwrap();
        let lido: Option<Estado> = load_or_quarantine(&path, "estado");
        assert_eq!(
            lido,
            Some(Estado {
                contas: vec!["a".into(), "b".into()]
            })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arquivo_corrompido_vai_para_quarentena_e_o_original_some_do_lugar() {
        // O ponto do teste: depois disso, um save() do chamador escreve num
        // caminho limpo e NÃO destrói o conteúdo que o usuário tinha.
        let dir = tmp_dir("corrompido");
        let path = dir.join("estado.json");
        std::fs::write(&path, "{ isto nao e json").unwrap();

        let lido: Option<Estado> = load_or_quarantine(&path, "estado");
        assert!(lido.is_none());
        assert!(
            !path.exists(),
            "o arquivo corrompido precisa sair do caminho de gravação"
        );

        let preservados: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("estado.json.corrupt-"))
            .collect();
        assert_eq!(
            preservados.len(),
            1,
            "esperava exatamente uma quarentena, achei {preservados:?}"
        );

        let conteudo = std::fs::read_to_string(dir.join(&preservados[0])).unwrap();
        assert_eq!(
            conteudo, "{ isto nao e json",
            "a quarentena precisa ser byte a byte"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arquivo_vazio_nao_gera_quarentena() {
        let dir = tmp_dir("vazio");
        let path = dir.join("estado.json");
        std::fs::write(&path, "   \n").unwrap();
        let lido: Option<Estado> = load_or_quarantine(&path, "estado");
        assert!(lido.is_none());
        assert!(path.exists(), "arquivo vazio nao tem o que preservar");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_valido_de_forma_errada_tambem_e_quarentena() {
        // Regressao: `serde_json::from_str::<T>` falhando por schema (e nao por
        // sintaxe) caia no mesmo `unwrap_or_default()` e zerava o estado.
        let dir = tmp_dir("schema");
        let path = dir.join("estado.json");
        std::fs::write(&path, r#"{"contas": "nao e uma lista"}"#).unwrap();
        let lido: Option<Estado> = load_or_quarantine(&path, "estado");
        assert!(lido.is_none());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quarentenas_repetidas_nao_se_sobrescrevem() {
        let dir = tmp_dir("repetida");
        let a = quarantine_path(&dir.join("estado.json"), 1);
        let b = quarantine_path(&dir.join("estado.json"), 2);
        assert_ne!(a, b);
        assert!(a.to_string_lossy().ends_with("estado.json.corrupt-1"));
    }
}
