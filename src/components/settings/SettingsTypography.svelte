<script lang="ts">
  import {
    getSettings,
    updateSettings,
    type TypographySettings,
  } from "$lib/stores/settings-store.svelte";
  import { t } from "$lib/i18n";

  type Preset = {
    id: string;
    nameKey: string;
    descKey: string;
    values: TypographySettings;
  };

  type FontOption = {
    value: string;
    label: string;
    family: "display" | "body" | "mono" | "any";
    bundled: boolean;
  };

  const PRESETS: Preset[] = [
    {
      id: "omniget-default",
      nameKey: "settings.typography.preset_omniget_default_name",
      descKey: "settings.typography.preset_omniget_default_desc",
      values: {
        font_display: "Bricolage Grotesque Variable",
        font_body: "Inter",
        font_mono: "IBM Plex Mono",
        line_height_base: 1.55,
        spacing_scale: 1.0,
        preset_name: "omniget-default",
      },
    },
    {
      id: "notion-like",
      nameKey: "settings.typography.preset_notion_like_name",
      descKey: "settings.typography.preset_notion_like_desc",
      values: {
        font_display: "Inter",
        font_body: "Inter",
        font_mono: "IBM Plex Mono",
        line_height_base: 1.5,
        spacing_scale: 1.0,
        preset_name: "notion-like",
      },
    },
    {
      id: "compact",
      nameKey: "settings.typography.preset_compact_name",
      descKey: "settings.typography.preset_compact_desc",
      values: {
        font_display: "DM Sans",
        font_body: "Inter",
        font_mono: "IBM Plex Mono",
        line_height_base: 1.45,
        spacing_scale: 0.9,
        preset_name: "compact",
      },
    },
    {
      id: "reading",
      nameKey: "settings.typography.preset_reading_name",
      descKey: "settings.typography.preset_reading_desc",
      values: {
        font_display: "Bricolage Grotesque Variable",
        font_body: "DM Sans",
        font_mono: "IBM Plex Mono",
        line_height_base: 1.7,
        spacing_scale: 1.1,
        preset_name: "reading",
      },
    },
    {
      id: "tech-minimal",
      nameKey: "settings.typography.preset_tech_minimal_name",
      descKey: "settings.typography.preset_tech_minimal_desc",
      values: {
        font_display: "IBM Plex Mono",
        font_body: "IBM Plex Mono",
        font_mono: "IBM Plex Mono",
        line_height_base: 1.5,
        spacing_scale: 1.0,
        preset_name: "tech-minimal",
      },
    },
  ];

  const FONT_OPTIONS: FontOption[] = [
    { value: "Bricolage Grotesque Variable", label: "Bricolage Grotesque", family: "display", bundled: true },
    { value: "Inter", label: "Inter", family: "any", bundled: true },
    { value: "DM Sans", label: "DM Sans", family: "any", bundled: true },
    { value: "IBM Plex Mono", label: "IBM Plex Mono", family: "any", bundled: true },
    { value: "system-ui", label: "System default", family: "any", bundled: false },
  ];

  const DEFAULTS: TypographySettings = {
    font_display: "Bricolage Grotesque Variable",
    font_body: "Inter",
    font_mono: "IBM Plex Mono",
    line_height_base: 1.55,
    spacing_scale: 1.0,
    preset_name: "omniget-default",
  };

  let settings = $derived(getSettings());
  let typo = $derived(settings?.typography ?? DEFAULTS);
  let activePresetId = $derived(typo.preset_name ?? null);
  let isModified = $derived.by(() => {
    if (!activePresetId) return true;
    const p = PRESETS.find((x) => x.id === activePresetId);
    if (!p) return true;
    return (
      p.values.font_display !== typo.font_display ||
      p.values.font_body !== typo.font_body ||
      p.values.font_mono !== typo.font_mono ||
      Math.abs(p.values.line_height_base - typo.line_height_base) > 0.001 ||
      Math.abs(p.values.spacing_scale - typo.spacing_scale) > 0.001
    );
  });

  function previewFontStack(option: FontOption): string {
    if (option.value === "system-ui") return "system-ui, sans-serif";
    return `'${option.value}', ui-sans-serif, system-ui, sans-serif`;
  }

  async function applyPreset(preset: Preset) {
    await updateSettings({ typography: preset.values });
  }

  async function patchTypo(patch: Partial<TypographySettings>) {
    await updateSettings({
      typography: {
        ...typo,
        ...patch,
        preset_name: null,
      },
    });
  }

  async function changeDisplay(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await patchTypo({ font_display: value });
  }

  async function changeBody(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await patchTypo({ font_body: value });
  }

  async function changeMono(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await patchTypo({ font_mono: value });
  }

  async function changeLineHeight(e: Event) {
    const value = parseFloat((e.target as HTMLInputElement).value);
    await patchTypo({ line_height_base: value });
  }

  async function changeSpacing(e: Event) {
    const value = parseFloat((e.target as HTMLInputElement).value);
    await patchTypo({ spacing_scale: value });
  }

  async function resetFonts() {
    await patchTypo({
      font_display: DEFAULTS.font_display,
      font_body: DEFAULTS.font_body,
      font_mono: DEFAULTS.font_mono,
    });
  }

  async function resetLineHeight() {
    await patchTypo({ line_height_base: DEFAULTS.line_height_base });
  }

  async function resetSpacing() {
    await patchTypo({ spacing_scale: DEFAULTS.spacing_scale });
  }
