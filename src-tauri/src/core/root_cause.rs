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

    if (s.contains("tls") || s.contains("handshake") || s.contains("fingerprint"))
        && crate::core::impersonation::stderr_indicates_fingerprint_block(&s)
    {
        return Some(Diagnosis {
            cause_key: "error.cause.tls_fingerprint",
            remedy: Remedy::UseImpersonation,
            detail: Some("site rejected the connection before any content".into()),
        });
    }

    if s.contains("nsig extraction failed") {
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
        // 403 puro e ambiguo e permanece desconhecido; 403 acompanhado de
        // "members-only" e conteudo pago e vira cookies. Trocar a ordem faria o
        // usuario instalar impersonation para resolver falta de login.
        assert_eq!(causa("ERROR: HTTP Error 403: Forbidden"), None);
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
            "ERROR: unable to extract video data",
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

/// Stable machine diagnosis. This deliberately distinguishes observations
/// (HTTP access denied) from hypotheses (authentication/fingerprint).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineDiagnosis {
    pub code: &'static str,
    pub confidence: &'static str,
    pub phase: &'static str,
    pub retryable: &'static str,
    pub next_action: &'static str,
    pub unknowns: Vec<&'static str>,
}
impl MachineDiagnosis {
    /// The single retry predicate shared by queue status, the durable terminal
    /// receipt, `download_diagnose` and `download_retry` admission.
    pub fn is_retryable(&self) -> bool {
        matches!(self.retryable, "bounded" | "after_cooldown")
    }
    /// Minimum wait before an explicit retry. A class without a server-given
    /// Retry-After still waits: rate limits a minute, platform blocks longer.
    pub fn cooldown_seconds(&self) -> u64 {
        match self.code {
            "BLOCKED_BY_PLATFORM" => 900,
            "RATE_LIMITED" => 60,
            _ => 5,
        }
    }
}

/// Fixed machine codes emitted by the confined worker and the MCP finalizer.
/// They are matched before free-text heuristics: a worker code is evidence
/// already classified at the source, and heuristics on labels (e.g. a bare
/// "not found" or a "429" inside an ID) must not relabel it.
const CODE_RULES: &[(&str, &str, &str, &str)] = &[
    (
        "BLOCKED_BY_PLATFORM",
        "connection",
        "after_cooldown",
        "change_network_or_wait",
    ),
    (
        "BOT_CHALLENGE",
        "authentication",
        "after_user_action",
        "open_local_account_flow",
    ),
    (
        "FORMAT_UNAVAILABLE",
        "format_selection",
        "after_user_action",
        "choose_available_format",
    ),
    ("BROKEN_SOURCE", "download", "bounded", "retry_transient"),
    (
        "OUTPUT_TOO_LARGE",
        "validation",
        "never",
        "choose_lower_quality",
    ),
    (
        "OUTPUT_SNAPSHOT_DENIED",
        "validation",
        "never",
        "inspect_job_before_retry",
    ),
    // OmniGet's egress policy refused the connection (not a remote 403).
    (
        "EGRESS_BLOCKED",
        "connection",
        "after_user_action",
        "grant_local_network_access",
    ),
    // OmniGet's per-download egress budget (egress.rs LimitReason): a policy
    // refusal, never retried as is. Emitted by src/mcp/worker.rs egress_limit.
    ("LIMIT_BYTES", "validation", "never", "choose_lower_quality"),
    ("LIMIT_TIME", "validation", "never", "choose_lower_quality"),
    (
        "LIMIT_CONNECTIONS",
        "validation",
        "never",
        "choose_lower_quality",
    ),
    // HTTP 5xx: the server failed, the content is not known to be gone.
    ("SERVER_ERROR", "connection", "bounded", "retry_transient"),
    // Proxy/TLS/DNS/connect failure of the path out (worker EGRESS_FAILED): transient, retried.
    ("EGRESS_FAILED", "connection", "bounded", "retry_transient"),
    // The stored URL is the redacted display form (history, recovery): the
    // original link must be pasted again; retrying would send `[REDACTED]`.
    ("LINK_EXPIRED", "validation", "never", "paste_original_link"),
];

