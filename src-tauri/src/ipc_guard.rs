//! Which webview may call which app command.
//!
//! Tauri injects `__TAURI_INTERNALS__` into every webview, remote pages
//! included, and the app has no ACL manifest for its own commands, so without
//! this guard a page loaded in a thread's Browser preview (`preview-*`) or in a
//! login window (`auth-*`) could call any app command. Plugin commands are
//! still gated by the capabilities in `capabilities/*.json`; this covers the
//! app half.

use tauri::ipc::Invoke;
use tauri::Wry;

/// Commands a remote preview page may call: the report channel of the script
/// the preview injects, and the test driver's report (inert unless the process
/// runs with `OMNIGET_TEST_DRIVER=1`).
const PREVIEW_COMMANDS: &[&str] = &["preview_report", "debug_report"];

/// True when the webview `label` may call the app command `command`.
pub fn allowed(label: &str, command: &str) -> bool {
    match label {
        "main" | "pet" | "limits-strip" => true,
        l if l.starts_with("omnidisc-stream-") => true,
        l if l.starts_with("preview-") => PREVIEW_COMMANDS.contains(&command),
        // Login windows load third-party sites; the app reads their cookies
        // and navigation from Rust, it never needs a call from the page.
        l if l.starts_with("auth-") => command == "debug_report",
        _ => false,
    }
}

/// Wraps the generated invoke handler with [`allowed`].
pub fn guard<F>(handler: F) -> impl Fn(Invoke<Wry>) -> bool + Send + Sync + 'static
where
    F: Fn(Invoke<Wry>) -> bool + Send + Sync + 'static,
{
    move |invoke: Invoke<Wry>| {
        let label = invoke.message.webview_ref().label().to_string();
        if !allowed(&label, invoke.message.command()) {
            tracing::warn!(
                "[ipc_guard] blocked `{}` from webview `{}`",
                invoke.message.command(),
                label
            );
            invoke
                .resolver
                .reject(format!("ERR_IPC_FORBIDDEN: command not allowed from `{label}`"));
            return true;
        }
        handler(invoke)
    }
}

#[cfg(test)]
mod tests {
    use super::allowed;

    #[test]
    fn app_windows_call_everything_remote_pages_almost_nothing() {
        for l in ["main", "pet", "limits-strip", "omnidisc-stream-42"] {
            assert!(allowed(l, "threads_dispatch"), "{l}");
        }
        assert!(allowed("preview-thr_1", "preview_report"));
        assert!(!allowed("preview-thr_1", "threads_dispatch"));
        assert!(!allowed("preview-thr_1", "open_auth_webview"));
        assert!(!allowed("auth-123", "open_auth_webview"));
        assert!(!allowed("auth-123", "preview_report"));
        assert!(!allowed("somewhere-else", "threads_dispatch"));
    }
}
