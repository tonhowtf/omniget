//! Traduz o stderr do yt-dlp em causa nomeada mais a correcao correspondente.
//!
//! O usuario nao le stderr. Ele le "falhou" e abre uma issue, ou desiste. Este
//! modulo e o que converte o B41, o B42 e o B43 em algo acionavel: sem ele,
//! cada um entrega capacidade que o usuario nao consegue acionar.
//!
//! Cada causa carrega **uma acao**, e a acao e o que a tela vira botao. Causa
//! sem acao correspondente e so um erro melhor escrito, e nao era isso o pedido.
//!
//! A ordem de deteccao importa: a primeira que casar vence, e as mais
//! especificas vem antes. Um 403 por fingerprint e um 403 por conteudo privado
//! parecem iguais no fim da mensagem, e mandar o usuario atras da solucao errada
//! e pior que nao sugerir nada.
//!
//! Origem: painel unico de causa raiz em controle de trafego aereo — uma tela
//! onde cada alarme tem causa e acao ao lado, em vez de uma lista de sintomas.

use serde::Serialize;

/// O que o usuario pode fazer. Vira botao na tela.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Remedy {
    /// Importar cookies do navegador.
    ImportCookies,
    /// Tentar de novo imitando um navegador (`--impersonate`).
    UseImpersonation,
    /// Subir ou configurar o provedor de PO Token.
    SetUpPotProvider,
    /// Trocar o client do YouTube — a cascata do B41 ja faz sozinha.
    SwitchPlayerClient,
    /// Esperar e tentar de novo; nada a configurar.
    WaitAndRetry,
    /// Liberar espaco em disco.
    FreeDiskSpace,
    /// Atualizar o yt-dlp.
    UpdateYtdlp,
    /// Nada que o app saiba sugerir.
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnosis {
    /// Chave de i18n. Nunca texto pronto: a mensagem e traduzida na tela.
    pub cause_key: &'static str,
    pub remedy: Remedy,
    /// Detalhe tecnico, para o painel de debug. Nao vai para o toast.
    pub detail: Option<String>,
}

/// Diagnostica uma falha de download a partir do stderr.
///
/// `None` significa que o app nao sabe — melhor admitir do que inventar causa,
/// porque um diagnostico errado manda o usuario para a solucao errada e ele
/// perde mais tempo do que se nao houvesse diagnostico nenhum.
pub fn diagnose(stderr: &str) -> Option<Diagnosis> {
    let s = stderr.to_lowercase();

    // Mais especificas primeiro. Varias destas terminam em 403 ou em
    // "unavailable", e a ordem e o que evita casar pela razao errada.
    if crate::core::pot_provider::stderr_indicates_missing_token(&s) {
        return Some(Diagnosis {
            cause_key: "error.cause.po_token_required",
            remedy: Remedy::SetUpPotProvider,
            detail: Some("youtube requires a proof-of-origin token".into()),
        });
    }

    if s.contains("sabr") {
        return Some(Diagnosis {
            cause_key: "error.cause.sabr_only",
            remedy: Remedy::SwitchPlayerClient,
            detail: Some("web client returned SABR-only formats".into()),
        });
    }

    if s.contains("http error 429") || s.contains("too many requests") {
        return Some(Diagnosis {
            cause_key: "error.cause.rate_limited",
            remedy: Remedy::WaitAndRetry,
            detail: None,
        });
    }

    if s.contains("no space left") || s.contains("disk full") || s.contains("os error 28") {
        return Some(Diagnosis {
            cause_key: "error.cause.disk_full",
            remedy: Remedy::FreeDiskSpace,
            detail: None,
        });
    }

    // Auth antes de fingerprint: "private video" e "members-only" sao
    // inequivocos, enquanto 403 sozinho e ambiguo.
    if s.contains("private video")
        || s.contains("video is private")
        || s.contains("members-only")
        || s.contains("login required")
        || s.contains("this video is available to this channel's members")
        || s.contains("requires purchase")
        || s.contains("sign in")
    {
        return Some(Diagnosis {
            cause_key: "error.cause.needs_login",
            remedy: Remedy::ImportCookies,
            detail: None,
        });
    }

    if crate::core::impersonation::stderr_indicates_fingerprint_block(&s) {
        return Some(Diagnosis {
            cause_key: "error.cause.tls_fingerprint",
            remedy: Remedy::UseImpersonation,
            detail: Some("site rejected the connection before any content".into()),
        });
    }

    if s.contains("nsig extraction failed") || s.contains("unable to extract") {
        return Some(Diagnosis {
            cause_key: "error.cause.extractor_outdated",
            remedy: Remedy::UpdateYtdlp,
            detail: None,
        });
    }

    None
}

