<script lang="ts">
  /** Tabela filtravel do Connections.csv, com duplicados e exportacao do filtrado. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Count = { name: string; count: number };
  type Connection = { name: string; url: string; email: string; company: string; position: string; connected_on: string; date: string; month: string };
  type DupGroup = { reason: string; items: Connection[] };
  type Result = {
    total: number; matched: number; returned: number; without_email: number; first: string; last: string;
    items: Connection[]; companies: Count[]; positions: Count[]; by_year: { period: string; count: number }[];
    duplicates: DupGroup[]; exports: string[];
  };

  let path = $state("");
  let query = $state("");
  let company = $state("");
  let from = $state("");
  let to = $state("");
  let sort = $state("date");
  let onlyWithoutEmail = $state(false);
  let onlyDuplicates = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ZIP = [{ name: "Zip", extensions: ["zip"] }];

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "li-connections") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  function opts(exportKind?: string) {
    return {
      path,
      query: query || null,
      company: company || null,
      from: from || null,
      to: to || null,
      sort,
      only_without_email: onlyWithoutEmail,
      only_duplicates: onlyDuplicates,
      limit: 800,
      export: exportKind ?? null,
    };
  }

  function revealLast() {
    const e = result?.exports.at(-1);
    if (e) reveal(e);
  }

  async function run(exportKind?: string) {
    if (!path || busy) return;
    busy = true;
    progress = null;
    try {
      result = await invoke<Result>("tool_li_connections", { opts: opts(exportKind) });
      if (exportKind && result.exports.length) showToast("success", result.exports[0]);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.liconn.source")}</div>
          <div class="group-row-sub mono">{path || $t("tools.liconn.source_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile(ZIP); if (f) path = f; }}>{$t("tools.liconn.pick_zip")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) path = d; }}>{$t("tools.liconn.pick_folder")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.liconn.search")}</div></div>
        <div class="group-row-trailing"><input class="input" type="search" bind:value={query} placeholder={$t("tools.liconn.search_ph")} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.liconn.company")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" bind:value={company} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.liconn.period")}</div><div class="group-row-sub">{$t("tools.liconn.period_sub")}</div></div>
        <div class="group-row-trailing btn-row">
          <input class="input" type="text" bind:value={from} placeholder="2019-01" style:width="7em" />
          <input class="input" type="text" bind:value={to} placeholder="2026-12" style:width="7em" />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.liconn.sort")}</div></div>
        <div class="group-row-trailing">
          <select class="input" bind:value={sort}>
            <option value="date">{$t("tools.liconn.sort_date")}</option>
            <option value="name">{$t("tools.liconn.sort_name")}</option>
            <option value="company">{$t("tools.liconn.sort_company")}</option>
          </select>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.liconn.filters")}</div></div>
        <div class="group-row-trailing btn-row">
          <label class="check"><input type="checkbox" bind:checked={onlyWithoutEmail} />{$t("tools.liconn.only_no_email")}</label>
          <label class="check"><input type="checkbox" bind:checked={onlyDuplicates} />{$t("tools.liconn.only_dupes")}</label>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">{#if busy}<div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}</div>
        <div class="group-row-trailing btn-row">
          {#if result}
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => run("csv")}>CSV</button>
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy} onclick={() => run("json")}>JSON</button>
          {/if}
          <button class="btn btn-primary" type="button" disabled={busy || !path} onclick={() => run()}>{busy ? $t("tools.common.working") : $t("tools.liconn.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.matched} / {result.total}</div>
            <div class="group-row-sub">{result.first} → {result.last} · {result.without_email} {$t("tools.liconn.without_email")} · {result.duplicates.length} {$t("tools.liconn.dupes")}</div>
          </div>
          <div class="group-row-trailing btn-row">
            {#if result.exports.length}<button class="btn btn-ghost btn-sm" type="button" onclick={revealLast}>{$t("tools.common.reveal")}</button>{/if}
          </div>
        </div>
      </div>
    </section>

    <section>
      <div class="group">
        {#each result.items.slice(0, 300) as c (c.url + c.name + c.connected_on)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{c.name}{#if !c.email}<span class="tag">{$t("tools.liconn.no_email")}</span>{/if}</div>
              <div class="group-row-sub">{c.position}{#if c.company} · {c.company}{/if}</div>
              <div class="group-row-sub mono">{c.date || c.connected_on}{#if c.email} · {c.email}{/if}</div>
            </div>
            <div class="group-row-trailing">{#if c.url}<button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(c.url)}>↗</button>{/if}</div>
          </div>
        {/each}
      </div>
    </section>

    <section>
      <div class="cols">
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.liconn.by_company")}</div></div></div>
          {#each result.companies.slice(0, 15) as c (c.name)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{c.name}</div></div><div class="group-row-trailing">{c.count}</div></div>
          {/each}
        </div>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.liconn.by_position")}</div></div></div>
          {#each result.positions.slice(0, 15) as c (c.name)}
            <div class="group-row"><div class="group-row-content"><div class="group-row-sub">{c.name}</div></div><div class="group-row-trailing">{c.count}</div></div>
          {/each}
        </div>
      </div>
    </section>

    {#if result.duplicates.length}
      <section>
        <div class="group">
          <div class="group-row"><div class="group-row-content"><div class="group-row-title">{$t("tools.liconn.dupes")}</div><div class="group-row-sub">{result.duplicates.length}</div></div></div>
          {#each result.duplicates.slice(0, 40) as g, i (i)}
            <div class="group-row">
              <div class="group-row-content">
                <div class="group-row-title">{g.items[0].name} <span class="tag">{$t(`tools.liconn.reason_${g.reason}`)}</span></div>
                <div class="group-row-sub mono">{g.items.map((x) => x.connected_on).join(" · ")}</div>
              </div>
              <div class="group-row-trailing">{g.items.length}</div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .check { display: inline-flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); }
  .cols { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: var(--space-4); }
</style>
