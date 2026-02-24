# OmniGet

Desktop download manager built with Tauri 2.0 (Rust backend) + SvelteKit (frontend). Modern design principles (2025-2026): clarity over features, immediate feedback, and universal accessibility. Monorepo: `src-tauri/` is Rust, `src/` is SvelteKit + TypeScript. Run `cargo tauri dev` to start.

## Commands
```bash
pnpm install          # install frontend deps
pnpm dev              # SvelteKit dev server only
cargo tauri dev       # full app (Rust + frontend)
cargo check           # typecheck Rust without building
pnpm check            # svelte-check + tsc
```

## Tech Stack

- **Backend:** Rust, Tauri 2.x, tokio, reqwest, serde, sqlx (SQLite), chromiumoxide
- **Frontend:** SvelteKit 2, Svelte 5 (runes: `$state`, `$derived`, `$effect`, `$props`), TypeScript strict
- **Styling:** Scoped CSS with CSS custom properties. NO Tailwind. NO CSS-in-JS.
- **Icons:** `@tabler/icons-svelte` — import individually: `import IconDownload from "@tabler/icons-svelte/IconDownload.svelte"`
- **Fonts:** System fonts as default; IBM Plex Mono for code and technical content only
- **i18n:** `sveltekit-i18n` with JSON locale files in `i18n/{lang}/`
- **Bundler:** Vite 5, adapter-static, pnpm as package manager

## Project Layout
src-tauri/src/
commands/       # Tauri IPC commands (invoked from frontend)
platforms/      # Download platform implementations (trait-based plugin system)
traits.rs     # PlatformDownloader trait — all platforms implement this
hotmart/      # Hotmart-specific: auth, api, parser, downloader
core/           # Shared engine: registry, hls_downloader, media_processor, queue
models/         # Data structs: media.rs, download.rs, settings.rs
storage/        # Persistence: config, database (SQLite), cache
src/
routes/         # SvelteKit file-based routing (+page.svelte, +layout.svelte)
components/     # Reusable UI, organized by domain (see Component Organization)
lib/            # Shared logic: stores/, i18n/

## Design System

### Core Principles

**Minimalism with Purpose**: Every element has a function. No decoration.

**Immediate Feedback**: Progress bars show percent + bytes + speed. Errors are explicit ("Missing API key" not "Error").

**Color as Guidance**: Accent for primary actions, error for destructive. Never communicate through color alone (pair with icon + text).

**Accessibility First**: WCAG 2.2 AA baseline. 4.5:1 text contrast, 3:1 focus indicators, all elements keyboard accessible.

### Color Architecture

CSS custom properties exclusively. NEVER hardcode colors. Theme via `[data-theme="dark"]` / `[data-theme="light"]`. Every token defined in both `:root` and `[data-theme="dark"]`.

**Tokens** (see app.css for values): `--primary`, `--secondary`, `--tertiary`, `--accent`, `--success`, `--error`, `--warning`, `--button`, `--button-hover`, `--button-press`, `--button-text`, `--button-stroke`, `--button-elevated`, `--sidebar-bg`, `--sidebar-highlight`, `--content-border`, `--input-border`, `--input-bg`, `--popup-bg`, `--dialog-backdrop`.

**Contrast tokens** for text on colored backgrounds: `--on-primary`, `--on-accent`, `--on-success`, `--on-error`, `--on-button`, `--on-button-elevated`.

**Layout constants**: `--padding: 12px`, `--border-radius: 11px`, `--sidebar-width: 80px`.

### Typography

System fonts for UI, monospace for code only:
- `--font-system: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif`
- `--font-mono: "IBM Plex Mono", "Courier New", monospace`

Headings: `font-weight: 500`, `margin-block: 0`. Scale: h1=24px, h2=20px, h3=16px, h4=14.5px, h5=12px, h6=11px.

Body: buttons=14.5px/500, .label=13px/500, .body=14px/400/1.6, .subtext=12.5px/500, .caption=11.5px/400, code=13px/mono.

### Spacing & Radius

Base unit: `var(--padding)` (12px). Derive: `calc(var(--padding) / 2)` (6px), `calc(var(--padding) * 2)` (24px). Border radius: `var(--border-radius)` (11px).

