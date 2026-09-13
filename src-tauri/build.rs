fn main() {
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
    tauri_build::build()
}
