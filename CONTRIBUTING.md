# Contributing to OmniGet

Thank you for your interest in contributing to OmniGet! This document covers everything you need to get started.

## Table of Contents

- [Project Overview](#project-overview)
- [Prerequisites](#prerequisites)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Development Workflow](#development-workflow)
- [Frontend Guidelines](#frontend-guidelines)
- [Backend Guidelines](#backend-guidelines)
- [Adding a New Platform](#adding-a-new-platform)
- [Internationalization (i18n)](#internationalization-i18n)
- [Submitting Changes](#submitting-changes)

---

## Project Overview

OmniGet is a cross-platform desktop download manager built with:

- **Frontend**: SvelteKit 2 + Svelte 5 (runes) + TypeScript
- **Backend**: Rust + Tauri 2.x
- **Package manager**: pnpm (frontend), Cargo (Rust)

The codebase is a monorepo: `src/` is the SvelteKit frontend, `src-tauri/` is the Rust backend.

---

## Prerequisites

Install the following before starting:

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) 18+
- [pnpm](https://pnpm.io/)
- Tauri prerequisites for your OS — follow the [Tauri v2 guide](https://v2.tauri.app/start/prerequisites/)

---

## Development Setup

```bash
git clone https://github.com/tonhowtf/omniget.git
cd omniget
pnpm install        # install frontend dependencies
cargo tauri dev     # start the full app (Rust + frontend hot-reload)
```

Other useful commands:

```bash
pnpm dev            # run the SvelteKit dev server only (no Tauri window)
pnpm check          # run svelte-check + tsc (TypeScript type checking)
cargo check         # typecheck Rust without building
pnpm tauri build    # create a production build
```

---

## Project Structure

```
omniget/
├── src/                        # SvelteKit frontend
│   ├── app.css                 # Global CSS custom properties (design tokens)
│   ├── app.html                # HTML shell
│   ├── routes/                 # File-based routing (+page.svelte, +layout.svelte)
│   ├── components/             # Reusable UI components, organized by domain
│   │   ├── buttons/            # ActionButton, SettingsButton, SettingsToggle, Switcher
│   │   ├── dialog/             # DialogContainer, DialogButton, SmallDialog, PickerDialog
│   │   ├── hints/              # Contextual hint UI
│   │   ├── hotmart/            # Hotmart-specific components
│   │   ├── icons/              # Custom SVG icons (only when @tabler/icons-svelte lacks one)
│   │   ├── mascot/             # Loop mascot animations
│   │   ├── misc/               # Toggle, Skeleton, Placeholder, SectionHeading, OuterLink
│   │   ├── omnibox/            # OmniboxInput, MediaPreview, FormatSelector, QualityPicker
│   │   ├── onboarding/         # First-run onboarding flow
│   │   ├── services/           # Platform service components
│   │   ├── settings/           # SettingsCategory, SettingsDropdown, SettingsInput
│   │   └── toast/              # Toast notification components
│   └── lib/
│       ├── stores/             # Svelte stores (download, settings, toast, etc.)
│       └── i18n/               # Locale files: en.json, pt.json, zh.json, ja.json, it.json, fr.json
│
└── src-tauri/                  # Rust backend
    ├── src/
    │   ├── commands/           # Tauri IPC commands (called from the frontend via invoke())
    │   ├── platforms/          # One directory per supported platform (trait-based plugin system)
    │   │   ├── mod.rs          # Platform registry
    │   │   ├── youtube/
    │   │   ├── instagram/
    │   │   └── ...
    │   ├── core/               # Shared engine: ytdlp, hls_downloader, queue, ffmpeg, etc.
    │   ├── models/             # Shared data structs (media.rs, download.rs, settings.rs)
    │   └── storage/            # Config and database persistence
    └── omniget-core/           # Workspace crate: platform traits and shared core logic
```

---

## Development Workflow

1. **Find or open an issue** before starting significant work, so we can discuss the approach.
2. **Create a branch** from `main`: `git checkout -b my-feature`.
3. **Make your changes**, following the guidelines below.
4. **Test manually** by running `cargo tauri dev`.
5. **Type-check** before opening a PR:
   ```bash
   pnpm check     # frontend
   cargo check    # backend
   ```
6. **Open a pull request** against `main` with a clear description of what changed and why.

---

## Frontend Guidelines

### Svelte 5 — Runes Only

Always use Svelte 5 runes. Do not use the legacy Options API.

```svelte
<!-- Good -->
<script lang="ts">
  let count = $state(0);
  let double = $derived(count * 2);

  const { label } = $props<{ label: string }>();
</script>

<!-- Bad: legacy store/reactive syntax -->
<script>
  import { writable } from 'svelte/store';
  let count = writable(0);
  $: double = $count * 2;
  export let label;
</script>
```

Use **discriminated unions** for multi-state values:

```ts
type DownloadState =
  | { status: "idle" }
  | { status: "downloading"; progress: number }
  | { status: "error"; message: string };
```

### TypeScript

- TypeScript `strict` mode is enabled. No `any` unless unavoidable.
- Use path aliases: `$components/`, `$lib/`, `$i18n/`.

### Styling

- **No Tailwind. No CSS-in-JS.**
- Use scoped `<style>` blocks in each component.
- Use CSS custom properties for all colors and layout constants. Never hardcode hex values.
- Derive spacing from `var(--padding)` (12px): `calc(var(--padding) / 2)` = 6px, `calc(var(--padding) * 2)` = 24px.
- Border radius: `var(--border-radius)` (11px).
- Theme via `[data-theme="dark"]` / `[data-theme="light"]`. Define every token in both `:root` and `[data-theme="dark"]`.

Available color tokens: `--primary`, `--secondary`, `--tertiary`, `--accent`, `--success`, `--error`, `--warning`, `--button`, `--button-hover`, `--button-press`, `--button-text`, `--sidebar-bg`, `--input-bg`, `--popup-bg`.

Contrast tokens for text on colored backgrounds: `--on-primary`, `--on-accent`, `--on-success`, `--on-error`, `--on-button`.

### Icons

Use `@tabler/icons-svelte`. Import individually:

```svelte
import IconDownload from "@tabler/icons-svelte/IconDownload.svelte";
```

Only add a custom SVG component in `components/icons/` when the icon you need does not exist in Tabler.

### Accessibility (WCAG 2.2 AA)

- Semantic HTML first. No `role="button"` on `<div>`.
- All interactive elements must be keyboard accessible.
- Text contrast: 4.5:1 minimum. Focus ring: 3:1, minimum 6px area.
- Use `aria-label` / `aria-labelledby` for unlabeled buttons.
- Use `role="switch"` + `aria-checked` for toggles.
- Use `aria-live="polite"` for dynamic content updates.
- Use `aria-valuenow/min/max` on progress bars.
- Never communicate state through color alone — pair with icon + text.
- Support `prefers-reduced-motion`: add `[data-reduce-motion="true"]` alternatives for all animations.
- Support RTL with `:dir(rtl)` overrides. Prefer CSS logical properties.

### UX Patterns

- **Optimistic UI**: update the interface immediately on user action, then send the command in the background. Revert on failure.
- **Explicit errors**: never show vague messages. Always say what happened, why, and what to do.
  - Bad: `"Error"` → Good: `"HTTP 429 – Retrying in 15s (attempt 2/3)"`
- **Download progress**: always show phase + percent + bytes + speed. See the `PhaseIndicator` pattern in `claude.md`.
- Transient feedback → `toast-store` (5s auto-dismiss). Persistent feedback → inline.

---

## Backend Guidelines

### Rust Conventions

- Use `async`/`await` with `tokio`.
- Return `Result<T, E>` from fallible functions. Use `anyhow` for error propagation in command handlers.
- Use `tracing` macros (`tracing::info!`, `tracing::warn!`, `tracing::error!`) — not `println!`.
- Avoid `unwrap()` in production paths. Use `?` or handle errors explicitly.

### Tauri IPC Commands

Commands live in `src-tauri/src/commands/`. Each command is a public async function decorated with `#[tauri::command]` and registered in `lib.rs`.

```rust
// src-tauri/src/commands/downloads.rs
#[tauri::command]
pub async fn start_download(url: String, state: State<'_, AppState>) -> Result<String, String> {
    // ...
}
```

Invoke from the frontend with:

```ts
import { invoke } from "@tauri-apps/api/core";
const id = await invoke<string>("start_download", { url });
```

### Events

Use Tauri events to push real-time updates from Rust to the frontend. Event definitions live in `src-tauri/src/core/events.rs`. Listen with `listen()` in the frontend stores (`src/lib/stores/download-listener.ts`).

---

## Adding a New Platform

OmniGet uses a **trait-based plugin system**. Each platform implements the `PlatformDownloader` trait defined in `omniget-core`.

### Steps

1. **Create a directory** under `src-tauri/src/platforms/your_platform/`.
2. **Implement `PlatformDownloader`** in `mod.rs`:
   ```rust
   use omniget_core::platforms::traits::PlatformDownloader;

   pub struct YourPlatform;

   #[async_trait]
   impl PlatformDownloader for YourPlatform {
       fn name(&self) -> &'static str { "your_platform" }
       fn matches(&self, url: &str) -> bool { url.contains("yourplatform.com") }
       async fn download(&self, ctx: DownloadContext) -> anyhow::Result<()> { ... }
   }
   ```
3. **Register the platform** in `src-tauri/src/platforms/mod.rs`.
4. **Add i18n strings** for any new UI text (see [Internationalization](#internationalization-i18n)).
5. **Add the platform to the supported platforms table** in `README.md`.

For platforms requiring browser automation or login, look at the `hotmart/` or `telegram/` implementations as reference.

---

## Internationalization (i18n)

All user-visible strings must go through the i18n system.

### Adding or editing strings

Locale files are in `src/lib/i18n/`. The canonical source is `en.json`.

When you add a new key, add it to **all locale files**. If you don't speak the language, add a placeholder marked with a comment so translators know it needs work:

```json
// fr.json
"my_new_key": "TODO: translate from English: \"My new string\""
```

### Using strings in components

```svelte
<script lang="ts">
  import { t } from "$i18n";
</script>

<p>{$t("namespace.my_new_key")}</p>
```

### Adding a new language

1. Create a new locale file (e.g., `src/lib/i18n/de.json`) by copying `en.json`.
2. Register the language in `src/lib/i18n/index.ts`.
3. Add the language to the settings dropdown.

---

## Submitting Changes

- **Bug fixes and small improvements**: open a PR directly.
- **New features or platforms**: open an issue first to discuss the design.
- **Breaking changes**: discuss in an issue before implementing.

### PR checklist

- [ ] `pnpm check` passes (no TypeScript/Svelte errors)
- [ ] `cargo check` passes (no Rust errors)
- [ ] New strings added to all locale files
- [ ] No hardcoded colors (use CSS custom properties)
- [ ] Accessibility requirements met for any new UI
- [ ] PR description explains what changed and why

---

## Questions

Open a [GitHub issue](https://github.com/tonhowtf/omniget/issues) if you have questions or run into problems during setup.