/// Todas as chaves de i18n que a tela precisa ter traduzidas.
///
/// Existe para o teste travar: causa nova sem chave correspondente vira texto
/// cru na interface, que e a forma mais visivel de regressao de i18n.
pub const ALL_CAUSE_KEYS: &[&str] = &[
    "error.cause.po_token_required",
    "error.cause.sabr_only",
    "error.cause.rate_limited",
    "error.cause.disk_full",
    "error.cause.needs_login",
    "error.cause.tls_fingerprint",
    "error.cause.extractor_outdated",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn causa(stderr: &str) -> Option<(&'static str, Remedy)> {
        diagnose(stderr).map(|d| (d.cause_key, d.remedy))
    }

    #[test]
    fn as_cinco_falhas_mais_frequentes_tem_causa_e_acao() {
        // Criterio de aceite do item.
        let casos = [
            (
                "ERROR: [youtube] abc: Sign in to confirm you're not a bot",
                "error.cause.po_token_required",
                Remedy::SetUpPotProvider,
            ),
            (
                "WARNING: [youtube] abc: YouTube is forcing SABR streaming",
                "error.cause.sabr_only",
                Remedy::SwitchPlayerClient,
            ),
            (
                "ERROR: HTTP Error 429: Too Many Requests",
                "error.cause.rate_limited",
                Remedy::WaitAndRetry,
            ),
            (
                "ERROR: unable to write data: [Errno 28] No space left on device",
                "error.cause.disk_full",
                Remedy::FreeDiskSpace,
            ),
            (
                "ERROR: [youtube] abc: Private video. Sign in if you've been granted access",
                "error.cause.needs_login",
                Remedy::ImportCookies,
            ),
        ];
        for (stderr, chave, acao) in casos {
            let d = diagnose(stderr).unwrap_or_else(|| panic!("sem diagnostico: {stderr:?}"));
            assert_eq!(d.cause_key, chave, "stderr: {stderr:?}");
            assert_eq!(d.remedy, acao, "stderr: {stderr:?}");
        }
    }

    #[test]
    fn video_privado_sem_frase_de_bot_manda_importar_cookies() {
        let d = diagnose("ERROR: [youtube] abc: This video is private").unwrap();
        assert_eq!(d.cause_key, "error.cause.needs_login");
        assert_eq!(d.remedy, Remedy::ImportCookies);
    }

    #[test]
    fn a_ordem_evita_mandar_o_usuario_para_a_solucao_errada() {
        // 403 puro e ambiguo e vira fingerprint; 403 acompanhado de
        // "members-only" e conteudo pago e vira cookies. Trocar a ordem faria o
        // usuario instalar impersonation para resolver falta de login.
        assert_eq!(
            causa("ERROR: HTTP Error 403: Forbidden"),
            Some(("error.cause.tls_fingerprint", Remedy::UseImpersonation))
        );
        assert_eq!(
            causa("ERROR: members-only content. HTTP Error 403: Forbidden"),
            Some(("error.cause.needs_login", Remedy::ImportCookies))
        );
    }

    #[test]
    fn falha_desconhecida_admite_que_nao_sabe() {
        // Inventar causa e pior que nao diagnosticar: manda o usuario atras da
        // solucao errada e ele perde mais tempo do que perderia sem nada.
        for desconhecido in [
            "ERROR: something nobody has seen before",
            "ERROR: ffmpeg exited with code 1",
            "",
        ] {
            assert_eq!(
                diagnose(desconhecido),
                None,
                "inventou causa: {desconhecido:?}"
            );
        }
    }

    #[test]
    fn sabr_vence_rate_limit_quando_os_dois_aparecem() {
        // O 429 e consequencia frequente da cascata de client tentando de novo;
        // reportar rate limit esconderia a causa de verdade.
        let d = diagnose("YouTube is forcing SABR streaming\nHTTP Error 429").unwrap();
        assert_eq!(d.cause_key, "error.cause.sabr_only");
    }

    #[test]
    fn toda_causa_esta_na_lista_de_chaves_de_i18n() {
        // Causa nova sem chave vira texto cru na interface.
        let amostras = [
            "sign in to confirm you're not a bot",
            "forcing sabr streaming",
            "http error 429",
            "no space left on device",
            "this video is private",
            "unable to handshake",
            "nsig extraction failed",
        ];
        for a in amostras {
            let d = diagnose(a).unwrap_or_else(|| panic!("sem diagnostico: {a:?}"));
            assert!(
                ALL_CAUSE_KEYS.contains(&d.cause_key),
                "{} nao esta em ALL_CAUSE_KEYS",
                d.cause_key
            );
        }
    }

    #[test]
    fn detalhe_tecnico_nao_vaza_para_a_causa() {
        // O detalhe vai para o painel de debug; a tela mostra a chave
        // traduzida. Sao campos separados de proposito.
        let d = diagnose("YouTube is forcing SABR streaming").unwrap();
        assert!(d.cause_key.starts_with("error.cause."));
        assert!(d.detail.is_some());
        assert!(!d.cause_key.contains(' '));
    }
}
