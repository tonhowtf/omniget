<script lang="ts">
  import { t } from "$lib/i18n";
  import { getAppearancePresentation } from "$lib/stores/settings-store.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingStatus from "./rebrand/SettingStatus.svelte";
  import GeneralChoice from "./rebrand/GeneralChoice.svelte";
  import ThemeSwatches from "./appearance/ThemeSwatches.svelte";
  import FontRow from "./appearance/FontRow.svelte";
  import { densityFor, densityPatch, type Density } from "./appearance/density";
  import { getSettings, saveSettings as updateSettings, MORE_THEMES } from "./settings-helpers";

  const OMNIGET_PRESET = "omniget-desktop";
  const UI_SYSTEM_FONTS = ["system-ui", "System", "", "sans-serif", "ui-sans-serif"] as const;
  const MONO_SYSTEM_FONTS = ["ui-monospace", "System", "system-ui", "", "monospace"] as const;

  const isSystemFont = (value: string) => (UI_SYSTEM_FONTS as readonly string[]).includes(value.trim());

  let settings = $derived(getSettings());
  let presentation = $derived(getAppearancePresentation());
  let mode = $derived(settings?.appearance?.mode ?? "system");
  let preset = $derived(settings?.appearance?.preset ?? OMNIGET_PRESET);
  let fixedTheme = $derived(presentation.namedTheme === null ? null : (MORE_THEMES.find((p) => p.id === presentation.namedTheme)?.label ?? presentation.namedTheme));
  let omnigetArt = $derived(presentation.namedTheme === null && preset === OMNIGET_PRESET ? OMNIGET_PRESET : undefined);
  let density = $derived(densityFor(settings?.typography));
  let fontBody = $derived(settings?.typography?.font_body ?? "system-ui");
  let fontDisplay = $derived(settings?.typography?.font_display ?? fontBody);
  let uiFont = $derived(isSystemFont(fontBody) && !isSystemFont(fontDisplay) ? fontDisplay : fontBody);
  let fontMono = $derived(settings?.typography?.font_mono ?? "ui-monospace");

  function resync(name: string, current: string) {
    for (const input of document.querySelectorAll<HTMLInputElement>(`input[name="${name}"]`)) input.checked = input.value === current;
  }

  async function setMode(next: string) {
    if (next === mode) return;
    if ((await updateSettings({ appearance: { mode: next } })) === null) resync("settings-appearance-mode", mode);
  }

  async function setPreset(next: string): Promise<boolean> {
    return (await updateSettings({ appearance: { preset: next } })) !== null;
  }

  async function setDensity(next: string) {
    if (next === density) return;
    if ((await updateSettings(densityPatch(next as Density))) === null) resync("settings-appearance-density", density ?? "");
  }

  async function saveUiFont(value: string): Promise<boolean> {
    return (await updateSettings({ typography: { font_display: value, font_body: value } })) !== null;
  }

  async function saveMonoFont(value: string): Promise<boolean> {
    return (await updateSettings({ typography: { font_mono: value } })) !== null;
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#snippet screen(theme: "light" | "dark", d: string)}
  <g data-theme={theme} data-preset={omnigetArt}>
    <path class="scr" {d} />
  </g>
{/snippet}

{#snippet screenLines(theme: "light" | "dark", x: number, width: number)}
  <g data-theme={theme} data-preset={omnigetArt}>
    <path class="txt" d={`M${x} 18h${width}M${x} 25h${Math.round(width * 0.6)}`} />
    <rect class="pill" x={x} y="31" width={Math.round(width * 0.5)} height="5" rx="2.5" />
  </g>
{/snippet}

{#snippet drawSystem()}
  {@render screen("light", "M40 8H15a3 3 0 0 0-3 3v28a3 3 0 0 0 3 3h25z")}
  {@render screen("dark", "M40 8h25a3 3 0 0 1 3 3v28a3 3 0 0 1-3 3H40z")}
  {@render screenLines("light", 17, 18)}
  {@render screenLines("dark", 45, 18)}
  <rect class="frame" x="12" y="8" width="56" height="34" rx="3" />
{/snippet}

{#snippet drawLight()}
  {@render screen("light", "M15 8h50a3 3 0 0 1 3 3v28a3 3 0 0 1-3 3H15a3 3 0 0 1-3-3V11a3 3 0 0 1 3-3z")}
  {@render screenLines("light", 18, 36)}
  <rect class="frame" x="12" y="8" width="56" height="34" rx="3" />
{/snippet}

{#snippet drawDark()}
  {@render screen("dark", "M15 8h50a3 3 0 0 1 3 3v28a3 3 0 0 1-3 3H15a3 3 0 0 1-3-3V11a3 3 0 0 1 3-3z")}
  {@render screenLines("dark", 18, 36)}
  <rect class="frame" x="12" y="8" width="56" height="34" rx="3" />
{/snippet}

{#snippet rows(gap: number, count: number, weight: number)}
  {#each Array.from({ length: count }, (_, i) => i) as i (i)}
    <path class="row" style:stroke-width={weight} d={`M14 ${11 + i * gap}h${i % 2 ? 36 : 52}`} />
  {/each}
{/snippet}

{#snippet drawCompact()}{@render rows(6, 6, 2)}{/snippet}
{#snippet drawDefault()}{@render rows(8.5, 4, 2.5)}{/snippet}
{#snippet drawComfortable()}{@render rows(12, 3, 3)}{/snippet}

{#if settings}
  <SettingGroup id="theme" icon="appearance" title={$t("settings.appearance.theme")} description={$t("settings.appearance.mode.desc")}>
    <div data-setting-key="settings.appearance.theme">
      <GeneralChoice
        name="settings-appearance-mode"
        label={$t("settings.appearance.theme") as string}
        value={mode}
        options={[
          { value: "system", draw: drawSystem, label: $t("settings.appearance.mode.system") as string, help: $t("settings.appearance.mode.system_desc") as string },
          { value: "light", draw: drawLight, label: $t("settings.appearance.mode.light") as string, help: $t("settings.appearance.mode.light_desc") as string },
          { value: "dark", draw: drawDark, label: $t("settings.appearance.mode.dark") as string, help: $t("settings.appearance.mode.dark_desc") as string },
        ]}
        onchange={setMode}
      />
      {#if fixedTheme}
        <p class="note" role="status"><bdi>{$t("settings.appearance.mode.fixed_by_theme", { name: fixedTheme })}</bdi></p>
      {/if}
      <SettingStatus path="appearance.mode" />
    </div>
  </SettingGroup>

  <SettingGroup id="colors" icon="palette" title={$t("settings.appearance.colors.title")} description={$t("settings.appearance.colors.desc")}>
    <div data-setting-key="settings.appearance.colors.title">
      <ThemeSwatches value={preset} omnigetId={OMNIGET_PRESET} mode={presentation.resolvedMode} onchange={setPreset} />
      <SettingStatus path="appearance.preset" />
    </div>
  </SettingGroup>

  <SettingGroup id="comfort" icon="text" title={$t("settings.appearance.comfort.title")} description={$t("settings.appearance.comfort.desc")}>
    <div data-setting-key="settings.appearance.density.label">
      <GeneralChoice
        name="settings-appearance-density"
        label={$t("settings.appearance.density.label") as string}
        value={density ?? ""}
        options={[
          { value: "compact", draw: drawCompact, label: $t("settings.appearance.density.compact") as string, help: $t("settings.appearance.density.compact_desc") as string },
          { value: "default", draw: drawDefault, label: $t("settings.appearance.density.default") as string, help: $t("settings.appearance.density.default_desc") as string },
          { value: "comfortable", draw: drawComfortable, label: $t("settings.appearance.density.comfortable") as string, help: $t("settings.appearance.density.comfortable_desc") as string },
        ]}
        onchange={setDensity}
      />
      {#if density === null}
        <p class="note" role="status"><bdi>{$t("settings.appearance.density.custom")}</bdi></p>
      {/if}
      <SettingStatus path="typography.spacing_scale" />
    </div>
    <SettingRow id="settings-reduce-motion" label={$t("settings.appearance.reduce_motion")} help={$t("settings.appearance.reduce_motion_desc")}
      settingKey="settings.appearance.reduce_motion" status="accessibility.reduce_motion">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, settings.accessibility?.reduce_motion ?? false,
          () => updateSettings({ accessibility: { reduce_motion: !settings.accessibility?.reduce_motion } }))}
      {/snippet}
    </SettingRow>
    <SettingRow id="settings-reduce-transparency" label={$t("settings.appearance.reduce_transparency")} help={$t("settings.appearance.reduce_transparency_desc")}
      settingKey="settings.appearance.reduce_transparency" status="accessibility.reduce_transparency">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, settings.accessibility?.reduce_transparency ?? false,
          () => updateSettings({ accessibility: { reduce_transparency: !settings.accessibility?.reduce_transparency } }))}
      {/snippet}
    </SettingRow>
  </SettingGroup>

  <SettingGroup id="fonts" icon="more" title={$t("settings.appearance.fonts.title")} description={$t("settings.appearance.fonts.desc")} advanced>
    <FontRow
      id="settings-font-ui"
      label={$t("settings.appearance.fonts.ui") as string}
      help={$t("settings.appearance.fonts.ui_desc") as string}
      note={fontDisplay.trim() !== fontBody.trim() ? ($t("settings.appearance.fonts.split_note") as string) : undefined}
      settingKey="settings.appearance.fonts.ui"
      status="typography.font_body"
      value={uiFont}
      systemValue="system-ui"
      systemValues={UI_SYSTEM_FONTS}
      systemLabel={$t("settings.appearance.fonts.system") as string}
      onsave={saveUiFont}
    />
    <FontRow
      id="settings-font-code"
      label={$t("settings.appearance.fonts.code") as string}
      help={$t("settings.appearance.fonts.code_desc") as string}
      settingKey="settings.appearance.fonts.code"
      status="typography.font_mono"
      value={fontMono}
      systemValue="ui-monospace"
      systemValues={MONO_SYSTEM_FONTS}
      systemLabel={$t("settings.appearance.fonts.system_mono") as string}
      onsave={saveMonoFont}
    />
  </SettingGroup>
{/if}

<style>
  .note { margin: var(--space-2) 0 0; font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .scr { fill: var(--primary); }
  .txt { fill: none; stroke: var(--secondary); stroke-width: 2; stroke-linecap: round; opacity: 0.7; }
  .pill { fill: var(--accent); }
  .frame { fill: none; stroke: var(--tertiary); }
  .row { fill: none; stroke: var(--tertiary); stroke-linecap: round; opacity: 0.7; }
</style>
