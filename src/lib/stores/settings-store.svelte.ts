import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  WORKSPACE_APPEARANCE_SOURCE,
  compareRevision,
  isApiError,
  isRevision,
  type AppearanceMode,
  type EffectId,
  type ImportAppearanceResult,
  type LoadStatus,
  type PendingEffect,
  type ReadySnapshot,
  type ResetScope,
  type RetryEffectResult,
  type Revision,
  type SettingsPublicV3,
  type SettingsSecretOperations,
  type SettingsSection,
  type SettingsSnapshot,
  type TypographySettings,
} from "./settings-v3-types";

export type {
  ApiError,
  EffectId,
  LoadStatus,
  PendingEffect,
  ResetScope,
  Revision,
  SettingsPublicV3,
  SettingsSecretOperations,
  SettingsSection,
  TypographySettings,
} from "./settings-v3-types";

/** Alias kept for the existing consumers: the redacted v3 public document. */
export type AppSettings = SettingsPublicV3;

type Patch = Record<string, unknown>;

export type FieldState = "idle" | "saving" | "saved" | "failed" | "pendingApply";
export type FieldStatus = { state: FieldState; error?: string; effectId?: EffectId };
export type SettingsLoadState =
  | { kind: "loading" }
  | { kind: "ready"; loadStatus: LoadStatus }
  | { kind: "failed"; code: string };
export type AppearanceMigrationState = "unknown" | "importing" | "done" | "failed";

// ---------------------------------------------------------------------------
// Appearance: one authority for theme, preset, typography and motion.
// ---------------------------------------------------------------------------

export const OMNIGET_PRESET = "omniget-desktop";
export const WORKSPACE_PRESET_IDS = [
  "amber-minimal", "orange-soft", "claude-amber", "amber-hearth", "amber-slate", "orange-vivid",
] as const;
const WORKSPACE_PRESETS = new Set<string>(WORKSPACE_PRESET_IDS);
const MODES = new Set<string>(["system", "light", "dark"]);

export type AppearancePresentation = {
  mode: AppearanceMode;
  preset: string;
  /** Named theme id ("Mais estilos") when the preset is one, else null. */
  namedTheme: string | null;
  resolvedMode: "light" | "dark";
  systemDark: boolean;
};

const TYPOGRAPHY_DEFAULTS: TypographySettings = {
  font_display: "System",
  font_body: "System",
  font_mono: "IBM Plex Mono",
  line_height_base: 1.55,
  spacing_scale: 1.0,
  preset_name: "system",
};

/**
 * The single spacing base. `src/app.css` declares the same values on :root
 * (guarded by a test), so scale 1 removes the inline overrides without a jump.
 */
export const SPACING_BASE_PX: Readonly<Record<string, number>> = {
  "--space-1": 4,
  "--space-2": 8,
  "--space-3": 12,
  "--space-4": 16,
  "--space-5": 20,
  "--space-6": 28,
  "--space-7": 40,
  "--space-8": 56,
  "--space-9": 80,
};

/** CSS generic families and system keywords: never quoted. */
const GENERIC_FAMILIES = new Set([
  "system-ui", "ui-sans-serif", "ui-serif", "ui-monospace", "ui-rounded",
  "sans-serif", "serif", "monospace", "cursive", "fantasy", "math", "emoji", "fangsong",
]);
const SYSTEM_UI_STACK = `system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif`;
const UI_MONOSPACE_STACK = `ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`;

function quoteFamily(name: string): string {
  if (GENERIC_FAMILIES.has(name)) return name;
  return `'${name.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}'`;
}

