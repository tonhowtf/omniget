#[test]
fn python_version_parsing_and_minimum() {
    assert_eq!(parse_python_version("3.14\n"), Some((3, 14)));
    assert_eq!(parse_python_version("3.9"), Some((3, 9)));
    assert!(parse_python_version("3.9").unwrap() < MIN_PYTHON);
    assert!(parse_python_version("3.10").unwrap() >= MIN_PYTHON);
    assert_eq!(parse_python_version("garbage"), None);
}

#[test]
fn zipapp_is_recognised_by_extension() {
    assert!(is_ytdlp_zipapp(Path::new("/x/bin/yt-dlp.pyz")));
    assert!(!is_ytdlp_zipapp(Path::new("/x/bin/yt-dlp")));
    assert!(!is_ytdlp_zipapp(Path::new("/x/bin/yt-dlp.exe")));
}

#[test]
fn dead_source_is_terminal_but_429_is_not() {
    assert!(is_terminal_ytdlp_error(
        "error: fragment 1 not found, unable to continue"
    ));
    assert!(is_terminal_ytdlp_error(
        "error: the downloaded file is empty"
    ));
    assert!(is_terminal_ytdlp_error(
        "error: [youtube] abc: video unavailable"
    ));
    assert!(is_terminal_ytdlp_error(
        "error: unable to download webpage: http error 404: not found"
    ));
    assert!(!is_terminal_ytdlp_error(
        "error: http error 429: too many requests"
    ));
    assert!(!is_terminal_ytdlp_error(
        "error: requested format is not available"
    ));
    assert!(!is_terminal_ytdlp_error("error: connection reset by peer"));
}

#[tokio::test]
async fn slots_cap_simultaneous_ytdlp_processes() {
    let p1 = acquire_ytdlp_slot("t1").await;
    let p2 = acquire_ytdlp_slot("t2").await;
    let p3 = acquire_ytdlp_slot("t3").await;
    assert_eq!(ytdlp_slots_available(), 0);
    let fourth = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        acquire_ytdlp_slot("t4"),
    )
    .await;
    assert!(fourth.is_err(), "a quarta espera pela vaga");
    drop(p1);
    let boot = hold_boot_slot("download").await;
    assert_eq!(ytdlp_slots_available(), 0);
    release_boot_slot(&boot);
    assert_eq!(ytdlp_slots_available(), 1);
    drop(p2);
    drop(p3);
    assert_eq!(ytdlp_slots_available(), YTDLP_SLOTS);
}

use super::{
    parse_fragment_progress, parse_planned_formats, parse_stream_info, postprocess_phase,
    prepare_override_args, tuner_host_key, YoutubeExtractorArgs, PROGRESS_TEMPLATE,
};

#[test]
fn youtube_args_saem_numa_flag_so() {
    let mut a = YoutubeExtractorArgs {
        player_client: Some("default".into()),
        formats_dashy: true,
        lang: Some("pt".into()),
    };
    assert_eq!(
        a.to_flags(),
        vec![
            "--extractor-args".to_string(),
            "youtube:player_client=default;formats=dashy;lang=pt".to_string()
        ]
    );
    a.set_client_from_flag("youtube:player_client=ios");
    assert_eq!(a.player_client.as_deref(), Some("ios"));
    a.set_client_from_flag("mweb");
    assert_eq!(a.player_client.as_deref(), Some("mweb"));
    a.player_client = None;
    a.formats_dashy = false;
    a.lang = None;
    assert!(a.to_flags().is_empty());
}

#[test]
fn template_de_progresso_traz_o_formato_e_o_fragmento() {
    let line = "download:  42.0%|eta:12|spd:4500000.0|dl:1000|tot:2000|est:NA|fi:12|fc:25|el:3.5|fid:299|h:1080|w:1920|fps:60.0|vc:avc1.64002a|ac:none|ext:mp4|fsz:257619653|note:1080p60";
    let s = parse_stream_info(line).expect("stream");
    assert_eq!(s.format_id, "299");
    assert_eq!(s.height, Some(1080));
    assert_eq!(s.fps, Some(60.0));
    assert_eq!(s.short_vcodec().as_deref(), Some("avc1"));
    assert!(s.has_video() && !s.has_audio());
    assert_eq!(s.filesize, Some(257_619_653));
    assert_eq!(parse_fragment_progress(line), Some((12, 25)));
    assert_eq!(super::parse_progress_line(line), Some(42.0));
    assert_eq!(super::parse_speed_line(line), Some(4_500_000.0));
    for key in [
        "fi:", "fc:", "fid:", "h:", "vc:", "ac:", "ext:", "fsz:", "note:",
    ] {
        assert!(PROGRESS_TEMPLATE.contains(&format!("|{key}")), "{key}");
    }
}

