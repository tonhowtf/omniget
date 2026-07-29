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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(nome: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "omniget-cas-{nome}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const H: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn o_caminho_e_fatiado_pelos_dois_primeiros_caracteres() {
        let p = object_path(Path::new("/store"), H).unwrap();
        assert_eq!(p, Path::new("/store/e3").join(&H[2..]));
    }

    #[test]
    fn hash_invalido_nao_vira_caminho() {
        // O hash e componente de path: aceitar entrada arbitraria seria
        // travessia de diretorio.
        for ruim in [
            "../../etc/passwd",
            "e3b0",
            "",
            &"z".repeat(64),
            &format!("{}/x", &H[..62]),
        ] {
            assert!(
                object_path(Path::new("/store"), ruim).is_none(),
                "aceitou {ruim:?}"
            );
        }
    }

    #[test]
    fn caixa_do_hash_nao_cria_dois_objetos() {
        let a = object_path(Path::new("/s"), H).unwrap();
        let b = object_path(Path::new("/s"), &H.to_uppercase()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn ingerir_duas_vezes_nao_duplica() {
        let dir = tmp("ingest");
        let store = dir.join("store");
        let src = dir.join("video.mp4");
        std::fs::write(&src, "conteudo").unwrap();

        let o1 = ingest(&store, &src, H).unwrap();
        let o2 = ingest(&store, &src, H).unwrap();
        assert_eq!(o1, o2);
        assert!(o1.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materializar_no_mesmo_volume_compartilha_o_inode() {
        // O ponto da feature: tres qualidades do mesmo video ocupam espaco uma
        // vez. Se isto virar copia, a feature nao existe.
        let dir = tmp("link");
        let store = dir.join("store");
        let src = dir.join("original.mp4");
        std::fs::write(&src, "bytes do video").unwrap();
        let object = ingest(&store, &src, H).unwrap();

        let d1 = dir.join("curso-a/aula.mp4");
        let d2 = dir.join("curso-b/aula.mp4");
        assert_eq!(materialize(&object, &d1).unwrap(), LinkOutcome::Linked);
        assert_eq!(materialize(&object, &d2).unwrap(), LinkOutcome::Linked);

        // Verificacao comportamental, nao por inode: escrever por um link e ler
        // pelo outro so funciona se os dois apontarem para o mesmo arquivo.
        // `same_file` nao consegue responder isso no Windows estavel (as APIs
        // de volume/indice seguem unstable), entao afirmar identidade de inode
        // faria o teste reprovar num lugar onde a feature funciona.
        std::fs::write(&d1, "editado por d1").unwrap();
        assert_eq!(
            std::fs::read_to_string(&d2).unwrap(),
            "editado por d1",
            "d1 e d2 deviam ser o mesmo arquivo"
        );

        #[cfg(unix)]
        {
            assert!(same_file(&d1, &d2));
            // object + src + d1 + d2
            assert!(
                link_count(&object).unwrap() >= 3,
                "esperava links compartilhados"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apagar_uma_copia_nao_afeta_as_outras() {
        let dir = tmp("unlink");
        let store = dir.join("store");
        let src = dir.join("o.bin");
        std::fs::write(&src, "dados").unwrap();
        let object = ingest(&store, &src, H).unwrap();

        let d1 = dir.join("a/x.bin");
        let d2 = dir.join("b/x.bin");
        materialize(&object, &d1).unwrap();
        materialize(&object, &d2).unwrap();

        std::fs::remove_file(&d1).unwrap();
        assert!(!d1.exists());
        assert_eq!(std::fs::read_to_string(&d2).unwrap(), "dados");
        assert!(object.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materializar_por_cima_do_mesmo_conteudo_e_no_op() {
        let dir = tmp("noop");
        let store = dir.join("store");
        let src = dir.join("o.bin");
        std::fs::write(&src, "dados").unwrap();
        let object = ingest(&store, &src, H).unwrap();
        let dest = dir.join("x.bin");

        assert_eq!(materialize(&object, &dest).unwrap(), LinkOutcome::Linked);
        #[cfg(unix)]
        assert_eq!(
            materialize(&object, &dest).unwrap(),
            LinkOutcome::AlreadyPresent
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materializar_substitui_arquivo_diferente_que_estava_no_destino() {
        let dir = tmp("replace");
        let store = dir.join("store");
        let src = dir.join("o.bin");
        std::fs::write(&src, "novo").unwrap();
        let object = ingest(&store, &src, H).unwrap();

        let dest = dir.join("x.bin");
        std::fs::write(&dest, "antigo e diferente").unwrap();
        materialize(&object, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "novo");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
