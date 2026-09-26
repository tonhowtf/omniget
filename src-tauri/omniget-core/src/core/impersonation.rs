//! Descobre se o yt-dlp empacotado consegue imitar um navegador.
//!
//! Sites com TLS fingerprinting recusam a conexao de um cliente HTTP comum e a
//! falha chega ao usuario como erro que ele nao consegue interpretar. O
//! `curl_cffi` resolve, mas **nao vem em todo build** do yt-dlp.
//!
//! ## Correcao de premissa
//!
//! O backlog dizia "nao vem em todos os builds — detectar ausencia e oferecer
//! instalacao guiada". Verificado no binario que o app empacota hoje
//! (yt-dlp 2026.07.23): ele **tem** `curl_cffi`, com alvos Chrome, Safari,
//! Edge, Firefox e Tor. Entao a deteccao continua certa, mas o caso comum e
//! presenca, nao ausencia — e o valor esta em **saber antes de tentar**, para
//! poder sugerir a flag em vez de deixar a falha vazar crua.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImpersonateTarget {
    pub client: String,
    pub os: Option<String>,
}

impl ImpersonateTarget {
    /// Valor de `--impersonate`, no formato `CLIENT[:OS]`.
    pub fn as_flag_value(&self) -> String {
        match &self.os {
            Some(os) => format!("{}:{}", self.client, os),
            None => self.client.clone(),
        }
    }
}

/// Le a saida de `yt-dlp --list-impersonate-targets`.
///
/// Formato real (2026.07):
/// ```text
/// [info] Available impersonate targets
/// Client          OS           Source
/// --------------------------------------
/// Chrome-133      Macos-15     curl_cffi
/// ```
///
/// Lista vazia significa build sem `curl_cffi`. Isso e diferente de erro ao
/// executar, e quem chama precisa distinguir os dois — por isso a funcao devolve
/// a lista e nao um booleano.
pub fn parse_targets(stdout: &str) -> Vec<ImpersonateTarget> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !l.starts_with('[')) // linha de [info]
        .filter(|l| !l.starts_with('-')) // separador
        .filter(|l| !l.starts_with("Client")) // cabecalho
        // Build sem curl_cffi lista os clientes com "(unavailable)": nao sao
        // alvos, e passar `--impersonate` a esse build so gera outro erro.
        .filter(|l| !l.contains("(unavailable)"))
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let client = cols.next()?.to_string();
            let os = cols.next().map(|s| s.to_string()).filter(|s| s != "-");
            Some(ImpersonateTarget { client, os })
        })
        .collect()
}

/// Qual yt-dlp gerido usar, dado o que existe em disco.
///
/// Ordem medida em 26/09 (macOS, `--version`): onedir 0,34-0,46 s; zipapp no
/// Python do sistema 0,6 s; onefile 13-25 s (o PyInstaller desempacota 72 MB e
/// o Gatekeeper varre tudo a cada processo). O zipapp so vale se o Python dele
/// tiver curl_cffi: sem isso Instagram perde o caminho graphql e Bilibili,
/// Vimeo, TikTok e Reddit caem em 412/401. Um pyz sem impersonate so e usado
/// quando nao ha outro binario gerido.
pub fn pick_ytdlp(
    onedir: Option<std::path::PathBuf>,
    zipapp: Option<std::path::PathBuf>,
    onefile: Option<std::path::PathBuf>,
    zipapp_impersonates: bool,
) -> Option<std::path::PathBuf> {
    if onedir.is_some() {
        return onedir;
    }
    match (zipapp, onefile) {
        (Some(zip), Some(_)) if zipapp_impersonates => Some(zip),
        (_, Some(onefile)) => Some(onefile),
        (zip, None) => zip,
    }
}

/// Tira o `--user-agent` padrao do app quando o comando usa `--impersonate`.
///
/// O yt-dlp manda o `--user-agent` como header fixo e so remove os headers
/// iguais aos seus padroes ao imitar um navegador (networking/impersonate.py
/// 137-143): o nosso Chrome/131 de Windows ia junto com o TLS do Chrome
/// 146/macOS, uma incoerencia detectavel. UA vindo da extensao ou da
/// configuracao fica: e escolha de quem configurou.
pub fn strip_default_user_agent(args: &mut Vec<String>, default_ua: &str) {
    if !args.iter().any(|a| a == "--impersonate") {
        return;
    }
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--user-agent" && args[i + 1] == default_ua {
            args.drain(i..i + 2);
        } else {
            i += 1;
        }
    }
}

