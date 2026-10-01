// WHY: Tauri resolves data_directory before WEBVIEW2_USER_DATA_FOLDER is read, so the portable profile path must be passed here.

use std::path::{Path, PathBuf};

pub fn portable_webview_dir(data_dir: Option<&Path>) -> Option<PathBuf> {
    data_dir.map(|d| d.join("webview"))
}

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

pub fn dev_webview_dir_from_env() -> Option<PathBuf> {
    if !cfg!(debug_assertions) || std::env::var("OMNIGET_DEV_PROFILE").ok().as_deref() != Some("1")
    {
        return None;
    }
    crate::core::paths::app_data_dir().map(|d| d.join("webview"))
}

pub fn with_dev_webview_dir<'a, R, M>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
) -> tauri::WebviewWindowBuilder<'a, R, M>
where
    R: tauri::Runtime,
    M: tauri::Manager<R>,
{
    #[cfg(all(debug_assertions, any(windows, target_os = "linux")))]
    if let Some(dir) = dev_webview_dir_from_env() {
        if std::fs::create_dir_all(&dir).is_ok() {
            return builder.data_directory(dir);
        }
    }
    builder
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
        assert!(got.starts_with(&dir));
    }

    #[test]
    fn caminho_com_espaco_sobrevive() {
        let dir = PathBuf::from("C:/Users/x/Downloads/OmniGet 0.7.7/data");
        let got = portable_webview_dir(Some(&dir)).expect("portatil tem diretorio");
        assert!(got.to_string_lossy().contains("OmniGet 0.7.7"));
        assert!(got.ends_with("webview"));
    }

    #[test]
    fn leitor_de_ambiente_exige_as_duas_variaveis() {
        let env = crate::secrets::test_env::EnvGuard::acquire();
        env.set("OMNIGET_PORTABLE", None);
        env.set("OMNIGET_DATA_DIR", None);
        assert_eq!(
            portable_webview_dir_from_env(),
            None,
            "sem env nao e portatil"
        );

        env.set("OMNIGET_PORTABLE", Some("1"));
        assert_eq!(
            portable_webview_dir_from_env(),
            None,
            "portatil sem data dir nao pode inventar caminho"
        );

        env.set("OMNIGET_DATA_DIR", Some("   "));
        assert_eq!(
            portable_webview_dir_from_env(),
            None,
            "data dir em branco nao pode virar caminho relativo"
        );

        env.set("OMNIGET_DATA_DIR", Some("/tmp/omniget-portatil/data"));
        assert_eq!(
            portable_webview_dir_from_env(),
            Some(PathBuf::from("/tmp/omniget-portatil/data/webview"))
        );

        env.set("OMNIGET_PORTABLE", Some("0"));
        assert_eq!(
            portable_webview_dir_from_env(),
            None,
            "so a string \"1\" liga o modo portatil"
        );
    }
}

fn parece_runtime_fixo(name: &str) -> bool {
    name.eq_ignore_ascii_case("WebView2Runtime")
        || name
            .to_ascii_lowercase()
            .starts_with("microsoft.webview2.fixedversionruntime.")
}

fn versao_do_nome(name: &str) -> Vec<u32> {
    name.split('.')
        .filter_map(|p| p.parse::<u32>().ok())
        .collect()
}

pub fn find_fixed_runtime(exe_dir: &Path) -> Option<PathBuf> {
    let mut achados: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(exe_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !parece_runtime_fixo(&name) {
                return None;
            }
            let dir = e.path();
            if !dir.join("msedgewebview2.exe").exists() {
                return None;
            }
            Some((versao_do_nome(&name), dir))
        })
        .collect();

    achados.sort_by(|a, b| a.0.cmp(&b.0));
    achados.pop().map(|(_, dir)| dir)
}

#[cfg(test)]
mod runtime_fixo_tests {
    use super::*;

    fn cria_runtime(raiz: &Path, nome: &str, com_exe: bool) {
        let dir = raiz.join(nome);
        std::fs::create_dir_all(&dir).expect("cria pasta");
        if com_exe {
            std::fs::write(dir.join("msedgewebview2.exe"), b"stub").expect("cria exe");
        }
    }

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omniget-rt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("cria tempdir");
        d
    }

    #[test]
    fn sem_runtime_ao_lado_nao_inventa_caminho() {
        let d = tempdir("vazio");
        std::fs::write(d.join("omniget.exe"), b"stub").expect("cria exe");
        assert_eq!(find_fixed_runtime(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pasta_sem_o_binario_e_recusada() {
        let d = tempdir("sem-exe");
        cria_runtime(&d, "WebView2Runtime", false);
        assert_eq!(
            find_fixed_runtime(&d),
            None,
            "nome certo sem msedgewebview2.exe nao pode ser aceito"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nome_curto_e_aceito() {
        let d = tempdir("curto");
        cria_runtime(&d, "WebView2Runtime", true);
        assert_eq!(find_fixed_runtime(&d), Some(d.join("WebView2Runtime")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nome_do_instalador_da_microsoft_e_aceito() {
        let d = tempdir("ms");
        let nome = "Microsoft.WebView2.FixedVersionRuntime.120.0.2210.91.x64";
        cria_runtime(&d, nome, true);
        assert_eq!(find_fixed_runtime(&d), Some(d.join(nome)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn com_duas_versoes_escolhe_a_maior() {
        let d = tempdir("duas");
        let velha = "Microsoft.WebView2.FixedVersionRuntime.109.0.1518.78.x64";
        let nova = "Microsoft.WebView2.FixedVersionRuntime.120.0.2210.91.x64";
        cria_runtime(&d, nova, true);
        cria_runtime(&d, velha, true);
        let got = find_fixed_runtime(&d).expect("achou runtime");
        assert_eq!(got, d.join(nova));
        assert_ne!(got, d.join(velha), "109 nao pode ganhar de 120");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pasta_qualquer_ao_lado_e_ignorada() {
        let d = tempdir("ruido");
        cria_runtime(&d, "plugins", true);
        cria_runtime(&d, "data", true);
        assert_eq!(find_fixed_runtime(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}

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
        let b = startup_banner("0.7.7", 1, false, None);
        assert!(b.contains("standard mode"));
        assert!(b.contains("<unresolved>"));
    }
}
