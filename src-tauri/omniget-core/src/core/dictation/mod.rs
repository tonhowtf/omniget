pub mod capture;
pub mod devices;
pub mod paste;
pub mod shortcut;
pub mod store;
pub mod text;

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore]
    async fn speech_round_trip_with_local_whisper() {
        let dir = tempfile::tempdir().unwrap();
        let aiff = dir.path().join("say.aiff");
        let ok = std::process::Command::new("say")
            .args(["-o"])
            .arg(&aiff)
            .arg("um, open omniget and download the video.")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let model = Some("whisper-tiny");
        if !crate::core::asr::is_ready(model) {
            if std::env::var_os("DICTATION_E2E_INSTALL").is_none() {
                return;
            }
            crate::core::asr::install(model, &crate::core::tools::noop_progress())
                .await
                .unwrap();
        }
        let args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-i"]
            .iter()
            .map(|s| s.to_string())
            .chain([aiff.to_string_lossy().to_string()])
            .chain(
                [
                    "-ac",
                    "1",
                    "-ar",
                    "16000",
                    "-acodec",
                    "pcm_s16le",
                    "-f",
                    "s16le",
                    "pipe:1",
                ]
                .iter()
                .map(|s| s.to_string()),
            )
            .collect();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let cap = super::capture::Capture::spawn(
            std::path::Path::new("ffmpeg"),
            &args,
            std::time::Duration::from_secs(30),
            |_| {},
            move || {
                let _ = tx.send(());
            },
        )
        .unwrap();
        rx.await.unwrap();
        let rec = cap.stop(std::time::Duration::ZERO).await.unwrap();
        let wav = super::capture::write_wav(dir.path(), "take.wav", &rec.samples).unwrap();
        let ms = super::capture::duration_ms(rec.samples.len());
        let t = crate::core::asr::transcribe_input(
            wav.as_os_str(),
            Some(ms),
            Some("en"),
            model,
            |_| {},
        )
        .await
        .unwrap();
        let rules = [super::text::Replacement {
            find: "omniget".into(),
            replace: "OmniGet".into(),
        }];
        let out = super::text::postprocess(
            &t.text,
            &super::text::PostProcess {
                language: "en",
                capitalize: true,
                remove_fillers: true,
                replacements: &rules,
            },
        );
        println!("raw={:?} out={:?}", t.text, out);
        assert!(out.to_lowercase().contains("download"));
    }
}
