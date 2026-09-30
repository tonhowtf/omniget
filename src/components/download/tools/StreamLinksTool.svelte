<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Stream = {
    format_id: string;
    kind: "av" | "video" | "audio";
    ext: string;
    resolution: string | null;
    height: number | null;
    fps: number | null;
    vcodec: string | null;
    acodec: string | null;
    filesize: number | null;
    filesize_is_estimate: boolean;
    tbr: number | null;
    protocol: string;
    url: string;
    headers: [string, string][];
    note: string | null;
  };
  type Links = { title: string; site: string; page_url: string; duration: number | null; streams: Stream[] };

  let url = $state("");
  let loading = $state(false);
  let error = $state<string | null>(null);
  let result = $state<Links | null>(null);
  let filter = $state<"all" | "av" | "video" | "audio">("all");

  const shown = $derived(result ? result.streams.filter((s) => filter === "all" || s.kind === filter) : []);

  async function list() {
    const target = url.trim();
    if (!target || loading) return;
    loading = true;
    error = null;
    result = null;
    try {
      result = await invoke<Links>("list_stream_links", { url: target });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg ? translateBackendError(msg, $t) : $t("downloads.tools.stream_links.error");
    } finally {
      loading = false;
    }
  }

  const codec = (c: string | null) => (c ? c.split(".")[0] : "");
  const segmented = (s: Stream) => !/^https?$/.test(s.protocol);
  const shell = (v: string) => `'${v.replace(/'/g, `'\\''`)}'`;

  function curl(s: Stream): string {
    const name = `${(result?.title || "media").replace(/[\\/:*?"<>|]+/g, "_").slice(0, 80)}.${s.ext || "bin"}`;
    const headers = s.headers.flatMap(([k, v]) => ["-H", shell(`${k}: ${v}`)]);
    return ["curl", "-L", "-o", shell(name), ...headers, shell(s.url)].join(" ");
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast("success", $t("downloads.tools.stream_links.copied"));
    } catch {
      showToast("error", $t("common.error"));
    }
  }

  function asJson(): string {
    return JSON.stringify(result, null, 2);
  }

  async function download(s: Stream) {
    if (!result) return;
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      showToast("error", $t("downloads.tools.stream_links.no_folder"));
      return;
    }
    // A video-only stream gets the best audio merged in, like lux does with DASH.
    const formatId = s.kind === "video" ? `${s.format_id}+bestaudio/${s.format_id}` : s.format_id;
    try {
      await invoke("download_from_url", {
        url: result.page_url || url.trim(),
        outputDir,
        downloadMode: null,
        quality: null,
        formatId,
        referer: null,
        cookieSlug: null,
      });
      showToast("success", $t("downloads.tools.stream_links.queued"));
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      showToast("error", msg ? translateBackendError(msg, $t) : $t("common.error"));
    }
  }
</script>

<div class="sl" data-agent-readiness={loading ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.stream_links.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      list();
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.stream_links.placeholder")}
      aria-label={$t("downloads.tools.stream_links.placeholder")}
      spellcheck="false"
      autocomplete="off"
    />
    <button class="button" type="submit" disabled={loading || !url.trim()}>
      {loading ? $t("downloads.tools.stream_links.loading") : $t("downloads.tools.stream_links.list")}
    </button>
  </form>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if result}
    <div class="head">
      <div class="meta">
        <strong>{result.title}</strong>
        <span class="dim">
          {result.site} · {$t("downloads.tools.stream_links.count", { count: result.streams.length })}
        </span>
      </div>
      <button class="button" onclick={() => copy(asJson())}>{$t("downloads.tools.stream_links.copy_json")}</button>
    </div>

    <div class="filters" role="group" aria-label={$t("downloads.tools.stream_links.filter")}>
      {#each ["all", "av", "video", "audio"] as f (f)}
        <button class="chip" class:on={filter === f} aria-pressed={filter === f} onclick={() => (filter = f as typeof filter)}>
          {$t(`downloads.tools.stream_links.kind_${f}`)}
        </button>
      {/each}
    </div>

    <ul class="list">
      {#each shown as s (s.format_id)}
        <li class="stream" data-format-id={s.format_id}>
          <div class="info">
            <span class="kind kind-{s.kind}">{$t(`downloads.tools.stream_links.kind_${s.kind}`)}</span>
            <span class="main">
              {s.resolution ?? (s.kind === "audio" ? $t("downloads.tools.stream_links.kind_audio") : "?")}{#if s.fps && s.kind !== "audio"}&nbsp;{Math.round(s.fps)}fps{/if}
              · {s.ext}
              {#if codec(s.vcodec) || codec(s.acodec)}· {[codec(s.vcodec), codec(s.acodec)].filter(Boolean).join(" + ")}{/if}
            </span>
            <span class="size">
              {#if s.filesize}{s.filesize_is_estimate ? "≈ " : ""}{formatBytes(s.filesize)}{:else}{$t("downloads.tools.stream_links.size_unknown")}{/if}
            </span>
            <span class="dim small">{s.format_id}{#if s.note} · {s.note}{/if}{#if segmented(s)} · {s.protocol}{/if}</span>
          </div>
          <div class="actions">
            <button class="link" onclick={() => copy(s.url)}>
              {segmented(s) ? $t("downloads.tools.stream_links.copy_playlist") : $t("downloads.tools.stream_links.copy_link")}
            </button>
            {#if !segmented(s)}
              <button class="link" onclick={() => copy(curl(s))}>{$t("downloads.tools.stream_links.copy_curl")}</button>
            {/if}
            <button class="button small-btn" onclick={() => download(s)}>{$t("downloads.tools.stream_links.download")}</button>
          </div>
        </li>
      {/each}
    </ul>
    <p class="dim small">{$t("downloads.tools.stream_links.expiry_note")}</p>
  {/if}
</div>

<style>
  .sl {
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
  .row input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .row input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .meta strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .filters {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .chip {
    padding: 2px var(--space-2);
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-dim);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .stream {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .info {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: baseline;
    column-gap: var(--space-2);
    min-width: 0;
    flex: 1;
  }
  .info .small {
    grid-column: 2 / 4;
  }
  .main {
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size {
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }
  .kind {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .kind-av {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
    border-radius: var(--radius-full);
  }
  .link:hover {
    background: var(--accent-soft);
  }
  .small-btn {
    font-size: var(--text-sm);
  }
</style>
