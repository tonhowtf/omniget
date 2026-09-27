/**
 * Typed wrappers of the `run_*` commands (src-tauri/src/commands/central/run.rs):
 * catalog loops on the Loops engine, playbooks, the project wizard, "Run now"
 * and the sandbox recipe.
 */
import { invoke } from "@tauri-apps/api/core";
import type { InstallPlan } from "$lib/central/catalog";

export type RunnerKind = "agent" | "account" | "acp" | "tool";

export interface RunnerSpec {
  kind: RunnerKind;
  id: string;
  account?: string | null;
  model?: string | null;
  permission?: string | null;
}

export interface RunnerCatalog {
  roster: { id: string; name: string; runtime: string }[];
  accounts: { id: string; cli: string; label: string; disabled: boolean }[];
  acp: { id: string; name: string; available: boolean; command: string | null }[];
  tools: {
    id: string;
    name: string;
    cmd: string;
    installed: boolean;
    path: string | null;
    system_prompt_flag: string | null;
    model_flag: string | null;
    permission_flags: Record<string, string[]>;
  }[];
}

export interface LoopPlan {
  loop_id: string;
  name: string;
  runner: string;
  prompt: string;
  interval_secs: number | null;
  interval: string | null;
  check_command: string | null;
  stop_condition: string | null;
  max_rounds: number;
  max_minutes: number;
  budget: string | null;
  components: string[];
  workspace: string | null;
}

export interface SandboxOpts {
  provider: "docker" | "e2b";
  bind_original?: boolean;
  key_id?: string | null;
  e2b_key_id?: string | null;
  network?: boolean;
}

export interface SandboxStatus {
  docker: { available: boolean; version?: string; error?: string; path?: string };
  tools: { tool: string; package: string }[];
  keys: { id: string; name: string; kind: string; has_key: boolean; e2b: boolean }[];
}

export interface SandboxChange {
  path: string;
  status: "added" | "modified" | "deleted";
  diff?: string;
  binary: boolean;
  conflict: boolean;
}

export interface SandboxRecord {
  id: string;
  provider: string;
  tool: string;
  project: string;
  work: string;
  bind_original: boolean;
  image: string | null;
  env_names: string[];
  argv_preview: string[];
  applied: string[];
}

export interface PlaybookStep {
  name: string;
  runner?: RunnerSpec | null;
  agent?: string | null;
  command?: string | null;
  prompt: string;
}

export interface Playbook {
  name: string;
  description: string;
  source?: string | null;
  runner: RunnerSpec;
  workspace?: string | null;
  steps: PlaybookStep[];
  setup: string[];
}

export interface PlaybookRun {
  id: string;
  name: string;
  description: string;
  source: string | null;
  workspace: string | null;
  steps: {
    name: string;
    agent_id: string;
    prompt: string;
    runner_label: string | null;
    job_id: string | null;
    state: string;
    output: string | null;
  }[];
  state: "running" | "done" | "failed" | "cancelled";
  current: number;
  created_ms: number;
  finished_ms: number | null;
  error: string | null;
}

export interface Detected {
  id: string;
  evidence: string;
}

export interface StackReport {
  root: string;
  languages: Detected[];
  frameworks: Detected[];
  package_managers: string[];
  build_command: string | null;
  test_command: string | null;
  lint_command: string | null;
  dev_command: string | null;
  has_git: boolean;
  top_dirs: string[];
  agent_files: string[];
  name: string | null;
  description: string | null;
}

export interface WizardPiece {
  id: string;
  kind: "rule" | "command" | "agent" | "setting" | "hook" | "mcp";
  name: string;
  description: string;
  template: string;
  selected: boolean;
  available: boolean;
  reason?: string;
  notes?: string[];
}

export interface WizardSession {
  id: string;
  project: string;
  stack: StackReport;
  templates: string[];
  pieces: WizardPiece[];
  agents_md: string;
  warnings: string[];
  targets: { id: string; name: string; installed: boolean; beta: boolean; in_project: boolean; tier: number; suggested: boolean }[];
  all_templates: string[];
}

export const runRunners = () => invoke<RunnerCatalog>("run_runners");

