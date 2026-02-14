# OmniGet

Desktop download manager built with Tauri 2.0 (Rust backend) + SvelteKit (frontend). Monorepo: `src-tauri/` is Rust, `src/` is SvelteKit + TypeScript. Run `cargo tauri dev` to start.

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
- **Font:** IBM Plex Mono (monospace) as primary and only font
- **i18n:** `sveltekit-i18n` with JSON locale files in `i18n/{lang}/`
- **Bundler:** Vite 5, adapter-static, pnpm as package manager

## Project Layout

```
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
  lib/            # Shared logic: state/, types/, settings/, api/, i18n/
```

## Design System

### Color Architecture

Use CSS custom properties exclusively. NEVER hardcode colors. The theme system uses `[data-theme="dark"]` and `[data-theme="light"]` selectors on a parent element.

```css
/* Semantic tokens — use these in components */
--primary          /* page background: white in light, black in dark */
--secondary        /* primary text: black in light, #e1e1e1 in dark */
--gray: #75757e    /* muted text, placeholders */

/* Interactive surface tokens */
--button           /* default button bg: #f4f4f4 light, #191919 dark */
--button-hover     /* hover state */
--button-press     /* active/pressed state */
--button-text      /* button label color */
--button-stroke    /* subtle border: rgba black 0.06 or rgba white 0.05 */
--button-box-shadow: 0 0 0 1px var(--button-stroke) inset

/* Elevated surface (cards, dropdowns, popovers) */
--button-elevated       /* #e3e3e3 light, #282828 dark */
--button-elevated-hover
--button-elevated-press

/* Structural */
--sidebar-bg       /* sidebar panel: #f4f4f4 light, #131313 dark */
--sidebar-highlight
--content-border   /* subtle divider between sidebar and content */
--input-border     /* form input borders: #adadb7 light, #383838 dark */
--popup-bg         /* dialog/popover backgrounds */
--dialog-backdrop  /* modal overlay with transparency */

/* Feedback */
--blue: #2f8af9    /* focus rings, progress bars, links */
--red: #ed2236     /* errors, destructive actions */
--green: #30bd1b   /* success states */
--orange: #f19a38  /* warnings */

/* Layout constants */
--padding: 12px
--border-radius: 11px
--sidebar-width: 80px
```

IMPORTANT: Every color has both light and dark values. When adding a new token, define it in both `:root` (light) and `[data-theme="dark"]`.

### Typography

Single font family throughout the entire app. All text uses IBM Plex Mono, monospace.

```css
/* Heading scale — all font-weight: 500, margin-block: 0 */
h1: 24px, letter-spacing: -1px
h2: 20px, letter-spacing: -1px
h3: 16px
h4: 14.5px
h5: 12px
h6: 11px

/* Body text */
buttons: 14.5px, font-weight: 500
.subtext: 12.5px, font-weight: 500, color: var(--gray)
.long-text: 14.5px, font-weight: 400, line-height: 1.8
```

### Spacing & Radius

Use `var(--padding)` (12px) as the base unit. Derive all spacing from it: `calc(var(--padding) / 2)`, `calc(var(--padding) * 2)`. Border radius is `var(--border-radius)` (11px) for all interactive elements. Nested elements use `calc(var(--border-radius) - var(--switcher-padding))`.

## Component Patterns

### File Organization

Group components by domain, not by type:

```
components/
  sidebar/       # Sidebar, SidebarTab, Logo
  save/          # Omnibox, DownloadButton, ClearButton
  settings/      # SettingsCategory, SettingsDropdown, SettingsInput
  buttons/       # ActionButton, SettingsButton, SettingsToggle, Switcher
  dialog/        # DialogContainer, DialogButton, SmallDialog, PickerDialog
  queue/         # ProcessingQueue, ProcessingQueueItem, ProgressBar
  misc/          # Toggle, Skeleton, Placeholder, SectionHeading, OuterLink
  icons/         # Custom SVG icon components (only when tabler doesn't have it)
  subnav/        # PageNav, PageNavTab, PageNavSection
```

### Button System

All buttons use the global `button` / `.button` base class. Variants via additional classes:

