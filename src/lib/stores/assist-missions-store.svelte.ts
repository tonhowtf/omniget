/**
 * Missions: a durable objective given to a bot or a group, with versioned
 * completion criteria, over the `assist_mission_*` commands
 * (`src-tauri/src/commands/assist/missions.rs`).
 *
 * A mission is `succeeded` only when every required criterion has a `pass`
 * receipt for the current artifact; a model saying "done" never finishes it.
 * The pure helpers below turn backend states into i18n keys, tones and
 * actions; they carry the logic and are what the tests cover. Learning and
 * pack-import helpers used by the bot panel and the Skills page live here too.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ── Types ──────────────────────────────────────────────────────────────

export type MissionState =
  | "draft"
  | "queued"
  | "running"
  | "verifying"
  | "succeeded"
  | "partial"
  | "blocked"
  | "paused"
  | "failed"
  | "cancelled";

export const MISSION_STATES: MissionState[] = [
  "draft", "queued", "running", "verifying", "succeeded", "partial", "blocked", "paused", "failed", "cancelled",
];

export interface MissionBudget {
  usd: number | null;
  tokens: number | null;
  turns: number | null;
  max_minutes: number | null;
}

export interface MissionSpent {
  usd_known: number;
  unknown_cost_calls: number;
  tokens: number;
  turns: number;
  by_purpose: Record<string, number>;
  /** Milliseconds of running/verifying work in finished activations (the clock `max_minutes` counts). */
  active_ms?: number;
  active_since_ms?: number | null;
}

export interface SuggestedAction {
  id: string;
  label: string;
  effect: string;
  needs_authorization: boolean;
}

export interface Diagnosis {
  code: string;
  stage: string;
  retryable: boolean;
  summary: string;
  evidence_refs: string[];
  correlation_id: string | null;
  suggested_actions: SuggestedAction[];
  detail: unknown;
}

export interface Mission {
  id: string;
  bot_id: string | null;
  room_id: string | null;
  conversation_id: string;
  /** Who owns it: the person, or the MCP client id of an external mission. */
  principal?: string;
  auth_origin: string;
  objective: string;
  state: MissionState;
  criteria_version: number;
  budget: MissionBudget;
  spent: MissionSpent;
  workspace: string | null;
  context_kind: string;
  pins: unknown;
  allow_replay: boolean;
  block: Diagnosis | null;
  result: { summary?: string; verdict?: unknown; delivered_by?: string } | null;
  revision: number;
  created_ms: number;
  updated_ms: number;
  started_ms: number | null;
  finished_ms: number | null;
}

export type Completion = "auto" | "human";

/** Required criteria per state, from the same verdict the detail counts. */
export interface VerdictCounts {
  passed: boolean;
  failing: number;
  missing: number;
  awaiting_human: number;
  partial: number;
}

export interface MissionRow extends Mission {
  tasks_total: number;
  tasks_done: number;
  verdict: string;
  verdict_counts?: VerdictCounts;
  completion: Completion;
  driving: boolean;
}

export type CriterionKind = "command" | "artifact" | "tool_result" | "rubric" | "human";
export const CRITERION_KINDS: CriterionKind[] = ["command", "artifact", "tool_result", "rubric", "human"];
export type Severity = "required" | "advisory";
export type Origin = "user" | "preset" | "proposed" | "import";
export type Acceptance = "auto" | "human" | "single_review";

export interface Criterion {
  id: string;
  version: number;
  kind: CriterionKind;
  severity: Severity;
  title: string;
  spec: Record<string, unknown>;
  origin: Origin;
  acceptance: Acceptance;
}

export type CriterionStatus = "pass" | "fail" | "unknown" | "missing" | "stale" | "awaiting_human";

export interface VerdictCriterion {
  id: string;
  title: string;
  kind: CriterionKind;
  severity: Severity;
  acceptance: Acceptance;
  objective: boolean;
  status: CriterionStatus;
  receipt_id: string | null;
  detail: string | null;
  confidence: number | null;
}

export interface Verdict {
  passed: boolean;
  criteria: VerdictCriterion[];
  failing: string[];
  missing: string[];
  awaiting_human: string[];
  advisory_failing: string[];
  completion: Completion;
}

export type TaskState = "pending" | "claimed" | "running" | "done" | "failed" | "cancelled" | "skipped" | "blocked";

export interface MissionTask {
  id: string;
  position: number;
  title: string;
  input: string;
  deps: string[];
  bot_id: string | null;
  state: TaskState;
  attempts: number;
  max_attempts: number;
  job_ids: string[];
  result: string | null;
  error: string | null;
}

export interface Receipt {
  id: string;
  criterion_id: string;
  criterion_version: number;
  artifact_ref: string | null;
  artifact_digest: string | null;
  verifier: string;
  verifier_version: string | null;
  status: string;
  exit_code: number | null;
  confidence: number | null;
  evidence: unknown;
  created_ms: number;
  invalidated_ms: number | null;
  invalidated_reason: string | null;
}

export interface MissionEffect {
  key: string;
  task_id: string | null;
  kind: string;
  fingerprint: string;
  state: string;
  detail: string | null;
  created_ms: number;
  updated_ms: number;
}

export interface MissionEvent {
  event_id: string;
  seq: number;
  kind: string;
  revision: number;
  payload: unknown;
  ts_ms: number;
}

