fn arg(args: &[String], flag: &str) -> Option<std::path::PathBuf> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(plan), Some(out), Some(sha)) = (
        arg(&args, "--plan"),
        arg(&args, "--out"),
        arg(&args, "--plan-sha256"),
    ) else {
        eprintln!(
            "usage: omniget-elevate --plan <signed batch> --out <results> --plan-sha256 <digest>"
        );
        std::process::exit(2);
    };
    if let Err(e) =
        omniget_core::core::washmybelly::elevate::helper_main(&plan, &out, &sha.to_string_lossy())
    {
        eprintln!("omniget-elevate: {e}");
        std::process::exit(1);
    }
}