/** Font stack for a chosen family. Custom families are preserved as typed. */
export function resolveFontStack(kind: "display" | "body" | "mono", family: string): string {
  const name = family.trim();
  if (kind === "mono") {
    if (name === "ui-monospace" || name === "" || name === "System" || name === "system-ui") return UI_MONOSPACE_STACK;
    if (name === "IBM Plex Mono") return `'IBM Plex Mono', 'DejaVu Sans Mono', 'Liberation Mono', 'Courier New', monospace`;
    if (GENERIC_FAMILIES.has(name)) return `${name}, monospace`;
    return `${quoteFamily(name)}, ui-monospace, 'Courier New', monospace`;
  }
  if (name === "System" || name === "") {
    return kind === "display"
      ? `-apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Segoe UI', system-ui, sans-serif`
      : `-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI', system-ui, sans-serif`;
  }
  if (name === "system-ui") return SYSTEM_UI_STACK;
  if (name === "ui-monospace") return UI_MONOSPACE_STACK;
  if (name === "Bricolage Grotesque Variable") {
    return `'Bricolage Grotesque Variable', 'Bricolage Grotesque', ui-sans-serif, system-ui, sans-serif`;
  }
  if (name === "Inter") {
    return kind === "display"
      ? `'Inter', 'DM Sans', ui-sans-serif, system-ui, sans-serif`
      : `'Inter', 'DM Sans', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif`;
  }
  if (name === "DM Sans") {
    return kind === "display"
      ? `'DM Sans', 'Inter', ui-sans-serif, system-ui, sans-serif`
      : `'DM Sans', 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif`;
  }
  if (name === "IBM Plex Mono") return `'IBM Plex Mono', ui-monospace, monospace`;
  if (GENERIC_FAMILIES.has(name)) return `${name}, system-ui, sans-serif`;
  return `${quoteFamily(name)}, ui-sans-serif, system-ui, sans-serif`;
}

function rootElement(): HTMLElement | null {
  return typeof document === "undefined" ? null : document.documentElement;
}

function applyTypography(typo: TypographySettings | undefined) {
  const rootEl = rootElement();
  if (!rootEl) return;
  const t = typo ?? TYPOGRAPHY_DEFAULTS;
  const root = rootEl.style;
  // 'OmniGet Lao' is unicode-range limited to Lao, so it only affects Lao glyphs.
  root.setProperty("--font-display", `'OmniGet Lao', ${resolveFontStack("display", t.font_display)}`);
  root.setProperty("--font-body", `'OmniGet Lao', ${resolveFontStack("body", t.font_body)}`);
  root.setProperty("--font-mono", resolveFontStack("mono", t.font_mono));
  root.setProperty("--leading-base", String(t.line_height_base));
  const scale = typeof t.spacing_scale === "number" && Number.isFinite(t.spacing_scale) ? t.spacing_scale : 1.0;
  // One base multiplied continuously; at 1 the stylesheet already holds base × 1.
  if (Math.abs(scale - 1.0) > 0.001) {
    for (const [key, base] of Object.entries(SPACING_BASE_PX)) {
      root.setProperty(key, `${(base * scale).toFixed(2)}px`);
    }
  } else {
    for (const key of Object.keys(SPACING_BASE_PX)) root.removeProperty(key);
  }
}

function applyAccessibility(a: AppSettings["accessibility"]) {
  const rootEl = rootElement();
  if (!rootEl) return;
  rootEl.setAttribute("data-reduce-motion", a?.reduce_motion ? "true" : "false");
  rootEl.setAttribute("data-reduce-transparency", a?.reduce_transparency ? "true" : "false");
}

/** Mode/preset of the confirmed document; before the import only the legacy theme counts. */
export function resolveAppearance(appearance: AppSettings["appearance"] | undefined): { mode: AppearanceMode; preset: string; namedTheme: string | null } {
  const legacy = typeof appearance?.theme === "string" ? appearance.theme : "system";
  const migrated = (appearance?.migration_version ?? 0) >= 1;
  let mode: AppearanceMode;
  let preset: string;
  if (migrated && MODES.has(appearance!.mode) && typeof appearance!.preset === "string" && appearance!.preset) {
    mode = appearance!.mode;
    preset = appearance!.preset;
  } else {
    mode = MODES.has(legacy) ? (legacy as AppearanceMode) : "system";
    preset = MODES.has(legacy) || !legacy ? OMNIGET_PRESET : legacy;
  }
  const namedTheme = preset !== OMNIGET_PRESET && !WORKSPACE_PRESETS.has(preset) ? preset : null;
  return { mode, preset, namedTheme };
}

