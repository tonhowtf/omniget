# Architecture

A map of the OmniGet codebase for someone about to change it. It says where things live and follows the three paths most contributions touch: a Tauri command, an MCP tool and a download. Setup and rules are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Contents

- [The shape of the app](#the-shape-of-the-app)
- [Crates](#crates)
- [Where things live](#where-things-live)
- [A Tauri command](#a-tauri-command)
- [The MCP server](#the-mcp-server)
- [A download, end to end](#a-download-end-to-end)
- [Managed tools](#managed-tools)
- [The frontend](#the-frontend)
- [The browser extension](#the-browser-extension)
- [Data on disk](#data-on-disk)
- [Tests](#tests)

## The shape of the app

OmniGet is a [Tauri 2](https://tauri.app) desktop app: a Rust backend and a SvelteKit (Svelte 5) frontend in a webview. The main product is a local MCP server that gives coding agents the tools of the app; the downloader comes second.

```
 coding agent ──HTTP, 127.0.0.1──┐
 browser extension ──────────────┤
                                 ▼
 frontend (src/) ──invoke──▶ Tauri app (src-tauri/src/) ──▶ omniget-core
        ▲                        │   commands, MCP, queue      extractors, yt-dlp,
        └──────── events ────────┘                             FFmpeg, tools
```

One rule shapes the layers: every feature is a function in the core first. A Tauri command and an MCP tool are two thin doors to that same function.

| Layer | Where | What goes there |
|---|---|---|
| Domain and service | `src-tauri/omniget-core/src/core/<area>/` | Pure logic, no Tauri |
| Repository | The same module, behind its own functions | Disk, database and network |
| Controllers | `src-tauri/src/commands/<area>.rs` and `src-tauri/src/mcp/` | Validate, then call the service |
| Settings | `src-tauri/omniget-core/src/models/settings.rs` | |

## Crates

The Cargo workspace is `src-tauri/Cargo.toml`.

| Crate | What it is | Binaries |
|---|---|---|
| `src-tauri/` (`omniget`) | The Tauri app | `omniget` |
| `omniget-core` | Shared library: platform extractors, yt-dlp, FFmpeg, tools, LLM and MCP code | none |
| `omniget-cli` | The command line and the helper processes | `omniget-cli`, `omniget-mcp` (stdio adapter), `omniget-worker`, `omniget-gateway`, `omniget-elevate`, `omniget-hook-shim` |
| `omniget-phonechat` | Phone chat library | none |
| `omniworld-server` | Relay for Canvas visits, self-hosted | `omniworld-server` |
| `omniget-atlas` | Build tooling for sprite atlases; the app does not depend on it | `atlas-pack` |

## Where things live

| Path | What it is |
|---|---|
| `src/routes/` | Screens. `/` is the home and the downloader |
| `src/lib/` | Stores, i18n, and one folder per feature |
| `src/components/` | Shared UI components |
| `src-tauri/src/lib.rs` | `run()`: builds the platform registry, the app state and the Tauri builder |
| `src-tauri/src/handlers.rs` | The list of every Tauri command |
| `src-tauri/src/commands/` | Tauri commands, one file or folder per feature |
| `src-tauri/src/core/` | App-side code around the core: the queue, URL resolution, the download journal, recovery |
| `src-tauri/src/mcp.rs`, `mcp/` | The MCP server |
| `src-tauri/src/local_bridge*.rs` | The local HTTP server and its routes |
| `src-tauri/omniget-core/src/platforms/` | One extractor per site, plus `generic_ytdlp` and `direct_file` |
| `src-tauri/omniget-core/src/core/` | The core by area: `ytdlp/`, `dependencies.rs`, `ffmpeg.rs`, `tools/`, `llm/`, `mcp/`, `global_dl/` and the feature areas |
| `src-tauri/capabilities/` | Tauri plugin and event permissions per window |
| `browser-extension/chrome/` | The extension. `firefox/` is generated from it |
| `claude-plugin/` | The Claude Code plugin |
| `flatpak/` | Flatpak manifest and AppStream metadata |
| `scripts/` | Checks, version bump, release, dev loop |

## A Tauri command

1. Write a `#[tauri::command]` function under `src-tauri/src/commands/`. `check_cookie_error` in `commands/downloads.rs` is a small one to read.
2. Add its path to `tauri::generate_handler![...]` in `src-tauri/src/handlers.rs`.
3. Call it from the frontend with `invoke("name", args)`, in the same pull request.

Every command passes through `src-tauri/src/ipc_guard.rs`. The app's own windows may call any command; preview, portal and login webviews, which show pages from the internet, only get a short list.

`pnpm check:ipc` compares every `invoke` and `listen` name in the frontend with what Rust registers and emits. Exceptions are in `scripts/ipc-allowlist.json`.

## The MCP server

**Transport.** MCP over HTTP at `POST /mcp`, served by the local bridge (`src-tauri/src/local_bridge.rs`) on `127.0.0.1`. It takes the first free port from 47720 to 47729; a debug build on the dev profile uses 47760 to 47769. A request with an `Origin` header is refused, and the `Host` has to be loopback, so a web page cannot reach it.

**Auth.** Each client has its own token (`src-tauri/src/mcp/access.rs`, `mcp/policy.rs`). Only the SHA-256 of the token is stored, in `tools/mcp/policy.sqlite3` inside the data folder.

**Permissions.** `policy::scope` maps every tool to a scope, and a tool whose scope the connection does not have is not listed at all. Scopes for reading and for writing are separate.

**`omniget_invoke`.** A registered Tauri command is reachable through this one tool (`mcp/invoke.rs`), from a catalog that `scripts/mcp-commands.mjs` builds out of `handlers.rs`. The same file holds the denied list: anything that returns a credential or a token, pairs a client, grants MCP access, answers an approval or runs a terminal command cannot be called from outside.

**Approvals.** A caller never approves its own request: the tools that answer an approval are on the denied list and behind their own scope (`src-tauri/src/approvals.rs`). With **Ask me first** on (`ask_first` in `mcp/access.rs`, off by default), posts on the person's accounts, system cleanups and new connections wait for a click in the OmniGet window. A tool that writes has to honour that setting.

**Adding a dedicated tool.**

1. Define it in the `tools()` list in `src-tauri/src/mcp/downloads.rs`.
2. Give it a scope in `policy::scope` (`mcp/policy.rs`). Without one, the tool does not exist.
3. Dispatch it in `downloads::call`.
4. Add its output schema in `mcp/output_schemas.rs`.
5. Run `pnpm gen:mcp-catalog`, which rewrites `src-tauri/omniget-cli/mcp-catalog.json`.

Prefer a few dense tools with a required `op` over many thin ones. Results are structured data; an empty result says why, and an error carries `code`, `retryable` and `next_step`.

**The stdio adapter.** `omniget-mcp` (`src-tauri/omniget-cli/src/bin/omniget-mcp.rs`) is for clients that only speak stdio. It forwards to the HTTP server and answers `tools/list` from the embedded `mcp-catalog.json`.

## A download, end to end

1. **The link.** In `src/routes/+page.svelte`, a pasted link calls `detect_platform`, then `prefetch_media_info` for the preview, then `download_from_url`.
2. **The command.** `download_from_url` in `src-tauri/src/commands/downloads.rs` checks the platform opt-out list, picks a downloader and puts the item in the queue.
3. **Picking a downloader.** `core/url_resolver.rs` asks `Platform::from_url` (`omniget-core/src/platforms/mod.rs`), then the direct-file check, then the registry (`omniget-core/src/core/registry.rs`). The registry is filled in `lib.rs` in order: gallery-dl, the native extractors, magnet and torrent, direct files, and last the generic yt-dlp downloader, which takes whatever nothing else claimed.
4. **The extractor.** Every downloader implements the `PlatformDownloader` trait in `omniget-core/src/platforms/traits.rs`. A new site is a new implementation of it.
5. **The queue.** `src-tauri/src/core/queue.rs` calls `downloader.download(...)` and turns its progress updates into events.
6. **Back to the screen.** The events are `queue-state-update`, `queue-item-progress`, `download-log-update` and `media-info-preview`. `src/lib/stores/download-listener.ts` listens and feeds the download store.

## Managed tools

OmniGet downloads yt-dlp, FFmpeg, gallery-dl and others for the user, into `bin/` inside the data folder (`omniget-core/src/core/dependencies.rs`). Nothing downloaded runs unchecked:

- yt-dlp is checked against the `SHA2-256SUMS` of its release.
- Assets from GitHub releases are checked against the digest the GitHub API reports. No digest, no install.
- FFmpeg from other hosts is checked against the published `.sha256`.
- aria2c is downloaded only on Windows, against a pinned hash.
- A binary installed through `install_staged_binary` is staged and has to answer `--version` before it replaces the old one. The single-file yt-dlp and the aria2c installers rely on the hash alone.

## The frontend

- Svelte 5 with runes. Screens are in `src/routes/`; feature code is in `src/lib/<feature>/`.
- Interface text lives in `src/lib/i18n/en.json`. The other languages come from Weblate. `pnpm check:i18n` flags missing keys and hard-coded text.
- `pnpm dev:web` opens the whole interface in a browser with no Rust build. `src/lib/dev/browser-preview/` answers `invoke` calls from `fixtures.ts`, then from empty values inferred from the types. See [scripts/dev/README.md](scripts/dev/README.md).

## The browser extension

`browser-extension/chrome/src/bridge-client.js` looks for the app on ports 47720 to 47729 and pairs with it. Pairing opens a single-use window of two minutes in the app; after that, every request carries a bearer token. What it sends (the page, its media and the cookies for that site) goes only to the local app.

## Data on disk

The data folder is resolved in `omniget-core/src/core/paths.rs`, in this order: `OMNIGET_DATA_DIR`, a redirect file left by moving the folder in Settings, then `wtf.tonho.omniget` in the system's data folder. Settings are `settings.json` in it.

A debug build uses `omniget-dev` instead, so it never touches an installed OmniGet. A `portable.txt` next to the executable turns on portable mode, with the data in `./data`.

## Tests

- Rust tests are `#[cfg(test)]` modules inside the file they test.
- Frontend tests are `*.test.ts` next to the code, run by Vitest.
- `pnpm verify` runs everything: format, Clippy and its baseline, Rust tests, svelte-check, Vitest, the i18n and IPC checks, and the extension and plugin tests. `pnpm verify --changed` runs only what your branch touched.

What gets a test and what does not is in [CONTRIBUTING.md](CONTRIBUTING.md#tests).
