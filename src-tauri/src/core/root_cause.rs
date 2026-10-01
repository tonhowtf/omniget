use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Remedy {
    ImportCookies,
    UseImpersonation,
    SetUpPotProvider,
    SwitchPlayerClient,
    WaitAndRetry,
    FreeDiskSpace,
    UpdateYtdlp,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnosis {
    pub cause_key: &'static str,
    pub remedy: Remedy,
    pub detail: Option<String>,
}

pub fn diagnose(stderr: &str) -> Option<Diagnosis> {
    let s = stderr.to_lowercase();

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
    pub fn is_retryable(&self) -> bool {
        matches!(self.retryable, "bounded" | "after_cooldown")
    }
    pub fn cooldown_seconds(&self) -> u64 {
        match self.code {
            "BLOCKED_BY_PLATFORM" => 900,
            "RATE_LIMITED" => 60,
            _ => 5,
        }
    }
}

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
    (
        "EGRESS_BLOCKED",
        "connection",
        "after_user_action",
        "grant_local_network_access",
    ),
    ("LIMIT_BYTES", "validation", "never", "choose_lower_quality"),
    ("LIMIT_TIME", "validation", "never", "choose_lower_quality"),
    (
        "LIMIT_CONNECTIONS",
        "validation",
        "never",
        "choose_lower_quality",
    ),
    ("SERVER_ERROR", "connection", "bounded", "retry_transient"),
    ("EGRESS_FAILED", "connection", "bounded", "retry_transient"),
    ("LINK_EXPIRED", "validation", "never", "paste_original_link"),
];

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
