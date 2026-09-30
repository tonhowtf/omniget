import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, TypographySettings } from "./settings-store.svelte";
import type { ReadySnapshot } from "./settings-v3-types";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

type SettingsStore = typeof import("./settings-store.svelte");
type LlmStore = typeof import("./llm-store.svelte");
let settingsStore: SettingsStore;
let llmStore: LlmStore | undefined;
let attributes: Map<string, string>;
let inlineStyle: Map<string, string>;
let media: ReturnType<typeof mediaQuery>;

/** Controlled IPC completion; no timers, Tauri process, provider, or personal profile. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function mediaQuery() {
  const callbacks = new Set<(event: { matches: boolean }) => void>();
  return {
    matches: false,
    media: "(prefers-color-scheme: dark)",
    addEventListener: vi.fn((_event: string, callback: (event: { matches: boolean }) => void) => callbacks.add(callback)),
    removeEventListener: vi.fn((_event: string, callback: (event: { matches: boolean }) => void) => callbacks.delete(callback)),
    change(matches: boolean) {
      this.matches = matches;
      for (const callback of callbacks) callback({ matches });
    },
  };
}

const typography: TypographySettings = {
  font_display: "System", font_body: "System", font_mono: "IBM Plex Mono",
  line_height_base: 1.55, spacing_scale: 1, preset_name: "system",
};

/** Minimal v3 fixture for the fields consumed by this store; no application data. */
function settings(mode = "light", scale = 1): AppSettings {
  return {
    schema_version: 3,
    appearance: { theme: mode, mode, preset: "omniget-desktop", legacy_theme: null, migration_version: 1, language: "en" },
    typography: { ...typography, spacing_scale: scale },
  } as AppSettings;
}

/** settings_get_v3/settings_patch_v3 envelope around the fixture. */
function snapshot(value: AppSettings, revision = "0"): ReadySnapshot {
  return { settings: value, revision, loadStatus: { kind: "loaded" }, applied: true, pendingEffects: [], fieldErrors: [] };
}

// Read the actual shipped defaults so this audit does not duplicate its own
// invented spacing scale. The fake CSSStyleDeclaration observes inline overrides;
// unoverridden values resolve to the stylesheet's :root values below.
const baseCss = readFileSync(new URL("../../app.css", import.meta.url), "utf8");
const baseSpacing = new Map(
  [...baseCss.matchAll(/(--space-[1-9]):\s*([\d.]+)px;/g)]
    .map((match) => [match[1], Number(match[2])] as const),
);
function currentSpacing(): Map<string, number> {
  return new Map([...baseSpacing].map(([key, fallback]) => [key, Number.parseFloat(inlineStyle.get(key) ?? String(fallback))]));
}
async function loadSnapshot(value: AppSettings, revision = "0"): Promise<void> {
  invoke.mockResolvedValueOnce(snapshot(value, revision));
  await settingsStore.loadSettings();
}

beforeEach(async () => {
  vi.resetModules();
  invoke.mockReset();
  attributes = new Map();
  inlineStyle = new Map();
  media = mediaQuery();
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  vi.stubGlobal("document", {
    documentElement: {
      setAttribute: (name: string, value: string) => attributes.set(name, value),
      getAttribute: (name: string) => attributes.get(name) ?? null,
      removeAttribute: (name: string) => attributes.delete(name),
      classList: { add: vi.fn(), remove: vi.fn() },
      style: {
        setProperty: (name: string, value: string) => inlineStyle.set(name, value),
        removeProperty: (name: string) => inlineStyle.delete(name),
      },
    },
  });
  vi.stubGlobal("window", { matchMedia: vi.fn(() => media) });
  settingsStore = await import("./settings-store.svelte");
});

afterEach(() => {
  llmStore?.resetLlmStore();
  llmStore = undefined;
  vi.unstubAllGlobals();
});