export interface BudgetPool {
  spent: { usd: number; turns: number; input_tokens: number; output_tokens: number; unknown_cost_turns: number };
  in_flight: number;
  reserved_usd: number;
  reserved_unknown_cost: number;
  reserved_tokens: number;
}

export interface MissionDetail {
  mission: Mission;
  criteria: Criterion[];
  tasks: MissionTask[];
  receipts: Receipt[];
  effects: MissionEffect[];
  verdict: Verdict | null;
  last_checkpoint: unknown;
  events: MissionEvent[];
  driving: boolean;
  checkpoints: { seq: number; reason: string; created_ms: number }[];
  pool: BudgetPool | null;
  context_preview: string | null;
}

export interface NewTask {
  key?: string;
  title: string;
  input: string;
  deps?: string[];
  bot_id?: string | null;
  max_attempts?: number;
}

export interface NewBudget {
  usd?: number | null;
  tokens?: number | null;
  turns?: number | null;
  max_minutes?: number | null;
}

export interface NewMission {
  objective: string;
  bot_id?: string | null;
  room_id?: string | null;
  conversation_id?: string | null;
  auth_origin?: "user_ui";
  criteria: Criterion[];
  tasks?: NewTask[];
  budget?: NewBudget;
  workspace?: string | null;
  start: boolean;
}

export interface LogPage {
  lines: string[];
  total: number;
  offset: number;
  next: number | null;
  clipped: boolean;
}

export interface PresetBot {
  key: string;
  name: string;
  role: string;
  instructions: string;
  capabilities: string[];
  skill: string | null;
  attribution: string | null;
}

export interface Preset {
  id: string;
  title: string;
  description: string;
  bots: PresetBot[];
  group: { title: string; coordinator: string; limits: unknown } | null;
  criteria: Criterion[];
  workspace: "project" | "none";
}

export interface AppliedPreset {
  preset: Preset;
  bots: { key: string; bot_id: string }[];
  room: { id: string; title: string } | null;
  criteria: Criterion[];
  workspace: string;
}

export interface MissionEventPayload {
  mission_id: string;
  state: MissionState;
  revision: number;
}

// ── Pure helpers (tested) ──────────────────────────────────────────────

export type Tone = "blue" | "orange" | "green" | "red" | "grey";

export function missionStateKey(state: MissionState): string {
  return `mission.state.${state}`;
}

export function missionTone(state: MissionState): Tone {
  switch (state) {
    case "queued":
    case "running":
    case "verifying":
      return "blue";
    case "blocked":
    case "partial":
    case "paused":
      return "orange";
    case "succeeded":
      return "green";
    case "failed":
      return "red";
    default:
      return "grey";
  }
}

export function isFinal(state: MissionState): boolean {
  return state === "succeeded" || state === "failed" || state === "cancelled";
}

export function isWorking(state: MissionState): boolean {
  return state === "queued" || state === "running" || state === "verifying";
}

export type MissionFilter = "all" | "active" | "attention" | "done";
export const MISSION_FILTERS: MissionFilter[] = ["all", "active", "attention", "done"];

export function matchesMissionFilter(m: Pick<Mission, "state">, filter: MissionFilter): boolean {
  switch (filter) {
    case "active":
      return isWorking(m.state) || m.state === "draft";
    case "attention":
      return m.state === "blocked" || m.state === "partial" || m.state === "paused" || m.state === "failed";
    case "done":
      return isFinal(m.state);
    default:
      return true;
  }
}

export type MissionAction = "start" | "pause" | "resume" | "cancel" | "verify" | "delete";

/** The buttons a mission in this state offers, in display order. */
export function missionActions(state: MissionState): MissionAction[] {
  switch (state) {
    case "draft":
      return ["start", "cancel", "delete"];
    case "queued":
    case "running":
      return ["pause", "verify", "cancel"];
    case "verifying":
      return ["pause", "cancel"];
    case "paused":
    case "blocked":
    case "partial":
      return ["resume", "verify", "cancel"];
    default:
      return ["delete"];
  }
}

export function taskStateKey(state: TaskState): string {
  return `mission.task_state.${state}`;
}

export function taskTone(state: TaskState): Tone {
  switch (state) {
    case "claimed":
    case "running":
      return "blue";
    case "blocked":
      return "orange";
    case "done":
      return "green";
    case "failed":
      return "red";
    default:
      return "grey";
  }
}

export function criterionStatusKey(status: CriterionStatus): string {
  return `mission.status.${status}`;
}

export function criterionTone(status: CriterionStatus): Tone {
  switch (status) {
    case "pass":
      return "green";
    case "fail":
      return "red";
    case "awaiting_human":
    case "stale":
      return "orange";
    default:
      return "grey";
  }
}

export function completionKey(c: Completion): string {
  return c === "human" ? "mission.completion.human" : "mission.completion.auto";
}

/** Only criteria the person or a preset wrote may run a command. */
export function criterionExecutable(c: Pick<Criterion, "kind" | "origin">): boolean {
  if (c.kind !== "command") return true;
  return c.origin === "user" || c.origin === "preset";
}

export function acceptanceNeedsHuman(c: Pick<Criterion, "kind" | "acceptance">): boolean {
  return c.kind === "human" || c.kind === "rubric" || c.acceptance === "human";
}

export interface VerdictRow extends VerdictCriterion {
  tone: Tone;
  statusKey: string;
  origin: Origin | null;
  executable: boolean;
  needsDecision: boolean;
}

