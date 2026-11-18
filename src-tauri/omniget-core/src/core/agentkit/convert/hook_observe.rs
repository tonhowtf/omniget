use std::path::Path;

use super::hook_shim::read_token;

pub fn check_token(data_dir: &Path, tool: &str, provided: &str) -> bool {
    let Some(expected) = read_token(data_dir, tool) else {
        return false;
    };
    let (a, b) = (expected.as_bytes(), provided.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
