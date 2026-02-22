#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn setup_environment() {
    std::env::remove_var("PYTHONHOME");
    std::env::remove_var("PYTHONPATH");

    if let Some(bin_dir) = dirs::data_dir().map(|d| d.join("omniget").join("bin")) {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let current = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}{}{}", bin_dir.display(), sep, current));
    }

    std::env::set_var("PYTHONIOENCODING", "utf-8");
    std::env::set_var("PYTHONUTF8", "1");
}

fn main() {
    setup_environment();
    omniget_lib::run()
}
