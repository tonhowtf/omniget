<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import * as api from "$lib/photos/api";
  import { SIMILARITY, defaultRemovals, formatBytes, keepsOne, savedBytes } from "$lib/photos/plan";
  import type { DupGroup, FileReport, Photo, RemoveMode } from "$lib/photos/types";
  import PhotoTile from "./PhotoTile.svelte";
  import RemoveDialog from "./RemoveDialog.svelte";

  let {
    photos,
    indexing,
    onopen,
    onremoved,
  }: {
    photos: Map<string, Photo>;
    indexing: boolean;
    onopen: (path: string, order: string[]) => void;
    onremoved: (report: FileReport) => void;
  } = $props();

  let level = $state<keyof typeof SIMILARITY>("close");
  let groups = $state<DupGroup[]>([]);
  let chosen = $state(new Set<string>());
  let loading = $state(true);
  let error = $state<string | null>(null);
  let confirming = $state(false);
  let removing = $state(false);

  let sizes = $derived(new Map([...photos.values()].map((p) => [p.path, p.size])));
  let visible = $derived(groups.map((g) => ({ ...g, paths: g.paths.filter((p) => photos.has(p)) })).filter((g) => g.paths.length > 1));
  let picked = $derived(visible.flatMap((g) => g.paths.filter((p) => chosen.has(p))));
  let bytes = $derived(savedBytes(visible, chosen, sizes));
  let emptied = $derived(visible.some((g) => !keepsOne(g, chosen)));
  let order = $derived(visible.flatMap((g) => g.paths));

  async function load() {
    loading = true;
    error = null;
    try {
      groups = await api.duplicates(SIMILARITY[level]);
      chosen = defaultRemovals(groups);
    } catch (e) {
      error = api.errorText(e);
    } finally {
      loading = false;
    }
  }

  function toggle(path: string) {
    const next = new Set(chosen);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    chosen = next;
  }

  function pickLevel(next: keyof typeof SIMILARITY) {
    if (next === level) return;
    level = next;
    void load();
  }

  async function remove(mode: RemoveMode) {
    confirming = false;
    removing = true;
    try {
      const report = await api.remove(picked, mode);
      onremoved(report);
      const gone = new Set(report.done);
      groups = groups.map((g) => ({ ...g, paths: g.paths.filter((p) => !gone.has(p)) })).filter((g) => g.paths.length > 1);
      chosen = new Set([...chosen].filter((p) => !gone.has(p)));
      if (report.failed.length) error = $t("photos.remove.some_failed", { count: report.failed.length }) as string;
    } catch (e) {
      error = api.errorText(e);
    } finally {
      removing = false;
    }
  }

  onMount(() => {
    void load();
  });
</script>

<div class="dp">
  <div class="dp-bar">
    <div class="segmented" role="radiogroup" aria-label={$t("photos.dups.level") as string}>
      <button type="button" class="segmented-btn" role="radio" aria-checked={level === "close"} onclick={() => pickLevel("close")}>{$t("photos.dups.level_close")}</button>
      <button type="button" class="segmented-btn" role="radio" aria-checked={level === "similar"} onclick={() => pickLevel("similar")}>{$t("photos.dups.level_similar")}</button>
    </div>
    <button type="button" class="btn btn-ghost btn-sm" disabled={loading} onclick={() => void load()}>{$t("photos.dups.refresh")}</button>
  </div>
  <p class="field-hint">{level === "close" ? $t("photos.dups.level_close_hint") : $t("photos.dups.level_similar_hint")}</p>

  {#if indexing}
    <p class="field-hint">{$t("photos.dups.indexing")}</p>
  {/if}

  {#if error}
    <div class="notice notice-warning" role="alert"><p class="notice-text">{error}</p></div>
  {/if}

  {#if loading}
    <div class="dp-empty"><span class="spinner" aria-hidden="true"></span><p>{$t("photos.dups.searching")}</p></div>
  {:else if visible.length === 0}
    <div class="dp-empty">
      <h2>{$t("photos.dups.none_title")}</h2>
      <p>{$t("photos.dups.none_body")}</p>
    </div>
  {:else}
    <p class="dp-summary" role="status">{$t("photos.dups.summary", { groups: visible.length, size: formatBytes(bytes) })}</p>
    {#each visible as group (group.keep)}
      <section class="dp-group">
        <header class="dp-group-head">
          <h3>{group.kind === "exact" ? $t("photos.dups.kind_exact") : $t("photos.dups.kind_similar")}</h3>
          <span>{$t("photos.dups.count", { count: group.paths.length })}</span>
          {#if !keepsOne(group, chosen)}
            <span class="tag tag-warning">{$t("photos.dups.all_marked")}</span>
          {/if}
        </header>
        <div class="dp-tiles">
          {#each group.paths as path (path)}
            {@const p = photos.get(path)}
            {#if p}
              <figure class="dp-item">
                <PhotoTile
                  photo={p}
                  selected={chosen.has(path)}
                  badge={path === group.keep ? ($t("photos.dups.best") as string) : null}
                  checkLabel={$t("photos.dups.mark") as string}
                  onopen={() => onopen(path, order)}
                  ontoggle={() => toggle(path)}
                />
                <figcaption title={p.path}>
                  <span class="dp-name">{p.name}</span>
                  <span class="dp-meta">{p.width} × {p.height} · {formatBytes(p.size)}</span>
                </figcaption>
              </figure>
            {/if}
          {/each}
        </div>
      </section>
    {/each}
  {/if}

  {#if visible.length > 0}
    <footer class="dp-foot">
      <span>{$t("photos.dups.marked", { count: picked.length, size: formatBytes(bytes) })}</span>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => (chosen = new Set())}>{$t("photos.dups.unmark")}</button>
      <button type="button" class="btn btn-primary" disabled={picked.length === 0 || removing} onclick={() => (confirming = true)}>
        {removing ? $t("photos.remove.working") : $t("photos.dups.remove", { count: picked.length })}
      </button>
    </footer>
  {/if}
</div>

{#if confirming}
  <RemoveDialog count={picked.length} {bytes} emptiesGroup={emptied} onclose={() => (confirming = false)} onconfirm={(mode) => void remove(mode)} />
{/if}

<style>
  .dp {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .dp-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .dp-summary {
    margin: 0;
    color: var(--text);
    font-size: var(--text-base);
    font-weight: 600;
  }

  .dp-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-4);
    color: var(--text-dim);
    text-align: center;
  }

  .dp-empty h2 {
    margin: 0;
    color: var(--text);
    font-size: var(--text-lg);
  }

  .dp-empty p {
    margin: 0;
    max-width: 440px;
  }

  .dp-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    content-visibility: auto;
    contain-intrinsic-size: auto 240px;
  }

  .dp-group-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-dim);
    font-size: var(--text-sm);
  }

  .dp-group-head h3 {
    margin: 0;
    color: var(--text);
    font-size: var(--text-sm);
    font-weight: 650;
  }

  .dp-tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-3);
  }

  .dp-item {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    margin: 0;
  }

  .dp-item figcaption {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: var(--text-xs);
  }

  .dp-name {
    overflow: hidden;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dp-meta {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .dp-foot {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    background: var(--pane-bg);
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
</style>
