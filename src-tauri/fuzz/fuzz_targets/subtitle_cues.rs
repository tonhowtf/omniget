#![no_main]

use libfuzzer_sys::fuzz_target;
use omniget_core::core::subtitle_merge::parse_cues;

fuzz_target!(|input: &str| {
    let _ = parse_cues(input);
});
