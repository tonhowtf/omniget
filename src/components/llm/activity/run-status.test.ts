import { describe, expect, it } from "vitest";
import { diffLineKind, matchesFilter, parseResolution, runStatus, secondsLeft } from "./run-status";
import type { RunView } from "$lib/stores/llm-jobs-store.svelte";

function run(partial: Partial<RunView>): RunView {
  return {
    id: "r",
    session_id: null,
    conversation_id: "c",
    bot_id: "b",
    parent_run_id: null,
    state: "running",
    resume_kind: null,
    runtime: null,
    cwd: null,
    instance_id: "i",
    launch_id: null,
    pid: null,
    input_preview: "",
    summary: null,
    error: null,
    resolution: null,
    tokens_in: 0,
    tokens_out: 0,
    cost_usd: null,
    events_dropped: 0,
    created_ms: 0,
    started_ms: null,
    ended_ms: null,
    updated_ms: 0,
    pending_permissions: 0,
    live: true,
    actions: [],
    tool_calls: 0,
    ...partial,
  };
}

describe("run status", () => {
  it("speaks plainly about every backend state", () => {
    expect(runStatus("preparing")).toBe("working");
    expect(runStatus("running")).toBe("working");
    expect(runStatus("waiting_user")).toBe("waiting");
    expect(runStatus("unknown")).toBe("interrupted");
    expect(runStatus("interrupted")).toBe("interrupted");
    expect(runStatus("completed")).toBe("done");
    expect(runStatus("failed")).toBe("failed");
  });

  it("puts unresolved interruptions and questions under attention", () => {
    expect(matchesFilter(run({ state: "unknown" }), "attention")).toBe(true);
    expect(matchesFilter(run({ state: "unknown", resolution: "marked_done" }), "attention")).toBe(false);
    expect(matchesFilter(run({ state: "unknown", resolution: "marked_done" }), "done")).toBe(true);
    expect(matchesFilter(run({ state: "running", pending_permissions: 1 }), "attention")).toBe(true);
    expect(matchesFilter(run({ state: "completed" }), "active")).toBe(false);
  });

  it("paints diff lines and reads resolutions", () => {
    expect(diffLineKind("+++ b/a.txt")).toBe("file");
    expect(diffLineKind("+new")).toBe("add");
    expect(diffLineKind("-old")).toBe("del");
    expect(diffLineKind("@@ -1 +1 @@")).toBe("hunk");
    expect(diffLineKind(" same")).toBe("ctx");
    expect(parseResolution("continued:job:j1")).toEqual({ kind: "continued", ref: "job:j1" });
    expect(parseResolution("discarded")).toEqual({ kind: "discarded", ref: null });
    expect(secondsLeft(5_000, 1_000)).toBe(4);
    expect(secondsLeft(1_000, 5_000)).toBe(0);
  });
});

describe("same words as the job list (U5)", () => {
  it("a run failed by cancellation reads as cancelled", async () => {
    const { runStatus, promptTitle } = await import("./run-status");
    expect(runStatus("failed", "ERR_LLM_CANCELLED: stopped")).toBe("cancelled");
    expect(runStatus("failed", "cancelled")).toBe("cancelled");
    expect(runStatus("failed", "ERR_LLM_NET: down")).toBe("failed");
    expect(promptTitle("[Mission 66cf60cd — objective]\nWrite hello.md\n\nmore")).toEqual({ title: "Write hello.md", mission: "66cf60cd" });
    expect(promptTitle("\n  plain prompt\nx")).toEqual({ title: "plain prompt", mission: null });
  });
});