const SYSTEM_DARK_QUERY = "(prefers-color-scheme: dark)";
let systemQuery: MediaQueryList | null = null;
let systemListener: ((event: { matches: boolean }) => void) | null = null;

function readSystemDark(): boolean {
  try {
    return typeof window !== "undefined" && typeof window.matchMedia === "function"
      && window.matchMedia(SYSTEM_DARK_QUERY).matches;
  } catch {
    return false;
  }
}

/** Follow OS changes only while the mode is system; the listener is removed otherwise. */
function syncSystemListener(follow: boolean) {
  if (follow && !systemListener) {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    try {
      const query = window.matchMedia(SYSTEM_DARK_QUERY);
      // Before the first confirmed read the fallback follows the OS as well.
      const listener = () => { applyTheme(settings?.appearance); };
      query.addEventListener("change", listener);
      systemQuery = query;
      systemListener = listener;
    } catch { /* No media queries in this runtime: the sampled value stays. */ }
  } else if (!follow && systemListener) {
    systemQuery?.removeEventListener("change", systemListener);
    systemQuery = null;
    systemListener = null;
  }
}

let presentation = $state<AppearancePresentation>({
  mode: "system", preset: OMNIGET_PRESET, namedTheme: null, resolvedMode: "dark", systemDark: true,
});

function applyTheme(appearance: AppSettings["appearance"] | undefined) {
  const { mode, preset, namedTheme } = resolveAppearance(appearance);
  syncSystemListener(mode === "system");
  const systemDark = readSystemDark();
  const resolvedMode: "light" | "dark" = mode === "system" ? (systemDark ? "dark" : "light") : mode;
  presentation = { mode, preset, namedTheme, resolvedMode, systemDark };
  const root = rootElement();
  if (!root) return;
  root.setAttribute("data-theme", namedTheme ?? resolvedMode);
  const workspace = namedTheme === null && WORKSPACE_PRESETS.has(preset);
  if (namedTheme === null) root.setAttribute("data-preset", preset);
  else root.removeAttribute("data-preset");
  // Workspace presets keep their palettes in workspace-design.css, now for every route.
  if (workspace) {
    root.setAttribute("data-ds-preset", preset);
    root.setAttribute("data-ds-mode", resolvedMode);
    root.classList.add("ds-scope");
  } else {
    root.removeAttribute("data-ds-preset");
    root.removeAttribute("data-ds-mode");
    root.classList.remove("ds-scope");
  }
}

export function getAppearancePresentation(): AppearancePresentation {
  return presentation;
}

// Locale is applied by the layout (it owns the translation loader); the store
// only says when the confirmed language changed.
type LocaleApplier = (language: string) => Promise<void> | void;
let localeApplier: LocaleApplier | null = null;
let appliedLanguage: string | null = null;
let localApplyFailed = $state<Record<string, string>>({});

export function setSettingsLocaleApplier(applier: LocaleApplier | null): void {
  localeApplier = applier;
  // The first read may have been confirmed before the layout mounted its applier.
  if (applier && settings) applyLocale(settings.appearance?.language);
}

function applyLocale(language: string | undefined) {
  if (!language || language === appliedLanguage) return;
  // Without an applier nothing was applied yet: remember nothing, so it runs on registration.
  if (!localeApplier) return;
  appliedLanguage = language;
  const target = language;
  Promise.resolve()
    .then(() => localeApplier?.(target))
    .then(() => {
      if (appliedLanguage === target) delete localApplyFailed["appearance.language"];
    })
    .catch(() => {
      if (appliedLanguage === target) localApplyFailed["appearance.language"] = "local_apply_failed";
    });
}

function applyPresentation(s: AppSettings) {
  applyTheme(s.appearance);
  applyTypography(s.typography);
  applyAccessibility(s.accessibility);
  applyLocale(s.appearance?.language);
}

