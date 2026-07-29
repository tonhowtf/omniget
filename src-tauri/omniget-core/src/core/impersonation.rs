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
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let client = cols.next()?.to_string();
            let os = cols.next().map(|s| s.to_string()).filter(|s| s != "-");
            Some(ImpersonateTarget { client, os })
        })
        .collect()
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