/** Verdict criteria joined with their definitions: required first, then advisory. */
export function verdictRows(verdict: Verdict | null, criteria: Criterion[]): VerdictRow[] {
  const byId = new Map(criteria.map((c) => [c.id, c]));
  const rows: VerdictRow[] = (verdict?.criteria ?? []).map((v) => {
    const def = byId.get(v.id);
    return {
      ...v,
      tone: criterionTone(v.status),
      statusKey: criterionStatusKey(v.status),
      origin: def?.origin ?? null,
      executable: def ? criterionExecutable(def) : true,
      needsDecision: v.status === "awaiting_human",
    };
  });
  // Criteria the verdict does not mention yet are shown as missing, never dropped.
  for (const c of criteria) {
    if (rows.some((r) => r.id === c.id)) continue;
    rows.push({
      id: c.id, title: c.title, kind: c.kind, severity: c.severity, acceptance: c.acceptance,
      objective: c.kind === "command" || c.kind === "artifact" || c.kind === "tool_result",
      status: "missing", receipt_id: null, detail: null, confidence: null,
      tone: criterionTone("missing"), statusKey: criterionStatusKey("missing"),
      origin: c.origin, executable: criterionExecutable(c), needsDecision: false,
    });
  }
  const rank = (s: Severity) => (s === "required" ? 0 : 1);
  return rows.sort((a, b) => rank(a.severity) - rank(b.severity));
}

export interface I18nLine {
  key: string;
  params?: Record<string, string | number>;
}

/** One plain sentence about where the verdict stands. */
export function verdictSummary(verdict: Verdict | null): I18nLine {
  if (!verdict) return { key: "mission.verdict.none" };
  if (verdict.passed) return { key: "mission.verdict.passed" };
  if (verdict.failing.length) return { key: "mission.verdict.failing", params: { count: verdict.failing.length } };
  if (verdict.awaiting_human.length) return { key: "mission.verdict.awaiting", params: { count: verdict.awaiting_human.length } };
  if (verdict.missing.length) return { key: "mission.verdict.missing", params: { count: verdict.missing.length } };
  return { key: "mission.verdict.pending" };
}

/**
 * The list row's verdict lines. With `counts` (what the backend counted from
 * the verdict, like the detail's `verdictSummary`) they are exact; the
 * English summary (`failing: a, b; not verified yet: c; waiting for your
 * decision: d`) is only a fallback for an older backend, and miscounts a
 * criterion whose title has a comma.
 */
export function rowVerdictLines(summary: string, counts?: VerdictCounts | null): I18nLine[] {
  if (counts) {
    if (counts.passed) return [{ key: "mission.verdict.passed" }];
    const out: I18nLine[] = [];
    if (counts.failing) out.push({ key: "mission.verdict.failing", params: { count: counts.failing } });
    if (counts.missing) out.push({ key: "mission.verdict.missing", params: { count: counts.missing } });
    if (counts.awaiting_human) out.push({ key: "mission.verdict.awaiting", params: { count: counts.awaiting_human } });
    return out;
  }
  const s = summary.trim();
  if (!s) return [];
  if (s === "every required criterion passed") return [{ key: "mission.verdict.passed" }];
  const out: I18nLine[] = [];
  for (const part of s.split(";")) {
    const [label, rest = ""] = part.split(":");
    const count = rest.split(",").map((x) => x.trim()).filter(Boolean).length;
    const l = label.trim();
    if (l === "failing") out.push({ key: "mission.verdict.failing", params: { count } });
    else if (l === "not verified yet") out.push({ key: "mission.verdict.missing", params: { count } });
    else if (l === "waiting for your decision") out.push({ key: "mission.verdict.awaiting", params: { count } });
  }
  return out;
}

export function formatUsd(v: number): string {
  if (v > 0 && v < 0.01) return `$${v.toFixed(4)}`;
  return `$${v.toFixed(2)}`;
}

/**
 * The money spent, never pretending unknown cost is free: with calls of
 * unknown cost and nothing known, there is no dollar figure at all.
 */
export function spendLine(spent: Pick<MissionSpent, "usd_known" | "unknown_cost_calls">): I18nLine {
  const unknown = spent.unknown_cost_calls ?? 0;
  const known = spent.usd_known ?? 0;
  if (unknown > 0 && known <= 0) return { key: "mission.budget.unknown_only", params: { calls: unknown } };
  if (unknown > 0) return { key: "mission.budget.known_plus_unknown", params: { usd: formatUsd(known), calls: unknown } };
  return { key: "mission.budget.known", params: { usd: formatUsd(known) } };
}

/** The limits a mission carries, one line each (empty when unlimited). */
export function budgetLimits(budget: Partial<MissionBudget> | null | undefined): I18nLine[] {
  if (!budget) return [];
  const out: I18nLine[] = [];
  if (budget.usd != null) out.push({ key: "mission.budget.limit_usd", params: { usd: formatUsd(budget.usd) } });
  if (budget.tokens != null) out.push({ key: "mission.budget.limit_tokens", params: { tokens: budget.tokens } });
  if (budget.turns != null) out.push({ key: "mission.budget.limit_turns", params: { turns: budget.turns } });
  if (budget.max_minutes != null) out.push({ key: "mission.budget.limit_minutes", params: { minutes: budget.max_minutes } });
  return out;
}

