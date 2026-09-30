// Builds the vendored meshoptimizer 1.3 (MIT, vendor/meshoptimizer) into a
// static library. Bindings are hand-written in src/meshopt.rs.
fn main() {
    let dir = std::path::Path::new("vendor/meshoptimizer/src");
    let mut build = cc::Build::new();
    build.cpp(true).warnings(false).opt_level(3);
    // No fused multiply-add contraction: clang contracts by default on ARM and
    // the simplifier's choices then differ from x86, so the same kit would
    // hash differently per machine.
    build.flag_if_supported("-ffp-contract=off");
    for entry in std::fs::read_dir(dir).expect("vendor/meshoptimizer/src") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("cpp") {
            println!("cargo:rerun-if-changed={}", path.display());
            build.file(path);
        }
    }
    println!("cargo:rerun-if-changed=vendor/meshoptimizer/src/meshoptimizer.h");
    build.compile("meshoptimizer13");
}
