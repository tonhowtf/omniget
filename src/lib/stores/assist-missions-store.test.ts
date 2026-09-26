import { beforeAll, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

type Store = typeof import("./assist-missions-store.svelte");
type Criterion = import("./assist-missions-store.svelte").Criterion;
type Verdict = import("./assist-missions-store.svelte").Verdict;
let store: Store;

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./assist-missions-store.svelte");
});

function crit(partial: Partial<Criterion>): Criterion {
  return { id: "c1", version: 1, kind: "command", severity: "required", title: "tests pass", spec: { command: "pnpm test" }, origin: "user", acceptance: "auto", ...partial };
}

describe("mission status", () => {
  it("gives every state a tone and a key", () => {
    for (const s of store.MISSION_STATES) {
      expect(store.missionStateKey(s)).toBe(`mission.state.${s}`);
      expect(["blue", "orange", "green", "red", "grey"]).toContain(store.missionTone(s));
    }
    expect(store.missionTone("running")).toBe("blue");
    expect(store.missionTone("blocked")).toBe("orange");
    expect(store.missionTone("succeeded")).toBe("green");
    expect(store.missionTone("failed")).toBe("red");
    expect(store.missionTone("cancelled")).toBe("grey");
  });

  it("offers only actions that make sense", () => {
    expect(store.missionActions("draft")).toContain("start");
    expect(store.missionActions("running")).toContain("pause");
    expect(store.missionActions("running")).not.toContain("resume");
    expect(store.missionActions("blocked")).toContain("resume");
    expect(store.missionActions("succeeded")).toEqual(["delete"]);
    expect(store.missionActions("cancelled")).not.toContain("verify");
  });

  it("files missions under the filters", () => {
    expect(store.matchesMissionFilter({ state: "blocked" }, "attention")).toBe(true);
    expect(store.matchesMissionFilter({ state: "running" }, "active")).toBe(true);
    expect(store.matchesMissionFilter({ state: "succeeded" }, "done")).toBe(true);
    expect(store.matchesMissionFilter({ state: "succeeded" }, "attention")).toBe(false);
  });
});

describe("verdict", () => {
  const verdict: Verdict = {
    passed: false,
    criteria: [
      { id: "c2", title: "reads well", kind: "rubric", severity: "advisory", acceptance: "human", objective: false, status: "awaiting_human", receipt_id: null, detail: null, confidence: null },
      { id: "c1", title: "tests pass", kind: "command", severity: "required", acceptance: "auto", objective: true, status: "fail", receipt_id: "r1", detail: "exit 1", confidence: 1 },
    ],
    failing: ["c1"], missing: [], awaiting_human: ["c2"], advisory_failing: [], completion: "human",
  };

  it("puts required criteria first and keeps undeclared ones as missing", () => {
    const rows = store.verdictRows(verdict, [crit({}), crit({ id: "c2", kind: "rubric", severity: "advisory" }), crit({ id: "c3", kind: "artifact", spec: { path: "out.md" } })]);
    expect(rows[0].severity).toBe("required");
    expect(rows.find((r) => r.id === "c1")?.tone).toBe("red");
    expect(rows.find((r) => r.id === "c2")?.needsDecision).toBe(true);
    expect(rows.find((r) => r.id === "c3")?.status).toBe("missing");
    expect(store.verdictRows(null, [])).toEqual([]);
  });

  it("summarizes failing before waiting, and passing plainly", () => {
    expect(store.verdictSummary(verdict)).toEqual({ key: "mission.verdict.failing", params: { count: 1 } });
    expect(store.verdictSummary({ ...verdict, failing: [] })).toEqual({ key: "mission.verdict.awaiting", params: { count: 1 } });
    expect(store.verdictSummary({ ...verdict, passed: true }).key).toBe("mission.verdict.passed");
    expect(store.verdictSummary(null).key).toBe("mission.verdict.none");
    expect(store.completionKey("human")).toBe("mission.completion.human");
    expect(store.completionKey("auto")).toBe("mission.completion.auto");
  });

  it("counts the list row summary", () => {
    expect(store.rowVerdictLines("every required criterion passed")).toEqual([{ key: "mission.verdict.passed" }]);
    expect(store.rowVerdictLines("failing: a, b; waiting for your decision: c")).toEqual([
      { key: "mission.verdict.failing", params: { count: 2 } },
      { key: "mission.verdict.awaiting", params: { count: 1 } },
    ]);
    expect(store.rowVerdictLines("")).toEqual([]);
  });

  it("the row counts what the detail counts, even with a comma in a title", () => {
    // Live demo 2026-09-25: "2 waiting" in the row, 1 in the detail.
    const summary = "waiting for your decision: O resumo tem no máximo 3 tópicos, cita a biblioteca e o domingo (c1)";
    const counts = { passed: false, failing: 0, missing: 0, awaiting_human: 1, partial: 0 };
    expect(store.rowVerdictLines(summary, counts)).toEqual([{ key: "mission.verdict.awaiting", params: { count: 1 } }]);
    const detail = store.verdictSummary({ passed: false, criteria: [], failing: [], missing: [], awaiting_human: ["O resumo tem no máximo 3 tópicos, cita a biblioteca e o domingo (c1)"], advisory_failing: [], completion: "human" } as never);
    expect(detail.params).toEqual({ count: 1 });
    expect(store.rowVerdictLines("x", { ...counts, passed: true })).toEqual([{ key: "mission.verdict.passed" }]);
  });

  it("only user and preset commands are executable", () => {
    expect(store.criterionExecutable(crit({ origin: "user" }))).toBe(true);
    expect(store.criterionExecutable(crit({ origin: "preset" }))).toBe(true);
    expect(store.criterionExecutable(crit({ origin: "import" }))).toBe(false);
    expect(store.criterionExecutable(crit({ origin: "proposed" }))).toBe(false);
    expect(store.criterionExecutable(crit({ kind: "artifact", origin: "import" }))).toBe(true);
  });
});