export const runLoopPrepare = (a: {
  itemId: string;
  runnerSpec: RunnerSpec;
  workspace: string | null;
  targets: string[];
  scope?: string | null;
}) => invoke<{ loop: LoopPlan; plan: InstallPlan | null }>("run_loop_prepare", a);

export interface LoopStart {
  runner: RunnerSpec;
  name: string;
  prompt: string;
  workspace: string | null;
  schedule: string | null;
  checkCommand: string | null;
  maxRounds: number | null;
  maxMinutes: number | null;
  maxCostUsd: number | null;
  source: string | null;
  agent?: string | null;
  planId?: string | null;
  sandbox?: SandboxOpts | null;
}

export const runLoopStart = (start: LoopStart) => invoke<{ id: string; name: string }>("run_loop_start", { start });

export const runAgentNow = (a: {
  agent: string | null;
  runnerSpec: RunnerSpec;
  prompt: string;
  workspace: string | null;
  sandbox: SandboxOpts | null;
}) => invoke<{ id: string }>("run_agent_now", a);

export const runPlaybookPrepare = (a: {
  itemId?: string | null;
  playbook?: Playbook | null;
  runnerSpec: RunnerSpec;
  workspace: string | null;
}) => invoke<Playbook>("run_playbook_prepare", a);

export const runPlaybookStart = (a: { playbook: Playbook; task: string | null; targets: string[] | null }) =>
  invoke<PlaybookRun>("run_playbook_start", a);
export const runPlaybooksList = () => invoke<PlaybookRun[]>("run_playbooks_list");
export const runPlaybookCancel = (id: string) => invoke<PlaybookRun>("run_playbook_cancel", { id });
export const runPlaybookDelete = (id: string) => invoke<{ ok: boolean }>("run_playbook_delete", { id });

export const runWizardDetect = (projectDir: string, extraTemplates: string[] = []) =>
  invoke<WizardSession>("run_wizard_detect", { projectDir, extraTemplates });
export const runWizardPlan = (a: {
  sessionId: string;
  selected: string[];
  targets: string[];
  agentsMd: string | null;
  policy?: string | null;
}) => invoke<InstallPlan>("run_wizard_plan", a);
export const runWizardReview = (projectDir: string, runnerSpec: RunnerSpec) =>
  invoke<{ id: string }>("run_wizard_review", { projectDir, runnerSpec });

export const runSandboxStatus = () => invoke<SandboxStatus>("run_sandbox_status");
export const runSandboxDockerfile = (tool: string) =>
  invoke<{ dockerfile: string; tag: string; dir: string }>("run_sandbox_dockerfile", { tool });
export const runSandboxDiff = (jobId: string) =>
  invoke<{ record: SandboxRecord; changes: SandboxChange[] }>("run_sandbox_diff", { jobId });
export const runSandboxApply = (jobId: string, paths: string[] | null) =>
  invoke<{ applied: string[]; skipped: string[] }>("run_sandbox_apply", { jobId, paths });
export const runSandboxDiscard = (jobId: string) => invoke<{ ok: boolean }>("run_sandbox_discard", { jobId });

/** Encodes a runner as one `<select>` value and back. */
export function runnerKey(r: RunnerSpec): string {
  return [r.kind, r.id, r.account ?? ""].join("|");
}

export function parseRunnerKey(k: string): RunnerSpec {
  const [kind, id, account] = k.split("|");
  return { kind: kind as RunnerKind, id, account: account || null };
}

/** Label of a job's agent id (`tool:claude` → `claude (CLI)`). */
export function agentLabel(id: string, names: Record<string, string> = {}): string {
  if (id.startsWith("tool:")) return `${id.slice(5)} (CLI)`;
  return names[id] ?? id;
}

/** `10m`, `2h`, `daily`, or a 5-field cron line. */
export function scheduleValid(s: string): boolean {
  const t = s.trim();
  if (!t || t === "on-demand") return true;
  if (/^@(hourly|daily|midnight|weekly|monthly)$/.test(t)) return true;
  if (t.split(/\s+/).length === 5) return true;
  return /^(\d+\s*(s|sec|m|min|h|hr|hour|hours|d|day|days|w|week|weeks)?|hourly|daily|nightly|weekly)$/i.test(t);
}
