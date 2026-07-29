//! Detecta que um video foi reeditado, censurado ou re-uploadado entre dois
//! downloads do mesmo id.
//!
//! O archive de dedup e o historico ja guardam o suficiente para ancorar isto:
//! ao re-baixar um id ja arquivado, compara-se o que mudou em vez de sobrescrever
//! em silencio. Feature de arquivista, e nenhum concorrente faz.
//!
//! Origem: `git diff`, aplicado a metadado de midia.

use serde::{Deserialize, Serialize};

/// O que se guarda de uma versao. Nao guarda o arquivo — so o suficiente para
/// afirmar que algo mudou e o que.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MediaSnapshot {
    pub duration_secs: Option<f64>,
    #[serde(default)]
    pub chapters: Vec<String>,
    /// SHA-256 do arquivo final, quando calculado.
    pub sha256: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Change {
    /// Duracao mudou alem da tolerancia. Corte de censura aparece aqui.
    Duration {
        before: f64,
        after: f64,
        delta: f64,
    },
    ChaptersAdded(Vec<String>),
    ChaptersRemoved(Vec<String>),
    TitleChanged {
        before: String,
        after: String,
    },
    /// Mesma duracao, mesmos chapters, bytes diferentes: re-encode ou
    /// re-upload. O caso mais silencioso, e o que so o hash pega.
    ContentOnly,
}

/// Diferenca menor que isto e ruido de remux, nao edicao.
///
/// Meio segundo: um remux muda a duracao em milissegundos por causa de
/// arredondamento de timestamp, e um corte de censura real nunca e tao curto.
pub const DURATION_TOLERANCE_SECS: f64 = 0.5;

/// O que mudou entre duas versoes. Vazio significa que nada detectavel mudou.
pub fn diff(before: &MediaSnapshot, after: &MediaSnapshot) -> Vec<Change> {
    let mut changes = Vec::new();

    if let (Some(a), Some(b)) = (before.duration_secs, after.duration_secs) {
        let delta = b - a;
        if delta.abs() > DURATION_TOLERANCE_SECS {
            changes.push(Change::Duration {
                before: a,
                after: b,
                delta,
            });
        }
    }

    let removed: Vec<String> = before
        .chapters
        .iter()
        .filter(|c| !after.chapters.contains(c))
        .cloned()
        .collect();
    if !removed.is_empty() {
        changes.push(Change::ChaptersRemoved(removed));
    }

    let added: Vec<String> = after
        .chapters
        .iter()
        .filter(|c| !before.chapters.contains(c))
        .cloned()
        .collect();
    if !added.is_empty() {
        changes.push(Change::ChaptersAdded(added));
    }

    if let (Some(a), Some(b)) = (&before.title, &after.title) {
        if a != b {
            changes.push(Change::TitleChanged {
                before: a.clone(),
                after: b.clone(),
            });
        }
    }

    // So vale reportar "so o conteudo mudou" quando nada mais mudou; junto de
    // uma mudanca de duracao seria ruido, porque o hash muda por consequencia.
    if changes.is_empty() {
        if let (Some(a), Some(b)) = (&before.sha256, &after.sha256) {
            if a != b {
                changes.push(Change::ContentOnly);
            }
        }
    }

    changes
}

/// Resumo de uma linha, para o historico.
pub fn summarize(changes: &[Change]) -> Option<String> {
    if changes.is_empty() {
        return None;
    }
    let parts: Vec<String> = changes
        .iter()
        .map(|c| match c {
            Change::Duration { delta, .. } if *delta < 0.0 => {
                format!("{:.0}s shorter", delta.abs())
            }
            Change::Duration { delta, .. } => format!("{delta:.0}s longer"),
            Change::ChaptersRemoved(v) => format!("{} chapter(s) removed", v.len()),
            Change::ChaptersAdded(v) => format!("{} chapter(s) added", v.len()),
            Change::TitleChanged { .. } => "title changed".to_string(),
            Change::ContentOnly => "re-encoded or re-uploaded".to_string(),
        })
        .collect();
    Some(parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(dur: f64, chapters: &[&str], hash: &str) -> MediaSnapshot {
        MediaSnapshot {
            duration_secs: Some(dur),
            chapters: chapters.iter().map(|s| s.to_string()).collect(),
            sha256: Some(hash.to_string()),
            title: None,
        }
    }

    #[test]
    fn identico_nao_reporta_nada() {
        let a = snap(600.0, &["intro", "demo"], "aaa");
        assert!(diff(&a, &a).is_empty());
        assert_eq!(summarize(&[]), None);
    }

    #[test]
    fn remux_nao_conta_como_edicao() {
        // Arredondamento de timestamp muda a duracao em milissegundos; reportar
        // isso como "video foi editado" treinaria o usuario a ignorar o aviso.
        let a = snap(600.0, &["intro"], "aaa");
        let b = snap(600.2, &["intro"], "bbb");
        let c = diff(&a, &b);
        assert_eq!(c, vec![Change::ContentOnly], "{c:?}");
    }

    #[test]
    fn corte_de_censura_aparece_como_duracao_menor() {
        let a = snap(600.0, &["intro", "polemica", "fim"], "aaa");
        let b = snap(540.0, &["intro", "fim"], "bbb");
        let c = diff(&a, &b);
        assert!(
            matches!(c[0], Change::Duration { delta, .. } if delta < -59.0),
            "{c:?}"
        );
        assert_eq!(c[1], Change::ChaptersRemoved(vec!["polemica".into()]));
        assert_eq!(summarize(&c).unwrap(), "60s shorter, 1 chapter(s) removed");
    }

    #[test]
    fn re_upload_sem_mudanca_visivel_so_aparece_pelo_hash() {
        // O caso mais silencioso: mesma duracao, mesmos chapters, bytes
        // diferentes. Sem o hash nao existiria sinal nenhum.
        let a = snap(600.0, &["intro"], "aaa");
        let b = snap(600.0, &["intro"], "zzz");
        assert_eq!(diff(&a, &b), vec![Change::ContentOnly]);
    }

    #[test]
    fn content_only_nao_polui_quando_algo_maior_mudou() {
        // O hash muda por consequencia de qualquer edicao; reporta-lo junto
        // seria ruido em toda deteccao real.
        let a = snap(600.0, &["intro"], "aaa");
        let b = snap(300.0, &["intro"], "bbb");
        let c = diff(&a, &b);
        assert!(!c.contains(&Change::ContentOnly), "{c:?}");
    }

    #[test]
    fn metadado_faltando_nao_inventa_mudanca() {
        // Snapshot antigo sem hash ou sem duracao nao pode virar falso positivo.
        let sem_hash = MediaSnapshot {
            duration_secs: Some(600.0),
            chapters: vec![],
            sha256: None,
            title: None,
        };
        let com_hash = snap(600.0, &[], "aaa");
        assert!(diff(&sem_hash, &com_hash).is_empty());

        let sem_dur = MediaSnapshot {
            duration_secs: None,
            ..com_hash.clone()
        };
        assert!(diff(&sem_dur, &com_hash).is_empty());
    }

    #[test]
    fn titulo_mudado_e_reportado() {
        let mut a = snap(600.0, &[], "aaa");
        let mut b = snap(600.0, &[], "aaa");
        a.title = Some("Aula 1".into());
        b.title = Some("Aula 1 (REUPLOAD)".into());
        assert_eq!(
            diff(&a, &b),
            vec![Change::TitleChanged {
                before: "Aula 1".into(),
                after: "Aula 1 (REUPLOAD)".into()
            }]
        );
    }
}