describe("budget", () => {
  it("never renders unknown cost as zero or free", () => {
    const line = store.spendLine({ usd_known: 0, unknown_cost_calls: 3 });
    expect(line.key).toBe("mission.budget.unknown_only");
    expect(JSON.stringify(line.params)).not.toContain("$0");
    expect(store.spendLine({ usd_known: 0.5, unknown_cost_calls: 2 })).toEqual({ key: "mission.budget.known_plus_unknown", params: { usd: "$0.50", calls: 2 } });
    expect(store.spendLine({ usd_known: 0, unknown_cost_calls: 0 })).toEqual({ key: "mission.budget.known", params: { usd: "$0.00" } });
    expect(store.formatUsd(0.0012)).toBe("$0.0012");
  });

  it("has no ratio when some cost is unknown", () => {
    const spent = { usd_known: 1, unknown_cost_calls: 0, tokens: 0, turns: 0, by_purpose: {} };
    const budget = { usd: 4, tokens: null, turns: null, max_minutes: null };
    expect(store.budgetRatio(spent, budget)).toBe(0.25);
    expect(store.budgetRatio({ ...spent, unknown_cost_calls: 1 }, budget)).toBeNull();
    expect(store.budgetRatio(spent, { ...budget, usd: null })).toBeNull();
    expect(store.budgetLimits({ usd: 2, tokens: null, turns: 10, max_minutes: null }).map((l) => l.key)).toEqual(["mission.budget.limit_usd", "mission.budget.limit_turns"]);
  });

  it("parses budget fields", () => {
    expect(store.parseBudgetField("")).toBeNull();
    expect(store.parseBudgetField("1,5")).toBe(1.5);
    expect(store.parseBudgetField("-1")).toBeUndefined();
    expect(store.parseBudgetField("abc")).toBeUndefined();
    expect(store.parseBudgetField("12.7", true)).toBe(13);
  });
});

