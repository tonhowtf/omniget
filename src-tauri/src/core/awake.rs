use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Held {
    downloads: bool,
    manual: bool,
    guard: Option<(keepawake::KeepAwake, bool)>,
}

fn cell() -> &'static Mutex<Held> {
    static HELD: OnceLock<Mutex<Held>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(Held::default()))
}

fn apply(held: &mut Held) -> bool {
    let display = held.manual;
    if !held.downloads && !held.manual {
        if held.guard.take().is_some() {
            tracing::debug!("[awake] sleep prevention released");
        }
        return true;
    }
    if held.guard.as_ref().is_some_and(|(_, d)| *d == display) {
        return true;
    }
    held.guard = None;
    match keepawake::Builder::default()
        .display(display)
        .idle(true)
        .sleep(true)
        .app_name("OmniGet")
        .reason(if display {
            "Keep awake"
        } else {
            "Active downloads"
        })
        .create()
    {
        Ok(g) => {
            held.guard = Some((g, display));
            tracing::debug!("[awake] sleep prevention engaged");
            true
        }
        Err(e) => {
            tracing::warn!("[awake] could not prevent sleep: {}", e);
            false
        }
    }
}

pub fn sync(active: bool) {
    let Ok(mut held) = cell().lock() else {
        return;
    };
    let wanted = active
        && (held.downloads
            || crate::storage::config::load_settings_standalone()
                .advanced
                .prevent_sleep);
    if wanted != held.downloads {
        held.downloads = wanted;
        if !apply(&mut held) {
            held.downloads = false;
        }
    }
}

pub fn set_manual(on: bool) -> bool {
    let Ok(mut held) = cell().lock() else {
        return false;
    };
    held.manual = on;
    let ok = apply(&mut held);
    if !ok {
        held.manual = false;
        apply(&mut held);
    }
    ok
}
