<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { arenaCompare, arenaError, type ArenaCompare, type ArenaView } from "$lib/central/arena";
  import Icon from "$components/central/threads/Icon.svelte";

  let { arena, oncompare }: { arena: ArenaView; oncompare?: (c: ArenaCompare | null) => void } = $props();

  let data = $state<ArenaCompare | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let showAgreed = $state(false);

  let key = $derived(`${arena.arenaId}:${arena.entries.map((e) => `${e.status}:${e.commitSha ?? ""}`).join(",")}`);
  $effect(() => {
    void key;
    untrack(() => void load());
  });

  async function load() {
    loading = true;
    error = null;
    try {
      data = await arenaCompare(arena.arenaId);
    } catch (e) {
      data = null;
      error = arenaError(e).message;
    } finally {
      loading = false;
      oncompare?.(data);
    }
  }

  let columns = $derived(arena.entries.filter((e) => data?.entries.includes(e.entryId)));
  let rows = $derived((data?.files ?? []).filter((f) => f.divergent || showAgreed));
  const HUES = ["var(--success)", "var(--purple)", "var(--warning)", "var(--accent)", "var(--error)"];
  const hue = (variant: number) => (variant === 0 ? "transparent" : HUES[(variant - 1) % HUES.length]);
</script>

<section class="cmp" aria-label={$t("llm.central.arena.compare.title")}>
  <header class="head">
    <strong>{$t("llm.central.arena.compare.title")}</strong>
    {#if data}
      <span class="chip bad">{$t("llm.central.arena.compare.divergent", { count: data.divergent })}</span>
      <span class="chip">{$t("llm.central.arena.compare.agreed", { count: data.agreed })}</span>
      {#if data.pending.length}<span class="muted">{$t("llm.central.arena.compare.pending", { count: data.pending.length })}</span>{/if}
    {/if}
    <div class="grow"></div>
    <label class="toggle"><input type="checkbox" bind:checked={showAgreed} /> {$t("llm.central.arena.compare.show_agreed")}</label>
    <button type="button" class="mini" onclick={load} disabled={loading} title={$t("llm.central.arena.refresh")}>
      <Icon name={loading ? "circle-notch" : "arrow-counter-clockwise"} size={12} />
    </button>
  </header>
  {#if error}
    <p class="err">{error}</p>
  {:else if data && !data.files.length}
    <p class="muted pad">{$t("llm.central.arena.no_changes")}</p>
  {:else if data}
    <div class="table" role="table" style:--cols={columns.length}>
      <div class="tr th" role="row">
        <span role="columnheader">{$t("llm.central.arena.compare.file")}</span>
        {#each columns as c (c.entryId)}<span role="columnheader" class="col-name" title={c.label}>{c.label}</span>{/each}
      </div>
      {#each rows as f (f.path)}
        <div class="tr" class:split={f.divergent} role="row">
          <span class="path mono" role="cell" title={f.path}>
            {#if f.divergent}<Icon name="arrows-split" size={12} />{/if}{f.path}
          </span>
          {#each columns as c (c.entryId)}
            {@const cell = f.cells.find((x) => x.entryId === c.entryId)}
            <span role="cell" class="cell" class:same={!cell || cell.status === "="} style:--hue={hue(cell?.variant ?? 0)}>
              {#if !cell || cell.status === "="}
                {$t("llm.central.arena.compare.untouched")}
              {:else}
                <b>{cell.status}</b>
                {#if cell.additions != null}<span class="add">+{cell.additions}</span>{/if}
                {#if cell.deletions != null}<span class="del">−{cell.deletions}</span>{/if}
              {/if}
            </span>
          {/each}
        </div>
      {/each}
      {#if !rows.length}<p class="muted pad">{$t("llm.central.arena.compare.none_divergent")}</p>{/if}
    </div>
  {:else}
    <p class="muted pad">{$t("llm.central.arena.working")}</p>
  {/if}
</section>

<style>
  .cmp { border: 1px solid var(--separator); border-radius: var(--radius-lg); background: var(--surface); margin: 10px 12px 0; display: flex; flex-direction: column; max-height: 40vh; min-height: 0; }
  .head { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-bottom: 1px solid var(--separator); flex-wrap: wrap; font-size: var(--text-sm); }
  .chip { font-size: var(--text-xs); font-weight: 600; padding: 1px 8px; border-radius: 999px; background: var(--fill-1); }
  .chip.bad { color: var(--warning); background: color-mix(in srgb, var(--warning) 12%, transparent); }
  .grow { flex: 1; }
  .toggle { display: inline-flex; align-items: center; gap: 4px; color: var(--text-muted); }
  .mini { width: 24px; height: 24px; display: inline-flex; align-items: center; justify-content: center; border: 0; border-radius: 6px; background: transparent; color: var(--text-muted); cursor: pointer; }
  .mini:hover:not(:disabled) { background: var(--fill-2); color: var(--text); }
  .table { overflow: auto; min-height: 0; }
  .tr { display: grid; grid-template-columns: minmax(180px, 2fr) repeat(var(--cols), minmax(110px, 1fr)); gap: 0; border-bottom: 1px solid var(--separator); font-size: var(--text-sm); }
  .tr > * { padding: 5px 10px; min-width: 0; }
  .th { position: sticky; top: 0; background: var(--surface); font-size: var(--text-xs); font-weight: 600; color: var(--text-muted); z-index: 1; }
  .col-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .split { background: color-mix(in srgb, var(--warning) 6%, transparent); }
  .path { display: flex; align-items: center; gap: 5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .split .path { color: var(--warning); font-weight: 600; }
  .cell { display: flex; align-items: center; gap: 5px; font-variant-numeric: tabular-nums; border-left: 3px solid var(--hue); }
  .cell.same { color: var(--text-muted); }
  .add { color: var(--success); }
  .del { color: var(--error); }
  .mono { font-family: var(--font-mono); }
  .muted { color: var(--text-muted); font-size: var(--text-sm); }
  .pad { padding: 10px; margin: 0; }
  .err { color: var(--error); margin: 0; padding: 10px; font-size: var(--text-sm); overflow-wrap: anywhere; }
</style>
