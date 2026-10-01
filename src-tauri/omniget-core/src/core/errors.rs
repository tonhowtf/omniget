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
    // A 403 on the media data, after the page opened and bytes came, is the
    // site refusing that download URL (expired signature, throttling), not
    // missing content: it fell under broken_source ("probably gone") before
    // (bench 27/09, public Blender films on YouTube). A new try usually works.
    if (lower.contains("unable to download video data")
        || lower.contains("refused part of the download"))
        && (lower.contains("403") || lower.contains("forbidden"))
    {
        return (
            "rate_limited",
            "The site refused the media download midway (HTTP 403). It is usually temporary: retry in a moment.",
        );
    }

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
