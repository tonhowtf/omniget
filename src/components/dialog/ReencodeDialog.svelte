<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import DialogContainer from "./DialogContainer.svelte";

  type Codec = "hevc" | "h264" | "av1";

  type ReencodeResult = {
    output_path: string;
    encoder_used: string;
    used_hwaccel: boolean;
    original_size_bytes: number;
    new_size_bytes: number;
    savings_pct: number;
    replaced_original: boolean;
  };

  let {
    inputPath = $bindable<string | null>(null),
  }: {
    inputPath?: string | null;
  } = $props();

  let isOpen = $derived(inputPath !== null);
  let codec = $state<Codec>("hevc");
  let replaceOriginal = $state(false);
  let busy = $state(false);

  function close() {
    if (busy) return;
    inputPath = null;
    codec = "hevc";
    replaceOriginal = false;
  }

  async function runReencode() {
    if (!inputPath || busy) return;
    busy = true;
    try {
      const result = await invoke<ReencodeResult>("reencode_video", {
        req: {
          input_path: inputPath,
          output_path: null,
          codec,
          cq: null,
          replace_original: replaceOriginal,
        },
      });
      const saved = formatBytes(result.original_size_bytes - result.new_size_bytes);
      const pct = result.savings_pct.toFixed(0);
      const toastKey = result.replaced_original
        ? "reencode.toast_replaced"
        : "reencode.toast_done";
      showToast(
        "info",
        $t(toastKey, { saved, pct, encoder: result.encoder_used }) as string,
      );
      inputPath = null;
      codec = "hevc";
      replaceOriginal = false;
    } catch (e: any) {
      const msg = typeof e === "string" ? e : e.message ?? $t("common.error");
      showToast("error", msg);
    } finally {
      busy = false;
    }
  }

  const codecOptions: { id: Codec; titleKey: string; subKey: string; tag?: string }[] = [
    { id: "hevc", titleKey: "reencode.codec_hevc_title", subKey: "reencode.codec_hevc_desc", tag: "recommended" },
    { id: "h264", titleKey: "reencode.codec_h264_title", subKey: "reencode.codec_h264_desc" },
    { id: "av1", titleKey: "reencode.codec_av1_title", subKey: "reencode.codec_av1_desc" },
  ];
</script>

<DialogContainer bind:isOpen onClose={close} titleId="reencode-dialog-title">
  <div class="reencode-header">
    <h2 id="reencode-dialog-title">{$t("reencode.title")}</h2>
    <button class="dialog-close" onclick={close} aria-label={$t("common.close")} disabled={busy}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M18 6L6 18M6 6l12 12" />
      </svg>
    </button>
  </div>

  <div class="reencode-body">
    {#if inputPath}
      <p class="reencode-target" title={inputPath}>
        {inputPath.split(/[\\/]/).pop()}
      </p>
    {/if}

    <fieldset class="codec-fieldset" disabled={busy}>
      <legend>{$t("reencode.codec_legend")}</legend>
      {#each codecOptions as opt}
        <label class="codec-option" class:selected={codec === opt.id}>
          <input
            type="radio"
            name="codec"
            value={opt.id}
            bind:group={codec}
          />
          <span class="codec-text">
            <span class="codec-title">
              {$t(opt.titleKey)}
              {#if opt.tag === "recommended"}
                <span class="codec-tag">{$t("reencode.tag_recommended")}</span>
              {/if}
            </span>
            <span class="codec-sub">{$t(opt.subKey)}</span>
          </span>
        </label>
      {/each}
    </fieldset>

    <label class="replace-row">
      <input
        type="checkbox"
        bind:checked={replaceOriginal}
        disabled={busy}
      />
      <span class="replace-text">
        <span class="replace-title">{$t("reencode.replace_title")}</span>
        <span class="replace-sub">{$t("reencode.replace_desc")}</span>
      </span>
    </label>
  </div>

  <div class="reencode-footer">
    <button class="btn-secondary" onclick={close} disabled={busy}>
      {$t("common.cancel")}
    </button>
    <button class="btn-primary" onclick={runReencode} disabled={busy || !inputPath}>
      {#if busy}
        <span class="spinner" aria-hidden="true"></span>
        {$t("reencode.encoding")}
      {:else}
        {$t("reencode.start")}
      {/if}
    </button>
  </div>
</DialogContainer>

<style>
  .reencode-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px 20px 4px;
  }

  .reencode-header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--secondary);
  }

  .dialog-close {
    background: transparent;
    border: none;
    color: var(--tertiary);
    padding: 4px;
    cursor: pointer;
    border-radius: var(--border-radius);
  }

  .dialog-close:hover:not(:disabled) {
    background: var(--button-hover);
    color: var(--secondary);
  }

  .dialog-close:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .reencode-body {
    padding: 8px 20px 16px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .reencode-target {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--tertiary);
    font-family: var(--font-mono, ui-monospace, monospace);
    background: var(--button);
    padding: 8px 10px;
    border-radius: var(--border-radius);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .codec-fieldset {
    border: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .codec-fieldset legend {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--tertiary);
    margin-bottom: 6px;
    padding: 0;
  }

  .codec-option {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--border-radius);
    background: var(--button);
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s;
  }

  .codec-option:hover {
    background: var(--button-hover);
  }

  .codec-option.selected {
    background: var(--button-elevated);
    border-color: var(--accent);
  }

  .codec-option input[type="radio"] {
    margin-top: 3px;
    accent-color: var(--accent);
    flex-shrink: 0;
  }

  .codec-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }

  .codec-title {
    font-size: 13.5px;
    font-weight: 500;
    color: var(--secondary);
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .codec-tag {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--accent);
    background: var(--accent-soft, color-mix(in srgb, var(--accent) 15%, transparent));
    padding: 2px 6px;
    border-radius: calc(var(--border-radius) / 2);
  }

  .codec-sub {
    font-size: 12px;
    color: var(--tertiary);
  }

  .replace-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--border-radius);
    background: var(--button);
    cursor: pointer;
  }

  .replace-row input[type="checkbox"] {
    margin-top: 3px;
    accent-color: var(--accent);
    flex-shrink: 0;
  }

  .replace-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }

  .replace-title {
    font-size: 13.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .replace-sub {
    font-size: 12px;
    color: var(--tertiary);
    line-height: 1.4;
  }

  .reencode-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px 16px;
    border-top: none;
  }

  .btn-secondary,
  .btn-primary {
    padding: 8px 16px;
    border-radius: var(--border-radius);
    border: none;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .btn-secondary {
    background: var(--button);
    color: var(--secondary);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--button-hover);
  }

  .btn-primary {
    background: var(--accent);
    color: var(--on-accent);
  }

  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }

  .btn-primary:disabled,
  .btn-secondary:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .spinner {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-top-color: transparent;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