describe("settings rebrand audit: IPC snapshots", () => {
  it("applies the confirmed snapshot and sends only the requested partial", async () => {
    await loadSnapshot(settings("light"));
    const confirmed = settings("dark");
    invoke.mockResolvedValueOnce(snapshot(confirmed, "1"));
    expect(await settingsStore.updateSettings({ appearance: { mode: "dark" } })).toEqual(confirmed);
    expect(invoke).toHaveBeenLastCalledWith("settings_patch_v3", {
      request: { expectedRevision: "0", patch: { appearance: { mode: "dark" } } },
    });
    expect(settingsStore.getSettings()).toEqual(confirmed);
    expect(attributes.get("data-theme")).toBe("dark");
  });

  it.each(["load", "update", "reset"] as const)("keeps the confirmed snapshot and presentation when %s fails", async (operation) => {
    const confirmed = settings("light", 1.1);
    await loadSnapshot(confirmed);
    const beforeStyle = new Map(inlineStyle);
    invoke.mockRejectedValueOnce(new Error("AUDIT_DISK_WRITE_REJECTED"));
    const pending = operation === "load" ? settingsStore.loadSettings()
      : operation === "reset" ? settingsStore.resetSettings()
      : settingsStore.updateSettings({ appearance: { mode: "dark" } });
    await expect(pending).rejects.toThrow("AUDIT_DISK_WRITE_REJECTED");
    expect(settingsStore.getSettings()).toEqual(confirmed);
    expect(attributes.get("data-theme")).toBe("light");
    expect(inlineStyle).toEqual(beforeStyle);
  });

  // SR-FE-01: the store serializes mutations, so the second intent is sent only
  // after the first confirmation, with that revision; the final state is the
  // latest intent even though its response was ready first.
  it("SR-FE-01: an older update response cannot replace the latest confirmed intent", async () => {
    await loadSnapshot(settings("light"));
    const older = deferred<ReadySnapshot>();
    const newer = deferred<ReadySnapshot>();
    invoke.mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    const first = settingsStore.updateSettings({ appearance: { mode: "dark" } });
    const second = settingsStore.updateSettings({ appearance: { mode: "light" } });
    newer.resolve(snapshot(settings("light"), "2"));
    older.resolve(snapshot(settings("dark"), "1"));
    await Promise.all([first, second]);
    expect(invoke).toHaveBeenLastCalledWith("settings_patch_v3", {
      request: { expectedRevision: "1", patch: { appearance: { mode: "light" } } },
    });
    expect({ theme: settingsStore.getSettings()?.appearance.mode, presented: attributes.get("data-theme") })
      .toEqual({ theme: "light", presented: "light" });
  });

  // SR-FE-01 (out of order): a patch response older than a revision already
  // confirmed by another writer must not roll the presentation back.
  it("SR-FE-01: a response older than the confirmed revision cannot roll back the presentation", async () => {
    await loadSnapshot(settings("light"), "4");
    const older = deferred<ReadySnapshot>();
    invoke.mockImplementation((command: string) => {
      if (command === "settings_patch_v3") return older.promise;
      if (command === "settings_get_v3") return Promise.resolve(snapshot(settings("light"), "6"));
      return Promise.reject(new Error(`Unexpected audit IPC: ${command}`));
    });
    const pending = settingsStore.updateSettings({ appearance: { mode: "dark" } });
    settingsStore.handleSettingsChanged({ revision: "6" });
    await vi.waitFor(() => expect(settingsStore.getSettingsRevision()).toBe("6"));
    older.resolve(snapshot(settings("dark"), "5"));
    await pending;
    expect({ revision: settingsStore.getSettingsRevision(), theme: settingsStore.getSettings()?.appearance.mode, presented: attributes.get("data-theme") })
      .toEqual({ revision: "6", theme: "light", presented: "light" });
  });

  // SR-FE-02: a stale startup read must not roll back a completed edit in RAM/UI.
  it("SR-FE-02: a late initial read cannot roll back a completed edit", async () => {
    const initialRead = deferred<ReadySnapshot>();
    let reads = 0;
    invoke.mockImplementation((command: string) => {
      if (command === "settings_get_v3") return reads++ === 0 ? initialRead.promise : Promise.resolve(snapshot(settings("light"), "0"));
      if (command === "settings_patch_v3") return Promise.resolve(snapshot(settings("dark"), "1"));
      return Promise.reject(new Error(`Unexpected audit IPC: ${command}`));
    });
    const loading = settingsStore.loadSettings();
    await settingsStore.updateSettings({ appearance: { mode: "dark" } });
    initialRead.resolve(snapshot(settings("light"), "0"));
    await loading;
    expect({ theme: settingsStore.getSettings()?.appearance.mode, presented: attributes.get("data-theme") })
      .toEqual({ theme: "dark", presented: "dark" });
  });
});

