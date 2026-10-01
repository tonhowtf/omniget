<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { roles } from "$lib/canvas/roles/store.svelte";
  import type { RoleInfo } from "$lib/canvas/roles/client";
  import RoleBadge from "./RoleBadge.svelte";

  let {
    workspaceId,
    value = null,
    onpick,
    onmanage,
    onclose,
  }: {
    workspaceId: string;
    value?: string | null;
    onpick: (roleId: string | null, role?: RoleInfo) => void;
    onmanage?: () => void;
    onclose?: () => void;
  } = $props();

  let query = $state("");
  let open = $state<string | null>(null);
  let active = $state(0);
  let input = $state<HTMLInputElement | null>(null);

  onMount(() => {
    void roles.load(workspaceId);
    queueMicrotask(() => input?.focus());
  });

  const items = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = roles.list;
    if (!q) return list;
    return list.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        (r.summary ?? "").toLowerCase().includes(q) ||
        r.prompt.toLowerCase().includes(q),
    );
  });

  const groups = $derived([
    { key: "workspace", label: $t("canvas.roles.scope.workspace", { default: "This workspace" }), items: items.filter((r) => r.scope === "workspace") },
    { key: "global", label: $t("canvas.roles.scope.global", { default: "Library" }), items: items.filter((r) => r.scope === "global") },
  ]);

  function pick(r: RoleInfo | null) {
    onpick(r ? r.id : null, r ?? undefined);
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose?.();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      active = Math.min(items.length, active + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(0, active - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      pick(active === 0 ? null : items[active - 1] ?? null);
    }
  }

  function flatIndex(r: RoleInfo): number {
    return items.indexOf(r) + 1;
  }
</script>

<div class="picker" data-canvas-chrome role="dialog" aria-label={$t("canvas.roles.picker.title", { default: "Choose a role" })} tabindex="-1" onkeydown={onkey}>
  <input
    bind:this={input}
    bind:value={query}
    class="search"
    type="search"
    placeholder={$t("canvas.roles.picker.search", { default: "Search roles" })}
    oninput={() => (active = 0)}
  />
  <div class="list" role="listbox">
    <button type="button" class="row none" class:active={active === 0} class:current={!value} role="option" aria-selected={!value} onclick={() => pick(null)}>
      <span class="name">{$t("canvas.roles.none", { default: "No role" })}</span>
    </button>
    {#each groups as g (g.key)}
      {#if g.items.length}
        <div class="group">{g.label}</div>
        {#each g.items as r (r.id)}
          <div class="item" class:active={active === flatIndex(r)} class:current={value === r.id}>
            <button type="button" class="row" role="option" aria-selected={value === r.id} onclick={() => pick(r)}>
              <RoleBadge icon={r.icon} color={r.color} size={22} />
              <span class="text">
                <span class="name">{r.name}</span>
                {#if r.summary}<span class="summary">{r.summary}</span>{/if}
              </span>
              {#if r.terminalsInUse.length}
                <span class="usage" title={$t("canvas.roles.in_use", { default: "Terminals using it" })}>{r.terminalsInUse.length}</span>
              {/if}
            </button>
            <button
              type="button"
              class="chev"
              class:open={open === r.id}
              aria-expanded={open === r.id}
              aria-label={$t("canvas.roles.picker.instructions", { default: "Show instructions" })}
              onclick={() => (open = open === r.id ? null : r.id)}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
            </button>
            {#if open === r.id}
              <pre class="prompt">{r.prompt}</pre>
            {/if}
          </div>
        {/each}
      {/if}
    {/each}
    {#if !items.length && query}
      <div class="empty">{$t("canvas.roles.picker.empty", { default: "No role matches" })}</div>
    {/if}
  </div>
  {#if onmanage}
    <button type="button" class="manage" onclick={() => onmanage?.()}>
      {$t("canvas.roles.manage", { default: "Manage roles…" })}
    </button>
  {/if}
</div>

<style>
  .picker {
    width: 340px;
    max-height: 460px;
    display: flex;
    flex-direction: column;
    border-radius: 12px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.32);
    overflow: hidden;
    outline: none;
  }
  .search {
    margin: 8px;
    padding: 7px 10px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--input-bg, transparent);
    color: var(--text);
    font-size: 13px;
  }
  .list {
    overflow-y: auto;
    padding: 0 6px 6px;
  }
  .group {
    padding: 8px 8px 4px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .item {
    display: grid;
    grid-template-columns: 1fr auto;
    border-radius: 8px;
  }
  .item.active,
  .row.none.active {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .item.current .name,
  .row.none.current .name {
    color: var(--accent);
  }
  .row {
    all: unset;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 8px;
    border-radius: 8px;
    cursor: pointer;
    min-width: 0;
  }
  .row:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .row.none {
    width: calc(100% - 16px);
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
  .chev {
    all: unset;
    display: grid;
    place-items: center;
    width: 28px;
    cursor: pointer;
    color: var(--text-muted);
    border-radius: 6px;
  }
  .chev svg {
    transition: transform 0.15s ease;
  }
  .chev.open svg {
    transform: rotate(90deg);
  }
  .prompt {
    grid-column: 1 / -1;
    margin: 0 8px 8px 40px;
    padding: 8px 10px;
    max-height: 180px;
    overflow: auto;
    white-space: pre-wrap;
    font: 12px/1.45 var(--font-mono, ui-monospace, monospace);
    color: var(--text-muted);
    background: var(--code-bg, rgba(127, 127, 127, 0.08));
    border-radius: 8px;
  }
  .empty {
    padding: 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
  }
  .manage {
    all: unset;
    padding: 9px 14px;
    border-top: 1px solid var(--border);
    font-size: 13px;
    color: var(--accent);
    cursor: pointer;
  }
  .manage:hover {
    background: var(--hover, rgba(127, 127, 127, 0.1));
  }
</style>
