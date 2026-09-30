import {
  getSettings,
  onConfirmedSnapshot,
  saveSettingsForm,
  type AppSettings,
  type SettingsSecretOperations,
} from "./settings-store.svelte";
import { isApiError } from "./settings-v3-types";

/**
 * In-memory drafts for forms with Save/Discard (proxy, fonts, hotkeys). One
 * draft per id survives tab changes inside Settings; nothing goes to
 * localStorage, so a secret typed into a draft lives only in this WebView.
 */
export type SettingsDraft<T extends Record<string, unknown>> = {
  readonly value: T;
  readonly dirty: boolean;
  readonly saving: boolean;
  /** Stable code; the UI translates it with settings.rebrand.state.save_failed. */
  readonly error: string | null;
  save(): Promise<boolean>;
  discard(): void;
};

export type SettingsDraftOptions<T extends Record<string, unknown>> = {
  id: string;
  read: (s: AppSettings) => T;
  toPatch: (draft: T) => Record<string, unknown>;
  secretOperations?: (draft: T) => SettingsSecretOperations | undefined;
};

function clone<T>(value: T): T {
  return value === undefined ? value : JSON.parse(JSON.stringify(value));
}

function same(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

type Entry = {
  options: SettingsDraftOptions<Record<string, unknown>>;
  draft: SettingsDraft<Record<string, unknown>>;
  stop: () => void;
};

const drafts = new Map<string, Entry>();

function errorCode(error: unknown): string {
  if (isApiError(error)) return error.code;
  if (error instanceof Error && /^settings_[a-z_]+$/.test(error.message)) return error.message.slice("settings_".length);
  return "save_failed";
}

function makeDraft(options: SettingsDraftOptions<Record<string, unknown>>): { draft: SettingsDraft<Record<string, unknown>>; stop: () => void } {
  const initial = getSettings();
  const state = $state({
    value: initial ? clone(options.read(initial)) : ({} as Record<string, unknown>),
    base: initial ? clone(options.read(initial)) : null as Record<string, unknown> | null,
    saving: false,
    error: null as string | null,
  });

  const resync = () => {
    const current = getSettings();
    if (!current) return;
    const next = options.read(current);
    state.value = clone(next);
    state.base = clone(next);
  };

  const stop = onConfirmedSnapshot((reason) => {
    const current = getSettings();
    if (!current) return;
    const dirty = state.base !== null && !same(state.value, state.base);
    // A reset starts a new generation: drafts go back to the confirmed values.
    if (reason === "reset" || !dirty) { resync(); if (reason === "reset") state.error = null; return; }
    // Keep the newer intent; only follow the confirmed base.
    state.base = clone(options.read(current));
  });

  const draft: SettingsDraft<Record<string, unknown>> = {
    get value() { return state.value; },
    get dirty() { return state.base !== null && !same(state.value, state.base); },
    get saving() { return state.saving; },
    get error() { return state.error; },
    async save() {
      if (state.saving) return false;
      state.saving = true;
      state.error = null;
      const snapshotOfDraft = clone(state.value);
      try {
        await saveSettingsForm(options.toPatch(snapshotOfDraft), options.secretOperations?.(snapshotOfDraft));
        // Values typed after Save was pressed stay a draft.
        if (same(state.value, snapshotOfDraft)) resync();
        else { const current = getSettings(); if (current) state.base = clone(options.read(current)); }
        return true;
      } catch (error) {
        state.error = errorCode(error);
        return false;
      } finally {
        state.saving = false;
      }
    },
    discard() {
      state.error = null;
      resync();
    },
  };
  return { draft, stop };
}

export function createSettingsDraft<T extends Record<string, unknown>>(options: SettingsDraftOptions<T>): SettingsDraft<T> {
  const existing = drafts.get(options.id);
  if (existing) {
    // Keep the draft text; take the newest mapping functions from the caller.
    existing.options.read = options.read as Entry["options"]["read"];
    existing.options.toPatch = options.toPatch as Entry["options"]["toPatch"];
    existing.options.secretOperations = options.secretOperations as Entry["options"]["secretOperations"];
    return existing.draft as SettingsDraft<T>;
  }
  const entryOptions = { ...options } as unknown as Entry["options"];
  const { draft, stop } = makeDraft(entryOptions);
  drafts.set(options.id, { options: entryOptions, draft, stop });
  return draft as SettingsDraft<T>;
}

/** Test seam and app-reset helper: forget every draft (memory only). */
export function clearSettingsDrafts(): void {
  for (const entry of drafts.values()) entry.stop();
  drafts.clear();
}