#[test]
fn linha_real_do_yt_dlp_vem_sem_o_prefixo_download() {
    let line = "  0.5%|eta:NA|spd:5144.701683851067|dl:1024|tot:NA|est:223779.0|fi:0|fc:1|el:0|fid:395|h:240|w:320|fps:15|vc:av01.0.00M.08.0.110.05.01.06.0|ac:none|ext:mp4|fsz:223779|note:240p";
    assert_eq!(super::parse_progress_line(line), Some(0.5));
    assert_eq!(super::parse_speed_line(line), Some(5144.701683851067));
    assert_eq!(super::parse_downloaded_bytes_line(line), Some(1024));
    assert_eq!(super::parse_total_bytes_line(line), Some(223779));
    assert_eq!(parse_fragment_progress(line), Some((0, 1)));
    let s = parse_stream_info(line).unwrap();
    assert_eq!(s.format_id, "395");
    assert_eq!(s.short_vcodec().as_deref(), Some("av01"));
    assert_eq!(s.format_note.as_deref(), Some("240p"));
    let audio = "100.0%|eta:NA|spd:2748686.0|dl:309288|tot:309288|est:NA|fi:NA|fc:NA|el:0.11|fid:140|h:NA|w:NA|fps:NA|vc:none|ac:mp4a.40.2|ext:m4a|fsz:309288|note:medium";
    let a = parse_stream_info(audio).unwrap();
    assert!(!a.has_video() && a.has_audio());
    assert_eq!(a.height, None);
    assert_eq!(parse_fragment_progress(audio), None);
}

#[test]
fn linha_sem_formato_nao_vira_stream() {
    assert!(
        parse_stream_info("download:  1.0%|eta:NA|spd:NA|dl:0|tot:NA|est:NA|fi:NA|fc:NA").is_none()
    );
    assert_eq!(parse_fragment_progress("download: 1%|fi:NA|fc:NA"), None);
    assert_eq!(parse_fragment_progress("download: 1%|fi:0|fc:0"), None);
}

#[test]
fn plano_de_formatos_e_fases_de_pos() {
    assert_eq!(
        parse_planned_formats("[info] abc: Downloading 2 format(s): 299+140"),
        Some(vec!["299".to_string(), "140".to_string()])
    );
    assert_eq!(
        parse_planned_formats("[info] abc: Downloading 1 format(s): 18"),
        Some(vec!["18".to_string()])
    );
    assert_eq!(parse_planned_formats("[download] Destination: x.mp4"), None);
    assert_eq!(
        postprocess_phase("[Metadata] Adding metadata to \"x.mp4\""),
        Some("postprocessing")
    );
    assert_eq!(
        postprocess_phase("[ExtractAudio] Destination: x.m4a"),
        Some("extracting_audio")
    );
    assert_eq!(
        postprocess_phase("[EmbedSubtitle] Embedding subtitles"),
        Some("embedding_subtitles")
    );
    assert_eq!(
        postprocess_phase("[Merger] Merging formats into \"x.mp4\""),
        None
    );
    assert_eq!(postprocess_phase("[download]  10%"), None);
}

