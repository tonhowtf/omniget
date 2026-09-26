/// tauri-build requires every `externalBin` to exist at compile time. The
/// worker sidecar is produced by `scripts/mcp/build-worker.mjs` (the release
/// `beforeBuildCommand`); `cargo test`/`clippy` in CI never run it. An empty
/// placeholder lets those compile; a release build overwrites it first.
fn ensure_worker_sidecar() {
    let (Ok(dir), Ok(target)) = (std::env::var("CARGO_MANIFEST_DIR"), std::env::var("TARGET"))
    else {
        return;
    };
    let ext = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let path = std::path::Path::new(&dir)
        .join("binaries")
        .join(format!("omniget-worker-{target}{ext}"));
    if !path.exists() {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        if std::fs::write(&path, b"").is_ok() {
            println!(
                "cargo:warning=omniget-worker sidecar missing; wrote an empty placeholder at {} (run scripts/mcp/build-worker.mjs for a working worker)",
                path.display()
            );
        }
    }
}

fn main() {
    ensure_worker_sidecar();
    // The bundled SQLite (libsqlite3-sys "bundled") defines sqlite3_* symbols
    // in this executable. When a linked shared library (webkit2gtk and its
    // deps reference sqlite3_open_v2, sqlite3_prepare_v2, ...) has an
    // undefined reference to one of them, ld auto-exports the definition so
    // the shared lib binds to it at runtime. Those exports then preempt
    // sqlite3 usage process-wide: libsqlite3.so.0's own sqlite3_initialize
    // is among the preempted calls, so the system library's global config is
    // never initialized while some of its internal calls execute in the
    // bundled copy. Any plugin using the system libsqlite3 (e.g. the
    // Telegram plugin's SqliteSession) then segfaults on its first
    // sqlite3_mutex_enter, which tail-jumps through the never-populated
    // mutex-methods table. --exclude-libs keeps static-archive symbols
    // local: shared libs bind to the real libsqlite3.so.0 and the bundled
    // copy stays private to the executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg=-Wl,--exclude-libs,ALL");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-ObjC");
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        if let Ok(out) = std::process::Command::new("xcode-select")
            .arg("-p")
            .output()
        {
            let xcode = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !xcode.is_empty() {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{xcode}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx");
            }
        }
    }
    // On Windows/MSVC the manifest that asks for Common Controls v6 is linked
    // into every target of this package instead of going in through the app's
    // resource file. The unit-test executable has no resource file, and without
    // the manifest Windows refuses to load it (STATUS_ENTRYPOINT_NOT_FOUND on
    // `TaskDialogIndirect`, which the dialog plugin imports).
    let msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if msvc {
        let attrs = tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        tauri_build::try_build(attrs).expect("failed to run tauri-build");
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    } else {
        tauri_build::build()
    }
}
