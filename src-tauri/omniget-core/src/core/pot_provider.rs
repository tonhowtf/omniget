//! Cliente do provedor de PO Token do YouTube.
//!
//! O YouTube passou a exigir Proof-of-Origin Token em parte dos clients, ligado
//! ao video e a sessao, com expiracao curta. Sem token, o download degrada para
//! formato pior ou e recusado — e a mensagem que chega ao usuario nao diz nada
//! disso.
//!
//! ## O que este modulo NAO faz, e por que
//!
//! O backlog descrevia "rodar o bgutil-ytdlp-pot-provider como servico lateral
//! gerenciado pelo app". Verificado contra o projeto real (1.3.1, marco/2026):
//! ele roda como **container Docker ou servidor Node**, e exige alem disso um
//! **plugin Python instalado dentro do yt-dlp**. Empacotar isso significaria o
//! app gerenciar um runtime Node com dependencias nativas e um diretorio de
//! plugin do yt-dlp — outra ordem de grandeza, e uma superficie de falha que o
//! usuario nao consegue diagnosticar.
//!
//! Entao aqui fica o lado que e do app: **descobrir se existe um provedor
//! acessivel, apontar o yt-dlp para ele, e nomear a falha quando nao houver.**
//! Empacotar o provedor fica registrado como divida, com o motivo.

use serde::Serialize;

/// Onde o bgutil escuta por padrao quando rodado local.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:4416";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProviderHealth {
    /// Respondeu e esta pronto.
    Ready { version: Option<String> },
    /// Endereco configurado mas ninguem atende.
    Unreachable { base_url: String },
    /// Atendeu mas respondeu erro — versao incompativel, normalmente.
    Unhealthy { detail: String },
    /// Nenhum provedor configurado. Nao e erro: e o estado padrao.
    NotConfigured,
}

impl ProviderHealth {
    pub fn is_usable(&self) -> bool {
        matches!(self, ProviderHealth::Ready { .. })
    }
}

/// Normaliza o endereco do provedor.
///
/// Aceita o que o usuario provavelmente digita (`localhost:4416`, com ou sem
/// barra no fim) e recusa o que viraria argumento quebrado no yt-dlp: espaco em
/// branco entra na string de `--extractor-args` e parte o argumento em dois.
pub fn normalize_base_url(raw: &str) -> Option<String> {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    let with_scheme = if t.starts_with("http://") || t.starts_with("https://") {
        t.to_string()
    } else {
        format!("http://{t}")
    };
    Some(with_scheme)
}

/// Argumentos que apontam o yt-dlp para o provedor.
///
/// Vazio quando nao ha provedor utilizavel: mandar a flag apontando para um
/// endereco morto faz o yt-dlp esperar o timeout em **todo** download, o que e
/// pior que nao ter provedor nenhum.
pub fn extractor_args(health: &ProviderHealth, base_url: &str) -> Vec<String> {
    if !health.is_usable() {
        return Vec::new();
    }
    let Some(url) = normalize_base_url(base_url) else {
        return Vec::new();
    };
    vec![
        "--extractor-args".to_string(),
        format!("youtubepot-bgutilhttp:base_url={url}"),
    ]
}

/// O stderr indica que faltou PO Token?
///
/// Casa pelas frases que o yt-dlp realmente emite quando o token falta ou
/// expira, e nao por "po_token" solto — a string aparece tambem em log de
/// sucesso quando o token foi fornecido.
pub fn stderr_indicates_missing_token(stderr_lower: &str) -> bool {
    const SIGNS: &[&str] = &[
        "a po token is required",
        "po token is missing",
        "missing a po token",
        "content is not available on this app",
        "please sign in to confirm you're not a bot",
        "sign in to confirm you're not a bot",
    ];
    SIGNS.iter().any(|s| stderr_lower.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_o_que_o_usuario_digita() {
        for (entrada, esperado) in [
            ("127.0.0.1:4416", "http://127.0.0.1:4416"),
            ("http://localhost:4416/", "http://localhost:4416"),
            ("  https://pot.example.com/ ", "https://pot.example.com"),
        ] {
            assert_eq!(normalize_base_url(entrada).as_deref(), Some(esperado));
        }
    }

    #[test]
    fn recusa_endereco_que_quebraria_o_argumento() {
        // Espaco entra na string de --extractor-args e parte o argumento em
        // dois, que e falha silenciosa e dificil de diagnosticar.
        for ruim in ["", "   ", "http://a b:4416", "localhost 4416"] {
            assert_eq!(normalize_base_url(ruim), None, "aceitou {ruim:?}");
        }
    }

    #[test]
    fn provedor_indisponivel_nao_gera_flag() {
        // Apontar para endereco morto faz o yt-dlp esperar timeout em TODO
        // download — pior que nao ter provedor.
        for h in [
            ProviderHealth::NotConfigured,
            ProviderHealth::Unreachable {
                base_url: DEFAULT_BASE_URL.into(),
            },
            ProviderHealth::Unhealthy {
                detail: "500".into(),
            },
        ] {
            assert!(
                extractor_args(&h, DEFAULT_BASE_URL).is_empty(),
                "gerou flag para {h:?}"
            );
        }
    }

    #[test]
    fn provedor_pronto_gera_a_flag_do_bgutil() {
        let h = ProviderHealth::Ready {
            version: Some("1.3.1".into()),
        };
        let args = extractor_args(&h, "localhost:4416");
        assert_eq!(args[0], "--extractor-args");
        assert_eq!(
            args[1],
            "youtubepot-bgutilhttp:base_url=http://localhost:4416"
        );
    }

    #[test]
    fn provedor_pronto_com_endereco_invalido_ainda_nao_gera_flag() {
        let h = ProviderHealth::Ready { version: None };
        assert!(extractor_args(&h, "  ").is_empty());
    }

    #[test]
    fn reconhece_as_frases_reais_de_token_faltando() {
        let casos = [
            "error: [youtube] abc: a po token is required for this client",
            "warning: [youtube] abc: the content is not available on this app",
            "error: sign in to confirm you're not a bot",
        ];
        for c in casos {
            assert!(stderr_indicates_missing_token(c), "nao reconheceu {c:?}");
        }
    }

    #[test]
    fn nao_confunde_log_de_sucesso_com_falta_de_token() {
        // "po_token" aparece em log normal quando o token FOI fornecido;
        // casar pela substring solta produziria falso positivo em todo
        // download bem sucedido com provedor ligado.
        let casos = [
            "[debug] [youtube] fetched po_token from provider",
            "[youtube] extracting player po_token: ok",
            "error: http error 429: too many requests",
            "",
        ];
        for c in casos {
            assert!(
                !stderr_indicates_missing_token(c),
                "falso positivo em {c:?}"
            );
        }
    }

    #[test]
    fn so_ready_e_utilizavel() {
        assert!(ProviderHealth::Ready { version: None }.is_usable());
        assert!(!ProviderHealth::NotConfigured.is_usable());
        assert!(!ProviderHealth::Unhealthy { detail: "x".into() }.is_usable());
    }
}