/** Retries the frontend-only application (locale) of the confirmed snapshot. */
export function retryLocalPresentation(): void {
  if (!settings) return;
  appliedLanguage = null;
  applyPresentation(settings);
}

// ---------------------------------------------------------------------------
// Confirmed snapshot. Only revisions >= the confirmed one are published.
// ---------------------------------------------------------------------------

let settings = $state<AppSettings | null>(null);
let confirmedRevision: Revision | null = null;
let loadState = $state<SettingsLoadState>({ kind: "loading" });
let pendingEffects = $state<PendingEffect[]>([]);
let statuses = $state<Record<string, FieldStatus>>({});
let migrationState = $state<AppearanceMigrationState>("unknown");

type SnapshotListener = (reason: "publish" | "reset") => void;
const snapshotListeners = new Set<SnapshotListener>();

/** Internal hook for settings-drafts: called after a confirmed snapshot is published. */
export function onConfirmedSnapshot(listener: SnapshotListener): () => void {
  snapshotListeners.add(listener);
  return () => snapshotListeners.delete(listener);
}

function notify(reason: "publish" | "reset") {
  for (const listener of snapshotListeners) {
    try { listener(reason); } catch { /* A draft never breaks the store. */ }
  }
}

function publish(snapshot: ReadySnapshot): boolean {
  if (!snapshot || !snapshot.settings || !isRevision(snapshot.revision)) return false;
  if (confirmedRevision !== null && compareRevision(snapshot.revision, confirmedRevision) < 0) return false;
  confirmedRevision = snapshot.revision;
  settings = snapshot.settings;
  pendingEffects = Array.isArray(snapshot.pendingEffects) ? snapshot.pendingEffects : [];
  loadState = { kind: "ready", loadStatus: snapshot.loadStatus ?? { kind: "loaded" } };
  if ((snapshot.settings.appearance?.migration_version ?? 0) >= 1) migrationState = "done";
  applyPresentation(snapshot.settings);
  notify("publish");
  return true;
}

export function getSettings(): AppSettings | null {
  return settings;
}

/** The confirmed revision (decimal string), or null before the first read. */
export function getSettingsRevision(): Revision | null {
  return confirmedRevision;
}

export function getSettingsLoadState(): SettingsLoadState {
  return loadState;
}

export function getPendingEffects(): PendingEffect[] {
  return pendingEffects;
}

export function getAppearanceMigrationState(): AppearanceMigrationState {
  return migrationState;
}

class SettingsLoadError extends Error {
  constructor(readonly code: string) {
    super(`settings_${code}`);
  }
}

async function readSnapshot(): Promise<SettingsSnapshot> {
  return invoke<SettingsSnapshot>("settings_get_v3");
}

function acceptRead(snapshot: SettingsSnapshot): boolean {
  if (!snapshot || snapshot.settings === null) {
    const code = snapshot?.loadStatus?.kind === "failed" ? snapshot.loadStatus.code : "read_failed";
    // A failed read never replaces a confirmed snapshot with editable defaults.
    if (!settings) loadState = { kind: "failed", code };
    throw new SettingsLoadError(code);
  }
  return publish(snapshot);
}

/** Before a mutation: a fresh read, never joined with an older in-flight load. */
async function ensureRevision(): Promise<void> {
  if (confirmedRevision !== null) return;
  try {
    acceptRead(await readSnapshot());
  } catch (error) {
    if (!settings && !(error instanceof SettingsLoadError)) loadState = { kind: "failed", code: "unavailable" };
    throw error;
  }
}

// ---------------------------------------------------------------------------
// Appearance import (runs once; the backend marker is the only authority).
// ---------------------------------------------------------------------------

const WORKSPACE_RAW_MAX_BYTES = 65536;

