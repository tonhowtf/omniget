//! `omniget-hook-shim`: runs a Claude Code hook for any coding tool. It turns
//! the tool's hook stdin into Claude's hook JSON, runs the original command and
//! turns the answer back into the tool's dialect; `--observe` reports the event
//! to the OmniGet app instead. Logic lives in
//! `omniget_core::core::agentkit::convert::hook_shim`.

fn main() {
    std::process::exit(omniget_core::core::agentkit::convert::hook_shim::main_entry());
}
