// Roles of the open workspace (workspace roles + global library), shared by
// the library, the editor and the picker. Refreshed on demand and after
// every write made through it.

import {
  errorText,
  rolesAssign,
  rolesDelete,
  rolesList,
  rolesSave,
  type RoleInfo,
  type RoleInput,
  type RoleScope,
} from './client';

class RolesStore {
  workspaceId = $state('');
  list = $state.raw<RoleInfo[]>([]);
  loading = $state(false);
  error = $state('');
  private inflight: Promise<void> | null = null;

  byId(id: string | null | undefined): RoleInfo | undefined {
    if (!id) return undefined;
    return this.list.find((r) => r.id === id);
  }

  async load(workspaceId: string, force = false): Promise<void> {
    if (!workspaceId) return;
    if (!force && workspaceId === this.workspaceId && this.list.length) return;
    if (this.inflight && workspaceId === this.workspaceId) return this.inflight;
    this.workspaceId = workspaceId;
    this.loading = true;
    this.inflight = rolesList(workspaceId)
      .then((l) => {
        if (this.workspaceId === workspaceId) {
          this.list = l;
          this.error = '';
        }
      })
      .catch((e) => {
        this.error = errorText(e);
      })
      .finally(() => {
        this.loading = false;
        this.inflight = null;
      });
    return this.inflight;
  }

  refresh(): Promise<void> {
    return this.load(this.workspaceId, true);
  }

  async save(scope: RoleScope, input: RoleInput) {
    const r = await rolesSave(this.workspaceId, scope, input);
    await this.refresh();
    return r;
  }

  async remove(id: string, scope: RoleScope, force = false) {
    await rolesDelete(this.workspaceId, id, scope, force);
    await this.refresh();
  }

  async assign(nodeId: string, roleId: string | null) {
    const r = await rolesAssign(this.workspaceId, nodeId, roleId);
    await this.refresh();
    return r;
  }
}

export const roles = new RolesStore();
