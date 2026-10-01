<script lang="ts">
  import type { TypographySettings } from "$lib/stores/settings-store.svelte";
  import { createSettingsDraft } from "$lib/stores/settings-drafts.svelte";
  import { t } from "$lib/i18n";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsSaveBar from "./rebrand/SettingsSaveBar.svelte";

  type Preset = {
    id: string;
    nameKey: string | null;
    descKey: string | null;
    values: TypographySettings;
  };

  type FontOption = {
    value: string;
    label: string | null;
    family: "display" | "body" | "mono" | "any";
  };

  const PRESETS: Preset[] = [
    {
      id: "omniget-desktop",
      nameKey: null,
      descKey: null,
      values: {
        font_display: "system-ui",
        font_body: "system-ui",
        font_mono: "ui-monospace",
        line_height_base: 1.55,
        spacing_scale: 1.0,
        preset_name: "omniget-desktop",
      },
    },
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
    { value: "system-ui", label: null, family: "any" },
    { value: "ui-monospace", label: null, family: "mono" },
    { value: "Bricolage Grotesque Variable", label: "Bricolage Grotesque", family: "display" },
    { value: "Inter", label: "Inter", family: "any" },
    { value: "DM Sans", label: "DM Sans", family: "any" },
    { value: "IBM Plex Mono", label: "IBM Plex Mono", family: "any" },
  ];
  const CUSTOM = "__custom__";

  const FALLBACK: TypographySettings = PRESETS[0].values;

  const draft = createSettingsDraft<TypographySettings & Record<string, unknown>>({
    id: "typography",
    read: (s) => ({ ...FALLBACK, ...(s.typography ?? {}) }),
    toPatch: (d) => ({
      typography: {
        font_display: d.font_display,
        font_body: d.font_body,
        font_mono: d.font_mono,
        line_height_base: d.line_height_base,
        spacing_scale: d.spacing_scale,
        preset_name: d.preset_name,
      },
    }),
  });

  let typo = $derived(draft.value);
  let customOpen = $state<Record<"font_display" | "font_body" | "font_mono", boolean>>({ font_display: false, font_body: false, font_mono: false });

  function sameValues(a: TypographySettings, b: TypographySettings): boolean {
    return a.font_display === b.font_display && a.font_body === b.font_body && a.font_mono === b.font_mono
      && Math.abs(a.line_height_base - b.line_height_base) < 0.001 && Math.abs(a.spacing_scale - b.spacing_scale) < 0.001;
  }

  let activePresetId = $derived(PRESETS.find((p) => sameValues(p.values, typo))?.id ?? null);

  function optionsFor(field: "font_display" | "font_body" | "font_mono"): FontOption[] {
    if (field === "font_mono") return FONT_OPTIONS;
    return FONT_OPTIONS.filter((f) => f.family !== "mono" && (field === "font_display" || f.family !== "display"));
  }

  function isKnown(field: "font_display" | "font_body" | "font_mono", value: string): boolean {
    return optionsFor(field).some((o) => o.value === value);
  }

  function fontStack(value: string, kind: "sans" | "mono"): string {
    if (value === "system-ui") return "system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";
    if (value === "ui-monospace") return "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";
    const generic = kind === "mono" ? "ui-monospace, monospace" : "system-ui, sans-serif";
    return `'${value.replace(/'/g, "")}', ${generic}`;
  }

  function applyPreset(preset: Preset) {
    Object.assign(draft.value, preset.values);
    customOpen = { font_display: false, font_body: false, font_mono: false };
  }

  function setField<K extends keyof TypographySettings>(key: K, value: TypographySettings[K]) {
    (draft.value as TypographySettings)[key] = value;
    draft.value.preset_name = null;
  }

  function onFontSelect(field: "font_display" | "font_body" | "font_mono", e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    if (value === CUSTOM) {
      customOpen[field] = true;
      return;
    }
    customOpen[field] = false;
    setField(field, value);
  }

  function onCustomInput(field: "font_display" | "font_body" | "font_mono", e: Event) {
    const value = (e.target as HTMLInputElement).value.trim();
    if (value) setField(field, value);
  }

  const FONT_FIELDS: { field: "font_display" | "font_body" | "font_mono"; labelKey: string; kind: "sans" | "mono" }[] = [
    { field: "font_display", labelKey: "settings.typography.display_label", kind: "sans" },
    { field: "font_body", labelKey: "settings.typography.body_label", kind: "sans" },
    { field: "font_mono", labelKey: "settings.typography.mono_label", kind: "mono" },
  ];

  let previewStyle = $derived(
    `--font-display: ${fontStack(typo.font_display, "sans")}; --font-body: ${fontStack(typo.font_body, "sans")}; --font-mono: ${fontStack(typo.font_mono, "mono")}; line-height: ${typo.line_height_base};`,
  );