describe("suggested actions", () => {
  const tasks = [{ id: "t1", state: "done" as const }, { id: "t2", state: "blocked" as const }];

  it("maps ids to commands on the blocked task", () => {
    expect(store.actionPlan({ id: "resume", needs_authorization: false }, "m1", tasks)).toEqual({ kind: "command", command: "assist_mission_resume", args: { id: "m1" }, confirm: false });
    expect(store.actionPlan({ id: "unblock_confirmed", needs_authorization: false }, "m1", tasks)).toMatchObject({ command: "assist_mission_unblock_task", args: { id: "m1", taskId: "t2", confirmed: true } });
    expect(store.actionPlan({ id: "unblock_retry", needs_authorization: false }, "m1", tasks)).toMatchObject({ args: { confirmed: false } });
    expect(store.actionPlan({ id: "unblock_retry", needs_authorization: false }, "m1", []).kind).toBe("unknown");
  });

  it("asks before a replay and before anything needing authorization", () => {
    expect(store.actionPlan({ id: "allow_replay", needs_authorization: false }, "m1", tasks)).toMatchObject({ command: "assist_mission_allow_replay", confirm: true });
    expect(store.actionPlan({ id: "cancel", needs_authorization: true }, "m1", tasks).confirm).toBe(true);
  });

  it("routes and local editors", () => {
    expect(store.actionPlan({ id: "open_mcp", needs_authorization: false }, "m1", tasks)).toMatchObject({ kind: "route", route: "/llm/mcp" });
    expect(store.actionPlan({ id: "change_bot", needs_authorization: false }, "m1", tasks)).toMatchObject({ kind: "route", route: "/llm/roster" });
    expect(store.actionPlan({ id: "edit_criteria", needs_authorization: false }, "m1", tasks)).toMatchObject({ kind: "local", local: "edit_criteria" });
    expect(store.actionPlan({ id: "edit_context", needs_authorization: false }, "m1", tasks)).toMatchObject({ kind: "local", local: "edit_context" });
    expect(store.actionPlan({ id: "raise_budget", needs_authorization: false }, "m1", tasks)).toMatchObject({ kind: "local", local: "edit_form" });
    expect(store.actionPlan({ id: "whatever", needs_authorization: false }, "m1", tasks).kind).toBe("unknown");
  });
});

describe("create form", () => {
  it("needs an objective and a required, complete criterion", () => {
    expect(store.validateDraft({ objective: " ", criteria: [crit({})] })).toBe("mission.create.err_objective");
    expect(store.validateDraft({ objective: "x", criteria: [] })).toBe("mission.create.err_required");
    expect(store.validateDraft({ objective: "x", criteria: [crit({ severity: "advisory" })] })).toBe("mission.create.err_required");
    expect(store.validateDraft({ objective: "x", criteria: [crit({ title: "" })] })).toBe("mission.create.err_title");
    expect(store.validateDraft({ objective: "x", criteria: [crit({ spec: { command: "" } })] })).toBe("mission.create.err_command");
    expect(store.validateDraft({ objective: "x", criteria: [crit({ kind: "artifact", spec: { path: "/etc/passwd" } })] })).toBe("mission.create.err_path_relative");
    expect(store.validateDraft({ objective: "x", criteria: [crit({ kind: "artifact", spec: { path: "../out" } })] })).toBe("mission.create.err_path_relative");
    expect(store.validateDraft({ objective: "x", criteria: [crit({ kind: "rubric", spec: { rubric: [" "] } })] })).toBe("mission.create.err_rubric");
    expect(store.validateDraft({ objective: "x", criteria: [crit({})] })).toBeNull();
  });

  it("new subjective criteria wait for a person", () => {
    expect(store.newCriterion("rubric").acceptance).toBe("human");
    expect(store.newCriterion("human").acceptance).toBe("human");
    expect(store.newCriterion("command").acceptance).toBe("auto");
    expect(store.newCriterion("command").origin).toBe("user");
    expect(store.newCriterion("command").id).not.toBe(store.newCriterion("command").id);
    expect(store.lines(" a\n\n b ")).toEqual(["a", "b"]);
  });

  it("sends the draft as a user_ui mission", async () => {
    invoke.mockResolvedValueOnce({});
    await store.createMission({ objective: "x", criteria: [], start: false });
    expect(invoke).toHaveBeenCalledWith("assist_mission_create", { draft: { auth_origin: "user_ui", objective: "x", criteria: [], start: false } });
    invoke.mockResolvedValueOnce({});
    await store.unblockTask("m1", "t1", null);
    expect(invoke).toHaveBeenCalledWith("assist_mission_unblock_task", { id: "m1", taskId: "t1", confirmed: null, note: null });
  });
});