#[test]
fn comando_editado_recebe_so_a_instrumentacao_que_falta() {
    let out = std::path::Path::new("/tmp/out");
    let url = "https://www.youtube.com/watch?v=abc";
    let argv: Vec<String> = ["yt-dlp", "-f", "18", "--newline", url]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let args = prepare_override_args(argv, url, out);
    assert_eq!(args[0], "-f", "o nome do programa sai");
    assert_eq!(args.iter().filter(|a| *a == "--newline").count(), 1);
    assert!(args.contains(&"--progress".to_string()));
    assert!(args.contains(&"--no-quiet".to_string()));
    assert!(args.contains(&PROGRESS_TEMPLATE.to_string()));
    assert!(args
        .iter()
        .any(|a| a.starts_with("after_move:OMNIGET_FILEPATH")));
    assert!(args.contains(&"-o".to_string()), "sem -o, recebe o padrão");
    assert_eq!(
        args.iter().filter(|a| *a == url).count(),
        1,
        "URL fica uma vez, no lugar do usuário"
    );
    let argv2: Vec<String> = ["-P", "/x", "--output=%(id)s.%(ext)s"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let args2 = prepare_override_args(argv2, url, out);
    assert!(!args2.contains(&"-o".to_string()));
    assert_eq!(
        args2.last().map(String::as_str),
        Some(url),
        "URL entra no fim quando falta"
    );
}

#[test]
fn so_baixa_yt_dlp_quando_a_release_e_mais_nova() {
    use super::newer_release_available as newer;
    assert!(!newer(Some((2026, 8, 30)), Some((2026, 8, 19))));
    assert!(
        !newer(Some((2026, 8, 19)), Some((2026, 8, 19))),
        "mesma versão não re-baixa"
    );
    assert!(newer(Some((2026, 8, 19)), Some((2026, 9, 2))));
    assert!(!newer(None, Some((2026, 9, 2))));
    assert!(!newer(Some((2026, 8, 19)), None));
    assert_eq!(
        super::parse_ytdlp_version("2026.09.04.232832"),
        Some((2026, 9, 4))
    );
}

#[test]
fn host_do_tuner_junta_as_variantes_do_youtube() {
    assert_eq!(
        tuner_host_key("https://www.youtube.com/watch?v=x"),
        "youtube"
    );
    assert_eq!(tuner_host_key("https://youtu.be/x"), "youtube");
    assert_eq!(
        tuner_host_key("https://rr1---sn-x.googlevideo.com/videoplayback"),
        "youtube"
    );
    assert_eq!(tuner_host_key("https://www.vimeo.com/1"), "vimeo.com");
    assert_eq!(tuner_host_key("nope"), "unknown");
}

use super::*;

#[test]
fn operator_command_keeps_args_after_the_proxy() {
    let argv: Vec<String> = [
        "-f",
        "b",
        "--proxy",
        "http://127.0.0.1:9",
        "--extractor-args",
        "youtube:formats=dashy",
        "https://www.youtube.com/watch?v=abc",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let ordered = operator_order(&argv);
    assert_eq!(
        &ordered[ordered.len() - 2..],
        ["--proxy", "http://127.0.0.1:9"]
    );
    assert!(
        ordered
            .iter()
            .position(|a| a == "--extractor-args")
            .unwrap()
            < ordered.len() - 2
    );
    assert_eq!(ordered.len(), argv.len());
}

#[test]
fn full_selector_passes_intact_and_dashy_prefers_dash() {
    let worker = "bv*[height<=720]+ba/b[height<=720]/bv*[height<=?720]+ba/b[height<=?720]/ba";
    assert!(is_full_format_selector(worker));
    assert!(!is_full_format_selector("137"));
    let s = download_format_selector(Some(worker), Some(720), "auto", true);
    assert_eq!(s, worker);
    assert!(!s.split('/').any(|alt| alt == "b" || alt == "bv*"));
    assert_eq!(
        download_format_selector(Some(worker), Some(720), "auto", false),
        worker
    );
    assert_eq!(
        download_format_selector(Some("137"), Some(720), "auto", true),
        "137/bv*[height<=720]+ba[ext=m4a]/bv*[height<=720]+ba/b[height<=720]/b"
    );
    assert_eq!(
        download_format_selector(None, Some(720), "auto", true),
        "bv*[height<=720]+ba[ext=m4a]/bv*[height<=720]+ba/b[height<=720]/b"
    );
    assert_eq!(download_format_selector(None, None, "audio", true), "ba/b");
    let sort = dashy_format_sort_args();
    assert_eq!(sort[0], "-S");
    let fields: Vec<&str> = sort[1].split(',').collect();
    assert_eq!(fields[0], "res");
    assert!(fields.contains(&"proto:http_dash_segments"));
    let res = fields.iter().position(|f| *f == "res").unwrap();
    let proto = fields.iter().position(|f| f.starts_with("proto")).unwrap();
    assert!(res < proto);
}

#[test]
fn playlist_dump_uses_top_level_url() {
    let dump = r#"{"id":"abc","title":"Video A","url":"https://example.com/video/abc","playlist_title":"My List"}"#;
    let (title, entries) = parse_playlist_dump(dump, "https://example.com/playlist/1");
    assert_eq!(title, "My List");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].url, "https://example.com/video/abc");
}

#[test]
fn playlist_dump_falls_back_to_webpage_url() {
    let dump = r#"{"id":"12345","title":"Lesson 1","webpage_url":"https://www.bilibili.com/cheese/play/ep12345","extractor_key":"BiliBiliCheese"}"#;
    let (_, entries) = parse_playlist_dump(dump, "https://www.bilibili.com/cheese/play/ss999");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].url,
        "https://www.bilibili.com/cheese/play/ep12345"
    );
}

#[test]
fn playlist_dump_fabricates_youtube_url_only_for_youtube_entries() {
    let dump = r#"{"id":"dQw4w9WgXcQ","title":"YT Video","ie_key":"Youtube"}"#;
    let (_, entries) = parse_playlist_dump(dump, "https://example.com/playlist");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].url,
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    );
}

#[test]
fn playlist_dump_fabricates_youtube_url_for_youtube_source() {
    let dump = r#"{"id":"dQw4w9WgXcQ","title":"YT Video"}"#;
    let (_, entries) = parse_playlist_dump(dump, "https://www.youtube.com/playlist?list=PL123");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].url,
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    );
}

#[test]
fn playlist_dump_skips_non_youtube_entry_without_url() {
    let dump = r#"{"id":"12345","title":"Lesson 1","extractor_key":"BiliBiliCheese"}"#;
    let (_, entries) = parse_playlist_dump(dump, "https://www.bilibili.com/cheese/play/ss999");
    assert!(entries.is_empty());
}

#[test]
fn parse_progress_download_prefix() {
    assert_eq!(parse_progress_line("download:  45.2%"), Some(45.2));
}

#[test]
fn parse_progress_100_percent() {
    assert_eq!(parse_progress_line("download:100.0%"), Some(100.0));
}

