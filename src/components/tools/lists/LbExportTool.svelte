<script lang="ts">
  /** Letterboxd: baixa o ZIP do export oficial com a sessão e normaliza tudo. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, openUrl, pct, pickDir, pickFile, reveal, type ToolProgress } from "$lib/tools/rt";

  type Entry = { date: string; kind: string; title: string; year: number | null; rating: number | null; review: string; list: string; url: string; tmdb_id: number | null };
  type PartCount = { part: string; entries: number };
  type Result = { used_session: boolean; source: string; entries: number; by_part: PartCount[]; tmdb_found: number; requests: number; files: string[]; dest: string; sample: Entry[] };

  const PARTS = ["diary", "ratings", "reviews", "watched", "watchlist", "lists", "likes"];

  let dest = $state("");
  let zipPath = $state("");
  let parts = $state<string[]>(["diary", "ratings", "reviews", "watchlist", "lists"]);
  let formats = $state<string[]>(["json", "csv", "md"]);
  let tmdb = $state(false);
  let tmdbLimit = $state(120);
  let keepZip = $state(false);
  let logged = $state(false);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const canRun = $derived(!!dest && (logged || !!zipPath) && parts.length > 0 && formats.length > 0);

  function toggle(list: string[], v: string): string[] {
    return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
  }

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "lb-export") progress = p;
    });
    try {
      logged = await invoke<boolean>("tool_lists_session", { domain: "letterboxd", accountSlug: null });
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
    try {
      result = await invoke<Result>("tool_lb_export", {
        opts: { dest, zip_path: zipPath || null, parts, formats, tmdb, tmdb_limit: tmdbLimit, delay_ms: 1200, keep_zip: keepZip },
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
          <div class="group-row-title">{$t("tools.lists.session")}</div>
          <div class="group-row-sub">{logged ? $t("tools.lbexport.session_on") : $t("tools.lbexport.session_off")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          <span class="tag" class:tag-success={logged}>{logged ? $t("tools.lists.logged_in") : $t("tools.lists.anonymous")}</span>
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl("https://letterboxd.com/settings/data/")}>{$t("tools.lists.open_site")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.lbexport.zip")}</div>
          <div class="group-row-sub mono">{zipPath ? baseName(zipPath) : $t("tools.lbexport.zip_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if zipPath}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (zipPath = "")}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFile([{ name: "ZIP", extensions: ["zip"] }]); if (f) zipPath = f; }}>{$t("tools.common.choose")}</button>
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
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lbexport.parts")}</div></div>
        <div class="group-row-trailing btn-row wrap">
          {#each PARTS as p (p)}
            <button class="btn btn-sm" class:btn-primary={parts.includes(p)} class:btn-ghost={!parts.includes(p)} type="button" onclick={() => (parts = toggle(parts, p))}>{$t(`tools.lbexport.part_${p}`)}</button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lists.formats")}</div></div>
        <div class="group-row-trailing btn-row">
          {#each ["json", "csv", "md"] as f (f)}
            <button class="btn btn-sm" class:btn-primary={formats.includes(f)} class:btn-ghost={!formats.includes(f)} type="button" onclick={() => (formats = toggle(formats, f))}>{f.toUpperCase()}</button>
          {/each}
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lbexport.tmdb")}</div><div class="group-row-sub">{$t("tools.lbexport.tmdb_note")}</div></div>
        <div class="group-row-trailing btn-row">
          {#if tmdb}<input class="input" type="number" min="10" max="2000" step="10" bind:value={tmdbLimit} style:width="6em" />{/if}
          <input type="checkbox" bind:checked={tmdb} />
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lbexport.keep_zip")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={keepZip} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing"><button class="btn btn-primary" type="button" disabled={busy || !canRun} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.lbexport.run")}</button></div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.entries} {$t("tools.lists.entries")}</div>
            <div class="group-row-sub">
              <span class="tag">{result.used_session ? $t("tools.lists.logged_in") : $t("tools.lists.anonymous")}</span>
              {#if result.tmdb_found}· {result.tmdb_found} {$t("tools.lbexport.tmdb_found")}{/if}
              {#if result.requests}· {result.requests} {$t("tools.lists.requests")}{/if}
            </div>
          </div>
          <div class="group-row-trailing"><button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(result?.dest ?? "")}>↗</button></div>
        </div>
        {#each result.by_part as p (p.part)}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{p.part}</div></div>
            <div class="group-row-trailing">{p.entries}</div>
          </div>
        {/each}
      </div>
    </section>
    <section>
      <div class="group">
        {#each result.sample as e (e.title + e.date + e.list)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-title">{e.title}{#if e.year} <span class="dim">({e.year})</span>{/if}</div>
              <div class="group-row-sub">{e.date || "—"} · {e.list}{#if e.rating} · {e.rating.toFixed(1)}/10{/if}</div>
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
  .wrap { flex-wrap: wrap; justify-content: flex-end; }
</style>
