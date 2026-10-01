<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { roles } from "$lib/canvas/roles/store.svelte";
  import { errorText, rolesDiscover, rolesImport, type Role, type RoleInfo } from "$lib/canvas/roles/client";
  import RoleBadge from "./RoleBadge.svelte";
  import RoleEditor from "./RoleEditor.svelte";

  let { workspaceId, onclose }: { workspaceId: string; onclose?: () => void } = $props();

  let selected = $state<string | null>(null);
  let creating = $state(false);
  let query = $state("");
  let error = $state("");
  let confirmDelete = $state<RoleInfo | null>(null);
  let discovered = $state<Role[] | null>(null);
  let editorKey = $state(0);

  onMount(() => {
    void roles.load(workspaceId, true);
  });

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? roles.list.filter((r) => r.name.toLowerCase().includes(q) || (r.summary ?? "").toLowerCase().includes(q)) : roles.list;
  });
  const current = $derived(creating ? null : roles.byId(selected));

  function select(id: string) {
    creating = false;
    selected = id;
    editorKey++;
  }

  function create() {
    creating = true;
    selected = null;
    editorKey++;
  }

  async function copyTo(r: RoleInfo) {
    error = "";
    try {
      const scope = r.scope === "global" ? "workspace" : "global";
      const saved = await roles.save(scope, {
        name: `${r.name} (${scope === "global" ? "library" : "copy"})`,
        prompt: r.prompt,
        summary: r.summary,
        icon: r.icon,
        color: r.color,
        agent: r.agent,
      });
      select(saved.id);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function remove(r: RoleInfo, force: boolean) {
    error = "";
    try {
      await roles.remove(r.id, r.scope, force);
      confirmDelete = null;
      if (selected === r.id) selected = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  function askDelete(r: RoleInfo) {
    if (r.terminalsInUse.length) confirmDelete = r;
    else void remove(r, false);
  }

  async function discover() {
    error = "";
    try {
      discovered = await rolesDiscover(workspaceId);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function importRoles(list: Role[]) {
    error = "";
    if (!list.length) return;
    try {
      await rolesImport(workspaceId, list.map((r) => r.name));
      await roles.refresh();
      const done = new Set(list.map((r) => r.id));
      discovered = (discovered ?? []).filter((d) => !done.has(d.id));
    } catch (e) {
      error = errorText(e);
    }
  }

  function isUpdate(r: Role): boolean {
    return roles.list.some((x) => x.scope === "workspace" && x.name.toLowerCase() === r.name.toLowerCase());
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape" && !creating && !selected) {
      e.preventDefault();
      onclose?.();
    }
  }
</script>

<div class="library" data-canvas-chrome role="dialog" aria-label={$t("canvas.roles.library.title", { default: "Roles" })} tabindex="-1" onkeydown={onkey}>
  <aside class="side">
    <div class="side-head">
      <h2>{$t("canvas.roles.library.title", { default: "Roles" })}</h2>
      <button type="button" class="new" onclick={create}>{$t("canvas.roles.library.new", { default: "New role" })}</button>
    </div>
    <input class="search" type="search" bind:value={query} placeholder={$t("canvas.roles.picker.search", { default: "Search roles" })} />
    <div class="list">
      {#each ["workspace", "global"] as scope (scope)}
        {@const group = filtered.filter((r) => r.scope === scope)}
        <div class="group">
          {scope === "workspace"
            ? $t("canvas.roles.scope.workspace", { default: "This workspace" })
            : $t("canvas.roles.scope.global", { default: "Library" })}
        </div>
        {#if !group.length}
          <div class="muted">{$t("canvas.roles.library.empty", { default: "No roles yet" })}</div>
        {/if}
        {#each group as r (r.id)}
          <button type="button" class="role" class:on={selected === r.id && !creating} onclick={() => select(r.id)}>
            <RoleBadge icon={r.icon} color={r.color} size={24} />
            <span class="text">
              <span class="name">{r.name}</span>
              {#if r.summary}<span class="summary">{r.summary}</span>{/if}
            </span>
            {#if r.terminalsInUse.length}<span class="usage">{r.terminalsInUse.length}</span>{/if}
          </button>
        {/each}
      {/each}
    </div>
    <button type="button" class="discover" onclick={discover}>{$t("canvas.roles.library.discover", { default: "Discover roles in the project" })}</button>
  </aside>

  <section class="main">
    {#if error}<p class="error" role="alert">{error}</p>{/if}

    {#if confirmDelete}
      <div class="confirm">
        <p>
          {$t("canvas.roles.library.delete_in_use", {
            default: "{{name}} is used by {{count}} terminal(s). Deleting removes it from them and restarts them.",
            name: confirmDelete.name,
            count: confirmDelete.terminalsInUse.length,
          })}
        </p>
        <div class="actions">
          <button type="button" class="ghost" onclick={() => (confirmDelete = null)}>{$t("canvas.roles.cancel", { default: "Cancel" })}</button>
          <button type="button" class="danger" onclick={() => confirmDelete && remove(confirmDelete, true)}>{$t("canvas.roles.delete", { default: "Delete" })}</button>
        </div>
      </div>
    {/if}

    {#if discovered}
      <div class="discovered">
        <div class="disc-head">
          <strong>{$t("canvas.roles.library.found", { default: "Found in the project" })}</strong>
          <span class="disc-actions">
            {#if discovered.length > 1}
              <button type="button" class="ghost" onclick={() => importRoles(discovered ?? [])}>{$t("canvas.roles.library.import_all", { default: "Import all" })}</button>
            {/if}
            <button type="button" class="ghost" onclick={() => (discovered = null)}>{$t("canvas.roles.close", { default: "Close" })}</button>
          </span>
        </div>
        {#if !discovered.length}
          <p class="muted">{$t("canvas.roles.library.found_none", { default: "No role.json files in .omniget/roles or .maestri/roles." })}</p>
        {/if}
        {#each discovered as d (d.id)}
          <div class="disc-row">
            <RoleBadge icon={d.icon} color={d.color} size={22} />
            <span class="name">{d.name}</span>
            {#if d.prompt}<span class="disc-prompt" title={d.prompt}>{d.prompt}</span>{/if}
            <button type="button" class="ghost" onclick={() => importRoles([d])}>{isUpdate(d) ? $t("canvas.roles.library.import_update", { default: "Update" }) : $t("canvas.roles.library.import", { default: "Import" })}</button>
          </div>
        {/each}
      </div>
    {/if}

    {#if creating || current}
      {#key editorKey}
        <RoleEditor
          role={current}
          onsaved={(id) => {
            creating = false;
            selected = id;
          }}
          oncancel={() => {
            creating = false;
            selected = null;
          }}
        />
      {/key}
      {#if current}
        <div class="more">
          <button type="button" class="ghost" onclick={() => current && copyTo(current)}>
            {current.scope === "global"
              ? $t("canvas.roles.library.copy_ws", { default: "Copy to this workspace" })
              : $t("canvas.roles.library.copy_lib", { default: "Copy to the library" })}
          </button>
          <button type="button" class="danger-link" onclick={() => current && askDelete(current)}>{$t("canvas.roles.delete", { default: "Delete" })}</button>
        </div>
      {/if}
    {:else}
      <div class="placeholder">
        <p>{$t("canvas.roles.library.hint", { default: "A role is a set of instructions a terminal starts with. Pick one to edit, or create your own." })}</p>
      </div>
    {/if}
  </section>

  {#if onclose}
    <button type="button" class="x" aria-label={$t("canvas.roles.close", { default: "Close" })} onclick={() => onclose?.()}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
    </button>
  {/if}
</div>

<style>
  .library {
    position: relative;
    display: grid;
    grid-template-columns: 280px 1fr;
    width: min(880px, calc(100vw - 32px));
    height: min(620px, calc(100vh - 48px));
    border-radius: 14px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.35);
    overflow: hidden;
    outline: none;
  }
  .side {
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
    min-height: 0;
  }
  .side-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 14px 6px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  .new {
    all: unset;
    font-size: 12px;
    padding: 5px 10px;
    border-radius: 7px;
    background: var(--accent);
    color: var(--on-accent, #fff);
    cursor: pointer;
    font-weight: 600;
  }
  .search {
    margin: 6px 12px;
    padding: 6px 10px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    font-size: 13px;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0 6px 8px;
  }
  .group {
    padding: 10px 8px 4px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .muted {
    padding: 2px 8px 6px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .role {
    all: unset;
    display: flex;
    align-items: center;
    gap: 10px;
    width: calc(100% - 16px);
    padding: 7px 8px;
    border-radius: 8px;
    cursor: pointer;
  }
  .role:hover {
    background: var(--hover, rgba(127, 127, 127, 0.1));
  }
  .role.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .summary {
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .usage {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 99px;
    background: var(--hover, rgba(127, 127, 127, 0.16));
    color: var(--text-muted);
  }
  .discover {
    all: unset;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
    font-size: 12px;
    color: var(--accent);
    cursor: pointer;
  }
  .main {
    padding: 18px 20px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .placeholder {
    flex: 1;
    display: grid;
    place-items: center;
    color: var(--text-muted);
    font-size: 13px;
    text-align: center;
    padding: 0 40px;
  }
  .more {
    display: flex;
    justify-content: space-between;
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }
  .ghost,
  .danger,
  .danger-link {
    all: unset;
    padding: 6px 12px;
    border-radius: 8px;
    font-size: 13px;
    cursor: pointer;
  }
  .ghost {
    color: var(--text-muted);
  }
  .ghost:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .danger {
    background: var(--danger, #e5484d);
    color: #fff;
    font-weight: 600;
  }
  .danger-link {
    color: var(--danger, #e5484d);
  }
  .confirm,
  .discovered {
    padding: 12px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--hover, rgba(127, 127, 127, 0.06));
    font-size: 13px;
  }
  .confirm p {
    margin: 0 0 10px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .disc-head,
  .disc-row {
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: space-between;
  }
  .disc-row {
    justify-content: flex-start;
    padding: 4px 0;
  }
  .disc-row .name {
    flex: 0 0 auto;
    font-weight: 500;
  }
  .disc-prompt {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--text-secondary, #8b8b8b);
    font-size: 12px;
  }
  .disc-actions {
    display: flex;
    gap: 6px;
  }
  .error {
    margin: 0;
    color: var(--danger, #e5484d);
    font-size: 12px;
  }
  .x {
    all: unset;
    position: absolute;
    top: 10px;
    right: 10px;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: 7px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .x:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  @media (max-width: 640px) {
    .library {
      grid-template-columns: 1fr;
      grid-template-rows: 40% 1fr;
    }
    .side {
      border-right: none;
      border-bottom: 1px solid var(--border);
    }
  }
</style>