#[test]
fn parse_progress_bare_percent() {
    assert_eq!(parse_progress_line("  92.5%"), Some(92.5));
}

#[test]
fn parse_progress_integer() {
    assert_eq!(parse_progress_line("download:100%"), Some(100.0));
}

#[test]
fn sha256sums_parsing() {
    let sums = "abc  not-ours\n\
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  yt-dlp.exe\n\
1111111111111111111111111111111111111111111111111111111111111111  yt-dlp_macos\n";
    assert_eq!(
        integrity::parse_sha256sums(sums, "yt-dlp.exe").as_deref(),
        Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );
    assert_eq!(
        integrity::parse_sha256sums(sums, "yt-dlp_macos").as_deref(),
        Some("1111111111111111111111111111111111111111111111111111111111111111")
    );
    assert_eq!(integrity::parse_sha256sums(sums, "yt-dlp_missing"), None);
    assert_eq!(integrity::parse_sha256sums("garbage line", "yt-dlp"), None);
}

#[test]
fn sha256_hex_known_vector() {
    assert_eq!(
        integrity::sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn ytdlp_release_base_per_channel() {
    assert!(ytdlp_release_base(YtdlpChannel::Stable).contains("yt-dlp/yt-dlp/releases"));
    assert!(ytdlp_release_base(YtdlpChannel::Nightly).contains("yt-dlp-nightly-builds"));
}

#[test]
fn archive_prefix_youtube_only() {
    assert_eq!(
        archive_extractor_prefix("https://www.youtube.com/@chan/videos"),
        Some("youtube")
    );
    assert_eq!(
        archive_extractor_prefix("https://youtu.be/abc"),
        Some("youtube")
    );
    assert_eq!(
        archive_extractor_prefix("https://space.bilibili.com/123"),
        None
    );
    assert_eq!(archive_extractor_prefix("not a url"), None);
}

#[test]
fn parse_speed_from_template() {
    let line = "download:  45.2%|eta:30|spd:1572864.0|dl:5242880|tot:11534336|est:NA";
    assert_eq!(parse_speed_line(line), Some(1572864.0));
    assert_eq!(parse_downloaded_bytes_line(line), Some(5242880));
    assert_eq!(parse_total_bytes_line(line), Some(11534336));
    assert_eq!(parse_eta_line(line), Some(30));
}

#[test]
fn parse_total_falls_back_to_estimate() {
    let line = "download:  10.0%|eta:NA|spd:NA|dl:1000|tot:NA|est:9999.0";
    assert_eq!(parse_total_bytes_line(line), Some(9999));
    assert_eq!(parse_speed_line(line), None);
}

#[test]
fn live_template_line_has_no_numeric_percent() {
    let line = "download:  N/A%|eta:NA|spd:518542.0|dl:3010560|tot:NA|est:NA";
    assert_eq!(parse_progress_line(line), None);
    assert_eq!(parse_downloaded_bytes_line(line), Some(3010560));
    assert_eq!(parse_speed_line(line), Some(518542.0));
}

#[test]
fn parse_size_token_units() {
    assert_eq!(parse_size_token("1B"), Some(1.0));
    assert_eq!(parse_size_token("2KiB"), Some(2048.0));
    assert_eq!(parse_size_token("2.87MiB"), Some(2.87 * 1024.0 * 1024.0));
    assert_eq!(parse_size_token("506.63KiB/s"), Some(506.63 * 1024.0));
    assert_eq!(parse_size_token("garbage"), None);
}

#[test]
fn parse_default_live_download_line() {
    let line = "[download]    2.87MiB at  506.63KiB/s (00:00:07) (frag 91/2097)";
    let (dl, spd) = parse_default_download_line(line).unwrap();
    assert!((dl - 2.87 * 1024.0 * 1024.0).abs() < 1.0);
    assert!((spd - 506.63 * 1024.0).abs() < 1.0);
}

#[test]
fn parse_default_download_line_ignores_percent_and_destination() {
    assert_eq!(
        parse_default_download_line("[download]  45.2% of 10.00MiB at 1.00MiB/s"),
        None
    );
    assert_eq!(
        parse_default_download_line("[download] Destination: video.mp4"),
        None
    );
}

#[test]
fn parse_progress_with_eta_field() {
    assert_eq!(parse_progress_line("download:  45.2%|eta:30"), Some(45.2));
}

#[test]
fn parse_eta_extracts_seconds() {
    assert_eq!(parse_eta_line("download:  45.2%|eta:30"), Some(30));
}

#[test]
fn parse_eta_na_returns_none() {
    assert_eq!(parse_eta_line("download:  45.2%|eta:NA"), None);
}

#[test]
fn parse_eta_missing_returns_none() {
    assert_eq!(parse_eta_line("download:  45.2%"), None);
}

#[test]
fn parse_eta_no_prefix_returns_none() {
    assert_eq!(parse_eta_line("  45.2%|eta:30"), None);
}

#[test]
fn parse_progress_garbage_returns_none() {
    assert_eq!(parse_progress_line("[info] Writing video subtitles"), None);
}

#[test]
fn parse_progress_aria2c_summary() {
    assert_eq!(
        parse_progress_line("[#1ce85c 35MiB/68MiB(50%) CN:8 DL:1.5MiB ETA:21s]"),
        Some(50.0)
    );
}

#[test]
fn parse_progress_aria2c_decimal() {
    assert_eq!(
        parse_progress_line("[#abc 100MiB/200MiB(50.5%) CN:5 DL:2MiB]"),
        Some(50.5)
    );
}

#[test]
fn parse_progress_aria2c_no_paren_returns_none() {
    assert_eq!(parse_progress_line("[#abc NOTICE]"), None);
}

#[test]
fn parse_progress_empty_returns_none() {
    assert_eq!(parse_progress_line(""), None);
}

#[test]
fn parse_destination_standard() {
    assert_eq!(
        parse_destination_line("[download] Destination: /tmp/video.mp4"),
        Some("/tmp/video.mp4".to_string())
    );
}

#[test]
fn parse_destination_merger() {
    assert_eq!(
        parse_destination_line("[Merger] Merging formats into \"/tmp/video.mp4\""),
        Some("/tmp/video.mp4".to_string())
    );
}

#[test]
fn parse_destination_no_match() {
    assert_eq!(parse_destination_line("[download] 100% of 50.0MiB"), None);
}

#[test]
fn parse_destination_empty_path_returns_none() {
    assert_eq!(parse_destination_line("[download] Destination:"), None);
}

#[test]
fn is_youtube_url_standard() {
    assert!(is_youtube_url(
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    ));
}

#[test]
fn is_youtube_url_short() {
    assert!(is_youtube_url("https://youtu.be/dQw4w9WgXcQ"));
}

#[test]
fn is_youtube_url_case_insensitive() {
    assert!(is_youtube_url("https://www.YouTube.com/watch?v=test"));
}

#[test]
fn is_youtube_url_other_site() {
    assert!(!is_youtube_url("https://vimeo.com/123456"));
}

#[test]
fn sanitize_strips_query_params() {
    let input = "Error downloading https://example.com/video?token=secret&key=123 failed";
    let result = sanitize_log_line(input);
    assert_eq!(result, "Error downloading https://example.com/video failed");
}

#[test]
fn sanitize_preserves_url_without_query() {
    let input = "Error downloading https://example.com/video failed";
    let result = sanitize_log_line(input);
    assert_eq!(result, input);
}

#[test]
fn sanitize_multiple_urls() {
    let input = "from https://a.com/x?s=1 to https://b.com/y?t=2 done";
    let result = sanitize_log_line(input);
    assert_eq!(result, "from https://a.com/x to https://b.com/y done");
}

#[test]
fn sanitize_no_urls() {
    let input = "plain error message";
    let result = sanitize_log_line(input);
    assert_eq!(result, input);
}

#[test]
fn extract_id_youtube_standard() {
    assert_eq!(
        extract_id_from_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
        Some("dQw4w9WgXcQ".to_string())
    );
}

#[test]
fn extract_id_youtu_be() {
    assert_eq!(
        extract_id_from_url("https://youtu.be/dQw4w9WgXcQ"),
        Some("dQw4w9WgXcQ".to_string())
    );
}

#[test]
fn extract_id_youtube_embed() {
    assert_eq!(
        extract_id_from_url("https://www.youtube.com/embed/dQw4w9WgXcQ"),
        Some("dQw4w9WgXcQ".to_string())
    );
}

#[test]
fn extract_id_shorts() {
    assert_eq!(
        extract_id_from_url("https://www.youtube.com/shorts/abc123"),
        Some("abc123".to_string())
    );
}

#[test]
fn extract_id_non_youtube() {
    assert_eq!(extract_id_from_url("https://vimeo.com/123456"), None);
}

#[test]
fn translate_error_429() {
    let err = translate_ytdlp_error("HTTP Error 429: Too Many Requests");
    assert!(err.to_string().contains("429"));
}

#[test]
fn translate_error_403() {
    let err = translate_ytdlp_error("HTTP Error 403: Forbidden");
    assert!(err.to_string().contains("403"));
}

#[test]
fn translate_error_403_midway_is_not_a_private_video() {
    let err =
        translate_ytdlp_error("ERROR: unable to download video data: HTTP Error 403: Forbidden")
            .to_string();
    assert!(err.contains("403"));
    assert!(!err.contains("private"), "{err}");
}

#[test]
fn translate_error_nsig() {
    let err = translate_ytdlp_error("nsig extraction failed");
    assert!(err.to_string().contains("extraction"));
}

#[test]
fn translate_error_unavailable() {
    let err = translate_ytdlp_error("Video unavailable");
    assert!(err.to_string().contains("unavailable"));
}

#[test]
fn translate_error_requested_format() {
    let err = translate_ytdlp_error("ERROR: Requested format is not available. Use --list-formats for a list of available formats");
    assert!(err.to_string().contains("Requested format"), "Got: {}", err);
    assert!(
        !err.to_string().contains("Video unavailable"),
        "Should not contain 'Video unavailable', got: {}",
        err
    );
}

#[test]
fn translate_error_private() {
    let err = translate_ytdlp_error("This is a private video");
    assert!(err.to_string().contains("private"));
}

#[test]
fn translate_error_timeout() {
    let err = translate_ytdlp_error("Connection timed out");
    assert!(err.to_string().contains("timed out"));
}

#[test]
fn bilibili_412_is_a_platform_block_not_a_restriction() {
    let stderr = "ERROR: [BiliBiliBangumi] 1933368: Unable to download webpage: HTTP Error 412: Precondition Failed (caused by <HTTPError 412: Precondition Failed>)";
    let msg = translate_ytdlp_error(stderr).to_string();
    assert!(msg.contains("412"), "{msg}");
    let (category, _) = crate::core::errors::classify_download_error(&msg);
    assert_eq!(category, "blocked_by_platform", "{msg}");
    assert!(!crate::core::errors::is_terminal_category(category));
    let own = translate_ytdlp_error("ERROR: [BilibiliSpaceVideo] 1: Request is blocked by server (412), please wait and try later.").to_string();
    assert_eq!(
        crate::core::errors::classify_download_error(&own).0,
        "blocked_by_platform",
        "{own}"
    );
}

#[test]
fn webpage_failures_keep_their_status() {
    let msg = translate_ytdlp_error(
        "ERROR: [x] 1: Unable to download webpage: HTTP Error 404: Not Found",
    )
    .to_string();
    assert!(msg.contains("HTTP 404"), "{msg}");
    assert_eq!(
        crate::core::errors::classify_download_error(&msg).0,
        "not_found"
    );
    let msg = translate_ytdlp_error(
        "ERROR: [x] 1: Unable to download webpage: HTTP Error 502: Bad Gateway",
    )
    .to_string();
    assert_eq!(
        crate::core::errors::classify_download_error(&msg).0,
        "server_error"
    );
}

#[test]
fn twitter_500_is_a_retryable_server_error() {
    let stderr = "ERROR: [twitter:amplify] 665052190608723968: Unable to download webpage: HTTP Error 500: Domain Not Found (caused by <HTTPError 500: Domain Not Found>)";
    let err = translate_ytdlp_error(stderr);
    for text in [err.to_string(), format!("{err:#}")] {
        let (cat, _) = crate::core::errors::classify_download_error(&text);
        assert_eq!(cat, "server_error", "{text}");
        assert!(!crate::core::errors::is_terminal_category(cat));
    }
}

#[test]
fn translate_error_instagram_empty_media_is_login_not_broken_extractor() {
    let stderr = "ERROR: [Instagram] BkfuX9UB-eK: Instagram sent an empty media response. Check if this post is accessible in your browser without being logged-in. If it is not, then use --cookies-from-browser or --cookies for the authentication. Otherwise, if the post is accessible in browser without being logged-in, please report this issue on  https://github.com/yt-dlp/yt-dlp/issues?q= , filling out the appropriate issue template. Confirm you are on the latest version using  yt-dlp -U";
    let err = translate_ytdlp_error(stderr);
    let msg = err.to_string();
    assert!(!msg.contains("extractor is broken"), "{msg}");
    assert_eq!(
        crate::core::errors::classify_download_error(&msg).0,
        "auth_required",
        "{msg}"
    );
    assert_eq!(
        crate::core::errors::classify_download_error(&format!("{err:#}")).0,
        "auth_required"
    );
}

#[test]
fn translate_error_broken_extractor_stays_classifiable() {
    let msg = translate_ytdlp_error(
        "ERROR: [BiliBili] 1: Unable to extract initial state; please report this issue",
    )
    .to_string();
    assert_eq!(
        crate::core::errors::classify_download_error(&msg).0,
        "extractor_failure",
        "{msg}"
    );
}

#[test]
fn egress_failures_are_egress_not_broken_extractor() {
    for stderr in [
            "ERROR: [generic] x: Unable to download webpage: ('Unable to connect to proxy', OSError('Tunnel connection failed: 403 Forbidden')) (caused by ProxyError(...)); please report this issue on https://github.com/yt-dlp/yt-dlp/issues",
            "ERROR: [Bluesky] x: Unable to download JSON metadata: <urlopen error [SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed: self-signed certificate (_ssl.c:1006)>",
            "ERROR: [generic] x: Unable to download webpage: <urlopen error [Errno 8] nodename nor servname provided, or not known>",
        ] {
            let err = translate_ytdlp_error(stderr);
            let msg = err.to_string();
            assert!(!msg.contains("extractor is broken"), "{msg}");
            assert_eq!(crate::core::errors::classify_download_error(&msg).0, "egress_failed", "{msg}");
            assert_eq!(crate::core::errors::classify_download_error(&format!("{err:#}")).0, "egress_failed");
        }
}

#[test]
fn worker_commands_reach_the_operator_sink_redacted() {
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_seen = seen.clone();
    super::set_command_sink(Some(Arc::new(move |line: &str| {
        sink_seen.lock().unwrap().push(line.to_string())
    })));
    let args: Vec<String> = [
            "-f", "bv*[height<=720]+ba/b[height<=720]/ba",
            "--proxy", "http://omniget:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef@127.0.0.1:4242/",
            "--add-headers", "Cookie:SESSDATA=SYNTHETIC_SECRET",
            "--add-headers", "Authorization:Bearer SYNTHETIC_SECRET",
            "--add-headers", "X-CSRFToken:SYNTHETIC_SECRET",
            "--add-headers", "Referer:https://www.bilibili.com",
            "--username", "SYNTHETIC_SECRET",
            "--extractor-args", "youtube:po_token=web.gvs+SYNTHETIC_SECRET",
            "https://www.bilibili.com/video/av114868162141203",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    super::note_operator_command(std::path::Path::new("/bin/yt-dlp.pyz"), &args, "native");
    super::set_command_sink(None);
    let lines = seen.lock().unwrap().clone();
    let line = lines
        .iter()
        .find(|l| l.contains("av114868162141203"))
        .expect("command reached the sink");
    assert!(line.starts_with("[native] $ yt-dlp "), "{line}");
    assert!(line.contains("height<=720"), "{line}");
    assert!(line.contains("Referer:https://www.bilibili.com"), "{line}");
    assert!(!line.contains("SYNTHETIC_SECRET"), "{line}");
    assert!(!line.contains("0123456789abcdef"), "{line}");
}

#[test]
fn raw_error_line_is_context_redacted_and_not_in_summary() {
    let stderr = "WARNING: x\nERROR: [x] 1: Unable to download webpage: HTTP Error 404: Not Found (https://cdn.example.com/a.m4s?token=SYNTHETIC_SECRET&sig=SYNTHETIC_SECRET)";
    let err = translate_ytdlp_error(stderr);
    let summary = err.to_string();
    let full = format!("{err:#}");
    assert!(!summary.contains("cdn.example.com"), "{summary}");
    assert!(
        full.contains("Unable to download webpage: HTTP Error 404"),
        "{full}"
    );
    assert!(!full.contains("SYNTHETIC_SECRET"), "{full}");
}

#[test]
fn translate_error_unknown_falls_through() {
    let err = translate_ytdlp_error("ERROR: some unknown thing happened");
    assert!(err.to_string().contains("yt-dlp"));
}

#[test]
fn parse_formats_empty_json() {
    let json = serde_json::json!({});
    assert!(parse_formats(&json).is_empty());
}

#[test]
fn parse_formats_extracts_fields() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "22",
                "ext": "mp4",
                "width": 1280,
                "height": 720,
                "fps": 30.0,
                "vcodec": "avc1.64001F",
                "acodec": "mp4a.40.2",
                "filesize": 50_000_000u64,
                "tbr": 2500.0,
                "format_note": "720p"
            }
        ]
    });
    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 1);
    assert_eq!(formats[0].format_id, "22");
    assert_eq!(formats[0].height, Some(720));
    assert!(formats[0].has_video);
    assert!(formats[0].has_audio);
    assert_eq!(formats[0].resolution, Some("1280x720".to_string()));
}

