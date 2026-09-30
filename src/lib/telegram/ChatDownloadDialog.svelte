<script lang="ts">
  // Download a whole chat with filters (like `tdl dl`): preview first, then queue.
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import TgModal from "./TgModal.svelte";
  import TgIcon from "./TgIcon.svelte";
  import OutputDir from "./OutputDir.svelte";
  import MediaFilterFields from "./MediaFilterFields.svelte";
  import TemplateField from "./TemplateField.svelte";
  import ScanList from "./ScanList.svelte";
  import { tg2, newScanId, formatBytes, formatDate, type ChatDownloadArgs, type ChatDownloadPreview, type TelegramChat } from "./api";
  import { dateToUnix, emptyFilter, filterFromForm, positiveInt, type FilterForm } from "./filters";
  import { cancelScan, failText, finishScan, scans, trackBatchCount, trackScan } from "./session.svelte";

  let {
    chat,
    outputDir = $bindable(""),
    onclose,
  }: { chat: TelegramChat; outputDir: string; onclose: () => void } = $props();

  let form = $state<FilterForm>(emptyFilter([]));
  let fromDate = $state("");
  let toDate = $state("");
  let fromId = $state("");
  let toId = $state("");
  let limit = $state("");
  let template = $state("");
  let skipSame = $state(true);
  let takeout = $state(false);
  let dedupe = $state(true);
  let scanId = $state<string | null>(null);
  let preview = $state<ChatDownloadPreview | null>(null);
  let previewKey = $state("");
  let previewing = $state(false);
  let starting = $state(false);
  let error = $state<string | null>(null);

  function args(): ChatDownloadArgs {
    const filter = filterFromForm(form);
    const fd = dateToUnix(fromDate);
    const td = dateToUnix(toDate, true);
    if (fd) filter.fromDate = fd;
    if (td) filter.toDate = td;
    const fi = positiveInt(fromId);
    const ti = positiveInt(toId);
    const lim = positiveInt(limit);
    if (fi) filter.fromMessageId = fi;
    if (ti) filter.toMessageId = ti;
    if (lim) filter.limit = lim;
    return { filter, template: template.trim() || undefined, outputDir: outputDir || undefined, skipSame, takeout, dedupe };
  }

  // The preview only matches the filters it was made with.
  let currentKey = $derived(JSON.stringify({ f: filterFromForm(form), fromDate, toDate, fromId, toId, limit, takeout, dedupe }));
  let stale = $derived(!!preview && previewKey !== currentKey);

  async function runPreview() {
    previewing = true;
    error = null;
    const id = newScanId();
    scanId = id;
    trackScan(id, chat.id, chat.title, "preview");
    try {
      const key = currentKey;
      const r = await tg2.previewChatDownload(chat, { ...args(), scanId: id });
      if (scans[id]?.cancelled) return;
      preview = r;
      previewKey = key;
    } catch (e) {
      if (!scans[id]?.cancelled) error = failText(e);
    } finally {
      finishScan(id);
      if (scanId === id) scanId = null;
      previewing = false;
    }
  }

  function stopPreview() {
    if (scanId) void cancelScan(scanId);
    previewing = false;
  }

  async function start() {
    if (!outputDir) {
      showToast("info", $t("telegram.common.choose_folder_first") as string);
      return;
    }
    starting = true;
    error = null;
    try {
      const r = await tg2.downloadChat(chat, args());
      if (r.batch_id == null || !r.count) {
        showToast("info", $t("telegram.chat_dl.nothing") as string);
        return;
      }
      trackBatchCount(r.batch_id, chat.title, r.count);
      showToast("success", $t("telegram.chat_dl.started", { count: r.count }) as string);
      onclose();
    } catch (e) {
      error = failText(e);
    } finally {
      starting = false;
    }
  }
</script>

