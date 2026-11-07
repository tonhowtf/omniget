<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";

  type StoreStats = { objects: number; bytes_on_disk: number; bytes_without_dedupe: number; savings_measurable: boolean };
  type DedupeReport = { examined: number; deduplicated: number; bytes_saved: number; savings_measurable: boolean; errors: string[] };
  type Outcome = { tone: "ok" | "info" | "error"; text: string };

  let stats = $state<StoreStats | null>(null);
  let statsLoading = $state(true);
  let statsError = $state(false);
  let running = $state(false);
  let outcome = $state<Outcome | null>(null);
  let warning = $state<string | null>(null);

  let saved = $derived(stats ? Math.max(0, stats.bytes_without_dedupe - stats.bytes_on_disk) : 0);

  function humanBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), units.length - 1);
    return `${(n / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  async function loadStats() {
    statsLoading = true;
    statsError = false;
    try { stats = await invoke<StoreStats>("content_store_stats"); }
    catch { statsError = true; }
    finally { statsLoading = false; }
  }

  async function run() {
    if (running) return;
    const chosen = await open({ multiple: true, title: $t("downloads.tools.dedupe.pick_files") as string });
    const paths = Array.isArray(chosen) ? chosen : chosen ? [chosen] : [];
    if (paths.length === 0) return;
    running = true;
    outcome = null;
    warning = null;
    try {
      const rep = await invoke<DedupeReport>("deduplicate_files", { paths });
      if (rep.deduplicated === 0) outcome = { tone: "info", text: $t("downloads.tools.dedupe.nothing", { examined: rep.examined }) as string };
      else if (rep.savings_measurable) outcome = { tone: "ok", text: $t("downloads.tools.dedupe.done", { count: rep.deduplicated, size: humanBytes(rep.bytes_saved) }) as string };
      else outcome = { tone: "ok", text: $t("downloads.tools.dedupe.done_unmeasured", { count: rep.deduplicated }) as string };
      if (rep.errors.length > 0) warning = $t("downloads.tools.dedupe.errors", { count: rep.errors.length }) as string;
      void loadStats();
    } catch (e) {
      outcome = { tone: "error", text: e instanceof Error ? e.message : String(e) };
    } finally {
      running = false;
    }
  }

  onMount(() => { void loadStats(); });
</script>

<div class="dd" data-agent-readiness={running || statsLoading ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.dedupe.lead")}</p>
  <div>
    <button class="button primary" type="button" disabled={running} onclick={run}>
      {running ? $t("downloads.tools.dedupe.running") : $t("downloads.tools.dedupe.action")}
    </button>
  </div>

  {#if outcome}
    <p class="msg" class:error={outcome.tone === "error"} role={outcome.tone === "error" ? "alert" : "status"}><bdi>{outcome.text}</bdi></p>
  {/if}
  {#if warning}
    <p class="msg warn" role="status">{warning}</p>
  {/if}

  <p class="dim" role="status">
    {#if statsLoading}
      {$t("settings.rebrand.state.loading")}
    {:else if statsError}
      <span class="error">{$t("downloads.tools.dedupe.stats_failed")}</span>
      <button class="button" type="button" onclick={() => void loadStats()}>{$t("settings.rebrand.state.retry")}</button>
    {:else if stats && stats.objects > 0 && stats.savings_measurable}
      {$t("downloads.tools.dedupe.stats", { objects: stats.objects, saved: humanBytes(saved) })}
    {:else if stats && stats.objects > 0}
      {$t("downloads.tools.dedupe.unmeasurable", { objects: stats.objects })}
    {:else}
      {$t("downloads.tools.dedupe.empty")}
    {/if}
  </p>
</div>

<style>
  .dd { display: flex; flex-direction: column; gap: var(--space-3); }
  .lead, .dim { margin: 0; color: var(--text-dim); font-size: var(--text-sm); }
  .dim { display: flex; align-items: center; gap: var(--space-2); flex-wrap: wrap; }
  .msg { margin: 0; font-size: var(--text-sm); color: var(--text); }
  .error { color: var(--error); }
  .msg.warn { color: var(--warning); }
</style>
