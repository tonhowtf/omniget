<script lang="ts">
  import { t } from "$lib/i18n";
  import { PRESETS as WORKSPACE_PRESETS } from "$lib/stores/workspace-design.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import WorkspaceAppearance from "./WorkspaceAppearance.svelte";
  import { getSettings, saveSettings as updateSettings, MORE_THEMES } from "./settings-helpers";

  const OMNIGET_PRESET = "omniget-desktop";
  const MODES = [
    { id: "system", labelKey: "settings.rebrand.appearance.system" },
    { id: "light", labelKey: "settings.rebrand.appearance.light" },
    { id: "dark", labelKey: "settings.rebrand.appearance.dark" },
  ] as const;

  let settings = $derived(getSettings());
  let mode = $derived(settings?.appearance?.mode ?? "system");
  let preset = $derived(settings?.appearance?.preset ?? OMNIGET_PRESET);
  let knownPreset = $derived(
    preset === OMNIGET_PRESET || WORKSPACE_PRESETS.some((p) => p.id === preset) || MORE_THEMES.some((p) => p.id === preset),
  );
  let isWorkspacePreset = $derived(WORKSPACE_PRESETS.some((p) => p.id === preset));
  let systemDark = $state(false);
  $effect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const sync = () => { systemDark = media.matches; };
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  });
  let resolvedMode = $derived<"light" | "dark">(mode === "system" ? (systemDark ? "dark" : "light") : mode);

  function setMode(next: string) {
    if (next !== mode) void updateSettings({ appearance: { mode: next } });
  }

  function setPreset(e: Event) {
    const select = e.target as HTMLSelectElement;
    const next = select.value;
    void updateSettings({ appearance: { preset: next } }).then((saved) => {
      if (!saved) select.value = getSettings()?.appearance?.preset ?? OMNIGET_PRESET;
    });
  }

  function onModeKey(e: KeyboardEvent) {
    const keys = ["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp"];
    if (!keys.includes(e.key)) return;
    e.preventDefault();
    const rtl = (e.currentTarget as HTMLElement).closest("[dir=rtl]") !== null;
    const forward = e.key === "ArrowDown" || (e.key === "ArrowRight") !== rtl;
    const index = MODES.findIndex((m) => m.id === mode);
    const next = MODES[(index + (forward ? 1 : MODES.length - 1)) % MODES.length];
    setMode(next.id);
    queueMicrotask(() => document.getElementById(`settings-appearance-mode-${next.id}`)?.focus());
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#if settings}
  <SettingGroup id="theme" title={$t("settings.appearance.theme")}>
    <div class="setting-mode-row" data-setting-key="settings.appearance.theme">
      <span class="mode-label" id="settings-appearance-mode-label">{$t("settings.appearance.theme")}</span>
      <div class="segmented" role="radiogroup" aria-labelledby="settings-appearance-mode-label" tabindex="-1" onkeydown={onModeKey}>
        {#each MODES as item (item.id)}
          <button
            type="button"
            role="radio"
            id={`settings-appearance-mode-${item.id}`}
            data-setting-key={`design.${item.id}`}
            aria-checked={mode === item.id}
            tabindex={mode === item.id ? 0 : -1}
            onclick={() => setMode(item.id)}
          >{$t(item.labelKey)}</button>
        {/each}
      </div>
    </div>

    <SettingRow id="settings-appearance-style" label={$t("design.title")} settingKey="settings.appearance.more_themes" status="appearance.preset">
      {#snippet control({ id, describedBy })}
        <select {id} class="select" aria-describedby={describedBy} value={preset} onchange={setPreset}>
          <option value={OMNIGET_PRESET}>OmniGet</option>
          {#if !knownPreset}<option value={preset}>{preset}</option>{/if}
          <optgroup label={$t("settings.appearance.more_themes")}>
            {#each WORKSPACE_PRESETS as item (item.id)}
              <option value={item.id}>{item.name}</option>
            {/each}
            {#each MORE_THEMES as item (item.id)}
              <option value={item.id}>{item.label}</option>
            {/each}
          </optgroup>
        </select>
      {/snippet}
    </SettingRow>
    {#if isWorkspacePreset}
      <WorkspaceAppearance {preset} mode={resolvedMode} />
    {/if}
  </SettingGroup>

  <SettingGroup id="accessibility" title={$t("settings.appearance.accessibility_title")}>
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
{/if}

<style>
  .setting-mode-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2) var(--space-3);
    min-block-size: 40px;
    padding-block: var(--space-2);
  }
  .mode-label { font-size: 13px; line-height: 20px; color: var(--text); }
  .segmented { display: inline-flex; flex-wrap: wrap; gap: var(--space-1); }
  .segmented button {
    min-block-size: 32px;
    padding-inline: var(--space-3);
    border: 1px solid var(--separator);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .segmented button[aria-checked="true"] { border-color: var(--accent); background: var(--accent-soft); color: var(--accent-text, var(--accent)); }
  .segmented button:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .setting-mode-row + :global(.setting-row) { border-block-start: 1px solid var(--separator); }
</style>
