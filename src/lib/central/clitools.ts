// Invokes tipados dos comandos `clitools_*` da Central (/llm/tools): tabela de
// CLIs, detecção com prova de dono, versão mais nova, planos com o comando
// visível, execução com log ao vivo, registro ACP, login e diagnóstico.
// Os erros chegam como "CODE: mensagem".

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const PROGRESS_EVENT = "clitools://progress";

// ---- tipos (espelham os structs Rust, snake_case) ----

export type ToolStatus = "active" | "maintenance" | "discontinued";
export type ToolKind = "cli" | "app" | "cli+app";
export type OwnerMethod =
  | "native" | "bun" | "pnpm" | "npm" | "brew" | "pipx" | "uv" | "pip" | "cargo" | "omniget" | "app" | "unknown";
export type VersionState = "unknown" | "current" | "behind_latest";
export type RunState = "running" | "succeeded" | "failed" | "unchanged";

export type CliTool = {
  id: string;
  name: string;
  kind: ToolKind;
  status: ToolStatus;
  status_note: string | null;
  docs: string;
  repo: string | null;
  binaries: string[];
  subcommand: string[];
  version_args: string[];
  apps: { macos: string[] };
  npm: string | null;
  npm_scripts: boolean;
  pip: string | null;
  cargo: string | null;
  installer: { unix: string | null; unix_shell: string; windows: string | null };
  brew: { name: string; cask: boolean } | null;
  winget: string | null;
  scoop: string | null;
  acp: { registry_id: string | null; native_args: string[] | null; adapter: string[] | null };
  login: string[];
  login_hint: string | null;
  auth_status: string[] | null;
  auth_status_exit_means_auth: boolean;
  auth_files: string[];
  config_env: string | null;
  self_update: { args: string[] | null; rerun_installer: boolean; markers: string[] } | null;
  auto_updates: boolean;
  config_paths: string[];
  log_paths: string[];
  deprecated: { path: string; note: string }[];
};

export type CommandSpec = {
  program: string;
  args: string[];
  env: Record<string, string>;
  display: string;
  lock_key: string;
  via_shell: boolean;
};

export type Owner = {
  method: OwnerMethod;
  proven: boolean;
  prefix: string | null;
  evidence: string;
  update: CommandSpec | null;
  manual: string | null;
};

export type Detection = {
  id: string;
  installed: boolean;
  binary: string | null;
  resolved_path: string | null;
  real_path: string | null;
  version: string | null;
  version_raw: string | null;
  app_path: string | null;
  app_version: string | null;
  owner: Owner;
  checked_at: number;
};

export type RunResult = {
  run_id: string;
  plan_id: string;
  tool_id: string;
  state: RunState;
  exit_code: number | null;
  output: string;
  truncated: boolean;
  version_before: string | null;
  version_after: string | null;
  duration_ms: number;
  warnings: string[];
};

export type ToolRow = { tool: CliTool; detection: Detection | null; last_run: RunResult | null };
export type ToolList = { tools: ToolRow[]; platform: string; detected: boolean };

export type Latest = {
  id: string;
  source: string;
  key: string;
  latest: string | null;
  installed: string | null;
  state: VersionState;
  checked_at: number;
  error: string | null;
};

export type Plan = {
  id: string;
  tool_id: string;
  action: "install" | "update";
  method: string;
  command: CommandSpec;
  runnable: boolean;
  reason: string | null;
  recommended: boolean;
  warnings: string[];
  owner: OwnerMethod | null;
  prefix: string | null;
};

export type Progress = {
  run_id: string;
  tool_id: string;
  kind: "start" | "line" | "end";
  stream: "stdout" | "stderr" | null;
  line: string | null;
  state: RunState;
  command: string | null;
};

export type Launch = {
  surface: "system" | "embedded";
  command: string;
  command_args: string[];
  env: Record<string, string>;
  unset: string[];
  display: string;
  script_path: string | null;
  script: string | null;
  program: string | null;
  args: string[];
  hint: string | null;
};

export type AcpLaunch = {
  agent_id: string;
  name: string;
  version: string;
  distribution: "npx" | "uvx" | "binary";
  command: string;
  args: string[];
  env: Record<string, string>;
  dir: string | null;
  sha256: string | null;
  sha256_verified: boolean;
  installed_at: number;
  warnings: string[];
};

export type AcpAgent = {
  id: string;
  name: string;
  version: string;
  description: string | null;
  repository: string | null;
  website: string | null;
  license: string | null;
  icon: string | null;
  methods: string[];
  binary_here: boolean;
  binary_verified: boolean;
  tool_id: string | null;
  installed: AcpLaunch | null;
};

export type AcpRegistry = {
  version: string | null;
  fetched_at: number;
  stale: boolean;
  error: string | null;
  platform: string;
  agents: AcpAgent[];
};

export type AuthState = "present" | "absent" | "unknown";
export type PathCheck = { path: string; resolved: string | null; exists: boolean | null; note: string | null };
export type DoctorReport = {
  id: string;
  name: string;
  detection: Detection;
  latest: Latest;
  auth: { state: AuthState; via: "command" | "file" | "none"; detail: string; output: string | null };
  config_env: [string, string | null] | null;
  config: PathCheck[];
  logs: PathCheck[];
  deprecated: PathCheck[];
  issues: string[];
};

/** Uma execução acompanhada na tela (plano ou instalação ACP). */
export type LogEntry = {
  key: string;
  title: string;
  command: string | null;
  lines: { stream: string | null; text: string }[];
  state: RunState;
  result: RunResult | null;
};

// ---- invokes ----

export const listTools = () => invoke<ToolList>("clitools_list");
export const detectTools = (force = false) => invoke<Detection[]>("clitools_detect", { force });
export const latestOf = (id: string, force = false) => invoke<Latest>("clitools_latest", { id, force });
export const planFor = (id: string, action: "install" | "update") =>
  invoke<Plan[]>("clitools_plan", { id, action });
export const runPlan = (planId: string) => invoke<RunResult>("clitools_run", { planId });
export const loginTool = (id: string, account?: string | null, surface: "system" | "embedded" = "system") =>
  invoke<Launch>("clitools_login", { id, account: account ?? null, surface });
export const acpRegistry = (force = false) => invoke<AcpRegistry>("clitools_acp_registry", { force });
export const acpInstall = (agentId: string, method?: string | null, allowUnverified = false) =>
  invoke<AcpLaunch>("clitools_acp_install", { agentId, method: method ?? null, allowUnverified });
export const doctorOf = (id: string) => invoke<DoctorReport>("clitools_doctor", { id });

export function onProgress(fn: (p: Progress) => void): Promise<UnlistenFn> {
  return listen<Progress>(PROGRESS_EVENT, (e) => fn(e.payload));
}

/** Código do erro "CODE: mensagem". */
export function errorCode(e: unknown): string {
  const s = String(e);
  const i = s.indexOf(":");
  return i > 0 ? s.slice(0, i).trim() : "";
}
