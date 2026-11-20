#![no_main]

use libfuzzer_sys::fuzz_target;
use omniget_core::core::music::lrc::parse;

fuzz_target!(|input: &str| {
    let _ = parse(input);
});
