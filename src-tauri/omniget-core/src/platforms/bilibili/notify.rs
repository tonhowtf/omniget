use once_cell::sync::OnceCell;
use serde::Serialize;

pub type Emitter = Box<dyn Fn(&str, serde_json::Value) + Send + Sync + 'static>;

static EMITTER: OnceCell<Emitter> = OnceCell::new();

pub fn set_emitter(emitter: Emitter) {
    let _ = EMITTER.set(emitter);
}

pub fn emit(event: &str, payload: serde_json::Value) {
    if let Some(em) = EMITTER.get() {
        em(event, payload);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionExpiredPayload {
    pub slug: Option<String>,
    pub uname_hint: Option<String>,
}

pub fn session_expired(slug: Option<&str>) {
    let payload = SessionExpiredPayload {
        slug: slug.map(String::from),
        uname_hint: None,
    };
    let value = serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null);
    emit("bilibili-session-expired", value);
}
