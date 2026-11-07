pub mod draw;
pub mod render;

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use acadrust::{CadDocument, DwgReadOptions, DwgReader, DxfReader, DxfWriter};

pub const WORKER_ARG: &str = "--omniget-cad-worker";

const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_DECOMPRESSED_BYTES: usize = 1 << 30;

static WORKER_HOST: AtomicBool = AtomicBool::new(false);

pub fn worker_entry() -> Option<i32> {
    if let Some(code) = run_worker(std::env::args_os().skip(1)) {
        return Some(code);
    }
    WORKER_HOST.store(true, Ordering::Relaxed);
    None
}

fn run_worker(args: impl Iterator<Item = OsString>) -> Option<i32> {
    let args: Vec<OsString> = args.collect();
    if args.first().map(OsString::as_os_str) != Some(OsStr::new(WORKER_ARG)) {
        return None;
    }
    let [_, input, from, output, to] = args.as_slice() else {
        return Some(2);
    };
    let (Some(from), Some(to)) = (from.to_str(), to.to_str()) else {
        return Some(2);
    };
    match convert_in_process(Path::new(input), from, Path::new(output), to) {
        Ok(()) => Some(0),
        Err(e) => {
            eprint!("{e}");
            Some(1)
        }
    }
}

pub fn read(path: &Path, from: &str) -> Result<CadDocument, String> {
    let bad = |e: acadrust::DxfError| format!("This drawing could not be read: {e}.");
    let doc = match from {
        "dwg" => {
            let options =
                DwgReadOptions::failsafe().with_max_decompressed_bytes(MAX_DECOMPRESSED_BYTES);
            DwgReader::from_file_with_options(path, options)
                .and_then(|mut r| r.read())
                .map_err(bad)
        }
        "dxf" => DxfReader::from_file(path)
            .and_then(|r| r.read())
            .map_err(bad),
        other => Err(format!("OmniGet can't read {other} drawings.")),
    }?;
    if doc.entity_count() == 0 {
        return Err("This drawing is empty or could not be read.".into());
    }
    Ok(doc)
}

pub fn convert_in_process(input: &Path, from: &str, output: &Path, to: &str) -> Result<(), String> {
    let doc = read(input, from)?;
    let bytes = match to {
        "dxf" => DxfWriter::new(&doc)
            .write_to_vec()
            .map_err(|e| format!("Could not write the DXF file: {e}."))?,
        "svg" => render::to_svg(&draw::collect(&doc)?)?.into_bytes(),
        "pdf" => render::to_pdf(&draw::collect(&doc)?)?,
        other => return Err(format!("OmniGet can't turn drawings into {other}.")),
    };
    std::fs::write(output, bytes).map_err(|e| format!("Could not write the output: {e}."))
}

fn too_slow() -> String {
    format!(
        "This drawing took more than {} seconds to read, so OmniGet stopped. The file may be damaged or too complex.",
        TIMEOUT.as_secs()
    )
}

pub fn convert_file(input: &Path, from: &str, output: &Path, to: &str) -> Result<(), String> {
    if WORKER_HOST.load(Ordering::Relaxed) {
        if let Ok(exe) = std::env::current_exe() {
            return in_worker(&exe, input, from, output, to, TIMEOUT);
        }
    }
    in_thread(input, from, output, to, TIMEOUT)
}

fn in_thread(
    input: &Path,
    from: &str,
    output: &Path,
    to: &str,
    timeout: Duration,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (input, from, output, to) = (
        input.to_path_buf(),
        from.to_string(),
        output.to_path_buf(),
        to.to_string(),
    );
    std::thread::Builder::new()
        .name("omniget-cad".into())
        .spawn(move || {
            let result =
                std::panic::catch_unwind(|| convert_in_process(&input, &from, &output, &to))
                    .unwrap_or_else(|_| {
                        Err(
                            "This drawing could not be read: the reader stopped unexpectedly."
                                .into(),
                        )
                    });
            let _ = tx.send(result);
        })
        .map_err(|e| format!("Could not start the drawing reader: {e}."))?;
    rx.recv_timeout(timeout).unwrap_or_else(|_| Err(too_slow()))
}

