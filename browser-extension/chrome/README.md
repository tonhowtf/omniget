# OmniGet Chrome Extension

MV3 extension for sending supported media pages to OmniGet.

## Load it locally

1. Install OmniGet and launch it once on Windows, macOS, or Linux so it can register the Chrome native host for your user profile.
2. Open `chrome://extensions`.
3. Enable `Developer mode`.
4. Click `Load unpacked`.
5. Select this folder: `browser-extension/chrome`

On macOS and Linux, that first launch writes Chrome's native messaging manifest into your user profile automatically.

The unpacked extension keeps a stable ID through the committed manifest key:

`dkjelkhaaakffpghdfalobccaaipajip`

## Chrome Web Store packaging

The committed `manifest.json` intentionally keeps its `key` so local unpacked installs keep the same development ID.

Before submitting a package to the Chrome Web Store, remove the `key` field from the submitted copy of `manifest.json`. The Chrome Web Store will assign its own extension ID.

Once the store ID is known, add it to the Chrome native host allowlist in [`src-tauri/src/native_host.rs`](../../src-tauri/src/native_host.rs) so the published extension can talk to OmniGet alongside the unpacked development build.

## What it does

- Colors the toolbar icon on supported media pages only.
- Keeps the icon gray and disabled on unsupported pages.
- Sends the current page URL to OmniGet through Chrome Native Messaging.
- Opens a local error page when OmniGet is missing or could not be launched.

## Quick test

```powershell
node --test browser-extension/chrome/tests/action-title.test.mjs browser-extension/chrome/tests/action-click.test.mjs browser-extension/chrome/tests/action-feedback.test.mjs browser-extension/chrome/tests/detect.test.mjs browser-extension/chrome/tests/error-content.test.mjs browser-extension/chrome/tests/manifest.test.mjs
```
