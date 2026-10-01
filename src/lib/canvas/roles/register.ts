// Roles registration. Canvas.svelte (not owned here) calls
// `registerRoles({ workspaceId, restart })` on mount and the returned
// function on destroy. It loads the roles of the workspace and listens to
// `canvas://role`: when Rust assigned a new role it stopped the process;
// `restart(nodeId)` asks the terminal node to reopen (its open path applies
// the role through `withRole` from ./launch).

import { onRoleEvent, type RoleEvent } from './client';
import { roles } from './store.svelte';

interface RolesHost {
  workspaceId(): string;
  /** Reopen the terminal of a node (same terminal id, new launch). */
  restart(nodeId: string, ev: RoleEvent): void;
}

export function registerRoles(host: RolesHost): () => void {
  const ws = host.workspaceId();
  if (ws) void roles.load(ws);
  let dead = false;
  const un = onRoleEvent((ev) => {
    if (dead || ev.workspaceId !== host.workspaceId()) return;
    void roles.refresh();
    host.restart(ev.nodeId, ev);
  }).catch(() => () => {});
  return () => {
    dead = true;
    void un.then((f) => f());
  };
}
