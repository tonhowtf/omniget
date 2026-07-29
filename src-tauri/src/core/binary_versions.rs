//! Guarda as versoes anteriores de um binario gerenciado, para voltar em um
//! clique quando uma atualizacao quebra um site.
//!
//! Hoje `check_ytdlp_freshness` re-baixa por mtime de 2 dias e a versao anterior
//! e simplesmente sobrescrita: se a nova quebrar, nao ha para onde voltar e o
//! usuario fica sem downloads ate o upstream corrigir.
//!
//! Encaixa no `replace_managed_binary`, que ja e atomico — este modulo so
//! resolve os nomes e a poda, que e a parte decidivel e testavel sem rede.
//!
//! Origem: Nix e pnpm, a ideia de manter a geracao anterior viva.

use std::path::{Path, PathBuf};

/// Quantas versoes anteriores ficam. Tres cobre o caso real (a atualizacao de
/// hoje quebrou, volto para a de ontem) sem virar acumulo de 100 MB.
pub const KEEP_VERSIONS: usize = 3;

const SUFFIX: &str = ".omniget-prev.";

/// Caminho de arquivamento de `binary`, carimbado.
///
/// O carimbo vai no fim e nao no meio do nome para o `find_tool` continuar
/// enxergando so o binario ativo: ele procura por nome exato, e um `yt-dlp`
/// arquivado como `yt-dlp.omniget-prev.123` nao colide.
pub fn archive_path(binary: &Path, stamp_ms: u128) -> PathBuf {
    let name = binary
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("binary");
    binary.with_file_name(format!("{name}{SUFFIX}{stamp_ms}"))
}

/// `Some(carimbo)` quando o arquivo e um arquivamento de `binary_name`.
pub fn parse_archive(file_name: &str, binary_name: &str) -> Option<u128> {
    let rest = file_name.strip_prefix(binary_name)?;
    let stamp = rest.strip_prefix(SUFFIX)?;
    stamp.parse().ok()
}

/// Versoes arquivadas de `binary`, da mais nova para a mais antiga.
pub fn list_archived(binary: &Path) -> Vec<(u128, PathBuf)> {
    let (Some(dir), Some(name)) = (binary.parent(), binary.file_name().and_then(|n| n.to_str()))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut out: Vec<(u128, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let fname = e.file_name();
            let fname = fname.to_str()?;
            parse_archive(fname, name).map(|stamp| (stamp, e.path()))
        })
        .collect();
    out.sort_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
    out
}

/// Quais arquivamentos devem ser apagados para respeitar `KEEP_VERSIONS`.
///
/// Separado da remocao de proposito: decidir o que apagar e testavel sem tocar
/// em disco, e apagar arquivo e a parte que nao se quer errar.
pub fn prune_list(archived: &[(u128, PathBuf)], keep: usize) -> Vec<PathBuf> {
    archived.iter().skip(keep).map(|(_, p)| p.clone()).collect()
}

/// Move o binario atual para o arquivo carimbado, antes de instalar o novo.
pub fn archive_current(binary: &Path, stamp_ms: u128) -> std::io::Result<Option<PathBuf>> {
    if !binary.exists() {
        return Ok(None);
    }
    let dest = archive_path(binary, stamp_ms);
    std::fs::rename(binary, &dest)?;

    for old in prune_list(&list_archived(binary), KEEP_VERSIONS) {
        let _ = std::fs::remove_file(old);
    }
    Ok(Some(dest))
}

