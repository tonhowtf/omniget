import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "./settings-store.svelte";
import type { ApiError, PendingEffect, ReadySnapshot, SettingsSnapshot } from "./settings-v3-types";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

type Store = typeof import("./settings-store.svelte");
type Drafts = typeof import("./settings-drafts.svelte");
type Types = typeof import("./settings-v3-types");
let store: Store;
let drafts: Drafts;
let types: Types;
let attributes: Map<string, string>;
let classes: Set<string>;
let inlineStyle: Map<string, string>;
let media: ReturnType<typeof mediaQuery>;
let storage: Map<string, string>;
let localStorageStub: { getItem: ReturnType<typeof vi.fn>; setItem: ReturnType<typeof vi.fn>; removeItem: ReturnType<typeof vi.fn> };

const WORKSPACE_KEY = "omniget.workspace.appearance.v1";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

function mediaQuery() {
  const callbacks = new Set<(event: { matches: boolean }) => void>();
  return {
    matches: false,
    callbacks,
    addEventListener: vi.fn((_event: string, callback: (event: { matches: boolean }) => void) => callbacks.add(callback)),
    removeEventListener: vi.fn((_event: string, callback: (event: { matches: boolean }) => void) => callbacks.delete(callback)),
    change(matches: boolean) {
      this.matches = matches;
      for (const callback of callbacks) callback({ matches });
    },
  };
}

/** Synthetic v3 document; only fields read by the store. */
function settings(overrides: { mode?: "system" | "light" | "dark"; preset?: string; migration?: number; language?: string; theme?: string; proxyHost?: string; proxyPort?: number } = {}): AppSettings {
  const mode = overrides.mode ?? "light";
  return {
    schema_version: 3,
    appearance: {
      theme: overrides.theme ?? mode,
      mode,
      preset: overrides.preset ?? "omniget-desktop",
      legacy_theme: null,
      migration_version: overrides.migration ?? 1,
      language: overrides.language ?? "en",
    },
    proxy: { enabled: true, host: overrides.proxyHost ?? "proxy.invalid", port: overrides.proxyPort ?? 8080, username: "", has_password: false },
    typography: { font_display: "system-ui", font_body: "system-ui", font_mono: "ui-monospace", line_height_base: 1.55, spacing_scale: 1, preset_name: "system" },
  } as AppSettings;
}

function snapshot(value: AppSettings, revision = "0", extra: Partial<ReadySnapshot> = {}): ReadySnapshot {
  return { settings: value, revision, loadStatus: { kind: "loaded" }, applied: true, pendingEffects: [], fieldErrors: [], ...extra };
}

/** Routes IPC by command; any command not listed fails loudly. */
function route(handlers: Record<string, (args: any) => unknown>) {
  invoke.mockImplementation((command: string, args: unknown) => {
    const handler = handlers[command];
    if (!handler) return Promise.reject(new Error(`Unexpected IPC: ${command}`));
    return Promise.resolve().then(() => handler(args));
  });
}

function calls(command: string) {
  return invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
}

beforeEach(async () => {
  vi.resetModules();
  invoke.mockReset();
  listen.mockReset();
  attributes = new Map();
  classes = new Set();
  inlineStyle = new Map();
  storage = new Map();
  media = mediaQuery();
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  vi.stubGlobal("document", {
    documentElement: {
      setAttribute: (name: string, value: string) => attributes.set(name, value),
      getAttribute: (name: string) => attributes.get(name) ?? null,
      removeAttribute: (name: string) => attributes.delete(name),
      classList: { add: (name: string) => classes.add(name), remove: (name: string) => classes.delete(name) },
      style: {
        setProperty: (name: string, value: string) => inlineStyle.set(name, value),
        removeProperty: (name: string) => inlineStyle.delete(name),
      },
    },
  });
  vi.stubGlobal("window", { matchMedia: vi.fn(() => media) });
  localStorageStub = {
    getItem: vi.fn((key: string) => storage.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => storage.set(key, value)),
    removeItem: vi.fn((key: string) => storage.delete(key)),
  };
  vi.stubGlobal("localStorage", localStorageStub);
  vi.stubGlobal("navigator", { languages: ["en-US"], language: "en-US" });
  store = await import("./settings-store.svelte");
  drafts = await import("./settings-drafts.svelte");
  types = await import("./settings-v3-types");
});

