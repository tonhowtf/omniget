<script lang="ts">
  import { tick } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { canvasPrefs, type AgentPreset } from "$lib/canvas/prefs.svelte";
  import SettingGroup from "$components/settings/rebrand/SettingGroup.svelte";
  import SettingsEmptyState from "$components/settings/rebrand/SettingsEmptyState.svelte";
  import AdvancedDisclosure from "./AdvancedDisclosure.svelte";
  import { canvasSaving, saveCanvas } from "./canvas-save.svelte";
  import { isBuiltinAgent } from "./ai-settings";

  type Draft = { name: string; command: string; args: string; env: string; icon: string; color: string; maestro: boolean; chat: boolean; cwd: string };
  const EMPTY: Draft = { name: "", command: "", args: "", env: "", icon: "", color: "", maestro: false, chat: false, cwd: "" };

  const prefs = $derived(canvasPrefs());
  const busy = $derived(canvasSaving.busy);
  const names = $derived(Object.keys(prefs.agent_presets));

  let draft = $state<Draft>({ ...EMPTY });
  let editing = $state<string | null>(null);
  let open = $state(false);
  let confirming = $state<string | null>(null);
  let missing = $state(false);
  let nameClash = $state<"" | "taken" | "builtin">("");

  async function startEditor(name: string | null) {
    editing = name;
    missing = false;
    nameClash = "";
    const p = name ? prefs.agent_presets[name] : null;
    draft = p
      ? {
          name: name!,
          command: p.command,
          args: (p.args ?? []).join(" "),
          env: Object.entries(p.env ?? {}).map(([k, v]) => `${k}=${v}`).join("\n"),
          icon: p.icon ?? "",
          color: p.color ?? "",
          maestro: !!p.maestro,
          chat: !!p.start_in_chat_mode,
          cwd: p.cwd ?? "",
        }
      : { ...EMPTY };
    open = true;
    await tick();
    document.getElementById("preset-name")?.focus();
  }

  function close() {
    open = false;
    editing = null;
    missing = false;
    nameClash = "";
    draft = { ...EMPTY };
  }

  function parseEnv(s: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of s.split(/\n/)) {
      const i = line.indexOf("=");
      if (i > 0) out[line.slice(0, i).trim()] = line.slice(i + 1).trim();
    }
    return out;
  }

  async function savePreset() {
    const name = draft.name.trim();
    if (!name || !draft.command.trim()) {
      missing = true;
      return;
    }
    nameClash = isBuiltinAgent(name) ? "builtin" : name !== editing && name in prefs.agent_presets ? "taken" : "";
    if (nameClash) {
      document.getElementById("preset-name")?.focus();
      return;
    }
    const d = $state.snapshot(draft);
    const was = editing;
    const preset: AgentPreset = {
      command: d.command.trim(),
      args: d.args.trim() ? d.args.trim().split(/\s+/) : [],
      env: parseEnv(d.env),
      icon: d.icon.trim() || null,
      color: d.color.trim() || null,
      maestro: d.maestro,
      start_in_chat_mode: d.chat,
      cwd: d.cwd.trim() || null,
    };
    const ok = await saveCanvas((c) => {
      if (was && was !== name) {
        delete c.agent_presets[was];
        if (c.default_agent === was) c.default_agent = name;
      }
      c.agent_presets[name] = preset;
    });
    if (ok) {
      showToast("success", $t("settings.ai.preset_saved") as string);
      close();
    }
  }

  function removePreset(name: string) {
    confirming = null;
    void saveCanvas((c) => {
      delete c.agent_presets[name];
      if (c.default_agent === name) c.default_agent = "shell";
    });
  }
</script>

