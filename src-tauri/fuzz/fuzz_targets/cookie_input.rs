#![no_main]

use libfuzzer_sys::fuzz_target;
use omniget_core::core::cookie_parser::{parse_bearer_input, parse_cookie_input};

fuzz_target!(|input: &str| {
    let _ = parse_cookie_input(input, "sessionid");
    let _ = parse_bearer_input(input);
});