/// Restaura um arquivamento por cima do binario ativo.
///
/// Recusa caminho que nao seja arquivamento **deste** binario: e um caminho que
/// apaga e sobrescreve, e aceitar um path arbitrario aqui deixaria qualquer
/// chamador substituir um executavel gerenciado.
pub fn rollback_to(binary: &Path, archived: &Path) -> std::io::Result<()> {
    let name = binary
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("binary has no file name"))?;
    let arch_name = archived
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("archive has no file name"))?;

    if parse_archive(arch_name, name).is_none() {
        return Err(std::io::Error::other(format!(
            "{arch_name} is not an archive of {name}"
        )));
    }
    if archived.parent() != binary.parent() {
        return Err(std::io::Error::other(
            "archive must live beside the managed binary",
        ));
    }

    std::fs::rename(archived, binary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(nome: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "omniget-bv-{nome}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn o_arquivamento_nao_colide_com_a_busca_pelo_binario_ativo() {
        // find_tool procura por nome exato; o carimbo tem que ir no fim.
        let p = archive_path(Path::new("/bin/yt-dlp"), 1234);
        assert_eq!(
            p.file_name().unwrap().to_str().unwrap(),
            "yt-dlp.omniget-prev.1234"
        );
        assert_ne!(p.file_name().unwrap(), "yt-dlp");
        assert_eq!(
            parse_archive("yt-dlp.omniget-prev.1234", "yt-dlp"),
            Some(1234)
        );
        assert_eq!(parse_archive("yt-dlp", "yt-dlp"), None);
        assert_eq!(parse_archive("ffmpeg.omniget-prev.1", "yt-dlp"), None);
    }

    #[test]
    fn arquivar_move_e_lista_da_mais_nova_para_a_mais_antiga() {
        let dir = tmp("lista");
        let bin = dir.join("yt-dlp");

        for stamp in [100u128, 300, 200] {
            std::fs::write(&bin, format!("versao {stamp}")).unwrap();
            archive_current(&bin, stamp).unwrap();
            assert!(!bin.exists(), "o binario ativo devia ter saido do lugar");
        }

        let arch = list_archived(&bin);
        assert_eq!(
            arch.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec![300, 200, 100]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_poda_mantem_exatamente_as_n_mais_novas() {
        let arch: Vec<(u128, PathBuf)> = (1..=6)
            .rev()
            .map(|i| (i as u128, PathBuf::from(format!("/x/b.omniget-prev.{i}"))))
            .collect();
        let apagar = prune_list(&arch, 3);
        assert_eq!(apagar.len(), 3);
        // Apaga as mais antigas, nunca as mais novas.
        assert!(apagar.iter().all(|p| {
            let n: u128 = p
                .to_str()
                .unwrap()
                .rsplit('.')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            n <= 3
        }));
        assert!(prune_list(&arch, 10).is_empty());
    }

    #[test]
    fn arquivar_poda_sozinho_ao_passar_do_limite() {
        let dir = tmp("poda");
        let bin = dir.join("yt-dlp");
        for stamp in 1..=(KEEP_VERSIONS as u128 + 2) {
            std::fs::write(&bin, "x").unwrap();
            archive_current(&bin, stamp).unwrap();
        }
        assert_eq!(list_archived(&bin).len(), KEEP_VERSIONS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_restaura_o_conteudo_da_versao_escolhida() {
        let dir = tmp("rollback");
        let bin = dir.join("yt-dlp");

        std::fs::write(&bin, "versao boa").unwrap();
        let arquivado = archive_current(&bin, 111).unwrap().unwrap();
        std::fs::write(&bin, "versao quebrada").unwrap();

        rollback_to(&bin, &arquivado).unwrap();
        assert_eq!(std::fs::read_to_string(&bin).unwrap(), "versao boa");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_recusa_caminho_que_nao_e_arquivamento_deste_binario() {
        // Caminho que apaga e sobrescreve executavel: aceitar path arbitrario
        // aqui deixaria qualquer chamador trocar um binario gerenciado.
        let dir = tmp("guarda");
        let bin = dir.join("yt-dlp");
        std::fs::write(&bin, "ativo").unwrap();

        let intruso = dir.join("qualquer-coisa");
        std::fs::write(&intruso, "malicioso").unwrap();
        assert!(rollback_to(&bin, &intruso).is_err());

        let de_outro = dir.join("ffmpeg.omniget-prev.1");
        std::fs::write(&de_outro, "outro binario").unwrap();
        assert!(rollback_to(&bin, &de_outro).is_err());

        let fora = std::env::temp_dir().join("yt-dlp.omniget-prev.9");
        std::fs::write(&fora, "de fora").unwrap();
        assert!(
            rollback_to(&bin, &fora).is_err(),
            "arquivamento fora do diretorio gerenciado precisa ser recusado"
        );

        assert_eq!(std::fs::read_to_string(&bin).unwrap(), "ativo");
        let _ = std::fs::remove_file(&fora);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arquivar_binario_inexistente_nao_e_erro() {
        let dir = tmp("ausente");
        let r = archive_current(&dir.join("nao-existe"), 1).unwrap();
        assert!(r.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