describe("settings rebrand audit: typography and system appearance", () => {
  it("restores shipped CSS spacing when scale returns to 1", async () => {
    expect(baseSpacing.size).toBe(9);
    await loadSnapshot(settings("light", 1.2));
    await loadSnapshot(settings("light", 1));
    expect(currentSpacing()).toEqual(baseSpacing);
    for (const key of baseSpacing.keys()) expect(inlineStyle.has(key)).toBe(false);
  });

  // SR-FE-03: one spacing base in CSS and TS; 90% -> 100% never shrinks a token.
  it("SR-FE-03: increasing spacing from 90% to 100% must not shrink any spacing token", async () => {
    await loadSnapshot(settings("light", 0.9));
    const smallerScale = currentSpacing();
    await loadSnapshot(settings("light", 1));
    const regressions = [...currentSpacing()]
      .filter(([key, value]) => value < smallerScale.get(key)!)
      .map(([token, at100]) => ({ token, at90: smallerScale.get(token), at100 }));
    expect(regressions).toEqual([]);
  });

  it("keeps an explicit light choice when the operating system becomes dark", async () => {
    await loadSnapshot(settings("light"));
    media.change(true);
    expect(attributes.get("data-theme")).toBe("light");
  });

  // SR-FE-04: system mode follows the OS through a media listener.
  it("SR-FE-04: system appearance follows an OS change without another settings load", async () => {
    await loadSnapshot(settings("system"));
    expect(attributes.get("data-theme")).toBe("light");
    media.change(true);
    expect(attributes.get("data-theme")).toBe("dark");
  });
});

describe("settings rebrand audit: late conversation reads", () => {
  async function pendingHistory() {
    llmStore = await import("./llm-store.svelte");
    const history = deferred<Array<{ id: string; agent_id: string; title: string; messages: number; updated_ms: number }>>();
    invoke.mockImplementation((command: string) => {
      if (command === "llm_conversation_list") return history.promise;
      if (command === "llm_conversation_get") return Promise.resolve({ messages: [] });
      return Promise.reject(new Error(`Unexpected audit IPC: ${command}`));
    });
    const loading = llmStore.loadConversations();
    const id = llmStore.newConversation("audit-agent");
    llmStore.setConversationDraft(id, "Unsaved text written while history loads");
    return { history, loading, id };
  }

  it("retains a new draft if the saved list has no competing conversation", async () => {
    const { history, loading, id } = await pendingHistory();
    history.resolve([{ id: "saved-other", agent_id: "other-agent", title: "Other", messages: 1, updated_ms: 1 }]);
    await loading;
    expect(llmStore!.getActiveConversation()?.id).toBe(id);
    expect(llmStore!.getConversationDraft(id)).toBe("Unsaved text written while history loads");
    expect(invoke).not.toHaveBeenCalledWith("llm_turn_start", expect.anything());
  });

  // SR-FE-05: a placeholder holding an unsent draft is no longer disposable.
  it("SR-FE-05: arriving history cannot displace a new conversation containing an unsent draft", async () => {
    const { history, loading, id } = await pendingHistory();
    history.resolve([{ id: "saved-same", agent_id: "audit-agent", title: "Saved", messages: 1, updated_ms: 1 }]);
    await loading;
    expect(invoke).not.toHaveBeenCalledWith("llm_turn_start", expect.anything());
    expect({
      activeId: llmStore!.getActiveConversation()?.id,
      listed: llmStore!.getConversations().some((conversation) => conversation.id === id),
      draft: llmStore!.getConversationDraft(llmStore!.getActiveConversation()?.id),
    }).toEqual({ activeId: id, listed: true, draft: "Unsaved text written while history loads" });
  });
});
