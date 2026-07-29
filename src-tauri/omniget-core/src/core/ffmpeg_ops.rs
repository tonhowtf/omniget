// Security-critical: validates the *transform* portion of an ffmpeg command
// (the args between the app-controlled input and output). AI/NL output is
// never executed without passing this allowlist, and input/output paths are
// injected by the app — never taken from the model — so this only has to
// guarantee the middle args can't read arbitrary files or escape into a shell.

const FLAGS_NO_VALUE: &[&str] = &["-y", "-vn", "-an", "-sn", "-dn", "-nostdin", "-shortest"];

const FLAGS_WITH_VALUE: &[&str] = &[
    "-ss",
    "-to",
    "-t",
    "-r",
    "-ar",
    "-ac",
    "-crf",
    "-preset",
    "-b:v",
    "-b:a",
    "-q:v",
    "-q:a",
    "-c",
    "-c:v",
    "-c:a",
    "-codec",
    "-pix_fmt",
    "-s",
    "-aspect",
    "-movflags",
    "-map",
    "-vf",
    "-af",
    "-filter:v",
    "-filter:a",
    "-metadata",
    "-threads",
    "-g",
    "-bf",
    "-profile:v",
    "-level",
    "-maxrate",
    "-bufsize",
    "-tune",
];

const FILTER_FLAGS: &[&str] = &["-vf", "-af", "-filter:v", "-filter:a"];

const ALLOWED_FILTERS: &[&str] = &[
    "scale",
    "crop",
    "fps",
    "transpose",
    "hflip",
    "vflip",
    "rotate",
    "pad",
    "setpts",
    "atempo",
    "volume",
    "aresample",
    "loudnorm",
    "silenceremove",
    "silencedetect",
    "afade",
    "fade",
    "format",
    "eq",
    "unsharp",
    "deshake",
    "setsar",
    "setdar",
    "lutyuv",
    "hue",
    "colorbalance",
    "gblur",
    "boxblur",
    "noise",
    "asetpts",
    "anull",
    "null",
    "palettegen",
    "paletteuse",
    "split",
];

const FORBIDDEN_SUBSTR: &[&str] = &[
    ";",
    "|",
    "&",
    "`",
    "$(",
    "${",
    "\n",
    "\r",
    ">",
    "<",
    "..",
    "movie=",
    "amovie=",
    "subtitles=",
    "ass=",
    "concat:",
    "file:",
    "pipe:",
    "\\",
    "://",
];

fn token_is_safe(tok: &str) -> bool {
    !FORBIDDEN_SUBSTR.iter().any(|bad| tok.contains(bad))
}

fn validate_filter_value(value: &str) -> Result<(), String> {
    // filterchains are separated by ',' (chain) and ';' is already forbidden.
    for filt in value.split(',') {
        let filt = filt.trim();
        if filt.is_empty() {
            continue;
        }
        let name: String = filt
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || !ALLOWED_FILTERS.contains(&name.as_str()) {
            return Err(format!("Filter not allowed: {}", name));
        }
    }
    Ok(())
}

pub fn validate_transform_args(args: &[String]) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if !token_is_safe(a) {
            return Err(format!("Unsafe token: {}", a));
        }
        if !a.starts_with('-') {
            return Err(format!("Unexpected positional argument: {}", a));
        }
        if FLAGS_NO_VALUE.contains(&a.as_str()) {
            i += 1;
            continue;
        }
        if FLAGS_WITH_VALUE.contains(&a.as_str()) {
            let Some(value) = args.get(i + 1) else {
                return Err(format!("Missing value for {}", a));
            };
            if !token_is_safe(value) {
                return Err(format!("Unsafe value for {}", a));
            }
            if FILTER_FLAGS.contains(&a.as_str()) {
                validate_filter_value(value)?;
            }
            i += 2;
            continue;
        }
        return Err(format!("Flag not allowed: {}", a));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Preset {
    pub args: Vec<String>,
    pub out_ext: &'static str,
}

fn s(v: &str) -> String {
    v.to_string()
}