```
.button              → default surface (--button bg, --button-text color, inset box-shadow)
.button.elevated     → raised surface (--button-elevated bg, no box-shadow)
.button.active       → selected/on state (--secondary bg, --primary text, inverted)
.button.active.color → active with custom color (skips default hover overrides)
```

Hover states use `@media (hover: hover)` to avoid sticky hover on touch. Active (press) states always apply. Disabled buttons get `cursor: default` only — no opacity change by default.

Focus: `outline: var(--focus-ring)` (solid 2px blue) with `outline-offset: var(--focus-ring-offset)` (-2px). Applied on `:focus-visible` only, never on `:focus`.

### Toggle Component

Pure CSS toggle switch. Takes a single `enabled: boolean` prop. Uses `transform: translateX()` with `cubic-bezier(0.53, 0.05, 0.02, 1.2)` for a springy feel. RTL-aware via `:dir(rtl)`. Background transitions between `--toggle-bg` and `--toggle-bg-enabled`.

### Switcher (Segmented Control)

Groups multiple `.button` children into a joined row. First/last children keep their outer radius, middle children get `border-radius: 0`. Uses negative margin (`margin-left: -1px`) to eliminate double borders. Has `.big` variant with container background and inner padding.

### Settings Components

Three reusable primitives:
- **SettingsToggle:** label + description + Toggle. Uses generic TypeScript to type-check setting context/id against the settings schema.
- **SettingsDropdown:** native `<select>` overlaid on a styled button. The select is `position: absolute`, transparent, covering the entire button. Shows current value via a separate `<span>`.
- **SettingsCategory:** wraps a section with `id` for hash-linking. Supports focus highlight animation on hash match (2s blue inset box-shadow keyframe).

All settings components call `updateSetting({ [context]: { [id]: value } })` for atomic partial updates.

### Dialog System

Dialogs use native `<dialog>` element with `showModal()`. Entry/exit animations via `open`/`closing` CSS classes with 150ms transition. Backdrop close via a separate `DialogBackdropClose` component. Stack managed by `$lib/state/dialogs` store — `killDialog()` pops the top dialog.

### Progress Bar

Simple div-in-div pattern. Outer: `--button-elevated` bg, 6px height, full rounded. Inner: `--blue` bg, width set to percentage with `transition: width 0.1s`. Indeterminate state uses a `Skeleton` shimmer component.

### Processing Queue Items

Discriminated union type: `waiting | running | done | error`. Each state shows different icon (loader spinner, check, X, exclamation). Uses `@tabler/icons-svelte` for all status icons. File type icons map: `{ file: IconFile, video: IconMovie, audio: IconMusic, image: IconPhoto }`.

## Layout Architecture

### Grid System

Root layout uses CSS Grid with two columns: sidebar + content area.

```css
#app {
  display: grid;
  grid-template-columns: calc(var(--sidebar-width) + var(--sidebar-inner-padding) * 2) 1fr;
  height: 100%;
  position: fixed;
}
```

Content area has `overflow: scroll` and a subtle `box-shadow` border against the sidebar (not a real border — using `--content-border` with `--content-border-thickness`).

### Mobile Breakpoint: 535px

At `max-width: 535px`, the grid flips to rows: content on top, sidebar as a fixed bottom tab bar.

```css
@media (max-width: 535px) {
  #app { grid-template-columns: unset; grid-template-rows: 1fr auto; }
  #sidebar { position: fixed; bottom: 0; flex-direction: row; width: 100%; }
  #content { order: -1; /* content above sidebar */ }
}
```

The sidebar becomes horizontal scrollable with fade gradients at edges (`--sidebar-mobile-gradient`). On mobile light theme, sidebar forces dark: `--sidebar-bg: #000000`.

### Sub-Navigation (Settings, About)

Two-column layout within content area: `grid-template-columns: 250px 1fr`. Navigation sidebar on left, page content on right (max-width: 600px). At `max-width: 750px`, collapses to single column with back button navigation (mobile drill-down pattern).

### Safe Area Handling

Full safe-area-inset support for notched/Dynamic Island devices:
- `env(safe-area-inset-top)` as content padding
- `env(safe-area-inset-bottom)` in sidebar height calculation
- `env(safe-area-inset-left)` for landscape iPhone sidebar

## State Management

### Settings Store

