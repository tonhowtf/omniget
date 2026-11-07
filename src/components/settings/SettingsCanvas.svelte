<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { canvasPrefs, watchCanvasPrefs, type CanvasPrefs } from "$lib/canvas/prefs.svelte";
  import { TERM_THEME_IDS } from "$lib/canvas/term/themes";
  import { SHORTCUT_ACTIONS, normCombo } from "$lib/canvas/interaction/keyboard";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import GeneralChoice from "./rebrand/GeneralChoice.svelte";
  import AdvancedDisclosure from "./ai/AdvancedDisclosure.svelte";
  import Subgroup from "./ai/Subgroup.svelte";
  import { canvasSaving, saveCanvas } from "./ai/canvas-save.svelte";
  import SettingsCanvasEnvironments from "./SettingsCanvasEnvironments.svelte";
  import SettingsCanvasImages from "./SettingsCanvasImages.svelte";
  import SettingsCanvasPortalSessions from "./SettingsCanvasPortalSessions.svelte";

  const prefs = $derived(canvasPrefs());
  const busy = $derived(canvasSaving.busy);
  const settings = $derived(getSettings());
  const worldOn = $derived(settings?.world?.enabled ?? true);
  const isMacOs = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const SHORTCUT_IDS = Object.keys(SHORTCUT_ACTIONS);
  const SIZE_KINDS = ["terminal", "note", "text", "portal"] as const;
  const hasSavedSizes = $derived(SIZE_KINDS.some((k) => !!prefs.defaults[k].size?.trim()));

  let recording = $state<string | null>(null);

  onMount(() => watchCanvasPrefs());

  function comboLabel(combo: string): string {
    if (!combo) return $t("canvas.settings.shortcutNone") as string;
    const names: Record<string, string> = isMacOs
      ? { mod: "⌘", alt: "⌥", shift: "⇧", arrowright: "→", arrowleft: "←", arrowup: "↑", arrowdown: "↓" }
      : { mod: "Ctrl+", alt: "Alt+", shift: "Shift+", arrowright: "→", arrowleft: "←", arrowup: "↑", arrowdown: "↓" };
    return combo.split("+").map((p) => names[p] ?? p.toUpperCase()).join("");
  }

  function currentCombo(id: string): string {
    return normCombo(prefs.shortcuts?.[id] ?? SHORTCUT_ACTIONS[id].combo);
  }

  function recordKey(id: string, e: KeyboardEvent) {
    if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      recording = null;
      return;
    }
    let combo = "";
    if (e.key !== "Backspace" && e.key !== "Delete") {
      const parts: string[] = [];
      if (isMacOs ? e.metaKey : e.ctrlKey) parts.push("mod");
      if (e.altKey) parts.push("alt");
      if (e.shiftKey) parts.push("shift");
      parts.push(e.key.toLowerCase());
      combo = normCombo(parts.join("+"));
    }
    recording = null;
    void saveCanvas((c) => {
      const map: Record<string, string> = { ...(c.shortcuts ?? {}) };
      if (combo) for (const other of SHORTCUT_IDS) if (other !== id && currentCombo(other) === combo) map[other] = "";
      map[id] = combo;
      c.shortcuts = map;
    });
  }

  function resetShortcut(id: string) {
    void saveCanvas((c) => {
      const map: Record<string, string> = { ...(c.shortcuts ?? {}) };
      delete map[id];
      c.shortcuts = map;
    });
  }

  function resetSizes() {
    void saveCanvas((c) => {
      for (const k of SIZE_KINDS) c.defaults[k].size = "";
    });
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void, disabled = false)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} disabled={busy || disabled} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#snippet node(x: number, y: number)}
  <rect class="node" x={x} y={y} width="26" height="18" rx="3" />
  <path class="node-line" d={`M${x + 5} ${y + 7}h14M${x + 5} ${y + 12}h9`} />
{/snippet}