/** Share of the dollar limit used, only when the cost is fully known. */
export function budgetRatio(spent: MissionSpent, budget: MissionBudget): number | null {
  if (budget.usd == null || budget.usd <= 0 || spent.unknown_cost_calls > 0) return null;
  return Math.min(1, spent.usd_known / budget.usd);
}

/** Parses a budget number field: blank → null, negative or garbage → undefined (invalid). */
export function parseBudgetField(raw: string | number | null | undefined, integer = false): number | null | undefined {
  if (raw == null) return null;
  const s = String(raw).trim();
  if (!s) return null;
  const n = Number(s.replace(",", "."));
  if (!Number.isFinite(n) || n < 0) return undefined;
  return integer ? Math.round(n) : n;
}

export type ActionPlan =
  | { kind: "command"; command: string; args: Record<string, unknown>; confirm: boolean }
  | { kind: "route"; route: string; confirm: boolean }
  | { kind: "local"; local: "edit_criteria" | "edit_context" | "edit_form"; confirm: boolean }
  | { kind: "unknown"; confirm: boolean };

export function blockedTaskId(tasks: Pick<MissionTask, "id" | "state">[]): string | null {
  return tasks.find((t) => t.state === "blocked")?.id ?? null;
}

/** A diagnosis' suggested action → what the button does. */
export function actionPlan(action: Pick<SuggestedAction, "id" | "needs_authorization">, missionId: string, tasks: Pick<MissionTask, "id" | "state">[]): ActionPlan {
  const confirm = !!action.needs_authorization;
  const taskId = blockedTaskId(tasks);
  switch (action.id) {
    case "resume":
      return { kind: "command", command: "assist_mission_resume", args: { id: missionId }, confirm };
    case "cancel":
      return { kind: "command", command: "assist_mission_cancel", args: { id: missionId }, confirm };
    case "unblock_confirmed":
      if (!taskId) return { kind: "unknown", confirm };
      return { kind: "command", command: "assist_mission_unblock_task", args: { id: missionId, taskId, confirmed: true }, confirm };
    case "unblock_retry":
      if (!taskId) return { kind: "unknown", confirm };
      return { kind: "command", command: "assist_mission_unblock_task", args: { id: missionId, taskId, confirmed: false }, confirm };
    case "allow_replay":
      // Replaying on another account/folder/runtime always asks first.
      return { kind: "command", command: "assist_mission_allow_replay", args: { id: missionId }, confirm: true };
    case "edit_criteria":
      return { kind: "local", local: "edit_criteria", confirm };
    case "edit_context":
      return { kind: "local", local: "edit_context", confirm };
    case "raise_budget":
      return { kind: "local", local: "edit_form", confirm };
    case "open_bot":
    case "change_bot":
      return { kind: "route", route: "/llm/roster", confirm };
    case "open_mcp":
      return { kind: "route", route: "/llm/mcp", confirm };
    default:
      return { kind: "unknown", confirm };
  }
}

let seq = 0;
function freshId(prefix: string): string {
  seq += 1;
  return `${prefix}${Date.now().toString(36)}${seq}`;
}

/** A blank criterion of this kind, as the person writes it. */
export function newCriterion(kind: CriterionKind): Criterion {
  const spec: Record<string, unknown> =
    kind === "command" ? { command: "" }
    : kind === "artifact" ? { path: "", contains: [], must_exist: true }
    : kind === "tool_result" ? { check: "reading_round", journey_id: "", min_items: 1, max_items: 5 }
    : kind === "rubric" ? { rubric: [] }
    : { prompt: "" };
  return {
    id: freshId("c"),
    version: 1,
    kind,
    severity: "required",
    title: "",
    spec,
    origin: "user",
    acceptance: kind === "rubric" || kind === "human" ? "human" : "auto",
  };
}

/** The create form checked before any call: the i18n key of the problem, or null. */
export function validateDraft(d: Pick<NewMission, "objective" | "criteria">): string | null {
  if (!d.objective.trim()) return "mission.create.err_objective";
  if (!d.criteria.some((c) => c.severity === "required")) return "mission.create.err_required";
  for (const c of d.criteria) {
    if (!c.title.trim()) return "mission.create.err_title";
    const s = c.spec ?? {};
    if (c.kind === "command" && !String(s.command ?? "").trim()) return "mission.create.err_command";
    if (c.kind === "artifact") {
      const p = String(s.path ?? "").trim();
      if (!p) return "mission.create.err_path";
      if (p.startsWith("/") || /^[A-Za-z]:[\\/]/.test(p) || p.split(/[\\/]/).includes("..")) return "mission.create.err_path_relative";
    }
    if (c.kind === "tool_result" && !String(s.journey_id ?? "").trim()) return "mission.create.err_journey";
    if (c.kind === "rubric" && !(Array.isArray(s.rubric) && s.rubric.some((x) => String(x).trim()))) return "mission.create.err_rubric";
    if (c.kind === "human" && !String(s.prompt ?? "").trim()) return "mission.create.err_prompt";
  }
  return null;
}

/** Splits a textarea into trimmed non-empty lines (rubric items, `contains`). */
export function lines(text: string): string[] {
  return text.split("\n").map((l) => l.trim()).filter(Boolean);
}

