<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";

  type Progress = { recorded_s: number; bytes: number; segments: number; skipped: number; variant: string | null; is_live: boolean };
  type Result = {
    path: string;
    recorded_s: number;
    bytes: number;
    segments: number;
    skipped: number;
    ended: "time_limit" | "stream_ended" | "stopped" | "stalled";
    remuxed: boolean;
    variant: string | null;
  };

  const QUALITIES = [0, 1080, 720, 480, 360];
  const DURATIONS = [0, 1, 5, 15, 30, 60, 120];

  let url = $state("");
  let quality = $state(0);
  let minutes = $state(0);
  let running = $state(false);
  let stopping = $state(false);
  let progress = $state<Progress | null>(null);
  let result = $state<Result | null>(null);
  let error = $state<string | null>(null);

  let unlisten: (() => void) | null = null;
  listen<Progress>("live-record-progress", (e) => {
    if (running) progress = e.payload;
  }).then((fn) => (unlisten = fn));
  onDestroy(() => unlisten?.());

  function clock(s: number) {
    const total = Math.floor(s);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const sec = total % 60;
    const mm = String(m).padStart(2, "0");
    const ss = String(sec).padStart(2, "0");
    return h ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
  }

  async function start() {
    const target = url.trim();
    if (!target || running) return;
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      error = $t("downloads.tools.live_record.no_folder");
      return;
    }
    running = true;
    stopping = false;
    error = null;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("live_record_start", {
        url: target,
        outputDir,
        maxHeight: quality || null,
        maxMinutes: minutes || null,
      });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg === "Cancelled" ? $t("downloads.tools.live_record.cancelled") : msg ? translateBackendError(msg, $t) : $t("common.error");
    } finally {
      running = false;
      stopping = false;
    }
  }

  function stop() {
    stopping = true;
    void invoke("live_record_stop");
  }
</script>

<div class="live" data-agent-readiness={running ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.live_record.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      start();
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.live_record.placeholder")}
      aria-label={$t("downloads.tools.live_record.placeholder")}
      spellcheck="false"
      autocomplete="off"
      disabled={running}
    />
    {#if running}
      <button class="button" type="button" onclick={stop} disabled={stopping}>{$t("downloads.tools.live_record.stop")}</button>
    {:else}
      <button class="button" type="submit" disabled={!url.trim()}>{$t("downloads.tools.live_record.start")}</button>
    {/if}
  </form>

  <div class="opts">
    <label>
      {$t("downloads.tools.live_record.quality")}
      <select bind:value={quality} disabled={running}>
        {#each QUALITIES as q (q)}
          <option value={q}>{q ? `${q}p` : $t("downloads.tools.live_record.best")}</option>
        {/each}
      </select>
    </label>
    <label>
      {$t("downloads.tools.live_record.duration")}
      <select bind:value={minutes} disabled={running}>
        {#each DURATIONS as d (d)}
          <option value={d}>{d ? $t("downloads.tools.live_record.minutes", { count: d }) : $t("downloads.tools.live_record.until_stop")}</option>
        {/each}
      </select>
    </label>
  </div>

  {#if running}
    <p class="status" role="status">
      <span class="dot" aria-hidden="true"></span>
      {#if progress}
        {$t("downloads.tools.live_record.recording", { time: clock(progress.recorded_s), size: formatBytes(progress.bytes) })}
        {#if progress.variant}<span class="dim"> · {progress.variant}</span>{/if}
        {#if progress.skipped}<span class="dim"> · {$t("downloads.tools.live_record.skipped", { count: progress.skipped })}</span>{/if}
      {:else}
        {$t("downloads.tools.live_record.connecting")}
      {/if}
    </p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if result}
    <div class="done">
      <p>
        {$t(`downloads.tools.live_record.ended_${result.ended}`)}
        {$t("downloads.tools.live_record.done", { time: clock(result.recorded_s), size: formatBytes(result.bytes) })}
        {#if result.skipped}<span class="dim"> · {$t("downloads.tools.live_record.skipped", { count: result.skipped })}</span>{/if}
        {#if !result.remuxed}<span class="dim"> · {$t("downloads.tools.live_record.kept_ts")}</span>{/if}
      </p>
      <button class="link" onclick={() => void invoke("reveal_file", { path: result!.path })}>{$t("downloads.tools.live_record.show")}</button>
    </div>
  {/if}
  <p class="dim small">{$t("downloads.tools.live_record.note")}</p>
</div>

<style>
  .live {
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
  .opts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }
  .opts label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  .opts select {
    background: var(--surface);
    color: var(--text);
    border: none;
    border-radius: var(--radius-md, 8px);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    padding: 2px var(--space-2);
  }
  .status,
  .done p {
    margin: 0;
    font-size: var(--text-sm);
  }
  .status {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-1);
    font-variant-numeric: tabular-nums;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--danger, #d33);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dot {
      animation: none;
    }
  }
  .done {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
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
</style>
