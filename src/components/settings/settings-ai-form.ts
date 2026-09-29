export type AiProvider = "none" | "openai" | "anthropic" | "local";

export type AiConfigView = {
  provider: AiProvider;
  kind: string;
  model: string;
  local_base_url: string;
  has_openai_key: boolean;
  has_anthropic_key: boolean;
};

export type AiSettingsDraft = {
  provider: AiProvider;
  model: string;
  localBaseUrl: string;
  openaiKey: string;
  anthropicKey: string;
  removeOpenaiKey: boolean;
  removeAnthropicKey: boolean;
};

export function aiSettingsPayload(draft: AiSettingsDraft) {
  return {
    provider: draft.provider,
    model: draft.model.trim(),
    localBaseUrl: draft.localBaseUrl.trim(),
    openaiKey: draft.removeOpenaiKey ? "" : draft.openaiKey.trim() || null,
    anthropicKey: draft.removeAnthropicKey ? "" : draft.anthropicKey.trim() || null,
  };
}

export function isAiSettingsDirty(draft: AiSettingsDraft, saved: AiConfigView | null): boolean {
  if (!saved) return false;
  const payload = aiSettingsPayload(draft);
  return payload.provider !== saved.provider || payload.model !== saved.model ||
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