function readWorkspaceAppearanceRaw(): string | null {
  try {
    const raw = localStorage.getItem(WORKSPACE_APPEARANCE_SOURCE);
    if (raw === null) return null;
    return new TextEncoder().encode(raw).length > WORKSPACE_RAW_MAX_BYTES ? null : raw;
  } catch {
    return null;
  }
}

function needsImport(): boolean {
  return !!settings && (settings.appearance?.migration_version ?? 0) < 1;
}

/** Must run inside the mutation queue. The legacy key is read, never removed. */
async function importAppearance(): Promise<void> {
  if (!needsImport()) return;
  migrationState = "importing";
  try {
    const result = await invoke<ImportAppearanceResult>("settings_import_appearance_v3", {
      request: { source: WORKSPACE_APPEARANCE_SOURCE, rawWorkspace: readWorkspaceAppearanceRaw() },
    });
    publish(result.snapshot);
    migrationState = needsImport() ? "failed" : "done";
  } catch {
    // Other settings stay usable; the next load tries again.
    migrationState = "failed";
  }
}

// ---------------------------------------------------------------------------
// Install locale: only for an absent profile whose language was never chosen.
// ---------------------------------------------------------------------------

const PRIMARY_LOCALES = new Set(["en", "pt", "es", "fr", "it", "ru", "el", "fa", "lo", "ja"]);

/** Maps an OS/browser language tag to one of the 12 catalogs; fallback "en". */
export function detectInstallLocale(tag: string | null | undefined): string {
  if (!tag) return "en";
  const parts = tag.trim().replace(/_/g, "-").toLowerCase().split("-").filter(Boolean);
  const primary = parts[0] ?? "";
  if (primary === "zh") {
    if (parts.includes("hant")) return "zh-TW";
    if (parts.includes("hans")) return "zh";
    return parts.some((part) => part === "tw" || part === "hk" || part === "mo") ? "zh-TW" : "zh";
  }
  return PRIMARY_LOCALES.has(primary) ? primary : "en";
}

/**
 * Presentation before the first confirmed read (or when it failed): OS light/dark on the
 * default preset. Never touches a confirmed snapshot and never persists anything.
 */
export function applyFallbackPresentation(): void {
  if (settings) return;
  applyTheme(undefined);
}

/** Interface language to use until the confirmed snapshot answers (06 locale rules). */
export function fallbackLocale(): string {
  return detectInstallLocale(systemLanguageTag());
}

function systemLanguageTag(): string | null {
  try {
    if (typeof navigator === "undefined") return null;
    return navigator.languages?.[0] ?? navigator.language ?? null;
  } catch {
    return null;
  }
}

async function applyInstallLocale(): Promise<void> {
  const current = settings?.appearance?.language;
  // The backend may already have detected it; an explicit or detected value wins.
  if (current && current !== "en") return;
  const detected = detectInstallLocale(systemLanguageTag());
  if (detected === "en") return;
  try { await updateSettings({ appearance: { language: detected } }); } catch { /* Stays "en". */ }
}

// ---------------------------------------------------------------------------
// Mutation queue: one serial chain per WebView; queued autosaves coalesce by field.
// ---------------------------------------------------------------------------

type Waiter = { resolve: (value: AppSettings) => void; reject: (error: unknown) => void };
type PatchTask = {
  kind: "patch";
  mode: "auto" | "form";
  patch: Patch;
  secretOperations?: SettingsSecretOperations;
  waiters: Waiter[];
};
type ResetTask = { kind: "reset"; scope: ResetScope; waiters: Waiter[] };
type ImportTask = { kind: "import"; waiters: Waiter[] };
type Task = PatchTask | ResetTask | ImportTask;

const queue: Task[] = [];
let running = false;
/** Autosave intents that failed, kept for an explicit retry (never secrets). */
const failedIntents = new Map<string, unknown>();

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function mergeInto(target: Patch, source: Patch) {
  for (const [key, value] of Object.entries(source)) {
    if (isPlainObject(value) && isPlainObject(target[key])) mergeInto(target[key] as Patch, value);
    else target[key] = isPlainObject(value) ? clonePatch(value) : value;
  }
}

