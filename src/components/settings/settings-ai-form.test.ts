import { afterEach, describe, expect, it, vi } from "vitest";
import {
  aiSettingsPayload, clearSavedAiSettingsDraft, isAiSettingsDirty, isLoopbackAiEndpoint,
  providerForKind, readAiSettingsDraft, retainAiSettingsDraft, type AiConfigView, type AiSettingsDraft,
  aiConnectionStatus, loadAiConnection, resetAiConnectionStatus, saveAiConnection, switchAiProvider,
  testAiConnection, type AiInvoke,
} from "./settings-ai-form";

const saved: AiConfigView = {
  provider: "local", kind: "openrouter", model: "example/model",
  local_base_url: "https://example.invalid/v1", has_openai_key: true, has_anthropic_key: false,
};
const draft = (): AiSettingsDraft => ({
  provider: saved.provider, kind: saved.kind, model: saved.model, localBaseUrl: saved.local_base_url,
  openaiKey: "", anthropicKey: "", removeOpenaiKey: false, removeAnthropicKey: false,
});

afterEach(() => { retainAiSettingsDraft(null); resetAiConnectionStatus(); });

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

  it("sends the picked provider-table row and reports a switch as a change", () => {
    expect(aiSettingsPayload(draft()).kind).toBe("openrouter");
    expect(isAiSettingsDirty({ ...draft(), kind: "deepseek" }, saved)).toBe(true);
    expect(aiSettingsPayload({ ...draft(), provider: "none", kind: "" }).kind).toBeNull();
  });

  it("derives the wire from the table row", () => {
    const kinds = [{ id: "anthropic", wire: "anthropic" }, { id: "openai", wire: "openai" }, { id: "groq", wire: "openai" }, { id: "gemini", wire: "gemini" }];
    expect(providerForKind("anthropic", kinds)).toBe("anthropic");
    expect(providerForKind("openai", kinds)).toBe("openai");
    expect(providerForKind("groq", kinds)).toBe("local");
    expect(providerForKind("gemini", kinds)).toBe("local");
    expect(providerForKind("none", kinds)).toBe("none");
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

/** Fake IPC: counts every command, never reaches a provider. */
function fakeInvoke(handlers: Record<string, (args?: Record<string, unknown>) => unknown>) {
  const fn = vi.fn(async (command: string, args?: Record<string, unknown>) => {
    const handler = handlers[command];
    if (!handler) throw new Error(`unexpected IPC ${command}`);
    return handler(args);
  });
  return { invoke: fn as unknown as AiInvoke, calls: (command: string) => fn.mock.calls.filter(([c]) => c === command).length, order: () => fn.mock.calls.map(([c]) => c) };
}

describe("AI connection save and test", () => {
  it("loading reads the saved connection only and never tests the provider", async () => {
    const ipc = fakeInvoke({ ai_get_config: () => saved });
    expect(await loadAiConnection(ipc.invoke)).toEqual(saved);
    expect(ipc.order()).toEqual(["ai_get_config"]);
    expect(aiConnectionStatus(saved)).toBe("unknown");
  });

  it("a dirty draft is saved first and tested only after the save is confirmed", async () => {
    const next = { ...saved, model: "new/model" };
    let release!: () => void;
    const pendingSave = new Promise<void>((done) => { release = done; });
    const ipc = fakeInvoke({ ai_set_config: async () => { await pendingSave; return next; }, ai_test: () => "ok" });
    const running = testAiConnection(ipc.invoke, { dirty: true, draft: { ...draft(), model: "new/model" }, saved });
    await Promise.resolve();
    expect(ipc.calls("ai_test")).toBe(0);
    release();
    expect(await running).toEqual({ kind: "ok", saved: next });
    expect(ipc.order()).toEqual(["ai_set_config", "ai_test"]);
    expect(aiConnectionStatus(next)).toBe("verified");
  });

  it("a failed save never reaches ai_test and keeps the draft", async () => {
    const form = { ...draft(), model: "unsaved/model" };
    retainAiSettingsDraft(form);
    const ipc = fakeInvoke({ ai_set_config: () => { throw "ERR_IO"; }, ai_test: () => "ok" });
    const outcome = await testAiConnection(ipc.invoke, { dirty: true, draft: form, saved });
    expect(outcome.kind).toBe("save_failed");
    expect(ipc.calls("ai_set_config")).toBe(1);
    expect(ipc.calls("ai_test")).toBe(0);
    expect(readAiSettingsDraft()?.model).toBe("unsaved/model");
    expect(aiConnectionStatus(saved)).toBe("unknown");
  });

  it("a clean form tests without saving; a failure is reported for that connection only", async () => {
    const ipc = fakeInvoke({ ai_test: () => { throw "ERR_AUTH"; } });
    const outcome = await testAiConnection(ipc.invoke, { dirty: false, draft: draft(), saved });
    expect(outcome.kind).toBe("test_failed");
    expect(ipc.order()).toEqual(["ai_test"]);
    expect(aiConnectionStatus(saved)).toBe("error");
    expect(aiConnectionStatus({ ...saved, model: "other" })).toBe("unknown");
  });

  it("saving again forgets an earlier verified result", async () => {
    const ipc = fakeInvoke({ ai_test: () => "ok", ai_set_config: () => saved });
    await testAiConnection(ipc.invoke, { dirty: false, draft: draft(), saved });
    expect(aiConnectionStatus(saved)).toBe("verified");
    await saveAiConnection(ipc.invoke, { ...draft(), openaiKey: "replacement" });
    expect(aiConnectionStatus(saved)).toBe("unknown");
    expect(ipc.calls("ai_test")).toBe(1);
  });
});

describe("AI provider switch", () => {
  it("clears only hidden draft fields and never turns a switch into key removal", () => {
    const marked = { ...draft(), provider: "openai" as const, localBaseUrl: "", openaiKey: "typed", removeOpenaiKey: true };
    const toAnthropic = switchAiProvider(marked, "anthropic");
    expect(toAnthropic).toMatchObject({ provider: "anthropic", openaiKey: "", removeOpenaiKey: false });
    expect(aiSettingsPayload(toAnthropic).openaiKey).toBeNull();
    expect(aiSettingsPayload(toAnthropic).anthropicKey).toBeNull();
  });

  it("keeps the shared connection key between endpoint and OpenAI, and drops the endpoint", () => {
    const local = { ...draft(), openaiKey: "typed" };
    expect(switchAiProvider(local, "openai")).toMatchObject({ openaiKey: "typed", localBaseUrl: "" });
    expect(switchAiProvider(local, "none")).toMatchObject({ openaiKey: "", localBaseUrl: saved.local_base_url });
    expect(switchAiProvider(local, "local")).toEqual(local);
  });
});
