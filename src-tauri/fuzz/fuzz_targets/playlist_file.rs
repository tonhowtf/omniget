#![no_main]

use libfuzzer_sys::fuzz_target;
use omniget_core::core::music::playlist_file::parse_any;

fuzz_target!(|input: &str| {
    for name in ["list.m3u", "list.m3u8", "list.pls", "list"] {
        let _ = parse_any(input, name, None);
    }
});
