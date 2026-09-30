// Typed wrappers over commands::canvas::roles and the `canvas://role` event.
// Role JSON is camelCase (serde rename_all), optionals omitted.

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { AgentKind } from '../model/types';

export type RoleScope = 'workspace' | 'global';

export interface Role {
  id: string;
  name: string;
  prompt: string;
  summary?: string;
  icon?: string;
  color?: string;
  agent?: AgentKind;
  /** Absent = global library. */
  workspaceId?: string;
  builtin?: boolean;
  createdAt?: string;
  updatedAt?: string;
}

export interface RoleInfo extends Role {
  scope: RoleScope;
  terminalsInUse: string[];
}

export interface RoleInput {
  id?: string;
  name: string;
  prompt: string;
  summary?: string;
  icon?: string;
  color?: string;
  agent?: AgentKind;
}

export interface RoleLaunch {
  roleId: string;
  roleName: string;
  cwd?: string | null;
  projectDir?: string | null;
  roleDir: string;
  args: string[];
  env: Record<string, string>;
  initialPrompt?: string;
  systemPrompt?: string;
  mode: string;
}

export interface RoleEvent {
  workspaceId: string;
  floorId: string;
  nodeId: string;
  terminalId: string;
  roleId?: string;
  restart: boolean;
}

export const ROLE_EVENT = 'canvas://role';

export const rolesList = (workspaceId: string) => invoke<RoleInfo[]>('canvas_roles_list', { workspaceId });
export const rolesGet = (workspaceId: string, roleId: string) =>
  invoke<Role | null>('canvas_roles_get', { workspaceId, roleId });
export const rolesSave = (workspaceId: string, scope: RoleScope, input: RoleInput) =>
  invoke<Role>('canvas_roles_save', { workspaceId, scope, input });
export const rolesDelete = (workspaceId: string, roleId: string, scope: RoleScope, force = false) =>
  invoke<void>('canvas_roles_delete', { workspaceId, roleId, scope, force });
export const rolesAssign = (workspaceId: string, nodeId: string, roleId: string | null) =>
  invoke<Role | null>('canvas_roles_assign', { workspaceId, nodeId, roleId });
export const rolesLaunch = (workspaceId: string, roleId: string, agent: AgentKind, cwd?: string | null) =>
  invoke<RoleLaunch>('canvas_roles_launch', { workspaceId, roleId, agent, cwd: cwd ?? null });
export const rolesDiscover = (workspaceId: string, dir?: string) =>
  invoke<Role[]>('canvas_roles_discover', { workspaceId, dir: dir ?? null });
export const rolesImport = (workspaceId: string, names: string[], dir?: string) =>
  invoke<Role[]>('canvas_roles_import', { workspaceId, dir: dir ?? null, names });

export function onRoleEvent(cb: (e: RoleEvent) => void): Promise<UnlistenFn> {
  return listen<RoleEvent>(ROLE_EVENT, (e) => cb(e.payload));
}

export function errorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === 'string') return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