/** One-line human description of a criterion's spec (the details stay collapsed). */
export function specSummary(c: Pick<Criterion, "kind" | "spec">): string {
  const s = c.spec ?? {};
  switch (c.kind) {
    case "command":
      return String(s.command ?? "");
    case "artifact": {
      const extra = Array.isArray(s.contains) && s.contains.length ? ` ∋ ${(s.contains as string[]).join(", ")}` : "";
      return `${String(s.path ?? "")}${extra}`;
    }
    case "tool_result":
      return `${String(s.check ?? "")} ${s.min_items ?? "?"}–${s.max_items ?? "?"}`;
    case "rubric":
      return Array.isArray(s.rubric) ? (s.rubric as string[]).join(" · ") : "";
    case "human":
      return String(s.prompt ?? "");
  }
}

// ── Backend default texts, localized where known (tested) ──────────────

/** Presets the backend ships (`packs/presets.rs`); their texts have i18n keys. */
export const KNOWN_PRESETS = ["dev-review", "research-sources", "reading-curation"];
/** Preset criterion ids whose default title has an i18n key. */
const KNOWN_PRESET_CRITERIA = ["tests", "review", "answer", "accept", "round", "fit"];

function presetKey(id: string): string {
  return id.replace(/-/g, "_");
}

/** `mission.presets.item.<id>.<field>` for a shipped preset, else null (show the backend text). */
export function presetTextKey(presetId: string, field: "title" | "description" | "group"): string | null {
  return KNOWN_PRESETS.includes(presetId) ? `mission.presets.item.${presetKey(presetId)}.${field}` : null;
}

/**
 * A criterion title the backend wrote by default (a controller's "Artifact 2",
 * a preset's criterion, the driver's deliverable), as an i18n line; null for
 * a title a person wrote.
 */
export function criterionTitleLine(c: Pick<Criterion, "id" | "title" | "origin"> & { spec?: Record<string, unknown> }): I18nLine | null {
  const artifact = /^(?:Artifact|File) (\d+)$/.exec(c.title);
  if (artifact && /^artifact-\d+$/.test(c.id)) {
    // A controller's numbered default reads better as the file it checks.
    const name = pathName(String(c.spec?.path ?? ""));
    if (name) return { key: "mission.default_title.file_named", params: { name } };
    return { key: "mission.default_title.artifact", params: { count: Number(artifact[1]) } };
  }
  if (c.origin === "preset" && KNOWN_PRESET_CRITERIA.includes(c.id)) return { key: `mission.default_title.preset_${c.id}` };
  if (c.title === "Deliverable written") return { key: "mission.default_title.deliverable" };
  return null;
}

/** Task titles the driver writes by itself. */
export function taskTitleLine(title: string): I18nLine | null {
  const round = /^Round (\d+): fix failing checks$/.exec(title);
  if (round) return { key: "mission.default_title.round_fix", params: { count: Number(round[1]) } };
  if (title === "Rework after your review") return { key: "mission.default_title.rework" };
  if (title === "Deliver") return { key: "mission.default_title.deliver" };
  return null;
}

/** The last segment of a relative or absolute path (`out/hello.md` → `hello.md`). */
export function pathName(p: string): string {
  const parts = p.trim().split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? "";
}

/** Side-effect kinds (`missions_effects.kind`) with a label. */
export function effectKindKey(kind: string): string | null {
  return kind === "agent_turn" ? "mission.effect_kind.agent_turn" : null;
}

/** Where a mission came from (`auth_origin`), as a label key; `chat:<id>` → chat. */
export function authOriginKey(origin: string | null | undefined): string | null {
  const head = String(origin ?? "").split(":")[0];
  return ["user_ui", "chat", "trigger", "external_mcp", "mcp"].includes(head)
    ? `mission.auth_origin.${head === "mcp" ? "external_mcp" : head}`
    : null;
}

/** A mission an MCP client created (runs under its execution grant). */
export function isExternalMission(m: Pick<Mission, "auth_origin" | "conversation_id">): boolean {
  const o = m.auth_origin ?? "";
  return o === "external_mcp" || o === "mcp" || (m.conversation_id ?? "").startsWith("external-mcp-");
}

/** The client name of an external mission (never an id or a token when a name is known). */
export function externalClientName(m: Pick<Mission, "principal">, names: Record<string, string>): string | null {
  const id = m.principal ?? "";
  return id ? names[id] ?? null : null;
}

// ── Budget revision (tested) ───────────────────────────────────────────

/** Block codes whose limit a person can raise from the mission (`raise_budget`). */
const BUDGET_CODES = ["ERR_MISSION_BUDGET", "ERR_LLM_BUDGET", "EXTERNAL_MISSION_BUDGET_EXCEEDED"];

/**
 * Which limit stopped the mission: `minutes` for the time limit, `tokens`
 * for spend; null when the block is not about a limit this mission owns.
 */
export function budgetBlockKind(block: Pick<Diagnosis, "code" | "summary" | "detail" | "suggested_actions"> | null | undefined): "minutes" | "tokens" | null {
  if (!block) return null;
  const raisable = BUDGET_CODES.includes(block.code) || block.suggested_actions.some((a) => a.id === "raise_budget");
  if (!raisable) return null;
  const detail = typeof block.detail === "string" ? block.detail : JSON.stringify(block.detail ?? "");
  if (/max_minutes=|time limit/i.test(`${detail} ${block.summary}`)) return "minutes";
  return "tokens";
}

