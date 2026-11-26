#![no_main]

use libfuzzer_sys::fuzz_target;
use omniget_core::core::music::icy::parse_block;

fuzz_target!(|input: &[u8]| {
    let _ = parse_block(input);
});
