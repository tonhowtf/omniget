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
