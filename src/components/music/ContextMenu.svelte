<script lang="ts" module>
  export type MenuItem =
    | { divider: true }
    | {
        label: string;
        icon?: string;
        action?: () => void;
        danger?: boolean;
        disabled?: boolean;
        checked?: boolean;
        sub?: MenuItem[];
        hint?: string;
      };
</script>

<script lang="ts">
  import { onMount, tick } from "svelte";
  import MusicIcon from "./MusicIcon.svelte";

  let { x, y, items, onclose }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ left: 0, top: 0 });
  let openSub = $state<number | null>(null);
  let subPos = $state({ left: 0, top: 0 });

  onMount(() => {
    void place();
    const away = (e: PointerEvent) => {
      if (el && !el.contains(e.target as Node)) onclose();
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onclose();
      }
    };
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", esc, true);
    window.addEventListener("blur", onclose);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", esc, true);
      window.removeEventListener("blur", onclose);
    };
  });

  async function place() {
    await tick();
    if (!el) return;
    const r = el.getBoundingClientRect();
    pos = {
      left: Math.min(x, window.innerWidth - r.width - 8),
      top: Math.min(y, window.innerHeight - r.height - 8),
    };
    el.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }

  function run(it: MenuItem) {
    if ("divider" in it || it.disabled) return;
    if (it.sub) return;
    it.action?.();
    onclose();
  }

  function showSub(i: number, e: Event) {
    const btn = e.currentTarget as HTMLElement;
    const r = btn.getBoundingClientRect();
    const right = r.right + 220 < window.innerWidth;
    subPos = { left: right ? r.right - 4 : r.left - 216, top: r.top - 4 };
    openSub = i;
  }

  function keynav(e: KeyboardEvent) {
    if (!el) return;
    const btns = [...el.querySelectorAll<HTMLButtonElement>(".cm-main > button:not(:disabled)")];
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown") btns[(i + 1) % btns.length]?.focus();
    else if (e.key === "ArrowUp") btns[(i - 1 + btns.length) % btns.length]?.focus();
    else return;
    e.preventDefault();
  }
</script>

<div class="cm" bind:this={el} style:left="{pos.left}px" style:top="{pos.top}px" role="menu" tabindex="-1" onkeydown={keynav}>
  <div class="cm-main">
    {#each items as it, i (i)}
      {#if "divider" in it}
        <div class="cm-div"></div>
      {:else}
        <button
          class="cm-item"
          class:danger={it.danger}
          role="menuitem"
          disabled={it.disabled}
          onclick={() => run(it)}
          onpointerenter={(e) => (it.sub ? showSub(i, e) : (openSub = null))}
          onfocus={(e) => (it.sub ? showSub(i, e) : (openSub = null))}
        >
          <span class="cm-ic">{#if it.checked}<MusicIcon name="check" size={14} />{:else if it.icon}<MusicIcon name={it.icon} size={14} />{/if}</span>
          <span class="cm-l">{it.label}</span>
          {#if it.hint}<span class="cm-h">{it.hint}</span>{/if}
          {#if it.sub}<MusicIcon name="chevron-right" size={13} />{/if}
        </button>
      {/if}
    {/each}
  </div>
  {#if openSub != null}
    {@const parent = items[openSub]}
    {#if !("divider" in parent) && parent.sub}
      <div class="cm cm-sub" style:left="{subPos.left}px" style:top="{subPos.top}px" role="menu">
        {#each parent.sub as it, j (j)}
          {#if "divider" in it}
            <div class="cm-div"></div>
          {:else}
            <button class="cm-item" class:danger={it.danger} role="menuitem" disabled={it.disabled} onclick={() => run(it)}>
              <span class="cm-ic">{#if it.checked}<MusicIcon name="check" size={14} />{:else if it.icon}<MusicIcon name={it.icon} size={14} />{/if}</span>
              <span class="cm-l">{it.label}</span>
            </button>
          {/if}
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .cm {
    position: fixed;
    z-index: 1000;
    min-width: 200px;
    max-width: 280px;
    max-height: 70vh;
    overflow-y: auto;
    padding: 4px;
    border-radius: var(--radius-md);
    background: var(--surface-hi);
    box-shadow:
      0 0 0 var(--hairline) var(--content-border),
      0 12px 32px rgba(0, 0, 0, 0.35);
    outline: none;
  }
  .cm-sub {
    position: fixed;
  }
  .cm-item {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border-radius: 5px;
    font-size: var(--text-base);
    color: var(--text);
    cursor: default;
  }
  .cm-item:hover,
  .cm-item:focus-visible {
    background: var(--accent);
    color: var(--on-accent);
  }
  .cm-item:disabled {
    opacity: 0.4;
  }
  .cm-item.danger {
    color: var(--danger);
  }
  .cm-item.danger:hover {
    background: var(--danger);
    color: white;
  }
  .cm-ic {
    width: 14px;
    display: grid;
    place-items: center;
  }
  .cm-l {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cm-h {
    font-size: var(--text-xs);
    opacity: 0.6;
  }
  .cm-div {
    height: var(--hairline);
    margin: 4px 6px;
    background: var(--content-border);
  }
</style>
