#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn check_portable_mode() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable.txt").exists() || dir.join(".portable").exists() {
                let data_dir = dir.join("data");
                let _ = std::fs::create_dir_all(&data_dir);
                std::env::set_var("OMNIGET_PORTABLE", "1");
                std::env::set_var("OMNIGET_DATA_DIR", data_dir.to_string_lossy().to_string());
            }
        }
    }
}

fn setup_environment() {
    std::env::remove_var("PYTHONHOME");
    std::env::remove_var("PYTHONPATH");

    if let Some(bin_dir) = omniget_lib::core::paths::app_data_dir().map(|d| d.join("bin")) {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let current = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}{}{}", bin_dir.display(), sep, current));
    }

    std::env::set_var("PYTHONIOENCODING", "utf-8");
    std::env::set_var("PYTHONUTF8", "1");

    #[cfg(target_os = "linux")]
    if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

fn main() {
    if omniget_lib::native_host::should_run_as_native_host() {
        if let Err(error) = omniget_lib::native_host::run_native_host() {
            eprintln!("native host error: {error}");
        }
        return;
    }

    check_portable_mode();
    setup_environment();
    omniget_lib::run()
}
