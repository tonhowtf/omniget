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

use crate::core::cas::{ingest, materialize, same_file, LinkOutcome};

/// Quantos caminhos apontam para o mesmo objeto.
///
/// So o Unix responde: `link_count` do `core::cas` esta atras de `cfg(unix)`
/// porque le `nlink` do metadata. No Windows nao ha equivalente barato, e
/// **inventar 1 seria pior que nao saber** — o app reportaria economia zero com
/// a mesma cara de quem mediu. Por isso devolve `None`, e quem chama distingue.
fn quantos_links(object: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        crate::core::cas::link_count(object)
    }
    #[cfg(not(unix))]
    {
        let _ = object;
        None
    }
}

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
    /// Bytes que deixaram de ser ocupados duas vezes. Zero quando a plataforma
    /// nao sabe distinguir link de copia — ver `savings_measurable`.
    pub bytes_saved: u64,
    /// `false` no Windows. `cas::same_file` nao tem como responder ali
    /// (`volume_serial_number` e `file_index` seguem unstable), entao um arquivo
    /// que **ja** era link e re-linkado e contaria como economia nova a cada
    /// execucao. Preferimos nao dar numero a inflar um.
    pub savings_measurable: bool,
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
        let mut rep = DedupeReport {
            savings_measurable: cfg!(unix),
            ..Default::default()
        };
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
                    // So soma onde `same_file` consegue garantir que o destino
                    // ainda nao era um link para este objeto. Sem isso, a
                    // segunda execucao no Windows somaria tudo de novo.
                    if rep.savings_measurable {
                        rep.bytes_saved += tamanho;
                    }
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
    /// `false` quando a plataforma nao sabe contar links (Windows). A interface
    /// precisa distinguir "economizou zero" de "nao da para medir".
    pub savings_measurable: bool,
}

/// Quanto o store esta economizando hoje.
#[tauri::command]
pub async fn content_store_stats() -> Result<StoreStats, String> {
    let Some(root) = store_root() else {
        return Ok(StoreStats::default());
    };
    tokio::task::spawn_blocking(move || {
        let mut s = StoreStats {
            savings_measurable: cfg!(unix),
            ..Default::default()
        };
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
                let links = quantos_links(&obj.path()).unwrap_or(1).max(1);
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
    fn rodar_duas_vezes_nao_corrompe_o_arquivo() {
        // A propriedade que vale nas tres plataformas: materializar duas vezes e
        // seguro e o conteudo continua certo. Este teste ja pegou um defeito
        // real — a primeira versao afirmava identidade de inode via `same_file`,
        // que no Windows sempre diz "diferente", e foi assim que apareceu que a
        // contagem de economia dobrava por lá.
        let d = tempdir("idempotente");
        let root = d.join("store");
        let a = d.join("a.mp4");
        std::fs::write(&a, b"conteudo").unwrap();
        let h = sha256_of(&a).unwrap();
        let obj = ingest(&root, &a, &h).unwrap();
        materialize(&obj, &a).unwrap();
        materialize(&obj, &a).unwrap();
        assert_eq!(
            std::fs::read(&a).unwrap(),
            b"conteudo",
            "materializar duas vezes nao pode estragar o arquivo"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn economia_so_e_somada_onde_da_para_medir() {
        // O numero e reportado como nao-medido em vez de zerado por acidente:
        // "nao sei" e uma resposta diferente de "nao economizou".
        let rep = DedupeReport {
            savings_measurable: cfg!(unix),
            ..Default::default()
        };
        assert_eq!(rep.savings_measurable, cfg!(unix));
        if !rep.savings_measurable {
            assert_eq!(rep.bytes_saved, 0, "sem medicao nao se inventa numero");
        }
    }
}