</script>

{#if settings}
  <section class="section">
    <h5 class="section-title">{$t("settings.typography.section_title")}</h5>

    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.typography.presets_title")}</span>
          <span class="setting-path">
            {$t("settings.typography.presets_desc")}
            {#if activePresetId && isModified}
              <em class="modified">{$t("settings.typography.modified_indicator")}</em>
            {/if}
          </span>
        </div>
      </div>

      <div class="presets-grid">
        {#each PRESETS as preset (preset.id)}
          {@const isActive = activePresetId === preset.id && !isModified}
          <button
            type="button"
            class="preset-card"
            class:active={isActive}
            onclick={() => applyPreset(preset)}
          >
            <div class="preset-head">
              <span class="preset-name">{$t(preset.nameKey)}</span>
              {#if isActive}<span class="check" aria-hidden="true">✓</span>{/if}
            </div>
            <span
              class="preset-sample"
              style:font-family="{previewFontStack({ value: preset.values.font_display, label: '', family: 'display', bundled: true })}"
            >
              Aa Bb 123
            </span>
            <span class="preset-desc">{$t(preset.descKey)}</span>
          </button>
        {/each}
      </div>
    </div>

    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.typography.fonts_title")}</span>
          <span class="setting-path">
            {$t("settings.typography.fonts_desc")}
          </span>
        </div>
        <button class="reset-btn" type="button" onclick={resetFonts}>
          {$t("settings.typography.reset")}
        </button>
      </div>

      <div class="font-row">
        <label class="font-field">
          <span class="font-label">{$t("settings.typography.display_label")}</span>
          <select value={typo.font_display} onchange={changeDisplay}>
            {#each FONT_OPTIONS.filter((f) => f.family === "display" || f.family === "any") as opt (opt.value)}
              <option value={opt.value}>{opt.label}{opt.bundled ? "" : ` (${$t("settings.dependencies.system_font_suffix")})`}</option>
            {/each}
          </select>
        </label>
        <span
          class="font-sample"
          style:font-family="{previewFontStack({ value: typo.font_display, label: '', family: 'display', bundled: true })}"
        >
          Aa Bb 123
        </span>
      </div>

      <div class="font-row">
        <label class="font-field">
          <span class="font-label">{$t("settings.typography.body_label")}</span>
          <select value={typo.font_body} onchange={changeBody}>
            {#each FONT_OPTIONS.filter((f) => f.family !== "mono") as opt (opt.value)}
              <option value={opt.value}>{opt.label}{opt.bundled ? "" : ` (${$t("settings.dependencies.system_font_suffix")})`}</option>
            {/each}
          </select>
        </label>
        <span
          class="font-sample"
          style:font-family="{previewFontStack({ value: typo.font_body, label: '', family: 'body', bundled: true })}"
        >
          {$t("settings.typography.body_sample")}
        </span>
      </div>

      <div class="font-row">
        <label class="font-field">
          <span class="font-label">{$t("settings.typography.mono_label")}</span>
          <select value={typo.font_mono} onchange={changeMono}>
            {#each FONT_OPTIONS as opt (opt.value)}
              <option value={opt.value}>{opt.label}{opt.bundled ? "" : ` (${$t("settings.dependencies.system_font_suffix")})`}</option>
            {/each}
          </select>
        </label>
        <span
          class="font-sample mono"
          style:font-family="{previewFontStack({ value: typo.font_mono, label: '', family: 'mono', bundled: true })}"
        >
          {`const x = 42;`}
        </span>
      </div>
    </div>

    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">
            {$t("settings.typography.line_height_label")} <span class="value-pill">{typo.line_height_base.toFixed(2)}</span>
          </span>
          <span class="setting-path">
            {$t("settings.typography.line_height_desc")}
          </span>
        </div>
        <button class="reset-btn" type="button" onclick={resetLineHeight}>
          {$t("settings.typography.reset")}
        </button>
      </div>
      <div class="slider-row">
        <span class="slider-edge">1.30</span>
        <input
          type="range"
          min="1.3"
          max="1.9"
          step="0.05"
          value={typo.line_height_base}
          oninput={changeLineHeight}
          aria-label={$t("settings.typography.line_height_label") as string}
        />
        <span class="slider-edge">1.90</span>
      </div>
    </div>

    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">
            {$t("settings.typography.spacing_label")} <span class="value-pill">{typo.spacing_scale.toFixed(2)}×</span>
          </span>
          <span class="setting-path">
            {$t("settings.typography.spacing_desc")}
          </span>
        </div>
        <button class="reset-btn" type="button" onclick={resetSpacing}>
          {$t("settings.typography.reset")}
        </button>
      </div>
      <div class="slider-row">
        <span class="slider-edge">0.85×</span>
        <input
          type="range"
          min="0.85"
          max="1.25"
          step="0.05"
          value={typo.spacing_scale}
          oninput={changeSpacing}
          aria-label={$t("settings.typography.spacing_label") as string}
        />
        <span class="slider-edge">1.25×</span>
      </div>
    </div>

    <div class="card preview-card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("settings.typography.preview_title")}</span>
          <span class="setting-path">
            {$t("settings.typography.preview_desc")}
          </span>
        </div>
      </div>
      <div class="preview-pane">
        <h1 class="preview-h1">Aula 03 · Anatomia do coração</h1>
        <h2 class="preview-h2">Conceitos chave</h2>
        <p class="preview-body">
          O coração tem <a class="preview-link">[[câmaras cardíacas]]</a>
          divididas em átrios e ventrículos. O ventrículo esquerdo é mais musculoso
          porque bombeia sangue contra a resistência sistêmica.
        </p>
        <ul class="preview-list">
          <li>Átrio direito recebe sangue venoso da veia cava.</li>
          <li>Ventrículo esquerdo bombeia para a aorta.</li>
        </ul>
        <pre class="preview-code"><code>{`def calcular_fc_max(idade: int) -> int:
    return 220 - idade`}</code></pre>
        <p class="preview-props">
          <span class="prop">status:: <strong>TODO</strong></span>
          <span class="prop">deadline:: 2026-05-12</span>
        </p>
      </div>
    </div>
  </section>
{/if}

<style>
  .presets-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 8px;
    margin-top: 8px;
  }
  .preset-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 14px;
    border: none;
    border-radius: 10px;
    background: var(--surface, var(--bg));
    color: inherit;
    text-align: left;
    cursor: pointer;
    transition: border-color 120ms ease, background 120ms ease;
  }
  .preset-card:hover {
    border-color: color-mix(in oklab, var(--accent) 50%, var(--content-border));
    background: color-mix(in oklab, var(--accent) 6%, transparent);
  }
  .preset-card.active {
    border-color: var(--accent);
    background: color-mix(in oklab, var(--accent) 10%, transparent);
  }
  .preset-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .preset-name {
    font-weight: 600;
    font-size: 13px;
  }
  .check {
    color: var(--accent);
    font-weight: 700;
  }
  .preset-sample {
    font-size: 22px;
    line-height: 1.1;
    color: color-mix(in oklab, currentColor 85%, transparent);
  }
  .preset-desc {
    font-size: 11px;
    line-height: 1.4;
    color: color-mix(in oklab, currentColor 60%, transparent);
  }

  .font-row {
    display: grid;
    grid-template-columns: 240px 1fr;
    gap: 16px;
    align-items: center;
    padding: 10px 0;
    border-top: none;
  }
  .font-row:first-of-type {
    border-top: 0;
  }
  .font-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .font-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: color-mix(in oklab, currentColor 60%, transparent);
    font-weight: 600;
  }
  .font-field select {
    padding: 6px 8px;
    border: none;
    border-radius: 6px;
    background: var(--bg);
    color: inherit;
    font: inherit;
    font-size: 13px;
  }
  .font-sample {
    font-size: 15px;
    color: color-mix(in oklab, currentColor 85%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .font-sample.mono {
    font-size: 13px;
  }

  .slider-row {
    display: grid;
    grid-template-columns: 48px 1fr 48px;
    align-items: center;
    gap: 12px;
    margin-top: 8px;
  }
  .slider-row input[type="range"] {
    width: 100%;
    accent-color: var(--accent);
  }
  .slider-edge {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    color: color-mix(in oklab, currentColor 55%, transparent);
    text-align: center;
  }
  .value-pill {
    margin-left: 6px;
    padding: 1px 8px;
    border-radius: 999px;
    background: color-mix(in oklab, var(--accent) 14%, transparent);
    color: var(--accent);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    font-weight: 600;
  }

  .reset-btn {
    padding: 4px 10px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    transition: background 120ms ease;
  }
  .reset-btn:hover {
    background: color-mix(in oklab, currentColor 8%, transparent);
  }

  .modified {
    color: var(--warning);
    font-style: italic;
    margin-left: 4px;
  }

  .preview-card {
    background: var(--surface, var(--bg));
  }
  .preview-pane {
    margin-top: 8px;
    padding: 16px 18px;
    border-radius: 10px;
    border: none;
    background: var(--bg);
    line-height: var(--leading-base);
  }
  .preview-h1 {
    margin: 0 0 8px;
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 700;
    line-height: 1.15;
  }
  .preview-h2 {
    margin: 16px 0 6px;
    font-family: var(--font-display);
    font-size: 18px;
    font-weight: 600;
    line-height: 1.2;
  }
  .preview-body {
    margin: 0 0 8px;
    font-family: var(--font-body);
    font-size: 14px;
  }
  .preview-link {
    color: var(--accent);
    text-decoration: underline;
    text-decoration-color: color-mix(in oklab, var(--accent) 40%, transparent);
  }
  .preview-list {
    margin: 0 0 8px;
    padding-left: 20px;
    font-family: var(--font-body);
    font-size: 14px;
  }
  .preview-list li {
    margin-bottom: 2px;
  }
  .preview-code {
    margin: 8px 0;
    padding: 10px 12px;
    background: color-mix(in oklab, currentColor 6%, transparent);
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    overflow-x: auto;
  }
  .preview-props {
    margin: 8px 0 0;
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
    font-family: var(--font-mono);
    font-size: 11px;
    color: color-mix(in oklab, currentColor 65%, transparent);
  }
  .prop strong {
    color: var(--accent);
  }
</style>