function clonePatch(patch: Patch): Patch {
  const copy: Patch = {};
  mergeInto(copy, patch);
  return copy;
}

function leafPaths(patch: Patch, prefix = ""): Array<[string, unknown]> {
  const out: Array<[string, unknown]> = [];
  for (const [key, value] of Object.entries(patch)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (isPlainObject(value) && Object.keys(value).length > 0) out.push(...leafPaths(value, path));
    else out.push([path, value]);
  }
  return out;
}

function setStatus(paths: string[], status: FieldStatus) {
  for (const path of paths) statuses[path] = status;
}

function errorCode(error: unknown): string {
  if (isApiError(error)) return error.code;
  if (error instanceof SettingsLoadError) return error.code;
  return "save_failed";
}

function enqueue(task: Task): Promise<AppSettings> {
  return new Promise<AppSettings>((resolve, reject) => {
    const waiter = { resolve, reject };
    if (task.kind === "patch" && task.mode === "auto") {
      const last = queue[queue.length - 1];
      if (last && last.kind === "patch" && last.mode === "auto") {
        mergeInto(last.patch, task.patch);
        last.waiters.push(waiter);
        return;
      }
    }
    task.waiters.push(waiter);
    queue.push(task);
    void pump();
  });
}

async function pump() {
  if (running) return;
  running = true;
  try {
    while (queue.length > 0) await runTask(queue.shift()!);
  } finally {
    running = false;
  }
}

function settle(task: Task, error?: unknown) {
  for (const waiter of task.waiters) {
    if (error !== undefined) waiter.reject(error);
    else waiter.resolve(settings!);
  }
}

/** After conflict or an indeterminate write: re-read, keep the intent, never resend it. */
async function reconcile() {
  try {
    const snapshot = await readSnapshot();
    if (snapshot.settings) publish(snapshot);
  } catch { /* The error of the mutation is what the caller sees. */ }
}

const APPEARANCE_STYLE_KEYS = ["theme", "mode", "preset", "legacy_theme"];

async function runTask(task: Task) {
  if (task.kind === "import") {
    try {
      await ensureRevision();
      await importAppearance();
      settle(task);
    } catch (error) {
      settle(task, error);
    }
    return;
  }
  const paths = task.kind === "patch" ? leafPaths(task.patch).map(([path]) => path) : [];
  try {
    await ensureRevision();
    if (task.kind === "patch" && isPlainObject(task.patch.appearance)
      && APPEARANCE_STYLE_KEYS.some((key) => key in (task.patch.appearance as Patch)) && needsImport()) {
      // The backend refuses appearance edits until the import committed.
      await importAppearance();
    }
    const expectedRevision = confirmedRevision!;
    const snapshot = task.kind === "patch"
      ? await invoke<ReadySnapshot>("settings_patch_v3", {
          request: task.secretOperations
            ? { expectedRevision, patch: task.patch, secretOperations: task.secretOperations }
            : { expectedRevision, patch: task.patch },
        })
      : await invoke<ReadySnapshot>("settings_reset_v3", { request: { expectedRevision, scope: task.scope } });
    publish(snapshot);
    if (task.kind === "reset") {
      failedIntents.clear();
      statuses = {};
      notify("reset");
    } else {
      for (const path of paths) failedIntents.delete(path);
      setStatus(paths, { state: "saved" });
    }
    settle(task);
  } catch (error) {
    if (isApiError(error) && (error.kind === "conflict" || error.kind === "indeterminate")) await reconcile();
    if (task.kind === "patch") {
      if (task.mode === "auto") for (const [path, value] of leafPaths(task.patch)) failedIntents.set(path, value);
      setStatus(paths, { state: "failed", error: errorCode(error) });
    }
    settle(task, error);
  }
}

