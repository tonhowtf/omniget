<script lang="ts">
  // Import subscriptions/history from NewPipe, FreeTube, Google Takeout, OPML
  // or a list of links (format detected by content in the backend), and
  // export the local subscriptions as OPML or NewPipe JSON.
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { back, canGoBack, go, type Route } from "$lib/tube/ui.svelte";
  import type { ImportReport } from "$lib/tube/types";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TIcon from "../TIcon.svelte";

  let { route }: { route: Extract<Route, { name: "import" }> } = $props();

  const SOURCES = [
    { name: "NewPipe", ext: ".json · .zip", key: "tube.import.src_newpipe" },
    { name: "FreeTube", ext: ".db", key: "tube.import.src_freetube" },
    { name: "Google Takeout", ext: ".zip · .csv · .json", key: "tube.import.src_takeout" },
    { name: "OPML", ext: ".opml · .xml", key: "tube.import.src_opml" },
  ];
  const EXTS = ["json", "zip", "db", "csv", "opml", "xml", "txt"];

  let dropping = $state(false);
  let busy = $state(false);
  let fileName = $state("");
  let report = $state<ImportReport | null>(null);
  let error = $state<string | null>(null);
  let exporting = $state<"opml" | "newpipe" | null>(null);

  let total = $derived(report ? report.subs_added + report.history_added + report.playlists_added : 0);

  async function importPath(path: string) {
    if (busy) return;
    busy = true;
    error = null;
    report = null;
    fileName = path.split(/[\\/]/).pop() ?? path;
    try {
      report = await api.importFile(path);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function choose() {
    const sel = await open({
      multiple: false,
      title: $t("tube.import.choose_title") as string,
      filters: [{ name: $t("tube.import.filter_name") as string, extensions: EXTS }],
    }).catch(() => null);
    if (typeof sel === "string") void importPath(sel);
  }

  async function exportAs(format: "opml" | "newpipe") {
    const path = await save({
      title: $t("tube.import.export_title") as string,
      defaultPath: format === "opml" ? "omnitube-subscriptions.opml" : "omnitube-newpipe-subscriptions.json",
      filters: [format === "opml" ? { name: "OPML", extensions: ["opml", "xml"] } : { name: "NewPipe JSON", extensions: ["json"] }],
    }).catch(() => null);
    if (!path) return;
    exporting = format;
    try {
      await api.exportSubs(path, format);
      showToast("success", $t("tube.import.exported") as string);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      exporting = null;
    }
  }

  function reset() {
    report = null;
    error = null;
    fileName = "";
  }

  onMount(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          const p = event.payload as { type: string; paths?: string[] };
          if (p.type === "enter" || p.type === "over") dropping = true;
          else if (p.type === "leave") dropping = false;
          else if (p.type === "drop") {
            dropping = false;
            const path = p.paths?.[0];
            if (path) void importPath(path);
          }
        })
        .then((fn) => {
          if (disposed) fn();
          else unlisten = fn;
        })
        .catch(() => {});
    } catch {
      /* outside Tauri */
    }
    return () => {
      disposed = true;
      unlisten?.();
    };
  });
</script>

