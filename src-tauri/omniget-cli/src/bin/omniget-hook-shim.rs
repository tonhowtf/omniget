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
