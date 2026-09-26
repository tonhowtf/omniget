/**
 * Bots: identity (profile), skill bindings and the effective capability
 * manifest, over the `assist_bot_*` commands (`src-tauri/src/commands/assist/bots.rs`).
 *
 * The manifest is the only source of "available": a ticked capability is
 * shown as available only when the backend says its tools are really offered
 * this turn. The pure helpers below turn backend states into i18n keys and
 * fix actions; they carry the logic and are what the tests cover.
 */
import { invoke } from "@tauri-apps/api/core";
import type { AgentDef } from "$lib/llm/types";

export type Capability = "memory" | "web" | "reading" | "project_code" | "delegate";
export const CAPABILITIES: Capability[] = ["memory", "web", "reading", "project_code", "delegate"];
export type MemoryPolicy = "remember" | "ask" | "read_only";

export interface BotProfile {
  bot_id: string;
  purpose: string;
  instructions: string;
  capabilities: Capability[];
  memory_policy: MemoryPolicy;
  default_connection?: string | null;
  created_at?: number;
  updated_at?: number;
}

export interface Binding {
  bot_id: string;
  skill: string;
  hash: string;
  allow_read: boolean;
  allow_scripts: boolean;
}

export interface BotView {
  agent: AgentDef;
  profile: BotProfile;
  bindings: Binding[];
}

export interface Fix {
  action: string;
  capability?: Capability;
  target?: string;
}

export type CapState = "available" | "partial" | "missing" | "not_requested";

export interface CapabilityStatus {
  id: Capability;
  requested: boolean;
  state: CapState;
  via?: "tools" | "builtin";
  tools: string[];
  missing_tools: string[];
  reason?: string;
  fix?: Fix;
}

export type BindingState =
  | "ready"
  | "drift"
  | "removed"
  | "invalid"
  | "missing_dependency"
  | "read_blocked"
  | "runtime_unsupported"
  | "not_projected";

export interface DepStatus {
  raw: string;
  kind: string;
  satisfied: boolean | null;
  reason?: string;
  fix?: Fix;
  server?: string;
  tool?: string;
  name?: string;
}

export interface BindingStatus {
  skill: string;
  description: string;
  state: BindingState;
  hash: string;
  installed_hash: string | null;
  recorded_hash: string | null;
  version: string | null;
  allow_read: boolean;
  allow_scripts: boolean;
  deps: DepStatus[];
  compatibility: string | null;
  tool: string | null;
  fixes: Fix[];
}

export interface CapabilityReport {
  bot_id: string;
  runtime_kind: "native" | "cli" | "acp";
  runtime: { label: string; version?: string | null; builtin_web: boolean; managed_tools: boolean; mcp_projection: boolean; limits?: string[] };
  projectless: boolean;
  profile: BotProfile;
  capabilities: CapabilityStatus[];
  skills: BindingStatus[];
  offered_tools: string[];
  unavailable_grants: { tool: string; reason: string }[];
}

export interface SkillRead {
  id: number;
  run_id: string | null;
  bot_id: string;
  conversation_id: string | null;
  skill: string;
  hash: string;
  file: string;
  bytes: number;
  ok: boolean;
  error: string | null;
  at: number;
}

export interface NewBot {
  name: string;
  purpose: string;
  instructions: string;
  capabilities: Capability[];
  memory_policy: MemoryPolicy;
  skills: string[];
  connection_agent_id: string;
}

// ── Pure helpers (tested) ──────────────────────────────────────────────

/** i18n key of a capability's name and of its one-line explanation. */
export function capabilityKey(cap: Capability): string {
  return `assist.bots.cap.${cap}`;
}
export function capabilityHintKey(cap: Capability): string {
  return `assist.bots.cap_hint.${cap}`;
}

/** The capabilities a new bot starts with: never code or shell. */
export function defaultCapabilities(): Capability[] {
  return ["memory"];
}

/** A payload the backend accepts, or the i18n key of what is wrong. */
export function validateNewBot(bot: NewBot): string | null {
  const name = bot.name.trim();
  if (!name) return "assist.bots.create.err_name";
  if ([...name].length > 48) return "assist.bots.create.err_name_long";
  if (!bot.connection_agent_id) return "assist.bots.create.err_connection";
  if (bot.purpose.length > 2000) return "assist.bots.create.err_purpose_long";
  return null;
}

/** Only the capability the bot asked for AND the backend reports as offered counts. */
export function isReallyAvailable(status: CapabilityStatus | undefined): boolean {
  return !!status && (status.state === "available" || status.state === "partial");
}

export function capStateKey(s: CapabilityStatus): string {
  if (s.state === "available") return s.via === "builtin" ? "assist.bots.state.builtin" : "assist.bots.state.available";
  if (s.state === "partial") return "assist.bots.state.partial";
  if (s.state === "not_requested") return "assist.bots.state.not_requested";
  return "assist.bots.state.missing";
}

