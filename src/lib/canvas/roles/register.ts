import { onRoleEvent, type RoleEvent } from './client';
import { roles } from './store.svelte';

interface RolesHost {
  workspaceId(): string;
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
