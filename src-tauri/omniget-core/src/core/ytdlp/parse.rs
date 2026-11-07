use super::*;

pub(super) fn sanitize_log_line(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut remaining = line;
    loop {
        let found = remaining
            .find("https://")
            .or_else(|| remaining.find("http://"));
        match found {
            Some(start) => {
                result.push_str(&remaining[..start]);
                let url_part = &remaining[start..];
                let url_end = url_part
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '>')
                    .unwrap_or(url_part.len());
                let url = &url_part[..url_end];
                if let Some(q) = url.find('?') {
                    result.push_str(&url[..q]);
                } else {
                    result.push_str(url);
                }
                remaining = &url_part[url_end..];
            }
            None => {
                result.push_str(remaining);
                break;
            }
        }
    }
    result
}

pub(crate) fn is_terminal_ytdlp_error(stderr_lower: &str) -> bool {
    if stderr_lower.contains("http error 429") {
        return false;
    }
    const PATTERNS: &[&str] = &[
        "not found, unable to continue",
        "the downloaded file is empty",
        "conflicting range",
        "video unavailable",
        "this video is not available",
        "video has been removed",
        "has been removed by the uploader",
        "this video is private",
        "video is private",
        "no video formats found",
        "unsupported url",
        "is not a valid url",
        "http error 404",
        "http error 410",
    ];
    PATTERNS.iter().any(|p| stderr_lower.contains(p))
}

