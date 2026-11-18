use tauri::ipc::Invoke;
use tauri::Wry;

const PREVIEW_COMMANDS: &[&str] = &["preview_report", "debug_report"];

pub fn allowed(label: &str, command: &str) -> bool {
    match label {
        "main" | "pet" | "limits-strip" | "usage-panel" | "quick-panel" | "omni" => true,
        l if l.starts_with("preview-") => PREVIEW_COMMANDS.contains(&command),
        l if l.starts_with("portal-") => {
            command == "canvas_portal_report" || command == "debug_report"
        }
        l if l.starts_with("auth-") => command == "debug_report",
        _ => false,
    }
}

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
            invoke.resolver.reject(format!(
                "ERR_IPC_FORBIDDEN: command not allowed from `{label}`"
            ));
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
        for l in ["main", "pet", "limits-strip", "usage-panel"] {
            assert!(allowed(l, "threads_import_external"), "{l}");
        }
        assert!(allowed("preview-thr_1", "preview_report"));
        assert!(!allowed("preview-thr_1", "threads_import_external"));
        assert!(!allowed("preview-thr_1", "open_auth_webview"));
        assert!(!allowed("auth-123", "open_auth_webview"));
        assert!(!allowed("auth-123", "preview_report"));
        assert!(!allowed("somewhere-else", "threads_import_external"));
    }

    #[test]
    fn every_app_window_label_is_allowed() {
        for l in [
            crate::commands::pet::WINDOW_LABEL,
            crate::limits_strip::commands::WINDOW_LABEL,
            crate::usage_tray::PANEL_LABEL,
            crate::quick_panel::PANEL_LABEL,
            crate::canvas::companion::OMNI_WINDOW,
        ] {
            assert!(allowed(l, "usage_tray_view"), "{l}");
        }
    }
}
