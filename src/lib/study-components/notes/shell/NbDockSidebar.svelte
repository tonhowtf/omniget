<script lang="ts">
  import { docksStore, DOCK_META, type DockIconKey } from "$lib/study-notes/docks-store.svelte";
import { t } from "$lib/i18n";
  import type { DockPosition } from "$lib/notes-bridge";

  type Props = { side: "left" | "right" };
  let { side }: Props = $props();

  const sideAsPos = $derived(side);

  const items = $derived(
    DOCK_META.filter((m) => {
      const cur = docksStore.states[m.id];
      const pos = cur?.position ?? m.defaultPosition;
      return pos === sideAsPos;
    }),
  );

  function isVisible(id: string): boolean {
    return docksStore.states[id]?.visible ?? false;
  }

  function toggle(id: string) {
    void docksStore.toggle(id);
  }

  function iconPath(key: DockIconKey): string {
    switch (key) {
      case "files":
        return "M3 7l1.7-3.4A2 2 0 0 1 6.5 2.5h11A2 2 0 0 1 19.3 3.6L21 7M3 7v12a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V7M3 7h18M8 11h8";
      case "list-tree":
        return "M21 12h-8M21 6h-8M21 18h-8M3 6v12M3 6l4 4M3 12l4-4M3 18l4-4";
      case "link":
        return "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71";
      case "bookmark":
        return "M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z";
      case "tags":
        return "M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82zM7 7h.01";
      case "inbox":
        return "M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z";
      case "share-2":
        return "M18 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 22a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.59 13.51l6.83 3.98M15.41 6.51l-6.82 3.98";
    }
  }
</script>

<aside class="nb-dock-sidebar" data-side={side} aria-label="Toggle docks {side}">
  {#each items as item (item.id)}
    <button
      type="button"
      class="icon-btn"
      class:active={isVisible(item.id)}
      onclick={() => toggle(item.id)}
      title={$t("study.notes.nb.sidebar_hint", { label: item.label, state: isVisible(item.id) ? $t("study.notes.nb.visible") : $t("study.notes.nb.hidden") })}
      aria-pressed={isVisible(item.id)}
      aria-label={item.label}
    >
      <svg
        viewBox="0 0 24 24"
        width="16"
        height="16"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d={iconPath(item.iconKey)} />
      </svg>
    </button>
  {/each}
</aside>

<style>
  .nb-dock-sidebar {
    width: 32px;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 4px 0;
    gap: 2px;
    background: color-mix(in oklab, var(--surface-bg, var(--primary)) 60%, transparent);
  }
  .nb-dock-sidebar[data-side="left"] {
    border-right: none;
  }
  .nb-dock-sidebar[data-side="right"] {
    border-left: none;
  }
  .icon-btn {
    width: 26px;
    height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    border-radius: 5px;
    cursor: pointer;
    color: var(--tertiary, var(--muted-fg));
    transition: background 120ms ease, color 120ms ease;
  }
  .icon-btn:hover {
    color: var(--secondary, var(--text));
    background: color-mix(in oklab, var(--content-border) 30%, transparent);
  }
  .icon-btn.active {
    color: var(--accent);
    background: color-mix(in oklab, var(--accent) 16%, transparent);
  }
</style>
