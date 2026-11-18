<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import TgModal from "$lib/telegram/TgModal.svelte";
  import TgIcon from "$lib/telegram/TgIcon.svelte";
  import OutputDir from "$lib/telegram/OutputDir.svelte";
  import * as dc from "./api";
  import type { DcChannel, DcExported, DownloadProgress, ExportFormat } from "./api";
  import { dateToMs } from "./format";

  let { channel, place, outputDir = $bindable(""), onclose }: { channel: DcChannel; place: string; outputDir: string; onclose: () => void } = $props();

  const FORMATS: ExportFormat[] = ["html", "json", "txt"];
  let formats = $state<Record<ExportFormat, boolean>>({ html: true, json: true, txt: false });
  let includeMedia = $state(false);
  let from = $state("");
  let to = $state("");
  let jobId = $state<string | null>(null);
  let progress = $state<DownloadProgress | null>(null);
  let result = $state<DcExported | null>(null);
  let error = $state<string | null>(null);

  let picked = $derived(FORMATS.filter((f) => formats[f]));
  let running = $derived(!!jobId);
  let pct = $derived(progress?.phase === "media" && progress.found ? Math.round((((progress.done ?? 0) + (progress.skipped ?? 0) + (progress.failed ?? 0)) / progress.found) * 100) : null);

  onMount(() => {
    const un = listen<DownloadProgress>("guildchat-export-progress", (e) => {
      if (e.payload.job_id === jobId) progress = e.payload;
    });
    return () => {
      void un.then((f) => f());
      if (jobId) void dc.cancelJob(jobId).catch(() => {});
    };
  });

  async function start() {
    const id = `dcex-${Date.now().toString(36)}`;
    jobId = id;
    progress = { job_id: id, phase: "scan", scanned: 0 };
    result = null;
    error = null;
    try {
      result = await dc.exportChannel(id, channel.id, outputDir || null, {
        formats: picked,
        after_ms: dateToMs(from),
        before_ms: dateToMs(to, true),
        include_media: includeMedia,
      });
    } catch (e) {
      const k = dc.errKey(e);
      error = k === "rate_limited" ? $t("guildchat.errors.rate_limited_s", { secs: dc.rateLimitSeconds(e) }) : $t(`guildchat.errors.${k}`);
    } finally {
      jobId = null;
    }
  }

  function cancel() {
    if (jobId) void dc.cancelJob(jobId).catch(() => {});
  }
</script>

<TgModal title={$t("guildchat.export.title")} subtitle={`${place} · ${channel.name}`} {onclose}>
  {#if !running && !result}
    <div class="field">
      <span class="field-label">{$t("guildchat.export.formats")}</span>
      <div class="chips">
        {#each FORMATS as f (f)}
          <label class="chip"><input type="checkbox" class="checkbox" bind:checked={formats[f]} /> {$t(`guildchat.export.format_${f}`)}</label>
        {/each}
      </div>
    </div>
    <label class="chip"><input type="checkbox" class="checkbox" bind:checked={includeMedia} /> {$t("guildchat.export.include_media")}</label>
    <div class="dates">
      <label class="field">
        <span class="field-label">{$t("guildchat.common.from")}</span>
        <input class="input" type="date" bind:value={from} />
      </label>
      <label class="field">
        <span class="field-label">{$t("guildchat.common.to")}</span>
        <input class="input" type="date" bind:value={to} />
      </label>
    </div>
    <span class="field-hint">{$t("guildchat.export.dates_hint")}</span>
    <div class="field">
      <span class="field-label">{$t("guildchat.common.folder")}</span>
      <OutputDir bind:value={outputDir} />
      <span class="field-hint">{$t("guildchat.common.folder_hint")}</span>
    </div>
  {:else}
    <div class="prog" aria-live="polite">
      {#if running}
        <div class="p-line">
          <span class="spinner"></span>
          <span>
            {#if progress?.phase === "media"}
              {$t("guildchat.export.media", { done: progress.done ?? 0, found: progress.found ?? 0 })}
            {:else}
              {$t("guildchat.export.reading", { num: progress?.scanned ?? 0 })}
            {/if}
          </span>
        </div>
        <div class="progress"><div class="progress-fill" class:indet={pct == null} style:width="{pct ?? 30}%"></div></div>
      {:else if result}
        <div class="notice notice-success">
          <TgIcon name={result.cancelled ? "x" : "check"} size={16} />
          <div class="notice-body">
            <div class="notice-title">{$t(result.cancelled ? "guildchat.export.cancelled" : "guildchat.export.done")}</div>
            <div class="notice-text">{$t("guildchat.export.summary", { num: result.messages, files: result.files.length })}</div>
          </div>
        </div>
      {/if}
    </div>
  {/if}
  {#if error}<p class="field-error">{error}</p>{/if}

  {#snippet footer()}
    {#if running}
      <button class="btn btn-secondary" onclick={cancel}>{$t("guildchat.common.cancel")}</button>
    {:else if result}
      <button class="btn btn-ghost" onclick={() => void dc.reveal(result?.files[0] ?? result?.folder ?? "").catch(() => {})}><TgIcon name="folder" /> {$t("guildchat.common.show_folder")}</button>
      <button class="btn btn-primary" onclick={onclose}>{$t("guildchat.common.done")}</button>
    {:else}
      <button class="btn btn-ghost" onclick={onclose}>{$t("guildchat.common.cancel")}</button>
      <button class="btn btn-primary" disabled={!picked.length} onclick={() => void start()}><TgIcon name="save" /> {$t("guildchat.export.start")}</button>
    {/if}
  {/snippet}
</TgModal>

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-base);
    color: var(--text);
    cursor: pointer;
  }

  .dates {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .prog {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) 0;
  }

  .p-line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-base);
    color: var(--text-muted);
  }

  .indet {
    animation: dc-indet 1.4s var(--ease-in-out) infinite;
  }

  @keyframes dc-indet {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(340%);
    }
  }
</style>
