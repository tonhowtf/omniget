import { describe, expect, it, vi } from "vitest";
import type { AiInvoke } from "$components/settings/settings-ai-form";
import { parseSummariesTarget } from "$lib/settings-navigation";
import { clearSummaryHistory, loadSummaryHistory, runSummary, summaryErrorKey, summaryFocusSelector } from "./summaries";

function fakeInvoke(result: (command: string) => unknown = () => null) {
  const fn = vi.fn(async (command: string, _args?: Record<string, unknown>) => result(command));
  return { invoke: fn as unknown as AiInvoke, fn };
}

describe("summaries route contract", () => {
  it("opening a deep link resolves a target without any IPC", () => {
    const { fn } = fakeInvoke();
    const target = parseSummariesTarget(new URLSearchParams("section=history&setting=settings.ai.history"));
    expect(target).toEqual({ section: "history", setting: "settings.ai.history" });
    expect(summaryFocusSelector(target)).toBe('[data-setting-key="settings.ai.history"]');
    expect(summaryFocusSelector(parseSummariesTarget(new URLSearchParams("section=form")))).toBe("#summaries-form");
    expect(summaryFocusSelector(parseSummariesTarget(new URLSearchParams("section=run&setting=x\"]")))).toBeNull();
    expect(fn).not.toHaveBeenCalled();
  });

  it("history is read newest first and clear is a single explicit command", async () => {
    const entries = [1, 2, 3].map((id) => ({ id, kind: "summary", url: `u${id}`, title: "", content: "", created_at_ms: id }));
    const { invoke, fn } = fakeInvoke((command) => (command === "ai_history_list" ? entries : null));
    expect((await loadSummaryHistory(invoke)).map((e) => e.id)).toEqual([3, 2, 1]);
    expect(fn.mock.calls.map(([c]) => c)).toEqual(["ai_history_list"]);
    await clearSummaryHistory(invoke);
    expect(fn.mock.calls.map(([c]) => c)).toEqual(["ai_history_list", "ai_history_clear"]);
  });

  it("a summary sends exactly one request with the saved style/lang contract", async () => {
    const { invoke, fn } = fakeInvoke();
    await runSummary(invoke, { url: "  https://example.invalid/v  ", style: "balanced", lang: " " });
    await runSummary(invoke, { url: "https://example.invalid/w", style: "short", lang: "pt" });
    expect(fn.mock.calls).toEqual([
      ["ai_summarize_url", { url: "https://example.invalid/v", style: null, lang: null }],
      ["ai_summarize_url", { url: "https://example.invalid/w", style: "short", lang: "pt" }],
    ]);
    await expect(runSummary(invoke, { url: " ", style: "short", lang: "" })).rejects.toThrow();
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("maps known backend codes to copy and leaves others alone", () => {
    expect(summaryErrorKey("no_transcript")).toBe("settings.ai.no_transcript");
    expect(summaryErrorKey("ai_not_configured")).toBe("settings.ai.not_configured");
    expect(summaryErrorKey("ERR_OTHER")).toBeNull();
  });
});