/** Minutes of work used so far (running/verifying only). */
export function minutesUsed(spent: Pick<MissionSpent, "active_ms" | "active_since_ms">, now = Date.now()): number {
  const live = spent.active_since_ms ? Math.max(0, now - spent.active_since_ms) : 0;
  return Math.round((((spent.active_ms ?? 0) + live) / 60_000) * 10) / 10;
}

export type BudgetRevision = { budget: NewBudget; error: null } | { budget: null; error: string };

/**
 * The inline "raise the limits" form → the `budget` argument. Blank keeps
 * the current value (left out); a value must be a whole number above zero.
 */
export function budgetRevision(raw: { tokens?: string | number | null; minutes?: string | number | null }): BudgetRevision {
  const tokens = parseBudgetField(raw.tokens, true);
  const minutes = parseBudgetField(raw.minutes, true);
  if (tokens === undefined || minutes === undefined || tokens === 0 || minutes === 0) {
    return { budget: null, error: "mission.budget_form.err_number" };
  }
  if (tokens == null && minutes == null) return { budget: null, error: "mission.budget_form.err_empty" };
  const budget: NewBudget = {};
  if (tokens != null) budget.tokens = tokens;
  if (minutes != null) budget.max_minutes = minutes;
  return { budget, error: null };
}

/** A refusal of `assist_mission_revise_budget`, as a plain sentence key. */
export function budgetErrorKey(err: string): string | null {
  if (err.startsWith("EXECUTION_GRANT_EXCEEDED")) return "mission.budget_form.err_grant";
  if (err.startsWith("EXECUTOR_REQUIRED")) return "mission.budget_form.err_executor";
  return null;
}

// ── Tasks stopped before or after their effect (tested) ────────────────

/** Errors that refuse a job before anything is sent: nothing ran. */
const REFUSED_BEFORE_DISPATCH = /\b(ERR_LLM_BUDGET|ERR_MISSION_BUDGET|EXTERNAL_MISSION_BUDGET_EXCEEDED|EXTERNAL_BUDGET_EXCEEDED|EXECUTION_GRANT_EXCEEDED|ERR_TOOL_DENIED|ERR_NO_AGENT|EXECUTION_NOT_GRANTED)\b/;

/**
 * Whether the app truly cannot tell if a blocked task's effect happened, so
 * the person must say: the effect journal has it pending/running/unknown,
 * or the task was blocked as `ERR_MISSION_EFFECT_UNKNOWN`. A job refused
 * before dispatch (budget, denied, no agent) never asks.
 */
export function taskEffectUnknown(
  task: Pick<MissionTask, "id" | "state" | "error">,
  effects: Pick<MissionEffect, "task_id" | "state">[],
): boolean {
  if (task.state !== "blocked") return false;
  const err = task.error ?? "";
  if (err.startsWith("ERR_MISSION_EFFECT_UNKNOWN")) return true;
  if (REFUSED_BEFORE_DISPATCH.test(err)) return false;
  return effects.some((e) => e.task_id === task.id && ["pending", "running", "unknown"].includes(e.state));
}

/** Spend by purpose (`book_spend`): known purposes have a label. */
export function purposeKey(purpose: string): string | null {
  return purpose === "work" || purpose === "review" ? `mission.budget.purpose.${purpose}` : null;
}

/** Side-effect states (`missions_effects.state`) with a label. */
export function effectStateKey(state: string): string | null {
  return ["pending", "running", "confirmed", "failed", "unknown"].includes(state) ? `mission.effect_state.${state}` : null;
}

/** Where a criterion came from, as a tag: a controller's proposal is not "a model's". */
export function originKey(origin: Origin, authOrigin: string | null | undefined): string {
  if (origin === "proposed" && authOrigin === "external_mcp") return "mission.origin.controller";
  return `mission.origin.${origin}`;
}

/** Why the mission waits for you, for the list row: approvals first, then the block. */
export function waitReason(m: Pick<Mission, "state" | "block">, asks: Pick<MissionAsk, "tool">[]): I18nLine | null {
  if (asks.length) return { key: "mission.approval.row", params: { count: asks.length, tool: asks[0].tool } };
  if ((m.state === "blocked" || m.state === "partial" || m.state === "paused") && m.block?.summary) {
    return { key: "mission.list.waits_reason", params: { reason: m.block.summary } };
  }
  return null;
}

// ── Paths typed by hand (tested) ───────────────────────────────────────

/** An absolute local path (`/x`, `C:\x`, `\\server\x`); `~` is not expanded by the backend. */
export function isAbsolutePath(p: string): boolean {
  const s = p.trim();
  return s.startsWith("/") || /^[A-Za-z]:[\\/]/.test(s) || s.startsWith("\\\\");
}

/** Middle-truncates a long path for a button label; the full path goes in a tooltip. */
export function shortPath(p: string, max = 48): string {
  if (p.length <= max) return p;
  const keep = Math.max(4, Math.floor((max - 1) / 2));
  return `${p.slice(0, keep)}…${p.slice(p.length - keep)}`;
}

// ── Tool approvals of a mission (tested) ───────────────────────────────

/** A pending `GrantMode::Ask` question (`llm_tool_asks_pending`). */
export interface MissionAsk {
  agent: string;
  request_id: string;
  tool_call_id: string;
  tool: string;
  preview?: string;
}

