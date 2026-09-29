import { afterEach, describe, expect, it } from "vitest";
import {
  aiSettingsPayload, clearSavedAiSettingsDraft, isAiSettingsDirty, isLoopbackAiEndpoint,
  readAiSettingsDraft, retainAiSettingsDraft, type AiConfigView, type AiSettingsDraft,
} from "./settings-ai-form";

const saved: AiConfigView = {
  provider: "local", kind: "openrouter", model: "example/model",
  local_base_url: "https://example.invalid/v1", has_openai_key: true, has_anthropic_key: false,
};
const draft = (): AiSettingsDraft => ({
  provider: saved.provider, model: saved.model, localBaseUrl: saved.local_base_url,
  openaiKey: "", anthropicKey: "", removeOpenaiKey: false, removeAnthropicKey: false,
});

afterEach(() => retainAiSettingsDraft(null));

describe("AI settings credential intent", () => {
  it("keeps a saved credential when the input is blank, but sends an explicit removal", () => {
    const form = draft();
    expect(aiSettingsPayload(form).openaiKey).toBeNull();
    expect(isAiSettingsDirty(form, saved)).toBe(false);
    form.removeOpenaiKey = true;
    expect(aiSettingsPayload(form).openaiKey).toBe("");
    expect(isAiSettingsDirty(form, saved)).toBe(true);
  });

  it("replaces only the edited key and keeps the other provider credential", () => {
    const form = { ...draft(), openaiKey: " test-key " };
    expect(aiSettingsPayload(form)).toMatchObject({ openaiKey: "test-key", anthropicKey: null });
    expect(isAiSettingsDirty(form, saved)).toBe(true);
  });

  it("does not report a change for endpoint normalization alone", () => {
    expect(isAiSettingsDirty({ ...draft(), localBaseUrl: `${saved.local_base_url}/ ` }, saved)).toBe(false);
  });

  it("does not label a remote lookalike as a local connection", () => {
    expect(isLoopbackAiEndpoint("http://localhost:11434/v1")).toBe(true);
    expect(isLoopbackAiEndpoint("http://[::1]:8080/v1")).toBe(true);
    expect(isLoopbackAiEndpoint("https://localhost.example.invalid/v1")).toBe(false);
    expect(isLoopbackAiEndpoint("https://localhost@example.invalid/v1")).toBe(false);
    expect(isLoopbackAiEndpoint("https://openrouter.ai/api/v1")).toBe(false);
  });
});

describe("AI settings draft across section navigation", () => {
  it("retains a copy of unfinished edits and clears them only after explicit discard", () => {
    const form = { ...draft(), model: "new/model", openaiKey: "test-only" };
    retainAiSettingsDraft(form);
    form.model = "changed-after-unmount";
    expect(readAiSettingsDraft()?.model).toBe("new/model");
    const reopened = readAiSettingsDraft()!;
    reopened.openaiKey = "another-edit";
    expect(readAiSettingsDraft()?.openaiKey).toBe("test-only");
    retainAiSettingsDraft(null);
    expect(readAiSettingsDraft()).toBeNull();
  });

  it("a completed old save does not erase a newer retained draft", () => {
    const saving = { ...draft(), model: "saving" };
    retainAiSettingsDraft({ ...saving, model: "newer" });
    clearSavedAiSettingsDraft(saving);
    expect(readAiSettingsDraft()?.model).toBe("newer");
    clearSavedAiSettingsDraft(readAiSettingsDraft()!);
    expect(readAiSettingsDraft()).toBeNull();
  });
});