#[test]
fn parse_formats_video_only() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "137",
                "ext": "mp4",
                "width": 1920,
                "height": 1080,
                "vcodec": "avc1.640028",
                "acodec": "none"
            }
        ]
    });
    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 1);
    assert!(formats[0].has_video);
    assert!(!formats[0].has_audio);
}

#[test]
fn aria2c_nunca_recebe_manifesto_fragmentado() {
    let args = aria2c_downloader_args("/opt/bin/aria2c", 8, None);
    let downloader = &args[1];
    assert!(downloader.starts_with("http,ftp:"), "{downloader}");
    for proto in ["m3u8", "m3u8_native", "m3u8_frag_urls", "dash_frag_urls"] {
        assert!(
            !downloader.contains(proto),
            "protocolo {proto} nao pode ir ao aria2c"
        );
    }
    assert_eq!(downloader, "http,ftp:/opt/bin/aria2c");
    assert!(args[3].starts_with("aria2c:"), "{}", args[3]);
}

#[test]
fn aria2c_downloader_args_rejeita_proxy_com_espaco() {
    let limpo = aria2c_downloader_args("/bin/aria2c", 2, Some("http://127.0.0.1:8080"));
    assert!(
        limpo[3].ends_with("--all-proxy=http://127.0.0.1:8080"),
        "{}",
        limpo[3]
    );
    let sujo = aria2c_downloader_args("/bin/aria2c", 2, Some("http://h:1 --seed-ratio=0"));
    assert!(!sujo[3].contains("--all-proxy"), "{}", sujo[3]);
    assert!(!sujo[3].contains("--seed-ratio"));
}

