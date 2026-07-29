//! Onde o WebView2 guarda os dados dele, no modo portatil.
//!
//! O `WEBVIEW2_USER_DATA_FOLDER` definido no `main.rs` e codigo morto: o Tauri
//! resolve `data_directory` para `LocalData/{identifier}` antes de a variavel
//! ser lida (`manager/webview.rs`), cria o diretorio, e o wry recebe um caminho
//! nao-vazio — a variavel nunca e consultada. O `tauri.conf.json` tambem nao
//! resolve, porque so aceita caminho relativo, resolvido sob `data_local_dir()`.
//!
//! A saida e nao deixar o Tauri criar a janela (`"create": false`) e construi-la
//! no `setup()` com `.data_directory(...)` explicito. Este modulo isola a parte
//! decidivel — qual diretorio usar — para poder ser testada sem subir o app.

use std::path::{Path, PathBuf};

/// Diretorio de dados do WebView2 quando o app roda em modo portatil.
///
/// `None` quando nao e portatil: nesse caso o comportamento padrao do Tauri
/// esta correto e nao deve ser sobrescrito.
pub fn portable_webview_dir(data_dir: Option<&Path>) -> Option<PathBuf> {
    data_dir.map(|d| d.join("webview"))
}

/// Le o modo portatil do ambiente, do jeito que o `main.rs` o publica.
///
/// Le `OMNIGET_DATA_DIR` em vez de re-detectar o `portable.txt`: a deteccao
/// acontece uma vez so, antes de qualquer coisa subir, e duplicar a regra aqui
/// criaria duas fontes de verdade que divergem no primeiro ajuste.
pub fn portable_webview_dir_from_env() -> Option<PathBuf> {
    if std::env::var("OMNIGET_PORTABLE").ok().as_deref() != Some("1") {
        return None;
    }
    let data_dir = std::env::var("OMNIGET_DATA_DIR").ok()?;
    if data_dir.trim().is_empty() {
        return None;
    }
    portable_webview_dir(Some(Path::new(&data_dir)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nao_portatil_nao_sobrescreve_o_padrao_do_tauri() {
        assert_eq!(portable_webview_dir(None), None);
    }

    #[test]
    fn portatil_fica_ao_lado_do_executavel() {
        let dir = PathBuf::from("/media/pendrive/OmniGet/data");
        let got = portable_webview_dir(Some(&dir)).expect("portatil tem diretorio");
        assert_eq!(got, dir.join("webview"));
        // O ponto da issue #195: nada pode acabar sob o perfil do usuario.
        assert!(got.starts_with(&dir));
    }

    #[test]
    fn caminho_com_espaco_sobrevive() {
        // Windows portatil vive em "C:\Users\x\Downloads\OmniGet 0.7.7\data".
        let dir = PathBuf::from("C:/Users/x/Downloads/OmniGet 0.7.7/data");
        let got = portable_webview_dir(Some(&dir)).expect("portatil tem diretorio");
        assert!(got.to_string_lossy().contains("OmniGet 0.7.7"));
        assert!(got.ends_with("webview"));
    }

    #[test]
    fn leitor_de_ambiente_exige_as_duas_variaveis() {
        // Um teste so, sequencial: `set_var` e global ao processo e os testes
        // rodam em paralelo. Nenhum outro teste toca estas duas variaveis, mas
        // dividir isto em varios seria corrida garantida.
        let restore = (
            std::env::var("OMNIGET_PORTABLE").ok(),
            std::env::var("OMNIGET_DATA_DIR").ok(),
        );

        std::env::remove_var("OMNIGET_PORTABLE");
        std::env::remove_var("OMNIGET_DATA_DIR");
        assert_eq!(
            portable_webview_dir_from_env(),
            None,
            "sem env nao e portatil"
        );

        std::env::set_var("OMNIGET_PORTABLE", "1");
        assert_eq!(
            portable_webview_dir_from_env(),
            None,
            "portatil sem data dir nao pode inventar caminho"
        );

        std::env::set_var("OMNIGET_DATA_DIR", "   ");
        assert_eq!(
            portable_webview_dir_from_env(),
            None,
            "data dir em branco nao pode virar caminho relativo"
        );

        std::env::set_var("OMNIGET_DATA_DIR", "/tmp/omniget-portatil/data");
        assert_eq!(
            portable_webview_dir_from_env(),
            Some(PathBuf::from("/tmp/omniget-portatil/data/webview"))
        );

        std::env::set_var("OMNIGET_PORTABLE", "0");
        assert_eq!(
            portable_webview_dir_from_env(),
            None,
            "so a string \"1\" liga o modo portatil"
        );

        match restore.0 {
            Some(v) => std::env::set_var("OMNIGET_PORTABLE", v),
            None => std::env::remove_var("OMNIGET_PORTABLE"),
        }
        match restore.1 {
            Some(v) => std::env::set_var("OMNIGET_DATA_DIR", v),
            None => std::env::remove_var("OMNIGET_DATA_DIR"),
        }
    }
}

/// Linha unica de boot, emitida antes de qualquer coisa poder sair em silencio.
///
/// O `tauri-plugin-single-instance` chama `std::process::exit(0)` sem logar nada
/// quando detecta outra instancia viva. Sem esta linha, um app que sai por esse
/// caminho e indistinguivel de um que crashou: o log simplesmente para. Custou
/// uma sessao inteira de investigacao descobrir isso, e o usuario que abrir o
/// app duas vezes nao tem nem log para olhar.
///
/// Com a linha, o diagnostico vira trivial: banner presente e nada depois
/// significa que outra instancia assumiu; banner ausente significa que o
/// processo nem chegou a subir.
pub fn startup_banner(version: &str, pid: u32, portable: bool, data_dir: Option<&Path>) -> String {
    let modo = if portable { "portable" } else { "standard" };
    let dir = data_dir
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "<unresolved>".to_string());
    format!("OmniGet {version} starting — pid {pid}, {modo} mode, data dir {dir}")
}

#[cfg(test)]
mod banner_tests {
    use super::*;

    #[test]
    fn banner_nomeia_o_modo_e_o_diretorio() {
        let b = startup_banner("0.7.7", 1234, true, Some(Path::new("/pendrive/data")));
        assert!(b.contains("0.7.7"));
        assert!(b.contains("pid 1234"));
        assert!(b.contains("portable mode"));
        assert!(b.contains("/pendrive/data"));
    }

    #[test]
    fn banner_nao_esconde_diretorio_nao_resolvido() {
        // Se o data dir nao resolveu, dizer isso vale mais do que omitir: e a
        // primeira coisa a olhar quando o app se comporta como se nao tivesse
        // configuracao nenhuma.
        let b = startup_banner("0.7.7", 1, false, None);
        assert!(b.contains("standard mode"));
        assert!(b.contains("<unresolved>"));
    }
}
