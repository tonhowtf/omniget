//! `omniget-elevate`: the only part of OmniGet that ever runs as
//! administrator. It is started by the app through the system prompt
//! (osascript on macOS, UAC on Windows, pkexec on Linux) with
//! `--plan <file> --out <file>`, reads one signed batch of typed operations
//! (washmybelly `ElevatedOp`), checks the signature, the expiry, the
//! protected paths and each fingerprint again, runs them, and writes one
//! result per operation. It accepts no command, script or free path.

fn arg(args: &[String], flag: &str) -> Option<std::path::PathBuf> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(plan), Some(out)) = (arg(&args, "--plan"), arg(&args, "--out")) else {
        eprintln!("usage: omniget-elevate --plan <signed batch> --out <results>");
        std::process::exit(2);
    };
    if let Err(e) = omniget_core::core::washmybelly::elevate::helper_main(&plan, &out) {
        eprintln!("omniget-elevate: {e}");
        std::process::exit(1);
    }
}
