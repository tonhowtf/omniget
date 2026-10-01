pub fn execute(mut args: Vec<String>, json: bool) -> ! {
    if json && !args.iter().any(|a| a == "--json") {
        args.push("--json".into());
    }
    let code = omniget_core::core::canvas::client::run_cli(&args);
    std::process::exit(code)
}