/** The job fields that tie a question to a mission. */
export interface MissionJobRef {
  id: string;
  conversation_id: string;
  request_id: string | null;
}

/**
 * The mission's turns run in its conversation or a per-bot sub-conversation
 * (`<conversation>-<bot>`, see `conversation_for` in `src-tauri/src/missions.rs`).
 */
export function jobOfMission(job: Pick<MissionJobRef, "id" | "conversation_id">, mission: Pick<Mission, "conversation_id">, jobIds: string[] = []): boolean {
  if (jobIds.includes(job.id)) return true;
  const conv = mission.conversation_id;
  if (!conv) return false;
  return job.conversation_id === conv || job.conversation_id.startsWith(`${conv}-`);
}

/** The pending questions raised by this mission's turns. */
export function missionAsks(asks: MissionAsk[], jobs: MissionJobRef[], mission: Pick<Mission, "conversation_id">, jobIds: string[] = []): MissionAsk[] {
  const requests = new Set(
    jobs.filter((j) => j.request_id && jobOfMission(j, mission, jobIds)).map((j) => j.request_id as string),
  );
  return asks.filter((a) => requests.has(a.request_id));
}

/** True when this question belongs to a mission conversation (the chat must not drop it). */
export function askBelongsToMission(ask: Pick<MissionAsk, "request_id">, jobs: MissionJobRef[], missions: Pick<Mission, "conversation_id">[]): boolean {
  const job = jobs.find((j) => j.request_id === ask.request_id);
  if (!job) return false;
  return missions.some((m) => jobOfMission(job, m));
}

export function shortTime(ms: number | null | undefined): string {
  if (!ms) return "—";
  const d = new Date(ms);
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  return String((err as { message?: string })?.message ?? err ?? "");
}

// ── Learning helpers (tested) ──────────────────────────────────────────

export type CandidateState = "proposed" | "evaluating" | "eligible" | "active" | "rejected" | "reverted";
export interface EvalRunLite {
  id: string;
  candidate_version: number;
  decision: "eligible" | "rejected" | "insufficient" | "unsupported";
  created_ms?: number;
}

/** Promote only with an eligible evaluation of the version on screen, the latest one. */
export function promotableRun(currentVersion: number, runs: EvalRunLite[]): EvalRunLite | null {
  const mine = runs.filter((r) => r.candidate_version === currentVersion);
  if (!mine.length) return null;
  const latest = mine.reduce((a, b) => ((b.created_ms ?? 0) >= (a.created_ms ?? 0) ? b : a));
  return latest.decision === "eligible" ? latest : null;
}

/**
 * A procedure version people can tell apart: the candidate's title, its
 * version and a short id. Two candidates are both "v1"; the version number
 * alone never says which one is active.
 */
export function procedureVersionParams(
  candidates: { id: string; title: string }[],
  candidateId: string | null,
  version: number | null,
): { title: string; version: string; id: string } {
  const id = candidateId ?? "";
  const title = candidates.find((c) => c.id === id)?.title?.trim() || id.slice(0, 8) || "—";
  return { title, version: version == null ? "—" : String(version), id: id.slice(0, 8) };
}

/** The candidate whose version is the one in use (what `learn_active` points to). */
export function isActiveCandidate(active: { candidate_id: string; version: number }[], c: { id: string }): boolean {
  return active.some((a) => a.candidate_id === c.id);
}

/** Label for an observation: model statements are hypotheses, duplicates count once. */
export function observationLabelKeys(o: { source: string; dup_of: string | null; revoked_ms: number | null }): string[] {
  const out: string[] = [`assist.learning.source.${o.source}`];
  if (o.source === "model") out.push("assist.learning.hypothesis");
  if (o.dup_of) out.push("assist.learning.duplicate");
  if (o.revoked_ms) out.push("assist.learning.revoked");
  return out;
}

export function candidateTone(state: CandidateState): Tone {
  switch (state) {
    case "active":
      return "green";
    case "eligible":
    case "evaluating":
      return "blue";
    case "rejected":
      return "red";
    case "reverted":
      return "orange";
    default:
      return "grey";
  }
}

// ── Pack import helpers (tested) ───────────────────────────────────────

export type PackKind = "skill" | "agent" | "rule" | "context" | "command" | "hook" | "script";
export type PlanAction =
  | "new" | "update" | "unchanged" | "conflict_local_changes" | "conflict_name" | "deferred" | "unsupported" | "refused";

/** Hooks and scripts are listed for transparency but never installed. */
export function packNeverInstalled(kind: string): boolean {
  return kind === "hook" || kind === "script";
}

export function packKey(item: { kind: string; name: string }): string {
  return `${item.kind}:${item.name}`;
}

export function isConflict(action: PlanAction): boolean {
  return action === "conflict_local_changes" || action === "conflict_name";
}

export function planActionTone(action: PlanAction): Tone {
  switch (action) {
    case "new":
      return "green";
    case "update":
      return "blue";
    case "conflict_local_changes":
    case "conflict_name":
    case "deferred":
      return "orange";
    case "refused":
    case "unsupported":
      return "red";
    default:
      return "grey";
  }
}

/** Overwrite only what is a conflict AND was ticked item by item. */
export function overwriteList(items: { key: string; action: PlanAction }[], ticked: Record<string, boolean>): string[] {
  return items.filter((i) => isConflict(i.action) && ticked[i.key]).map((i) => i.key);
}

