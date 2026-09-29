<script lang="ts">
  /**
   * Work (roadmap 3.9, stage 2): one list of everything the agents do for
   * you — background tasks, loops, missions, the Central's threads and chat
   * turns that wait for an answer — in five sections, each row linking to
   * the page that owns its detail. Live from the same stores as the header
   * badge (`$lib/stores/llm-work-store`), no timer.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getWorkErrors, getWorkItems, initWorkStore, retryWork } from "$lib/stores/llm-work-store.svelte";
  import { groupBySection, pillTone, type WorkItem } from "$lib/llm/work";
  import { shortTime } from "$lib/stores/llm-jobs-store.svelte";

  let items = $derived(getWorkItems());
  let groups = $derived(groupBySection(items));
  let errors = $derived(getWorkErrors());

  onMount(() => initWorkStore());

  function detailText(item: WorkItem): string {
    if (!item.detail) return "";
    if (item.pill === "approval") return $t("llm.work.asks_for", { tool: item.detail }) as string;
    return item.detail;
  }
</script>

<svelte:head><title>{$t("llm.workspace.work")}</title></svelte:head>

<div class="page page-wide work-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.workspace.work")}</h1>
      <p class="page-lede">{$t("llm.work.lede")}</p>
    </div>
  </header>

  {#if errors.length}
    <div class="error-banner" role="alert">
      <span>{$t("llm.work.partial", { error: errors.join(" · ") })}</span>
      <button type="button" class="button" onclick={retryWork}>{$t("llm.surface.retry")}</button>
    </div>
  {/if}

  {#each groups as group (group.section)}
    {#if group.items.length > 0 || group.section === "needs_you"}
      <section class="stack" aria-labelledby="work-{group.section}">
        <h2 class="section-header-title" id="work-{group.section}">
          {$t(`llm.work.section.${group.section}`)} <span class="count">{group.items.length}</span>
        </h2>
        {#if group.items.length === 0}
          <p class="hint">{$t("llm.work.nothing_waits")}</p>
        {:else}
          <div class="rows">
            {#each group.items as item (item.key)}
              <a class="row" href={item.href} data-work-key={item.key}>
                <span class="pill {pillTone(item.pill)}">{$t(`llm.work.pill.${item.pill}`)}</span>
                <span class="tag">{$t(`llm.work.source.${item.source}`)}</span>
                <span class="line">{item.title || item.id}</span>
                {#if detailText(item)}<span class="detail">{detailText(item)}</span>{/if}
                <span class="time">{shortTime(item.at)}</span>
              </a>
            {/each}
          </div>
        {/if}
      </section>
    {/if}
  {/each}

  {#if items.length === 0 && errors.length === 0}
    <p class="hint">{$t("llm.work.empty")} <a href="/llm/jobs">{$t("llm.surface.create")}</a></p>
  {/if}
</div>

<style>
  .work-page { flex: 1; min-height: 0; overflow-y: auto; }
  .stack { display: flex; flex-direction: column; gap: var(--space-3); margin-bottom: var(--space-5); min-width: 0; }
  .count { margin-left: 6px; font-size: var(--text-sm); color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
  .error-banner { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); flex-wrap: wrap; margin-bottom: var(--space-4); padding: var(--space-2) var(--space-3); border-radius: var(--radius-md); color: var(--red); background: color-mix(in srgb, var(--red) 10%, transparent); font-size: var(--text-sm); overflow-wrap: anywhere; }
  .rows { display: flex; flex-direction: column; border: var(--hairline) solid var(--separator); border-radius: var(--radius-md); overflow: hidden; }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-3); padding: var(--space-2) var(--space-3); font-size: var(--text-sm); color: inherit; text-decoration: none; min-width: 0; }
  .row + .row { border-top: var(--hairline) solid var(--separator); }
  .row:hover { background: var(--fill-1); }
  .row:focus-visible { outline: var(--focus-ring); outline-offset: -2px; }
  .line { flex: 1; min-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .detail { flex-shrink: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--text-xs); color: var(--text-muted); }
  .time { margin-left: auto; font-size: var(--text-xs); color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .tag { flex-shrink: 0; padding: 0 6px; border-radius: 999px; font-size: var(--text-xs); color: var(--text-muted); background: var(--fill-2); }
  .pill { display: inline-flex; align-items: center; flex-shrink: 0; padding: 1px 8px; border-radius: 999px; font-size: var(--text-xs); font-weight: 600; white-space: nowrap; color: var(--text-muted); background: var(--fill-2); }
  .pill.blue { color: var(--blue); background: color-mix(in srgb, var(--blue) 14%, transparent); }
  .pill.orange { color: var(--orange); background: color-mix(in srgb, var(--orange) 16%, transparent); }
  .pill.green { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .pill.red { color: var(--red); background: color-mix(in srgb, var(--red) 14%, transparent); }
</style>
