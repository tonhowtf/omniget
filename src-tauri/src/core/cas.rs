//! Store endereçado por conteudo: o mesmo video em tres qualidades, ou o mesmo
//! PDF em quatro cursos, ocupa espaço uma vez.
//!
//! O arquivo real vive no store sob o hash do conteudo, e cada lugar onde ele
//! aparece e um hardlink. Apagar uma copia nao apaga as outras — o kernel so
//! libera o espaço quando o ultimo link some.
//!
//! Origem: restic e borg. O chunking definido por conteudo deles fica de fora:
//! aqui o arquivo inteiro e a unidade, o que perde dedup parcial mas dispensa
//! formato proprio e mantem os arquivos legiveis por qualquer programa.
//!
//! **Limite conhecido:** hardlink nao atravessa sistema de arquivos. O app deixa
//! o usuario escolher a pasta de saida, entao a copia e o caminho normal, nao a
//! excecao.

use std::path::{Path, PathBuf};

/// Caminho de um conteudo dentro do store, fatiado pelos dois primeiros
/// caracteres do hash.
///
/// O fatiamento existe porque um diretorio unico com dezenas de milhares de
/// entradas degrada listagem em qualquer sistema de arquivos.
///
/// Rejeita hash que nao seja SHA-256 hexadecimal: o valor vira componente de
/// caminho, e aceitar entrada arbitraria aqui seria travessia de diretorio.
pub fn object_path(store_root: &Path, sha256: &str) -> Option<PathBuf> {
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let lower = sha256.to_lowercase();
    let (prefix, rest) = lower.split_at(2);
    Some(store_root.join(prefix).join(rest))
}

/// Como o arquivo foi materializado no destino.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOutcome {
    /// Hardlink: espaco compartilhado com o store.
    Linked,
    /// Copia: destino em outro sistema de arquivos, ou hardlink recusado.
    Copied,
    /// Destino ja apontava para o mesmo conteudo.
    AlreadyPresent,
}

/// Coloca `sha256` no store, a partir de `source`, sem duplicar se ja existir.
pub fn ingest(store_root: &Path, source: &Path, sha256: &str) -> std::io::Result<PathBuf> {
    let object = object_path(store_root, sha256)
        .ok_or_else(|| std::io::Error::other(format!("invalid sha256: {sha256:?}")))?;

    if object.exists() {
        return Ok(object);
    }
    if let Some(parent) = object.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Hardlink primeiro: ingerir nao deve duplicar o espaco enquanto o arquivo
    // de origem ainda existe.
    match std::fs::hard_link(source, &object) {
        Ok(()) => Ok(object),
        Err(_) => {
            std::fs::copy(source, &object)?;
            Ok(object)
        }
    }
}

/// Materializa o objeto do store em `dest`, preferindo hardlink.
pub fn materialize(object: &Path, dest: &Path) -> std::io::Result<LinkOutcome> {
    if dest.exists() && same_file(object, dest) {
        return Ok(LinkOutcome::AlreadyPresent);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    match std::fs::hard_link(object, dest) {
        Ok(()) => Ok(LinkOutcome::Linked),
        Err(_) => {
            // Volume diferente e o caso normal, nao a excecao: a pasta de saida
            // e escolhida pelo usuario e costuma estar em outro disco.
            std::fs::copy(object, dest)?;
            Ok(LinkOutcome::Copied)
        }
    }
}

/// Dois caminhos apontam para o mesmo inode?
pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        // Sem `dev`/`ino` no Windows estavel (`volume_serial_number` e
        // `file_index` seguem unstable), a resposta conservadora e "nao sei,
        // trate como diferente". Efeito pratico: `materialize` sobre um destino
        // que ja e o mesmo arquivo refaz o link em vez de devolver
        // `AlreadyPresent`. Idempotente e correto, so um pouco mais caro.
        let _ = (a, b);
        false
    }
}

/// Quantos links apontam para este objeto. `1` significa que so o store o
/// segura e apagar libera espaco de verdade.
#[cfg(unix)]
pub fn link_count(object: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(object).ok().map(|m| m.nlink())
}