<SettingGroup id="presets" icon="blocks" title={$t("settings.ai.presets_title") as string} description={$t("settings.ai.presets_desc") as string}>
  {#snippet actions()}
    {#if !open}<button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => startEditor(null)}>{$t("settings.ai.preset_add")}</button>{/if}
  {/snippet}
  <div class="list" data-setting-key="canvas.settings.presetsTitle">
    {#each names as name (name)}
      {@const p = prefs.agent_presets[name]}
      <div class="preset">
        <div class="preset-text">
          <span class="preset-name">{#if p.color}<span class="swatch" style:background={p.color}></span>{/if}<bdi>{name}</bdi></span>
          <span class="preset-cmd"><bdi>{[p.command, ...(p.args ?? [])].join(" ")}</bdi></span>
        </div>
        <div class="preset-actions">
          {#if confirming === name}
            <span class="ask"><bdi>{$t("settings.ai.preset_remove_ask", { name })}</bdi></span>
            <button type="button" class="btn btn-secondary btn-sm" onclick={() => (confirming = null)}>{$t("canvas.settings.cancel")}</button>
            <button type="button" class="btn btn-secondary btn-sm danger" disabled={busy} onclick={() => removePreset(name)}>{$t("canvas.settings.remove")}</button>
          {:else}
            <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => startEditor(name)}>{$t("canvas.settings.edit")}</button>
            <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => (confirming = name)}>{$t("canvas.settings.remove")}</button>
          {/if}
        </div>
      </div>
    {:else}
      {#if !open}<SettingsEmptyState title={$t("settings.ai.presets_empty") as string} />{/if}
    {/each}
  </div>

  {#if open}
    <form class="editor" onsubmit={(e) => { e.preventDefault(); void savePreset(); }}>
      <div class="pair">
        <label>{$t("canvas.settings.presetName")}<input id="preset-name" class="input-text" bind:value={draft.name} placeholder="Aider" aria-invalid={(missing && !draft.name.trim()) || !!nameClash} aria-describedby={nameClash ? "preset-name-error" : undefined} oninput={() => (nameClash = "")} /></label>
        <label>{$t("canvas.settings.presetCommand")}<input class="input-text mono" bind:value={draft.command} placeholder="aider" spellcheck="false" aria-invalid={missing && !draft.command.trim()} /></label>
      </div>
      <label>{$t("canvas.settings.presetArgs")}<input class="input-text mono" bind:value={draft.args} placeholder="--model sonnet" spellcheck="false" /></label>
      {#if nameClash}<p class="error" id="preset-name-error" role="alert"><bdi>{$t(nameClash === "builtin" ? "settings.ai.preset_name_builtin" : "settings.ai.preset_name_taken", { name: draft.name.trim() })}</bdi></p>{/if}
      {#if missing}<p class="error" role="alert"><bdi>{$t("canvas.settings.presetNeeds")}</bdi></p>{/if}
      <AdvancedDisclosure id="preset-editor">
        <div class="fields">
          <label>{$t("canvas.settings.presetEnv")}<textarea class="input-text mono" rows="2" bind:value={draft.env} spellcheck="false"></textarea></label>
          <label>{$t("canvas.settings.presetCwd")}<input class="input-text mono" bind:value={draft.cwd} spellcheck="false" /></label>
          <div class="pair">
            <label>{$t("canvas.settings.presetIcon")}<input class="input-text" bind:value={draft.icon} placeholder="hammer" /></label>
            <label>{$t("canvas.settings.presetColor")}<input class="input-text" bind:value={draft.color} placeholder="#FF8800" /></label>
          </div>
          <label class="check"><input type="checkbox" bind:checked={draft.maestro} />{$t("canvas.settings.presetMaestro")}</label>
          <label class="check"><input type="checkbox" bind:checked={draft.chat} />{$t("canvas.settings.presetChat")}</label>
        </div>
      </AdvancedDisclosure>
      <div class="buttons">
        <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={close}>{$t("canvas.settings.cancel")}</button>
        <button type="submit" class="btn btn-primary btn-sm" disabled={busy}>{editing ? $t("canvas.settings.savePreset") : $t("canvas.settings.addPreset")}</button>
      </div>
    </form>
  {/if}
</SettingGroup>

<style>
  .list { display: flex; flex-direction: column; }
  .preset { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--space-2) var(--space-3); padding-block: var(--space-2); }
  .preset + .preset { border-block-start: 1px solid var(--separator); }
  .preset-text { display: flex; flex-direction: column; min-inline-size: 0; }
  .preset-name { font-size: var(--text-base); line-height: 20px; color: var(--text); overflow-wrap: anywhere; }
  .preset-cmd { font-family: var(--font-mono); font-size: var(--text-sm); line-height: 18px; color: var(--text-muted); overflow-wrap: anywhere; }
  .preset-actions { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-1); }
  .ask { font-size: var(--text-sm); color: var(--text); }
  .danger { color: var(--error); }
  .swatch { display: inline-block; inline-size: 10px; block-size: 10px; border-radius: var(--radius-full); margin-inline-end: 6px; vertical-align: middle; }
  .editor { display: flex; flex-direction: column; gap: var(--space-2); margin-block-start: var(--space-2); padding: var(--space-3); border: 1px solid var(--content-border); border-radius: var(--radius-md); background: var(--primary); }
  .fields { display: flex; flex-direction: column; gap: var(--space-2); }
  .editor label { display: flex; flex-direction: column; gap: var(--space-1); flex: 1; min-inline-size: 0; font-size: var(--text-sm); color: var(--text-muted); }
  .editor label.check { flex-direction: row; align-items: center; gap: var(--space-2); color: var(--text); }
  .editor label.check input { accent-color: var(--accent); }
  .pair { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .pair > label { flex: 1 1 12rem; }
  .mono { font-family: var(--font-mono); }
  .error { margin: 0; font-size: var(--text-sm); color: var(--error); }
  .buttons { display: flex; justify-content: flex-end; gap: var(--space-2); }
</style>