fn in_worker(
    exe: &Path,
    input: &Path,
    from: &str,
    output: &Path,
    to: &str,
    timeout: Duration,
) -> Result<(), String> {
    let mut child = crate::core::process::std_command(exe)
        .arg(WORKER_ARG)
        .arg(input)
        .arg(from)
        .arg(output)
        .arg(to)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start the drawing reader: {e}."))?;
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(err) = stderr.as_mut() {
            let _ = err.take(64 * 1024).read_to_string(&mut text);
        }
        text
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(too_slow());
            }
        }
    };
    let message = reader.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else if message.trim().is_empty() {
        Err("This drawing could not be read: the reader stopped unexpectedly.".into())
    } else {
        Err(message.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::entities::{Arc, EntityType, Line, Text};
    use acadrust::types::Vector3;

    fn sample_doc() -> CadDocument {
        let mut doc = CadDocument::new();
        let v = |x: f64, y: f64| Vector3 { x, y, z: 0.0 };
        let line = Line {
            start: v(0.0, 0.0),
            end: v(200.0, 100.0),
            ..Line::default()
        };
        doc.add_entity(EntityType::Line(line)).unwrap();
        let mut arc = Arc::new();
        arc.center = v(100.0, 50.0);
        arc.radius = 25.0;
        arc.end_angle = std::f64::consts::PI;
        doc.add_entity(EntityType::Arc(arc)).unwrap();
        let mut text = Text::new();
        text.value = "Kitchen".into();
        text.insertion_point = v(20.0, 80.0);
        text.height = 6.0;
        doc.add_entity(EntityType::Text(text)).unwrap();
        doc
    }

    fn sample_dxf(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("plan.dxf");
        DxfWriter::new(&sample_doc()).write_to_file(&path).unwrap();
        path
    }

    #[test]
    fn dxf_becomes_svg_and_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let dxf = sample_dxf(dir.path());
        let svg = dir.path().join("plan.svg");
        convert_file(&dxf, "dxf", &svg, "svg").unwrap();
        let text = std::fs::read_to_string(&svg).unwrap();
        assert!(text.contains(">Kitchen</text>"));
        assert_eq!(text.matches("<path").count(), 1);
        let png = super::super::svg::rasterize(text.as_bytes())
            .unwrap()
            .to_rgba8();
        assert_eq!(png.width(), 2400);
        assert!(png.pixels().any(|p| p[0] < 50 && p[1] < 50 && p[2] < 50));
        let pdf = dir.path().join("plan.pdf");
        convert_file(&dxf, "dxf", &pdf, "pdf").unwrap();
        assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn dwg_becomes_dxf_and_svg() {
        let dir = tempfile::tempdir().unwrap();
        let dwg = dir.path().join("plan.dwg");
        acadrust::DwgWriter::write_to_file(&dwg, &sample_doc()).unwrap();
        let dxf = dir.path().join("plan.dxf");
        convert_file(&dwg, "dwg", &dxf, "dxf").unwrap();
        let back = read(&dxf, "dxf").unwrap();
        assert_eq!(back.model_space_entities().count(), 3);
        let svg = dir.path().join("plan.svg");
        convert_file(&dwg, "dwg", &svg, "svg").unwrap();
        assert!(std::fs::read_to_string(&svg).unwrap().contains("Kitchen"));
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn worker_process_reports_results_errors_and_hangs() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output) = (dir.path().join("in.dwg"), dir.path().join("out.dxf"));
        let ok = script(
            dir.path(),
            "ok",
            r#"[ "$1" = "--omniget-cad-worker" ] && echo done > "$4""#,
        );
        in_worker(&ok, &input, "dwg", &output, "dxf", Duration::from_secs(10)).unwrap();
        assert!(output.exists());
        let fails = script(
            dir.path(),
            "fails",
            "echo 'This drawing could not be read: bad header.' >&2; exit 1",
        );
        let err = in_worker(
            &fails,
            &input,
            "dwg",
            &output,
            "dxf",
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert_eq!(err, "This drawing could not be read: bad header.");
        let hangs = script(dir.path(), "hangs", "sleep 30");
        let started = Instant::now();
        let err = in_worker(
            &hangs,
            &input,
            "dwg",
            &output,
            "dxf",
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(
            err.contains("seconds") && started.elapsed() < Duration::from_secs(5),
            "{err}"
        );
    }

    #[test]
    fn worker_arguments_run_the_conversion() {
        let dir = tempfile::tempdir().unwrap();
        let dxf = sample_dxf(dir.path());
        let out = dir.path().join("copy.svg");
        let args = [
            WORKER_ARG.into(),
            dxf.into_os_string(),
            "dxf".into(),
            out.clone().into_os_string(),
            "svg".into(),
        ];
        assert_eq!(run_worker(args.into_iter()), Some(0));
        assert!(out.exists());
        assert_eq!(run_worker(["--other".into()].into_iter()), None);
        assert_eq!(run_worker([WORKER_ARG.into()].into_iter()), Some(2));
    }

    #[test]
    fn slow_readers_time_out() {
        let dir = tempfile::tempdir().unwrap();
        let dxf = sample_dxf(dir.path());
        let err = in_thread(
            &dxf,
            "dxf",
            &dir.path().join("x.svg"),
            "svg",
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(err.contains("seconds"), "{err}");
    }

    #[test]
    fn damaged_drawings_are_explained() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.dwg");
        std::fs::write(&bad, b"AC1032 not really a drawing").unwrap();
        let err = convert_file(&bad, "dwg", &dir.path().join("out.dxf"), "dxf").unwrap_err();
        assert!(err.contains("drawing"), "{err}");
    }
}
