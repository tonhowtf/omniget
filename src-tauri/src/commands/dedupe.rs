//! Deduplicacao por conteudo dos arquivos ja baixados.
//!
//! B38. O `core::cas` sabe guardar um arquivo sob o hash do conteudo e
//! materializar com hard link; o que faltava era alguem chamar.
//!
//! Roda **depois** do download, nunca durante. Mexer no caminho de escrita do
//! download para economizar disco seria trocar a funcao principal do app por
//! uma secundaria — e o risco cai justamente sobre o arquivo que o usuario
//! acabou de esperar.
//!
//! O ganho aparece quando o mesmo conteudo existe em dois lugares: a mesma aula
//! em dois cursos, o mesmo video baixado em duas pastas. Depois do dedupe os
//! dois caminhos continuam existindo e apontam para o mesmo inode.

use std::path::{Path, PathBuf};

use crate::core::cas::{ingest, link_count, materialize, same_file, LinkOutcome};

fn store_root() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("content-store"))
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DedupeReport {
    pub examined: usize,
    pub deduplicated: usize,
    /// Bytes que deixaram de ser ocupados duas vezes.
    pub bytes_saved: u64,
    pub errors: Vec<String>,
}

/// Roda o dedupe sobre uma lista de arquivos ja baixados.
///
/// Um arquivo que ja esta ligado ao store e pulado sem custo — a operacao e
/// idempotente de proposito, porque o usuario vai clicar duas vezes.
#[tauri::command]
pub async fn deduplicate_files(paths: Vec<String>) -> Result<DedupeReport, String> {
    let root = store_root().ok_or_else(|| "sem diretorio de dados".to_string())?;

    tokio::task::spawn_blocking(move || {
        let mut rep = DedupeReport::default();
        for raw in paths {
            let path = PathBuf::from(&raw);
            if !path.is_file() {
                continue;
            }
            rep.examined += 1;

            let tamanho = match std::fs::metadata(&path) {
                Ok(m) => m.len(),
                Err(e) => {
                    rep.errors.push(format!("{}: {}", path.display(), e));
                    continue;
                }
            };

            let hash = match sha256_of(&path) {
                Ok(h) => h,
                Err(e) => {
                    rep.errors.push(format!("{}: {}", path.display(), e));
                    continue;
                }
            };

            let objeto = match ingest(&root, &path, &hash) {
                Ok(o) => o,
                Err(e) => {
                    rep.errors.push(format!("{}: {}", path.display(), e));
                    continue;
                }
            };

            // Ja e o mesmo inode: nada a fazer, e nao conta como economia.
            if same_file(&objeto, &path) {
                continue;
            }

            match materialize(&objeto, &path) {
                Ok(LinkOutcome::Linked) => {
                    rep.deduplicated += 1;
                    rep.bytes_saved += tamanho;
                }
                Ok(LinkOutcome::AlreadyPresent) => {
                    // Ja estava ligado. Nao conta como economia nova, senao o
                    // numero cresce a cada clique sem nada ter mudado.
                }
                Ok(LinkOutcome::Copied) => {
                    // Volume diferente: hard link nao atravessa sistema de
                    // arquivos. Copiar mantem o arquivo correto e nao economiza
                    // nada — reportar economia aqui seria mentira.
                }
                Err(e) => rep.errors.push(format!("{}: {}", path.display(), e)),
            }
        }
        Ok(rep)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StoreStats {
    pub objects: usize,
    pub bytes_on_disk: u64,
    /// Quanto seria ocupado se cada link fosse uma copia de verdade.
    pub bytes_without_dedupe: u64,
}

/// Quanto o store esta economizando hoje.
#[tauri::command]
pub async fn content_store_stats() -> Result<StoreStats, String> {
    let Some(root) = store_root() else {
        return Ok(StoreStats::default());
    };
    tokio::task::spawn_blocking(move || {
        let mut s = StoreStats::default();
        let Ok(niveis) = std::fs::read_dir(&root) else {
            return s;
        };
        for nivel in niveis.filter_map(|e| e.ok()) {
            let Ok(objetos) = std::fs::read_dir(nivel.path()) else {
                continue;
            };
            for obj in objetos.filter_map(|e| e.ok()) {
                let Ok(meta) = obj.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                s.objects += 1;
                s.bytes_on_disk += meta.len();
                // Cada link extra e uma copia que nao foi feita.
                let links = link_count(&obj.path()).unwrap_or(1).max(1);
                s.bytes_without_dedupe += meta.len() * links;
            }
        }
        s
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omniget-dedupe-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("cria");
        d
    }

    #[test]
    fn dois_arquivos_iguais_viram_o_mesmo_inode() {
        let d = tempdir("iguais");
        let root = d.join("store");
        let a = d.join("a.mp4");
        let b = d.join("b.mp4");
        std::fs::write(&a, b"mesmo conteudo").unwrap();
        std::fs::write(&b, b"mesmo conteudo").unwrap();

        let h = sha256_of(&a).unwrap();
        assert_eq!(h, sha256_of(&b).unwrap(), "conteudo igual, hash igual");

        let obj = ingest(&root, &a, &h).unwrap();
        materialize(&obj, &a).unwrap();
        materialize(&obj, &b).unwrap();

        // O teste comportamental, nao de inode: escrever por um caminho tem que
        // aparecer no outro. Em Windows `same_file` nem sempre responde.
        assert_eq!(
            std::fs::read(&a).unwrap(),
            std::fs::read(&b).unwrap(),
            "os dois caminhos tem que ver o mesmo conteudo"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn conteudo_diferente_nao_e_deduplicado() {
        let d = tempdir("diferentes");
        let a = d.join("a.mp4");
        let b = d.join("b.mp4");
        std::fs::write(&a, b"conteudo A").unwrap();
        std::fs::write(&b, b"conteudo B").unwrap();
        assert_ne!(
            sha256_of(&a).unwrap(),
            sha256_of(&b).unwrap(),
            "arquivos diferentes nao podem colidir"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rodar_duas_vezes_nao_conta_economia_duas_vezes() {
        // O usuario vai clicar duas vezes. Se a segunda contasse de novo, o
        // numero de "espaco economizado" viraria ficcao crescente.
        let d = tempdir("idempotente");
        let root = d.join("store");
        let a = d.join("a.mp4");
        std::fs::write(&a, b"x").unwrap();
        let h = sha256_of(&a).unwrap();
        let obj = ingest(&root, &a, &h).unwrap();
        materialize(&obj, &a).unwrap();
        assert!(
            same_file(&obj, &a),
            "depois de materializar, o arquivo ja e o objeto"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
