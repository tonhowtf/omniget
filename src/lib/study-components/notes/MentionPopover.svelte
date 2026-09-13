<script lang="ts">
  import { t } from "$lib/i18n";
  import type { MentionItem } from "./mention-suggestions";

  type Props = {
    items: MentionItem[];
    selectedIndex: number;
    kind: "page" | "tag" | null;
    position: { x: number; y: number };
    onPick: (item: MentionItem) => void;
  };

  let { items, selectedIndex, kind, position, onPick }: Props = $props();

  const headerLabel = $derived(kind === "tag" ? $t("study.notes.nb.mention_tags") : $t("study.notes.nb.mention_pages"));
</script>

{#if items.length > 0}
  <div
    class="mention-popover"
    style:left={`${position.x}px`}
    style:top={`${position.y}px`}
    role="listbox"
    aria-label={headerLabel}
  >
    <header class="popover-header">{headerLabel}</header>
    {#each items as item, i (item.id)}
      <button
        type="button"
        class="row"
        class:active={i === selectedIndex}
        role="option"
        aria-selected={i === selectedIndex}
        onmousedown={(e) => {
          e.preventDefault();
          onPick(item);
        }}
      >
        <span class="indicator">{kind === "tag" ? "#" : "[["}</span>
        <span class="body">
          <span class="label">{item.label}</span>
          {#if item.hint}<span class="hint">{item.hint}</span>{/if}
        </span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .mention-popover {
    position: fixed;
    z-index: 1000;
    min-width: 240px;
    max-width: 320px;
    max-height: 320px;
    overflow-y: auto;
    background: var(--surface);
    border: none;
    border-radius: var(--border-radius);
    box-shadow: 0 12px 32px color-mix(in oklab, black 28%, transparent);
    padding: 4px;
  }
  .popover-header {
    font-size: 10px;
    font-weight: 600;
    color: var(--tertiary);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 4px 10px 2px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 6px 10px;
    border: 0;
    border-radius: var(--border-radius);
    background: transparent;
    cursor: pointer;
    text-align: left;
    color: var(--text);
    font: inherit;
  }
  .row:hover {
    background: color-mix(in oklab, var(--accent) 8%, transparent);
  }
  .row.active {
    background: color-mix(in oklab, var(--accent) 14%, transparent);
  }
  .indicator {
    flex: 0 0 auto;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--accent);
    user-select: none;
  }
  .body {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .label {
    font-size: 13px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    font-size: 11px;
    color: var(--tertiary);
  }
</style>
