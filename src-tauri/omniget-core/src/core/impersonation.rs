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
