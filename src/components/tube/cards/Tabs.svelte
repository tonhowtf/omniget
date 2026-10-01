<script lang="ts">
  export type TabDef = { id: string; label: string };

  let {
    tabs,
    active,
    onselect,
    label = "",
  }: { tabs: TabDef[]; active: string; onselect: (id: string) => void; label?: string } = $props();

  let bar = $state<HTMLDivElement>();
  let ind = $state({ left: 0, width: 0, ready: false });

  $effect(() => {
    void active;
    void tabs.length;
    if (!bar) return;
    const measure = () => {
      const el = bar?.querySelector<HTMLElement>(`[data-tab="${CSS.escape(active)}"]`);
      if (!el) {
        ind = { left: 0, width: 0, ready: false };
        return;
      }
      ind = { left: el.offsetLeft, width: el.offsetWidth, ready: true };
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(bar);
    return () => ro.disconnect();
  });

  function key(e: KeyboardEvent, i: number) {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const n = tabs[(i + (e.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length];
    onselect(n.id);
    bar?.querySelector<HTMLElement>(`[data-tab="${CSS.escape(n.id)}"]`)?.focus();
  }
</script>

<div class="tabs" role="tablist" aria-label={label || undefined} bind:this={bar}>
  {#each tabs as tab, i (tab.id)}
    <button
      role="tab"
      data-tab={tab.id}
      aria-selected={tab.id === active}
      tabindex={tab.id === active ? 0 : -1}
      class:on={tab.id === active}
      onclick={() => onselect(tab.id)}
      onkeydown={(e) => key(e, i)}
    >
      {tab.label}
    </button>
  {/each}
  <span class="ind" class:ready={ind.ready} style:transform="translateX({ind.left}px)" style:width="{ind.width}px"></span>
</div>

<style>
  .tabs {
    position: relative;
    display: flex;
    gap: 24px;
    overflow-x: auto;
    scrollbar-width: none;
    border-bottom: 1px solid var(--yt-divider);
  }
  .tabs::-webkit-scrollbar {
    display: none;
  }
  button {
    flex: none;
    height: 48px;
    padding: 0;
    border: 0;
    font: 500 16px/22px var(--yt-font);
    color: var(--yt-text-2);
    background: none;
    cursor: pointer;
    transition: color var(--yt-dur-base);
  }
  button:hover,
  button.on {
    color: var(--yt-text);
  }
  button:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: -2px;
    border-radius: 4px;
  }
  .ind {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    border-radius: 2px;
    background: var(--yt-text);
    opacity: 0;
  }
  .ind.ready {
    opacity: 1;
    transition: transform 300ms var(--yt-ease-standard), width 300ms var(--yt-ease-standard);
  }
  @media (prefers-reduced-motion: reduce) {
    .ind.ready {
      transition: none;
    }
  }
</style>
