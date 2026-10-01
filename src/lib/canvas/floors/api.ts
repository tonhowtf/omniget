// Floors and environments: typed wrappers over the `canvas_floor_*` and
// `canvas_env_*` commands, and the events the Rust host emits.

import { invoke } from '@tauri-apps/api/core';

export const EVENT_FLOOR_PROGRESS = 'canvas://floor-progress';
export const EVENT_FLOORS = 'canvas://floors';

type FloorMode = 'clone' | 'worktree' | 'plain';
export type HookKind = 'setup' | 'run' | 'teardown';

export interface FloorView {
  id: string;
  name: string;
  color?: string;
  branch?: string;
  isGitIsolated?: boolean;
  path?: string;
  createdAt: string;
  mode: FloorMode;
  port?: number;
  baseBranch?: string;
  isGround: boolean;
  nodeCount: number;
  terminalCount: number;
  ahead?: number;
  dirty?: number;
  missing: boolean;
}

interface CreateFloorInput {
  name: string;
  branch?: string;
  existingBranch?: boolean;
  noGit?: boolean;
  copyGround?: boolean;
  preferWorktree?: boolean;
  color?: string;
  runSetup?: boolean;
}

interface HookCommand {
  id: string;
  command: string;
  name?: string;
  enabled: boolean;
}

export interface FloorHooks {
  setup: HookCommand[];
  run: HookCommand[];
  teardown: HookCommand[];
  autoRunSetup: boolean;
}

export interface HookRun {
  id: string;
  kind: HookKind;
  command: string;
  code: number | null;
  ok: boolean;
  output: string;
  durationMs: number;
}

interface CreateResult {
  floor: FloorView;
  setup: HookRun[];
  warnings?: string[];
}

export interface FloorProgress {
  workspaceId: string;
  floorId: string;
  name: string;
  phase: 'clone' | 'branch' | 'layout' | 'setup' | 'done' | 'failed';
  message?: string;
}

interface FileStat {
  path: string;
  added: number | null;
  deleted: number | null;
}

export interface LandPreview {
  floorBranch: string;
  target: string;
  createsTarget: boolean;
  upToDate: boolean;
  fastForward: boolean;
  commits: string[];
  files: FileStat[];
  conflicts: string[];
  floorDirty: string[];
  floorUntracked?: string[];
  groundDirty: string[];
  targetCheckedOut: boolean;
  branches: string[];
  blocker?: string;
}

interface LandOptions {
  into?: string;
  deleteBranch?: boolean;
  keepFloor?: boolean;
  message?: string;
}

export interface LandOutcome {
  status: 'landed' | 'up_to_date';
  target: string;
  commit?: string;
  fastForward: boolean;
  commits: number;
  floorRemoved: boolean;
  teardown?: HookRun[];
}

interface DiscardOptions {
  keepBranch?: boolean;
  force?: boolean;
}

interface DiscardOutcome {
  removed: boolean;
  branchKept?: string;
  branchDeleted?: string;
  teardown?: HookRun[];
}

type EnvKind = 'local' | 'tmux' | 'ssh' | 'docker' | 'sandbox' | 'custom' | 'wsl';

export interface EnvRecord {
  kind: EnvKind;
  host?: string;
  dir?: string;
  sshOptions?: string;
  user?: string;
  /** Custom runtime name (kind `custom`). */
  runtime?: string;
  /** Keep the shell in a tmux session on that side. */
  persistent?: boolean;
}

interface EnvCheck {
  name: string;
  ok: boolean;
  detail?: string;
}

interface EnvTest {
  ok: boolean;
  output: string;
  checks?: EnvCheck[];
}

export interface CustomRuntime {
  exec: string;
  args: string;
  interactive: string;
  shell?: string;
  bridgeHost?: string;
}

export interface EnvProviders {
  tmux: boolean;
  ssh: boolean;
  docker: boolean;
  sandbox: boolean;
  custom: boolean;
  wsl: boolean;
  runtimes: Record<string, CustomRuntime>;
}