## Component Patterns

### File Organization
components/
buttons/       # ActionButton, SettingsButton, SettingsToggle, Switcher
dialog/        # DialogContainer, DialogButton, SmallDialog, PickerDialog
hints/         # Contextual hints UI
hotmart/       # Hotmart-specific components
icons/         # Custom SVG icon components (only when tabler doesn't have it)
mascot/        # Loop mascot animations
misc/          # Toggle, Skeleton, Placeholder, SectionHeading, OuterLink
omnibox/       # OmniboxInput, MediaPreview, FormatSelector, QualityPicker, DownloadModeSelector
onboarding/    # First-run onboarding flow
services/      # Platform service components
settings/      # SettingsCategory, SettingsDropdown, SettingsInput
toast/         # Toast notification components

### Button System

Classes: `.button` (default), `.button.elevated`, `.button.active`, `.button.active.color`. Hover via `@media (hover: hover)`. Focus: `outline: var(--focus-ring)` on `:focus-visible` only.

### Custom Controls

- **Toggle**: `<input type="checkbox" role="switch" aria-checked>` with CSS transform animation. RTL-aware via `:dir(rtl)`.
- **Select**: Native `<select>` overlaid on styled button (position: absolute, transparent). Native provides a11y, button provides visuals.
- **Input**: Always pair with `<label>` + `aria-describedby` for helper text.

### Switcher

Joined button row. Middle children get `border-radius: 0`. Negative margin eliminates double borders. `.big` variant with container bg.

### Settings Components

- **SettingsToggle**: label + description + Toggle. Generic TS for type-safe setting context/id.
- **SettingsDropdown**: native select overlay pattern.
- **SettingsCategory**: section with `id` for hash-linking + focus highlight animation.

All call `updateSetting({ [context]: { [id]: value } })`.

### Dialog System

Native `<dialog>` + `showModal()`. ARIA: `role="dialog"`, `aria-modal="true"`, `aria-labelledby`. First element gets `autofocus`. ESC/backdrop closes. 150ms entry/exit animation.

### Progress Bar

Outer: `--button-elevated`, 6px, rounded. Inner: status-colored (`--blue` downloading, `--green` complete, `--red` error, `--gray` paused), width=percent, `transition: width 0.3s`. Indeterminate: Skeleton shimmer. Always determinate for downloads: `45% • 234 MB / 512 MB • 2.5 MB/s`.

TO-DO: Add ETA display (`~3m 20s` via `(total - downloaded) / speed`).

### Processing Queue Items

Discriminated union: `waiting | running | done | error`. Icons: spinner, check, X, exclamation. File type map: `{ video: IconMovie, audio: IconMusic, image: IconPhoto, file: IconFile }`.

## Layout Architecture

### Grid System

Two-column CSS Grid: sidebar + content. Content scrolls, sidebar fixed. Shadow border via `--content-border`.

### Mobile Breakpoint: 535px

Grid flips to rows. Sidebar becomes fixed bottom tab bar (horizontal scroll). Do NOT force dark theme on mobile.

### Sub-Navigation

Two-column: 250px nav + content (max 600px). Collapses at 750px to single column with back button.

### Safe Area

`env(safe-area-inset-top/bottom/left)` on all fixed/sticky elements.

## Download UX Principles

### Phase Indicators

Show download phase explicitly (never "is it hung?"):
1. "Preparing" — Item enqueued, building config
2. "Fetching Info" — Running yt-dlp info extraction (only if cache miss)
3. "Starting" — Download engine initializing (via -1.0 sentinel)
4. "Connecting" — yt-dlp spawned, awaiting first response (via -2.0 sentinel)
5. "Downloading" — Data flowing (percent > 0%)

TO-DO: Add explicit "Merging" phase when yt-dlp emits `[Merger]` line (currently shows ~99% with no phase label).

### Determinate Progress

ALWAYS determinate bars: `234 MB / 512 MB (45%) • 2.5 MB/s`. ETA = (total - downloaded) / speed. Smooth speed with rolling average.

TO-DO: ETA display not yet implemented. Speed rolling average not yet implemented (raw yt-dlp speed used directly).

### Optimistic UI