{#snippet bgGrid()}
  <path class="grid" d="M14 1v48M27 1v48M40 1v48M53 1v48M66 1v48M1 13h78M1 25h78M1 37h78" />
  {@render node(27, 16)}
{/snippet}

{#snippet bgPlain()}
  {@render node(27, 16)}
{/snippet}

{#snippet bgTransparent()}
  <path class="checker" d="M1 1h13v12H1zM27 1h13v12H27zM53 1h13v12H53zM14 13h13v12H14zM40 13h13v12H40zM66 13h13v12H66zM1 25h13v12H1zM27 25h13v12H27zM53 25h13v12H53zM14 37h13v12H14zM40 37h13v12H40zM66 37h13v12H66z" />
  {@render node(27, 16)}
{/snippet}

{#snippet matOpaque()}
  <path class="stripes" d="M6 10h68M6 18h68M6 26h68M6 34h68M6 42h68" />
  <rect class="node solid" x="20" y="12" width="40" height="26" rx="4" />
{/snippet}

{#snippet matGlass()}
  <path class="stripes" d="M6 10h68M6 18h68M6 26h68M6 34h68M6 42h68" />
  <rect class="node glass" x="20" y="12" width="40" height="26" rx="4" />
{/snippet}

<SettingGroup id="canvas" icon="canvas" title={$t("canvas.settings.title") as string} description={$t("settings.ai.canvas_desc") as string}>
  <SettingRow id="world-enabled" label={$t("world.settings.enabled")} help={$t("world.settings.enabled_desc")} settingKey="world.settings.enabled" status="world.enabled">
    {#snippet control({ id, describedBy })}
      <button {id} type="button" class="toggle" class:on={worldOn} role="switch" aria-checked={worldOn} aria-labelledby={`${id}-label`} aria-describedby={describedBy} onclick={() => updateSettings({ world: { enabled: !worldOn } })}><span class="toggle-knob"></span></button>
    {/snippet}
  </SettingRow>

  <div class="block" data-setting-key="canvas.settings.background">
    <span class="block-label">{$t("canvas.settings.background")}</span>
    <GeneralChoice
      name="canvas-background"
      label={$t("canvas.settings.background") as string}
      value={prefs.background}
      options={[
        { value: "grid", draw: bgGrid, label: $t("canvas.settings.bgGrid") as string, help: $t("settings.ai.bg_grid_help") as string },
        { value: "plain", draw: bgPlain, label: $t("canvas.settings.bgPlain") as string, help: $t("settings.ai.bg_plain_help") as string },
        { value: "transparent", draw: bgTransparent, label: $t("canvas.settings.bgTransparent") as string, help: $t("settings.ai.bg_transparent_help") as string },
      ]}
      onchange={(v) => saveCanvas((c) => (c.background = v as CanvasPrefs["background"]))}
    />
  </div>

  <div class="block" data-setting-key="canvas.settings.material">
    <span class="block-label">{$t("canvas.settings.material")}</span>
    <GeneralChoice
      name="canvas-material"
      label={$t("canvas.settings.material") as string}
      value={prefs.node_material}
      options={[
        { value: "opaque", draw: matOpaque, label: $t("canvas.settings.matOpaque") as string, help: $t("settings.ai.mat_opaque_help") as string },
        { value: "glass", draw: matGlass, label: $t("canvas.settings.matGlass") as string, help: $t("settings.ai.mat_glass_help") as string },
      ]}
      onchange={(v) => saveCanvas((c) => (c.node_material = v as CanvasPrefs["node_material"]))}
    />
  </div>

  <SettingRow id="canvas-zoom-modifier" label={$t("canvas.settings.zoomModifier")} settingKey="canvas.settings.zoomModifier">
    {#snippet control({ id, describedBy })}
      <select {id} class="input-text select" aria-describedby={describedBy} disabled={busy} value={prefs.zoom_modifier}
        onchange={(e) => saveCanvas((c) => (c.zoom_modifier = e.currentTarget.value as CanvasPrefs["zoom_modifier"]))}>
        <option value="alt">{$t("canvas.settings.modAlt")}</option>
        <option value="ctrl">{$t("canvas.settings.modCtrl")}</option>
        <option value="cmd">{$t("canvas.settings.modCmd")}</option>
      </select>
    {/snippet}
  </SettingRow>
  <SettingRow id="canvas-right-drag-pan" label={$t("canvas.settings.rightDragPan")} settingKey="canvas.settings.rightDragPan">
    {#snippet control({ id, describedBy })}
      {@render toggle(id, describedBy, prefs.right_drag_pan, () => saveCanvas((c) => (c.right_drag_pan = !c.right_drag_pan)))}
    {/snippet}
  </SettingRow>
  <SettingRow id="canvas-focus-on-hover" label={$t("canvas.settings.focusOnHover")} settingKey="canvas.settings.focusOnHover">
    {#snippet control({ id, describedBy })}
      {@render toggle(id, describedBy, prefs.focus_on_hover, () => saveCanvas((c) => (c.focus_on_hover = !c.focus_on_hover)))}
    {/snippet}
  </SettingRow>

  <AdvancedDisclosure id="canvas">
    <Subgroup id="canvas-look" title={$t("settings.ai.canvas_look") as string}>
      <SettingRow id="canvas-selection-style" label={$t("canvas.settings.selectionStyle")} settingKey="canvas.settings.selectionStyle">
        {#snippet control({ id, describedBy })}
          <select {id} class="input-text select" aria-describedby={describedBy} disabled={busy} value={prefs.selection_style}
            onchange={(e) => saveCanvas((c) => (c.selection_style = e.currentTarget.value as CanvasPrefs["selection_style"]))}>
            <option value="solid">{$t("canvas.settings.selSolid")}</option>
            <option value="dashed">{$t("canvas.settings.selDashed")}</option>
            <option value="corners">{$t("canvas.settings.selCorners")}</option>
            <option value="corner_dots">{$t("canvas.settings.selCornerDots")}</option>
            <option value="elevation">{$t("canvas.settings.selElevation")}</option>
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-cord-style" label={$t("canvas.settings.cordStyle")} settingKey="canvas.settings.cordStyle">
        {#snippet control({ id, describedBy })}
          <select {id} class="input-text select" aria-describedby={describedBy} disabled={busy} value={prefs.cord_style}
            onchange={(e) => saveCanvas((c) => (c.cord_style = e.currentTarget.value as CanvasPrefs["cord_style"]))}>
            <option value="bezier">{$t("canvas.settings.cordCurve")}</option>
            <option value="rope">{$t("canvas.settings.cordRope")}</option>
            <option value="circuit">{$t("canvas.settings.cordCircuit")}</option>
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-zoom-step" label={$t("canvas.settings.zoomStep")} help={$t("canvas.settings.zoomStepHint")} settingKey="canvas.settings.zoomStep">
        {#snippet control({ id, describedBy })}
          <select {id} class="input-text select" aria-describedby={describedBy} disabled={busy} value={String(prefs.zoom_step)}
            onchange={(e) => saveCanvas((c) => (c.zoom_step = Number(e.currentTarget.value)))}>
            {#each [0.05, 0.1, 0.15, 0.2, 0.25, 0.5] as v (v)}
              <option value={String(v)}>{Math.round(v * 100)}%</option>
            {/each}
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-spell-check" label={$t("canvas.settings.spellCheck")} settingKey="canvas.settings.spellCheck">
        {#snippet control({ id, describedBy })}
          {@render toggle(id, describedBy, prefs.spell_check, () => saveCanvas((c) => (c.spell_check = !c.spell_check)))}
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-always-copy" label={$t("canvas.settings.alwaysCopyFiles")} settingKey="canvas.settings.alwaysCopyFiles">
        {#snippet control({ id, describedBy })}
          {@render toggle(id, describedBy, prefs.always_copy_files, () => saveCanvas((c) => (c.always_copy_files = !c.always_copy_files)))}
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-note-font" label={$t("canvas.settings.noteFont")} settingKey="canvas.settings.noteFont">
        {#snippet control({ id, describedBy })}
          <input {id} class="input-text" placeholder="Georgia" aria-describedby={describedBy} disabled={busy} value={prefs.defaults.note.font}
            onchange={(e) => saveCanvas((c) => (c.defaults.note.font = e.currentTarget.value.trim()))} />
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-note-render" label={$t("canvas.settings.noteRender")} settingKey="canvas.settings.noteRender">
        {#snippet control({ id, describedBy })}
          <select {id} class="input-text select" aria-describedby={describedBy} disabled={busy} value={prefs.defaults.note.render}
            onchange={(e) => saveCanvas((c) => (c.defaults.note.render = e.currentTarget.value as "formatted" | "raw"))}>
            <option value="formatted">{$t("canvas.notes.formatted")}</option>
            <option value="raw">{$t("canvas.notes.raw")}</option>
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-reset-sizes" label={$t("settings.ai.sizes_label")} help={$t("settings.ai.sizes_help")} settingKey="settings.ai.sizes_label"
        disabledReason={hasSavedSizes ? undefined : ($t("settings.ai.sizes_none") as string)}>
        {#snippet control({ id, describedBy })}
          <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} disabled={busy || !hasSavedSizes} onclick={resetSizes}>{$t("settings.ai.sizes_reset")}</button>
        {/snippet}
      </SettingRow>
    </Subgroup>

    <Subgroup id="canvas-terminal" title={$t("canvas.settings.terminalTitle") as string}>
      <SettingRow id="canvas-term-theme" label={$t("canvas.settings.termTheme")} settingKey="canvas.settings.termTheme">
        {#snippet control({ id, describedBy })}
          <select {id} class="input-text select" aria-describedby={describedBy} disabled={busy} value={prefs.terminal.theme}
            onchange={(e) => saveCanvas((c) => (c.terminal.theme = e.currentTarget.value))}>
            {#each TERM_THEME_IDS as theme (theme)}
              <option value={theme}>{theme === "system" ? $t("canvas.settings.termThemeSystem") : theme}</option>
            {/each}
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-term-font" label={$t("canvas.settings.termFont")} settingKey="canvas.settings.termFont">
        {#snippet control({ id, describedBy })}
          <input {id} class="input-text" placeholder="JetBrains Mono" aria-describedby={describedBy} disabled={busy} value={prefs.terminal.font_family}
            onchange={(e) => saveCanvas((c) => (c.terminal.font_family = e.currentTarget.value.trim()))} />
        {/snippet}
      </SettingRow>
      <SettingRow id="canvas-term-padding" label={$t("canvas.settings.termPadding")} settingKey="canvas.settings.termPadding">
        {#snippet control({ id, describedBy })}
          <div class="pair">
            <input {id} class="input-text num" type="number" min="0" max="64" aria-describedby={describedBy} disabled={busy} value={prefs.terminal.padding.x}
              onchange={(e) => saveCanvas((c) => (c.terminal.padding.x = Number(e.currentTarget.value) || 0))} />
            <input class="input-text num" type="number" min="0" max="64" aria-label={$t("canvas.settings.termPaddingY") as string} disabled={busy} value={prefs.terminal.padding.y}
              onchange={(e) => saveCanvas((c) => (c.terminal.padding.y = Number(e.currentTarget.value) || 0))} />
          </div>
        {/snippet}
      </SettingRow>
      {#if isMacOs || prefs.terminal.option_as_meta}
        <SettingRow id="canvas-option-meta" label={$t("canvas.settings.optionAsMeta")} settingKey="canvas.settings.optionAsMeta">
          {#snippet control({ id, describedBy })}
            {@render toggle(id, describedBy, prefs.terminal.option_as_meta, () => saveCanvas((c) => (c.terminal.option_as_meta = !c.terminal.option_as_meta)))}
          {/snippet}
        </SettingRow>
      {/if}
    </Subgroup>

    <Subgroup id="canvas-shortcuts" title={$t("canvas.settings.shortcutsTitle") as string} intro={$t("canvas.settings.shortcutsIntro") as string} settingKey="canvas.settings.shortcutsTitle">
      {#each SHORTCUT_IDS as sid (sid)}
        <SettingRow id={`canvas-shortcut-${sid}`} label={$t(`canvas.settings.shortcut.${sid}`, { default: sid })}>
          {#snippet control({ id, describedBy })}
            <div class="pair">
              <button {id} type="button" class="input-text mono shortcut" class:rec={recording === sid} aria-describedby={describedBy} disabled={busy}
                onclick={() => (recording = sid)}
                onblur={() => recording === sid && (recording = null)}
                onkeydown={(e) => recording === sid && recordKey(sid, e)}>
                {recording === sid ? $t("canvas.settings.shortcutRecording") : comboLabel(currentCombo(sid))}
              </button>
              {#if prefs.shortcuts && sid in prefs.shortcuts}
                <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => resetShortcut(sid)}>{$t("canvas.settings.shortcutReset")}</button>
              {/if}
            </div>
          {/snippet}
        </SettingRow>
      {/each}
    </Subgroup>

    <SettingsCanvasImages />
    <SettingsCanvasEnvironments />
    <SettingsCanvasPortalSessions />
  </AdvancedDisclosure>
</SettingGroup>

<style>
  .block { padding-block: var(--space-2); border-block-start: 1px solid var(--separator); }
  .block-label { display: block; font-size: var(--text-base); line-height: 20px; color: var(--text); }
  .pair { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: center; justify-content: flex-end; }
  .num { inline-size: 80px; }
  .mono { font-family: var(--font-mono); }
  .shortcut { min-inline-size: 120px; text-align: center; cursor: pointer; }
  .shortcut.rec { outline: 2px solid var(--accent); }
  .grid { fill: none; stroke: var(--content-border); }
  .checker { fill: var(--content-border); opacity: 0.6; }
  .stripes { fill: none; stroke: var(--accent); stroke-width: 2; stroke-linecap: round; opacity: 0.5; }
  .node { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .node.solid { fill: var(--primary); }
  .node.glass { fill: var(--surface-hi-src, var(--surface-src)); fill-opacity: 0.45; }
  .node-line { fill: none; stroke: var(--tertiary); stroke-width: 2; stroke-linecap: round; opacity: 0.6; }
</style>
