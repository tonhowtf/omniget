<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { addMenu, connectionMenu, nodeMenu, type MenuEntry } from "$lib/canvas/menu/model";

  const engine = useEngine();
  let el = $state<HTMLDivElement | null>(null);
  let menuH = $state(0);

  const menu = $derived(engine.spawnMenu);
  const entries = $derived.by((): MenuEntry[] => {
    const m = menu;
    if (!m) return [];
    if (m.nodeId) return nodeMenu(engine, m.nodeId);
    if (m.connectionId) return connectionMenu(engine, m.connectionId);
    return addMenu(engine, { wx: m.wx, wy: m.wy, connectFrom: m.connectFrom, free: m.free }).flatMap((x) =>
      x.id === "terminal" && x.children ? x.children : [x],
    );
  });
  const pos = $derived.by(() => {
    if (!menu) return { x: 0, y: 0 };
    const w = 220;
    const h = Math.min(engine.camera.vh - 16, menuH || 32 * entries.length + 48);
    return {
      x: Math.min(menu.sx, Math.max(0, engine.camera.vw - w - 8)),
      y: Math.min(menu.sy, Math.max(0, engine.camera.vh - h - 8)),
    };
  });

  function act(item: MenuEntry) {
    try {
      void Promise.resolve(item.run?.()).catch(() => {});
    } finally {
      engine.spawnMenu = null;
    }
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      engine.spawnMenu = null;
      return;
    }
    const buttons = el ? Array.from(el.querySelectorAll<HTMLButtonElement>("button[role='menuitem']")) : [];
    if (!buttons.length) return;
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Home" || e.key === "End") {
      e.preventDefault();
      e.stopPropagation();
      const n = buttons.length;
      const next =
        e.key === "Home" ? 0 : e.key === "End" ? n - 1 : e.key === "ArrowDown" ? (i + 1 + n) % n : (i - 1 + n) % n;
      buttons[next].focus();
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      e.stopPropagation();
      (buttons[i] ?? buttons[0]).click();
    }
  }

  $effect(() => {
    if (menu && el) el.querySelector<HTMLButtonElement>("button")?.focus();
  });
</script>

{#snippet item(entry: MenuEntry, depth: number)}
  {#if entry.sepBefore && depth === 0}<div class="sep"></div>{/if}
  {#if entry.children}
    <div class="group" style:padding-left="{10 + depth * 10}px">{$t(entry.key, { default: entry.label, ...entry.vars })}</div>
    {#each entry.children as child (child.id)}
      {@render item(child, depth + 1)}
    {/each}
  {:else}
    <button
      type="button"
      role="menuitem"
      class:danger={entry.danger}
      style:padding-left="{10 + depth * 10}px"
      onclick={() => act(entry)}
    >
      {$t(entry.key, { default: entry.label, ...entry.vars })}
    </button>
  {/if}
{/snippet}

{#if menu}
  <div
    class="menu"
    bind:this={el}
    data-canvas-chrome
    role="menu"
    tabindex="-1"
    style:left="{pos.x}px"
    style:top="{pos.y}px"
    bind:clientHeight={menuH}
    onkeydown={onkey}
  >
    {#if !menu.nodeId && !menu.connectionId}
      <div class="title">
        {menu.connectFrom
          ? $t("canvas.engine.menu.connectNew", { default: "New connected…" })
          : $t("canvas.engine.menu.add", { default: "Add" })}
      </div>
    {/if}
    {#each entries as entry (entry.id)}
      {@render item(entry, 0)}
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: absolute;
    min-width: 200px;
    padding: 4px;
    border-radius: 10px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
    display: flex;
    flex-direction: column;
    z-index: 20;
    outline: none;
    max-height: calc(100% - 16px);
    overflow-y: auto;
  }
  .title {
    padding: 4px 10px;
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  button {
    all: unset;
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
  }
  button:hover,
  button:focus-visible {
    background: var(--accent);
    color: var(--on-accent, #fff);
  }
  .danger {
    color: var(--danger, #e5484d);
  }
  .group {
    padding: 6px 10px 2px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
</style>