/// Diagnosis carried by a fixed worker/finalizer code, if the text has one.
pub fn diagnose_code(text: &str) -> Option<MachineDiagnosis> {
    for (code, phase, retryable, action) in CODE_RULES {
        if text.contains(code) {
            return Some(MachineDiagnosis {
                code,
                confidence: "probable",
                phase,
                retryable,
                next_action: action,
                unknowns: if *code == "BLOCKED_BY_PLATFORM" {
                    vec!["The platform did not say how long the block lasts"]
                } else {
                    vec![]
                },
            });
        }
    }
    None
}

pub fn machine_diagnose(text: &str) -> MachineDiagnosis {
    if let Some(d) = diagnose_code(text) {
        return d;
    }
    let s = text.to_ascii_lowercase();
    let rules: &[(&[&str], &str, &str, &str, &str)] = &[
        (
            &["invalid url"],
            "INVALID_URL",
            "validation",
            "never",
            "correct_url",
        ),
        (
            &["unsupported url"],
            "UNSUPPORTED_URL",
            "resolution",
            "never",
            "choose_supported_url",
        ),
        (
            &["drm protected", "drm-protected"],
            "DRM_UNSUPPORTED",
            "resolution",
            "never",
            "use_authorized_unprotected_source",
        ),
        (
            &["not a bot", "captcha", "bot challenge"],
            "BOT_CHALLENGE",
            "authentication",
            "after_user_action",
            "open_local_account_flow",
        ),
        (
            &[
                "session expired",
                "cookie expired",
                "cookies are no longer valid",
            ],
            "AUTH_EXPIRED",
            "authentication",
            "after_user_action",
            "open_local_account_flow",
        ),
        (
            &[
                "login required",
                "sign in",
                "private video",
                "members-only",
                "http 401",
                "http error 401",
                "401 unauthorized",
            ],
            "AUTH_REQUIRED",
            "authentication",
            "after_user_action",
            "open_local_account_flow",
        ),
        (
            &[
                "geo restricted",
                "not available in your country",
                "geo-restricted",
            ],
            "GEO_RESTRICTED",
            "resolution",
            "never",
            "check_availability",
        ),
        (
            &[
                "ip address is blocked",
                "your ip is blocked",
                "blocked from accessing",
                "ip has been blocked",
                "platform blocked access",
            ],
            "BLOCKED_BY_PLATFORM",
            "connection",
            "after_cooldown",
            "change_network_or_wait",
        ),
        (
            // HTTP 5xx: the server failed; the content is not known to be gone.
            &[
                "http error 500",
                "http error 502",
                "http error 503",
                "http error 504",
                "http 500",
                "http 502",
                "http 503",
                "http 504",
                "service unavailable",
                "bad gateway",
                "gateway timeout",
                "internal server error",
            ],
            "SERVER_ERROR",
            "connection",
            "bounded",
            "retry_transient",
        ),
        (
            &["http error 429", "http 429", "too many requests"],
            "RATE_LIMITED",
            "connection",
            "after_cooldown",
            "wait_for_cooldown",
        ),
        (
            &[
                "http error 404",
                "http 404",
                "video unavailable",
                "has been removed",
            ],
            "NOT_FOUND",
            "resolution",
            "never",
            "check_source",
        ),
        (
            &["http error 403", "403 forbidden", "http 403 access denied"],
            "ACCESS_DENIED",
            "connection",
            "unknown",
            "inspect_more_evidence",
        ),
        (
            &["no space left", "disk full", "os error 28"],
            "DISK_FULL",
            "writing",
            "after_user_action",
            "choose_approved_destination",
        ),
        (
            &["permission denied", "read-only file system"],
            "OUTPUT_PERMISSION_DENIED",
            "writing",
            "after_user_action",
            "choose_approved_destination",
        ),
        (
            &["ffmpeg not found", "ffmpeg is not installed"],
            "FFMPEG_MISSING",
            "postprocessing",
            "after_user_action",
            "install_dependency_locally",
        ),
        (
            &["error merging", "conversion failed", "ffmpeg exited"],
            "POSTPROCESS_FAILED",
            "postprocessing",
            "unknown",
            "inspect_more_evidence",
        ),
        (
            &[
                "requested format is not available",
                "no video formats found",
            ],
            "FORMAT_UNAVAILABLE",
            "format_selection",
            "after_user_action",
            "choose_available_format",
        ),
        (
            &["timed out", "timeout"],
            "NETWORK_TIMEOUT",
            "connection",
            "bounded",
            "retry_transient",
        ),
        (
            &[
                "name or service not known",
                "name resolution",
                "nodename nor servname",
            ],
            "DNS_FAILURE",
            "connection",
            "bounded",
            "check_network",
        ),
        (
            &["certificate verify failed", "tls handshake", "ssl error"],
            "TLS_FAILURE",
            "connection",
            "after_user_action",
            "check_network_and_clock",
        ),
        (
            &["unable to extract", "nsig extraction failed"],
            "EXTRACTOR_FAILURE",
            "resolution",
            "unknown",
            "check_engine_version_and_evidence",
        ),
        (
            &["checksum mismatch", "hash mismatch"],
            "INTEGRITY_FAILURE",
            "validation",
            "bounded",
            "retry_transient",
        ),
        (
            &[
                "invalid media",
                "invalid data found when processing",
                "returned html instead of media",
                "integrity_failed",
                "output_invalid",
            ],
            "OUTPUT_INVALID",
            "validation",
            "unknown",
            "inspect_more_evidence",
        ),
        (
            &["cancelled", "canceled"],
            "CANCELLED",
            "finalization",
            "never",
            "none",
        ),
        (
            &["interrupted"],
            "INTERRUPTED",
            "finalization",
            "after_reconciliation",
            "inspect_job_before_retry",
        ),
    ];
    for (needles, code, phase, retryable, action) in rules {
        if needles.iter().any(|n| s.contains(n)) {
            return MachineDiagnosis {
                code,
                confidence: if *code == "ACCESS_DENIED" {
                    "low"
                } else {
                    "probable"
                },
                phase,
                retryable,
                next_action: action,
                unknowns: if *code == "ACCESS_DENIED" {
                    vec!["A 403 alone cannot distinguish authentication, rate limits, geoblocking, expired URLs or a bot challenge"]
                } else if *code == "EXTRACTOR_FAILURE" {
                    vec!["Extraction failure does not prove the installed engine is outdated"]
                } else {
                    vec![]
                },
            };
        }
    }
    MachineDiagnosis {
        code: "UNKNOWN",
        confidence: "unknown",
        phase: "unknown",
        retryable: "unknown",
        next_action: "inspect_more_evidence",
        unknowns: vec!["No specific evidence matched; inspect paginated logs"],
    }
}
#[cfg(test)]
mod machine_tests {
    use super::*;
    #[test]
    fn labeled_diagnostic_corpus() {
        let cases = [
            ("invalid URL", "INVALID_URL"),
            ("Unsupported URL", "UNSUPPORTED_URL"),
            ("DRM protected", "DRM_UNSUPPORTED"),
            ("Sign in to confirm you're not a bot", "BOT_CHALLENGE"),
            ("session expired", "AUTH_EXPIRED"),
            ("Login required", "AUTH_REQUIRED"),
            ("not available in your country", "GEO_RESTRICTED"),
            ("HTTP Error 429", "RATE_LIMITED"),
            ("HTTP Error 404", "NOT_FOUND"),
            ("HTTP Error 403: Forbidden", "ACCESS_DENIED"),
            ("No space left on device", "DISK_FULL"),
            ("Permission denied", "OUTPUT_PERMISSION_DENIED"),
            ("ffmpeg not found", "FFMPEG_MISSING"),
            ("ffmpeg exited with code 1", "POSTPROCESS_FAILED"),
            ("Requested format is not available", "FORMAT_UNAVAILABLE"),
            ("HTTP 401 Unauthorized downloading example", "AUTH_REQUIRED"),
            ("HTTP 404 Not Found downloading example", "NOT_FOUND"),
            ("HTTP 429 Too Many Requests downloading example", "RATE_LIMITED"),
            ("Server returned HTML instead of media — the link may have expired or needs a login", "OUTPUT_INVALID"),
            ("Connection timed out", "NETWORK_TIMEOUT"),
            ("Temporary failure in name resolution", "DNS_FAILURE"),
            ("certificate verify failed", "TLS_FAILURE"),
            (
                "Unable to extract data (latest version)",
                "EXTRACTOR_FAILURE",
            ),
            ("checksum mismatch", "INTEGRITY_FAILURE"),
            ("Invalid media", "OUTPUT_INVALID"),
            ("Cancelled", "CANCELLED"),
            ("Interrupted", "INTERRUPTED"),
            ("progress 98%", "UNKNOWN"),
            ("unexplained failure", "UNKNOWN"),
        ];
        for (input, expected) in cases {
            let d = machine_diagnose(input);
            assert_eq!(d.code, expected, "{input}");
            assert!(!d.next_action.is_empty());
        }
        assert_eq!(machine_diagnose("HTTP Error 403").confidence, "low");
        assert!(!machine_diagnose("Unable to extract").unknowns.is_empty());
    }
    #[test]
    fn worker_codes_win_over_misleading_labels_and_share_one_retry_predicate() {
        // Queue legacy label "Content not found ... (FORMAT_UNAVAILABLE)".
        let d = machine_diagnose(
            "[finalization] error: Content not found or has been deleted. (FORMAT_UNAVAILABLE)",
        );
        assert_eq!(d.code, "FORMAT_UNAVAILABLE");
        assert!(!d.is_retryable());
        let d = machine_diagnose("[omniget] download failed: BROKEN_SOURCE: the source stopped serving part of the media");
        assert_eq!(
            (d.code, d.retryable, d.next_action),
            ("BROKEN_SOURCE", "bounded", "retry_transient")
        );
        assert!(d.is_retryable());
        let d =
            machine_diagnose("BLOCKED_BY_PLATFORM: the platform blocked access from this network");
        assert_eq!(d.code, "BLOCKED_BY_PLATFORM");
        assert_eq!(d.retryable, "after_cooldown");
        assert_eq!(d.cooldown_seconds(), 900);
        let d = machine_diagnose("HTTP 429 rate limited");
        assert_eq!(
            (d.code, d.next_action, d.cooldown_seconds()),
            ("RATE_LIMITED", "wait_for_cooldown", 60)
        );
        for text in [
            "OUTPUT_TOO_LARGE: 81411033 bytes",
            "OUTPUT_SNAPSHOT_DENIED_OR_SIZE_LIMIT",
        ] {
            let d = machine_diagnose(text);
            assert_eq!(d.retryable, "never", "{text}");
            assert!(!d.is_retryable());
        }
        assert_eq!(machine_diagnose("network timeout").cooldown_seconds(), 5);
        assert!(!machine_diagnose("Cancelled").is_retryable());
    }

    #[test]
    fn egress_limit_codes_are_policy_refusals() {
        for code in ["LIMIT_BYTES", "LIMIT_TIME", "LIMIT_CONNECTIONS"] {
            let text = format!("{code}: stopped at OmniGet's per-download budget");
            let d = machine_diagnose(&text);
            assert_eq!((d.code, d.retryable), (code, "never"));
            assert!(!crate::core::queue::is_retryable_error_message(&text));
        }
    }

    #[test]
    fn egress_failed_is_a_transient_connection_code() {
        let d = machine_diagnose("EGRESS_FAILED: network egress failed (proxy, TLS or DNS) before the platform answered; retry later");
        assert_eq!((d.code, d.retryable), ("EGRESS_FAILED", "bounded"));
    }
}
