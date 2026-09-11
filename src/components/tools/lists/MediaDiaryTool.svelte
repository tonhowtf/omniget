<script lang="ts">
  /** Diário único: junta os exports dos três serviços mais o do Spotify. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pct, pickDir, pickFiles, reveal, type ToolProgress } from "$lib/tools/rt";

  type Entry = { date: string; kind: string; title: string; year: number | null; creator: string; rating: number | null; source: string; url: string; times: number };
  type SourceCount = { source: string; file: string; entries: number };
  type YearCount = { year: string; total: number; filmes: number; series: number; livros: number; musicas: number };
  type Result = { entries: number; merged: number; sources: SourceCount[]; years: YearCount[]; first_date: string; last_date: string; files: string[]; dest: string; sample: Entry[] };

  let inputs = $state<string[]>([]);
  let spotify = $state<string[]>([]);
  let dest = $state("");
  let formats = $state<string[]>(["json", "csv", "md"]);
  let dedup = $state(true);
  let onlyDated = $state(false);
  let minPlays = $state(3);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const canRun = $derived(!!dest && (inputs.length > 0 || spotify.length > 0) && formats.length > 0);

  function toggle(list: string[], v: string): string[] {
    return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
  }

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "media-diary") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!canRun || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_media_diary", {
        opts: { inputs, spotify, min_ms: 30000, min_plays: minPlays, dest, formats, dedup, only_dated: onlyDated },
      });
      showToast("success", `${result.entries} ${$t("tools.lists.entries")}`);
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
          <div class="group-row-title">{$t("tools.diary.inputs")}</div>
          <div class="group-row-sub mono">{inputs.map(baseName).join(" · ") || $t("tools.diary.inputs_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if inputs.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (inputs = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) inputs = [...inputs, d]; }}>{$t("tools.diary.add_dir")}</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFiles([{ name: "Export", extensions: ["json", "csv"] }]); if (f.length) inputs = [...inputs, ...f]; }}>{$t("tools.diary.add_files")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.diary.spotify")}</div>
          <div class="group-row-sub mono">{spotify.map(baseName).join(" · ") || $t("tools.diary.spotify_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if spotify.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (spotify = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) spotify = [...spotify, d]; }}>{$t("tools.common.add")}</button>
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
        <div class="group-row-content"><div class="group-row-title">{$t("tools.diary.dedup")}</div><div class="group-row-sub">{$t("tools.diary.dedup_note")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={dedup} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.diary.only_dated")}</div><div class="group-row-sub">{$t("tools.diary.only_dated_note")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={onlyDated} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.diary.min_plays")}</div><div class="group-row-sub">{$t("tools.diary.min_plays_note")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="1" max="100" step="1" bind:value={minPlays} style:width="6em" /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ? baseName(progress.message) : "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !canRun} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.diary.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.entries} {$t("tools.lists.entries")}</div>
            <div class="group-row-sub">{result.first_date || "—"} — {result.last_date || "—"}{#if result.merged} · {result.merged} {$t("tools.diary.merged")}{/if}</div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>↗</button></div>
        </div>
        {#each result.sources as s (s.file)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{s.source}</div><div class="group-row-sub mono">{baseName(s.file)}</div></div>
            <div class="group-row-trailing">{s.entries}</div>
          </div>
        {/each}
      </div>
    </section>
    <section>
      <div class="group">
        {#each result.years as y (y.year)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-title">{y.year}</div>
              <div class="group-row-sub">{y.filmes} {$t("tools.diary.movies")} · {y.series} {$t("tools.diary.shows")} · {y.livros} {$t("tools.diary.books")} · {y.musicas} {$t("tools.diary.songs")}</div>
            </div>
            <div class="group-row-trailing">{y.total}</div>
          </div>
        {/each}
      </div>
    </section>
    <section>
      <div class="group">
        {#each result.sample as e (e.title + e.date + e.source)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{e.title}{#if e.year} <span class="dim">({e.year})</span>{/if}</div>
              <div class="group-row-sub">{e.date || "—"} · {$t(`tools.diary.kind_${e.kind}`)} · {e.source}{#if e.rating} · {e.rating.toFixed(1)}/10{/if}{#if e.times > 1} · {e.times}×{/if}</div>
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
