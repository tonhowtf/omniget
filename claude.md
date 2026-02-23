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
- **Fonts:** System fonts as default; IBM Plex Mono for code and technical content only
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

#### Primary Palette

```css
/* Semantic surface tokens — use these in components */
--primary          /* page background: #ffffff light, #0a0a0a dark */
--secondary        /* primary text: #1a1a1a light, #e8e8e8 dark */
--tertiary         /* muted text: #666666 light, #999999 dark */

/* Action colors */
--accent           /* primary actions, focus rings: #F25C05 light, #FF7D38 dark */
--success          /* positive feedback: #1E5E3A light, #2E9B5A dark */
--error            /* destructive/error: #c41e3a light, #ff4757 dark */
--warning          /* attention: #d97706 light, #f59e0b dark */

/* Interactive surface tokens */
--button           /* default button bg: #f0f0f0 light, #1a1a1a dark */
--button-hover     /* hover state: #e0e0e0 light, #2a2a2a dark */
--button-press     /* active/pressed state: #d0d0d0 light, #3a3a3a dark */
--button-text      /* button label color: inherits --secondary */
--button-stroke    /* subtle border: rgba(0,0,0,0.08) light, rgba(255,255,255,0.12) dark */
--button-box-shadow: 0 0 0 1px var(--button-stroke) inset

/* Elevated surface (cards, dropdowns, popovers) */
--button-elevated       /* #e8e8e8 light, #252525 dark */
--button-elevated-hover /* #dcdcdc light, #323232 dark */
--button-elevated-press /* #d0d0d0 light, #3a3a3a dark */

/* Structural */
--sidebar-bg       /* sidebar panel: #f8f8f8 light, #0f0f0f dark */
--sidebar-highlight /* active tab: #efefef light, #1f1f1f dark */
--content-border   /* divider between sidebar and content: rgba(0,0,0,0.06) light, rgba(255,255,255,0.08) dark */
--input-border     /* form input borders: #b8b8b8 light, #3a3a3a dark */
--input-bg         /* form input backgrounds: #ffffff light, #1a1a1a dark */
--popup-bg         /* dialog/popover backgrounds: #ffffff light, #1a1a1a dark */
--dialog-backdrop  /* modal overlay: rgba(0,0,0,0.4) light, rgba(0,0,0,0.6) dark */

/* Layout constants */
--padding: 12px
--border-radius: 11px
--sidebar-width: 80px
```

IMPORTANT: Every color has both light and dark values. When adding a new token, define it in both `:root` (light) and `[data-theme="dark"]`.

#### Contrast Tokens (WCAG Compliance)

These tokens ensure sufficient contrast for accessibility. Use when text and background don't inherit from the same token hierarchy:

```css
--on-primary       /* text on --primary: --secondary in light, --secondary in dark */
--on-accent        /* text on --accent: #ffffff (always light) */
--on-success       /* text on --success: #ffffff (always light) */
--on-error         /* text on --error: #ffffff (always light) */
--on-button        /* text on --button: --secondary (inherited via --button-text) */
--on-button-elevated /* text on --button-elevated: --secondary */
```

### Typography

Typographic hierarchy uses system fonts for body text and UI, monospace only for code/technical content.

#### Font Stack

```css
/* System fonts: platform-native rendering, faster load, better accessibility */
--font-system: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
--font-mono: "IBM Plex Mono", "Courier New", monospace;

/* Default: body text, headings, buttons use system fonts */
body, .button, label { font-family: var(--font-system); }

/* Technical content only: code blocks, file paths, monospace output */
code, pre, .monospace { font-family: var(--font-mono); }
```

#### Heading Scale

All headings use `font-weight: 500` and `margin-block: 0`:

```css
h1: 24px, letter-spacing: -1px
h2: 20px, letter-spacing: -1px
h3: 16px
h4: 14.5px
h5: 12px
h6: 11px
```

#### Body Text