afterEach(() => {
  drafts.clearSettingsDrafts();
  vi.unstubAllGlobals();
});

describe("revisions", () => {
  it("validates canonical u64 decimals and compares beyond Number precision", () => {
    for (const ok of ["0", "9", "10", "9007199254740993", "18446744073709551615"]) expect(types.isRevision(ok)).toBe(true);
    for (const bad of ["", "01", "-1", "1.0", "18446744073709551616", " 1", 1 as unknown as string]) expect(types.isRevision(bad)).toBe(false);
    expect(Number("9007199254740993") === Number("9007199254740992")).toBe(true);
    expect(types.compareRevision("9007199254740993", "9007199254740992")).toBe(1);
  });

  it("discards a read whose revision is lower only beyond 2^53", async () => {
    route({ settings_get_v3: () => snapshot(settings({ mode: "dark" }), "9007199254740993") });
    await store.loadSettings();
    route({ settings_get_v3: () => snapshot(settings({ mode: "light" }), "9007199254740992") });
    await store.loadSettings();
    expect(store.getSettingsRevision()).toBe("9007199254740993");
    expect(attributes.get("data-theme")).toBe("dark");
    route({ settings_get_v3: () => snapshot(settings({ mode: "light" }), "9007199254740994") });
    await store.loadSettings();
    expect(attributes.get("data-theme")).toBe("light");
  });

  it("an equal revision refreshes pending effects without dropping a dirty draft", async () => {
    route({ settings_get_v3: () => snapshot(settings(), "3") });
    await store.loadSettings();
    const draft = drafts.createSettingsDraft({ id: "proxy", read: (s) => ({ host: s.proxy?.host ?? "" }), toPatch: (d) => ({ proxy: { host: d.host } }) });
    (draft.value as { host: string }).host = "typed.invalid";
    const effect: PendingEffect = { id: "network.proxy", targetRevision: "3", state: "failed", code: "apply_failed", retryable: true };
    route({ settings_get_v3: () => snapshot(settings(), "3", { applied: false, pendingEffects: [effect] }) });
    await store.loadSettings();
    expect(store.getPendingEffects()).toEqual([effect]);
    expect(draft.value).toEqual({ host: "typed.invalid" });
    expect(draft.dirty).toBe(true);
  });
});