Pause/Resume/Remove: update UI instantly, send command in background, revert on failure.

### Error Feedback

Never vague. Always: what happened + why + what to do.
- ❌ "Error" → ✅ "HTTP 429 - Retrying in 15s..."
- ❌ "Loading..." → ✅ "Cannot access cookies - Continuing without auth"

Toast for transient (5s auto-dismiss). Inline for persistent.

### Completion Feedback

Color → --success, icon → checkmark, haptic → hapticConfirm(), toast → "Downloaded: file.mp4 (123 MB)" + "Open folder".

TO-DO: "Open folder" button not yet implemented on completed items (i18n key `open_folder` exists but is unused).

### Rate Limiting (429)

Detect → show "Retrying in 10s (2/3)" → rotate player_client/cookies → after 3: detailed error + manual retry button.

TO-DO: Retry status ("Retrying in 10s (2/3)") is only logged server-side via `tracing::warn!`, not shown to the user. Need to emit retry events to frontend.

### Batch Operations

Aggregate: "3 Downloading, 2 Paused". Sidebar badge (red circle). Peek panel: top 3 + total.

## State Management

### Settings Store

Partial-merge: store holds user-changed values only. Derived store merges with defaults via `ts-deepmerge`.

### Schema Versioning

`schemaVersion: number` + migration functions keyed by version. Increment version + add migrator when changing settings.

### Theme System

`auto | light | dark`. Applied as `data-theme` on wrapper div (not html/body).

### Other Stores

All stores in `$lib/stores/`: `download-store` (queue state), `download-listener` (Tauri event listeners), `media-preview-store` (URL preview), `toast-store`, `settings-store`, `update-store`, `convert-store`, `changelog-store`, `onboarding-store`, `clipboard-monitor`.

## Accessibility

WCAG 2.2 AA compliance required.

### ARIA Rules

Semantic HTML first. `role="dialog"` + `aria-modal`. `aria-label`/`aria-labelledby` for unlabeled buttons. `role="switch"` for toggles. `aria-live="polite"` for dynamic updates. `aria-valuenow/min/max` for progress. No `role="button"` on `<div>`.

### Preferences

`[data-reduce-motion="true"]` → disable all animation. `[data-reduce-transparency="true"]` → solid backdrops. Settings: `reduceMotion`, `reduceTransparency`, `disableHaptics`.

### Contrast

Text: 4.5:1. Focus: 3:1 + min 6px area. Never color alone. Use `--on-*` tokens. Test in forced-colors mode.

### Haptics

`hapticSwitch()`, `hapticConfirm()`, `hapticError()`. Gated by `device.supports.haptics && !disableHaptics`.

### Focus & Keyboard

Tab order: natural DOM. `:focus-visible` only. Focus ring: 2px --accent, offset 2px. ESC closes dialogs. Enter/Space activates buttons. Arrow keys navigate lists.

### RTL

`:dir(rtl)` for overrides. Logical properties preferred.

## Loop Mascot

Expressive creature reacting to download state (idle → downloading → paused → complete → error). Pure visual feedback — never required for understanding. IP protected.

## i18n

Lazy per route. `i18n/{lang}/{namespace}.json`. `$t("namespace.key")`. Default: `en`. All visible strings via `$t()`.

## Coding Rules

### Svelte & TypeScript

Svelte 5 runes only (`$state`, `$derived`, `$effect`, `$props`). TypeScript strict. Discriminated unions for state.

### Styling

Scoped `<style>`. `$components/`, `$lib/`, `$i18n/` aliases. `@media (hover: hover)` + `:active` fallback. `pointer-events: none` on images/SVGs. Scrollbar hidden. No `!important` except a11y. Safe area insets.

### Feedback

Immediate UI on click (optimistic). Progress: phase + percent + bytes + speed. Errors: explicit + actionable. Tooltips: 5-10 words + shortcut.

### Focus & Animations

`:focus-visible` ring (2px --accent, offset 2px). `prefers-reduced-motion` alternative. Dialog: autofocus first element, trap, ESC closes.

### Downloads

Determinate progress with ARIA. Phase icon + text (no color-alone). Error → detail + Retry button. Batch → sidebar badge + aggregate.
