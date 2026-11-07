<script lang="ts">
  import { t, locale } from "$lib/i18n";
  import { LANGUAGES, formatBytes, languageLabel } from "$lib/dictation/logic";
  import type { Device, ModelProgress, Settings, Status } from "$lib/dictation/types";
  import Icon from "./Icon.svelte";
  import ShortcutRecorder from "./ShortcutRecorder.svelte";
  import ReplacementsEditor from "./ReplacementsEditor.svelte";

  let {
    settings,
    status,
    devices,
    devicesLoading,
    devicesError,
    shortcutError,
    shortcutSaving,
    locked,
    modelProgress,
    previewInput = $bindable(),
    previewOutput,
    onchange,
    ontoggleshortcut,
    onpickshortcut,
    onrecordstart,
    onrecordend,
    onrefreshdevices,
    ondownloadmodel,
  }: {
    settings: Settings;
    status: Status;
    devices: Device[];
    devicesLoading: boolean;
    devicesError: string | null;
    shortcutError: string | null;
    shortcutSaving: boolean;
    locked: boolean;
    modelProgress: ModelProgress | null;
    previewInput: string;
    previewOutput: string;
    onchange: (patch: Partial<Settings>) => void;
    ontoggleshortcut: (on: boolean) => void;
    onpickshortcut: (combo: string) => void;
    onrecordstart: () => void;
    onrecordend: () => void;
    onrefreshdevices: () => void;
    ondownloadmodel: (id: string) => void;
  } = $props();

  const LIMITS = [30, 60, 120, 300, 600];

  let selectedModel = $derived(status.models.find((m) => m.id === settings.model) ?? null);
  let deviceMissing = $derived(!!settings.device && !devicesLoading && devices.length > 0 && !devices.some((d) => d.id === settings.device));
  let percent = $derived(modelProgress ? Math.round(modelProgress.fraction * 100) : 0);

  function limitLabel(s: number): string {
    return s < 60 ? ($t("dictation.limit_seconds", { n: s }) as string) : ($t("dictation.limit_minutes", { n: s / 60 }) as string);
  }

  function value(e: Event): string {
    return (e.currentTarget as HTMLSelectElement | HTMLInputElement).value;
  }
</script>