<div class="iv" data-agent-readiness={busy ? "loading" : error ? "error" : "ready"} data-agent-surface="omnitube-import" data-route={route.name}>
  <header class="iv-head">
    {#if canGoBack()}
      <button class="yt-btn yt-btn--icon" onclick={() => back()} aria-label={$t("tube.common.back") as string}><TIcon name="back" size={22} /></button>
    {/if}
    <div>
      <h1>{$t("tube.import.title")}</h1>
      <p>{$t("tube.import.subtitle")}</p>
    </div>
  </header>

  {#if report}
    <section class="iv-card iv-report" aria-live="polite">
      <div class="iv-report-top">
        <span class="iv-ok"><TIcon name={report.errors.length && !total ? "alert" : "check"} size={28} /></span>
        <div>
          <h2>{total ? $t("tube.import.done_title") : $t("tube.import.nothing_title")}</h2>
          <p>{$t("tube.import.done_from", { name: fileName, format: report.format })}</p>
        </div>
      </div>
      <div class="iv-stats">
        <div class="iv-stat"><strong>{report.subs_added}</strong><span>{$t("tube.import.stat_subs")}</span></div>
        <div class="iv-stat"><strong>{report.subs_skipped}</strong><span>{$t("tube.import.stat_skipped")}</span></div>
        <div class="iv-stat"><strong>{report.history_added}</strong><span>{$t("tube.import.stat_history")}</span></div>
        <div class="iv-stat"><strong>{report.playlists_added}</strong><span>{$t("tube.import.stat_playlists")}</span></div>
      </div>
      {#if report.errors.length}
        <details class="iv-errors">
          <summary>{$t("tube.import.errors", { count: report.errors.length })}</summary>
          <ul>
            {#each report.errors.slice(0, 50) as e, i (i)}<li>{e}</li>{/each}
          </ul>
        </details>
      {/if}
      <div class="iv-actions">
        {#if report.subs_added}
          <button class="yt-btn yt-btn--inverted" onclick={() => go({ name: "subscriptions" })}><TIcon name="subscriptions" size={18} />{$t("tube.import.open_subs")}</button>
        {/if}
        <button class="yt-btn" onclick={reset}>{$t("tube.import.another")}</button>
      </div>
    </section>
  {:else}
    <button class="iv-drop" class:dropping class:busy onclick={choose} disabled={busy} aria-describedby="iv-drop-hint">
      <img src="/icons/tube/import-128.webp" alt="" width="112" height="112" />
      {#if busy}
        <strong>{$t("tube.import.importing", { name: fileName })}</strong>
        <span class="iv-progress" aria-hidden="true"><i></i></span>
      {:else}
        <strong>{dropping ? $t("tube.import.drop_now") : $t("tube.import.drop")}</strong>
        <span id="iv-drop-hint">{$t("tube.import.drop_hint")}</span>
        <span class="yt-btn yt-btn--inverted iv-choose"><TIcon name="upload" size={18} />{$t("tube.import.choose")}</span>
      {/if}
    </button>
    {#if error}
      <p class="iv-error" role="alert"><TIcon name="alert" size={16} />{$t("tube.import.failed", { reason: error })}</p>
    {/if}
  {/if}

  <section class="iv-card">
    <h2 class="iv-h2">{$t("tube.import.sources")}</h2>
    <ul class="iv-sources">
      {#each SOURCES as s (s.name)}
        <li>
          <span class="iv-src-name">{s.name}<span class="iv-ext">{s.ext}</span></span>
          <span class="iv-src-desc">{$t(s.key)}</span>
        </li>
      {/each}
      <li>
        <span class="iv-src-name">{$t("tube.import.src_links_name")}<span class="iv-ext">.txt</span></span>
        <span class="iv-src-desc">{$t("tube.import.src_links")}</span>
      </li>
    </ul>
  </section>

  <section class="iv-card">
    <h2 class="iv-h2">{$t("tube.import.export")}</h2>
    <p class="iv-muted">{$t("tube.import.export_desc")}</p>
    <div class="iv-actions">
      <button class="yt-btn" onclick={() => exportAs("opml")} disabled={!!exporting}><TIcon name={exporting === "opml" ? "loader" : "download"} size={18} />OPML</button>
      <button class="yt-btn" onclick={() => exportAs("newpipe")} disabled={!!exporting}><TIcon name={exporting === "newpipe" ? "loader" : "download"} size={18} />NewPipe JSON</button>
    </div>
  </section>
</div>

<style>
  .iv {
    max-width: 760px; margin: 0 auto; padding: 24px 24px 64px;
    display: flex; flex-direction: column; gap: 20px;
    color: var(--yt-text); background: var(--yt-bg); min-height: 100%;
    box-sizing: border-box;
  }
  .iv-head { display: flex; align-items: center; gap: 8px; }
  .iv-head h1 { margin: 0; font: var(--yt-type-headline-m); }
  .iv-head p { margin: 2px 0 0; font: var(--yt-type-meta); color: var(--yt-text-2); }

  .iv-drop {
    display: flex; flex-direction: column; align-items: center; gap: 8px;
    padding: 36px 24px; border-radius: 16px;
    border: 2px dashed var(--yt-outline); background: var(--yt-bg-b);
    color: var(--yt-text); text-align: center; cursor: pointer;
    transition: border-color var(--yt-dur-base), background var(--yt-dur-base), transform var(--yt-dur-base) var(--yt-ease-standard);
  }
  .iv-drop:hover:not(:disabled) { border-color: var(--yt-text-2); }
  .iv-drop.dropping { border-color: var(--yt-accent); background: color-mix(in srgb, var(--yt-accent) 8%, var(--yt-bg-b)); transform: scale(1.01); }
  .iv-drop:disabled { cursor: progress; }
  .iv-drop:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 3px; }
  .iv-drop img { width: 112px; height: 112px; filter: drop-shadow(0 8px 24px rgba(0, 0, 0, .3)); }
  .iv-drop strong { font: 500 18px/24px var(--yt-font); }
  .iv-drop span { font: var(--yt-type-meta); color: var(--yt-text-2); }
  .iv-drop .iv-choose { margin-top: 8px; color: var(--yt-on-inverted); pointer-events: none; }
  .iv-progress { position: relative; width: 240px; height: 4px; border-radius: 999px; background: var(--yt-tonal); overflow: hidden; margin-top: 8px; }
  .iv-progress i { position: absolute; top: 0; bottom: 0; width: 40%; border-radius: inherit; background: var(--yt-accent); animation: iv-slide 1.1s var(--yt-ease-standard) infinite; }
  @keyframes iv-slide { from { left: -40%; } to { left: 100%; } }
  .iv-error { display: flex; align-items: center; gap: 8px; margin: 0; padding: 12px 16px; border-radius: 12px; font: var(--yt-type-meta); color: var(--yt-error); background: color-mix(in srgb, var(--yt-error) 10%, transparent); }

  .iv-card { padding: 20px; border-radius: 12px; background: var(--yt-bg-b); border: 1px solid var(--yt-divider); }
  .iv-h2 { margin: 0 0 12px; font: var(--yt-type-headline-s); font-size: 18px; }
  .iv-muted { margin: -4px 0 12px; font: var(--yt-type-meta); color: var(--yt-text-2); }
  .iv-sources { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 12px; }
  .iv-sources li { display: flex; flex-direction: column; gap: 4px; padding: 12px; border-radius: 10px; background: var(--yt-tonal); }
  .iv-src-name { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; font: var(--yt-type-body-strong); }
  .iv-ext { font: 500 11px/16px var(--font-mono, monospace); color: var(--yt-text-2); }
  .iv-src-desc { font: 400 12px/17px var(--yt-font); color: var(--yt-text-2); }
  .iv-actions { display: flex; gap: 8px; flex-wrap: wrap; }

  .iv-report-top { display: flex; align-items: center; gap: 16px; margin-bottom: 16px; }
  .iv-report-top h2 { margin: 0; font: var(--yt-type-headline-s); }
  .iv-report-top p { margin: 2px 0 0; font: var(--yt-type-meta); color: var(--yt-text-2); }
  .iv-ok { display: grid; place-items: center; width: 52px; height: 52px; border-radius: 50%; color: var(--yt-green); background: color-mix(in srgb, var(--yt-green) 15%, transparent); flex: none; }
  .iv-stats { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; margin-bottom: 16px; }
  .iv-stat { display: flex; flex-direction: column; gap: 2px; padding: 12px; border-radius: 10px; background: var(--yt-tonal); }
  .iv-stat strong { font: var(--yt-type-headline-m); font-variant-numeric: tabular-nums; }
  .iv-stat span { font: 400 12px/16px var(--yt-font); color: var(--yt-text-2); }
  .iv-errors { margin-bottom: 16px; font: var(--yt-type-meta); color: var(--yt-text-2); }
  .iv-errors summary { cursor: pointer; color: var(--yt-error); }
  .iv-errors ul { margin: 8px 0 0; padding-left: 18px; max-height: 200px; overflow: auto; font-family: var(--font-mono, monospace); font-size: 12px; }

  @container omnitube (max-width: 600px) {
    .iv-stats { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  @media (prefers-reduced-motion: reduce) {
    .iv-progress i { animation: none; left: 0; width: 100%; opacity: .6; }
    .iv-drop { transition: none; }
  }
</style>
