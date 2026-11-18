<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import * as api from "./api";
  import type { MediaKind, WaChat, WaDownloadDone, WaMediaStats } from "./api";
  import { onDownloadDone, onDownloadProgress } from "./session.svelte";
  import WaModal from "./WaModal.svelte";
  import WaIcon from "./WaIcon.svelte";

  let {
    chat,
    online,
    outputDir = $bindable(""),
    onclose,
  }: { chat: WaChat; online: boolean; outputDir: string; onclose: () => void } = $props();

  let stats = $state<WaMediaStats | null>(null);
  let statsError = $state<string | null>(null);
  let picked = $state<Record<MediaKind, boolean>>({ image: true, video: true, audio: true, document: true, sticker: false });
  let jobId = $state<string | null>(null);
  let total = $state(0);
  let index = $state(0);
  let finished = $state<WaDownloadDone | null>(null);
  let startError = $state<string | null>(null);
  let starting = $state(false);
  let retrying = $state(false);

  const ICONS: Record<MediaKind, string> = { image: "photo", video: "video", audio: "music", document: "file", sticker: "sticker" };

  let kinds = $derived(api.MEDIA_KINDS.filter((k) => picked[k]));
  let selectedCount = $derived(stats ? kinds.reduce((n, k) => n + stats![k], 0) : 0);
  let selectedBytes = $derived(stats ? kinds.reduce((n, k) => n + stats![`${k}_bytes` as const], 0) : 0);
  let percent = $derived(total > 0 ? Math.round((Math.min(index, total) / total) * 100) : 0);

  onMount(() => {
    void api
      .mediaStats(chat.jid)
      .then((s) => (stats = s))
      .catch((e) => (statsError = api.errText(e)));
    if (!outputDir) void api.defaultDir().then((d) => (outputDir = outputDir || d)).catch(() => {});
    const offP = onDownloadProgress((p) => {
      if (!jobId || p.job_id !== jobId) return;
      if (p.stage === "retrying") {
        retrying = true;
        return;
      }
      retrying = false;
      index = (p.index ?? 0) + 1;
      total = p.total ?? total;
    });
    const offD = onDownloadDone((d) => {
      if (!jobId || d.job_id !== jobId) return;
      finished = d;
      retrying = false;
      index = d.total;
    });
    return () => {
      offP();
      offD();
    };
  });

  async function pickDir() {
    try {
      const dir = await open({ directory: true, multiple: false, defaultPath: outputDir || undefined });
      if (typeof dir === "string" && dir) outputDir = dir;
    } catch {
    }
  }

  async function start() {
    starting = true;
    startError = null;
    finished = null;
    try {
      const job = await api.downloadChatMedia(chat.jid, kinds, outputDir || null);
      jobId = job.job_id;
      total = job.total;
      index = 0;
    } catch (e) {
      startError = api.errText(e);
    } finally {
      starting = false;
    }
  }

  function errorLabel(raw: string): string {
    const k = api.knownError(raw);
    return k ? $t(`phonechat.errors.${k}`) : raw;
  }

  let running = $derived(jobId !== null && finished === null);
</script>

<WaModal title={$t("phonechat.media.title")} subtitle={chat.name} width={520} onclose={onclose}>
  {#if statsError}
    <div class="notice notice-warning"><div class="notice-body"><div class="notice-text">{statsError}</div></div></div>
  {:else if !stats}
    <div class="center"><span class="spinner"></span></div>
  {:else}
    <fieldset class="kinds" disabled={running}>
      <legend class="field-label">{$t("phonechat.media.filter")}</legend>
      {#each api.MEDIA_KINDS as k (k)}
        <label class="kind">
          <input type="checkbox" bind:checked={picked[k]} disabled={stats[k] === 0} />
          <WaIcon name={ICONS[k]} />
          <span class="kind-name">{$t(`phonechat.media.kinds.${k}`)}</span>
          <span class="kind-count">{stats[k]} · {api.formatBytes(stats[`${k}_bytes`])}</span>
        </label>
      {/each}
    </fieldset>

    <div class="field">
      <span class="field-label">{$t("phonechat.media.folder")}</span>
      <button class="btn btn-ghost btn-sm out" onclick={pickDir} disabled={running} title={outputDir}>
        <WaIcon name="folder" />
        <span class="path">{outputDir || $t("phonechat.media.choose_folder")}</span>
      </button>
      <span class="field-hint">{$t("phonechat.media.folder_hint")}</span>
    </div>

    <p class="hint">{$t("phonechat.media.expired_hint")}</p>

    {#if jobId}
      <div class="progress" aria-live="polite">
        <div class="bar"><div class="bar-fill" style:width="{percent}%"></div></div>
        <span class="progress-text">
          {#if finished}
            {finished.cancelled ? $t("phonechat.media.cancelled") : $t("phonechat.media.finished")}
          {:else}
            {$t("phonechat.media.progress", { index: Math.min(index, total), total })}
          {/if}
        </span>
        {#if retrying && !finished}
          <span class="progress-text">{$t("phonechat.media.retrying_hint")}</span>
        {/if}
      </div>
    {/if}

    {#if finished}
      <div class="notice {finished.failed > 0 ? 'notice-warning' : 'notice-success'}">
        <div class="notice-body">
          <div class="notice-text">
            {$t("phonechat.media.summary", { done: finished.done, skipped: finished.skipped, failed: finished.failed })}
          </div>
          {#each Object.entries(finished.errors) as [reason, n] (reason)}
            <div class="notice-text err-line">{n}× {errorLabel(reason)}</div>
          {/each}
        </div>
      </div>
    {/if}

    {#if startError}
      <div class="notice notice-warning"><div class="notice-body"><div class="notice-text">{errorLabel(startError)}</div></div></div>
    {/if}
  {/if}

  {#snippet footer()}
    {#if finished}
      <button class="btn btn-ghost" onclick={() => void api.reveal(finished?.dir ?? "").catch(() => {})}>{$t("phonechat.media.reveal_folder")}</button>
      <button class="btn btn-primary" onclick={onclose}>{$t("phonechat.close")}</button>
    {:else if running}
      <button class="btn btn-secondary" onclick={() => jobId && void api.cancelDownload(jobId)}>{$t("phonechat.media.cancel")}</button>
    {:else}
      <span class="foot-info">{$t("phonechat.media.selected", { count: selectedCount, size: api.formatBytes(selectedBytes) })}</span>
      <button class="btn btn-ghost" onclick={onclose}>{$t("phonechat.cancel")}</button>
      <button class="btn btn-primary" onclick={start} disabled={!online || starting || selectedCount === 0} title={online ? undefined : $t("phonechat.errors.offline")}>
        {#if starting}<span class="spinner"></span>{/if}
        {$t("phonechat.media.start")}
      </button>
    {/if}
  {/snippet}
</WaModal>

<style>
  .center {
    display: flex;
    justify-content: center;
    padding: var(--space-5);
  }

  .kinds {
    border: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .kind {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    font-size: var(--text-base);
    color: var(--text);
    cursor: pointer;
  }

  .kind:hover {
    background: var(--fill-1);
  }

  .kind-name {
    flex: 1;
  }

  .kind-count {
    font-size: var(--text-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .out {
    align-self: flex-start;
    max-width: 100%;
    min-width: 0;
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }

  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
    line-height: 1.5;
  }

  .progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .bar {
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    overflow: hidden;
  }

  .bar-fill {
    height: 100%;
    background: var(--accent);
    transition: width var(--duration-base) var(--ease-out);
  }

  .progress-text {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .err-line {
    font-size: var(--text-xs);
  }

  .foot-info {
    margin-right: auto;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