pub(super) fn stated_http_error(lower: &str) -> Option<u16> {
    let at = lower.find("http error ")? + "http error ".len();
    let digits: String = lower[at..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (digits.len() == 3).then(|| digits.parse().ok()).flatten()
}

pub(super) fn raw_error_line(stderr: &str) -> Option<String> {
    let line = stderr.lines().rev().map(str::trim).find(|l| {
        let t = l.to_lowercase();
        t.starts_with("error:") || t.starts_with("error ")
    })?;
    let line = line
        .strip_prefix("ERROR: ")
        .or_else(|| line.strip_prefix("ERROR:"))
        .unwrap_or(line);
    let bounded: String = line.chars().take(600).collect();
    let red = crate::core::assist::missions::diag::redact(&bounded);
    (!red.trim().is_empty()).then_some(red)
}

pub(super) fn translate_ytdlp_error(stderr: &str) -> anyhow::Error {
    let friendly = friendly_ytdlp_error(stderr).to_string();
    match raw_error_line(stderr) {
        Some(raw) if !friendly.contains(raw.as_str()) => anyhow!(raw).context(friendly),
        _ => anyhow!(friendly),
    }
}

pub(super) fn friendly_ytdlp_error(stderr: &str) -> anyhow::Error {
    let lower = stderr.to_lowercase();

    if lower.contains("errno 22")
        && (lower.contains("textiowrapper")
            || lower.contains("encoding=")
            || lower.contains("exception ignored"))
    {
        return anyhow!(
            "Console encoding error (non-UTF-8 locale). Update yt-dlp in Settings → Dependencies, or run `chcp 65001` in a terminal and reopen the app."
        );
    }
    let error_lines: String = lower
        .lines()
        .filter(|l| l.trim_start().starts_with("error"))
        .collect::<Vec<_>>()
        .join("\n");
    let egress_scope = if error_lines.is_empty() {
        lower.as_str()
    } else {
        error_lines.as_str()
    };
    if crate::core::errors::is_egress_failure(egress_scope) {
        return anyhow!(
            "Network egress failed (proxy, TLS or DNS) before the site answered. Retry; check the connection if it persists."
        );
    }
    if lower.contains("empty media response") {
        return anyhow!(
            "Instagram sent an empty media response: this post requires login. Import cookies for this site in Settings → Cookies, then retry."
        );
    }
    if lower.contains("http error 412") || lower.contains("blocked by server") {
        return anyhow!(
            "The platform blocked access from this network (HTTP 412 Precondition Failed, anti-crawler risk control). Wait or change network, then retry."
        );
    }

    if lower.contains("http error 429") {
        return anyhow!("Server returned error 429 (too many requests). Try again later.");
    }
    if lower.contains("unable to download video data") && lower.contains("http error 403") {
        return anyhow!(
            "The site refused part of the download (HTTP 403 on the media data, not on the page). It is usually temporary: retry; if it keeps happening, update yt-dlp in Settings."
        );
    }
    if lower.contains("http error 403") || lower.contains("forbidden") {
        return anyhow!("Access denied (403). The video may be private or region-restricted.");
    }
    if lower.contains("sign in to confirm")
        || lower.contains("login required")
        || lower.contains("log in to")
        || lower.contains("logged in")
        || lower.contains("registered users")
        || lower.contains("fresh cookies")
        || stderr.contains("请先登录")
        || stderr.contains("需要登录")
        || stderr.contains("登录后可")
        || stderr.contains("仅登录用户")
        || lower.contains("this video is only available") && lower.contains("members")
    {
        return anyhow!(
            "This video requires login. Import cookies for this site in Settings → Cookies, then retry."
        );
    }
    if lower.contains("invalid data found when processing input") {
        return anyhow!(
            "Downloaded streams are DRM-protected and cannot be merged. This content is not supported."
        );
    }
    if lower.contains("nsig extraction failed") || lower.contains("nsig") {
        return anyhow!("Video extraction failed. Update yt-dlp or try again.");
    }
    if lower.contains("[facebook]") && lower.contains("cannot parse data") {
        return anyhow!(
            "Facebook did not show this post without a login. Import cookies for this site in Settings → Cookies, then retry."
        );
    }
    if lower.contains("cannot parse data")
        || lower.contains("please report this issue")
        || (lower.contains("confirm you are on the latest version") && lower.contains("yt-dlp"))
    {
        return anyhow!(
            "yt-dlp extractor is broken for this site. Update yt-dlp in Settings → Dependencies, then retry."
        );
    }
    if lower.contains("requested format") && lower.contains("not available") {
        return anyhow!(
            "Requested format is not available. The download will retry with a compatible format."
        );
    }
    if lower.contains("video unavailable") || lower.contains("not available") {
        return anyhow!("Video unavailable or removed.");
    }
    if lower.contains("private video") {
        return anyhow!("This video is private.");
    }
    if lower.contains("copyright") {
        return anyhow!("Video blocked due to copyright.");
    }
    if lower.contains("geo") && lower.contains("block") {
        return anyhow!("Video restricted in your region.");
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return anyhow!("Connection timed out. Check your internet and try again.");
    }
    if lower.contains("ffmpeg") && (lower.contains("not found") || lower.contains("no such file")) {
        return anyhow!("FFmpeg not found. Install FFmpeg to download this format.");
    }
    if lower.contains("unsupported url") || lower.contains("no suitable infojson") {
        return anyhow!("Unsupported URL. Check that the link is correct.");
    }
    if lower.contains("unable to download") && lower.contains("webpage") {
        return match stated_http_error(&lower) {
            Some(status) => anyhow!(
                "Failed to access the page (HTTP {status}). Check the link and your connection."
            ),
            None => anyhow!("Failed to access the page. Check the link and your connection."),
        };
    }
    if lower.contains("is not a valid url") || lower.contains("no video formats") {
        return anyhow!("No video formats found for this link.");
    }

    let last_error_line = stderr
        .lines()
        .rev()
        .find(|l| {
            let t = l.trim().to_lowercase();
            t.starts_with("error:") || t.starts_with("error ")
        })
        .unwrap_or("")
        .trim();

    let msg = if !last_error_line.is_empty() {
        last_error_line
            .strip_prefix("ERROR: ")
            .or_else(|| last_error_line.strip_prefix("ERROR:"))
            .or_else(|| last_error_line.strip_prefix("error: "))
            .unwrap_or(last_error_line)
    } else {
        let trimmed = stderr.trim();
        if trimmed.len() > 300 {
            &trimmed[..300]
        } else {
            trimmed
        }
    };

    anyhow!("yt-dlp: {}", msg)
}

pub(super) fn template_body(line: &str) -> Option<&str> {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("download:") {
        return Some(rest);
    }
    if t.contains("|eta:") && t.contains("|spd:") {
        return Some(t);
    }
    None
}

pub(super) fn parse_progress_line(line: &str) -> Option<f64> {
    let line = line.trim();

    if let Some(pct) = parse_aria2c_progress(line) {
        return Some(pct);
    }

    let body = if let Some(rest) = template_body(line) {
        rest
    } else if line.ends_with('%') {
        line
    } else {
        return None;
    };

    let pct_part = body.split('|').next()?.trim().trim_end_matches('%');
    let pct_str = pct_part.split_whitespace().last()?;
    pct_str.parse::<f64>().ok()
}

pub(super) fn parse_aria2c_progress(line: &str) -> Option<f64> {
    if !line.starts_with("[#") {
        return None;
    }
    let open = line.find('(')?;
    let after = &line[open + 1..];
    let close = after.find("%)")?;
    after[..close].trim().parse::<f64>().ok()
}

pub(super) fn template_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let body = template_body(line)?;
    for part in body.split('|') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(key) {
            let rest = rest.trim();
            if rest.is_empty() || rest.eq_ignore_ascii_case("na") {
                return None;
            }
            return Some(rest);
        }
    }
    None
}

