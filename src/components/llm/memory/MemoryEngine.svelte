<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import {
    memoryEntities,
    memoryStats,
    prepareSemanticMemory,
    retainText,
    searchMemory,
    setMemorySettings,
    type MemoryEntity,
    type MemoryStats,
    type RecallResult,
  } from "$lib/stores/assist-memory-store.svelte";

  let stats = $state<MemoryStats | null>(null);
  let entities = $state<MemoryEntity[]>([]);
  let query = $state("");
  let result = $state<RecallResult | null>(null);
  let searching = $state(false);
  let preparing = $state(false);
  let teachText = $state("");
  let teachContext = $state("");
  let teaching = $state(false);
  let message = $state<string | null>(null);
  let errorKey = $state<string | null>(null);

  const day = (ms: number) => new Date(ms).toLocaleDateString();

  async function refresh() {
    const [s, e] = await Promise.all([memoryStats(), memoryEntities(40)]);
    if (s.ok) stats = s.value;
    if (e.ok) entities = e.value;
  }

  async function search() {
    const q = query.trim();
    if (!q || searching) return;
    searching = true;
    const out = await searchMemory(q, 20);
    searching = false;
    if (out.ok) {
      result = out.value;
      errorKey = null;
    } else errorKey = out.errorKey;
  }

  async function toggle(key: "auto_retain" | "consolidate", value: boolean) {
    const out = await setMemorySettings({ [key]: value });
    if (out.ok) stats = out.value;
  }

  async function prepare() {
    preparing = true;
    const out = await prepareSemanticMemory();
    preparing = false;
    if (out.ok) {
      message = $t("assist.memory.engine.prepared", { count: out.value }) as string;
      await refresh();
    } else errorKey = out.errorKey;
  }

  async function teach() {
    const text = teachText.trim();
    if (!text || teaching) return;
    teaching = true;
    const out = await retainText(text, teachContext.trim() || undefined);
    teaching = false;
    if (out.ok) {
      teachText = "";
      message = $t("assist.memory.engine.taught", {
        created: out.value.created.length,
        again: out.value.reinforced.length,
      }) as string;
      await refresh();
    } else errorKey = out.errorKey;
  }

  onMount(() => {
    void refresh();
  });
</script>