// Deterministic, known-safe operations. These never go through the validator
// because they are constructed here, but they also pass it (covered by tests).
/// Alvo EBU R128 de -16 LUFS com teto de -1.5 dBTP: o mesmo que apps de
/// podcast usam para fala. `LRA=11` mantem alguma dinamica em vez de achatar.
pub const VOICE_BOOST_FILTER: &str = "loudnorm=I=-16:TP=-1.5:LRA=11";

/// Remove silencio em qualquer ponto (`stop_periods=-1`), a partir de 0.35 s
/// abaixo de -38 dB. Abaixo disso a fala fica picotada entre palavras.
pub const SMART_SPEED_FILTER: &str =
    "silenceremove=stop_periods=-1:stop_duration=0.35:stop_threshold=-38dB";

/// Argumentos que medem o silencio sem escrever arquivo, para estimar o ganho
/// de duracao antes de o usuario aplicar.
///
/// Nao passa por `validate_transform_args`: usa `-f null`, e `-f` fica fora da
/// allowlist de proposito. Sao argumentos constantes montados pelo app, sem
/// nenhuma entrada de usuario ou de modelo.
pub fn silence_probe_args() -> Vec<String> {
    vec![
        s("-vn"),
        s("-af"),
        s("silencedetect=noise=-38dB:d=0.35"),
        s("-f"),
        s("null"),
    ]
}

/// Total de silencio detectado, em segundos, a partir do stderr do ffmpeg.
///
/// O ffmpeg emite `silence_start` e `silence_end` em linhas separadas; um
/// `silence_start` sem `silence_end` significa que o arquivo terminou em
/// silencio e e ignorado, porque nao da para saber a duracao sem a duracao
/// total do arquivo.
pub fn parse_silence_total_secs(stderr: &str) -> f64 {
    let mut total = 0.0;
    for line in stderr.lines() {
        let Some(idx) = line.find("silence_duration:") else {
            continue;
        };
        let rest = &line[idx + "silence_duration:".len()..];
        if let Some(value) = rest.split_whitespace().next() {
            if let Ok(secs) = value.parse::<f64>() {
                if secs.is_finite() && secs > 0.0 {
                    total += secs;
                }
            }
        }
    }
    total
}

pub fn preset(action: &str, start: Option<&str>, end: Option<&str>) -> Result<Preset, String> {
    match action {
        "extract_audio" => Ok(Preset {
            args: vec![s("-vn"), s("-c:a"), s("libmp3lame"), s("-q:a"), s("2")],
            out_ext: "mp3",
        }),
        "mute" => Ok(Preset {
            args: vec![s("-c"), s("copy"), s("-an")],
            out_ext: "mp4",
        }),
        "to_mp4" => Ok(Preset {
            args: vec![
                s("-c:v"),
                s("libx264"),
                s("-c:a"),
                s("aac"),
                s("-preset"),
                s("fast"),
                s("-crf"),
                s("23"),
                s("-movflags"),
                s("+faststart"),
            ],
            out_ext: "mp4",
        }),
        // Voice Boost: normaliza volume para EBU R128 (-16 LUFS, o alvo usado
        // por apps de podcast) sem tocar no video.
        "voice_boost" => Ok(Preset {
            args: vec![
                s("-af"),
                s(VOICE_BOOST_FILTER),
                s("-c:v"),
                s("copy"),
                s("-c:a"),
                s("aac"),
                s("-b:a"),
                s("192k"),
            ],
            out_ext: "mp4",
        }),
        // Smart Speed: corta o silencio de aula gravada.
        //
        // Saida e AUDIO, nao video, e isso e uma restricao real e nao uma
        // simplificacao: `silenceremove` descarta amostras de audio sem mexer no
        // PTS do video, entao aplicar sobre um mp4 dessincronizaria imagem e som.
        // Cortar as duas streams nos mesmos pontos exigiria `select`/`aselect`
        // com expressoes, que e outra ordem de grandeza de trabalho — e o caso de
        // uso que originou a feature (Overcast) consome aula como audio de
        // qualquer forma. Vem com loudnorm junto porque quem precisa de um
        // costuma precisar do outro.
        "smart_speed" => Ok(Preset {
            args: vec![
                s("-vn"),
                s("-af"),
                s(&format!("{SMART_SPEED_FILTER},{VOICE_BOOST_FILTER}")),
                s("-c:a"),
                s("aac"),
                s("-b:a"),
                s("128k"),
            ],
            out_ext: "m4a",
        }),
        "to_gif" => Ok(Preset {
            args: vec![s("-vf"), s("fps=12,scale=480:-1:flags=lanczos"), s("-an")],
            out_ext: "gif",
        }),
        "trim" => {
            let st = start.unwrap_or("").trim();
            let en = end.unwrap_or("").trim();
            if st.is_empty() && en.is_empty() {
                return Err("Trim needs a start or end".to_string());
            }
            let valid = |v: &str| {
                !v.is_empty()
                    && v.chars()
                        .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
            };
            let mut args = Vec::new();
            if !st.is_empty() {
                if !valid(st) {
                    return Err("Invalid start time".to_string());
                }
                args.push(s("-ss"));
                args.push(st.to_string());
            }
            if !en.is_empty() {
                if !valid(en) {
                    return Err("Invalid end time".to_string());
                }
                args.push(s("-to"));
                args.push(en.to_string());
            }
            args.push(s("-c"));
            args.push(s("copy"));
            Ok(Preset {
                args,
                out_ext: "mp4",
            })
        }
        _ => Err(format!("Unknown action: {}", action)),
    }
}

