// Typed wrappers of the city commands (`src-tauri/src/commands/city3d/city.rs`,
// agent G2) and the two events. Every mutation of the city goes through the
// same core pipeline an MCP agent uses (apply → solver → checker → policy).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Blueprint,
  Charter,
  CityProgram,
  InstanceList,
  PatchOp,
  Proposal,
  ProposalResult,
  TerrainMesh,
  Violation,
} from "./types";

export const EVENT_FOUNDING = "city3d:founding";
export const EVENT_CHANGED = "city3d:changed";
export const EVENT_COUNCIL = "city3d:council";

export type Region = [number, number];
export type PublishPolicy = "auto" | "review";
export type Role = "prefeitura" | "conselho" | "morador";

export interface RoleBudget {
  limit_tokens: number;
  used_tokens: number;
  period_start_ms: number;
  period_ms: number;
}

export interface CitySettings {
  council_every_min: number | null;
  works_rate_per_min: number;
  works_last_ms: number;
}

export interface CityInfo {
  city_id: string;
  name?: string;
  publish_policy?: PublishPolicy;
  charter?: Charter | null;
  regions?: Region[];
  pending_review?: number;
  works?: number;
  settings?: CitySettings;
  budgets?: Record<string, RoleBudget>;
  /** Server mode: the instance URL. */
  server?: string;
}

export interface CityStatus {
  mode: "local" | "server";
  city: CityInfo | null;
  server?: { base: string; city: string };
}

export interface FoundResult {
  founded: boolean;
  city_id: string;
  violations: Violation[];
  revisions: [number[], string][];
}

export interface FoundingEvent {
  job: string;
  /** terrain | asking | checking | retry | founded | failed */
  stage: string;
  message: string;
  violations: Violation[];
  done: boolean;
  error?: string;
  attempt: number;
  tokens: number;
  city_id?: string;
}

export interface ChangedEvent {
  region: Region;
  revision: string;
}

export interface RegionData {
  program: CityProgram;
  revision: string;
  blueprint: Blueprint | null;
  violations: Violation[];
}

export interface RegionMesh {
  revision: string;
  instances: InstanceList;
  terrain: TerrainMesh;
}

export type ProposalState =
  | "draft"
  | "applied"
  | "rejected"
  | "pending_review"
  | "refused"
  | "stale"
  | "founded"
  | "work_done";

export interface Outcome extends ProposalResult {
  log_id: string | null;
  state: ProposalState;
}

export interface Author {
  /** person | llm | mcp | system */
  kind: string;
  name: string;
  role?: string | null;
}

export interface LogEntry {
  id: string;
  at_ms: number;
  region: Region;
  author: Author;
  reason: string;
  ops: PatchOp[];
  base_revision: string;
  revision: string | null;
  violations: Violation[];
  state: ProposalState;
  summary: Record<string, unknown> | null;
  admin: boolean;
  idempotency_key: string | null;
  tokens: number | null;
  resolution: string | null;
}

export interface Work {
  region: Region;
  id: string;
  lot: string;
  use: string;
  label: string | null;
  progress: number;
}

export interface CityLog {
  proposals: LogEntry[];
  works: Work[];
}

export interface RoleStep {
  role: Role;
  state: string;
  reason: string;
  log_id: string | null;
  violations: Violation[];
  tokens: number;
  message: string;
}

export interface CouncilReport {
  region: Region;
  model: string;
  steps: RoleStep[];
}

export interface ModelChoice {
  provider: string;
  model: string;
  local: boolean;
}

export type View = "top" | "n" | "e" | "s" | "w";

// ── Status and founding ─────────────────────────────────────────────────

export const status = () => invoke<CityStatus>("city3d_status");

export const providers = () => invoke<ModelChoice[]>("city3d_providers");

export const foundExample = (opts: { cityId?: string; seed?: string; name?: string } = {}) =>
  invoke<FoundResult>("city3d_found_example", {
    cityId: opts.cityId ?? null,
    seed: opts.seed ?? null,
    name: opts.name ?? null,
  });

