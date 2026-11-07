import { invoke } from "@tauri-apps/api/core";

type ToolStatus = "active" | "maintenance" | "discontinued";
type ToolKind = "cli" | "app" | "cli+app";
type OwnerMethod =
  | "native" | "bun" | "pnpm" | "npm" | "brew" | "pipx" | "uv" | "pip" | "cargo" | "omniget" | "app" | "unknown";
type RunState = "running" | "succeeded" | "failed" | "unchanged";

type CliTool = {
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

type CommandSpec = {
  program: string;
  args: string[];
  env: Record<string, string>;
  display: string;
  lock_key: string;
  via_shell: boolean;
};

type Owner = {
  method: OwnerMethod;
  proven: boolean;
  prefix: string | null;
  evidence: string;
  update: CommandSpec | null;
  manual: string | null;
};

type Detection = {
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

type RunResult = {
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
type ToolList = { tools: ToolRow[]; platform: string; detected: boolean };

export const listTools = () => invoke<ToolList>("clitools_list");
export const detectTools = (force = false) => invoke<Detection[]>("clitools_detect", { force });