<div class="dc-set">
  <section class="dc-set-group" aria-labelledby="dc-set-shortcut">
    <h3 id="dc-set-shortcut" class="dc-set-title">{$t("dictation.set_shortcut")}</h3>
    <div class="dc-set-card">
      <div class="dc-set-line">
        <span class="dc-set-label">
          <span class="field-label">{$t("dictation.shortcut_on")}</span>
          <span class="field-hint">{$t("dictation.shortcut_on_hint")}</span>
        </span>
        <button
          type="button"
          class="toggle"
          class:on={settings.enabled}
          role="switch"
          aria-checked={settings.enabled}
          aria-label={$t("dictation.shortcut_on") as string}
          disabled={shortcutSaving}
          onclick={() => ontoggleshortcut(!settings.enabled)}
        ><span class="toggle-knob"></span></button>
      </div>
      <ShortcutRecorder
        value={settings.shortcut}
        platform={status.platform}
        defaultValue={status.default_shortcut}
        disabled={locked || shortcutSaving}
        error={shortcutError}
        saving={shortcutSaving}
        {onrecordstart}
        {onrecordend}
        onpick={onpickshortcut}
      />
      <div class="dc-set-field">
        <span class="field-label" id="dc-mode-label">{$t("dictation.mode")}</span>
        <div class="segmented" role="radiogroup" aria-labelledby="dc-mode-label">
          {#each ["hold", "toggle"] as const as m (m)}
            <button type="button" class="segmented-btn" role="radio" aria-checked={settings.mode === m} disabled={locked} onclick={() => onchange({ mode: m })}>
              {$t(`dictation.mode_${m}`)}
            </button>
          {/each}
        </div>
        <span class="field-hint">{$t(`dictation.mode_${settings.mode}_hint`)}</span>
      </div>
    </div>
  </section>

  <section class="dc-set-group" aria-labelledby="dc-set-mic">
    <h3 id="dc-set-mic" class="dc-set-title">{$t("dictation.set_mic")}</h3>
    <div class="dc-set-card">
      <div class="dc-set-inline">
        <select class="input dc-set-select" aria-label={$t("dictation.mic") as string} value={settings.device ?? ""} disabled={locked} onchange={(e) => onchange({ device: value(e) || null })}>
          <option value="">{$t("dictation.mic_default")}</option>
          {#each devices as d (d.id)}
            <option value={d.id}>{d.name}</option>
          {/each}
          {#if deviceMissing}
            <option value={settings.device}>{$t("dictation.mic_missing", { name: settings.device })}</option>
          {/if}
        </select>
        <button type="button" class="btn btn-ghost btn-sm btn-icon" disabled={devicesLoading} onclick={onrefreshdevices} aria-label={$t("dictation.mic_refresh") as string} title={$t("dictation.mic_refresh") as string}>
          {#if devicesLoading}<span class="spinner dc-set-spin" aria-hidden="true"></span>{:else}<Icon name="refresh" size={13} />{/if}
        </button>
      </div>
      {#if devicesError}
        <p class="field-error">{devicesError}</p>
      {:else if deviceMissing}
        <p class="field-hint">{$t("dictation.mic_missing_hint")}</p>
      {:else if status.platform === "linux" && devices.length === 0 && !devicesLoading}
        <p class="field-hint">{$t("dictation.mic_linux_hint")}</p>
      {/if}
    </div>
  </section>

  <section class="dc-set-group" aria-labelledby="dc-set-asr">
    <h3 id="dc-set-asr" class="dc-set-title">{$t("dictation.set_transcription")}</h3>
    <div class="dc-set-card">
      <label class="dc-set-field">
        <span class="field-label">{$t("dictation.language")}</span>
        <select class="input dc-set-select" value={settings.language} disabled={locked} onchange={(e) => onchange({ language: value(e) })}>
          {#each LANGUAGES as code (code)}
            <option value={code}>{code === "auto" ? $t("dictation.language_auto") : languageLabel(code, $locale)}</option>
          {/each}
        </select>
        <span class="field-hint">{$t("dictation.language_hint")}</span>
      </label>
      <label class="dc-set-field">
        <span class="field-label">{$t("dictation.model")}</span>
        <select class="input dc-set-select" value={settings.model} disabled={locked || status.installing} onchange={(e) => onchange({ model: value(e) })}>
          {#each status.models as m (m.id)}
            <option value={m.id}>{$t(`dictation.model_${m.id.replace("whisper-", "")}`)} · {formatBytes(m.bytes)}{m.downloaded ? "" : ` · ${$t("dictation.model_not_downloaded")}`}</option>
          {/each}
        </select>
      </label>
      {#if status.installing && modelProgress}
        <div class="dc-set-progress">
          <div class="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} aria-label={$t("dictation.model_downloading") as string}>
            <div class="progress-fill" style:width="{percent}%"></div>
          </div>
          <span class="field-hint">{modelProgress.stage === "runtime" ? $t("dictation.runtime_downloading", { percent }) : $t("dictation.model_downloading_pct", { percent })}</span>
        </div>
      {:else if selectedModel && !(selectedModel.downloaded && status.model_ready)}
        <div class="dc-set-inline">
          <span class="field-hint">{$t("dictation.model_needed", { size: formatBytes(selectedModel.bytes) })}</span>
          <button type="button" class="btn btn-secondary btn-sm" disabled={status.installing || (!status.runtime_installed && !status.runtime_downloadable)} onclick={() => ondownloadmodel(selectedModel.id)}>
            <Icon name="download" size={12} />
            {$t("dictation.model_download")}
          </button>
        </div>
      {:else}
        <span class="field-hint dc-set-ok"><Icon name="done" size={12} /> {$t("dictation.model_ready")}</span>
      {/if}
    </div>
  </section>

  <section class="dc-set-group" aria-labelledby="dc-set-output">
    <h3 id="dc-set-output" class="dc-set-title">{$t("dictation.set_output")}</h3>
    <div class="dc-set-card">
      <div class="segmented" role="radiogroup" aria-labelledby="dc-set-output">
        {#each ["paste", "copy"] as const as o (o)}
          <button type="button" class="segmented-btn" role="radio" aria-checked={settings.output === o} disabled={locked} onclick={() => onchange({ output: o })}>
            {$t(`dictation.output_${o}`)}
          </button>
        {/each}
      </div>
      <span class="field-hint">{$t(`dictation.output_${settings.output}_hint`)}</span>
      {#if settings.output === "paste" && !status.paste_supported}
        <p class="dc-set-note"><Icon name="warning" size={13} /> {$t("dictation.paste_linux_missing")}</p>
      {/if}
      {#if settings.output === "paste"}
        <label class="dc-set-check">
          <input type="checkbox" class="checkbox" checked={settings.restore_clipboard} disabled={locked} onchange={(e) => onchange({ restore_clipboard: (e.currentTarget as HTMLInputElement).checked })} />
          <span>
            <span class="field-label">{$t("dictation.restore_clipboard")}</span>
            <span class="field-hint">{$t("dictation.restore_clipboard_hint")}</span>
          </span>
        </label>
      {/if}
    </div>
  </section>

  <section class="dc-set-group" aria-labelledby="dc-set-text">
    <h3 id="dc-set-text" class="dc-set-title">{$t("dictation.set_text")}</h3>
    <div class="dc-set-card">
      <label class="dc-set-check">
        <input type="checkbox" class="checkbox" checked={settings.capitalize} disabled={locked} onchange={(e) => onchange({ capitalize: (e.currentTarget as HTMLInputElement).checked })} />
        <span>
          <span class="field-label">{$t("dictation.capitalize")}</span>
          <span class="field-hint">{$t("dictation.capitalize_hint")}</span>
        </span>
      </label>
      <label class="dc-set-check">
        <input type="checkbox" class="checkbox" checked={settings.remove_fillers} disabled={locked} onchange={(e) => onchange({ remove_fillers: (e.currentTarget as HTMLInputElement).checked })} />
        <span>
          <span class="field-label">{$t("dictation.fillers")}</span>
          <span class="field-hint">{$t("dictation.fillers_hint")}</span>
        </span>
      </label>
      <div class="dc-set-field">
        <span class="field-label">{$t("dictation.replacements")}</span>
        <span class="field-hint">{$t("dictation.replacements_hint")}</span>
        <ReplacementsEditor rules={settings.replacements} disabled={locked} onchange={(replacements) => onchange({ replacements })} />
      </div>
      <label class="dc-set-field">
        <span class="field-label">{$t("dictation.preview")}</span>
        <input class="input" type="text" bind:value={previewInput} placeholder={$t("dictation.preview_ph") as string} />
      </label>
      {#if previewInput.trim()}
        <p class="dc-set-preview" aria-live="polite"><span class="dc-set-preview-label">{$t("dictation.preview_result")}</span> {previewOutput}</p>
      {/if}
    </div>
  </section>

  <section class="dc-set-group" aria-labelledby="dc-set-limits">
    <h3 id="dc-set-limits" class="dc-set-title">{$t("dictation.set_limits")}</h3>
    <div class="dc-set-card">
      <label class="dc-set-field">
        <span class="field-label">{$t("dictation.max_length")}</span>
        <select class="input dc-set-select" value={String(settings.max_seconds)} disabled={locked} onchange={(e) => onchange({ max_seconds: Number(value(e)) })}>
          {#each LIMITS as s (s)}
            <option value={String(s)}>{limitLabel(s)}</option>
          {/each}
          {#if !LIMITS.includes(settings.max_seconds)}
            <option value={String(settings.max_seconds)}>{limitLabel(settings.max_seconds)}</option>
          {/if}
        </select>
        <span class="field-hint">{$t("dictation.max_length_hint")}</span>
      </label>
      <label class="dc-set-check">
        <input type="checkbox" class="checkbox" checked={settings.save_history} disabled={locked} onchange={(e) => onchange({ save_history: (e.currentTarget as HTMLInputElement).checked })} />
        <span>
          <span class="field-label">{$t("dictation.save_history")}</span>
          <span class="field-hint">{$t("dictation.save_history_hint")}</span>
        </span>
      </label>
    </div>
  </section>
</div>

<style>
  .dc-set {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .dc-set-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .dc-set-title {
    margin: 0;
    padding: 0 var(--space-1);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: var(--track-caps);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .dc-set-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .dc-set-line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .dc-set-label,
  .dc-set-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .dc-set-field .segmented {
    align-self: flex-start;
  }

  .dc-set-inline {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .dc-set-inline .dc-set-select {
    flex: 1;
    min-width: 0;
  }

  .dc-set-select {
    width: 100%;
  }

  .dc-set-check {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    cursor: pointer;
  }

  .dc-set-check > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .dc-set-check .checkbox {
    margin-top: 2px;
  }

  .dc-set-note {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .dc-set-ok {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--success);
  }

  .dc-set-progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .dc-set-spin {
    width: 12px;
    height: 12px;
  }

  .dc-set-preview {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-2);
    font-size: var(--text-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .dc-set-preview-label {
    color: var(--text-dim);
  }
</style>
