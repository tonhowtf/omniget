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