describe("learning and packs", () => {
  it("promotes only with the latest eligible run of the current version", () => {
    expect(store.promotableRun(2, [{ id: "a", candidate_version: 1, decision: "eligible", created_ms: 1 }])).toBeNull();
    // Live demo 2026-09-25: B and C were both shown as "v1".
    const cands = [{ id: "8358da0b-aaaa", title: "Fonte no fim" }, { id: "158b953c-bbbb", title: "Tópicos curtos" }];
    const b = store.procedureVersionParams(cands, "8358da0b-aaaa", 1);
    const c = store.procedureVersionParams(cands, "158b953c-bbbb", 1);
    expect(b).toEqual({ title: "Fonte no fim", version: "1", id: "8358da0b" });
    expect(c.title).not.toBe(b.title);
    expect(store.procedureVersionParams([], "158b953c-bbbb", null)).toEqual({ title: "158b953c", version: "—", id: "158b953c" });
    expect(store.isActiveCandidate([{ candidate_id: "158b953c-bbbb", version: 1 }], { id: "158b953c-bbbb" })).toBe(true);
    expect(store.isActiveCandidate([{ candidate_id: "158b953c-bbbb", version: 1 }], { id: "8358da0b-aaaa" })).toBe(false);
    expect(store.promotableRun(2, [{ id: "a", candidate_version: 2, decision: "eligible", created_ms: 1 }, { id: "b", candidate_version: 2, decision: "rejected", created_ms: 2 }])).toBeNull();
    expect(store.promotableRun(2, [{ id: "a", candidate_version: 2, decision: "rejected", created_ms: 1 }, { id: "b", candidate_version: 2, decision: "eligible", created_ms: 2 }])?.id).toBe("b");
  });

  it("labels model statements as hypotheses and duplicates", () => {
    expect(store.observationLabelKeys({ source: "model", dup_of: "o1", revoked_ms: null })).toEqual(["assist.learning.source.model", "assist.learning.hypothesis", "assist.learning.duplicate"]);
    expect(store.observationLabelKeys({ source: "explicit_feedback", dup_of: null, revoked_ms: 5 })).toContain("assist.learning.revoked");
  });

  it("never installs hooks or scripts and overwrites only ticked conflicts", () => {
    expect(store.packNeverInstalled("hook")).toBe(true);
    expect(store.packNeverInstalled("script")).toBe(true);
    expect(store.packNeverInstalled("skill")).toBe(false);
    const items = [
      { key: "skill:a", action: "conflict_local_changes" as const },
      { key: "skill:b", action: "conflict_name" as const },
      { key: "skill:c", action: "new" as const },
      { key: "rule:d", action: "unchanged" as const },
    ];
    expect(store.overwriteList(items, { "skill:a": true, "skill:c": true })).toEqual(["skill:a"]);
    expect(store.planHasWork([items[0], items[3]], {})).toBe(false);
    expect(store.planHasWork([items[0]], { "skill:a": true })).toBe(true);
    expect(store.packKey({ kind: "skill", name: "x" })).toBe("skill:x");
  });
});

describe("tool approvals of a mission (D1)", () => {
  const mission = { conversation_id: "conv-1" };
  const jobs = [
    { id: "j1", conversation_id: "conv-1", request_id: "r1" },
    { id: "j2", conversation_id: "conv-1-bot2", request_id: "r2" },
    { id: "j3", conversation_id: "conv-10", request_id: "r3" },
    { id: "j4", conversation_id: "other", request_id: "r4" },
  ];
  const asks = ["r1", "r2", "r3", "r4", "r5"].map((r) => ({ agent: "a", request_id: r, tool_call_id: `t-${r}`, tool: "fs_write" }));

  it("keeps the questions of the mission conversation and its per-bot sub-conversations", () => {
    expect(store.missionAsks(asks, jobs, mission).map((a) => a.request_id)).toEqual(["r1", "r2"]);
  });

  it("matches a task's own job ids even in another conversation", () => {
    expect(store.missionAsks(asks, jobs, mission, ["j4"]).map((a) => a.request_id)).toEqual(["r1", "r2", "r4"]);
  });

  it("tells the chat store which questions belong to a mission", () => {
    expect(store.askBelongsToMission(asks[1], jobs, [mission])).toBe(true);
    expect(store.askBelongsToMission(asks[3], jobs, [mission])).toBe(false);
    expect(store.askBelongsToMission(asks[4], jobs, [mission])).toBe(false);
  });

  it("says why the row waits: approvals first, then the block", () => {
    expect(store.waitReason({ state: "running", block: null }, [{ tool: "fs_write" }])).toEqual({ key: "mission.approval.row", params: { count: 1, tool: "fs_write" } });
    const block = { code: "X", stage: "s", retryable: true, summary: "Stuck", evidence_refs: [], correlation_id: null, suggested_actions: [], detail: null };
    expect(store.waitReason({ state: "blocked", block }, [])).toEqual({ key: "mission.list.waits_reason", params: { reason: "Stuck" } });
    expect(store.waitReason({ state: "running", block: null }, [])).toBeNull();
  });
});