describe("appearance import", () => {
  const legacy = (migration = 0) => settings({ migration, theme: "dark", mode: "system" });

  it("absent key: imports with rawWorkspace null and waits for the response", async () => {
    const response = deferred<unknown>();
    route({
      settings_get_v3: () => snapshot(legacy(), "0"),
      settings_import_appearance_v3: () => response.promise,
    });
    const loading = store.loadSettings();
    await vi.waitFor(() => expect(calls("settings_import_appearance_v3")).toHaveLength(1));
    expect(calls("settings_import_appearance_v3")[0]).toEqual({ request: { source: WORKSPACE_KEY, rawWorkspace: null } });
    expect(store.getAppearanceMigrationState()).toBe("importing");
    response.resolve({ outcome: "imported", snapshot: snapshot(settings({ mode: "dark" }), "1") });
    await loading;
    expect(store.getAppearanceMigrationState()).toBe("done");
    expect(attributes.get("data-theme")).toBe("dark");
  });

  it("valid key: sends it verbatim, applies the imported preset and keeps the key", async () => {
    const raw = JSON.stringify({ preset: "claude-amber", mode: "dark" });
    storage.set(WORKSPACE_KEY, raw);
    route({
      settings_get_v3: () => snapshot(legacy(), "0"),
      settings_import_appearance_v3: () => ({ outcome: "imported", snapshot: snapshot(settings({ mode: "dark", preset: "claude-amber" }), "1") }),
    });
    await store.loadSettings();
    expect(calls("settings_import_appearance_v3")[0]).toEqual({ request: { source: WORKSPACE_KEY, rawWorkspace: raw } });
    expect(attributes.get("data-theme")).toBe("dark");
    expect(attributes.get("data-preset")).toBe("claude-amber");
    expect(attributes.get("data-ds-preset")).toBe("claude-amber");
    expect(attributes.get("data-ds-mode")).toBe("dark");
    expect(classes.has("ds-scope")).toBe(true);
    expect(localStorageStub.removeItem).not.toHaveBeenCalled();
    expect(storage.get(WORKSPACE_KEY)).toBe(raw);
  });

  it("corrupted key is sent as text for the backup; oversized is treated as absent", async () => {
    storage.set(WORKSPACE_KEY, "{not json");
    route({
      settings_get_v3: () => snapshot(legacy(), "0"),
      settings_import_appearance_v3: () => ({ outcome: "imported", snapshot: snapshot(settings(), "1") }),
    });
    await store.loadSettings();
    expect(calls("settings_import_appearance_v3")[0]).toEqual({ request: { source: WORKSPACE_KEY, rawWorkspace: "{not json" } });

    vi.resetModules();
    store = await import("./settings-store.svelte");
    invoke.mockClear();
    storage.set(WORKSPACE_KEY, "x".repeat(65537));
    route({
      settings_get_v3: () => snapshot(legacy(), "0"),
      settings_import_appearance_v3: () => ({ outcome: "imported", snapshot: snapshot(settings(), "1") }),
    });
    await store.loadSettings();
    expect(calls("settings_import_appearance_v3")[0]).toEqual({ request: { source: WORKSPACE_KEY, rawWorkspace: null } });
  });

  it("a failed import keeps other settings usable, keeps the key and retries on the next load", async () => {
    storage.set(WORKSPACE_KEY, JSON.stringify({ preset: "orange-soft", mode: "light" }));
    let attempt = 0;
    route({
      settings_get_v3: () => snapshot(legacy(), "0"),
      settings_import_appearance_v3: () => {
        attempt += 1;
        if (attempt === 1) throw { kind: "unavailable", code: "backup_failed", fieldErrors: [] } satisfies ApiError;
        return { outcome: "imported", snapshot: snapshot(settings({ preset: "orange-soft" }), "1") };
      },
    });
    const loaded = await store.loadSettings();
    expect(loaded.appearance.language).toBe("en");
    expect(store.getAppearanceMigrationState()).toBe("failed");
    expect(store.getSettingsLoadState()).toEqual({ kind: "ready", loadStatus: { kind: "loaded" } });
    expect(storage.has(WORKSPACE_KEY)).toBe(true);
    await store.loadSettings();
    expect(calls("settings_import_appearance_v3")).toHaveLength(2);
    expect(store.getAppearanceMigrationState()).toBe("done");
  });

  it("repeated boots with the marker never read the legacy key again", async () => {
    storage.set(WORKSPACE_KEY, JSON.stringify({ preset: "amber-slate", mode: "dark" }));
    route({ settings_get_v3: () => snapshot(settings(), "7") });
    await store.loadSettings();
    await store.loadSettings();
    expect(calls("settings_import_appearance_v3")).toHaveLength(0);
    expect(localStorageStub.getItem).not.toHaveBeenCalled();
  });

  it("already_imported publishes the current state and is not retried", async () => {
    route({
      settings_get_v3: () => snapshot(legacy(), "4"),
      settings_import_appearance_v3: () => ({ outcome: "already_imported", snapshot: snapshot(settings({ mode: "dark" }), "5") }),
    });
    await store.loadSettings();
    expect(store.getAppearanceMigrationState()).toBe("done");
    expect(store.getSettingsRevision()).toBe("5");
  });

  it("an appearance edit before migration imports first, then patches with the new revision", async () => {
    route({
      settings_get_v3: () => snapshot(legacy(), "0"),
      settings_import_appearance_v3: () => ({ outcome: "imported", snapshot: snapshot(settings(), "1") }),
      settings_patch_v3: () => snapshot(settings({ mode: "dark" }), "2"),
    });
    await store.updateSettings({ appearance: { mode: "dark" } });
    const order = invoke.mock.calls.map(([name]) => name);
    expect(order.indexOf("settings_import_appearance_v3")).toBeLessThan(order.indexOf("settings_patch_v3"));
    expect(calls("settings_patch_v3")[0]).toEqual({ request: { expectedRevision: "1", patch: { appearance: { mode: "dark" } } } });
  });
});

