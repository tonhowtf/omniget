<script lang="ts">
  // Floors overview (Maestri ⌘⇧\): the workspace's floors as a stack of
  // slabs, ground at the bottom. Each slab says its branch, how many
  // terminals it has and how many need you. ↑/↓ move, Enter or a double
  // click switches and closes, Esc closes.
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { EVENT_FLOORS, errorText, listFloors, type FloorView } from "$lib/canvas/floors/api";

  interface Activity {
    floorId: string;
    attention: number;
    working: number;
    alive: number;
    terminals: number;
  }

  interface Props {
    workspaceId: string;
    floorId: string;
    onSwitch: (floorId: string) => void;
    onClose: () => void;
  }
  let { workspaceId, floorId, onSwitch, onClose }: Props = $props();

  let floors = $state<FloorView[]>([]);
  let activity = $state<Record<string, Activity>>({});
  let cursor = $state(0);
  let placed = false;
  let error = $state("");
  let box: HTMLDivElement | undefined = $state();

  // Top of the stack first: the newest floor above, the ground at the bottom.
  const stack = $derived([...floors].sort((a, b) => (a.isGround ? 1 : b.isGround ? -1 : b.createdAt.localeCompare(a.createdAt))));

  async function refresh() {
    try {
      const [f, a] = await Promise.all([
        listFloors(workspaceId),
        invoke<Activity[]>("canvas_floor_activity", { workspaceId }),
      ]);
      floors = f;
      activity = Object.fromEntries(a.map((x) => [x.floorId, x]));
      // Only the first load puts the cursor on the current floor; the poll
      // must not undo the person's ↑/↓.
      if (!placed) {
        const i = stack.findIndex((x) => x.id === floorId);
        if (i >= 0) cursor = i;
        placed = true;
      }
    } catch (e) {
      error = errorText(e);
    }
  }

  let unlisten: UnlistenFn | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;
  onMount(async () => {
    await refresh();
    box?.focus();
    unlisten = await listen(EVENT_FLOORS, () => void refresh());
    // States change without a floors event: poll while open.
    timer = setInterval(() => void refresh(), 2000);
  });
  onDestroy(() => {
    unlisten?.();
    if (timer) clearInterval(timer);
  });

  function pick(id: string) {
    if (id !== floorId) onSwitch(id);
    onClose();
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      e.stopPropagation();
      cursor = Math.max(0, cursor - 1);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      e.stopPropagation();
      cursor = Math.min(stack.length - 1, cursor + 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      const f = stack[cursor];
      if (f) pick(f.id);
    } else if ((e.metaKey || e.ctrlKey) && e.shiftKey && (e.code === "Backslash" || e.key === "|" || e.key === "\\")) {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    }
  }
</script>

<div
  class="overview"
  data-canvas-chrome
  role="dialog"
  aria-label={$t("canvas.floors.overview.title", { default: "Floors" })}
  tabindex="-1"
  bind:this={box}
  onkeydown={onkey}