describe("backend default texts (D7) and labels (D8)", () => {
  it("localizes a controller's default artifact title only", () => {
    expect(store.criterionTitleLine({ id: "artifact-0", title: "Artifact 1", origin: "proposed" })).toEqual({ key: "mission.default_title.artifact", params: { count: 1 } });
    expect(store.criterionTitleLine({ id: "c9", title: "Artifact 1", origin: "user" })).toBeNull();
    expect(store.criterionTitleLine({ id: "tests", title: "anything", origin: "preset" })).toEqual({ key: "mission.default_title.preset_tests" });
    expect(store.criterionTitleLine({ id: "tests", title: "mine", origin: "user" })).toBeNull();
  });

  it("localizes driver task titles, purposes, effect states and presets", () => {
    expect(store.taskTitleLine("Round 3: fix failing checks")).toEqual({ key: "mission.default_title.round_fix", params: { count: 3 } });
    expect(store.taskTitleLine("Write the report")).toBeNull();
    expect(store.purposeKey("work")).toBe("mission.budget.purpose.work");
    expect(store.purposeKey("other")).toBeNull();
    expect(store.effectStateKey("confirmed")).toBe("mission.effect_state.confirmed");
    expect(store.effectStateKey("weird")).toBeNull();
    expect(store.presetTextKey("dev-review", "title")).toBe("mission.presets.item.dev_review.title");
    expect(store.presetTextKey("custom", "title")).toBeNull();
  });

  it("names a controller's proposal as the controller's", () => {
    expect(store.originKey("proposed", "external_mcp")).toBe("mission.origin.controller");
    expect(store.originKey("proposed", "user_ui")).toBe("mission.origin.proposed");
    expect(store.originKey("preset", "external_mcp")).toBe("mission.origin.preset");
  });

  it("checks typed paths and truncates long ones", () => {
    expect(store.isAbsolutePath("/Users/me/x")).toBe(true);
    expect(store.isAbsolutePath("C:\\work")).toBe(true);
    expect(store.isAbsolutePath("~/x")).toBe(false);
    expect(store.isAbsolutePath("relative/x")).toBe(false);
    expect(store.shortPath("/a/b")).toBe("/a/b");
    const long = "/Users/someone/Documents/projects/a-very-long-folder-name/and-more/deeper";
    const s = store.shortPath(long, 30);
    expect(s.length).toBeLessThanOrEqual(30);
    expect(s).toContain("…");
    expect(s.startsWith("/Users")).toBe(true);
    expect(s.endsWith("deeper")).toBe(true);
  });

  it("offers one resume action when paused", () => {
    expect(store.missionActions("paused").filter((a) => a === "resume")).toHaveLength(1);
  });
});