const REASONS = new Set([
  "module_has_no_tools",
  "runtime_cannot_reach_tools",
  "projectless",
  "denied_by_user",
  "tool_not_registered",
  "tool_not_granted",
  "runtime_builtin",
  "capability_off",
  "web_unavailable",
  "memory_unavailable",
  "project_unavailable",
  "shell_unavailable",
  "scripts_not_allowed",
  "mcp_not_granted",
  "mcp_not_connected",
  "not_checkable",
]);

export function reasonKey(reason: string | undefined | null): string | null {
  if (!reason) return null;
  return REASONS.has(reason) ? `assist.bots.reason.${reason}` : "assist.bots.reason.other";
}

export function bindingStateKey(state: BindingState): string {
  return `assist.bots.skill_state.${state}`;
}

/** Tone for a badge: good, warn (fixable), bad (not usable). */
export function bindingTone(state: BindingState): "good" | "warn" | "bad" {
  if (state === "ready") return "good";
  if (state === "removed" || state === "invalid" || state === "runtime_unsupported") return "bad";
  return "warn";
}

export type FixKind = "command" | "route" | "local";

/** What a fix button does: run a command here, open a page, or change a local toggle. */
export function fixPlan(fix: Fix): { kind: FixKind; labelKey: string; route?: string } {
  switch (fix.action) {
    case "enable_capability":
      return { kind: "local", labelKey: "assist.bots.fix.enable_capability" };
    case "allow_scripts":
    case "allow_read":
      return { kind: "local", labelKey: `assist.bots.fix.${fix.action}` };
    case "accept_version":
    case "reinstall":
    case "unbind":
    case "reload_skills":
      return { kind: "command", labelKey: `assist.bots.fix.${fix.action}` };
    case "open_project":
      return { kind: "route", labelKey: "assist.bots.fix.open_project", route: "/llm" };
    case "switch_connection":
      return { kind: "local", labelKey: "assist.bots.fix.switch_connection" };
    case "add_mcp":
    case "grant_tool":
      return { kind: "route", labelKey: `assist.bots.fix.${fix.action}`, route: "/llm/mcp" };
    case "check_capability":
      return { kind: "local", labelKey: "assist.bots.fix.check_capability" };
    default:
      return { kind: "local", labelKey: "assist.bots.fix.other" };
  }
}

/** Short version badge: metadata version when declared, else the hash prefix. */
export function versionLabel(s: { version: string | null; hash: string; installed_hash?: string | null }): string {
  const h = (s.installed_hash || s.hash || "").slice(0, 8);
  return s.version ? `${s.version} · ${h}` : h;
}

/** Roster agents a bot can start on: the ones that carry a connection. */
export function connectionAgents(agents: AgentDef[]): AgentDef[] {
  return agents.filter((a) => {
    if (a.runtime?.kind === "cli" || a.runtime?.kind === "acp") return true;
    if (a.model?.policy === "fixed") return !!a.model.model.model && a.model.model.provider !== "fake";
    return a.model?.policy === "route" && a.model.chain.length > 0;
  });
}

export function errorText(err: unknown): string {
  return String((err as { message?: string })?.message ?? err ?? "");
}

// ── Commands ───────────────────────────────────────────────────────────

export function createBot(bot: NewBot): Promise<BotView> {
  return invoke<BotView>("assist_bot_create", { bot });
}
export function getBot(botId: string): Promise<BotView> {
  return invoke<BotView>("assist_bot_get", { botId });
}
export function saveProfile(botId: string, profile: BotProfile): Promise<BotProfile> {
  return invoke<BotProfile>("assist_bot_save_profile", { botId, profile });
}
export function setConnection(botId: string, connectionAgentId: string): Promise<BotView> {
  return invoke<BotView>("assist_bot_set_connection", { botId, connectionAgentId });
}
export function bindSkill(botId: string, skill: string): Promise<BotView> {
  return invoke<BotView>("assist_bot_bind_skill", { botId, skill });
}
export function unbindSkill(botId: string, skill: string): Promise<BotView> {
  return invoke<BotView>("assist_bot_unbind_skill", { botId, skill });
}
export function setSkillGrants(botId: string, skill: string, allowRead: boolean, allowScripts: boolean): Promise<BotView> {
  return invoke<BotView>("assist_bot_skill_grants", { botId, skill, allowRead, allowScripts });
}
export function capabilities(botId: string, conversationId?: string | null): Promise<CapabilityReport> {
  return invoke<CapabilityReport>("assist_bot_capabilities", { botId, conversationId: conversationId ?? null });
}
export function skillReads(botId: string, limit = 20): Promise<SkillRead[]> {
  return invoke<SkillRead[]>("assist_bot_skill_reads", { botId, runId: null, limit });
}
export function acceptSkillVersion(name: string): Promise<unknown> {
  return invoke("llm_skills_accept", { name });
}
export function reinstallSkill(name: string): Promise<unknown> {
  return invoke("llm_skills_reinstall", { name });
}
export function reloadSkills(): Promise<unknown> {
  return invoke("llm_skills_reproject");
}
