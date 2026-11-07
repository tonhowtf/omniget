pub mod audio;
pub mod models;
pub mod separate;
pub mod sets;
pub mod spectral;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};

use audio::{FfmpegSink, FfmpegSource, StemFormat, StemTags};
use separate::{Control, PcmSink, Separator};

pub struct Output {
    pub path: PathBuf,
    pub tags: StemTags,
}

pub fn part_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    path.with_file_name(format!("{stem}.part.{ext}"))
}

pub fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() && !part_path(&path).exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for i in 2..1000 {
        let p = dir.join(format!("{stem} ({i}).{ext}"));
        if !p.exists() && !part_path(&p).exists() {
            return p;
        }
    }
    path
}

pub fn output_path(
    source: &Path,
    out_dir: Option<&Path>,
    label: &str,
    format: StemFormat,
) -> PathBuf {
    let dir = out_dir
        .map(Path::to_path_buf)
        .or_else(|| source.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "track".into());
    let name = crate::core::filename::sanitize_path_component(&format!("{stem} ({label})"));
    unique(dir.join(format!("{name}.{}", format.extension())))
}

pub fn separate_file(
    input: &Path,
    sep: &mut dyn Separator,
    outputs: &[Output],
    format: StemFormat,
    ctl: &Control<'_>,
) -> anyhow::Result<u64> {
    if outputs.len() != sep.stems().len() {
        return Err(anyhow!(
            "{} outputs for {} stems",
            outputs.len(),
            sep.stems().len()
        ));
    }
    let parts: Vec<PathBuf> = outputs.iter().map(|o| part_path(&o.path)).collect();
    let cleanup = || {
        for p in &parts {
            let _ = std::fs::remove_file(p);
        }
    };
    let result = (|| -> anyhow::Result<u64> {
        for p in &parts {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("creating {}", dir.display()))?;
            }
        }
        let mut src = FfmpegSource::open(input)?;
        let mut sinks = outputs
            .iter()
            .zip(&parts)
            .map(|(o, p)| FfmpegSink::create(p, format, &o.tags))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let samples = {
            let mut refs: Vec<&mut dyn PcmSink> =
                sinks.iter_mut().map(|s| s as &mut dyn PcmSink).collect();
            separate::run(&mut src, sep, &mut refs, ctl)?
        };
        drop(src);
        for s in sinks {
            s.finish()?;
        }
        for (i, (o, p)) in outputs.iter().zip(&parts).enumerate() {
            if let Err(e) = std::fs::rename(p, &o.path) {
                for done in &outputs[..i] {
                    let _ = std::fs::remove_file(&done.path);
                }
                return Err(anyhow!(e).context(format!("moving to {}", o.path.display())));
            }
        }
        Ok(samples)
    })();
    if result.is_err() {
        cleanup();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::separate::{PcmSource, CHANNELS, MODEL_BINS, SAMPLE_RATE};
    use super::*;

    fn has_ffmpeg() -> bool {
        crate::core::process::std_command("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn synth(frames: usize) -> Vec<f32> {
        let mut seed = 0x9e37_79b9u32;
        (0..frames)
            .flat_map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.05;
                let t = i as f32 / SAMPLE_RATE as f32;
                let voice = 0.4 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
                let bass = 0.3 * (2.0 * std::f32::consts::PI * 80.0 * t).sin();
                [voice + bass + noise, voice - bass + noise]
            })
            .collect()
    }

    fn write_input(path: &Path, data: &[f32]) {
        let mut sink = FfmpegSink::create(path, StemFormat::Wav, &StemTags::default()).unwrap();
        sink.write(data).unwrap();
        sink.finish().unwrap();
    }

    fn decode(path: &Path) -> Vec<f32> {
        let mut src = FfmpegSource::open(path).unwrap();
        let mut all = Vec::new();
        let mut buf = vec![0.0f32; 1 << 15];
        loop {
            let n = src.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
        }
        all
    }

    fn probe(path: &Path) -> String {
        let out = crate::core::process::std_command("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,sample_rate,channels",
                "-of",
                "csv=p=0",
            ])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    struct Band;

    impl Separator for Band {
        fn stems(&self) -> Vec<String> {
            vec!["vocals".into(), "instrumental".into()]
        }

        fn estimate(&mut self, mags: &[f32]) -> anyhow::Result<Vec<Vec<f32>>> {
            let voice: Vec<f32> = mags
                .iter()
                .enumerate()
                .map(|(i, m)| if i % MODEL_BINS > 20 { *m } else { 0.0 })
                .collect();
            let rest: Vec<f32> = mags.iter().zip(&voice).map(|(m, v)| m - v).collect();
            Ok(vec![voice, rest])
        }
    }

    fn outputs(dir: &Path, ext: &str) -> Vec<Output> {
        ["Song (Vocals)", "Song (Instrumental)"]
            .iter()
            .map(|n| Output {
                path: dir.join(format!("{n}.{ext}")),
                tags: StemTags {
                    title: n.to_string(),
                    artist: "Test".into(),
                    ..Default::default()
                },
            })
            .collect()
    }

    #[test]
    fn output_names_sit_next_to_the_source_and_never_clash() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("My Song.mp3");
        let first = output_path(&src, None, "Instrumental", StemFormat::Flac);
        assert_eq!(first, dir.path().join("My Song (Instrumental).flac"));
        std::fs::write(&first, b"x").unwrap();
        let second = output_path(&src, None, "Instrumental", StemFormat::Flac);
        assert_eq!(second, dir.path().join("My Song (Instrumental) (2).flac"));
        let other = dir.path().join("stems");
        let third = output_path(&src, Some(&other), "Voz/Vocal", StemFormat::Mp3);
        assert_eq!(third.parent(), Some(other.as_path()));
        assert!(!third.file_name().unwrap().to_string_lossy().contains('/'));
    }

    #[test]
    fn separates_a_real_file_end_to_end() {
        if !has_ffmpeg() {
            eprintln!("ffmpeg not found, skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("Canção do mês.wav");
        let frames = SAMPLE_RATE as usize * 12 + 123;
        write_input(&input, &synth(frames));

        let outs = outputs(dir.path(), "flac");
        let ctl = Control {
            cancelled: &|| false,
            progress: &|_| {},
        };
        let n = separate_file(&input, &mut Band, &outs, StemFormat::Flac, &ctl).unwrap();
        assert_eq!(n as usize, frames);
        for o in &outs {
            assert!(o.path.is_file(), "{}", o.path.display());
            assert!(!part_path(&o.path).exists());
            assert_eq!(probe(&o.path), "flac,44100,2");
            assert_eq!(decode(&o.path).len(), frames * CHANNELS);
        }
        let (meta, _) = crate::core::music::tags::read(&outs[1].path).unwrap();
        assert_eq!(meta.title, "Song (Instrumental)");
        assert_eq!(meta.artist, "Test");
    }

    #[test]
    fn mp3_output_and_video_like_input() {
        if !has_ffmpeg() {
            eprintln!("ffmpeg not found, skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("mix.wav");
        write_input(&wav, &synth(SAMPLE_RATE as usize * 3));
        let mkv = dir.path().join("clip.mkv");
        let ok = crate::core::process::std_command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", "color=c=black:s=64x64:r=10:d=3"])
            .arg("-i")
            .arg(&wav)
            .args([
                "-c:v",
                "mpeg4",
                "-c:a",
                "pcm_s16le",
                "-ar",
                "48000",
                "-shortest",
            ])
            .arg(&mkv)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "could not build the test video");

        let outs = outputs(dir.path(), "mp3");
        let ctl = Control {
            cancelled: &|| false,
            progress: &|_| {},
        };
        separate_file(&mkv, &mut Band, &outs, StemFormat::Mp3, &ctl).unwrap();
        for o in &outs {
            assert_eq!(probe(&o.path), "mp3,44100,2");
            let got = decode(&o.path).len() / CHANNELS;
            let want = SAMPLE_RATE as usize * 3;
            assert!(got.abs_diff(want) < 4096, "{got} vs {want}");
        }
    }

    #[test]
    fn a_cancelled_run_leaves_no_files() {
        if !has_ffmpeg() {
            eprintln!("ffmpeg not found, skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("mix.wav");
        write_input(&input, &synth(SAMPLE_RATE as usize * 20));
        let outs = outputs(dir.path(), "wav");
        let ctl = Control {
            cancelled: &|| true,
            progress: &|_| {},
        };
        let err = separate_file(&input, &mut Band, &outs, StemFormat::Wav, &ctl).unwrap_err();
        assert!(err.downcast_ref::<separate::Cancelled>().is_some());
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, vec!["mix.wav".to_string()]);
    }

    #[test]
    fn unreadable_input_is_reported() {
        if !has_ffmpeg() {
            eprintln!("ffmpeg not found, skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("broken.mp3");
        std::fs::write(&input, b"not audio at all").unwrap();
        let outs = outputs(dir.path(), "flac");
        let ctl = Control {
            cancelled: &|| false,
            progress: &|_| {},
        };
        assert!(separate_file(&input, &mut Band, &outs, StemFormat::Flac, &ctl).is_err());
        assert!(!outs[0].path.exists());
    }

    #[test]
    fn spleeter_model_runs_when_available() {
        let Some(dir) = std::env::var_os("OMNIGET_STEMS_TEST_MODEL_DIR").map(PathBuf::from) else {
            eprintln!("OMNIGET_STEMS_TEST_MODEL_DIR not set, skipping");
            return;
        };
        if !has_ffmpeg() || !crate::core::onnxrt::is_installed() {
            eprintln!("ffmpeg or ONNX Runtime missing, skipping");
            return;
        }
        let m = models::pick(None).unwrap();
        let files: Vec<(String, PathBuf)> = m
            .files
            .iter()
            .map(|f| (f.stem.to_string(), dir.join(f.local)))
            .collect();
        let mut sep = separate::OrtSeparator::from_files(&files).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let input = match std::env::var_os("OMNIGET_STEMS_TEST_INPUT") {
            Some(p) => PathBuf::from(p),
            None => {
                let p = tmp.path().join("mix.wav");
                write_input(&p, &synth(SAMPLE_RATE as usize * 30));
                p
            }
        };
        let outs = outputs(tmp.path(), "flac");
        let ctl = Control {
            cancelled: &|| false,
            progress: &|_| {},
        };
        let started = std::time::Instant::now();
        let n = separate_file(&input, &mut sep, &outs, StemFormat::Flac, &ctl).unwrap();
        let secs = started.elapsed().as_secs_f64();
        let audio = n as f64 / SAMPLE_RATE as f64;
        eprintln!(
            "spleeter: {audio:.1}s of audio in {secs:.1}s (x{:.2} real time)",
            audio / secs
        );
        let mix = decode(&input);
        let vocals = decode(&outs[0].path);
        let inst = decode(&outs[1].path);
        assert_eq!(vocals.len() as u64, n * CHANNELS as u64);
        assert_eq!(inst.len(), vocals.len());
        let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
        let rest: Vec<f32> = mix
            .iter()
            .zip(vocals.iter().zip(&inst))
            .map(|(m, (v, i))| m - v - i)
            .collect();
        eprintln!(
            "rms mix {:.4} vocals {:.4} instrumental {:.4} leftover {:.5}",
            rms(&mix),
            rms(&vocals),
            rms(&inst),
            rms(&rest)
        );
        assert!(rms(&rest) < 2e-3, "the stems must add back up to the mix");
    }
}