// Parses a model-proposed ffmpeg invocation into validated transform args.
// Strips a leading "ffmpeg", drops any -i/input and the trailing output
// (paths are app-controlled), then runs the allowlist.
pub fn sanitize_ai_command(raw: &str) -> Result<Vec<String>, String> {
    let raw = raw.trim();
    if raw.contains('\n') || raw.contains('\r') {
        return Err("Command must be a single line".to_string());
    }
    let toks: Vec<String> = raw.split_whitespace().map(|t| t.to_string()).collect();
    if toks.is_empty() {
        return Err("Empty command".to_string());
    }
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    if toks[0].eq_ignore_ascii_case("ffmpeg") {
        i = 1;
    }
    while i < toks.len() {
        let t = &toks[i];
        if t == "-i" {
            // skip -i and its path (app injects the real input)
            i += 2;
            continue;
        }
        // drop a trailing bare output path (last token, not a flag/value)
        if !t.starts_with('-') && i == toks.len() - 1 {
            i += 1;
            continue;
        }
        out.push(t.clone());
        i += 1;
    }
    validate_transform_args(&out)?;
    Ok(out)
}

// Pulls scene-change timestamps out of ffmpeg's showinfo stderr (lines like
// "... showinfo ... pts_time:12.34 ...").
pub fn parse_scene_times(stderr: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        if !line.contains("showinfo") && !line.contains("pts_time") {
            continue;
        }
        if let Some(pos) = line.find("pts_time:") {
            let rest = &line[pos + "pts_time:".len()..];
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(v) = num.parse::<f64>() {
                out.push(v);
            }
        }
    }
    out
}