describe("system appearance and presentation", () => {
  it("follows the OS only while in system mode and removes the listener otherwise", async () => {
    route({ settings_get_v3: () => snapshot(settings({ mode: "system" }), "1") });
    await store.loadSettings();
    expect(attributes.get("data-theme")).toBe("light");
    media.change(true);
    expect(attributes.get("data-theme")).toBe("dark");
    expect(store.getAppearancePresentation().resolvedMode).toBe("dark");
    route({ settings_patch_v3: () => snapshot(settings({ mode: "light" }), "2") });
    await store.updateSettings({ appearance: { mode: "light" } });
    expect(media.removeEventListener).toHaveBeenCalled();
    media.change(true);
    expect(attributes.get("data-theme")).toBe("light");
  });

  it("the sync cleanup also removes the system listener", async () => {
    listen.mockResolvedValue(() => {});
    const stop = store.startSettingsSync();
    route({ settings_get_v3: () => snapshot(settings({ mode: "system" }), "1") });
    await store.loadSettings();
    expect(media.callbacks.size).toBe(1);
    stop();
    expect(media.callbacks.size).toBe(0);
  });

  it("named themes set data-theme to their id; legacy theme patches become mode/preset", async () => {
    route({ settings_get_v3: () => snapshot(settings({ preset: "dracula", theme: "dracula", mode: "dark" }), "1") });
    await store.loadSettings();
    expect(attributes.get("data-theme")).toBe("dracula");
    expect(attributes.has("data-preset")).toBe(false);
    route({ settings_patch_v3: () => snapshot(settings({ mode: "light" }), "2") });
    await store.updateSettings({ appearance: { theme: "light" } });
    expect(calls("settings_patch_v3")[0]).toEqual({ request: { expectedRevision: "1", patch: { appearance: { mode: "light", preset: "omniget-desktop" } } } });
    route({ settings_patch_v3: () => snapshot(settings({ preset: "eink-day", theme: "eink-day" }), "3") });
    await store.updateSettings({ appearance: { theme: "eink-day" } });
    expect(calls("settings_patch_v3").at(-1)).toEqual({ request: { expectedRevision: "2", patch: { appearance: { preset: "eink-day" } } } });
    expect(attributes.get("data-theme")).toBe("eink-day");
  });

  it("font keywords stay unquoted, custom families stay quoted and Lao stays in the fallback", async () => {
    expect(store.resolveFontStack("body", "system-ui")).toMatch(/^system-ui,/);
    expect(store.resolveFontStack("mono", "ui-monospace")).toMatch(/^ui-monospace,/);
    expect(store.resolveFontStack("mono", "ui-monospace")).not.toContain("'ui-monospace'");
    expect(store.resolveFontStack("display", "My Font")).toMatch(/^'My Font',/);
    expect(store.resolveFontStack("mono", "Fira Code")).toMatch(/^'Fira Code', ui-monospace/);
    route({ settings_get_v3: () => snapshot(settings(), "1") });
    await store.loadSettings();
    expect(inlineStyle.get("--font-body")).toMatch(/^'OmniGet Lao', system-ui,/);
    expect(inlineStyle.get("--font-mono")).toMatch(/^ui-monospace,/);
  });

  it("reduce motion from the preference is exposed on the root", async () => {
    route({ settings_get_v3: () => snapshot({ ...settings(), accessibility: { reduce_motion: true } } as AppSettings, "1") });
    await store.loadSettings();
    expect(attributes.get("data-reduce-motion")).toBe("true");
  });
});

