<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { pluralKey } from "$lib/home/plural";
  import { translateBackendError } from "$lib/error-translate";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Item = {
    id: string;
    title: string;
    bytes: number | null;
    estimate: boolean;
    duration: number | null;
    height: number | null;
  };
  type Check = {
    items: Item[];
    total_bytes: number;
    unknown: number;
    estimated: boolean;
    limit_reached: boolean;
    limit: number;
    output_dir: string;
    free_bytes: number | null;
  };

  // 0 = audio only, null = best available.
  const QUALITIES: (number | null)[] = [null, 2160, 1440, 1080, 720, 480, 360, 0];

  let url = $state("");
  let quality = $state<number | null>(1080);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let result = $state<Check | null>(null);
  let checkedUrl = $state("");
  let checkedQuality = $state<number | null>(null);

  const fits = $derived(result && result.free_bytes != null ? result.total_bytes <= result.free_bytes : null);
  const left = $derived(result && result.free_bytes != null ? result.free_bytes - result.total_bytes : null);
  const totalDuration = $derived(result ? result.items.reduce((s, i) => s + (i.duration ?? 0), 0) : 0);

  function qualityLabel(q: number | null): string {
    if (q === null) return $t("downloads.tools.size_check.best");
    if (q === 0) return $t("downloads.tools.size_check.audio");
    return `${q}p`;
  }

  function clock(seconds: number): string {
    const s = Math.round(seconds);
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const ss = String(s % 60).padStart(2, "0");
    return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
  }

  async function check() {
    const target = url.trim();
    if (!target || loading) return;
    loading = true;
    error = null;
    result = null;
    try {
      result = await invoke<Check>("check_download_size", { url: target, quality });
      checkedUrl = target;
      checkedQuality = quality;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg ? translateBackendError(msg, $t) : $t("downloads.tools.size_check.error");
    } finally {
      loading = false;
    }
  }

  async function download() {
    if (!result) return;
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      showToast("error", $t("downloads.tools.size_check.no_folder"));
      return;
    }
    const audio = checkedQuality === 0;
    try {
      await invoke("download_from_url", {
        url: checkedUrl,
        outputDir,
        downloadMode: audio ? "audio" : null,
        quality: audio || checkedQuality === null ? "best" : `${checkedQuality}p`,
        formatId: null,
        referer: null,
        cookieSlug: null,
      });
      showToast("success", $t("downloads.tools.size_check.queued"));
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      showToast("error", msg ? translateBackendError(msg, $t) : $t("common.error"));
    }
  }
</script>

<div class="sc" data-agent-readiness={loading ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.size_check.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      check();
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.size_check.placeholder")}
      aria-label={$t("downloads.tools.size_check.placeholder")}
      spellcheck="false"
      autocomplete="off"
    />
    <select bind:value={quality} aria-label={$t("downloads.tools.size_check.quality")}>
      {#each QUALITIES as q (q)}
        <option value={q}>{qualityLabel(q)}</option>
      {/each}
    </select>
    <button class="button" type="submit" disabled={loading || !url.trim()}>
      {loading ? $t("downloads.tools.size_check.loading") : $t("downloads.tools.size_check.check")}
    </button>
  </form>
  {#if loading}
    <p class="dim small">{$t("downloads.tools.size_check.slow_note")}</p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if result}
    <div class="summary" class:bad={fits === false} data-fits={fits}>
      <div class="total">
        <span class="big">{result.estimated ? "≈ " : ""}{formatBytes(result.total_bytes)}</span>
        <span class="dim">
          {$t(pluralKey("downloads.tools.size_check.items", result.items.length), { count: result.items.length })} · {qualityLabel(checkedQuality)}{#if totalDuration > 0}
            · {clock(totalDuration)}{/if}
        </span>
      </div>
      {#if result.free_bytes != null}
        <p class="verdict">
          {#if fits}
            {$t("downloads.tools.size_check.fits", { left: formatBytes(left ?? 0) })}
          {:else}
            {$t("downloads.tools.size_check.no_room", { free: formatBytes(result.free_bytes) })}
          {/if}
        </p>
        <p class="dim small folder" title={result.output_dir}>{result.output_dir}</p>
      {/if}
      {#if result.unknown > 0}
        <p class="dim small">{$t(pluralKey("downloads.tools.size_check.unknown", result.unknown), { count: result.unknown })}</p>
      {/if}
      {#if result.limit_reached}
        <p class="dim small">{$t("downloads.tools.size_check.limit", { count: result.limit })}</p>
      {/if}
      {#if result.items.length === 1}
        <button class="button" onclick={download}>{$t("downloads.tools.size_check.download")}</button>
      {/if}
    </div>

    {#if result.items.length > 1}
      <ul class="list">
        {#each result.items as item (item.id)}
          <li class="item">
            <span class="name" title={item.title}>{item.title || item.id}</span>
            {#if item.duration}<span class="dim small">{clock(item.duration)}</span>{/if}
            <span class="size">
              {#if item.bytes != null}{item.estimate ? "≈ " : ""}{formatBytes(item.bytes)}{:else}—{/if}
            </span>
          </li>
        {/each}
      </ul>
    {/if}
    {#if result.estimated}
      <p class="dim small">{$t("downloads.tools.size_check.estimate_note")}</p>
    {/if}
  {/if}
</div>

<style>
  .sc {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .row {
    display: flex;
    gap: var(--space-2);
  }
  .row input,
  .row select {
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .row input {
    flex: 1;
    min-width: 0;
  }
  .row input:focus-visible,
  .row select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .summary {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .summary.bad {
    box-shadow: inset 0 0 0 var(--hairline) var(--danger, #d33);
  }
  .total {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .big {
    font-size: var(--text-xl, 20px);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .verdict {
    margin: 0;
    font-size: var(--text-sm);
  }
  .summary.bad .verdict {
    color: var(--danger, #d33);
  }
  .folder {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .item {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
  }
  .name {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size {
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }
</style>