// Reduces mono s16le PCM to `buckets` normalized peak values (0.0..=1.0).
pub fn pcm_s16le_peaks(bytes: &[u8], buckets: usize) -> Vec<f32> {
    if buckets == 0 || bytes.len() < 2 {
        return Vec::new();
    }
    let sample_count = bytes.len() / 2;
    let per_bucket = sample_count.div_ceil(buckets).max(1);
    let mut peaks = Vec::with_capacity(buckets);
    let mut cur_max: i32 = 0;
    let mut in_bucket = 0usize;
    for i in 0..sample_count {
        let lo = bytes[i * 2] as i16;
        let hi = bytes[i * 2 + 1] as i16;
        let sample = ((hi << 8) | (lo & 0xff)) as i32;
        let amp = sample.unsigned_abs() as i32;
        if amp > cur_max {
            cur_max = amp;
        }
        in_bucket += 1;
        if in_bucket >= per_bucket {
            peaks.push((cur_max as f32 / 32768.0).min(1.0));
            cur_max = 0;
            in_bucket = 0;
        }
    }
    if in_bucket > 0 {
        peaks.push((cur_max as f32 / 32768.0).min(1.0));
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scene_times() {
        let stderr = "[Parsed_showinfo_1 @ 0x] n:0 pts_time:1.5 type:I\n\
                      garbage line\n\
                      [Parsed_showinfo_1 @ 0x] n:1 pts_time:42.0 type:P";
        assert_eq!(parse_scene_times(stderr), vec![1.5, 42.0]);
    }

    #[test]
    fn peaks_bucketize_and_normalize() {
        // 4 samples: 0, 16384, -32768, 8192 (s16le little-endian bytes)
        let bytes: Vec<u8> = vec![
            0x00, 0x00, // 0
            0x00, 0x40, // 16384
            0x00, 0x80, // -32768
            0x00, 0x20, // 8192
        ];
        let peaks = pcm_s16le_peaks(&bytes, 2);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[1] - 1.0).abs() < 0.01);
        assert!(peaks[0] <= 1.0 && peaks[0] >= 0.0);
    }

    #[test]
    fn peaks_empty_safe() {
        assert!(pcm_s16le_peaks(&[], 4).is_empty());
        assert!(pcm_s16le_peaks(&[1, 2, 3, 4], 0).is_empty());
    }

    #[test]
    fn allows_safe_transform() {
        let args: Vec<String> = ["-c:v", "libx264", "-crf", "23", "-an"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(validate_transform_args(&args).is_ok());
    }

    #[test]
    fn rejects_unknown_flag() {
        let args = vec!["-attach".to_string(), "x".to_string()];
        assert!(validate_transform_args(&args).is_err());
    }

    #[test]
    fn rejects_shell_metachars() {
        let args = vec!["-vf".to_string(), "scale=2:2; rm -rf /".to_string()];
        assert!(validate_transform_args(&args).is_err());
    }

    #[test]
    fn rejects_file_reading_filters() {
        let args = vec!["-vf".to_string(), "movie=/etc/passwd".to_string()];
        assert!(validate_transform_args(&args).is_err());
        let args2 = vec!["-vf".to_string(), "subtitles=secret.srt".to_string()];
        assert!(validate_transform_args(&args2).is_err());
    }

    #[test]
    fn rejects_input_flag_in_transform() {
        let args = vec!["-i".to_string(), "/etc/passwd".to_string()];
        assert!(validate_transform_args(&args).is_err());
    }

    #[test]
    fn allows_known_filterchain() {
        let args = vec![
            "-vf".to_string(),
            "fps=12,scale=480:-1:flags=lanczos".to_string(),
        ];
        assert!(validate_transform_args(&args).is_ok());
    }

    #[test]
    fn presets_pass_validation() {
        for action in ["extract_audio", "mute", "to_mp4", "to_gif"] {
            let p = preset(action, None, None).unwrap();
            assert!(
                validate_transform_args(&p.args).is_ok(),
                "preset {action} failed validation"
            );
        }
        let trim = preset("trim", Some("00:00:01"), Some("00:00:05")).unwrap();
        assert!(validate_transform_args(&trim.args).is_ok());
    }

    #[test]
    fn trim_rejects_bad_time() {
        assert!(preset("trim", Some("1; rm"), None).is_err());
    }

    #[test]
    fn sanitize_strips_ffmpeg_io() {
        let cmd = "ffmpeg -i input.mp4 -c:v libx264 -crf 20 out.mp4";
        let args = sanitize_ai_command(cmd).unwrap();
        assert_eq!(args, vec!["-c:v", "libx264", "-crf", "20"]);
    }

    #[test]
    fn sanitize_rejects_dangerous_ai_output() {
        assert!(sanitize_ai_command("ffmpeg -i in -vf subtitles=x.srt out").is_err());
        assert!(sanitize_ai_command("ffmpeg -i in -c copy out && rm -rf /").is_err());
    }

    // --- B36: Smart Speed e Voice Boost ---

    /// stderr real do ffmpeg com `silencedetect`, de uma aula gravada.
    const SILENCEDETECT_STDERR: &str = "\
[silencedetect @ 0x7f8] silence_start: 12.4213\n\
[silencedetect @ 0x7f8] silence_end: 15.0021 | silence_duration: 2.58076\n\
[silencedetect @ 0x7f8] silence_start: 48.9\n\
[silencedetect @ 0x7f8] silence_end: 52.4 | silence_duration: 3.5\n\
[silencedetect @ 0x7f8] silence_start: 300.1\n\
size=N/A time=00:10:00.00 bitrate=N/A speed= 412x\n";

    #[test]
    fn estimativa_de_silencio_soma_so_os_intervalos_fechados() {
        let total = parse_silence_total_secs(SILENCEDETECT_STDERR);
        // 2.58076 + 3.5; o terceiro silence_start nao tem end e e ignorado.
        assert!((total - 6.08076).abs() < 1e-6, "somou {total}");
    }

    #[test]
    fn estimativa_ignora_log_sem_silencio() {
        assert_eq!(parse_silence_total_secs(""), 0.0);
        assert_eq!(parse_silence_total_secs("size=N/A time=00:10:00.00"), 0.0);
        // Valor ilegivel nao pode virar NaN e contaminar a soma.
        assert_eq!(
            parse_silence_total_secs("silence_duration: nao-e-numero"),
            0.0
        );
    }

    #[test]
    fn smart_speed_sai_como_audio_e_nao_como_video() {
        // Restricao real, nao escolha estetica: silenceremove descarta amostras
        // de audio sem mexer no PTS do video, entao saida em mp4
        // dessincronizaria. O teste trava essa decisao.
        let p = preset("smart_speed", None, None).expect("preset existe");
        assert_eq!(p.out_ext, "m4a");
        assert!(p.args.contains(&"-vn".to_string()), "{:?}", p.args);
        assert!(!p.args.iter().any(|a| a == "-c:v"), "{:?}", p.args);
    }

    #[test]
    fn voice_boost_preserva_o_video() {
        let p = preset("voice_boost", None, None).expect("preset existe");
        assert_eq!(p.out_ext, "mp4");
        let idx = p.args.iter().position(|a| a == "-c:v").expect("tem -c:v");
        assert_eq!(p.args[idx + 1], "copy");
        assert!(!p.args.contains(&"-vn".to_string()));
    }

    #[test]
    fn os_presets_novos_passam_pela_allowlist_de_seguranca() {
        // Defesa em profundidade: video_op_preset revalida os args do preset
        // antes de executar, entao um preset que nao passe aqui quebraria em
        // runtime em vez de na compilacao.
        for action in ["smart_speed", "voice_boost"] {
            let p = preset(action, None, None).expect("preset existe");
            validate_transform_args(&p.args)
                .unwrap_or_else(|e| panic!("preset {action} reprovado na allowlist: {e}"));
        }
    }

    #[test]
    fn a_sonda_de_silencio_nao_passa_pela_allowlist_de_proposito() {
        // Ela usa `-f null`, e `-f` NAO esta na allowlist — de proposito. Liberar
        // `-f` deixaria o caminho de IA escolher muxer arbitrario, o que amplia
        // superficie de seguranca para ganhar nada. A sonda e montada so pelo app,
        // com argumentos constantes e sem entrada do usuario, entao nao precisa
        // passar por la. Este teste trava as duas metades dessa decisao.
        let args = silence_probe_args();
        assert!(validate_transform_args(&args).is_err());
        assert!(args.iter().all(|a| !a.contains(';')
            && !a.contains('|')
            && !a.contains('`')
            && !a.contains("$(")));
        assert_eq!(
            args,
            silence_probe_args(),
            "argumentos precisam ser constantes"
        );
    }

    #[test]
    fn filtros_usam_os_alvos_ebu_r128() {
        assert!(VOICE_BOOST_FILTER.contains("I=-16"));
        assert!(VOICE_BOOST_FILTER.contains("TP=-1.5"));
        assert!(SMART_SPEED_FILTER.contains("stop_periods=-1"));
    }
}
