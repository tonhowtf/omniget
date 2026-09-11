<script lang="ts">
  /** Goodreads: dispara o "Export Library" oficial, espera e enriquece por prateleira. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, fmtSecs, onToolProgress, openUrl, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Entry = { date: string; title: string; year: number | null; creator: string; rating: number | null; review: string; list: string; url: string };
  type ShelfCount = { shelf: string; books: number; reviews: number; rated: number };
  type Result = { used_session: boolean; source: string; entries: number; shelves: ShelfCount[]; reviews: number; enriched: number; waited_secs: number; requests: number; files: string[]; dest: string; sample: Entry[] };

  let dest = $state("");
  let csvPath = $state("");
  let formats = $state<string[]>(["json", "csv", "md"]);
  let byShelf = $state(true);
  let force = $state(false);
  let enrich = $state("");
  let waitSecs = $state(300);
  let logged = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const canRun = $derived(!!dest && (logged || !!csvPath) && formats.length > 0);

  function toggle(list: string[], v: string): string[] {
    return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
  }

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "gr-export") progress = p;
    });
    try {
      logged = await invoke<boolean>("tool_lists_session", { domain: "goodreads", accountSlug: null });
    } catch {
      logged = false;
    }
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!canRun || busy) return;
    busy = true;
    result = null;
    progress = null;
    const shelves = enrich.split(",").map((s) => s.trim()).filter(Boolean);
    try {
      result = await invoke<Result>("tool_gr_export", {
        opts: { dest, csv_path: csvPath || null, formats, by_shelf: byShelf, force, enrich_shelves: shelves, max_pages: 20, wait_secs: waitSecs, delay_ms: 2000 },
      });
      showToast("success", `${result.entries} ${$t("tools.grexport.books")}`);
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
          <div class="group-row-title">{$t("tools.lists.session")}</div>
          <div class="group-row-sub">{logged ? $t("tools.grexport.session_on") : $t("tools.grexport.session_off")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <span class="tag" class:tag-success={logged}>{logged ? $t("tools.lists.logged_in") : $t("tools.lists.anonymous")}</span>
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl("https://www.goodreads.com/review/import")}>{$t("tools.lists.open_site")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.grexport.csv")}</div>
          <div class="group-row-sub mono">{csvPath ? baseName(csvPath) : $t("tools.grexport.csv_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if csvPath}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (csvPath = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile([{ name: "CSV", extensions: ["csv"] }]); if (f) csvPath = f; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lists.dest")}</div><div class="group-row-sub mono">{dest || "—"}</div></div>
        <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dest = d; }}>{$t("tools.common.choose")}</button></div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lists.formats")}</div></div>
        <div class="group-row-trailing btn-row">
          {#each ["json", "csv", "md"] as f (f)}
            <button class="btn btn-sm" class:btn-primary={formats.includes(f)} class:btn-ghost={!formats.includes(f)} type="button" onclick={() => (formats = toggle(formats, f))}>{f.toUpperCase()}</button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.grexport.by_shelf")}</div><div class="group-row-sub">{$t("tools.grexport.by_shelf_note")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={byShelf} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.grexport.enrich")}</div><div class="group-row-sub">{$t("tools.grexport.enrich_note")}</div></div>
        <div class="group-row-trailing"><input class="input" type="text" bind:value={enrich} placeholder="read, favorites" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.grexport.wait")}</div><div class="group-row-sub">{$t("tools.grexport.wait_note")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="30" max="3600" step="30" bind:value={waitSecs} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.grexport.force")}</div><div class="group-row-sub">{$t("tools.grexport.force_note")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={force} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !canRun} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.grexport.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.entries} {$t("tools.grexport.books")}</div>
            <div class="group-row-sub">
              {result.reviews} {$t("tools.grexport.reviews")}
              {#if result.enriched}· {result.enriched} {$t("tools.grexport.enriched")}{/if}
              {#if result.waited_secs}· {fmtSecs(result.waited_secs)} {$t("tools.grexport.waited")}{/if}
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>↗</button></div>
        </div>
        {#each result.shelves.slice(0, 30) as s (s.shelf)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{s.shelf}</div></div>
            <div class="group-row-trailing">{s.books} · {s.reviews} {$t("tools.grexport.reviews")}</div>
          </div>
        {/each}
      </div>
    </section>
    <section>
      <div class="group">
        {#each result.sample as e (e.title + e.date)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{e.title}{#if e.year} <span class="dim">({e.year})</span>{/if}</div>
              <div class="group-row-sub">{e.creator} · {e.date || "—"} · {e.list}{#if e.rating} · {e.rating.toFixed(1)}/10{/if}</div>
            </div>
            {#if e.url}<div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(e.url)}>↗</button></div>{/if}
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