/// O stderr indica recusa por fingerprint de TLS?
pub fn stderr_indicates_fingerprint_block(stderr_lower: &str) -> bool {
    const SIGNS: &[&str] = &[
        "unable to handshake",
        "ssl: sslv3_alert_handshake_failure",
        "the read operation timed out",
        "cloudflare",
        "just a moment...",
        "enable javascript and cookies to continue",
        "http error 403: forbidden",
    ];
    SIGNS.iter().any(|s| stderr_lower.contains(s))
}

/// Melhor alvo para tentar de novo, dado o que o build oferece.
///
/// Prefere Chrome de desktop: e o que a maioria dos sites com fingerprinting
/// espera ver, e um alvo mobile ou Tor muda outras coisas alem do fingerprint.
pub fn preferred_target(targets: &[ImpersonateTarget]) -> Option<&ImpersonateTarget> {
    let desktop_chrome = targets.iter().find(|t| {
        t.client.starts_with("Chrome")
            && t.os
                .as_deref()
                .is_some_and(|os| !os.starts_with("Android") && !os.starts_with("Ios"))
    });
    desktop_chrome
        .or_else(|| targets.iter().find(|t| t.client.starts_with("Chrome")))
        .or_else(|| targets.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Saida real do yt-dlp 2026.07.23 empacotado pelo app, com a ordem
    /// embaralhada de proposito: com o Chrome de desktop em primeiro, o teste
    /// de preferencia passaria mesmo se a funcao so pegasse o primeiro alvo.
    const REAL: &str = "\
[info] Available impersonate targets\n\
Client          OS           Source\n\
--------------------------------------\n\
Chrome-99       Android-12   curl_cffi\n\
Safari-17.2     Ios-17.2     curl_cffi\n\
Chrome-131      Android-14   curl_cffi\n\
Chrome-133      Macos-15     curl_cffi\n\
Chrome-136      Macos-15     curl_cffi\n\
Safari-18.0     Ios-18.0     curl_cffi\n\
Tor-14.5        Macos-14     curl_cffi\n";

    #[test]
    fn le_a_saida_real_ignorando_cabecalho_e_separador() {
        let t = parse_targets(REAL);
        assert_eq!(t.len(), 7, "{t:?}");
        let chrome133 = t
            .iter()
            .find(|x| x.client == "Chrome-133")
            .expect("Chrome-133");
        assert_eq!(chrome133.os.as_deref(), Some("Macos-15"));
        assert!(!t.iter().any(|x| x.client.starts_with('[')));
        assert!(!t.iter().any(|x| x.client == "Client"));
    }

    #[test]
    fn build_sem_curl_cffi_da_lista_vazia() {
        // Lista vazia e o sinal de ausencia, e e diferente de erro ao executar
        // o binario — por isso a funcao devolve lista e nao booleano.
        let sem = "[info] Available impersonate targets\nClient          OS           Source\n--------------------------------------\n";
        assert!(parse_targets(sem).is_empty());
        assert!(parse_targets("").is_empty());
    }

    #[test]
    fn prefere_chrome_de_desktop() {
        // Alvo mobile ou Tor muda mais que o fingerprint, e a maioria dos
        // sites que bloqueiam espera ver um Chrome de desktop.
        let t = parse_targets(REAL);
        let escolhido = preferred_target(&t).unwrap();
        assert_ne!(
            escolhido.client, t[0].client,
            "pegar o primeiro alvo nao pode passar neste teste"
        );
        assert_eq!(escolhido.client, "Chrome-133");
        assert_eq!(escolhido.os.as_deref(), Some("Macos-15"));
    }

    #[test]
    fn cai_para_chrome_mobile_quando_nao_ha_desktop() {
        let so_mobile =
            "Chrome-99       Android-12   curl_cffi\nTor-14.5        Macos-14     curl_cffi\n";
        let t = parse_targets(so_mobile);
        assert_eq!(preferred_target(&t).unwrap().client, "Chrome-99");
    }

    #[test]
    fn sem_chrome_nenhum_pega_o_primeiro_disponivel() {
        let t = parse_targets("Safari-18.0     Ios-18.0     curl_cffi\n");
        assert_eq!(preferred_target(&t).unwrap().client, "Safari-18.0");
        assert!(preferred_target(&[]).is_none());
    }

    #[test]
    fn monta_o_valor_da_flag_no_formato_do_yt_dlp() {
        let t = parse_targets(REAL);
        let chrome133 = t.iter().find(|x| x.client == "Chrome-133").unwrap();
        assert_eq!(chrome133.as_flag_value(), "Chrome-133:Macos-15");
        let sem_os = ImpersonateTarget {
            client: "Chrome".into(),
            os: None,
        };
        assert_eq!(sem_os.as_flag_value(), "Chrome");
    }

    /// Saida real do zipapp `yt-dlp.pyz` no python3.14 do Homebrew (sem
    /// curl_cffi), 26/09. As linhas "(unavailable)" nao sao alvos: contar como
    /// alvo fazia o retry passar `--impersonate Chrome` a um build que recusa.
    #[test]
    fn linhas_unavailable_do_pyz_sem_curl_cffi_nao_sao_alvos() {
        let pyz = "[info] Available impersonate targets\n\
Client    OS   Source\n\
--------------------------------------------\n\
Tor       -    curl_cffi>=0.11 (unavailable)\n\
Edge      -    curl_cffi (unavailable)\n\
Firefox   -    curl_cffi>=0.10 (unavailable)\n\
Safari    -    curl_cffi (unavailable)\n\
Chrome    -    curl_cffi (unavailable)\n";
        assert!(parse_targets(pyz).is_empty(), "{:?}", parse_targets(pyz));
    }

    #[test]
    fn onedir_ganha_de_tudo() {
        let pick = pick_ytdlp(
            Some(PathBuf::from("/b/yt-dlp_onedir/yt-dlp_macos")),
            Some(PathBuf::from("/b/yt-dlp.pyz")),
            Some(PathBuf::from("/b/yt-dlp")),
            true,
        );
        assert_eq!(pick, Some(PathBuf::from("/b/yt-dlp_onedir/yt-dlp_macos")));
    }

    #[test]
    fn pyz_sem_curl_cffi_perde_para_binario_com_impersonate() {
        let pick = pick_ytdlp(
            None,
            Some(PathBuf::from("/b/yt-dlp.pyz")),
            Some(PathBuf::from("/b/yt-dlp")),
            false,
        );
        assert_eq!(pick, Some(PathBuf::from("/b/yt-dlp")));
    }

    #[test]
    fn pyz_com_curl_cffi_ganha_do_onefile() {
        let pick = pick_ytdlp(
            None,
            Some(PathBuf::from("/b/yt-dlp.pyz")),
            Some(PathBuf::from("/b/yt-dlp")),
            true,
        );
        assert_eq!(pick, Some(PathBuf::from("/b/yt-dlp.pyz")));
    }

    #[test]
    fn pyz_sem_curl_cffi_so_quando_nao_ha_outro() {
        assert_eq!(
            pick_ytdlp(None, Some(PathBuf::from("/b/yt-dlp.pyz")), None, false),
            Some(PathBuf::from("/b/yt-dlp.pyz"))
        );
        assert_eq!(pick_ytdlp(None, None, None, true), None);
    }

    #[test]
    fn user_agent_padrao_sai_quando_ha_impersonate() {
        let ua = "Mozilla/5.0 (Windows NT 10.0) Chrome/131.0.0.0";
        let mut args: Vec<String> = [
            "--no-warnings",
            "--user-agent",
            ua,
            "--impersonate",
            "Chrome-146:Macos-26",
            "https://x",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        strip_default_user_agent(&mut args, ua);
        assert!(
            !args.iter().any(|a| a == "--user-agent" || a == ua),
            "{args:?}"
        );
        assert_eq!(args.len(), 4);
    }

    #[test]
    fn user_agent_fica_sem_impersonate_ou_quando_e_do_usuario() {
        let ua = "Mozilla/5.0 (Windows NT 10.0) Chrome/131.0.0.0";
        let mut sem: Vec<String> = ["--user-agent", ua, "https://x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        strip_default_user_agent(&mut sem, ua);
        assert_eq!(sem.len(), 3, "sem --impersonate o UA fica");
        let mut do_usuario: Vec<String> = [
            "--user-agent",
            "MeuNavegador/1",
            "--impersonate",
            "Chrome",
            "u",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        strip_default_user_agent(&mut do_usuario, ua);
        assert_eq!(do_usuario.len(), 5, "UA da extensao/config nao e o padrao");
    }

    #[test]
    fn reconhece_bloqueio_por_fingerprint() {
        for c in [
            "error: unable to handshake with the server",
            "error: <title>just a moment...</title> cloudflare",
            "error: http error 403: forbidden",
        ] {
            assert!(stderr_indicates_fingerprint_block(c), "{c:?}");
        }
    }

    #[test]
    fn nao_confunde_falha_comum_com_bloqueio_de_fingerprint() {
        // Sugerir impersonation para um video privado mandaria o usuario
        // atras da solucao errada.
        for c in [
            "error: [youtube] abc: video unavailable",
            "error: http error 429: too many requests",
            "error: unable to download webpage: name or service not known",
            "",
        ] {
            assert!(
                !stderr_indicates_fingerprint_block(c),
                "falso positivo: {c:?}"
            );
        }
    }
}