#[test]
fn aria2c_nunca_zera_conexoes() {
    let args = aria2c_downloader_args("/bin/aria2c", 0, None);
    assert!(
        args[3].contains("-x 1") && args[3].contains("-j 1"),
        "{}",
        args[3]
    );
}

#[test]
fn parse_ytdlp_version_le_os_formatos_reais() {
    assert_eq!(parse_ytdlp_version("2026.06.09"), Some((2026, 6, 9)));
    assert_eq!(
        parse_ytdlp_version("2025.12.31.232815"),
        Some((2025, 12, 31))
    );
    assert_eq!(
        parse_ytdlp_version("2026.06.10.232815-nightly"),
        Some((2026, 6, 10))
    );
    for lixo in ["", "unknown", "1.2", "2026.13.01", "2026.06.99"] {
        assert_eq!(parse_ytdlp_version(lixo), None, "aceitou {lixo:?}");
    }
}

#[test]
fn piso_de_versao_cobre_as_cves_de_2026_06_09() {
    assert_eq!(ytdlp_version_is_supported("2026.06.09"), Some(true));
    assert_eq!(ytdlp_version_is_supported("2026.07.04"), Some(true));
    assert_eq!(ytdlp_version_is_supported("2026.06.08"), Some(false));
    assert_eq!(ytdlp_version_is_supported("sei la"), None);
}