export const getEnvProviders = () => invoke<EnvProviders>('canvas_env_providers_get');
export const setEnvProviders = (patch: Partial<EnvProviders> | Record<string, unknown>) =>
  invoke<EnvProviders>('canvas_env_providers_set', { patch });

/** A persistent session (tmux `omniget-*`) here or in a remote environment; `error` when that environment did not answer. */
export interface EnvSession {
  session?: string;
  createdS?: number;
  attached?: boolean;
  command?: string;
  /** Where it runs: `local`, `docker:web`, `ssh:me@box`, `custom:podman`. */
  env: string;
  kind: EnvKind;
  host?: string;
  error?: string;
}

export const envSessions = () => invoke<EnvSession[]>('canvas_env_sessions');
export const envSessionKill = (session: string, env?: string) =>
  invoke<void>('canvas_env_session_kill', { session, env: env ?? null });

export interface SshHost {
  host: string;
  hostName?: string;
  user?: string;
  port?: string;
  proxyJump?: string;
}

/** Hosts of `~/.ssh/config` for the SSH picker. */
export const sshHosts = () => invoke<SshHost[]>('canvas_env_ssh_hosts');

interface EnvFiles {
  remote: boolean;
  env?: string;
  root?: string;
  hits?: { path: string; isDir: boolean }[];
}

/** Files where the terminal runs (`remote: false` = a local terminal). */
export const envFiles = (nodeId: string, dir: string | null, query: string) =>
  invoke<EnvFiles>('canvas_env_files', { nodeId, dir, query });

/** Local files for a terminal: copied to `/tmp/omniget-drops/` on a remote one; the paths to type. */
export const envDrop = (nodeId: string, paths: string[]) => invoke<string[]>('canvas_env_drop', { nodeId, paths });

export const listFloors = (workspaceId: string) => invoke<FloorView[]>('canvas_floor_list', { workspaceId });

export const createFloor = (workspaceId: string, input: CreateFloorInput) =>
  invoke<CreateResult>('canvas_floor_create', { workspaceId, input });

export const landPreview = (workspaceId: string, floorId: string, into?: string) =>
  invoke<LandPreview>('canvas_floor_land_preview', { workspaceId, floorId, into: into || null });

export const landFloor = (workspaceId: string, floorId: string, options: LandOptions) =>
  invoke<LandOutcome>('canvas_floor_land', { workspaceId, floorId, options });

export const discardFloor = (workspaceId: string, floorId: string, options: DiscardOptions) =>
  invoke<DiscardOutcome>('canvas_floor_discard', { workspaceId, floorId, options });

export const runHook = (workspaceId: string, floorId: string, kind: HookKind, hookId?: string) =>
  invoke<HookRun[]>('canvas_floor_run_hook', { workspaceId, floorId, kind, hookId: hookId ?? null });

export const getHooks = (workspaceId: string) => invoke<FloorHooks>('canvas_floor_hooks_get', { workspaceId });

export const setHooks = (workspaceId: string, hooks: FloorHooks) =>
  invoke<FloorHooks>('canvas_floor_hooks_set', { workspaceId, hooks });

export const getEnv = (workspaceId: string) => invoke<EnvRecord>('canvas_env_get', { workspaceId });
export const setEnv = (workspaceId: string, env: EnvRecord) => invoke<EnvRecord>('canvas_env_set', { workspaceId, env });
export const testEnv = (env: EnvRecord) => invoke<EnvTest>('canvas_env_test', { env });

export const setMaestro = (workspaceId: string, floorId: string, nodeId: string, on: boolean) =>
  invoke<void>('canvas_maestro_set', { workspaceId, floorId, nodeId, on });

export function newHook(command = ''): HookCommand {
  return { id: crypto.randomUUID(), command, enabled: true };
}

/** Rust errors start with a code (`CANVAS_LAND_BLOCKED: ...`); show the rest. */
export function errorText(e: unknown): string {
  const s = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
  return s.replace(/^[A-Z_]+:\s*/, '');
}
