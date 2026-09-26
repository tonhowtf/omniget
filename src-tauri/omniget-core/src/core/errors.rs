/// Categorias que não adianta tentar de novo: a fonte está quebrada ou o
/// pós-processamento falhou por falta de dados, não por azar de rede.
pub fn is_terminal_category(category: &str) -> bool {
    matches!(
        category,
        "not_found"
            | "broken_source"
            | "postprocess_failed"
            | "restricted"
            | "auth_required"
            | "access_denied"
            | "invalid_output"
    )
}

/// HTTP status stated next to an HTTP marker ("HTTP 503", "HTTP Error 401:",
/// "status code 502"). A bare number is not a status: IDs and URLs carry
/// "404"/"503" by chance.
fn http_status(lower: &str) -> Option<u16> {
    for marker in [
        "http error ",
        "http/1.1 ",
        "http/2 ",
        "http ",
        "status code ",
        "status ",
    ] {
        let mut rest = lower;
        while let Some(at) = rest.find(marker) {
            let after = &rest[at + marker.len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.len() == 3 {
                if let Ok(code) = digits.parse::<u16>() {
                    if (100..600).contains(&code) {
                        return Some(code);
                    }
                }
            }
            rest = &rest[at + marker.len()..];
        }
    }
    None
}

/// Phrases platforms and yt-dlp use for an age gate.
fn is_age_statement(lower: &str) -> bool {
    [
        "age-restricted",
        "age restricted",
        "age-gated",
        "age gate",
        "age verification",
        "verify your age",
        "confirm your age",
        "inappropriate for some users",
        "adult content",
        "nsfw",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

/// The path out failed (proxy, TLS, DNS, connect timeout) before the platform
/// answered. yt-dlp appends "please report this issue" to some of these, so
/// this runs before the extractor rule (benchmark 26/09, Bluesky in the worker).
pub fn is_egress_failure(lower: &str) -> bool {
    [
        "unable to connect to proxy",
        "tunnel connection failed",
        "proxyerror",
        "proxy error",
        "certificate verify failed",
        "certificate_verify_failed",
        "bad certificate",
        "invalid peer certificate",
        "name resolution",
        "failed to lookup address",
        "nodename nor servname",
        "name or service not known",
        "getaddrinfo failed",
        "connect timeout",
        "egress failed",
    ]
    .iter()
    .any(|p| lower.contains(p))
        // reqwest: "client error (Connect): operation timed out" (Bluesky CDN
        // edge that never answers from this network, benchmark 26/09).
        || (lower.contains("(connect)") && lower.contains("timed out"))
}

/// The text with every URL replaced by `<url>`. Ports, IDs and paths carry
/// "403"/"404"/"429" by chance: gate G05's fixture on 127.0.0.1:63403 turned
/// an "HTTP 401 ... downloading http://127.0.0.1:63403/..." into
/// access_denied (1 of 2 runs, the port is random).
fn without_urls(lower: &str) -> String {
    let mut out = String::with_capacity(lower.len());
    let mut rest = lower;
    loop {
        let at = [rest.find("http://"), rest.find("https://")]
            .into_iter()
            .flatten()
            .min();
        let Some(at) = at else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        out.push_str("<url>");
        let tail = &rest[at..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | '('))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
}

pub fn classify_download_error(error: &str) -> (&str, &str) {
    let lower = without_urls(&error.to_lowercase());
    let status = http_status(&lower);

    if is_egress_failure(&lower) {
        return (
            "egress_failed",
            "Network egress failed (proxy, TLS or DNS) before the platform answered. Retry; check the connection if it persists.",
        );
    }

    // A platform statement that it blocked this network (TikTok: "Your IP
    // address is blocked from accessing this post"). Not a rate limit and not
    // a missing post; waiting or another network is the remedy.
    if lower.contains("ip address is blocked")
        || lower.contains("your ip is blocked")
        || lower.contains("blocked from accessing")
        || lower.contains("ip has been blocked")
        || lower.contains("platform blocked access")
    {
        return (
            "blocked_by_platform",
            "The platform blocked access to this content from this network.",
        );
    }

    // 5xx is the server failing, not the content missing: "HTTP 503 Service
    // Unavailable" used to match "unavailable" below and became a terminal
    // not_found. Transient, retried with backoff.
    if status.is_some_and(|s| (500..600).contains(&s))
        || lower.contains("service unavailable")
        || lower.contains("bad gateway")
        || lower.contains("gateway timeout")
        || lower.contains("gateway time-out")
        || lower.contains("internal server error")
    {
        return (
            "server_error",
            "The server had a temporary error (service unavailable). Try again later.",
        );
    }

    // Antes de tudo que casa com "yt-dlp"/"ffmpeg": a mensagem do Reddit era
    // "yt-dlp: Preprocessing: Conversion failed!" e caía em `ytdlp_needed`,
    // que mandava instalar o yt-dlp que já estava instalado.
    if lower.contains("fragment") && (lower.contains("not found") || lower.contains("404"))
        || lower.contains("downloaded file is empty")
        || lower.contains("conflicting range")
        || lower.contains("unable to download video data")
        || lower.contains("no video formats found")
    {
        return (
            "broken_source",
            "The source served incomplete or missing media (fragments not found). The content is probably gone.",
        );
    }

    if lower.contains("conversion failed")
        || lower.contains("postprocessing:") && lower.contains("error")
    {
        return (
            "postprocess_failed",
            "Post-processing failed: the downloaded streams are incomplete or unsupported.",
        );
    }

    if lower.contains("returned html instead of media") {
        return ("invalid_output", "The server returned a page instead of media. Authentication or an expired link is possible, but not confirmed.");
    }
    if (lower.contains("403") || lower.contains("forbidden"))
        && !["login required", "sign in", "authentication required"]
            .iter()
            .any(|s| lower.contains(s))
    {
        return ("access_denied", "Access was denied. A 403 alone does not identify whether authentication, rate limits, an expired URL or another restriction caused it.");
    }
    // 401 is the server asking for credentials, unlike an ambiguous 403.
    if status == Some(401)
        || lower.contains("unauthorized")
        || lower.contains("unauthorised")
        || lower.contains("cookie")
        || lower.contains("login")
        || lower.contains("sign in")
        || lower.contains("authentication")
    {
        return ("auth_required", "This content requires login. Install the browser extension and visit the site while logged in.");
    }

    if lower.contains("captcha")
        || lower.contains("blocking")
        || lower.contains("rate limit")
        || lower.contains("429")
        || lower.contains("too many")
    {
        return (
            "rate_limited",
            "Too many requests. Try again in a few minutes.",
        );
    }

    // Age is a statement, never the bare substring: "age" is inside "page",
    // "webpage", "image", "message", "storage". yt-dlp's "Unable to download
    // webpage: HTTP Error 412" became "Failed to access the page" and then a
    // terminal SOURCE_RESTRICTED (benchmark 26/09, Bilibili av114868162141203).
    if lower.contains("private") || lower.contains("restricted") || is_age_statement(&lower) {
        return ("restricted", "This content is private or age-restricted.");
    }

    if lower.contains("downloaded file") && lower.contains("not found") {
        return (
            "file_missing",
            "Downloaded file could not be located in the output folder.",
        );
    }

    if lower.contains("not found")
        || lower.contains("404")
        || lower.contains("unavailable")
        || lower.contains("deleted")
    {
        return ("not_found", "Content not found or has been deleted.");
    }

    // yt-dlp's own statements, before the broad "ffmpeg"/"yt-dlp" rules below:
    // "ERROR: [soundcloud] 1: Requested format is not available" used to become
    // "install yt-dlp" because the line mentions yt-dlp (benchmark 26/09).
    if lower.contains("requested format is not available") {
        return (
            "format_unavailable",
            "No format of this media fits the requested options.",
        );
    }
    if lower.contains("protected by a password") || lower.contains("--video-password") {
        return ("auth_required", "This content is password-protected.");
    }
    if lower.contains("empty media response") {
        return (
            "auth_required",
            "The platform returned no media without a login.",
        );
    }
    // "extractor is broken" is ytdlp::translate_ytdlp_error's own wording for
    // the lines above; without it a translated extractor failure fell through
    // to "unknown" (ENGINE_FAILED in the worker).
    if lower.contains("unable to extract")
        || lower.contains("failed to parse json")
        || lower.contains("please report this issue")
        || lower.contains("extractor is broken")
    {
        return (
            "extractor_failure",
            "The extractor could not read this page; the site may have changed. Updating yt-dlp may help, but that is not confirmed.",
        );
    }

    if lower.contains("ffmpeg") || lower.contains("mux") || lower.contains("merge") {
        return (
            "ffmpeg_needed",
            "FFmpeg is required for this download. Install it from Settings.",
        );
    }

    // Only a missing/unrunnable binary means "install yt-dlp"; any other line
    // that mentions yt-dlp is an error yt-dlp reported, not its absence.
    let names_ytdlp = lower.contains("yt-dlp") || lower.contains("ytdlp");
    let missing = [
        "not found",
        "no such file",
        "not installed",
        "could not find",
        "failed to spawn",
        "permission denied",
        "cannot execute",
        "is required",
    ]
    .iter()
    .any(|m| lower.contains(m));
    if (names_ytdlp && missing) || lower.contains("no downloader") {
        return (
            "ytdlp_needed",
            "yt-dlp is required. Install it from Settings.",
        );
    }

    if lower.contains("nsig") || lower.contains("signature") || lower.contains("cipher") {
        return (
            "ytdlp_outdated",
            "yt-dlp needs updating. Restart the app to auto-update.",
        );
    }

    ("unknown", error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ambiguous_denials_do_not_assert_login() {
        assert_eq!(
            classify_download_error("HTTP 403 Forbidden").0,
            "access_denied"
        );
        assert_eq!(classify_download_error("Server returned HTML instead of media — the link may have expired or needs a login").0, "invalid_output");
        assert_eq!(
            classify_download_error("HTTP 403: login required").0,
            "auth_required"
        );
    }

    #[test]
    fn ytdlp_reported_errors_are_not_a_missing_ytdlp() {
        // Benchmark 26/09: 8 cases showed "yt-dlp is required" while yt-dlp
        // was installed and had reported a specific error.
        let cases = [
            ("ERROR: [soundcloud] 309699954: Requested format is not available. Use --list-formats", "format_unavailable"),
            ("yt-dlp exited: ERROR: [BiliBili] 1wP4y1P72h: Unable to extract initial state; please report this issue", "extractor_failure"),
            ("ERROR: [vimeo:review] 99: Failed to parse JSON (caused by JSONDecodeError)", "extractor_failure"),
            ("ERROR: [vimeo] 119195465: This album is protected by a password, use the --video-password option", "auth_required"),
            ("ERROR: [Instagram] BkfuX9UB-eK: Instagram sent an empty media response", "auth_required"),
        ];
        for (msg, want) in cases {
            assert_eq!(classify_download_error(msg).0, want, "{msg}");
        }
        assert_eq!(
            classify_download_error("yt-dlp: No such file or directory").0,
            "ytdlp_needed"
        );
        assert_eq!(
            classify_download_error("failed to spawn yt-dlp").0,
            "ytdlp_needed"
        );
        assert_ne!(
            classify_download_error("yt-dlp exited with code 1").0,
            "ytdlp_needed"
        );
    }

    #[test]
    fn dead_source_is_terminal_not_unknown() {
        for msg in [
            "yt-dlp: Preprocessing: Conversion failed!",
            "ERROR: fragment 1 not found, unable to continue",
            "ERROR: The downloaded file is empty",
            "ERROR: Conflicting range for fragment",
            "ERROR: unable to download video data: HTTP Error 404",
        ] {
            let (cat, _) = classify_download_error(msg);
            assert!(is_terminal_category(cat), "{msg} → {cat}");
            assert_ne!(cat, "unknown");
            assert_ne!(cat, "ytdlp_needed");
        }
    }

    #[test]
    fn d05_server_errors_are_transient_not_not_found() {
        for msg in [
            "HTTP 503 Service Unavailable downloading http://127.0.0.1:47841/flaky.mp4",
            "ERROR: unable to download video data: HTTP Error 502: Bad Gateway",
            "HTTP Error 500: Internal Server Error",
            "HTTP 504 Gateway Timeout",
            "Service Unavailable",
        ] {
            let (cat, _) = classify_download_error(msg);
            assert_eq!(cat, "server_error", "{msg}");
            assert!(!is_terminal_category(cat), "{msg}");
        }
        // The hint re-classifies to the same class (queue messages embed it).
        let (_, hint) = classify_download_error("HTTP 503 Service Unavailable");
        assert_eq!(classify_download_error(hint).0, "server_error");
        // Content statements keep their class; a bare number is not a status.
        assert_eq!(classify_download_error("Video unavailable").0, "not_found");
        assert_eq!(
            classify_download_error("HTTP Error 404: Not Found").0,
            "not_found"
        );
        assert_eq!(
            classify_download_error("post 5031234 deleted").0,
            "not_found"
        );
    }

    #[test]
    fn d09_platform_401_requires_auth() {
        for msg in [
            "HTTP 401 Unauthorized downloading http://127.0.0.1:47841/secret401.mp4",
            "ERROR: [generic] Unable to download webpage: HTTP Error 401: Unauthorized",
            "401 Unauthorized",
        ] {
            assert_eq!(classify_download_error(msg).0, "auth_required", "{msg}");
        }
        // 403 alone stays ambiguous (never asserts login).
        assert_eq!(
            classify_download_error("HTTP 403 Forbidden").0,
            "access_denied"
        );
    }

    #[test]
    fn d16_platform_ip_block_is_its_own_class() {
        let msg = "ERROR: [TikTok] 6748451240264420610: Your IP address is blocked from accessing this post";
        let (cat, hint) = classify_download_error(msg);
        assert_eq!(cat, "blocked_by_platform");
        assert!(!is_terminal_category(cat));
        assert_eq!(classify_download_error(hint).0, "blocked_by_platform");
    }

    #[test]
    fn page_is_not_an_age_restriction() {
        // Benchmark 26/09: the translated "Unable to download webpage" matched
        // the substring "age" and became a terminal SOURCE_RESTRICTED.
        for msg in [
            "Failed to access the page. Check the link and your connection.",
            "Failed to access the page (HTTP 412). Check the link and your connection.",
            "Failed to write image to storage",
        ] {
            assert_ne!(classify_download_error(msg).0, "restricted", "{msg}");
        }
        for msg in [
            "ERROR: [youtube] x: Sign in to confirm your age. This video may be inappropriate for some users.",
            "This video is age-restricted",
            "This is a private video",
            "Video restricted in your region.",
        ] {
            assert!(matches!(classify_download_error(msg).0, "restricted" | "auth_required"), "{msg}");
        }
        assert_eq!(
            classify_download_error(
                "The platform blocked access from this network (HTTP 412 risk control)."
            )
            .0,
            "blocked_by_platform"
        );
    }

    #[test]
    fn g05_numbers_inside_urls_are_not_statuses() {
        // Gate G05 failed 1 of 2 runs with "401 diagnosed as UNKNOWN": the
        // fixture listened on 127.0.0.1:63403, the bare "403" in the URL made
        // the 401 an access_denied (whose worker label no rule knows).
        let msg = "HTTP 401 Unauthorized downloading http://127.0.0.1:63403/g05-22/secret401.mp4";
        assert_eq!(classify_download_error(msg).0, "auth_required", "{msg}");
        let msg = "HTTP 401 Unauthorized downloading https://h.example:40429/a/404/b.mp4";
        assert_eq!(classify_download_error(msg).0, "auth_required", "{msg}");
        assert_eq!(
            classify_download_error(
                "error decoding response from https://x.example/v/4031234/403.json"
            )
            .0,
            "unknown"
        );
        // Statements outside the URL still count.
        assert_eq!(
            classify_download_error("HTTP 403 Forbidden downloading http://127.0.0.1:5000/a.mp4").0,
            "access_denied"
        );
        assert_eq!(
            classify_download_error("HTTP 404 Not Found downloading http://127.0.0.1:5000/a.mp4").0,
            "not_found"
        );
    }

    #[test]
    fn translated_broken_extractor_is_extractor_failure() {
        assert_eq!(
            classify_download_error("yt-dlp extractor is broken for this site. Update yt-dlp in Settings → Dependencies, then retry.").0,
            "extractor_failure"
        );
    }

    #[test]
    fn egress_failures_have_their_own_retryable_class() {
        // Bluesky/yt-dlp inside the worker: proxy, TLS and DNS failures of the
        // path out, not a broken extractor (yt-dlp appends "please report this
        // issue" to many of them) and not an unknown.
        for msg in [
            "ERROR: [generic] x: Unable to download webpage: ('Unable to connect to proxy', OSError('Tunnel connection failed: 403')) please report this issue",
            "error sending request for url (https://bsky.social/xrpc/x): client error (Connect): invalid peer certificate: bad certificate format",
            "ERROR: Unable to download webpage: <urlopen error [SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed: unable to get local issuer certificate>",
            "error sending request: dns error: failed to lookup address information: Temporary failure in name resolution",
            "error sending request for url (https://x/): operation timed out (connect timeout)",
            "ENGINE_FAILED: native: error sending request for url (https://public.api.bsky.app/xrpc/x): client error (Connect): operation timed out; yt-dlp fallback failed: Connection timed out.",
        ] {
            let (cat, hint) = classify_download_error(msg);
            assert_eq!(cat, "egress_failed", "{msg}");
            assert!(!is_terminal_category(cat), "{msg}");
            assert_eq!(classify_download_error(hint).0, "egress_failed", "{hint}");
        }
    }

    #[test]
    fn network_and_rate_limit_still_retry() {
        assert!(!is_terminal_category(
            classify_download_error("HTTP Error 429").0
        ));
        assert!(!is_terminal_category(
            classify_download_error("connection reset").0
        ));
    }
}
