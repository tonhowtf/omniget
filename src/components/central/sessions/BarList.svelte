<script lang="ts">
  /**
   * Lista ranqueada com barra horizontal de um tom só (magnitude): nome,
   * trilho, valor. Um item pode ter `sub` (linha secundária), `tag` (ex.:
   * "beta") e virar botão quando `onpick` existe.
   */
  type Item = { key: string; label: string; value: number; sub?: string; tag?: string; title?: string };

  interface Props {
    items: Item[];
    format?: (v: number) => string;
    limit?: number;
    onpick?: (key: string) => void;
    empty?: string;
  }

  let { items, format = (v) => String(v), limit = 8, onpick, empty = "—" }: Props = $props();

  let shown = $derived(items.slice(0, limit));
  let max = $derived(Math.max(1e-9, ...shown.map((i) => i.value)));
</script>

{#if shown.length === 0}
  <p class="empty">{empty}</p>
{:else}
  <ul class="bars">
    {#each shown as it (it.key)}
      <li>
        {#if onpick}
          <button type="button" class="row" onclick={() => onpick?.(it.key)} title={it.title ?? it.label}>
            {@render body(it)}
          </button>
        {:else}
          <div class="row" title={it.title ?? it.label}>{@render body(it)}</div>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

{#snippet body(it: Item)}
  <span class="name">
    <span class="label">{it.label}</span>
    {#if it.tag}<span class="tag">{it.tag}</span>{/if}
    {#if it.sub}<span class="sub">{it.sub}</span>{/if}
  </span>
  <span class="track" aria-hidden="true"><i style:width="{(Math.max(0, it.value) / max) * 100}%"></i></span>
  <span class="num">{format(it.value)}</span>
{/snippet}

<style>
  .bars {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    width: 100%;
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(40px, 1fr) auto;
    align-items: center;
    gap: 10px;
    padding: 5px 6px;
    border: none;
    background: none;
    border-radius: var(--radius-sm);
    font: inherit;
    font-size: var(--text-sm);
    color: var(--text);
    text-align: left;
  }
  button.row {
    cursor: pointer;
  }
  @media (hover: hover) {
    button.row:hover {
      background: var(--fill-1);
    }
  }
  button.row:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -1px;
  }
  .name {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: var(--text-caption);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    align-self: flex-start;
    font-size: var(--text-caption);
    color: var(--text-muted);
    background: var(--fill-2);
    border-radius: var(--radius-xs);
    padding: 0 4px;
    margin-top: 1px;
  }
  .track {
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--viz-empty);
    overflow: hidden;
  }
  .track i {
    display: block;
    height: 100%;
    border-radius: var(--radius-full);
    background: var(--viz-seq);
  }
  .num {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    text-align: right;
    min-width: 3.5em;
  }
  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
