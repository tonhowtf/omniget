//! `omniget canvas …`: the cords of the OmniGet canvas, for an agent running
//! in a canvas terminal. Thin: parsing, HTTP and output live in
//! `omniget_core::core::canvas::client`, shared with the multi-call
//! `omniget` helper the app installs in `<app_data>/bin/`.

/// Runs the subcommand and exits with its code (0 ok, 2 usage or not
/// allowed, 3 not finished yet, 4 released to avoid a deadlock, 1 error).
pub fn execute(mut args: Vec<String>, json: bool) -> ! {
    if json && !args.iter().any(|a| a == "--json") {
        args.push("--json".into());
    }
    let code = omniget_core::core::canvas::client::run_cli(&args);
    std::process::exit(code)
}
