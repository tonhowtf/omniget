export type AiProvider = "none" | "openai" | "anthropic" | "local";

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

export function aiSettingsPayload(draft: AiSettingsDraft) {
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
