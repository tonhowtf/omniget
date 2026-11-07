<script lang="ts">
  import { untrack } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import Modal from "$components/music/Modal.svelte";
  import type { ExportFormat, ExportOptions } from "$lib/photos/types";

  let {
    count,
    anyEdited,
    initial,
    onclose,
    onexport,
  }: {
    count: number;
    anyEdited: boolean;
    initial: ExportOptions;
    onclose: () => void;
    onexport: (options: ExportOptions) => void;
  } = $props();

  const FORMATS: { id: ExportFormat; label: string }[] = [
    { id: "jpeg", label: "JPG" },
    { id: "png", label: "PNG" },
    { id: "webp", label: "WebP" },
    { id: "tiff", label: "TIFF" },
  ];
  const SIZES = [0, 3840, 2560, 1920, 1280, 800];

  const start = untrack(() => ({ ...initial }));
  let format = $state<ExportFormat>(start.format);
  let quality = $state(start.quality ?? 88);
  let maxSize = $state(start.max_size ?? 0);
  let outDir = $state<string | null>(start.out_dir);
  let applyEdits = $state(start.apply_edits);

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof picked === "string") outDir = picked;
  }

  function submit() {
    onexport({
      format,
      quality: format === "jpeg" ? quality : null,
      max_size: maxSize > 0 ? maxSize : null,
      out_dir: outDir,
      apply_edits: applyEdits,
    });
  }
</script>

<Modal title={(count === 1 ? $t("photos.export.title_one") : $t("photos.export.title_many", { count })) as string} width={460} {onclose}>
  <div class="ex">
    <div class="field">
      <span class="field-label">{$t("photos.export.format")}</span>
      <div class="segmented" role="radiogroup" aria-label={$t("photos.export.format") as string}>
        {#each FORMATS as f (f.id)}
          <button type="button" class="segmented-btn" role="radio" aria-checked={format === f.id} onclick={() => (format = f.id)}>{f.label}</button>
        {/each}
      </div>
      {#if format === "webp"}
        <span class="field-hint">{$t("photos.export.webp_hint")}</span>
      {/if}
    </div>

    {#if format === "jpeg"}
      <label class="field">
        <span class="ex-head">
          <span class="field-label">{$t("photos.export.quality")}</span>
          <output>{quality}</output>
        </span>
        <input type="range" min="40" max="100" step="1" bind:value={quality} />
        <span class="field-hint">{$t("photos.export.quality_hint")}</span>
      </label>
    {/if}

    <label class="field">
      <span class="field-label">{$t("photos.export.size")}</span>
      <select class="input" bind:value={maxSize}>
        {#each SIZES as s (s)}
          <option value={s}>{s === 0 ? $t("photos.export.size_original") : $t("photos.export.size_px", { size: s })}</option>
        {/each}
      </select>
    </label>

    <div class="field">
      <span class="field-label">{$t("photos.export.folder")}</span>
      <div class="ex-folder">
        <span class="ex-folder-path" title={outDir ?? ""}>{outDir ?? $t("photos.export.folder_same")}</span>
        <button type="button" class="btn btn-secondary btn-sm" onclick={() => void chooseFolder()}>{$t("photos.export.folder_choose")}</button>
        {#if outDir}
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => (outDir = null)}>{$t("photos.export.folder_reset")}</button>
        {/if}
      </div>
      <span class="field-hint">{$t("photos.export.never_overwrites")}</span>
    </div>

    {#if anyEdited}
      <label class="ex-check">
        <input type="checkbox" class="checkbox" bind:checked={applyEdits} />
        <span>{$t("photos.export.apply_edits")}</span>
      </label>
    {/if}
  </div>

  <div class="dialog-actions">
    <button type="button" class="btn" onclick={onclose}>{$t("photos.editor.cancel")}</button>
    <button type="button" class="btn btn-primary" onclick={submit}>{$t("photos.export.action")}</button>
  </div>
</Modal>

<style>
  .ex {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    margin: var(--space-3) 0 var(--space-4);
  }

  .ex-head {
    display: flex;
    justify-content: space-between;
  }

  .ex-head output {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .ex input[type="range"] {
    accent-color: var(--accent);
  }

  .ex-folder {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .ex-folder-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-size: var(--text-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }

  .ex-check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text);
  }
</style>