pub(super) fn parse_eta_line(line: &str) -> Option<u64> {
    let raw = template_field(line, "eta:")?;
    raw.parse::<u64>()
        .ok()
        .or_else(|| raw.parse::<f64>().ok().map(|f| f.max(0.0) as u64))
}

pub(super) fn parse_speed_line(line: &str) -> Option<f64> {
    let raw = template_field(line, "spd:")?;
    raw.parse::<f64>().ok().filter(|s| *s > 0.0)
}

pub(super) fn parse_total_bytes_line(line: &str) -> Option<u64> {
    template_field(line, "tot:")
        .and_then(|v| v.parse::<f64>().ok())
        .or_else(|| template_field(line, "est:").and_then(|v| v.parse::<f64>().ok()))
        .filter(|v| *v > 0.0)
        .map(|v| v as u64)
}

pub(super) fn parse_downloaded_bytes_line(line: &str) -> Option<u64> {
    template_field(line, "dl:")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v >= 0.0)
        .map(|v| v as u64)
}

pub(super) fn parse_stream_info(line: &str) -> Option<StreamInfo> {
    let format_id = template_field(line, "fid:")?.to_string();
    let num = |key: &str| template_field(line, key).and_then(|v| v.parse::<f64>().ok());
    let text = |key: &str| template_field(line, key).map(|v| v.to_string());
    Some(StreamInfo {
        format_id,
        height: num("h:").map(|v| v as u32).filter(|v| *v > 0),
        width: num("w:").map(|v| v as u32).filter(|v| *v > 0),
        fps: num("fps:").filter(|v| *v > 0.0),
        vcodec: text("vc:"),
        acodec: text("ac:"),
        ext: text("ext:"),
        filesize: num("fsz:").filter(|v| *v > 0.0).map(|v| v as u64),
        format_note: text("note:"),
    })
}

pub(super) fn parse_fragment_progress(line: &str) -> Option<(u32, u32)> {
    let idx = template_field(line, "fi:")?.parse::<f64>().ok()? as u32;
    let count = template_field(line, "fc:")?.parse::<f64>().ok()? as u32;
    if count == 0 {
        return None;
    }
    Some((idx, count))
}