Partial-merge pattern: store holds only user-changed values (partial). A derived store merges with defaults using `ts-deepmerge`. This means defaults can change across versions without overwriting user preferences.

```
storedSettings (readable) → holds PartialSettings from localStorage
settings (derived)        → merges storedSettings with defaultSettings
updateSetting(partial)    → deep-merges partial into stored, writes to localStorage
```

### Settings Schema Versioning

Settings use `schemaVersion: number` for migrations. Migration functions are keyed by target version and run sequentially:

```typescript
const migrations: Record<number, Migrator> = {
  [3]: (s) => { /* v2→v3 transform */ },
  [4]: (s) => { /* v3→v4 transform */ },
};
export const migrate = (settings) =>
  Object.keys(migrations)
    .map(Number)
    .filter(v => v > settings.schemaVersion)
    .reduce((s, v) => migrations[v](s), settings);
```

When adding settings, increment `schemaVersion` in defaults and add a migration function. Migrations handle renames, moves between categories, type changes, and removal of deprecated keys.

### Theme System

Three-way: `auto | light | dark`. Auto resolves via `window.matchMedia('(prefers-color-scheme: dark)')` with a live change listener. Theme is applied as `data-theme` attribute on a wrapper div, NOT on `<html>` or `<body>`. Status bar meta tag color differs for mobile vs desktop.

### Other Stores

- `$lib/state/omnibox` — reactive `link` string for the URL input
- `$lib/state/dialogs` — dialog stack (array), push/kill pattern
- `$lib/state/queue-visibility` — toggle processing queue panel
- `$lib/state/task-manager/` — download tasks and worker coordination

## Accessibility

### Data Attributes for Preferences

Accessibility preferences are exposed as data attributes on the root element, then targeted in CSS:

```css
[data-reduce-motion="true"] * { animation: none !important; transition: none !important; }
[data-reduce-transparency="true"] { --dialog-backdrop: /* higher opacity */ }
```

Settings for accessibility: `reduceMotion`, `reduceTransparency`, `disableHaptics`, `dontAutoOpenQueue`.

### Haptic Feedback

Haptics use a DOM checkbox-switch hack (creating a temporary `<input type="checkbox" switch>`, clicking it, then removing). Three patterns: `hapticSwitch()` (single tap), `hapticConfirm()` (double tap 120ms apart), `hapticError()` (triple tap). Always gated by `device.supports.haptics && !settings.accessibility.disableHaptics`.

### Focus Management

After navigation, auto-focus the element with `data-first-focus` attribute. All interactive elements use `:focus-visible` (never `:focus`) for ring display. Links outside sidebar/subnav get `outline-offset: 3px` and `border-radius: 2px`.

### RTL Support

Fully bidirectional. Use `:dir(rtl)` pseudo-class for directional overrides. Logical properties preferred where possible, but physical overrides exist for complex layouts (sidebar padding, download button borders, toggle direction).

## i18n

Translations loaded lazily per route. File structure: `i18n/{lang}/{namespace}.json`. Access via `$t("namespace.key")`. Default locale: `en`. Language auto-detection from browser with manual override in settings. All user-visible strings MUST go through `$t()` — never hardcode text.

## Coding Rules

- Svelte 5 runes only: `$state`, `$derived`, `$effect`, `$props`. No `let x` for reactive declarations in new code.
- TypeScript strict mode. Use discriminated unions for state variants (see queue types).
- Generic TypeScript on settings components to enforce compile-time correctness of setting context/id/value.
- Scoped `<style>` in every component. Use `:global()` sparingly and only for child component overrides.
- Use `$components/` alias for component imports, `$lib/` for lib, `$i18n/` for translations.
- Global styles only in `app.css`. Component styles always scoped.
- `@media (hover: hover)` for hover states. Always provide `:active` fallback.
- All images and SVGs get `pointer-events: none`.
- `user-select: none` globally, `user-select: text` explicitly on readable content.
- Scrollbar hidden everywhere: `scrollbar-width: none` + `::-webkit-scrollbar { display: none }`.
- No `!important` except for accessibility overrides (`reduceMotion`).
- Animations: define as `@keyframes` in the component, provide reduced-motion alternative. Prefer `transform` and `opacity` for performance.
- Safe area: always account for `env(safe-area-inset-*)` in fixed/sticky elements.