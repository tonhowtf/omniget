<script lang="ts">
  import { formatBinding } from "$lib/platform";
  import { onMount } from "svelte";
  import { notebooksStore } from "$lib/study-notes/notebooks-store.svelte";

  let open = $state(false);
  let rootRef = $state<HTMLDivElement | null>(null);

  const active = $derived(notebooksStore.active);
  const visible = $derived(notebooksStore.visible);

  onMount(() => {
    if (!notebooksStore.hydrated) {
      void notebooksStore.hydrate();
    }
    function onWinClick(e: MouseEvent) {
      if (!open) return;
      const target = e.target as Node | null;
      if (rootRef && target && rootRef.contains(target)) return;
      open = false;
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && open) {
        open = false;
      }
    }
    window.addEventListener("mousedown", onWinClick);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onWinClick);
      window.removeEventListener("keydown", onKey);
    };
  });

  function pick(id: number) {
    void notebooksStore.setActive(id);
    open = false;
  }
</script>

<div class="badge-wrap" bind:this={rootRef}>
  <button
    type="button"
    class="badge"
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open = !open)}
    title="Notebook ativo (Ctrl+Alt+1..9 alterna)"
  >
    {#if active?.color}
      <span class="dot" style:background={active.color}></span>
    {:else}
      <span class="dot dim"></span>
    {/if}
    <span class="name">{active?.name ?? "Pessoal"}</span>
    <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <polyline points="6 9 12 15 18 9" />
    </svg>
  </button>

  {#if open}
    <div class="menu" role="menu">
      {#each visible as nb, i (nb.id)}
        <button
          type="button"
          role="menuitem"
          class="menu-item"
          class:active={nb.id === active?.id}
          onclick={() => pick(nb.id)}
        >
          {#if nb.color}
            <span class="dot" style:background={nb.color}></span>
          {:else}
            <span class="dot dim"></span>
          {/if}
          <span class="menu-name">{nb.name}</span>
          <span class="menu-count">{nb.page_count}</span>
          <span class="menu-shortcut">{formatBinding(`CmdOrCtrl+Alt+${i + 1}`)}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .badge-wrap {
    position: relative;
    display: inline-flex;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    background: color-mix(in oklab, var(--accent) 8%, transparent);
    border: none;
    border-radius: 999px;
    color: var(--text);
    font: inherit;
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: background 120ms, border-color 120ms;
  }
  .badge:hover {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    border-color: var(--accent);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--accent);
    flex-shrink: 0;
  }
  .dot.dim {
    background: color-mix(in oklab, var(--text) 25%, transparent);
  }
  .name {
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu {
    position: absolute;
    bottom: calc(100% + 6px);
    left: 0;
    z-index: 120;
    min-width: 220px;
    background: var(--bg);
    border: none;
    border-radius: 10px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.32);
  }
  .menu-item {
    display: grid;
    grid-template-columns: auto 1fr auto auto;
    gap: 8px;
    align-items: center;
    padding: 6px 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    text-align: left;
    transition: background 120ms;
  }
  .menu-item:hover {
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .menu-item.active {
    background: color-mix(in oklab, var(--accent) 15%, transparent);
  }
  .menu-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu-count {
    color: var(--tertiary);
    font-size: 10px;
  }
  .menu-shortcut {
    color: var(--tertiary);
    font-family: ui-monospace, monospace;
    font-size: 10px;
  }
</style>
