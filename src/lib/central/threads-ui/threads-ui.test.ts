import { describe, expect, it } from "vitest";
import type { ActivityRow, MessageRow, ThreadDetail, TurnDetail } from "$lib/central/threads/types";
import { composeMessage, detectTrigger, recallable, replaceTrigger } from "./composer";
import { stableSplit, looksLikePath } from "./markdown";
import { deriveRows } from "./timeline";
import { splitUnifiedDiff, summarize, toWorkEntry } from "./work";

const T0 = "2026-09-22T10:00:00.000Z";
const at = (s: number) => new Date(Date.parse(T0) + s * 1000).toISOString();

function msg(id: string, role: string, text: string, s: number, turnId = "t1"): MessageRow {
  return { messageId: id, threadId: "thr", turnId, role, text, attachments: [], streaming: false, createdAt: at(s), updatedAt: at(s) };
}

function act(id: string, payload: Record<string, unknown>, s: number, kind = "item.completed"): ActivityRow {
  return { activityId: id, threadId: "thr", turnId: "t1", kind, tone: "tool", summary: String(payload.toolName ?? ""), payload, sequence: s, createdAt: at(s), updatedAt: at(s) };
}

function turn(state: TurnDetail["turn"]["state"], messages: MessageRow[], activities: ActivityRow[]): TurnDetail {
  return {
    turn: { turnId: "t1", threadId: "thr", ordinal: 1, messageId: "u1", state, model: null, requestedAt: at(0), startedAt: at(0), completedAt: state === "running" ? null : at(134), errorMessage: null },
    messages,
    activities,
    approvals: [],
    userInputs: [],
    plans: [],
    usage: null,
  };
}

const detailOf = (t: TurnDetail): ThreadDetail => ({ threadId: "thr", turns: [t], looseMessages: [], looseActivities: [], hasMore: false, beforeTurn: null, session: null, appliedThrough: 0, loading: false });
const opts = { expandedFolds: new Set<string>(), collapsedFolds: new Set<string>(), expandedGroups: new Set<string>(), interruptedHere: new Set<string>(), latestTurnId: "t1", planMode: false };

describe("composer triggers", () => {
  it("detects each trigger from the token before the caret", () => {
    expect(detectTrigger("/mo", 3)).toMatchObject({ kind: "command", query: "mo", start: 0 });
    expect(detectTrigger("fix @src/li", 11)).toMatchObject({ kind: "file", query: "src/li", start: 4 });
    expect(detectTrigger("see #12", 7)).toMatchObject({ kind: "pr", query: "12" });
    expect(detectTrigger("use $rev", 8)).toMatchObject({ kind: "skill", query: "rev" });
    expect(detectTrigger("costs $5", 8)).toBeNull();
    expect(detectTrigger("a /b", 4)).toBeNull();
  });

  it("replaces the token without doubling spaces", () => {
    const trig = detectTrigger("open @fo bar", 8)!;
    expect(replaceTrigger("open @fo bar", trig, "@foo.ts").text).toBe("open @foo.ts bar");
  });

  it("wraps context chips in one envelope that recall strips", () => {
    const out = composeMessage("look", [{ id: "c1", kind: "pr", label: "#3", value: "Pull request #3" }]);
    expect(out).toContain("<omniget_context>");
    expect(out).toContain("omniget-context://v1/pr/c1");
    expect(recallable(out)).toBe("look");
  });
});

describe("incremental markdown", () => {
  it("splits after the last closed block, never inside a fence", () => {
    const text = "para one\n\n```js\nconst a = 1;\n\nconst b = 2;\n";
    expect(text.slice(0, stableSplit(text))).toBe("para one\n\n");
    expect(stableSplit("no boundary yet")).toBe(0);
  });

  it("recognises path-like inline code", () => {
    expect(looksLikePath("src/lib/a.ts")).toBe(true);
    expect(looksLikePath("a.ts:12")).toBe(true);
    expect(looksLikePath("npm install")).toBe(false);
    expect(looksLikePath("1.2.3")).toBe(false);
  });
});

describe("work entries", () => {
  it("counts distinct edited files in the one-line summary", () => {
    const entries = [
      toWorkEntry(act("a", { itemType: "command_execution", status: "completed", data: { input: { command: ["bash", "-lc", "git status"] }, output: "ok" } }, 1)),
      toWorkEntry(act("b", { itemType: "file_change", status: "completed", toolName: "Edit", data: { input: { file_path: "a.ts", old_string: "x", new_string: "y" } } }, 2)),
      toWorkEntry(act("c", { itemType: "file_change", status: "completed", toolName: "Edit", data: { input: { file_path: "a.ts", old_string: "y", new_string: "z" } } }, 3)),
    ];
    expect(entries[0].command).toBe("git status");
    expect(entries[0].name).toBe("git");
    expect(entries[1].diff).toContain("+y");
    expect(summarize(entries)).toEqual([
      { key: "commands", count: 1 },
      { key: "edits", count: 1 },
    ]);
  });

  it("flags a 'completed' command whose output says it failed", () => {
    const e = toWorkEntry(act("a", { itemType: "command_execution", status: "completed", data: { input: { command: "foo" }, output: "zsh: command not found: foo" } }, 1));
    expect(e.status).toBe("failed");
  });

  it("splits a multi-file unified diff", () => {
    const diff = "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-x\n+y\ndiff --git a/n.ts b/n.ts\nnew file mode 100644\n--- /dev/null\n+++ b/n.ts\n@@ -0,0 +1,2 @@\n+1\n+2";
    const files = splitUnifiedDiff(diff);
    expect(files.map((f) => [f.path, f.status, f.additions, f.deletions])).toEqual([
      ["a.ts", "modified", 1, 1],
      ["n.ts", "added", 2, 0],
    ]);
  });
});

describe("timeline folding", () => {
  const work = [
    act("t1:item:1", { itemType: "command_execution", status: "completed", data: { input: { command: "ls" } } }, 2),
    act("t1:item:2", { itemType: "command_execution", status: "completed", data: { input: { command: "pwd" } } }, 3),
  ];

  it("folds the work of a settled turn behind 'Worked for' and keeps the answer", () => {
    const rows = deriveRows(detailOf(turn("completed", [msg("u1", "user", "hi", 0), msg("t1:a", "assistant", "done", 10)], work)), opts);
    expect(rows.map((r) => r.kind)).toEqual(["user", "fold", "assistant", "meta"]);
    const fold = rows[1] as Extract<(typeof rows)[number], { kind: "fold" }>;
    expect(fold.durationMs).toBe(134_000);
    expect(fold.open).toBe(false);
    expect(fold.summary).toEqual([{ key: "commands", count: 2 }]);
  });

  it("keeps a live row with a stable key while the turn runs", () => {
    const rows = deriveRows(detailOf(turn("running", [msg("u1", "user", "hi", 0)], work)), opts);
    expect(rows.map((r) => r.id)).toEqual(["u1", "live:t1", "working:t1"]);
  });

  it("opens an interrupted turn without an answer", () => {
    const rows = deriveRows(detailOf(turn("interrupted", [msg("u1", "user", "hi", 0)], work)), opts);
    const fold = rows.find((r) => r.kind === "fold") as Extract<(typeof rows)[number], { kind: "fold" }>;
    expect(fold.interrupted).toBe(true);
    expect(fold.open).toBe(true);
  });
});
