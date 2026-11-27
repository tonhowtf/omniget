use super::*;

pub const INFO_JSON_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(5 * 60);

pub(super) fn info_json_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OMNIGET_INFO_JSON_DIR").map(PathBuf::from) {
        if dir.is_absolute() {
            return Some(dir);
        }
    }
    Some(
        crate::core::paths::app_data_dir()?
            .join("cache")
            .join("info-json"),
    )
}

pub(super) fn info_json_file(dir: &Path, url: &str) -> PathBuf {
    let hash = crate::core::dependencies::integrity::sha256_hex(url.trim().as_bytes());
    dir.join(format!("{}.info.json", &hash[..32]))
}

pub(super) fn info_json_is_loadable(info: &serde_json::Value) -> bool {
    let kind = info
        .get("_type")
        .and_then(|t| t.as_str())
        .unwrap_or("video");
    kind == "video"
        && info
            .get("formats")
            .and_then(|f| f.as_array())
            .is_some_and(|f| !f.is_empty())
}

pub(super) fn info_json_fresh(path: &Path, now: std::time::SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| now.duration_since(m).ok())
        .is_some_and(|age| age <= INFO_JSON_MAX_AGE)
}

pub(super) fn prune_info_json_dir(dir: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.to_string_lossy().ends_with(".info.json") && !info_json_fresh(&path, now) {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn save_info_json(url: &str, raw: &[u8], info: &serde_json::Value) {
    if !info_json_is_loadable(info) {
        return;
    }
    let Some(dir) = info_json_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    prune_info_json_dir(&dir, std::time::SystemTime::now());
    let mut keys = vec![url.to_string()];
    if let Some(page) = info.get("webpage_url").and_then(|u| u.as_str()) {
        if page != url {
            keys.push(page.to_string());
        }
    }
    for key in keys {
        let path = info_json_file(&dir, &key);
        let tmp = path.with_extension("tmp");
        let written = (|| {
            use std::io::Write;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(raw)?;
            std::fs::rename(&tmp, &path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

pub fn fresh_info_json(url: &str) -> Option<PathBuf> {
    let path = info_json_file(&info_json_dir()?, url);
    if info_json_fresh(&path, std::time::SystemTime::now()) {
        return Some(path);
    }
    let _ = std::fs::remove_file(&path);
    None
}

pub fn forget_info_json(url: &str) {
    if let Some(dir) = info_json_dir() {
        let _ = std::fs::remove_file(info_json_file(&dir, url));
    }
}

pub(super) fn cached_video_info(url: &str) -> Option<serde_json::Value> {
    let raw = std::fs::read(fresh_info_json(url)?).ok()?;
    serde_json::from_slice(&raw).ok()
}

pub(super) fn url_or_info_json_args(url: &str, info_json: Option<&Path>) -> Vec<String> {
    match info_json {
        Some(path) => vec![
            "--load-info-json".to_string(),
            path.to_string_lossy().into_owned(),
        ],
        None => vec![url.to_string()],
    }
}

pub async fn get_video_info(
    ytdlp: &Path,
    url: &str,
    extra_flags: &[String],
) -> anyhow::Result<serde_json::Value> {
    let _timer_start = std::time::Instant::now();

    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let is_yt = is_youtube_url(url);
    let clients: &[Option<&str>] = if is_yt && !crate::core::dependencies::worker_mode() {
        &[None, Some("youtube:player_client=default,mweb")]
    } else {
        &[None]
    };

    let mut last_error = String::new();

    for (attempt, client) in clients.iter().enumerate() {
        tracing::info!(
            "[yt-dlp] info fetch attempt {}/{} for URL",
            attempt + 1,
            clients.len()
        );

        let mut args = vec![
            "--dump-single-json".to_string(),
            "--no-warnings".to_string(),
            "--no-playlist".to_string(),
            "--encoding".to_string(),
            "utf-8".to_string(),
            "--socket-timeout".to_string(),
            "15".to_string(),
            "--retries".to_string(),
            "1".to_string(),
            "--extractor-retries".to_string(),
            "2".to_string(),
            "--retry-sleep".to_string(),
            "exp=1:30".to_string(),
            "--skip-download".to_string(),
        ];
        args.extend(default_user_agent_args(ytdlp_can_impersonate(ytdlp)));
        args.extend(js_runtime_args());

        if let Some(extractor_args) = client {
            args.push("--extractor-args".to_string());
            args.push(extractor_args.to_string());
        }

        append_metadata_cookie_args(&mut args, url, extra_flags, "video info");

        let proxy = crate::core::http_client::proxy_url();
        if attempt == 0 {
            if let Some(dl_id) = log_hook::current_download_id() {
                let line = format!(
                    "[network] yt-dlp metadata {}",
                    proxy_log_label(proxy.as_deref())
                );
                log_hook::emit_log(dl_id, &line);
            }
        }
        if let Some(proxy_url) = proxy {
            args.push("--proxy".to_string());
            args.push(proxy_url);
        }
        args.extend(extra_flags.iter().cloned());
        args.push(url.to_string());

        note_operator_command(ytdlp, &args, "metadata");
        let _slot = acquire_ytdlp_slot("video info").await;
        let child = ytdlp_command(ytdlp)
            .args(&args)
            .args(worker_retry_args(crate::core::dependencies::worker_mode()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;
        tracing::debug!(
            "[perf] get_video_info: yt-dlp process spawned at {:?} (attempt {})",
            _timer_start.elapsed(),
            attempt + 1
        );

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(VIDEO_INFO_PROCESS_TIMEOUT_SECS),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| {
            tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
            anyhow!(
                "Timeout fetching video info ({}s)",
                VIDEO_INFO_PROCESS_TIMEOUT_SECS
            )
        })?
        .map_err(|e| {
            tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
            anyhow!("Failed to run yt-dlp: {}", e)
        })?;

        tracing::debug!(
            "[perf] get_video_info: yt-dlp process exited at {:?} (attempt {})",
            _timer_start.elapsed(),
            attempt + 1
        );

        if result.status.success() {
            let json: serde_json::Value = serde_json::from_slice(&result.stdout)
                .map_err(|e| anyhow!("yt-dlp returned invalid JSON: {}", e))?;
            save_info_json(url, &result.stdout, &json);
            tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
            return Ok(json);
        }

        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        tracing::debug!(
            "[yt-dlp info] stderr ({} bytes): {}",
            stderr.len(),
            stderr.trim()
        );
        let stderr_lower = stderr.to_lowercase();
        if stderr_lower.contains("http error 429") {
            rate_limit_429_increment();
            let sanitized_url = sanitize_log_line(url);
            tracing::warn!(
                "[yt-429] rate limit in get_video_info: url={} attempt={}/{}",
                sanitized_url,
                attempt + 1,
                clients.len()
            );
        }

        let is_retryable = is_yt
            && attempt < clients.len() - 1
            && (stderr_lower.contains("requested format")
                || stderr_lower.contains("not available")
                || stderr_lower.contains("http error 403")
                || stderr_lower.contains("nsig")
                || stderr_lower.contains("http error 429"));

        if is_retryable {
            tracing::warn!(
                "[yt-dlp] info fetch attempt {} failed, retrying with fallback player_client: {}",
                attempt + 1,
                stderr.trim().lines().last().unwrap_or("")
            );
            if stderr_lower.contains("http error 429") {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            last_error = stderr;
            continue;
        }

        tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
        return Err(translate_ytdlp_error(&stderr));
    }

    tracing::debug!("[perf] get_video_info took {:?}", _timer_start.elapsed());
    Err(translate_ytdlp_error(&last_error))
}

pub(super) async fn select_available_subtitle_lang(
    ytdlp: &Path,
    url: &str,
    extra_flags: &[String],
    include_auto: bool,
) -> anyhow::Result<Option<String>> {
    let requested = requested_caption_locales();
    let json = match cached_video_info(url) {
        Some(json) => json,
        None => get_video_info(ytdlp, url, extra_flags).await?,
    };
    let (manual, auto) = subtitle_languages_from_json(&json);
    let available: Vec<String> = if include_auto {
        manual.iter().chain(auto.iter()).cloned().collect()
    } else {
        manual.clone()
    };

    let mut matched: Vec<String> = Vec::new();
    for lang in &requested {
        if let Some(m) = matching_subtitle_lang(lang, &available) {
            if !matched.contains(&m) {
                matched.push(m);
            }
        }
    }
    if !matched.is_empty() {
        return Ok(Some(matched.join(",")));
    }

    let requested_label = if requested.is_empty() {
        "(empty)".to_string()
    } else {
        requested.join(", ")
    };
    let manual_label = format_lang_list(&manual);
    let auto_label = format_lang_list(&auto);
    let line = if manual.is_empty() && auto.is_empty() {
        format!(
            "[subtitles] No downloadable subtitles found; continuing without subtitles. Requested: {}",
            requested_label
        )
    } else if manual.is_empty() && !auto.is_empty() && !include_auto {
        format!(
            "[subtitles] No manual subtitles found; auto-generated subtitles are available but disabled. Continuing without subtitles. Requested: {}; auto: {}",
            requested_label, auto_label
        )
    } else {
        format!(
            "[subtitles] None of the requested subtitles are available; continuing without subtitles. Requested: {}; manual: {}; auto: {}",
            requested_label, manual_label, auto_label
        )
    };
    tracing::warn!("{}", line);
    if let Some(dl_id) = log_hook::current_download_id() {
        log_hook::emit_log(dl_id, &line);
    }

    Ok(None)
}

pub(super) fn subtitle_languages_from_json(json: &serde_json::Value) -> (Vec<String>, Vec<String>) {
    fn collect(map: Option<&serde_json::Value>) -> Vec<String> {
        let mut langs = Vec::new();
        if let Some(obj) = map.and_then(|v| v.as_object()) {
            for (lang, formats) in obj {
                if lang == "live_chat" {
                    continue;
                }
                if formats.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
                    langs.push(lang.clone());
                }
            }
        }
        langs.sort_by_key(|s| s.to_ascii_lowercase());
        langs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        langs
    }

    (
        collect(json.get("subtitles")),
        collect(json.get("automatic_captions")),
    )
}

pub(super) fn matching_subtitle_lang(requested: &str, available: &[String]) -> Option<String> {
    available
        .iter()
        .find(|lang| normalize_lang_token(lang) == normalize_lang_token(requested))
        .cloned()
        .or_else(|| {
            available
                .iter()
                .find(|lang| subtitle_lang_matches(requested, lang))
                .cloned()
        })
}

pub(super) fn subtitle_lang_matches(requested: &str, available: &str) -> bool {
    let requested = normalize_lang_token(requested);
    let available = normalize_lang_token(available);
    if requested.is_empty() || available.is_empty() {
        return false;
    }
    if requested == available {
        return true;
    }

    let req_base = requested.split('-').next().unwrap_or("");
    let avail_base = available.split('-').next().unwrap_or("");
    if req_base != avail_base {
        return false;
    }

    if req_base == "zh" {
        return zh_subtitle_group(&requested) == zh_subtitle_group(&available)
            || requested == "zh"
            || available == "zh";
    }

    requested == req_base || available == avail_base
}

pub(super) fn normalize_lang_token(lang: &str) -> String {
    lang.trim().replace('_', "-").to_ascii_lowercase()
}

pub(super) fn zh_subtitle_group(lang: &str) -> &'static str {
    match lang {
        "zh-tw" | "zh-hant" | "zh-hk" | "zh-mo" => "traditional",
        _ => "simplified",
    }
}

pub(super) fn format_lang_list(langs: &[String]) -> String {
    if langs.is_empty() {
        "none".to_string()
    } else {
        langs.join(", ")
    }
}

pub async fn get_playlist_info(
    ytdlp: &Path,
    url: &str,
    extra_flags: &[String],
) -> anyhow::Result<(String, Vec<PlaylistEntry>)> {
    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let mut args = vec![
        "--flat-playlist".to_string(),
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--retries".to_string(),
        "3".to_string(),
        "--extractor-retries".to_string(),
        "3".to_string(),
        "--retry-sleep".to_string(),
        "exp=1:15".to_string(),
        "--user-agent".to_string(),
        CHROME_UA.to_string(),
    ];
    args.extend(js_runtime_args());

    if is_youtube_url(url) {
        args.push("--extractor-args".to_string());
        args.push("youtube:player_client=default".to_string());
    }

    append_metadata_cookie_args(&mut args, url, extra_flags, "playlist info");

    args.extend(proxy_args());
    args.extend(extra_flags.iter().cloned());
    args.push(url.to_string());

    note_operator_command(ytdlp, &args, "playlist");
    let _slot = acquire_ytdlp_slot("playlist info").await;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        ytdlp_command(ytdlp)
            .args(&args)
            .args(worker_retry_args(crate::core::dependencies::worker_mode()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| anyhow!("Timeout fetching playlist (120s)"))?
    .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_lower = stderr.to_lowercase();
        if stderr_lower.contains("http error 429") {
            rate_limit_429_increment();
            let sanitized_url = sanitize_log_line(url);
            let player_client = if is_youtube_url(url) {
                "default"
            } else {
                "n/a"
            };
            tracing::warn!(
                "[yt-429] rate limit in get_playlist_info: url={} player_client={} retries=3",
                sanitized_url,
                player_client
            );
        }
        return Err(anyhow!(
            "yt-dlp playlist failed: {}",
            extract_error_message(&stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_playlist_dump(&stdout, url))
}

pub(super) fn parse_playlist_dump(stdout: &str, source_url: &str) -> (String, Vec<PlaylistEntry>) {
    let mut entries = Vec::new();
    let mut playlist_title = String::new();
    let source_is_youtube = is_youtube_url(source_url);

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if playlist_title.is_empty() {
                playlist_title = json
                    .get("playlist_title")
                    .or_else(|| json.get("playlist"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("playlist")
                    .to_string();
            }

            let id = json
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = json
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let url = json
                .get("url")
                .or_else(|| json.get("webpage_url"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let url = match url {
                Some(u) => u,
                None => {
                    // WHY: only build a youtube.com/watch URL for real YouTube entries (#157).
                    // Only fabricate a youtube.com/watch URL when the entry is
                    // actually a YouTube video; otherwise skip the entry rather
                    // than produce a bogus URL (issue #157).
                    let entry_is_youtube = json
                        .get("ie_key")
                        .or_else(|| json.get("extractor_key"))
                        .and_then(|v| v.as_str())
                        .map(|k| k.eq_ignore_ascii_case("youtube"))
                        .unwrap_or(source_is_youtube);
                    if entry_is_youtube && !id.is_empty() {
                        format!("https://www.youtube.com/watch?v={}", id)
                    } else {
                        tracing::warn!(
                            "[ytdlp] skipping playlist entry without url/webpage_url: id={}",
                            id
                        );
                        continue;
                    }
                }
            };
            let duration = json.get("duration").and_then(|v| v.as_f64());

            if !id.is_empty() {
                entries.push(PlaylistEntry {
                    id,
                    title,
                    url,
                    duration,
                });
            }
        }
    }

    (playlist_title, entries)
}

pub struct PlaylistEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration: Option<f64>,
}

pub fn archive_extractor_prefix(url: &str) -> Option<&'static str> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))?;
    if host.contains("youtube.com") || host.contains("youtu.be") {
        Some("youtube")
    } else {
        None
    }
}

pub async fn get_playlist_info_incremental(
    ytdlp: &Path,
    url: &str,
    seen_ids: &[String],
    extractor_prefix: &str,
    playlist_end: u32,
) -> anyhow::Result<(String, Vec<PlaylistEntry>)> {
    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let archive_path =
        std::env::temp_dir().join(format!("omniget-chan-archive-{}.txt", std::process::id()));
    {
        let mut content = String::with_capacity(seen_ids.len() * 24);
        for id in seen_ids {
            if !id.is_empty() {
                content.push_str(extractor_prefix);
                content.push(' ');
                content.push_str(id);
                content.push('\n');
            }
        }
        std::fs::write(&archive_path, content)?;
    }

    let mut args = vec![
        "--flat-playlist".to_string(),
        "--dump-json".to_string(),
        "--no-warnings".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--retries".to_string(),
        "3".to_string(),
        "--extractor-retries".to_string(),
        "3".to_string(),
        "--user-agent".to_string(),
        CHROME_UA.to_string(),
        "--lazy-playlist".to_string(),
        "--break-on-existing".to_string(),
        "--download-archive".to_string(),
        archive_path.to_string_lossy().to_string(),
        "--playlist-end".to_string(),
        playlist_end.to_string(),
    ];
    args.extend(js_runtime_args());
    if is_youtube_url(url) {
        args.push("--extractor-args".to_string());
        args.push("youtube:player_client=default".to_string());
    }
    args.extend(proxy_args());
    args.push(url.to_string());

    note_operator_command(ytdlp, &args, "playlist");
    let _slot = acquire_ytdlp_slot("playlist info").await;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        ytdlp_command(ytdlp)
            .args(&args)
            .args(worker_retry_args(crate::core::dependencies::worker_mode()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    let _ = std::fs::remove_file(&archive_path);

    let output = output
        .map_err(|_| anyhow!("Timeout fetching playlist (120s)"))?
        .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let (title, entries) = parse_playlist_dump(&stdout, url);

    if !output.status.success() && entries.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_lower = stderr.to_lowercase();
        if stderr_lower.contains("--break-on-existing")
            || stderr_lower.contains("already been recorded")
            || stderr_lower.contains("stopping further")
        {
            return Ok((title, entries));
        }
        if stderr_lower.contains("http error 429") {
            rate_limit_429_increment();
        }
        return Err(anyhow!(
            "yt-dlp playlist failed: {}",
            extract_error_message(&stderr)
        ));
    }

    Ok((title, entries))
}

#[cfg(test)]
mod single_extraction_tests {
    use super::*;

    #[test]
    fn ua_padrao_so_sem_impersonate() {
        assert!(default_user_agent_args(true).is_empty());
        assert_eq!(
            default_user_agent_args(false),
            vec!["--user-agent".to_string(), CHROME_UA.to_string()]
        );
    }

    #[test]
    fn script_de_pacote_nao_conta_como_build_com_curl_cffi() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("yt-dlp");
        std::fs::write(&script, "#!/opt/homebrew/bin/python3\nimport yt_dlp\n").unwrap();
        assert!(!is_standalone_ytdlp_build(&script));
        let macho = tmp.path().join("yt-dlp_macos");
        std::fs::write(&macho, [0xcf, 0xfa, 0xed, 0xfe, 0, 0]).unwrap();
        assert!(is_standalone_ytdlp_build(&macho));
        assert!(!is_standalone_ytdlp_build(&tmp.path().join("nao-existe")));
    }

    #[test]
    fn download_usa_o_json_e_cai_para_o_url() {
        let p = Path::new("/tmp/x.info.json");
        assert_eq!(
            url_or_info_json_args("https://a/b", Some(p)),
            vec![
                "--load-info-json".to_string(),
                "/tmp/x.info.json".to_string()
            ]
        );
        assert_eq!(
            url_or_info_json_args("https://a/b", None),
            vec!["https://a/b".to_string()]
        );
    }

    #[test]
    fn so_video_com_formatos_e_reaproveitavel() {
        let ok = serde_json::json!({"id":"1","formats":[{"format_id":"18"}]});
        assert!(info_json_is_loadable(&ok));
        assert!(!info_json_is_loadable(
            &serde_json::json!({"_type":"playlist","entries":[]})
        ));
        assert!(!info_json_is_loadable(
            &serde_json::json!({"id":"1","formats":[]})
        ));
        assert!(!info_json_is_loadable(
            &serde_json::json!({"_type":"url","url":"https://x"})
        ));
    }

    #[test]
    fn validade_de_cinco_minutos() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.info.json");
        std::fs::write(&f, "{}").unwrap();
        let now = std::time::SystemTime::now();
        assert!(info_json_fresh(&f, now));
        assert!(info_json_fresh(
            &f,
            now + std::time::Duration::from_secs(299)
        ));
        assert!(!info_json_fresh(
            &f,
            now + std::time::Duration::from_secs(301)
        ));
        assert!(!info_json_fresh(&tmp.path().join("sumiu.info.json"), now));
        prune_info_json_dir(tmp.path(), now + std::time::Duration::from_secs(600));
        assert!(!f.exists(), "expirado sai na poda");
    }

    #[test]
    fn grava_pelo_url_e_pelo_webpage_url_e_esquece() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("OMNIGET_INFO_JSON_DIR", tmp.path());
        let info = serde_json::json!({
            "id":"1","webpage_url":"https://site/v/1","formats":[{"format_id":"b"}]
        });
        let raw = serde_json::to_vec(&info).unwrap();
        save_info_json("https://site/v/1?utm=x", &raw, &info);
        let a = fresh_info_json("https://site/v/1?utm=x").expect("pelo URL pedido");
        let b = fresh_info_json("https://site/v/1").expect("pelo webpage_url");
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), raw);
        assert_eq!(
            cached_video_info("https://site/v/1?utm=x"),
            Some(info.clone())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        forget_info_json("https://site/v/1");
        assert!(fresh_info_json("https://site/v/1").is_none());
        assert!(cached_video_info("https://site/v/1").is_none());
        std::env::remove_var("OMNIGET_INFO_JSON_DIR");
    }
}
