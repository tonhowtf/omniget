type AiProvider = "none" | "openai" | "anthropic" | "local";

export type AiConfigView = {
  provider: AiProvider;
  kind: string;
  model: string;
  local_base_url: string;
  has_openai_key: boolean;
  has_anthropic_key: boolean;
};

/** A row of the provider table (`tool_keys_kinds`) with the flags the form reads. */
export type AiKind = {
  id: string;
  name: string;
  base_url: string;
  env: string;
  wire: string;
  needs_key: boolean;
  accepts_key: boolean;
  base_url_editable: boolean;
};

export type AiSettingsDraft = {
  provider: AiProvider;
  /** The provider-table row picked, or "" when AI is off. */
  kind: string;
  model: string;
  localBaseUrl: string;
  openaiKey: string;
  anthropicKey: string;
  removeOpenaiKey: boolean;
  removeAnthropicKey: boolean;
};

/** The wire a table row speaks, the same rule as `ai::provider_for_kind`. */
export function providerForKind(kind: string, kinds: readonly Pick<AiKind, "id" | "wire">[]): AiProvider {
  if (!kind || kind === "none") return "none";
  if (kind === "anthropic" || kinds.find(k => k.id === kind)?.wire === "anthropic") return "anthropic";
  return kind === "openai" ? "openai" : "local";
}

function aiSettingsPayload(draft: AiSettingsDraft) {
  return {
    provider: draft.provider,
    kind: draft.provider === "none" ? null : draft.kind || null,
    model: draft.model.trim(),
    localBaseUrl: draft.localBaseUrl.trim(),
    openaiKey: draft.removeOpenaiKey ? "" : draft.openaiKey.trim() || null,
    anthropicKey: draft.removeAnthropicKey ? "" : draft.anthropicKey.trim() || null,
  };
}

export function isAiSettingsDirty(draft: AiSettingsDraft, saved: AiConfigView | null): boolean {
  if (!saved) return false;
  const payload = aiSettingsPayload(draft);
  return payload.provider !== saved.provider || (payload.kind !== null && payload.kind !== saved.kind) || payload.model !== saved.model ||
    payload.localBaseUrl.replace(/\/+$/, "") !== saved.local_base_url.replace(/\/+$/, "") ||
    payload.openaiKey !== null || payload.anthropicKey !== null;
}

export function isLoopbackAiEndpoint(value: string): boolean {
  try {
    const { hostname } = new URL(value);
    return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "[::1]";
  } catch {
    return false;
  }
}

// Session memory only: drafts may contain keys and must never enter localStorage.
let retainedDraft: AiSettingsDraft | null = null;

export function retainAiSettingsDraft(draft: AiSettingsDraft | null): void {
  retainedDraft = draft ? { ...draft } : null;
}

export function readAiSettingsDraft(): AiSettingsDraft | null {
  return retainedDraft ? { ...retainedDraft } : null;
}

export function clearSavedAiSettingsDraft(savedDraft: AiSettingsDraft): void {
  if (retainedDraft && JSON.stringify(retainedDraft) === JSON.stringify(savedDraft)) {
    retainedDraft = null;
  }
}

export type AiInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
type AiConnectionStatus = "unknown" | "verified" | "error";

// Result of the last explicit test, bound to the saved connection it ran on.
// Any later save makes it "unknown" again: a filled form is never "verified".
let lastTest: { fingerprint: string; ok: boolean } | null = null;

function fingerprint(config: AiConfigView): string {
  return JSON.stringify([config.provider, config.kind, config.model, config.local_base_url, config.has_openai_key, config.has_anthropic_key]);
}

export function aiConnectionStatus(saved: AiConfigView | null): AiConnectionStatus {
  if (!saved || saved.provider === "none" || !lastTest || lastTest.fingerprint !== fingerprint(saved)) return "unknown";
  return lastTest.ok ? "verified" : "error";
}

/** Local read only: never contacts the provider. */
export function loadAiConnection(invoke: AiInvoke): Promise<AiConfigView> {
  return invoke<AiConfigView>("ai_get_config");
}

/** Persists the draft. A confirmed save forgets any earlier test result. */
export async function saveAiConnection(invoke: AiInvoke, draft: AiSettingsDraft): Promise<AiConfigView> {
  const result = await invoke<AiConfigView>("ai_set_config", aiSettingsPayload(draft));
  lastTest = null;
  clearSavedAiSettingsDraft(draft);
  return result;
}

type AiTestOutcome =
  | { kind: "save_failed"; error: unknown }
  | { kind: "ok"; saved: AiConfigView | null }
  | { kind: "test_failed"; saved: AiConfigView | null; error: unknown };

/**
 * Explicit test. A dirty draft is saved first and the provider is contacted
 * only once that save is confirmed; a failed save never reaches `ai_test`.
 */
export async function testAiConnection(
  invoke: AiInvoke,
  opts: { dirty: boolean; draft: AiSettingsDraft; saved: AiConfigView | null },
): Promise<AiTestOutcome> {
  let saved = opts.saved;
  if (opts.dirty) {
    try {
      saved = await saveAiConnection(invoke, opts.draft);
    } catch (error) {
      return { kind: "save_failed", error };
    }
  }
  try {
    await invoke<string>("ai_test");
    if (saved) lastTest = { fingerprint: fingerprint(saved), ok: true };
    return { kind: "ok", saved };
  } catch (error) {
    if (saved) lastTest = { fingerprint: fingerprint(saved), ok: false };
    return { kind: "test_failed", saved, error };
  }
}