/** Legacy `appearance.theme` edits become mode/preset; the backend keeps `theme` coherent. */
function normalizeLegacyPatch(partial: Patch): Patch {
  const appearance = partial.appearance;
  if (!isPlainObject(appearance) || !("theme" in appearance)) return partial;
  const { theme, ...rest } = appearance;
  const next: Patch = { ...rest };
  if (typeof theme === "string") {
    if (MODES.has(theme)) {
      next.mode = theme;
      if (resolveAppearance(settings?.appearance).namedTheme !== null && !("preset" in rest)) next.preset = OMNIGET_PRESET;
    } else if (!("preset" in rest)) {
      next.preset = theme;
    }
  }
  return { ...partial, appearance: next };
}

/** Autosave of a toggle/select: minimal patch, queued by field, confirmed revision only. */
export async function updateSettings(partial: Record<string, unknown>): Promise<AppSettings> {
  const patch = clonePatch(normalizeLegacyPatch(partial));
  setStatus(leafPaths(patch).map(([path]) => path), { state: "saving" });
  return enqueue({ kind: "patch", mode: "auto", patch, waiters: [] });
}

/** Explicit Save of a form (proxy, fonts, hotkeys). Secrets only as write-only operations. */
export async function saveSettingsForm(
  patch: Record<string, unknown>,
  secretOperations?: SettingsSecretOperations,
): Promise<AppSettings> {
  const copy = clonePatch(normalizeLegacyPatch(patch));
  setStatus(leafPaths(copy).map(([path]) => path), { state: "saving" });
  return enqueue({ kind: "patch", mode: "form", patch: copy, secretOperations, waiters: [] });
}

/** Explicit retry of failed autosave intents under `prefix`, with the current revision. */
export async function retryFailedSettings(prefix: string): Promise<AppSettings | null> {
  const patch: Patch = {};
  for (const [path, value] of failedIntents) {
    if (path !== prefix && !path.startsWith(`${prefix}.`) && prefix !== "") continue;
    const keys = path.split(".");
    let node = patch;
    for (const key of keys.slice(0, -1)) node = (node[key] = isPlainObject(node[key]) ? node[key] : {}) as Patch;
    node[keys[keys.length - 1]] = value;
  }
  if (Object.keys(patch).length === 0) return null;
  return updateSettings(patch);
}

export async function resetSettings(scope: ResetScope = { kind: "app" }): Promise<AppSettings> {
  // Queued edits of the previous generation must not re-apply values after the reset.
  const superseded = new Error("settings_superseded");
  for (let index = queue.length - 1; index >= 0; index -= 1) {
    const task = queue[index];
    if (task.kind !== "patch") continue;
    queue.splice(index, 1);
    setStatus(leafPaths(task.patch).map(([path]) => path), { state: "idle" });
    settle(task, superseded);
  }
  return enqueue({ kind: "reset", scope, waiters: [] });
}

export async function retrySettingsEffect(effectId: EffectId): Promise<void> {
  const target = pendingEffects.find((effect) => effect.id === effectId)?.targetRevision ?? confirmedRevision;
  if (!target) throw new SettingsLoadError("unavailable");
  const result = await invoke<RetryEffectResult>("settings_retry_effect_v3", {
    request: { effectId, targetRevision: target },
  });
  publish(result.snapshot);
}

/** A first read still unanswered after this long is shown as a failure (with retry). */
export const SETTINGS_READ_TIMEOUT_MS = 15000;

/** Reads the confirmed document; a late read never rolls back a newer confirmation. */
export async function loadSettings(): Promise<AppSettings> {
  let snapshot: SettingsSnapshot;
  // The shell no longer waits for this read. If it hangs, Settings leaves the skeleton
  // for a real error state; an answer that arrives later still publishes normally.
  const slow = settings ? null : setTimeout(() => {
    if (!settings && loadState.kind === "loading") loadState = { kind: "failed", code: "timeout" };
  }, SETTINGS_READ_TIMEOUT_MS);
  try {
    snapshot = await readSnapshot();
  } catch (error) {
    // Transport failure is unavailability, never a new profile.
    if (!settings) loadState = { kind: "failed", code: "unavailable" };
    throw error;
  } finally {
    if (slow) clearTimeout(slow);
  }
  const published = acceptRead(snapshot);
  if (needsImport()) await enqueue({ kind: "import", waiters: [] }).catch(() => undefined);
  if (published && snapshot.loadStatus.kind === "absent") await applyInstallLocale();
  return settings!;
}

