export async function init() {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installBrowserPreview } = await import("$lib/dev/browser-preview");
    installBrowserPreview();
  }
}
