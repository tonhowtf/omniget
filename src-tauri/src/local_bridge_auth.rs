use axum::http::HeaderMap;

pub fn check_bearer(headers: &HeaderMap, expected: &str) -> bool {
    let Some(raw) = headers.get("authorization").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some(provided) = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
    else {
        return false;
    };
    constant_time_eq::constant_time_eq(provided.trim().as_bytes(), expected.as_bytes())
}