describe("locale", () => {
  it("reset applies the restored language through the locale applier", async () => {
    const applied: string[] = [];
    store.setSettingsLocaleApplier((language) => { applied.push(language); });
    route({ settings_get_v3: () => snapshot(settings({ language: "pt" }), "4") });
    await store.loadSettings();
    route({ settings_reset_v3: () => snapshot(settings({ language: "en" }), "5") });
    await store.resetSettings({ kind: "section", section: "general" });
    expect(calls("settings_reset_v3")[0]).toEqual({ request: { expectedRevision: "4", scope: { kind: "section", section: "general" } } });
    await vi.waitFor(() => expect(applied).toEqual(["pt", "en"]));
  });

  it("an applier registered after the first read still applies the confirmed language", async () => {
    route({ settings_get_v3: () => snapshot(settings({ language: "fa" }), "1") });
    await store.loadSettings();
    const applied: string[] = [];
    store.setSettingsLocaleApplier((language) => { applied.push(language); });
    await vi.waitFor(() => expect(applied).toEqual(["fa"]));
  });

  it("before the first read: OS language and OS light/dark, nothing persisted", () => {
    vi.stubGlobal("navigator", { languages: ["pt-BR"], language: "pt-BR" });
    media.matches = false;
    store.applyFallbackPresentation();
    expect(store.fallbackLocale()).toBe("pt");
    expect(attributes.get("data-theme")).toBe("light");
    media.change(true);
    expect(attributes.get("data-theme")).toBe("dark");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("maps OS tags to the 12 catalogs", () => {
    const cases: Array<[string, string]> = [
      ["zh-TW", "zh-TW"], ["zh_HK", "zh-TW"], ["zh-MO", "zh-TW"], ["zh-Hant", "zh-TW"], ["zh-Hant-CN", "zh-TW"],
      ["zh-CN", "zh"], ["zh", "zh"], ["zh-Hans-HK", "zh"], ["pt-BR", "pt"], ["pt_PT", "pt"], ["fa-IR", "fa"],
      ["lo-LA", "lo"], ["ja-JP", "ja"], ["el", "el"], ["de-DE", "en"], ["", "en"],
    ];
    for (const [tag, expected] of cases) expect([tag, store.detectInstallLocale(tag)]).toEqual([tag, expected]);
  });

  it("new install without a chosen language saves the detected locale once", async () => {
    vi.stubGlobal("navigator", { languages: ["zh-HK"], language: "zh-HK" });
    route({
      settings_get_v3: () => ({ ...snapshot(settings(), "1"), loadStatus: { kind: "absent" } }),
      settings_patch_v3: () => snapshot(settings({ language: "zh-TW" }), "2"),
    });
    const loaded = await store.loadSettings();
    expect(calls("settings_patch_v3")).toEqual([{ request: { expectedRevision: "1", patch: { appearance: { language: "zh-TW" } } } }]);
    expect(loaded.appearance.language).toBe("zh-TW");
  });

  it("does not touch a language the backend already set or an existing profile", async () => {
    vi.stubGlobal("navigator", { languages: ["ja-JP"], language: "ja-JP" });
    route({ settings_get_v3: () => ({ ...snapshot(settings({ language: "fr" }), "1"), loadStatus: { kind: "absent" } }) });
    await store.loadSettings();
    route({ settings_get_v3: () => snapshot(settings(), "1") });
    await store.loadSettings();
    expect(calls("settings_patch_v3")).toHaveLength(0);
  });
});

describe("first read", () => {
  it("a hung read becomes a real failure; a late answer still publishes", async () => {
    vi.useFakeTimers();
    try {
      const read = deferred<SettingsSnapshot>();
      invoke.mockImplementation(() => read.promise);
      const pending = store.loadSettings();
      expect(store.getSettingsLoadState()).toEqual({ kind: "loading" });
      await vi.advanceTimersByTimeAsync(store.SETTINGS_READ_TIMEOUT_MS);
      expect(store.getSettingsLoadState()).toEqual({ kind: "failed", code: "timeout" });
      expect(store.getSettings()).toBeNull();
      read.resolve(snapshot(settings(), "1"));
      await pending;
      expect(store.getSettingsLoadState()).toEqual({ kind: "ready", loadStatus: { kind: "loaded" } });
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("mutation queue", () => {
  it("two quick edits behind an in-flight save coalesce into one minimal patch with the confirmed revision", async () => {
    route({ settings_get_v3: () => snapshot(settings(), "1") });
    await store.loadSettings();
    const first = deferred<ReadySnapshot>();
    const patches: unknown[] = [];
    invoke.mockImplementation((command: string, args: { request: unknown }) => {
      if (command !== "settings_patch_v3") return Promise.reject(new Error(command));
      patches.push(args.request);
      return patches.length === 1 ? first.promise : Promise.resolve(snapshot(settings({ mode: "dark" }), "3"));
    });
    const a = store.updateSettings({ start_minimized: true });
    const b = store.updateSettings({ appearance: { mode: "dark" } });
    const c = store.updateSettings({ exit_on_close: true });
    expect(store.getFieldStatus("appearance.mode").state).toBe("saving");
    first.resolve(snapshot(settings(), "2"));
    await Promise.all([a, b, c]);
    expect(patches).toEqual([
      { expectedRevision: "1", patch: { start_minimized: true } },
      { expectedRevision: "2", patch: { appearance: { mode: "dark" }, exit_on_close: true } },
    ]);
    expect(store.getFieldStatus("appearance.mode").state).toBe("saved");
    expect(store.getFieldStatus("start_minimized").state).toBe("saved");
  });

  it("rejection keeps the snapshot, marks the field failed, and an explicit retry uses the current revision", async () => {
    route({ settings_get_v3: () => snapshot(settings(), "1") });
    await store.loadSettings();
    const invalid: ApiError = { kind: "validation", code: "invalid_request", fieldErrors: [{ path: "proxy.port", code: "out_of_range" }] };
    route({ settings_patch_v3: () => { throw invalid; } });
    await expect(store.updateSettings({ proxy: { port: 8081 } })).rejects.toEqual(invalid);
    expect(store.getSettings()?.proxy?.port).toBe(8080);
    expect(store.getFieldStatus("proxy.port")).toEqual({ state: "failed", error: "invalid_request" });
    expect(store.getFieldStatus("proxy").state).toBe("failed");
    route({ settings_patch_v3: () => snapshot(settings({ proxyPort: 8081 }), "2") });
    await store.retryFailedSettings("proxy");
    expect(calls("settings_patch_v3")[0]).toEqual({ request: { expectedRevision: "1", patch: { proxy: { port: 8081 } } } });
    expect(store.getFieldStatus("proxy.port").state).toBe("saved");
  });

  it("conflict re-reads, keeps the intent and never resends it (secrets included)", async () => {
    route({ settings_get_v3: () => snapshot(settings(), "1") });
    await store.loadSettings();
    const conflict: ApiError = { kind: "conflict", code: "revision_conflict", currentRevision: "2", fieldErrors: [] };
    route({
      settings_patch_v3: () => { throw conflict; },
      settings_get_v3: () => snapshot(settings({ proxyHost: "other.invalid" }), "2"),
    });
    const draft = drafts.createSettingsDraft({
      id: "proxy",
      read: (s) => ({ host: s.proxy?.host ?? "", password: "" }),
      toPatch: (d) => ({ proxy: { host: d.host } }),
      secretOperations: (d) => (d.password ? { proxyPassword: { op: "set", value: d.password } } : undefined),
    });
    Object.assign(draft.value, { host: "mine.invalid", password: "synthetic-secret" });
    expect(await draft.save()).toBe(false);
    expect(calls("settings_patch_v3")).toHaveLength(1);
    expect(calls("settings_patch_v3")[0]).toEqual({ request: {
      expectedRevision: "1", patch: { proxy: { host: "mine.invalid" } }, secretOperations: { proxyPassword: { op: "set", value: "synthetic-secret" } },
    } });
    expect(store.getSettingsRevision()).toBe("2");
    expect(draft.error).toBe("revision_conflict");
    expect(draft.value).toEqual({ host: "mine.invalid", password: "synthetic-secret" });
    expect(draft.dirty).toBe(true);
    await Promise.resolve();
    expect(calls("settings_patch_v3")).toHaveLength(1);
  });

  it("reset drops queued edits of the previous generation", async () => {
    route({ settings_get_v3: () => snapshot(settings(), "1") });
    await store.loadSettings();
    const inFlight = deferred<ReadySnapshot>();
    invoke.mockImplementation((command: string) => {
      if (command === "settings_patch_v3") return inFlight.promise;
      if (command === "settings_reset_v3") return Promise.resolve(snapshot(settings(), "3"));
      return Promise.reject(new Error(command));
    });
    const running = store.updateSettings({ start_minimized: true });
    const queued = store.updateSettings({ exit_on_close: true });
    const reset = store.resetSettings();
    await expect(queued).rejects.toThrow("settings_superseded");
    inFlight.resolve(snapshot(settings(), "2"));
    await running;
    await reset;
    expect(invoke.mock.calls.map(([name]) => name)).toEqual(["settings_get_v3", "settings_patch_v3", "settings_reset_v3"]);
    expect(calls("settings_reset_v3")[0]).toEqual({ request: { expectedRevision: "2", scope: { kind: "app" } } });
    expect(store.getFieldStatus("exit_on_close").state).toBe("idle");
  });

  it("a failed read keeps an unloaded store in the failed state and does not invent defaults", async () => {
    const failed: SettingsSnapshot = { settings: null, revision: null, loadStatus: { kind: "failed", code: "invalid_document", recoveryId: null }, applied: false, pendingEffects: [], fieldErrors: [] };
    route({ settings_get_v3: () => failed });
    await expect(store.loadSettings()).rejects.toThrow("settings_invalid_document");
    expect(store.getSettings()).toBeNull();
    expect(store.getSettingsLoadState()).toEqual({ kind: "failed", code: "invalid_document" });
  });
});

describe("effects and sync", () => {
  it("a failed backend effect shows pendingApply and retries against its target revision", async () => {
    const effect: PendingEffect = { id: "app.autostart", targetRevision: "4", state: "failed", code: "apply_failed", retryable: true };
    route({ settings_get_v3: () => snapshot(settings(), "4", { applied: false, pendingEffects: [effect] }) });
    await store.loadSettings();
    expect(store.getFieldStatus("start_with_system")).toEqual({ state: "pendingApply", effectId: "app.autostart", error: "apply_failed" });
    route({ settings_retry_effect_v3: () => ({ outcome: "applied", snapshot: snapshot(settings(), "4") }) });
    await store.retrySettingsEffect("app.autostart");
    expect(calls("settings_retry_effect_v3")[0]).toEqual({ request: { effectId: "app.autostart", targetRevision: "4" } });
    expect(store.getFieldStatus("start_with_system").state).toBe("idle");
  });

  it("settings-v3-changed coalesces reads, ignores stale revisions and unsubscribes", async () => {
    let handler: ((event: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn();
    listen.mockImplementation((_name: string, callback: (event: { payload: unknown }) => void) => { handler = callback; return Promise.resolve(unlisten); });
    route({ settings_get_v3: () => snapshot(settings(), "5") });
    await store.loadSettings();
    const stop = store.startSettingsSync();
    await vi.waitFor(() => expect(handler).toBeDefined());
    expect(listen).toHaveBeenCalledWith("settings-v3-changed", expect.any(Function));
    invoke.mockClear();
    const read = deferred<ReadySnapshot>();
    invoke.mockImplementation(() => read.promise);
    handler!({ payload: { revision: "4" } });
    expect(invoke).not.toHaveBeenCalled();
    handler!({ payload: { revision: "6" } });
    handler!({ payload: { revision: "7" } });
    handler!({ payload: { revision: "8" } });
    read.resolve(snapshot(settings({ mode: "dark" }), "8"));
    await vi.waitFor(() => expect(store.getSettingsRevision()).toBe("8"));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    await new Promise((done) => setTimeout(done, 0));
    expect(invoke).toHaveBeenCalledTimes(2);
    stop();
    expect(unlisten).toHaveBeenCalled();
  });
});

describe("drafts", () => {
  beforeEach(async () => {
    route({ settings_get_v3: () => snapshot(settings(), "1") });
    await store.loadSettings();
  });

  it("keeps the draft across remounts by id, discards to the confirmed snapshot", () => {
    const options = { id: "typography", read: (s: AppSettings) => ({ body: s.typography?.font_body ?? "" }), toPatch: (d: { body: string }) => ({ typography: { font_body: d.body } }) };
    const draft = drafts.createSettingsDraft(options);
    expect(draft.dirty).toBe(false);
    draft.value.body = "My Font";
    expect(draft.dirty).toBe(true);
    const again = drafts.createSettingsDraft(options);
    expect(again).toBe(draft);
    expect(again.value.body).toBe("My Font");
    again.discard();
    expect(draft.value.body).toBe("system-ui");
    expect(draft.dirty).toBe(false);
  });

  it("an error preserves the draft; a later success resyncs with the confirmed snapshot", async () => {
    const draft = drafts.createSettingsDraft({ id: "proxy", read: (s) => ({ host: s.proxy?.host ?? "" }), toPatch: (d) => ({ proxy: { host: d.host } }) });
    draft.value.host = "new.invalid";
    route({ settings_patch_v3: () => { throw { kind: "persistence", code: "write_failed", committed: false, fieldErrors: [] } satisfies ApiError; } });
    expect(await draft.save()).toBe(false);
    expect(draft.error).toBe("write_failed");
    expect(draft.value.host).toBe("new.invalid");
    expect(store.getSettings()?.proxy?.host).toBe("proxy.invalid");
    route({ settings_patch_v3: () => snapshot(settings({ proxyHost: "new.invalid" }), "2") });
    expect(await draft.save()).toBe(true);
    expect(draft.error).toBeNull();
    expect(draft.dirty).toBe(false);
  });

  it("a clean draft follows confirmed changes from other writers; a reset discards dirty drafts", async () => {
    const draft = drafts.createSettingsDraft({ id: "proxy", read: (s) => ({ host: s.proxy?.host ?? "" }), toPatch: (d) => ({ proxy: { host: d.host } }) });
    route({ settings_get_v3: () => snapshot(settings({ proxyHost: "b.invalid" }), "2") });
    await store.loadSettings();
    expect(draft.value.host).toBe("b.invalid");
    draft.value.host = "typed.invalid";
    route({ settings_reset_v3: () => snapshot(settings({ proxyHost: "" }), "3") });
    await store.resetSettings({ kind: "section", section: "advanced" });
    expect(draft.value.host).toBe("");
    expect(draft.dirty).toBe(false);
  });
});