>
  <header>
    <h3>{$t("canvas.floors.overview.title", { default: "Floors" })}</h3>
    <span class="hint">{$t("canvas.floors.overview.hint", { default: "↑ ↓ to move, Enter to go, Esc to close" })}</span>
  </header>
  {#if error}<p class="error">{error}</p>{/if}
  <ol class="stack" role="listbox" tabindex="-1" aria-activedescendant={stack[cursor] ? `floor-${stack[cursor].id}` : undefined}>
    {#each stack as f, i (f.id)}
      {@const a = activity[f.id]}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <li
        id={`floor-${f.id}`}
        role="option"
        aria-selected={i === cursor}
        class:cursor={i === cursor}
        class:current={f.id === floorId}
        style:--fc={f.color ?? "var(--accent)"}
        style:--depth={stack.length - 1 - i}
        onmouseenter={() => (cursor = i)}
        onclick={() => (cursor = i)}
        ondblclick={() => pick(f.id)}
      >
        <div class="slab" aria-hidden="true">
          {#if a?.attention}<span class="pin">{a.attention}</span>{/if}
          {#each Array(Math.min(a?.terminals ?? f.terminalCount, 8)) as _, k (k)}
            <span class="dot" class:busy={k < (a?.working ?? 0)} class:needs={k >= (a?.working ?? 0) && k < (a?.working ?? 0) + (a?.attention ?? 0)}></span>
          {/each}
        </div>
        <div class="label">
          <strong>{f.isGround ? $t("canvas.floors.ground", { default: "Ground" }) : f.name}</strong>
          <span class="muted">
            {#if f.branch}<code>{f.branch}</code>{/if}
            {#if !f.isGitIsolated && !f.isGround}{$t("canvas.floors.overview.shared", { default: "shares the ground folder" })}{/if}
          </span>
          <span class="stats">
            {$t("canvas.floors.overview.terminals", { default: "{{count}} terminals", count: f.terminalCount })}
            {#if a?.working}· {$t("canvas.floors.overview.working", { default: "{{count}} working", count: a.working })}{/if}
            {#if a?.attention}<b class="needs-you">· {$t("canvas.floors.overview.attention", { default: "{{count}} need you", count: a.attention })}</b>{/if}
            {#if f.ahead}· {$t("canvas.floors.overview.ahead", { default: "{{count}} commits ahead", count: f.ahead })}{/if}
            {#if f.dirty}· {$t("canvas.floors.overview.dirty", { default: "{{count}} changed files", count: f.dirty })}{/if}
          </span>
        </div>
        {#if f.id === floorId}<span class="here">{$t("canvas.floors.overview.here", { default: "You are here" })}</span>{/if}
      </li>
    {/each}
  </ol>
</div>

<style>
  .overview {
    width: min(560px, calc(100vw - 32px));
    max-height: 80vh;
    overflow: auto;
    background: var(--popup-bg, var(--surface));
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 16px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
    padding: 14px 16px;
    outline: none;
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 10px;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .hint,
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .error {
    color: var(--danger);
    font-size: 12px;
  }
  .stack {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  li {
    display: grid;
    grid-template-columns: 150px 1fr auto;
    align-items: center;
    gap: 14px;
    padding: 6px 8px;
    border-radius: 12px;
    cursor: pointer;
    border: 1px solid transparent;
  }
  li.cursor {
    background: color-mix(in srgb, var(--fc) 10%, transparent);
    border-color: color-mix(in srgb, var(--fc) 40%, transparent);
  }
  /* The slab: a floor plate seen from above at an angle, stacked. */
  .slab {
    position: relative;
    height: 46px;
    border-radius: 8px;
    background: linear-gradient(180deg, color-mix(in srgb, var(--fc) 55%, var(--surface)), color-mix(in srgb, var(--fc) 30%, var(--surface)));
    border: 1px solid color-mix(in srgb, var(--fc) 70%, black);
    transform: perspective(420px) rotateX(48deg);
    transform-origin: 50% 70%;
    box-shadow: 0 10px 0 -4px color-mix(in srgb, var(--fc) 40%, black);
    display: flex;
    flex-wrap: wrap;
    align-content: center;
    justify-content: center;
    gap: 4px;
    padding: 4px 10px;
  }
  li.current .slab {
    outline: 2px solid var(--fc);
    outline-offset: 2px;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--text) 45%, transparent);
  }
  .dot.busy {
    background: var(--accent);
  }
  .dot.needs {
    background: var(--danger, #e5484d);
  }
  .pin {
    position: absolute;
    top: -8px;
    right: -6px;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--danger, #e5484d);
    color: white;
    font-size: 11px;
    font-weight: 700;
    line-height: 18px;
    text-align: center;
    transform: rotateX(-48deg);
  }
  .label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .label strong {
    font-size: 14px;
  }
  .label code {
    font-size: 11px;
  }
  .stats {
    font-size: 12px;
    color: var(--text-secondary, var(--text-muted));
  }
  .needs-you {
    color: var(--danger, #e5484d);
  }
  .here {
    font-size: 11px;
    color: var(--fc);
    font-weight: 600;
    white-space: nowrap;
  }
  @media (max-width: 480px) {
    li {
      grid-template-columns: 90px 1fr;
    }
    .here {
      display: none;
    }
  }
</style>