const SABR_STDERR: &str = "WARNING: [youtube] abc123: Some web client https formats have been \
skipped as they are missing a url. YouTube is forcing SABR streaming for this client. \
See  https://github.com/yt-dlp/yt-dlp/issues/12482  for more details\n\
ERROR: [youtube] abc123: Requested format is not available";

#[test]
fn sabr_no_stderr_dispara_a_cascata_na_ordem() {
    let lower = SABR_STDERR.to_lowercase();
    assert_eq!(
        sabr_fallback_client(&lower, 0).as_deref(),
        Some("youtube:player_client=tv_downgraded")
    );
    assert_eq!(
        sabr_fallback_client(&lower, 1).as_deref(),
        Some("youtube:player_client=ios")
    );
    assert_eq!(
        sabr_fallback_client(&lower, 2).as_deref(),
        Some("youtube:player_client=android")
    );
    assert_eq!(
        sabr_fallback_client(&lower, 3).as_deref(),
        Some("youtube:player_client=web_safari")
    );
}

#[test]
fn cascata_esgotada_devolve_none_em_vez_de_repetir() {
    let lower = SABR_STDERR.to_lowercase();
    assert_eq!(sabr_fallback_client(&lower, 4), None);
    assert_eq!(sabr_fallback_client(&lower, 99), None);
}