describe("budget revision (U1)", () => {
  const diag = (code: string, detail: unknown = null, summary = "The budget for this work is used up.", actions: string[] = ["raise_budget"]) => ({
    code, summary, detail, suggested_actions: actions.map((id) => ({ id, label: "", effect: "", needs_authorization: true })),
  });
  it("knows which limit stopped the mission", () => {
    expect(store.budgetBlockKind(diag("ERR_MISSION_BUDGET", "ERR_MISSION_BUDGET: ...\nused_minutes=15.0 max_minutes=15", "The mission used its time limit (15.0 of 15 minutes of work)."))).toBe("minutes");
    expect(store.budgetBlockKind(diag("ERR_LLM_BUDGET", "token budget: 0 used + 4191 would pass 300"))).toBe("tokens");
    expect(store.budgetBlockKind(diag("EXTERNAL_MISSION_BUDGET_EXCEEDED"))).toBe("tokens");
    expect(store.budgetBlockKind(diag("ERR_MISSION_STAGNANT", null, "stuck", ["edit_context"]))).toBeNull();
    expect(store.budgetBlockKind(null)).toBeNull();
  });
  it("the form sends only what was filled, as whole numbers", () => {
    expect(store.budgetRevision({ tokens: "60000", minutes: "" })).toEqual({ budget: { tokens: 60000 }, error: null });
    expect(store.budgetRevision({ tokens: "", minutes: "30" })).toEqual({ budget: { max_minutes: 30 }, error: null });
    expect(store.budgetRevision({ tokens: "", minutes: "" }).error).toBe("mission.budget_form.err_empty");
    expect(store.budgetRevision({ tokens: "-5", minutes: "" }).error).toBe("mission.budget_form.err_number");
    expect(store.budgetRevision({ tokens: "0", minutes: "" }).error).toBe("mission.budget_form.err_number");
    expect(store.budgetRevision({ tokens: "abc" }).error).toBe("mission.budget_form.err_number");
  });
  it("names the grant refusal and counts only worked minutes", () => {
    expect(store.budgetErrorKey("EXECUTION_GRANT_EXCEEDED: tokens above the grant's ceiling")).toBe("mission.budget_form.err_grant");
    expect(store.budgetErrorKey("ERR_MISSION_STATE: nope")).toBeNull();
    expect(store.minutesUsed({ active_ms: 90_000, active_since_ms: null })).toBe(1.5);
    expect(store.minutesUsed({ active_ms: 60_000, active_since_ms: 1_000 }, 61_000)).toBe(2);
  });
  it("calls the revise command with the mission, the budget and resume", async () => {
    invoke.mockResolvedValueOnce({});
    await store.reviseBudget("m1", { tokens: 10 }, true);
    expect(invoke).toHaveBeenLastCalledWith("assist_mission_revise_budget", { id: "m1", budget: { tokens: 10 }, resume: true });
  });
});

describe("effect question only when the effect is unknown (U3)", () => {
  const task = (error: string | null) => ({ id: "t1", state: "blocked" as const, error });
  it("never asks after a refusal before dispatch", () => {
    expect(store.taskEffectUnknown(task("ERR_LLM_BUDGET: token budget: 0 used or in flight + 4191 would pass 300"), [])).toBe(false);
    expect(store.taskEffectUnknown(task("ERR_LLM_BUDGET: x"), [{ task_id: "t1", state: "unknown" }])).toBe(false);
  });
  it("asks when the journal or the error says the effect is unknown", () => {
    expect(store.taskEffectUnknown(task("ERR_MISSION_EFFECT_UNKNOWN: job j1"), [])).toBe(true);
    expect(store.taskEffectUnknown(task("app restarted"), [{ task_id: "t1", state: "running" }])).toBe(true);
    expect(store.taskEffectUnknown(task("app restarted"), [{ task_id: "t2", state: "running" }, { task_id: "t1", state: "confirmed" }])).toBe(false);
    expect(store.taskEffectUnknown({ id: "t1", state: "done", error: null }, [{ task_id: "t1", state: "unknown" }])).toBe(false);
  });
});

describe("external missions and readable labels (U6, D7)", () => {
  it("marks MCP missions and names their client", () => {
    expect(store.isExternalMission({ auth_origin: "external_mcp", conversation_id: "x" })).toBe(true);
    expect(store.isExternalMission({ auth_origin: "user_ui", conversation_id: "external-mcp-abc" })).toBe(true);
    expect(store.isExternalMission({ auth_origin: "user_ui", conversation_id: "c1" })).toBe(false);
    expect(store.externalClientName({ principal: "p1" }, { p1: "C14 controller" })).toBe("C14 controller");
    expect(store.externalClientName({ principal: "p2" }, { p1: "C14 controller" })).toBeNull();
  });
  it("labels origins, effect kinds and numbered file criteria", () => {
    expect(store.authOriginKey("external_mcp")).toBe("mission.auth_origin.external_mcp");
    expect(store.authOriginKey("chat:abc")).toBe("mission.auth_origin.chat");
    expect(store.authOriginKey("weird")).toBeNull();
    expect(store.effectKindKey("agent_turn")).toBe("mission.effect_kind.agent_turn");
    expect(store.criterionTitleLine({ id: "artifact-0", title: "Artifact 1", origin: "proposed", spec: { path: "out/hello.md" } })).toEqual({ key: "mission.default_title.file_named", params: { name: "hello.md" } });
    expect(store.pathName("C:\\x\\y.txt")).toBe("y.txt");
  });
});
