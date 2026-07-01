use std::sync::atomic::{AtomicBool, Ordering};

static JSON_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_json_mode(enabled: bool) {
    JSON_MODE.store(enabled, Ordering::SeqCst);
}

pub fn is_json_mode() -> bool {
    JSON_MODE.load(Ordering::SeqCst)
}

pub fn print_json<T: serde::Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("JSON serialization error: {}", e),
    }
}

pub fn print_text(text: &str) {
    if !is_json_mode() {
        println!("{}", text);
    }
}
