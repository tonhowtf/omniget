/// Categorias que não adianta tentar de novo: a fonte está quebrada ou o
/// pós-processamento falhou por falta de dados, não por azar de rede.
pub fn is_terminal_category(category: &str) -> bool {
    matches!(
        category,
        "not_found" | "broken_source" | "postprocess_failed" | "restricted" | "auth_required"
    )
}

pub fn classify_download_error(error: &str) -> (&str, &str) {
    let lower = error.to_lowercase();

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

    if lower.contains("cookie")
        || lower.contains("login")
        || lower.contains("sign in")
        || lower.contains("authentication")
        || lower.contains("403")
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

    if lower.contains("private") || lower.contains("restricted") || lower.contains("age") {
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

    if lower.contains("ffmpeg") || lower.contains("mux") || lower.contains("merge") {
        return (
            "ffmpeg_needed",
            "FFmpeg is required for this download. Install it from Settings.",
        );
    }

    if lower.contains("yt-dlp") || lower.contains("ytdlp") || lower.contains("no downloader") {
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
    fn network_and_rate_limit_still_retry() {
        assert!(!is_terminal_category(
            classify_download_error("HTTP Error 429").0
        ));
        assert!(!is_terminal_category(
            classify_download_error("connection reset").0
        ));
    }
}
