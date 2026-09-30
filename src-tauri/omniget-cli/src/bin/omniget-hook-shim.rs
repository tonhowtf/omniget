//! `omniget-hook-shim`: runs a Claude Code hook for any coding tool. It turns
//! the tool's hook stdin into Claude's hook JSON, runs the original command and
//! turns the answer back into the tool's dialect; `--observe` reports the event
//! to the OmniGet app instead. Logic lives in
//! `omniget_core::core::agentkit::convert::hook_shim`.
//!
//! Multi-call: the app also installs this binary as `<app_data>/bin/omniget`
//! for the canvas terminals. Called by that name it answers
//! `omniget canvas …` (`omniget_core::core::canvas::client`) and hands any
//! other subcommand to the next `omniget` on PATH.

fn main() {
    let called_as = std::env::args_os()
        .next()
        .map(std::path::PathBuf::from)
        .and_then(|p| {
            p.file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase())
        })
        .unwrap_or_default();
    if called_as == "omniget" {
        std::process::exit(omniget_core::core::canvas::client::multicall_main());
    }
    std::process::exit(omniget_core::core::agentkit::convert::hook_shim::main_entry());
}