pub(super) fn parse_planned_formats(line: &str) -> Option<Vec<String>> {
    if !line.contains("[info]") {
        return None;
    }
    let (_, rest) = line.split_once("format(s): ")?;
    let ids: Vec<String> = rest
        .trim()
        .split('+')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

pub(super) fn postprocess_phase(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    if t.starts_with("[ExtractAudio]") {
        return Some("extracting_audio");
    }
    if t.starts_with("[EmbedSubtitle]") {
        return Some("embedding_subtitles");
    }
    const POST: &[&str] = &[
        "[Metadata]",
        "[EmbedThumbnail]",
        "[SubtitlesConvertor]",
        "[ThumbnailsConvertor]",
        "[SponsorBlock]",
        "[ModifyChapters]",
        "[SplitChapters]",
        "[VideoRemuxer]",
        "[FixupM4a]",
        "[FixupStretched]",
        "[FixupDuplicateMoov]",
        "[FixupDurationMismatch]",
        "[MoveFiles]",
    ];
    if POST.iter().any(|p| t.starts_with(p)) {
        return Some("postprocessing");
    }
    None
}

pub(super) fn postprocess_step_name(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    const STEPS: &[(&str, &str)] = &[
        ("[Merger]", "merge"),
        ("[ExtractAudio]", "extract_audio"),
        ("[VideoConvertor]", "convert"),
        ("[VideoRemuxer]", "remux"),
        ("[EmbedSubtitle]", "embed_subtitles"),
        ("[SubtitlesConvertor]", "convert_subtitles"),
        ("[EmbedThumbnail]", "embed_thumbnail"),
        ("[ThumbnailsConvertor]", "convert_thumbnail"),
        ("[Metadata]", "metadata"),
        ("[SponsorBlock]", "sponsorblock"),
        ("[ModifyChapters]", "sponsorblock"),
        ("[SplitChapters]", "split_chapters"),
        ("[FixupM3u8]", "fixup"),
        ("[FixupM4a]", "fixup"),
        ("[FixupStretched]", "fixup"),
        ("[FixupDuplicateMoov]", "fixup"),
        ("[FixupDurationMismatch]", "fixup"),
        ("[MoveFiles]", "move"),
    ];
    STEPS
        .iter()
        .find(|(prefix, _)| t.starts_with(prefix))
        .map(|(_, step)| *step)
}

pub(super) fn is_ffmpeg_input_open_failure(stderr_lower: &str) -> bool {
    stderr_lower.contains("error opening input files")
        || stderr_lower.contains("error opening input file")
        || (stderr_lower.contains("postprocessing") && stderr_lower.contains("result too large"))
}

pub(super) fn postprocess_failure_message(stderr: &str, last_step: Option<&str>) -> Option<String> {
    let lower = stderr.to_lowercase();
    if lower.contains("invalid data found when processing input") {
        return None;
    }
    let is_postprocess = lower.contains("postprocessing:")
        || lower.contains("conversion failed")
        || is_ffmpeg_input_open_failure(&lower);
    if !is_postprocess {
        return None;
    }
    let raw = raw_error_line(stderr).unwrap_or_default();
    let detail = raw.strip_prefix("Postprocessing: ").unwrap_or(&raw).trim();
    let step = last_step.unwrap_or("postprocess");
    let advice = if is_ffmpeg_input_open_failure(&lower) {
        "open_input"
    } else {
        "generic"
    };
    Some(format!(
        "Post-processing failed at the {step} step ({advice}): {detail}"
    ))
}

pub(super) fn parse_size_token(token: &str) -> Option<f64> {
    let t = token.trim().trim_end_matches("/s").trim();
    let split = t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let value: f64 = num.trim().parse().ok()?;
    let mult = match unit.trim() {
        "" | "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "KB" | "kB" | "K" => 1000.0,
        "MB" | "M" => 1_000_000.0,
        "GB" | "G" => 1_000_000_000.0,
        "TB" | "T" => 1_000_000_000_000.0,
        _ => return None,
    };
    Some(value * mult)
}

pub(super) fn parse_default_download_line(line: &str) -> Option<(f64, f64)> {
    let body = line.split("[download]").nth(1)?.trim();
    if body.contains('%') {
        return None;
    }
    let (size_part, rest) = body.split_once(" at ")?;
    let size = parse_size_token(size_part.trim())?;
    let speed_token = rest.split_whitespace().next()?;
    let speed = parse_size_token(speed_token)?;
    Some((size, speed))
}

pub(super) async fn find_downloaded_file(output_dir: &Path, url: &str) -> anyhow::Result<PathBuf> {
    let video_id = extract_id_from_url(url).unwrap_or_default();
    let media_extensions: &[&str] = &[
        "mp4", "mkv", "webm", "m4a", "mp3", "ogg", "opus", "flac", "avi", "mov", "ts", "m4v",
        "3gp", "aac", "wav",
    ];
    let now = std::time::SystemTime::now();
    let recency_limit = std::time::Duration::from_secs(1800);

    std::fs::create_dir_all(output_dir)?;
    let read_dir = std::fs::read_dir(output_dir)?;
    let mut candidates: Vec<(PathBuf, std::time::SystemTime, bool)> = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".part") || name.ends_with(".ytdl") || name.starts_with('.') {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_media = media_extensions
            .iter()
            .any(|&e| ext.eq_ignore_ascii_case(e));
        if !is_media {
            continue;
        }

        if let Ok(meta) = entry.metadata() {
            if meta.len() == 0 {
                continue;
            }
            if let Ok(modified) = meta.modified() {
                let is_recent = now.duration_since(modified).unwrap_or_default() < recency_limit;
                let matches_id = !video_id.is_empty() && name.contains(&video_id);

                if matches_id || is_recent {
                    candidates.push((path, modified, matches_id));
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));

    if let Some((p, _, _)) = candidates.into_iter().next() {
        return Ok(p);
    }

    let fallback_limit = std::time::Duration::from_secs(120);
    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
    if let Ok(entries) = std::fs::read_dir(output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".part") || name.ends_with(".ytdl") || name.starts_with('.') {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.len() == 0 {
                    continue;
                }
                if let Ok(modified) = meta.modified() {
                    if now.duration_since(modified).unwrap_or_default() < fallback_limit
                        && newest.as_ref().is_none_or(|(_, t)| modified > *t)
                    {
                        newest = Some((path, modified));
                    }
                }
            }
        }
    }

    newest
        .map(|(p, _)| p)
        .ok_or_else(|| anyhow!(
            "Download reported success but no matching file appeared in {:?}. \
             This can happen on Windows with a non-UTF-8 console locale when the title contains non-Latin characters. \
             Try `chcp 65001` in a terminal before launching the app, or update yt-dlp in Settings → Dependencies.",
            output_dir
        ))
}