```css
buttons: 14.5px, font-weight: 500
.label: 13px, font-weight: 500
.body: 14px, font-weight: 400, line-height: 1.6
.subtext: 12.5px, font-weight: 500, color: var(--tertiary)
.caption: 11.5px, font-weight: 400, color: var(--tertiary)
.long-text: 14.5px, font-weight: 400, line-height: 1.8

/* Monospace content */
code, .code: 13px, font-family: var(--font-mono), font-weight: 400
```

### Spacing & Radius

Use `var(--padding)` (12px) as the base unit. Derive all spacing from it: `calc(var(--padding) / 2)` (6px), `calc(var(--padding) * 2)` (24px). Border radius is `var(--border-radius)` (11px) for all interactive elements. Nested elements use `calc(var(--border-radius) - var(--switcher-padding))`.

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

Focus: `outline: var(--focus-ring)` (solid 2px accent) with `outline-offset: var(--focus-ring-offset)` (2px). Applied on `:focus-visible` only, never on `:focus`.

### Custom Controls (with ARIA)

#### Toggle (Checkbox Switch)

Pure CSS toggle switch. Takes a single `enabled: boolean` prop. Uses `transform: translateX()` with `cubic-bezier(0.53, 0.05, 0.02, 1.2)` for springy feel. RTL-aware via `:dir(rtl)`.

```html
<label>
  <input type="checkbox" role="switch" aria-checked="true" />
  <span class="toggle"></span>
</label>
```

Ensure `aria-checked` reflects current state. Background transitions between `--toggle-bg` and `--toggle-bg-enabled`.

#### Select (Dropdown)

Native `<select>` overlaid on a styled button. The select element is `position: absolute`, transparent, covering the entire button. Current value displayed via a separate `<span>`.

```html
<div class="select-wrapper">
  <select aria-label="Choose option">
    <option>Option 1</option>
    <option>Option 2</option>
  </select>
  <button class="button" aria-hidden="true">
    <span class="select-label">Option 1</span>
  </button>
</div>
```

The native select provides accessibility; the button provides visual styling.

#### Input Field

Standard `<input>` with label and optional description:

```html
<label for="input-id">
  <span>Label Text</span>
  <input id="input-id" type="text" aria-describedby="hint-id" />
</label>
<small id="hint-id" class="caption">Helper text</small>
```

Always pair with `<label>` and use `aria-describedby` for helper text.

### Switcher (Segmented Control)

Groups multiple `.button` children into a joined row. First/last children keep their outer radius, middle children get `border-radius: 0`. Uses negative margin (`margin-left: -1px`) to eliminate double borders. Has `.big` variant with container background and inner padding.

### Settings Components

Three reusable primitives:
- **SettingsToggle:** label + description + Toggle. Uses generic TypeScript to type-check setting context/id against the settings schema.
- **SettingsDropdown:** native `<select>` overlaid on a styled button. The select is `position: absolute`, transparent, covering the entire button. Shows current value via a separate `<span>`.
- **SettingsCategory:** wraps a section with `id` for hash-linking. Supports focus highlight animation on hash match (2s accent inset box-shadow keyframe).

All settings components call `updateSetting({ [context]: { [id]: value } })` for atomic partial updates.

### Dialog System

Dialogs use native `<dialog>` element with `showModal()`. Implements ARIA Dialog role and requirements:

```html
<dialog role="dialog" aria-modal="true" aria-labelledby="dialog-title">
  <h2 id="dialog-title">Dialog Title</h2>
  <p>Dialog content</p>
  <button autofocus>Confirm</button>
</dialog>
```

Entry/exit animations via `open`/`closing` CSS classes with 150ms transition. Backdrop close via a separate `DialogBackdropClose` component. Stack managed by `$lib/state/dialogs` store — `killDialog()` pops the top dialog.

#### Dialog Requirements