<TgModal title={$t("telegram.chat_dl.title")} subtitle={$t("telegram.chat_dl.hint", { chat: chat.title })} width={640} {onclose}>
  <MediaFilterFields bind:form />

  <details class="more">
    <summary>{$t("telegram.chat_dl.range")}</summary>
    <div class="grid2">
      <label class="field">
        <span class="field-label">{$t("telegram.chat_dl.from_date")}</span>
        <input class="input" type="date" bind:value={fromDate} />
      </label>
      <label class="field">
        <span class="field-label">{$t("telegram.chat_dl.to_date")}</span>
        <input class="input" type="date" bind:value={toDate} />
      </label>
      <label class="field">
        <span class="field-label">{$t("telegram.chat_dl.from_id")}</span>
        <input class="input" inputmode="numeric" bind:value={fromId} placeholder="1" />
      </label>
      <label class="field">
        <span class="field-label">{$t("telegram.chat_dl.to_id")}</span>
        <input class="input" inputmode="numeric" bind:value={toId} placeholder="∞" />
      </label>
      <label class="field">
        <span class="field-label">{$t("telegram.chat_dl.limit")}</span>
        <input class="input" inputmode="numeric" bind:value={limit} placeholder={$t("telegram.chat_dl.no_limit")} />
      </label>
    </div>
  </details>

  <div class="field">
    <span class="field-label">{$t("telegram.common.output_folder")}</span>
    <OutputDir bind:value={outputDir} />
  </div>

  <TemplateField bind:value={template} {chat} sample={preview?.sample?.[0] ?? null} />

  <div class="group">
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.chat_dl.skip_same")}</span>
        <span class="group-row-sub">{$t("telegram.chat_dl.skip_same_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={skipSame} aria-label={$t("telegram.chat_dl.skip_same")} onclick={() => (skipSame = !skipSame)}><span class="toggle-knob"></span></button>
    </div>
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.chat_dl.takeout")}</span>
        <span class="group-row-sub">{$t("telegram.chat_dl.takeout_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={takeout} aria-label={$t("telegram.chat_dl.takeout")} onclick={() => (takeout = !takeout)}><span class="toggle-knob"></span></button>
    </div>
    <div class="group-row">
      <div class="group-row-content">
        <span class="group-row-title">{$t("telegram.dedupe.option")}</span>
        <span class="group-row-sub">{$t("telegram.dedupe.option_hint")}</span>
      </div>
      <button class="toggle" role="switch" aria-checked={dedupe} aria-label={$t("telegram.dedupe.option")} onclick={() => (dedupe = !dedupe)}><span class="toggle-knob"></span></button>
    </div>
  </div>

  {#if scanId}<ScanList only={[scanId]} compact />{/if}

  {#if preview}
    <div class="pv" class:stale>
      <div class="pv-head">
        <TgIcon name="eye" size={16} />
        <strong>{$t("telegram.chat_dl.preview_result", { count: preview.count, size: formatBytes(preview.total_bytes) })}</strong>
        {#if stale}<span class="tag tag-warning">{$t("telegram.chat_dl.stale")}</span>{/if}
      </div>
      {#if preview.output_dir}<div class="pv-dir" title={preview.output_dir}>{preview.output_dir}</div>{/if}
      {#if preview.from_index || preview.scanned}
        <div class="pv-dir">
          {preview.scanned ? $t("telegram.scan.read_n", { count: preview.scanned.toLocaleString() }) : ""}{preview.scanned && preview.from_index ? " · " : ""}{preview.from_index ? $t("telegram.scan.from_index") : ""}
        </div>
      {/if}
      {#if preview.sample?.length}
        <ul class="pv-list">
          {#each preview.sample as s (s.message_id)}
            <li class:exists={s.exists}>
              <span class="pv-name" title={s.path ?? s.file_name}>{s.path ?? s.file_name}</span>
              {#if s.exists}<span class="tag">{$t("telegram.chat_dl.exists")}</span>{/if}
              <span class="pv-meta">{formatBytes(s.file_size)} · {formatDate(s.date)}</span>
            </li>
          {/each}
        </ul>
        {#if preview.count > preview.sample.length}
          <p class="pv-more">{$t("telegram.chat_dl.and_more", { count: preview.count - preview.sample.length })}</p>
        {/if}
      {/if}
    </div>
  {/if}

  {#if error}<p class="field-error">{error}</p>{/if}

  {#snippet footer()}
    {#if previewing}
      <button class="btn btn-secondary" onclick={stopPreview}>
        <TgIcon name="x" /> {$t("telegram.scan.stop")}
      </button>
    {:else}
      <button class="btn btn-secondary" onclick={runPreview}>
        <TgIcon name="eye" /> {$t("telegram.chat_dl.preview")}
      </button>
    {/if}
    <button class="btn btn-primary" onclick={start} disabled={starting || !preview || stale || preview.count === 0}>
      {#if starting}<span class="spinner"></span>{:else}<TgIcon name="download" />{/if}
      {preview && !stale ? $t("telegram.chat_dl.start_n", { count: preview.count }) : $t("telegram.chat_dl.start")}
    </button>
  {/snippet}
</TgModal>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .more summary {
    cursor: pointer;
    font-size: var(--text-sm);
    font-weight: 550;
    color: var(--text-muted);
  }

  .more[open] summary {
    margin-bottom: var(--space-2);
  }

  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .pv {
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--fill-1);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    transition: opacity var(--duration-base) var(--ease-out);
  }

  .pv.stale {
    opacity: 0.6;
  }

  .pv-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text);
    font-size: var(--text-sm);
  }

  .pv-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 180px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .pv-dir {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
  }

  .pv-list li.exists .pv-name {
    color: var(--text-dim);
  }

  .pv-list li {
    align-items: center;
    display: flex;
    gap: var(--space-2);
    font-size: var(--text-xs);
  }

  .pv-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--text);
  }

  .pv-meta,
  .pv-more {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .pv-more {
    margin: 0;
    font-size: var(--text-xs);
  }

  @media (max-width: 520px) {
    .grid2 {
      grid-template-columns: 1fr;
    }
  }
</style>