pub fn parse_formats(json: &serde_json::Value) -> Vec<FormatInfo> {
    let formats = match json.get("formats").and_then(|v| v.as_array()) {
        Some(f) => f,
        None => return Vec::new(),
    };

    let mut result = Vec::new();
    for f in formats {
        let format_id = match f.get("format_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let ext = f
            .get("ext")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let width = f.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
        let height = f.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);
        let fps = f.get("fps").and_then(|v| v.as_f64());
        let vcodec = f
            .get("vcodec")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let acodec = f
            .get("acodec")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let filesize = f
            .get("filesize")
            .or_else(|| f.get("filesize_approx"))
            .and_then(|v| v.as_u64());
        let tbr = f.get("tbr").and_then(|v| v.as_f64());
        let format_note = f
            .get("format_note")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let has_video = vcodec.as_deref().map(|v| v != "none").unwrap_or(false);
        let has_audio = acodec.as_deref().map(|v| v != "none").unwrap_or(false);

        if !has_video && !has_audio {
            continue;
        }

        let resolution = match (width, height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some(format!("{}x{}", w, h)),
            _ => f
                .get("resolution")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        };

        result.push(FormatInfo {
            format_id,
            ext,
            resolution,
            width,
            height,
            fps,
            vcodec,
            acodec,
            filesize,
            tbr,
            has_video,
            has_audio,
            format_note,
        });
    }

    result
}

pub(super) fn extract_id_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_lowercase();

    if host.contains("youtu.be") {
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        return segments.first().map(|s| s.to_string());
    }

    if host.contains("youtube.com") && parsed.path().starts_with("/embed/") {
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        return segments.last().map(|s| s.to_string());
    }

    if host.contains("youtube.com") {
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        if matches!(segments.first(), Some(&"shorts") | Some(&"live")) {
            return segments.get(1).map(|s| s.to_string());
        }
        return parsed
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.to_string());
    }

    if host.contains("bilibili.com") || host == "b23.tv" {
        for seg in parsed.path().split('/').filter(|s| !s.is_empty()) {
            if (seg.starts_with("BV") && seg.len() >= 10)
                || (seg.starts_with("av") && seg.len() > 2)
            {
                return Some(seg.trim_end_matches('/').to_string());
            }
        }
    }

    None
}