- **Role & Attributes:** `role="dialog"`, `aria-modal="true"`, `aria-labelledby` referencing the title
- **Focus Trap:** First interactive element receives `autofocus`. Focus must cycle within dialog.
- **Backdrop Close:** ESC key or click outside closes the dialog. Backdrop uses `--dialog-backdrop` with appropriate contrast.
- **Scrolling:** Long content uses `max-height` with `overflow: auto`. Scrollbar hidden unless needed.
- **Positioning:** Centered via CSS `inset: 0; margin: auto`. Max width 600px or 90% viewport width.
- **Animations:** Entry (scale 0.95 → 1, opacity 0 → 1, 150ms ease-out). Exit (reverse, 150ms ease-in).

### Progress Bar

Simple div-in-div pattern. Outer: `--button-elevated` bg, 6px height, full rounded. Inner: `--accent` bg (not blue), width set to percentage with `transition: width 0.1s`. Indeterminate state uses a `Skeleton` shimmer component.

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

The sidebar becomes horizontal scrollable with fade gradients at edges. **Do NOT force dark theme on mobile in light mode** — respect the user's theme preference consistently across all breakpoints.

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

### Semantic HTML & ARIA

Use semantic HTML (`<button>`, `<label>`, `<dialog>`) as the foundation. ARIA is supplementary:
- `role="dialog"` + `aria-modal="true"` for dialog containers
- `aria-label` / `aria-labelledby` for unlabeled buttons
- `aria-describedby` for form hints and longer descriptions
- `aria-checked`, `aria-pressed` for custom toggle/button states
- `role="switch"` for checkbox-style toggles
- Avoid `role="button"` on `<div>` — use `<button>` instead

### Data Attributes for Preferences

Accessibility preferences are exposed as data attributes on the root element, then targeted in CSS:

```css
[data-reduce-motion="true"] * { animation: none !important; transition: none !important; }
[data-reduce-transparency="true"] { --dialog-backdrop: rgba(0, 0, 0, 0.75); }
```

Settings for accessibility: `reduceMotion`, `reduceTransparency`, `disableHaptics`, `dontAutoOpenQueue`.

### Contrast & Color Dependency

- **Text contrast:** Minimum 4.5:1 for body text, 3:1 for large text (18px+)
- **Color as information:** Never use color alone — pair with icon, text, or pattern
- **Focus indicators:** Solid outline, 2px, 2px offset. Test with forced-colors media query
- **Contrast tokens:** Use `--on-*` tokens for text on colored backgrounds

### Haptic Feedback

Haptics use a DOM checkbox-switch hack (creating a temporary `<input type="checkbox" switch>`, clicking it, then removing). Three patterns: `hapticSwitch()` (single tap), `hapticConfirm()` (double tap 120ms apart), `hapticError()` (triple tap). Always gated by `device.supports.haptics && !settings.accessibility.disableHaptics`.

### Focus Management

After navigation, auto-focus the element with `data-first-focus` attribute. All interactive elements use `:focus-visible` (never `:focus`) for ring display. Links and buttons get `outline: var(--focus-ring)` with `outline-offset: 2px`. Focus ring color is `--accent`.

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
- `user-select: auto` globally. Set `user-select: none` on interactive elements (buttons, icons). Set `user-select: text` explicitly on readable content (paragraphs, code blocks, technical text).
- Scrollbar hidden everywhere: `scrollbar-width: none` + `::-webkit-scrollbar { display: none }`.
- No `!important` except for accessibility overrides (`reduceMotion`, `reduceTransparency`).
- Animations: define as `@keyframes` in the component, provide reduced-motion alternative. Prefer `transform` and `opacity` for performance.
- Safe area: always account for `env(safe-area-inset-*)` in fixed/sticky elements.
- Focus ring: positive offset (2px), accent color (`--accent`), applied on `:focus-visible` only.
- Dialog focus: first interactive element receives `autofocus`, focus trap prevents escape, ESC or backdrop click closes.
