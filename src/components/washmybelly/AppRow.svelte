<script lang="ts">
  import { t } from "$lib/i18n";
  import { formatBytes } from "$lib/washmybelly/format";
  import { lastUsed, type App } from "$lib/washmybelly/apps";
  import AppTile from "./AppTile.svelte";
  import Icon from "./ui/Icon.svelte";

  let {
    app,
    selected = false,
    sourceLabel,
    reasonText,
    now,
    onSelect,
  }: {
    app: App;
    selected?: boolean;
    sourceLabel: (source: string) => string;
    reasonText: (reason: string | null | undefined) => string;
    now: number;
    onSelect: (app: App) => void;
  } = $props();

  let used = $derived(lastUsed(app.last_used, now));
  let locked = $derived(!app.removable);
</script>

<button
  type="button"
  class="wmb-app-row"
  class:selected
  class:locked
  aria-current={selected ? "true" : undefined}
  onclick={() => onSelect(app)}
>
  <AppTile name={app.name} source={app.source} dim={locked} />
  <span class="text">
    <span class="name" title={app.name}>{app.name}</span>
    <span class="sub">
      {#if app.version}<span class="ver">{app.version}</span><span class="dot">·</span>{/if}
      {#if locked}
        <span class="why"><Icon name="lock" size={10} />{reasonText(app.reason)}</span>
      {:else if used}
        <span class:stale={used.stale} title={sourceLabel(app.source)}>{$t(used.key, { count: used.count })}</span>
      {:else}
        <span>{sourceLabel(app.source)}</span>
      {/if}
    </span>
  </span>
  <span class="size">{app.bytes != null ? formatBytes(app.bytes) : ""}</span>
</button>

<style>
  .wmb-app-row {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-height: 50px;
    padding: 7px var(--space-3);
    cursor: default;
    position: relative;
  }
  .wmb-app-row + :global(.wmb-app-row)::before {
    content: "";
    position: absolute;
    top: 0;
    left: calc(var(--space-3) + 32px + var(--space-3));
    right: 0;
    height: var(--hairline);
    background: var(--separator);
  }
  @media (hover: hover) {
    .wmb-app-row:hover:not(.selected) {
      background: var(--fill-1);
    }
  }
  .wmb-app-row.selected {
    background: var(--accent-soft);
  }
  .wmb-app-row:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .name {
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-sm);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    opacity: 0.6;
  }
  .ver {
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 0;
  }
  .stale {
    color: var(--warning);
  }
  .why {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .locked .name {
    color: var(--text-dim);
  }
  .size {
    flex-shrink: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