</script>

<div class="typography-form">
  <div class="presets" role="group" aria-label={$t("settings.typography.presets_title")} data-setting-key="settings.typography.presets_title">
    <p class="presets-title">{$t("settings.typography.presets_title")}</p>
    <p class="presets-desc">{$t("settings.typography.presets_desc")}</p>
    <div class="presets-grid">
      {#each PRESETS as preset (preset.id)}
        {@const isActive = activePresetId === preset.id}
        <button type="button" class="preset-card" class:active={isActive} aria-pressed={isActive} onclick={() => applyPreset(preset)}>
          <span class="preset-name">{preset.nameKey ? $t(preset.nameKey) : "OmniGet"}</span>
          <span class="preset-sample" style:font-family={fontStack(preset.values.font_display, "sans")}>Aa Bb 123</span>
          {#if preset.descKey}<span class="preset-desc">{$t(preset.descKey)}</span>{/if}
        </button>
      {/each}
    </div>
  </div>

  <div data-setting-key="settings.typography.fonts_title">
    {#each FONT_FIELDS as item (item.field)}
      {@const value = typo[item.field]}
      {@const custom = customOpen[item.field] || !isKnown(item.field, value)}
      <SettingRow id={`settings-typography-${item.field}`} label={$t(item.labelKey)} help={item.field === "font_display" ? $t("settings.typography.fonts_desc") : undefined} settingKey={item.labelKey}>
        {#snippet control({ id, describedBy })}
          <select {id} class="select" aria-describedby={describedBy} value={custom ? CUSTOM : value} onchange={(e) => onFontSelect(item.field, e)}>
            {#each optionsFor(item.field) as opt (opt.value)}
              <option value={opt.value}>{opt.label ?? $t("settings.appearance.theme_system")}</option>
            {/each}
            <option value={CUSTOM}>{$t("settings.rebrand.appearance.custom_font")}</option>
          </select>
        {/snippet}
      </SettingRow>
      {#if custom}
        <SettingRow id={`settings-typography-${item.field}-custom`} label={$t("settings.rebrand.appearance.custom_font")} help={$t("settings.rebrand.appearance.font_hint")}>
          {#snippet control({ id, describedBy })}
            <input {id} type="text" class="input-text" dir="ltr" spellcheck="false" aria-describedby={describedBy}
              value={isKnown(item.field, value) ? "" : value} oninput={(e) => onCustomInput(item.field, e)} />
          {/snippet}
        </SettingRow>
      {/if}
    {/each}
  </div>

  <SettingRow id="settings-typography-line-height" label={$t("settings.typography.line_height_label")} help={$t("settings.typography.line_height_desc")} settingKey="settings.typography.line_height_label">
    {#snippet control({ id, describedBy })}
      <span class="range-field">
        <input {id} type="range" min="1.3" max="1.9" step="0.05" aria-describedby={describedBy}
          value={typo.line_height_base} oninput={(e) => setField("line_height_base", parseFloat((e.target as HTMLInputElement).value))} />
        <output for={id} dir="ltr">{typo.line_height_base.toFixed(2)}</output>
      </span>
    {/snippet}
  </SettingRow>

  <SettingRow id="settings-typography-spacing" label={$t("settings.typography.spacing_label")} help={$t("settings.typography.spacing_desc")} settingKey="settings.typography.spacing_label">
    {#snippet control({ id, describedBy })}
      <span class="range-field">
        <input {id} type="range" min="0.85" max="1.25" step="0.05" aria-describedby={describedBy}
          value={typo.spacing_scale} oninput={(e) => setField("spacing_scale", parseFloat((e.target as HTMLInputElement).value))} />
        <output for={id} dir="ltr">{typo.spacing_scale.toFixed(2)}×</output>
      </span>
    {/snippet}
  </SettingRow>

  <div class="preview" data-setting-key="settings.typography.preview_title">
    <p class="presets-title">{$t("settings.typography.preview_title")}</p>
    <p class="presets-desc">{$t("settings.typography.preview_desc")}</p>
    <div class="preview-pane" style={previewStyle}>
      <h3 class="preview-h1">Aula 03 · Anatomia do coração</h3>
      <h4 class="preview-h2">Conceitos chave</h4>
      <p class="preview-body">
        O coração tem <span class="preview-link">[[câmaras cardíacas]]</span>
        divididas em átrios e ventrículos. O ventrículo esquerdo é mais musculoso
        porque bombeia sangue contra a resistência sistêmica.
      </p>
      <pre class="preview-code" dir="ltr"><code>{`def calcular_fc_max(idade: int) -> int:
    return 220 - idade`}</code></pre>
    </div>
  </div>

  <SettingsSaveBar
    dirty={draft.dirty}
    saving={draft.saving}
    error={draft.error}
    onSave={() => { void draft.save(); }}
    onDiscard={() => { draft.discard(); customOpen = { font_display: false, font_body: false, font_mono: false }; }}
  />
</div>

<style>
  .typography-form { display: flex; flex-direction: column; min-inline-size: 0; }
  .presets, .preview { padding-block: var(--space-2); }
  .presets-title { margin: 0; font-size: 13px; line-height: 20px; color: var(--text); }
  .presets-desc { margin: 2px 0 0; font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .presets-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 150px), 1fr));
    gap: var(--space-2);
    margin-block-start: var(--space-2);
  }
  .preset-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-block-size: 36px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--separator);
    border-radius: 6px;
    background: var(--surface);
    color: inherit;
    text-align: start;
    font: inherit;
    cursor: pointer;
  }
  .preset-card:hover { background: var(--surface-hi); }
  .preset-card.active { border-color: var(--accent); background: var(--accent-soft); }
  .preset-card:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .preset-name { font-size: 13px; font-weight: 600; }
  .preset-sample { font-size: 16px; }
  .preset-desc { font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .range-field { display: flex; align-items: center; gap: var(--space-2); inline-size: 100%; }
  .range-field input { flex: 1 1 auto; min-inline-size: 0; }
  .range-field output { flex: none; min-inline-size: 3.5em; text-align: end; font-variant-numeric: tabular-nums; font-size: 12px; color: var(--text-muted); }
  .preview-pane {
    margin-block-start: var(--space-2);
    padding: var(--space-4);
    border: 1px solid var(--separator);
    border-radius: 10px;
    background: var(--bg);
    overflow-wrap: anywhere;
  }
  .preview-h1 { margin: 0 0 8px; font-family: var(--font-display); font-size: 22px; font-weight: 700; line-height: 1.15; }
  .preview-h2 { margin: 12px 0 6px; font-family: var(--font-display); font-size: 16px; font-weight: 600; line-height: 1.2; }
  .preview-body { margin: 0 0 8px; font-family: var(--font-body); font-size: 14px; }
  .preview-link { color: var(--accent-text, var(--accent)); text-decoration: underline; }
  .preview-code {
    margin: 8px 0 0;
    padding: 10px 12px;
    background: color-mix(in oklab, currentColor 6%, transparent);
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    overflow-x: auto;
  }
</style>