<section class="block engine" aria-label={$t("assist.memory.engine.title")}>
  <header class="head">
    <div>
      <h3 class="block-title">{$t("assist.memory.engine.title")}</h3>
      <p class="dim">{$t("assist.memory.engine.lede")}</p>
    </div>
  </header>

  {#if stats}
    <p class="counts">
      {$t("assist.memory.engine.counts", {
        active: stats.active,
        vectors: stats.with_vector,
        entities: stats.entities,
        notes: stats.notes,
      })}
      {#if stats.retains_today > 0}
        · {$t("assist.memory.engine.today", { facts: stats.facts_today })}
      {/if}
    </p>
    {#if stats.last_retain_error}
      <p class="notice error" role="status">{$t("assist.memory.engine.retain_failed")}</p>
    {/if}
    <div class="toggles">
      <label class="check">
        <input type="checkbox" checked={stats.auto_retain} onchange={(e) => toggle("auto_retain", e.currentTarget.checked)} />
        <span>{$t("assist.memory.engine.auto_retain")}</span>
      </label>
      <label class="check">
        <input type="checkbox" checked={stats.consolidate} onchange={(e) => toggle("consolidate", e.currentTarget.checked)} />
        <span>{$t("assist.memory.engine.consolidate")}</span>
      </label>
      {#if !stats.semantic_ready || stats.with_vector < stats.active}
        <button type="button" class="button small" disabled={preparing} onclick={prepare}>
          {preparing ? $t("assist.memory.engine.preparing") : $t("assist.memory.engine.prepare")}
        </button>
      {/if}
    </div>
  {/if}

  <form class="search" role="search" onsubmit={(e) => { e.preventDefault(); void search(); }}>
    <input class="input" type="search" bind:value={query} placeholder={$t("assist.memory.engine.search_placeholder")} aria-label={$t("assist.memory.engine.search_label")} />
    <button type="submit" class="button" disabled={searching || !query.trim()}>{$t("assist.memory.engine.search")}</button>
  </form>

  {#if result}
    <div class="probe">
      {#if result.window}
        <span class="chip">{$t("assist.memory.engine.window", { from: day(result.window[0]), to: day(result.window[1] - 1) })}</span>
      {/if}
      {#each result.entities as name}<span class="chip">{name}</span>{/each}
      {#if !result.semantic}<span class="chip muted">{$t("assist.memory.engine.no_semantic")}</span>{/if}
    </div>
    {#if result.hits.length === 0}
      <p class="empty">{$t("assist.memory.engine.no_hits")}</p>
    {:else}
      <ol class="hits">
        {#each result.hits as hit (hit.id)}
          <li class="hit">
            <p class="content">{hit.content}</p>
            <p class="meta">
              {#each hit.why.arms as arm}<span class="arm arm-{arm}">{$t(`assist.memory.engine.arm.${arm}`)}</span>{/each}
              {#if hit.why.occurred_start}<span>{$t("assist.memory.engine.happened", { date: day(hit.why.occurred_start) })}</span>{/if}
              {#if hit.why.proofs > 1}<span>{$t("assist.memory.engine.proofs", { count: hit.why.proofs })}</span>{/if}
              <span class="dim">{hit.source_line}</span>
            </p>
          </li>
        {/each}
      </ol>
    {/if}
  {/if}

  {#if entities.length}
    <h4 class="sub">{$t("assist.memory.engine.entities")}</h4>
    <ul class="entities">
      {#each entities as e (e.id)}
        <li class="entity">
          <p class="name">{e.name} <span class="dim">· {e.kind} · {$t("assist.memory.engine.mentions", { count: e.mentions })}</span></p>
          {#if e.note}<p class="note">{e.note}</p>{/if}
        </li>
      {/each}
    </ul>
  {/if}

  <details class="teach">
    <summary>{$t("assist.memory.engine.teach")}</summary>
    <form class="teach-form" onsubmit={(e) => { e.preventDefault(); void teach(); }}>
      <label class="field">
        <span class="field-label">{$t("assist.memory.engine.teach_label")}</span>
        <textarea class="input" rows="5" bind:value={teachText} placeholder={$t("assist.memory.engine.teach_placeholder")}></textarea>
      </label>
      <label class="field">
        <span class="field-label">{$t("assist.memory.engine.teach_context")}</span>
        <input class="input" bind:value={teachContext} />
      </label>
      <div><button type="submit" class="button primary" disabled={teaching || !teachText.trim()}>{teaching ? $t("assist.memory.engine.teaching") : $t("assist.memory.engine.teach_go")}</button></div>
    </form>
  </details>

  {#if message}<p class="notice" role="status">{message}</p>{/if}
  {#if errorKey}<p class="notice error" role="alert">{$t(errorKey)}</p>{/if}
</section>

<style>
  .engine { display: grid; gap: 10px; }
  .head { display: flex; justify-content: space-between; gap: 12px; }
  .dim { margin: 2px 0 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; max-width: 72ch; }
  .counts { margin: 0; font-size: 13px; color: var(--text); }
  .toggles { display: flex; flex-wrap: wrap; gap: 12px; align-items: center; }
  .check { display: inline-flex; gap: 6px; align-items: center; font-size: 13px; }
  .search { display: flex; gap: 8px; }
  .search .input { flex: 1; min-width: 0; }
  .probe { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { font-size: 12px; padding: 2px 8px; border-radius: 999px; background: var(--surface-2, var(--separator)); color: var(--text); }
  .chip.muted { color: var(--text-muted); }
  .hits, .entities { margin: 0; padding: 0; list-style: none; display: grid; gap: 8px; }
  .hit, .entity { padding: 8px 0; border-bottom: 1px solid var(--separator); }
  .content, .name { margin: 0; font-size: 14px; line-height: 1.5; }
  .meta { margin: 4px 0 0; display: flex; flex-wrap: wrap; gap: 6px; font-size: 12px; color: var(--text-muted); }
  .arm { padding: 0 6px; border-radius: var(--radius-sm); border: 1px solid var(--separator); color: var(--text); }
  .note { margin: 4px 0 0; white-space: pre-wrap; font-size: 13px; line-height: 1.5; color: var(--text); }
  .sub { margin: 8px 0 0; font-size: 13px; font-weight: 600; }
  .empty { margin: 0; color: var(--text-muted); font-size: 13px; }
  .teach summary { cursor: pointer; font-size: 13px; color: var(--accent-text); border-radius: var(--radius-sm); }
  .teach-form { display: grid; gap: 8px; padding-top: 8px; }
  .teach-form textarea { width: 100%; resize: vertical; }
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  .teach summary:focus-visible, .button:focus-visible, input:focus-visible, textarea:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  @media (max-width: 560px) { .search { flex-direction: column; } }
</style>