#[test]
fn erro_que_nao_e_sabr_nao_troca_de_client() {
    for stderr in [
        "error: [youtube] abc: video unavailable",
        "error: http error 429: too many requests",
        "error: [youtube] abc: nsig extraction failed",
        "error: unable to download webpage: timed out",
        "",
    ] {
        assert_eq!(
            sabr_fallback_client(stderr, 0),
            None,
            "trocou de client sem SABR: {stderr:?}"
        );
    }
}

#[test]
fn a_cascata_nao_contem_o_client_que_causa_o_problema() {
    assert!(!SABR_CLIENT_CASCADE.contains(&"web"));
    assert!(!SABR_CLIENT_CASCADE.contains(&"default"));
    assert_eq!(SABR_CLIENT_CASCADE.len(), 4);
}

#[test]
fn sem_variavel_nao_ha_provedor_de_pot_token() {
    let restore = std::env::var("OMNIGET_POT_PROVIDER_URL").ok();

    std::env::remove_var("OMNIGET_POT_PROVIDER_URL");
    assert_eq!(pot_provider_base_url(), None, "sem env nao ha provedor");

    std::env::set_var("OMNIGET_POT_PROVIDER_URL", "   ");
    assert_eq!(
        pot_provider_base_url(),
        None,
        "espaco em branco viraria argumento quebrado no yt-dlp"
    );

    std::env::set_var("OMNIGET_POT_PROVIDER_URL", "localhost:4416/");
    assert_eq!(
        pot_provider_base_url(),
        Some("http://localhost:4416".to_string()),
        "o que o usuario digita tem que virar URL valida"
    );

    match restore {
        Some(v) => std::env::set_var("OMNIGET_POT_PROVIDER_URL", v),
        None => std::env::remove_var("OMNIGET_POT_PROVIDER_URL"),
    }
}

#[tokio::test]
async fn provedor_morto_nao_gera_argumento() {
    let health = probe_pot_provider("http://127.0.0.1:1").await;
    assert!(
        !health.is_usable(),
        "endereco morto nao pode ser considerado utilizavel"
    );
    assert!(
        super::super::pot_provider::extractor_args(&health, "http://127.0.0.1:1").is_empty(),
        "provedor inalcancavel nao pode virar --extractor-args"
    );
}
