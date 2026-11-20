use super::*;

pub(super) fn parse_destination_line(line: &str) -> Option<String> {
    let line = line.trim();

    if let Some(rest) = line.strip_prefix("[download] Destination:") {
        let path = rest.trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    if let Some(rest) = line.strip_prefix("[Merger] Merging formats into \"") {
        let path = rest.trim_end_matches('"');
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    if line.starts_with("[info] Writing video thumbnail") {
        if let Some((_, path)) = line.split_once(" to: ") {
            let path = path.trim();
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }

    None
}

fn thumbnail_output(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    let converted = path.with_extension("jpg");
    if converted.exists() {
        converted
    } else {
        path.to_path_buf()
    }
}

struct Unfinished<'a>(&'a Path);

impl Drop for Unfinished<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

pub(super) fn free_compatible_target(source: &Path, stem: &str) -> PathBuf {
    let wanted = source.with_file_name(format!("{stem}.mp4"));
    if wanted == source {
        return wanted;
    }
    let mut candidate = wanted.clone();
    let mut n = 2;
    while candidate.exists() {
        candidate = crate::core::global_dl::jobs::numbered(&wanted, n);
        n += 1;
    }
    candidate
}

async fn make_output_compatible(path: &Path, mute: bool) -> anyhow::Result<PathBuf> {
    let probe = crate::core::ffmpeg::probe(path).await?;
    let codec_of = |kind: &str| {
        probe
            .streams
            .iter()
            .find(|s| s.codec_type == kind && s.codec_name != "mjpeg" && s.codec_name != "png")
            .map(|s| s.codec_name.clone())
    };
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let Some(video_codec) = codec_of("video") else {
        return Ok(path.to_path_buf());
    };
    let audio_codec = codec_of("audio");
    let plan = compatible_video_plan(
        extension,
        Some(video_codec.as_str()),
        audio_codec.as_deref(),
    );
    if plan == CompatibleVideoPlan::Keep {
        return Ok(path.to_path_buf());
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "video".to_string());
    let temp = path.with_file_name(format!("{stem}.{extension}.compat.mp4"));
    let target = free_compatible_target(path, &stem);
    let (video_args, audio_args): (&[&str], &[&str]) = match plan {
        CompatibleVideoPlan::Keep | CompatibleVideoPlan::Remux => {
            (&["-c:v", "copy"], &["-c:a", "copy"])
        }
        CompatibleVideoPlan::ConvertAudio => (&["-c:v", "copy"], &["-c:a", "aac", "-b:a", "192k"]),
        CompatibleVideoPlan::ConvertAll => (
            &[
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "medium", "-crf", "18",
            ],
            &["-c:a", "aac", "-b:a", "192k"],
        ),
    };
    let _unfinished = Unfinished(&temp);
    let status = crate::core::process::command("ffmpeg")
        .kill_on_drop(true)
        .args(["-y", "-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v:0"])
        .args(if mute {
            &[][..]
        } else {
            &["-map", "0:a:0?"][..]
        })
        .args(video_args)
        .args(if mute { &["-an"][..] } else { audio_args })
        .args(["-movflags", "+faststart"])
        .arg(&temp)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| anyhow!("Failed to run ffmpeg: {}", e))?;
    if !status.success() {
        anyhow::bail!("ffmpeg returned {}", status);
    }
    if path != target {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&temp, &target)?;
    Ok(target)
}

async fn compatible_or_original(file_path: PathBuf, mute: bool) -> PathBuf {
    match make_output_compatible(&file_path, mute).await {
        Ok(converted) => converted,
        Err(e) => {
            let line = format!(
                "[omniget] could not convert to H.264/AAC MP4, keeping the original file: {e}"
            );
            tracing::warn!("{}", line);
            if let Some(id) = log_hook::current_download_id() {
                log_hook::emit_log(id, &line);
            }
            file_path
        }
    }
}

pub async fn make_direct_download_compatible(
    file_path: PathBuf,
    progress: Option<&mpsc::Sender<ProgressUpdate>>,
) -> PathBuf {
    if !compatible_video_enabled() {
        return file_path;
    }
    let ffmpeg_available = crate::core::ffmpeg::is_ffmpeg_available().await;
    if !compatible_pass_applies(true, ffmpeg_available, &file_path) {
        return file_path;
    }
    if let Some(progress) = progress {
        let _ = progress
            .send(ProgressUpdate::phase("postprocessing", 99.0))
            .await;
    }
    compatible_or_original(file_path, false).await
}

pub async fn finish_direct_video(
    file_path: PathBuf,
    downloaded_bytes: u64,
    progress: Option<&mpsc::Sender<ProgressUpdate>>,
) -> (PathBuf, u64) {
    let file_path = make_direct_download_compatible(file_path, progress).await;
    let size = std::fs::metadata(&file_path)
        .map(|m| m.len())
        .unwrap_or(downloaded_bytes);
    (file_path, size)
}

pub(super) fn overall_progress(
    planned_count: usize,
    phase: &mut u32,
    last_raw_percent: &mut Option<f64>,
    max_reported: f64,
    percent: f64,
) -> f64 {
    if planned_count == 1 {
        *last_raw_percent = Some(percent);
        return percent;
    }
    adjusted_multi_stream_progress(phase, last_raw_percent, max_reported, percent)
}

pub(super) fn adjusted_multi_stream_progress(
    phase: &mut u32,
    last_raw_percent: &mut Option<f64>,
    max_reported: f64,
    percent: f64,
) -> f64 {
    if *phase <= 1
        && max_reported >= 49.0
        && last_raw_percent.is_some_and(|last| last >= 95.0 && percent < 95.0)
    {
        *phase = 2;
    }
    *last_raw_percent = Some(percent);

    if *phase <= 1 {
        percent * 0.5
    } else {
        50.0 + percent * 0.5
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn download_video(
    ytdlp: &Path,
    url: &str,
    output_dir: &Path,
    quality_height: Option<u32>,
    progress: mpsc::Sender<ProgressUpdate>,
    download_mode: Option<&str>,
    format_id: Option<&str>,
    filename_template: Option<&str>,
    referer: Option<&str>,
    cancel_token: CancellationToken,
    cookie_file: Option<&Path>,
    concurrent_fragments: u32,
    download_subtitles: bool,
    extra_flags: &[String],
    audio_format: Option<&str>,
) -> anyhow::Result<DownloadResult> {
    let _timer_start = std::time::Instant::now();

    if is_youtube_url(url) {
        yt_rate_limiter().acquire().await;
    }

    let _ = progress.send(ProgressUpdate::percent(-1.0)).await;
    let download_started_at = std::time::SystemTime::now();
    let _active = crate::core::adaptive_concurrency::ActiveDownloadGuard::new();

    let mode = download_mode.unwrap_or("auto");
    let is_audio_only = mode == "audio";
    let tuner_host = tuner_host_key(url);

    if let Some(argv) = log_hook::current_argv_override() {
        return run_override_command(
            ytdlp,
            url,
            output_dir,
            argv,
            progress,
            cancel_token,
            is_audio_only,
            download_started_at,
            download_subtitles,
        )
        .await;
    }

    let (ffmpeg_available, ffmpeg_location, aria2c_path) = tokio::join!(
        crate::core::ffmpeg::is_ffmpeg_available(),
        find_ffmpeg_location_cached(),
        crate::core::dependencies::ensure_aria2c(),
    );

    let full_selector = format_id.is_some_and(is_full_format_selector);
    let thumbnails_only = extra_flags.iter().any(|f| f == "--skip-download");
    let compatible_output = !thumbnails_only
        && mode != "audio"
        && compatible_video_enabled()
        && format_id.is_none_or(|f| matches!(f, "b" | "worst") || prefers_compatible_video(f));
    let format_selector = if compatible_output && format_id.is_none() {
        compatible_format_selector(quality_height, mode, ffmpeg_available)
    } else {
        download_format_selector(format_id, quality_height, mode, ffmpeg_available)
    };

    let max_name = file_name_budget(output_dir, cfg!(target_os = "windows"));
    let template = filename_template
        .map(|t| t.to_string())
        .unwrap_or_else(|| default_output_template(max_name));
    let output_template = output_dir.join(&template).to_string_lossy().to_string();

    std::fs::create_dir_all(output_dir)?;

    let explicit_cookie_header = has_explicit_cookie_header(extra_flags);
    let per_domain_cookies = if explicit_cookie_header || cookie_file.is_some() {
        None
    } else {
        per_domain_cookie_file(url)
    };
    let managed_only = managed_cookies_only();
    let allow_fallback = !managed_only
        && !explicit_cookie_header
        && cookie_file.is_none()
        && per_domain_cookies.is_none();

    let manual_cookie_header = if allow_fallback {
        manual_cookie_header_setting()
    } else {
        None
    };
    let manual_cookie_enabled = manual_cookie_header.is_some();
    let global_cookie_file = if allow_fallback && !manual_cookie_enabled {
        global_cookie_file()
    } else {
        None
    };

    let ext_cookies = if allow_fallback && !manual_cookie_enabled && global_cookie_file.is_none() {
        extension_cookie_file()
    } else {
        None
    };

    let had_global_cookie_file = global_cookie_file.is_some();
    let had_ext_cookies = ext_cookies.is_some();

    let effective_cookie_file = cookie_file
        .map(|p| p.to_path_buf())
        .or_else(|| per_domain_cookies.clone())
        .or_else(|| global_cookie_file.map(std::path::PathBuf::from))
        .or(ext_cookies);

    let cfb_setting = if !allow_fallback
        || manual_cookie_enabled
        || explicit_cookie_header
        || effective_cookie_file.is_some()
    {
        String::new()
    } else {
        cookies_from_browser_setting()
    };

    if let Some(dl_id) = log_hook::current_download_id() {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| url.to_string());
        let slug = log_hook::current_cookie_slug();
        let line = if explicit_cookie_header {
            "[cookies] using explicit Cookie header from extra flags".to_string()
        } else if cookie_file.is_some() {
            "[cookies] using caller-supplied cookie file".to_string()
        } else if let Some(ref p) = per_domain_cookies {
            let slug_label = slug.as_deref().unwrap_or("_default");
            format!(
                "[cookies] using managed cookie for {} (account: {}) → {}",
                host,
                slug_label,
                p.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            )
        } else if manual_cookie_enabled {
            format!(
                "[cookies] no managed cookie for {}; using manual Cookie header from settings",
                host
            )
        } else if had_global_cookie_file {
            format!(
                "[cookies] no managed cookie for {}; using global cookie file from settings",
                host
            )
        } else if had_ext_cookies {
            format!(
                "[cookies] no managed cookie for {}; using browser-extension cookies",
                host
            )
        } else if !cfb_setting.is_empty() {
            format!(
                "[cookies] no managed cookie for {}; using --cookies-from-browser {}",
                host, cfb_setting
            )
        } else {
            format!(
                "[cookies] no cookies available for {} — downloading without auth",
                host
            )
        };
        log_hook::emit_log(dl_id, &line);
    }

    let mut base_args = vec![
        "-f".to_string(),
        format_selector,
        "--ignore-config".to_string(),
        "--no-embed-info-json".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--print".to_string(),
        "after_move:OMNIGET_FILEPATH:%(filepath)s".to_string(),
        "--no-quiet".to_string(),
        "--progress".to_string(),
    ];
    base_args.extend(js_runtime_args());

    if mode == "audio" {
        let target_fmt = audio_format.unwrap_or("m4a");
        if format_id.is_none() && target_fmt == "m4a" {
            base_args.push("-S".to_string());
            base_args.push("+codec:aac:m4a".to_string());
        } else {
            base_args.push("-x".to_string());
            base_args.push("--audio-format".to_string());
            base_args.push(target_fmt.to_string());
            base_args.push("--audio-quality".to_string());
            base_args.push("0".to_string());
        }
    }

    if (format_id.is_none() || full_selector) && mode != "audio" && ffmpeg_available {
        base_args.push("--merge-output-format".to_string());
        base_args.push("mp4".to_string());
    }

    if let Some(ref_url) = referer {
        base_args.push("--referer".to_string());
        base_args.push(ref_url.to_string());
        base_args.push("--add-headers".to_string());
        base_args.push(format!("Referer:{}", ref_url));
    }

    if let Some(ext_headers) = ext_headers_for_url(url) {
        for (name, value) in ext_headers {
            let lower = name.to_lowercase();
            if lower == "referer" || lower == "cookie" || lower == "user-agent" {
                continue;
            }
            base_args.push("--add-headers".to_string());
            base_args.push(format!("{}:{}", name, value));
        }
    }

    if let Some(ref cf) = effective_cookie_file {
        base_args.push("--cookies".to_string());
        base_args.push(cf.to_string_lossy().to_string());
    }
    if let Some(ref cookie_header) = manual_cookie_header {
        append_cookie_header(&mut base_args, cookie_header);
    }

    if let Some(ref loc) = ffmpeg_location {
        base_args.push("--ffmpeg-location".to_string());
        base_args.push(loc.clone());
    }

    let requested_fragments = if concurrent_fragments == 0 {
        concurrent_fragments_value()
    } else {
        concurrent_fragments
    };
    let tuned_fragments = tuned_concurrency(&tuner_host, requested_fragments).await;
    let effective_fragments = if is_youtube_url(url) {
        let rate_limit_count = rate_limit_429_count();
        let max_frags = if rate_limit_count >= 2 {
            2
        } else if rate_limit_count > 0 {
            4
        } else {
            MAX_N_YOUTUBE
        };
        tuned_fragments.min(max_frags)
    } else {
        tuned_fragments
    };
    let effective_fragments = effective_fragments.max(1);
    remember_concurrency(&tuner_host, effective_fragments);
    base_args.push("-N".to_string());
    base_args.push(effective_fragments.to_string());

    let is_youtube = is_youtube_url(url);
    let mut yt_args = YoutubeExtractorArgs::default();
    if is_youtube {
        yt_args.player_client = Some("default".to_string());
        yt_args.formats_dashy = effective_fragments > 1;
        if yt_args.formats_dashy && mode != "audio" {
            base_args.extend(dashy_format_sort_args());
        }
        yt_args.lang = translate_metadata_lang().map(|l| normalize_youtube_lang(&l));

        base_args.push("--throttled-rate".to_string());
        base_args.push("100K".to_string());

        base_args.push("--sleep-subtitles".to_string());
        base_args.push("5".to_string());

        if rate_limit_429_count() > 0 {
            base_args.extend([
                "--sleep-requests".to_string(),
                "1".to_string(),
                "--sleep-interval".to_string(),
                "2".to_string(),
                "--max-sleep-interval".to_string(),
                "5".to_string(),
            ]);
        }
    }

    base_args.extend(["--buffer-size".to_string(), "1M".to_string()]);
    if !is_youtube_url(url) && !crate::core::dependencies::worker_mode() {
        base_args.extend(["--http-chunk-size".to_string(), "10M".to_string()]);
    }

    let mut use_aria2c = aria2c_path.is_some()
        && mode != "audio"
        && effective_cookie_file.is_none()
        && cfb_setting.is_empty()
        && !manual_cookie_enabled
        && !explicit_cookie_header;

    match ext_user_agent_for_url(url).or_else(user_agent_setting) {
        Some(ua) => base_args.extend(["--user-agent".to_string(), ua]),
        None => base_args.extend(default_user_agent_args(ytdlp_can_impersonate(ytdlp))),
    }
    base_args.extend(insecure_tls_args());
    base_args.extend([
        "--no-warnings".to_string(),
        "--no-mtime".to_string(),
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--retries".to_string(),
        "5".to_string(),
        "--fragment-retries".to_string(),
        "5".to_string(),
        "--extractor-retries".to_string(),
        "3".to_string(),
        "--file-access-retries".to_string(),
        "3".to_string(),
        "--retry-sleep".to_string(),
        "exp=1:30".to_string(),
        "--trim-filenames".to_string(),
        max_name.to_string(),
        "--no-playlist".to_string(),
        "--newline".to_string(),
        "--progress-delta".to_string(),
        "0.5".to_string(),
        "--progress-template".to_string(),
        PROGRESS_TEMPLATE.to_string(),
        "-o".to_string(),
        output_template,
        "--skip-unavailable-fragments".to_string(),
    ]);

    base_args.extend(proxy_args());
    base_args.extend(extra_flags.iter().cloned());

    let caller_sponsorblock = extra_flags
        .iter()
        .any(|f| f.starts_with("--sponsorblock") || f == "--no-sponsorblock");
    if sponsorblock_enabled() && is_youtube_url(url) && !caller_sponsorblock {
        let cats = sponsorblock_categories();
        let cat_arg = if cats.is_empty() {
            "default".to_string()
        } else {
            cats.join(",")
        };
        let flag = if sponsorblock_mode() == "mark" {
            "--sponsorblock-mark"
        } else {
            "--sponsorblock-remove"
        };
        base_args.push(flag.to_string());
        base_args.push(cat_arg);
    }

    if split_chapters_enabled() {
        base_args.push("--split-chapters".to_string());
    }

    if embed_metadata_enabled() {
        base_args.push("--embed-metadata".to_string());
    }

    if embed_thumbnail_enabled() {
        base_args.push("--embed-thumbnail".to_string());
        base_args.push("--convert-thumbnails".to_string());
        base_args.push("jpg".to_string());
    }

    if let Some(rate) = speed_limit_value() {
        base_args.push("--limit-rate".to_string());
        base_args.push(rate);
    }

    if thumbnails_only {
        base_args.retain(|a| a != "--embed-thumbnail" && a != "--embed-metadata");
        base_args.push("--ignore-no-formats-error".to_string());
    }

    if live_from_start_enabled() {
        base_args.push("--live-from-start".to_string());
        base_args.push("--no-part".to_string());
    }

    if cfg!(target_os = "windows") {
        base_args.push("--windows-filenames".to_string());
    }

    let include_auto_subs = include_auto_subs_setting();
    let mut selected_subtitle_lang: Option<String> = None;
    let mut should_download_subs =
        download_subtitles && !thumbnails_only && rate_limit_429_count() < 2;
    if should_download_subs {
        match select_available_subtitle_lang(ytdlp, url, extra_flags, include_auto_subs).await {
            Ok(Some(lang)) => {
                if let Some(dl_id) = log_hook::current_download_id() {
                    log_hook::emit_log(
                        dl_id,
                        &format!("[subtitles] using selected subtitle language: {}", lang),
                    );
                }
                selected_subtitle_lang = Some(lang);
            }
            Ok(None) => {
                should_download_subs = false;
            }
            Err(err) => {
                should_download_subs = false;
                let line = format!(
                    "[subtitles] could not check subtitles; continuing without subtitles: {}",
                    err
                );
                tracing::warn!("{}", line);
                if let Some(dl_id) = log_hook::current_download_id() {
                    log_hook::emit_log(dl_id, &line);
                }
            }
        }
    }
    let subtitle_args = if should_download_subs {
        let mut args = vec!["--write-sub".to_string()];
        if include_auto_subs {
            args.push("--write-auto-sub".to_string());
        }
        let caption_locale = selected_subtitle_lang.unwrap_or_else(caption_locale_setting);
        args.extend([
            "--sub-lang".to_string(),
            caption_locale,
            "--sub-format".to_string(),
            "best".to_string(),
        ]);
        if !keep_vtt_setting() {
            args.extend(["--convert-subs".to_string(), "srt".to_string()]);
        }
        args
    } else {
        Vec::new()
    };

    let mut info_json = fresh_info_json(url);
    let max_attempts: usize = if crate::core::dependencies::worker_mode() {
        1
    } else {
        3
    } + usize::from(info_json.is_some());
    let mut extra_args: Vec<String> = Vec::new();
    let mut last_error = String::new();
    let mut use_subtitles = should_download_subs;
    let mut use_cfb = !cfb_setting.is_empty() && !explicit_cookie_header && !manual_cookie_enabled;
    let mut format_already_simplified = false;
    let mut drop_chunk_size = false;
    let mut last_was_429 = false;
    let mut shortened_after_open_failure = false;
    let destinations: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let last_step: Arc<Mutex<Option<&'static str>>> = Arc::new(Mutex::new(None));

    for attempt in 0..max_attempts {
        tracing::info!("[yt-dlp] download attempt {}/{}", attempt + 1, max_attempts);
        if cancel_token.is_cancelled() {
            tracing::debug!("[perf] download_video took {:?}", _timer_start.elapsed());
            anyhow::bail!("Download cancelled");
        }

        if attempt > 0 {
            let wait: u64 = if last_was_429 {
                match attempt {
                    1 => 3,
                    2 => 8,
                    _ => 15,
                }
            } else {
                1
            };
            tracing::info!(
                "[yt-dlp] retry {}/{} after {}s (429={})",
                attempt,
                max_attempts - 1,
                wait,
                last_was_429
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            cleanup_part_files(output_dir, &destinations).await;
        }
        if let Ok(mut step) = last_step.lock() {
            *step = None;
        }

        let mut args = base_args.clone();
        if drop_chunk_size {
            strip_chunk_size(&mut args);
        }
        if is_youtube {
            args.extend(yt_args.to_flags());
        }

        if use_subtitles {
            args.extend(subtitle_args.iter().cloned());
        }

        if use_cfb {
            args.push("--cookies-from-browser".to_string());
            args.push(cfb_setting.clone());
        }

        if use_aria2c && !use_cfb {
            if let Some(ref a2_path) = aria2c_path {
                let conns = if is_youtube_url(url) {
                    effective_fragments.max(1)
                } else {
                    effective_fragments.clamp(8, 16)
                };
                args.extend(aria2c_downloader_args(
                    &a2_path.to_string_lossy(),
                    conns,
                    crate::core::http_client::proxy_url().as_deref(),
                ));
            }
        }

        args.extend(extra_args.iter().cloned());
        let from_info = if attempt == 0 {
            info_json.as_deref()
        } else {
            None
        };
        args.extend(url_or_info_json_args(url, from_info));
        strip_default_user_agent(&mut args);

        let engine = if use_aria2c && !use_cfb && aria2c_path.is_some() {
            "aria2c"
        } else {
            "native"
        };
        record_command(
            ytdlp,
            &args,
            attempt as u32 + 1,
            max_attempts as u32,
            yt_args.player_client.clone().filter(|_| is_youtube),
            effective_fragments,
            engine,
            false,
        );
        let attempt_started = std::time::Instant::now();

        let boot_slot = hold_boot_slot("download").await;
        let mut cmd = ytdlp_command(ytdlp);
        cmd.args(&args)
            .args(worker_retry_args(crate::core::dependencies::worker_mode()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("Failed to start yt-dlp: {}", e))?;
        let registered_download_id = log_hook::current_download_id();
        if let (Some(download_id), Some(pid)) = (registered_download_id, child.id()) {
            register_download_process(download_id, pid);
        }
        tracing::debug!(
            "[perf] download_video: yt-dlp process spawned at {:?} (attempt {})",
            _timer_start.elapsed(),
            attempt + 1
        );

        let _ = progress.send(ProgressUpdate::percent(-2.0)).await;

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
        let stderr_pipe = child.stderr.take().ok_or_else(|| anyhow!("No stderr"))?;

        let lines = BufReader::new(stdout).lines();

        let captured_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let line_reader = spawn_stdout_reader(
            lines,
            progress.clone(),
            captured_path.clone(),
            destinations.clone(),
            last_step.clone(),
            log_hook::current_download_id(),
            is_audio_only,
            _timer_start,
            Some(boot_slot.clone()),
        );

        let stderr_log_id = log_hook::current_download_id();
        let stderr_boot_slot = boot_slot.clone();
        let stderr_reader = tokio::spawn(async move {
            let mut buf = String::new();
            let stderr_buf = BufReader::new(stderr_pipe);
            let mut stderr_lines = stderr_buf.lines();
            let mut first = true;
            while let Ok(Some(line)) = stderr_lines.next_line().await {
                if first {
                    first = false;
                    release_boot_slot(&stderr_boot_slot);
                }
                if let Some(id) = stderr_log_id {
                    log_hook::emit_log(id, &line);
                }
                buf.push_str(&line);
                buf.push('\n');
            }
            buf
        });

        let status = tokio::select! {
            s = child.wait() => s.map_err(|e| anyhow!("yt-dlp process failed: {}", e))?,
            _ = cancel_token.cancelled() => {
                kill_process_group(&child);
                let _ = child.kill().await;
                if let Some(download_id) = registered_download_id {
                    unregister_download_process(download_id);
                }
                let _ = line_reader.await;
                let _ = stderr_reader.await;
                cleanup_part_files(output_dir, &destinations).await;
                tracing::debug!("[perf] download_video took {:?}", _timer_start.elapsed());
                anyhow::bail!("Download cancelled");
            }
        };

        if let Some(download_id) = registered_download_id {
            unregister_download_process(download_id);
        }
        release_boot_slot(&boot_slot);

        let _ = line_reader.await;
        let stderr_content = stderr_reader.await.unwrap_or_default();

        if status.success() {
            let _ = progress.send(ProgressUpdate::percent(100.0)).await;

            let file_path = {
                let guard = captured_path.lock().unwrap();
                guard.clone()
            };
            let file_path = if thumbnails_only {
                file_path.map(|p| thumbnail_output(&p))
            } else {
                file_path
            };

            let file_path = match file_path {
                Some(p) if p.exists() => {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let is_audio_ext = matches!(
                        ext.to_lowercase().as_str(),
                        "m4a" | "mp3" | "ogg" | "opus" | "flac" | "aac" | "wav"
                    );
                    if is_audio_ext && !is_audio_only {
                        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        let mp4_candidate = p.with_file_name(format!("{}.mp4", stem));
                        if mp4_candidate.exists() {
                            mp4_candidate
                        } else {
                            tracing::warn!(
                                "[yt-dlp] captured path {} has an audio extension for a video download and no .mp4 sibling; falling back to output-dir glob scan",
                                p.display()
                            );
                            find_downloaded_file(output_dir, url).await.unwrap_or(p)
                        }
                    } else {
                        p
                    }
                }
                _ => {
                    tracing::warn!(
                        "[yt-dlp] no output path captured from yt-dlp stdout (OMNIGET_FILEPATH / Destination / Merger); falling back to output-dir glob scan"
                    );
                    find_downloaded_file(output_dir, url).await?
                }
            };
            if download_subtitles {
                let moved = ensure_subtitles_next_to_media(
                    output_dir,
                    &file_path,
                    download_started_at,
                    url,
                );
                if moved > 0 {
                    tracing::info!(
                        "[yt-dlp] moved {} subtitle file(s) next to {}",
                        moved,
                        file_path.display()
                    );
                }
            }

            if use_subtitles && keep_vtt_setting() {
                convert_vtt_sidecars_to_srt(&file_path).await;
            }

            let file_path = if compatible_output && ffmpeg_available {
                let _ = progress
                    .send(ProgressUpdate::phase("postprocessing", 99.0))
                    .await;
                compatible_or_original(file_path, mode == "mute").await
            } else {
                file_path
            };

            let meta = std::fs::metadata(&file_path)?;
            record_throughput(
                &tuner_host,
                effective_fragments,
                meta.len(),
                attempt_started.elapsed().as_secs_f64(),
            )
            .await;
            tracing::debug!("[perf] download_video took {:?}", _timer_start.elapsed());
            return Ok(DownloadResult {
                file_path,
                file_size_bytes: meta.len(),
                duration_seconds: 0.0,
                torrent_id: None,
            });
        }

        last_error = stderr_content;
        let stderr_lower = last_error.to_lowercase();

        if attempt == 0 && info_json.take().is_some() {
            forget_info_json(url);
            tracing::warn!("[yt-dlp] download from saved info JSON failed, retrying with the URL");
            continue;
        }

        if is_terminal_ytdlp_error(&stderr_lower) {
            let last_line = last_error.lines().last().unwrap_or("unknown error").trim();
            tracing::warn!(
                "[yt-dlp] source is gone, not retrying: {}",
                sanitize_log_line(last_line)
            );
            if let Some(id) = log_hook::current_download_id() {
                log_hook::emit_log(
                    id,
                    "[omniget] the source is missing or broken; not retrying",
                );
            }
            break;
        }

        if attempt < max_attempts - 1 {
            if use_aria2c
                && (stderr_lower.contains("aria2") || stderr_lower.contains("external downloader"))
            {
                use_aria2c = false;
                tracing::warn!("[yt-dlp] aria2c failed, retrying with native downloader");
            }

            last_was_429 = stderr_lower.contains("http error 429");

            if last_was_429 {
                let is_subtitle_only_429 = last_error.lines().all(|line| {
                    let ll = line.to_lowercase();
                    !ll.contains("429") || ll.contains("subtitle")
                });

                if use_subtitles {
                    use_subtitles = false;
                    tracing::warn!(
                        "[yt-dlp] 429 detected, disabling subtitle download for remaining retries"
                    );
                }

                if is_subtitle_only_429 {
                    tracing::warn!("[yt-dlp] subtitle-only 429, retrying without subtitles (keeping current player_client)");
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                } else {
                    rate_limit_429_increment();
                    record_host_rate_limit(&tuner_host).await;
                    let sanitized_url = sanitize_log_line(url);
                    let player_client = if is_youtube_url(url) {
                        "default"
                    } else {
                        "n/a"
                    };
                    let cookies_enabled = use_cfb || effective_cookie_file.is_some();
                    tracing::warn!(
                        "[yt-429] rate limit in download_video: url={} attempt={}/{} player_client={} cookies={} aria2c={}",
                        sanitized_url,
                        attempt + 1,
                        max_attempts,
                        player_client,
                        cookies_enabled,
                        use_aria2c
                    );
                    let base_secs = 10u64 * 2u64.pow(attempt as u32);
                    let jitter_secs = (attempt as u64 * 7 + url.len() as u64) % 5;
                    let wait_secs = base_secs + jitter_secs;
                    tracing::warn!(
                        "[yt-dlp] rate limited (429), waiting {}s (base={}s + jitter={}s)",
                        wait_secs,
                        base_secs,
                        jitter_secs
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;

                    if is_youtube {
                        let client = match attempt {
                            0 => "mweb",
                            _ => "ios",
                        };
                        yt_args.set_client_from_flag(client);
                        tracing::warn!(
                            "[yt-dlp] 429 detected, rotating player_client to {}",
                            client
                        );
                    }
                }
            }

            let address_refused =
                (last_was_429 && attempt >= 1) || stderr_lower.contains("sign in to confirm you");
            if address_refused && move_to_next_pool_proxy(&mut base_args) {
                tracing::warn!(
                    "[yt-dlp] this address was refused, moving to the next proxy in the pool"
                );
            }

            if stderr_lower.contains("nsig") && is_youtube {
                let client = if attempt == 0 { "ios" } else { "mweb" };
                yt_args.set_client_from_flag(client);
                tracing::warn!("[yt-dlp] nsig error, switching to {}", client);
            }

            if is_youtube_url(url) {
                if let Some(client) = sabr_fallback_client(&stderr_lower, attempt) {
                    yt_args.set_client_from_flag(&client);
                    tracing::warn!(
                        "[yt-dlp] SABR-only formats detected, switching player_client to {}",
                        client
                    );
                }
            }

            if stderr_lower.contains("conflicting range") && !drop_chunk_size {
                drop_chunk_size = true;
                tracing::warn!("[yt-dlp] conflicting range, retrying without --http-chunk-size");
            }

            if (stderr_lower.contains("http error 403") || stderr_lower.contains("forbidden"))
                && !extra_args.contains(&"--force-ipv4".to_string())
            {
                extra_args.push("--force-ipv4".to_string());
                tracing::warn!("[yt-dlp] 403 forbidden, adding --force-ipv4");
            }

            if crate::core::impersonation::stderr_indicates_fingerprint_block(&stderr_lower)
                && !extra_args.iter().any(|a| a == "--impersonate")
            {
                match impersonate_targets().await {
                    Some(targets) => {
                        if let Some(target) = crate::core::impersonation::preferred_target(&targets)
                        {
                            let flag = target.as_flag_value();
                            extra_args.push("--impersonate".to_string());
                            extra_args.push(flag.clone());
                            tracing::warn!(
                                "[yt-dlp] fingerprint block detected, trying --impersonate {}",
                                flag
                            );
                        }
                    }
                    None => {
                        tracing::warn!(
                            "[yt-dlp] fingerprint block, but this build has no impersonation targets"
                        );
                    }
                }
            }

            if crate::core::pot_provider::stderr_indicates_missing_token(&stderr_lower) {
                match pot_provider_base_url() {
                    Some(base) => {
                        let health = probe_pot_provider(&base).await;
                        let args = crate::core::pot_provider::extractor_args(&health, &base);
                        if args.is_empty() {
                            tracing::warn!(
                                "[yt-dlp] PO token provider at {} did not respond; continuing without it",
                                base
                            );
                        } else if !extra_args.iter().any(|a| a.contains("youtubepot")) {
                            tracing::warn!(
                                "[yt-dlp] PO token required, using the provider configured at {}",
                                base
                            );
                            extra_args.extend(args);
                        }
                    }
                    None => {
                        tracing::warn!(
                            "[yt-dlp] PO token required and no provider configured; see OMNIGET_POT_PROVIDER_URL"
                        );
                    }
                }
            }

            if stderr_lower.contains("subtitle") && use_subtitles && !last_was_429 {
                tracing::warn!("[yt-dlp] subtitle error detected, disabling subtitles for retry");
                use_subtitles = false;
            }

            if stderr_lower.contains("timed out") || stderr_lower.contains("timeout") {
                tracing::warn!("[yt-dlp] socket timeout on attempt {}", attempt + 1);
            }

            if stderr_lower.contains("certificate") || stderr_lower.contains("ssl") {
                tracing::warn!("[yt-dlp] SSL/certificate error on attempt {}", attempt + 1);
            }

            if (stderr_lower.contains("invalid argument") || stderr_lower.contains("errno 22"))
                && !extra_args.contains(&"--restrict-filenames".to_string())
            {
                extra_args.push("--restrict-filenames".to_string());
                tracing::warn!("[yt-dlp] Errno 22, adding --restrict-filenames");
            }

            if (stderr_lower.contains("403") || stderr_lower.contains("forbidden"))
                && referer.is_none()
                && !extra_args.contains(&"--referer".to_string())
            {
                let fallback = ext_referer_for_url(url).or_else(|| {
                    url::Url::parse(url).ok().and_then(|parsed| {
                        let host = parsed.host_str()?;
                        Some(format!("{}://{}/", parsed.scheme(), host))
                    })
                });
                if let Some(ref_url) = fallback {
                    tracing::info!("[yt-dlp] 403, adding fallback referer: {}", ref_url);
                    extra_args.push("--referer".to_string());
                    extra_args.push(ref_url.clone());
                    extra_args.push("--add-headers".to_string());
                    extra_args.push(format!("Referer:{}", ref_url));
                }
            }

            if ((stderr_lower.contains("could not") && stderr_lower.contains("cookie"))
                || stderr_lower.contains("cookies-from-browser")
                || stderr_lower.contains("failed to decrypt")
                || stderr_lower.contains("keyring")
                || stderr_lower.contains("permission denied"))
                && use_cfb
            {
                use_cfb = false;
                tracing::warn!("[yt-dlp] cookies-from-browser failed. Use the browser extension or set a cookie file in Settings.");
                COOKIE_ERROR_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
            }

            if (stderr_lower.contains("sign in") || stderr_lower.contains("login required"))
                && !use_cfb
                && effective_cookie_file.is_none()
            {
                tracing::warn!("[yt-dlp] login required. Install the browser extension and visit the site while logged in.");
            }

            let retry_shorter =
                is_ffmpeg_input_open_failure(&stderr_lower) && !shortened_after_open_failure;
            if retry_shorter {
                shortened_after_open_failure = true;
                extra_args.extend(["--trim-filenames".to_string(), "80".to_string()]);
                yt_args.formats_dashy = false;
                tracing::warn!(
                    "[yt-dlp] ffmpeg could not open its input, retrying with a shorter name and without DASH fragments"
                );
                if let Some(id) = log_hook::current_download_id() {
                    log_hook::emit_log(
                        id,
                        "[omniget] ffmpeg could not open the downloaded streams; retrying with a shorter file name",
                    );
                }
            } else if stderr_lower.contains("requested format")
                && stderr_lower.contains("not available")
                || stderr_lower.contains("ffmpeg") && stderr_lower.contains("not found")
                || stderr_lower.contains("postprocessing")
            {
                if format_already_simplified {
                    tracing::warn!(
                        "[yt-dlp] format/postprocessing error after simplification, giving up"
                    );
                    break;
                }

                yt_args.player_client = None;
                base_args.retain(|a| a != "--merge-output-format" && a != "mp4");

                if let Some(pos) = base_args.iter().position(|a| a == "-f") {
                    base_args.remove(pos + 1);
                    base_args.remove(pos);
                }
                tracing::warn!("[yt-dlp] format/postprocessing error on attempt {}, removed -f and player_client to use yt-dlp defaults", attempt + 1);
                format_already_simplified = true;
            }

            let last_line = last_error.lines().last().unwrap_or("unknown error").trim();
            let sanitized = sanitize_log_line(last_line);
            tracing::warn!(
                "[yt-dlp] attempt {}/{} failed: {}",
                attempt + 1,
                max_attempts,
                sanitized
            );
            continue;
        }
    }

    tracing::debug!("[perf] download_video took {:?}", _timer_start.elapsed());
    let failed_step = last_step.lock().ok().and_then(|step| *step);
    if let Some(message) = postprocess_failure_message(&last_error, failed_step) {
        return Err(anyhow!(message));
    }
    Err(translate_ytdlp_error(&last_error))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_stdout_reader(
    mut lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    progress_tx: mpsc::Sender<ProgressUpdate>,
    captured_path_writer: Arc<Mutex<Option<PathBuf>>>,
    destinations: Arc<Mutex<Vec<PathBuf>>>,
    last_step: Arc<Mutex<Option<&'static str>>>,
    log_id: Option<u64>,
    is_audio_only: bool,
    _timer_start: std::time::Instant,
    boot_slot: Option<BootSlot>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut boot_slot = boot_slot;
        let mut phase = 0u32;
        let mut last_raw_percent: Option<f64> = None;
        let mut max_reported = 0.0f64;
        let mut first_line_logged = false;
        let mut first_progress_logged = false;
        let mut authoritative_capture = false;
        let mut last_send = std::time::Instant::now();
        let throttle = std::time::Duration::from_millis(250);
        let mut last_stream_id: Option<String> = None;
        let mut prev_streams_bytes: u64 = 0;
        let mut current_stream_bytes: u64 = 0;
        let mut planned_count: usize = 0;
        let mut download_started = false;
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(slot) = boot_slot.take() {
                release_boot_slot(&slot);
            }
            if let Some(id) = log_id {
                log_hook::emit_log(id, &line);
            }
            if let Some(step) = postprocess_step_name(&line) {
                if let Ok(mut current) = last_step.lock() {
                    *current = Some(step);
                }
            }
            if let Some(plan) = parse_planned_formats(&line) {
                planned_count = plan.len();
                let _ = progress_tx
                    .send(ProgressUpdate {
                        percent: max_reported,
                        planned_formats: Some(plan),
                        ..Default::default()
                    })
                    .await;
                continue;
            }
            if let Some(pp) = postprocess_phase_after_download(&line, download_started) {
                let p = max_reported.clamp(98.0, 99.0);
                let _ = progress_tx.send(ProgressUpdate::phase(pp, p)).await;
                max_reported = max_reported.max(p);
                last_send = std::time::Instant::now();
                continue;
            }
            if !first_line_logged {
                first_line_logged = true;
                tracing::debug!(
                    "[perf] download_video first_byte_time: {:?}",
                    _timer_start.elapsed()
                );
            }
            if let Some(rest) = line.strip_prefix("OMNIGET_FILEPATH:") {
                let final_path = rest.trim();
                if !final_path.is_empty() && final_path != "NA" {
                    authoritative_capture = true;
                    let mut guard = captured_path_writer.lock().unwrap();
                    *guard = Some(PathBuf::from(final_path));
                }
                continue;
            }
            if let Some(dest) = parse_destination_line(&line) {
                let dest_path = PathBuf::from(&dest);
                if let Ok(mut seen) = destinations.lock() {
                    seen.push(dest_path.clone());
                }
                if let Some(id) = log_id {
                    leftovers::note_task_destination(id, &dest_path);
                }
                let ext = dest_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let is_subtitle =
                    matches!(ext.as_str(), "vtt" | "srt" | "ass" | "ssa" | "sub" | "lrc");
                if !is_subtitle && !authoritative_capture {
                    phase += 1;
                    let mut guard = captured_path_writer.lock().unwrap();
                    *guard = Some(dest_path);
                }
            }
            if line.contains("[Merger]")
                || line.contains("[FixupM3u8]")
                || line.contains("[VideoConvertor]")
                || (line.contains("[ffmpeg]") && line.to_lowercase().contains("merg"))
            {
                let merging_progress = max_reported.clamp(95.0, 98.0).max(max_reported);
                let _ = progress_tx
                    .send(ProgressUpdate::phase("merging", merging_progress))
                    .await;
                max_reported = max_reported.max(merging_progress);
                last_send = std::time::Instant::now();
                continue;
            }
            if let Some(pct) = parse_progress_line(&line) {
                download_started = true;
                if !first_progress_logged && pct > 0.0 {
                    first_progress_logged = true;
                    tracing::debug!(
                        "[perf] download_video: first_progress > 0% at {:?}",
                        _timer_start.elapsed()
                    );
                }
                let eta = parse_eta_line(&line);
                let speed = parse_speed_line(&line);
                if let (Some(id), Some(e)) = (log_id, eta) {
                    record_eta(id, e);
                }
                let stream = parse_stream_info(&line);
                let fragment = parse_fragment_progress(&line);
                let stream_id = stream.as_ref().map(|s| s.format_id.clone());
                let stream_changed = stream_id.is_some() && stream_id != last_stream_id;
                if stream_changed {
                    last_stream_id = stream_id;
                    prev_streams_bytes += current_stream_bytes;
                    current_stream_bytes = 0;
                }
                if let Some(dl) = parse_downloaded_bytes_line(&line) {
                    current_stream_bytes = current_stream_bytes.max(dl);
                }
                let overall_bytes = Some(prev_streams_bytes + current_stream_bytes);
                let single_total = if planned_count <= 1 {
                    parse_total_bytes_line(&line)
                } else {
                    None
                };
                let attach = |mut u: ProgressUpdate| {
                    u.stream = stream.clone();
                    u.fragment_index = fragment.map(|f| f.0);
                    u.fragment_count = fragment.map(|f| f.1);
                    u
                };
                if is_audio_only {
                    if stream_changed || pct >= 99.0 || last_send.elapsed() >= throttle {
                        let dl = parse_downloaded_bytes_line(&line);
                        let tot = parse_total_bytes_line(&line);
                        let _ = progress_tx
                            .send(attach(ProgressUpdate::rich(pct, dl, tot, speed, eta)))
                            .await;
                        last_send = std::time::Instant::now();
                    }
                } else {
                    let adjusted = overall_progress(
                        planned_count,
                        &mut phase,
                        &mut last_raw_percent,
                        max_reported,
                        pct,
                    );
                    if stream_changed
                        || (adjusted > max_reported
                            && (adjusted >= 99.0 || last_send.elapsed() >= throttle))
                    {
                        max_reported = max_reported.max(adjusted);
                        let _ = progress_tx
                            .send(attach(ProgressUpdate::rich(
                                max_reported,
                                overall_bytes,
                                single_total,
                                speed,
                                eta,
                            )))
                            .await;
                        last_send = std::time::Instant::now();
                    }
                }
            } else if line.trim_start().starts_with("download:") || line.contains("[download]") {
                let dl = parse_downloaded_bytes_line(&line)
                    .or_else(|| parse_default_download_line(&line).map(|(d, _)| d as u64));
                let speed = parse_speed_line(&line)
                    .or_else(|| parse_default_download_line(&line).map(|(_, s)| s));
                if (dl.is_some() || speed.is_some()) && last_send.elapsed() >= throttle {
                    let _ = progress_tx
                        .send(ProgressUpdate::rich(0.0, dl, None, speed, None))
                        .await;
                    last_send = std::time::Instant::now();
                }
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_override_command(
    ytdlp: &Path,
    url: &str,
    output_dir: &Path,
    argv: Vec<String>,
    progress: mpsc::Sender<ProgressUpdate>,
    cancel_token: CancellationToken,
    is_audio_only: bool,
    download_started_at: std::time::SystemTime,
    download_subtitles: bool,
) -> anyhow::Result<DownloadResult> {
    let timer_start = std::time::Instant::now();
    std::fs::create_dir_all(output_dir)?;
    let args = prepare_override_args(argv, url, output_dir);
    record_command(ytdlp, &args, 1, 1, None, 0, "custom", true);

    let boot_slot = hold_boot_slot("download (custom command)").await;
    let mut cmd = ytdlp_command(ytdlp);
    cmd.args(&args)
        .args(worker_retry_args(crate::core::dependencies::worker_mode()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("Failed to start yt-dlp: {}", e))?;
    let registered_download_id = log_hook::current_download_id();
    if let (Some(download_id), Some(pid)) = (registered_download_id, child.id()) {
        register_download_process(download_id, pid);
    }
    let _ = progress.send(ProgressUpdate::percent(-2.0)).await;

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
    let stderr_pipe = child.stderr.take().ok_or_else(|| anyhow!("No stderr"))?;
    let captured_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let destinations: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let line_reader = spawn_stdout_reader(
        BufReader::new(stdout).lines(),
        progress.clone(),
        captured_path.clone(),
        destinations.clone(),
        Arc::new(Mutex::new(None)),
        registered_download_id,
        is_audio_only,
        timer_start,
        Some(boot_slot.clone()),
    );
    let stderr_boot_slot = boot_slot.clone();
    let stderr_reader = tokio::spawn(async move {
        let mut buf = String::new();
        let mut stderr_lines = BufReader::new(stderr_pipe).lines();
        let mut first = true;
        while let Ok(Some(line)) = stderr_lines.next_line().await {
            if first {
                first = false;
                release_boot_slot(&stderr_boot_slot);
            }
            if let Some(id) = registered_download_id {
                log_hook::emit_log(id, &line);
            }
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let status = tokio::select! {
        s = child.wait() => s.map_err(|e| anyhow!("yt-dlp process failed: {}", e))?,
        _ = cancel_token.cancelled() => {
            kill_process_group(&child);
            let _ = child.kill().await;
            if let Some(download_id) = registered_download_id {
                unregister_download_process(download_id);
            }
            let _ = line_reader.await;
            let _ = stderr_reader.await;
            cleanup_part_files(output_dir, &destinations).await;
            anyhow::bail!("Download cancelled");
        }
    };
    if let Some(download_id) = registered_download_id {
        unregister_download_process(download_id);
    }
    release_boot_slot(&boot_slot);
    let _ = line_reader.await;
    let stderr_content = stderr_reader.await.unwrap_or_default();

    if !status.success() {
        return Err(translate_ytdlp_error(&stderr_content));
    }
    let _ = progress.send(ProgressUpdate::percent(100.0)).await;
    let file_path = {
        let guard = captured_path.lock().unwrap();
        guard.clone()
    };
    let file_path = match file_path {
        Some(p) if p.exists() => p,
        _ => find_downloaded_file(output_dir, url).await?,
    };
    if download_subtitles {
        ensure_subtitles_next_to_media(output_dir, &file_path, download_started_at, url);
    }
    let meta = std::fs::metadata(&file_path)?;
    Ok(DownloadResult {
        file_path,
        file_size_bytes: meta.len(),
        duration_seconds: 0.0,
        torrent_id: None,
    })
}

pub(super) async fn convert_vtt_sidecars_to_srt(video_path: &Path) {
    let dir = match video_path.parent() {
        Some(d) => d.to_path_buf(),
        None => return,
    };
    let stem = match video_path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s.to_string(),
        None => return,
    };
    let ffmpeg = match crate::core::dependencies::find_tool("ffmpeg").await {
        Some(p) => p,
        None => return,
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&stem) || !name.to_lowercase().ends_with(".vtt") {
            continue;
        }
        let vtt_path = entry.path();
        let srt_path = vtt_path.with_extension("srt");
        if srt_path.exists() {
            continue;
        }
        let result = crate::core::process::command(&ffmpeg)
            .arg("-y")
            .arg("-i")
            .arg(&vtt_path)
            .arg(&srt_path)
            .output()
            .await;
        match result {
            Ok(out) if out.status.success() => {
                tracing::info!(
                    "[yt-dlp] converted subtitle sidecar {} to srt (vtt kept)",
                    name
                );
            }
            _ => {
                tracing::warn!(
                    "[yt-dlp] failed to convert subtitle sidecar {} to srt",
                    name
                );
                let _ = std::fs::remove_file(&srt_path);
            }
        }
    }
}

// The yt-dlp process runs in its own group (process_group(0)), so ffmpeg and
// aria2c children die with it instead of being orphaned.
fn kill_process_group(child: &tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) {
        // SAFETY: a group created by process_group(0) at spawn.
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = child;
}

// WHY: only this download's temporaries; the folder is shared with other
// downloads and other programs, so a blanket *.part sweep breaks them.
pub(super) async fn cleanup_part_files(dir: &Path, destinations: &Mutex<Vec<PathBuf>>) {
    let prefixes: Vec<String> = destinations
        .lock()
        .map(|d| {
            d.iter()
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    if prefixes.is_empty() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (name.ends_with(".part") || name.ends_with(".ytdl"))
                && prefixes.iter().any(|p| name.starts_with(p.as_str()))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub(super) fn ensure_subtitles_next_to_media(
    output_dir: &Path,
    media_path: &Path,
    started_at: std::time::SystemTime,
    url: &str,
) -> usize {
    let Some(media_dir) = media_path.parent() else {
        return 0;
    };
    let Some(media_stem) = media_path.file_stem().and_then(|s| s.to_str()) else {
        return 0;
    };
    let media_stem = media_stem.to_string();
    let video_id = extract_id_from_url(url).unwrap_or_default();
    let cutoff = started_at
        .checked_sub(std::time::Duration::from_secs(5))
        .unwrap_or(started_at);
    let mut candidates = Vec::new();
    collect_subtitle_candidates(output_dir, cutoff, &mut candidates, 0);

    let mut moved = 0usize;
    for path in candidates {
        if path.parent() == Some(media_dir) {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let matches_media = stem.starts_with(&media_stem)
            || media_stem.starts_with(stem)
            || (!video_id.is_empty() && name.contains(&video_id));
        if !matches_media {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let dest = unique_sidecar_path(media_dir.join(file_name));
        if move_file_best_effort(&path, &dest) {
            moved += 1;
        }
    }
    moved
}

pub(super) fn collect_subtitle_candidates(
    dir: &Path,
    cutoff: std::time::SystemTime,
    out: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_subtitle_candidates(&path, cutoff, out, depth + 1);
            continue;
        }
        if !path.is_file() || !is_subtitle_path(&path) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() == 0 {
            continue;
        }
        if meta.modified().map(|m| m >= cutoff).unwrap_or(false) {
            out.push(path);
        }
    }
}

pub(super) fn is_subtitle_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "vtt" | "srt" | "ass" | "ssa" | "sub" | "lrc" | "ttml"
    )
}

pub(super) fn unique_sidecar_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("subtitle");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    for idx in 1..1000 {
        let file_name = if ext.is_empty() {
            format!("{} ({})", stem, idx)
        } else {
            format!("{} ({}).{}", stem, idx, ext)
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

pub(super) fn move_file_best_effort(from: &Path, to: &Path) -> bool {
    if std::fs::rename(from, to).is_ok() {
        return true;
    }
    if std::fs::copy(from, to).is_ok() {
        let _ = std::fs::remove_file(from);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore = "needs ffmpeg with libvpx"]
    async fn a_file_in_a_codec_editors_reject_is_converted_to_h264_and_aac() {
        let dir = std::env::temp_dir().join(format!("omniget-compat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("clip.webm");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=d=1:s=160x120",
                "-f",
                "lavfi",
                "-i",
                "sine=d=1",
                "-c:v",
                "libvpx-vp9",
                "-c:a",
                "libopus",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let converted = super::make_output_compatible(&source, false).await.unwrap();
        assert_eq!(converted, dir.join("clip.mp4"));
        assert!(!source.exists());
        let probe = crate::core::ffmpeg::probe(&converted).await.unwrap();
        let codecs: Vec<&str> = probe
            .streams
            .iter()
            .map(|s| s.codec_name.as_str())
            .collect();
        assert!(codecs.contains(&"h264"), "{codecs:?}");
        assert!(codecs.contains(&"aac"), "{codecs:?}");
        let again = super::make_output_compatible(&converted, false)
            .await
            .unwrap();
        assert_eq!(again, converted);
        std::fs::remove_dir_all(dir).unwrap();
    }

    use super::*;

    #[tokio::test]
    async fn cleanup_only_removes_this_downloads_temporaries() {
        let dir = std::env::temp_dir().join(format!("omniget-cleanup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ours = [
            "A [x1].f137.mp4.part",
            "A [x1].f137.mp4.ytdl",
            "A [x1].f140.m4a.part",
        ];
        let theirs = [
            "B [y2].mp4.part",
            "B [y2].mp4.ytdl",
            "firefox.zip.part",
            "A [x1].mp4",
        ];
        for name in ours.iter().chain(theirs.iter()) {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let destinations = Mutex::new(vec![
            dir.join("A [x1].f137.mp4"),
            dir.join("A [x1].f140.m4a"),
        ]);
        cleanup_part_files(&dir, &destinations).await;
        for name in ours {
            assert!(!dir.join(name).exists(), "{name} should be removed");
        }
        for name in theirs {
            assert!(dir.join(name).exists(), "{name} should be kept");
        }
        cleanup_part_files(&dir, &Mutex::new(Vec::new())).await;
        assert!(dir.join("B [y2].mp4.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