// ---------------------------------------------------------------------------
// Field status and cross-window sync.
// ---------------------------------------------------------------------------

const EFFECT_BY_PATH: Array<[RegExp, EffectId]> = [
  [/^proxy(\.|$)/, "network.proxy"],
  [/^advanced\.max_concurrent_segments$/, "downloads.segments"],
  [/^advanced\.(max_concurrent_downloads|stagger_delay_ms|max_retries|prevent_sleep)$/, "downloads.queue"],
  [/^download\.(hotkey|clip_hotkey|music_hotkey)_/, "shortcuts.registration"],
  [/^start_with_system$/, "app.autostart"],
  [/^rpc(\.|$)/, "integrations.rpc"],
  [/^llm\.prune_/, "llm.pruning"],
  [/^llm\.(turn_|ask_timeout_|approval_notify_delay_s|job_default_deadline_min)/, "llm.watchdog"],
  [/^llm\.jobs_/, "llm.jobs"],
  [/^bridge\.enabled$/, "bridge.lifecycle"],
];

function effectForPath(path: string): EffectId | null {
  for (const [pattern, id] of EFFECT_BY_PATH) if (pattern.test(path)) return id;
  return null;
}

const STATE_WEIGHT: Record<FieldState, number> = { idle: 0, saved: 1, pendingApply: 2, saving: 3, failed: 4 };

/** Status of a field, or the most urgent status under a prefix (e.g. "proxy"). */
export function getFieldStatus(path: string): FieldStatus {
  let found: FieldStatus = { state: "idle" };
  for (const [key, status] of Object.entries(statuses)) {
    if (key !== path && !key.startsWith(`${path}.`)) continue;
    if (STATE_WEIGHT[status.state] > STATE_WEIGHT[found.state]) found = status;
  }
  if (found.state === "saving" || found.state === "failed") return found;
  for (const [key, code] of Object.entries(localApplyFailed)) {
    if (key === path || key.startsWith(`${path}.`)) return { state: "pendingApply", error: code };
  }
  const effectId = effectForPath(path);
  const effect = effectId ? pendingEffects.find((item) => item.id === effectId) : undefined;
  if (effect && (effect.state === "failed" || effect.state === "restart_required")) {
    return { state: "pendingApply", effectId: effect.id, ...(effect.code ? { error: effect.code } : {}) };
  }
  return found;
}

let refreshing: Promise<void> | null = null;
let refreshAgain = false;

function scheduleRefresh() {
  if (refreshing) { refreshAgain = true; return; }
  refreshing = (async () => {
    do {
      refreshAgain = false;
      try {
        const snapshot = await readSnapshot();
        if (snapshot.settings) publish(snapshot);
      } catch { /* The next event or reopening reconciles. */ }
    } while (refreshAgain);
  })().finally(() => { refreshing = null; });
}

/** Handles a `settings-v3-changed` payload: coalesced re-read, stale revisions ignored. */
export function handleSettingsChanged(payload: { revision?: unknown } | null | undefined): void {
  const revision = payload?.revision;
  if (!isRevision(revision)) return;
  if (confirmedRevision !== null && compareRevision(revision, confirmedRevision) < 0) return;
  scheduleRefresh();
}

/** Subscribes to other writers (windows, backend) and returns the cleanup. */
export function startSettingsSync(): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  listen<{ revision: Revision }>("settings-v3-changed", (event) => handleSettingsChanged(event.payload))
    .then((stop) => { if (disposed) stop(); else unlisten = stop; })
    .catch(() => { /* No event bridge (browser preview); commands still answer. */ });
  return () => {
    disposed = true;
    unlisten?.();
    syncSystemListener(false);
  };
}