/** Whether applying the plan would change anything. */
export function planHasWork(items: { key: string; action: PlanAction }[], ticked: Record<string, boolean>): boolean {
  return items.some((i) => i.action === "new" || i.action === "update" || (isConflict(i.action) && !!ticked[i.key]));
}

// ── Live refresh ───────────────────────────────────────────────────────

let tick = $state(0);
let lastEvent = $state<MissionEventPayload | null>(null);
let started = false;

/** Bumps on every `assist://mission` event; read it in an `$effect` to refresh. */
export function getMissionTick(): number {
  return tick;
}
export function getLastMissionEvent(): MissionEventPayload | null {
  return lastEvent;
}

export function initMissionsStore(): void {
  if (started) return;
  started = true;
  void listen<MissionEventPayload>("assist://mission", (ev) => {
    lastEvent = ev.payload;
    tick += 1;
  }).catch(() => {
    started = false;
  });
}

// ── Commands ───────────────────────────────────────────────────────────

export function listMissions(filter: { state?: string; bot_id?: string; conversation_id?: string; limit?: number } = {}): Promise<MissionRow[]> {
  return invoke<MissionRow[]>("assist_mission_list", { filter });
}
export function getMission(id: string): Promise<MissionDetail> {
  return invoke<MissionDetail>("assist_mission_get", { id });
}
export function createMission(draft: NewMission): Promise<MissionDetail> {
  return invoke<MissionDetail>("assist_mission_create", { draft: { auth_origin: "user_ui", ...draft } });
}
export function missionFromChat(input: { conversation_id: string; bot_id: string; objective: string; criteria: Criterion[]; budget?: NewBudget; start: boolean }): Promise<MissionDetail> {
  return invoke<MissionDetail>("assist_mission_from_chat", { input });
}
export function startMission(id: string): Promise<unknown> {
  return invoke("assist_mission_start", { id });
}
export function pauseMission(id: string): Promise<unknown> {
  return invoke("assist_mission_pause", { id });
}
export function resumeMission(id: string, note?: string | null): Promise<unknown> {
  return invoke("assist_mission_resume", { id, note: note?.trim() ? note.trim() : null });
}
export function cancelMission(id: string): Promise<unknown> {
  return invoke("assist_mission_cancel", { id });
}
export function deleteMission(id: string): Promise<unknown> {
  return invoke("assist_mission_delete", { id });
}
export function noteMission(id: string, text: string): Promise<unknown> {
  return invoke("assist_mission_note", { id, text });
}
export function reviseCriteria(id: string, criteria: Criterion[], reason: string): Promise<unknown> {
  return invoke("assist_mission_revise_criteria", { id, criteria, reason });
}
export function acceptCriterion(id: string, criterionId: string, accept: boolean, note?: string | null): Promise<unknown> {
  return invoke("assist_mission_accept", { id, criterionId, accept, note: note?.trim() ? note.trim() : null });
}
export function unblockTask(id: string, taskId: string, confirmed: boolean | null, note?: string | null): Promise<unknown> {
  return invoke("assist_mission_unblock_task", { id, taskId, confirmed, note: note?.trim() ? note.trim() : null });
}
export function allowReplay(id: string): Promise<unknown> {
  return invoke("assist_mission_allow_replay", { id });
}
/** Starts a re-check in the background; the result arrives as `assist://mission` events. */
export function verifyMission(id: string): Promise<{ mission: Mission; started: boolean; already_running: boolean }> {
  return invoke("assist_mission_verify", { id });
}
export function missionDiagnostics(id: string, path?: string | null): Promise<unknown> {
  return invoke("assist_mission_diagnostics", { id, path: path ?? null });
}
export function missionLog(id: string, taskId: string, offset = 0, limit = 200): Promise<LogPage> {
  return invoke<LogPage>("assist_mission_log", { id, taskId, offset, limit });
}
export function missionPresets(): Promise<Preset[]> {
  return invoke<Preset[]>("assist_mission_presets");
}
export function applyPreset(input: { preset_id: string; connection_agent_id: string; with_group: boolean }): Promise<AppliedPreset> {
  return invoke<AppliedPreset>("assist_mission_apply_preset", { input });
}
/** Raises (or sets) the limits; fields left out keep their value. With `resume`, a blocked mission continues. */
export function reviseBudget(id: string, budget: NewBudget, resume: boolean): Promise<MissionDetail> {
  return invoke<MissionDetail>("assist_mission_revise_budget", { id, budget, resume });
}

/** MCP client id → display name, from the execution grants (revoked clients included). Never tokens. */
export async function mcpClientNames(): Promise<Record<string, string>> {
  try {
    const r = await invoke<{ grants?: { principal: string; principal_name: string }[] }>("tool_mcp_execution_grants_list");
    const out: Record<string, string> = {};
    for (const g of r?.grants ?? []) if (g.principal && g.principal_name) out[g.principal] = g.principal_name;
    return out;
  } catch {
    return {};
  }
}

export function groupPlan(roomId: string, objective: string): Promise<NewTask[]> {
  return invoke<NewTask[]>("assist_mission_group_plan", { roomId, objective });
}

/** Runs a planned command action (the only generic invoke of the store). */
export function runActionCommand(plan: Extract<ActionPlan, { kind: "command" }>): Promise<unknown> {
  return invoke(plan.command, plan.args);
}
