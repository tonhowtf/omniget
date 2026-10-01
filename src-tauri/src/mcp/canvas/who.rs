use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, OnceLock};

use omniget_core::core::canvas::store::agent_origin;

use super::super::policy::Principal;

const ORIGIN_MAX: usize = 128;
const CLIENT_MAX: usize = 80;

#[derive(Clone, Debug)]
pub struct Who {
    pub origin: String,
    pub client: String,
    pub scopes: Vec<String>,
}

tokio::task_local! {
    static WHO: Who;
}

thread_local! {
    static WHO_SYNC: RefCell<Option<Who>> = const { RefCell::new(None) };
}

fn client_names() -> &'static Mutex<HashMap<String, String>> {
    static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

pub fn remember_client(principal_id: &str, name: &str) {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(CLIENT_MAX)
        .collect();
    let name = name.trim();
    if principal_id.is_empty() || name.is_empty() {
        return;
    }
    let mut map = client_names().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() > 256 && !map.contains_key(principal_id) {
        map.clear();
    }
    map.insert(principal_id.to_string(), name.to_string());
}

fn client_name(principal_id: &str) -> Option<String> {
    client_names()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(principal_id)
        .cloned()
}

pub fn slug(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in raw.trim().chars() {
        if c.is_alphanumeric() || matches!(c, '.' | '_') {
            out.push(c);
            dash = false;
        } else if (c.is_whitespace() || c == '-' || c == ':' || c == '/')
            && !dash
            && !out.is_empty()
        {
            out.push('-');
            dash = true;
        }
        if out.len() >= ORIGIN_MAX - 8 {
            break;
        }
    }
    out.trim_end_matches('-').to_string()
}

pub fn of(p: &Principal) -> Who {
    if let Some(node) = p.id.strip_prefix("canvas:") {
        let mut origin = agent_origin(node);
        origin.truncate(ORIGIN_MAX);
        return Who {
            origin,
            client: p.name.clone(),
            scopes: p.scopes.clone(),
        };
    }
    let display = match client_name(&p.id) {
        Some(c) if !p.name.trim().is_empty() && !c.eq_ignore_ascii_case(p.name.trim()) => {
            format!("{c} · {}", p.name.trim())
        }
        Some(c) => c,
        None => p.name.clone(),
    };
    let mut label = slug(&p.name);
    if label.is_empty() {
        label = slug(&display);
    }
    if label.is_empty() {
        label = p.id.chars().take(8).collect();
    }
    let mut origin = format!("mcp:{label}");
    while origin.len() > ORIGIN_MAX {
        origin.pop();
    }
    Who {
        origin,
        client: if display.trim().is_empty() {
            label
        } else {
            display
        },
        scopes: p.scopes.clone(),
    }
}

pub fn current() -> Option<Who> {
    WHO.try_with(Clone::clone)
        .ok()
        .or_else(|| WHO_SYNC.with(|c| c.borrow().clone()))
}

pub fn origin() -> String {
    current()
        .map(|w| w.origin)
        .unwrap_or_else(|| super::ORIGIN.to_string())
}

pub async fn scope<F: Future>(who: Who, f: F) -> F::Output {
    WHO.scope(who, f).await
}

pub fn enter<R>(who: Option<Who>, f: impl FnOnce() -> R) -> R {
    WHO_SYNC.with(|c| *c.borrow_mut() = who);
    let out = f();
    WHO_SYNC.with(|c| *c.borrow_mut() = None);
    out
}