export const foundStart = (opts: {
  brief: string;
  provider?: string | null;
  model?: string | null;
  cityId?: string | null;
  seed?: string | null;
  attempts?: number | null;
}) =>
  invoke<{ job: string; model: string }>("city3d_found_start", {
    brief: opts.brief,
    provider: opts.provider ?? null,
    model: opts.model ?? null,
    cityId: opts.cityId ?? null,
    seed: opts.seed ?? null,
    attempts: opts.attempts ?? null,
  });

export const foundCancel = (job: string) => invoke<boolean>("city3d_found_cancel", { job });

// ── Regions and proposals ───────────────────────────────────────────────

export const region = (r: Region) => invoke<RegionData>("city3d_region", { region: r });

export const regionMesh = (r: Region) => invoke<RegionMesh>("city3d_region_mesh", { region: r });

export const propose = (proposal: Proposal, reason?: string) =>
  invoke<Outcome>("city3d_propose", { proposal, reason: reason ?? null });

export const apply = (proposal: Proposal, reason?: string) =>
  invoke<Outcome>("city3d_apply", { proposal, reason: reason ?? null });

export const review = (id: string, approve: boolean, reason?: string) =>
  invoke<Outcome | { log_id: string; state: "refused" }>("city3d_review", {
    id,
    approve,
    reason: reason ?? null,
  });

export const preview = (r: Region, view: View = "top", px = 512) =>
  invoke<{ png_base64: string }>("city3d_preview", { region: r, view, px });

export const log = (limit = 100) => invoke<CityLog>("city3d_log", { limit });

export const setPolicy = (policy: PublishPolicy) =>
  invoke<{ publish_policy: PublishPolicy }>("city3d_policy_set", { policy });

// ── Roles and construction ──────────────────────────────────────────────

export const councilTick = (
  opts: { region?: Region; roles?: Role[]; provider?: string | null; model?: string | null } = {},
) =>
  invoke<CouncilReport>("city3d_council_tick", {
    region: opts.region ?? null,
    roles: opts.roles ?? null,
    provider: opts.provider ?? null,
    model: opts.model ?? null,
  });

export const councilSchedule = (minutes: number | null) =>
  invoke<{ council_every_min: number | null }>("city3d_council_schedule", { minutes });

export const budgetSet = (role: Role, limitTokens: number) =>
  invoke<Record<string, RoleBudget>>("city3d_budget_set", { role, limitTokens });

export const worksTick = () =>
  invoke<{ advanced: { region: Region; id: string; progress: number }[]; points?: number; mode?: string }>(
    "city3d_works_tick",
  );

/** Advances local construction while the city is open. Returns the stop. */
export function startWorksTicker(everyMs = 5000): () => void {
  let stopped = false;
  const id = setInterval(() => {
    if (!stopped) void worksTick().catch(() => {});
  }, everyMs);
  void worksTick().catch(() => {});
  return () => {
    stopped = true;
    clearInterval(id);
  };
}

// ── Server mode ─────────────────────────────────────────────────────────

export const serverConnect = (city: string, server?: string | null) =>
  invoke<{ connected: boolean; base: string; city: string; cities: { city_id: string; name: string }[] }>(
    "city3d_server_connect",
    { city, server: server ?? null },
  );

export const serverDisconnect = () => invoke<{ connected: false }>("city3d_server_disconnect");

export const serverStatus = () =>
  invoke<{ connected: boolean; base?: string; city?: string }>("city3d_server_status");

// ── Events ──────────────────────────────────────────────────────────────

export function onFounding(cb: (e: FoundingEvent) => void): Promise<UnlistenFn> {
  return listen<FoundingEvent>(EVENT_FOUNDING, (e) => cb(e.payload));
}

export function onChanged(cb: (e: ChangedEvent) => void): Promise<UnlistenFn> {
  return listen<ChangedEvent>(EVENT_CHANGED, (e) => cb(e.payload));
}

export function onCouncil(cb: (e: CouncilReport | { error: string }) => void): Promise<UnlistenFn> {
  return listen<CouncilReport | { error: string }>(EVENT_COUNCIL, (e) => cb(e.payload));
}

/** Errors come back as `ERR_CITY3D_<CODE>: message`. */
export function errorCode(e: unknown): string {
  const text = String((e as { message?: string })?.message ?? e);
  return text.match(/^(ERR_[A-Z0-9_]+)/)?.[1] ?? "ERR_CITY3D";
}
