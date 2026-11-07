<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { canvasPrefs, watchCanvasPrefs, type CanvasPrefs } from "$lib/canvas/prefs.svelte";
  import SettingGroup from "$components/settings/rebrand/SettingGroup.svelte";
  import SettingRow from "$components/settings/rebrand/SettingRow.svelte";
  import GeneralChoice from "$components/settings/rebrand/GeneralChoice.svelte";
  import AgentLogo from "$components/agents/AgentLogo.svelte";
  import AdvancedDisclosure from "./AdvancedDisclosure.svelte";
  import { agentChoices } from "./ai-settings";
  import { canvasSaving, saveCanvas } from "./canvas-save.svelte";

  const prefs = $derived(canvasPrefs());
  const busy = $derived(canvasSaving.busy);
  const agents = $derived(agentChoices(Object.keys(prefs.agent_presets), prefs.default_agent));

  onMount(() => watchCanvasPrefs());

  function agentName(id: string): string {
    return prefs.agent_presets[id] ? id : ($t(`canvas.term.agent.${id}`, { default: id }) as string);
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} disabled={busy} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#snippet agentNode()}
  <rect class="node" x="8" y="15" width="24" height="20" rx="3" />
  <path class="prompt" d="M12 22l3 2.500-3 2.500M18 28h8" />
{/snippet}

{#snippet awakeWorking()}
  <rect class="node" x="10" y="10" width="40" height="28" rx="3" />
  <path class="prompt" d="M15 19l3 2.500-3 2.500M21 25h14" />
  <circle class="sun" cx="63" cy="17" r="5" />
  <path class="ray" d="M63 7v3M63 24v3M53 17h3M70 17h3" />
{/snippet}

{#snippet awakeAlways()}
  <circle class="sun" cx="40" cy="25" r="8" />
  <path class="ray" d="M40 9v4M40 37v4M24 25h4M52 25h4M29 14l2.500 2.500M48.500 33.500L51 36M29 36l2.500-2.500M48.500 16.500L51 14" />
{/snippet}

{#snippet awakeNever()}
  <path class="moon" d="M45 12a13 13 0 1 0 8 22 11 11 0 0 1-8-22z" />
  <path class="zz" d="M58 13h5l-5 6h5M64 7h3l-3 4h3" />
{/snippet}

{#snippet notesFree()}
  {@render agentNode()}
  <rect class="item" x="44" y="8" width="14" height="12" rx="2" />
  <rect class="item" x="58" y="28" width="14" height="12" rx="2" />
  <rect class="item" x="40" y="32" width="12" height="10" rx="2" />
{/snippet}

{#snippet notesStack()}
  {@render agentNode()}
  <rect class="item back" x="50" y="11" width="20" height="16" rx="2" />
  <rect class="item back" x="47" y="15" width="20" height="16" rx="2" />
  <rect class="item" x="44" y="19" width="20" height="16" rx="2" />
{/snippet}

{#snippet portalsFree()}
  {@render agentNode()}
  <rect class="portal" x="46" y="7" width="26" height="16" rx="2" />
  <path class="portal-bar" d="M46 11h26" />
  <rect class="portal" x="40" y="29" width="22" height="14" rx="2" />
  <path class="portal-bar" d="M40 33h22" />
{/snippet}

{#snippet portalsBeside()}
  {@render agentNode()}
  <rect class="portal back" x="41" y="11" width="28" height="20" rx="2" />
  <rect class="portal" x="37" y="15" width="28" height="20" rx="2" />
  <path class="portal-bar" d="M37 19h28" />
{/snippet}

<SettingGroup id="agents-canvas" icon="terminal" title={$t("settings.ai.agents_canvas_title") as string} description={$t("settings.ai.agents_canvas_desc") as string}>
  <fieldset class="agents" data-setting-key="canvas.settings.defaultAgent" disabled={busy}>
    <legend>{$t("canvas.settings.defaultAgent")}</legend>
    {#each agents as id (id)}
      <label class="agent" class:selected={prefs.default_agent === id}>
        <input type="radio" name="canvas-default-agent" value={id} checked={prefs.default_agent === id} onchange={() => saveCanvas((c) => (c.default_agent = id))} />
        <AgentLogo id={id === "shell" ? "" : id} name={id === "shell" ? ">" : agentName(id)} size={18} />
        <span class="agent-name"><bdi>{agentName(id)}</bdi></span>
      </label>
    {/each}
  </fieldset>

  <div class="block" data-setting-key="canvas.settings.preventSleep">
    <span class="block-label">{$t("canvas.settings.preventSleep")}</span>
    <GeneralChoice
      name="canvas-prevent-sleep"
      label={$t("canvas.settings.preventSleep") as string}
      value={prefs.power.prevent_sleep}
      options={[
        { value: "while_working", draw: awakeWorking, label: $t("canvas.settings.sleepWorking") as string, help: $t("settings.ai.sleep_working_help") as string },
        { value: "always", draw: awakeAlways, label: $t("canvas.settings.sleepAlways") as string, help: $t("settings.ai.sleep_always_help") as string },
        { value: "never", draw: awakeNever, label: $t("canvas.settings.sleepNever") as string, help: $t("settings.ai.sleep_never_help") as string },
      ]}
      onchange={(v) => saveCanvas((c) => (c.power.prevent_sleep = v as CanvasPrefs["power"]["prevent_sleep"]))}
    />
  </div>

  <SettingRow id="canvas-notify-attention" label={$t("canvas.settings.notifyAttention")} help={$t("settings.ai.notify_attention_help")} settingKey="canvas.settings.notifyAttention">
    {#snippet control({ id, describedBy })}
      {@render toggle(id, describedBy, prefs.notifications.attention, () => saveCanvas((c) => (c.notifications.attention = !c.notifications.attention)))}
    {/snippet}
  </SettingRow>

  <div class="block" data-setting-key="canvas.settings.agentPlacement">
    <span class="block-label">{$t("canvas.settings.agentPlacement")}</span>
    <GeneralChoice
      name="canvas-agent-placement"
      label={$t("canvas.settings.agentPlacement") as string}
      value={prefs.defaults.agent_placement}
      options={[
        { value: "free", draw: notesFree, label: $t("settings.ai.place_free") as string, help: $t("settings.ai.notes_free_help") as string },
        { value: "stack", draw: notesStack, label: $t("settings.ai.place_stack") as string, help: $t("settings.ai.notes_stack_help") as string },
      ]}
      onchange={(v) => saveCanvas((c) => (c.defaults.agent_placement = v as "free" | "stack"))}
    />
  </div>

  <div class="block" data-setting-key="canvas.settings.portalPlacement">
    <span class="block-label">{$t("canvas.settings.portalPlacement")}</span>
    <GeneralChoice
      name="canvas-portal-placement"
      label={$t("canvas.settings.portalPlacement") as string}
      value={prefs.defaults.portal_placement}
      options={[
        { value: "free", draw: portalsFree, label: $t("settings.ai.place_free") as string, help: $t("settings.ai.portals_free_help") as string },
        { value: "beside", draw: portalsBeside, label: $t("settings.ai.place_beside") as string, help: $t("settings.ai.portals_beside_help") as string },
      ]}
      onchange={(v) => saveCanvas((c) => (c.defaults.portal_placement = v as "free" | "beside"))}
    />
  </div>

  <AdvancedDisclosure id="agents-canvas">
    <SettingRow id="canvas-badges-non-maestro" label={$t("canvas.settings.badgesNonMaestro")} settingKey="canvas.settings.badgesNonMaestro">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, prefs.notifications.badges_for_non_maestro, () => saveCanvas((c) => (c.notifications.badges_for_non_maestro = !c.notifications.badges_for_non_maestro)))}
      {/snippet}
    </SettingRow>
    <SettingRow id="canvas-memory-limit" label={$t("canvas.settings.memoryLimit")} help={$t("canvas.settings.memoryLimitHint")} settingKey="canvas.settings.memoryLimit">
      {#snippet control({ id, describedBy })}
        <input {id} class="input-text num" type="number" min="0" step="100" aria-describedby={describedBy} disabled={busy} value={prefs.terminal.memory_limit_mb}
          onchange={(e) => saveCanvas((c) => (c.terminal.memory_limit_mb = Math.max(0, Math.round(Number(e.currentTarget.value) || 0))))} />
      {/snippet}
    </SettingRow>
  </AdvancedDisclosure>
</SettingGroup>

<style>
  .agents { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 150px), 1fr)); gap: var(--space-2); min-inline-size: 0; margin: var(--space-1) 0 var(--space-2); padding: 0; border: 0; }
  .agents legend { float: left; inline-size: 100%; padding: 0 0 var(--space-1); font-size: var(--text-base); line-height: 20px; color: var(--text); }
  .agents legend + * { clear: both; }
  .agent { position: relative; display: flex; align-items: center; gap: var(--space-2); padding: 6px var(--space-2); border: 1px solid var(--content-border); border-radius: var(--radius-md); background: var(--primary); cursor: pointer; }
  .agents:not(:disabled) .agent:hover { background: var(--surface-hi); }
  .agent.selected { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
  .agent:has(input:focus-visible) { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .agent input { position: absolute; opacity: 0; inline-size: 1px; block-size: 1px; }
  .agent-name { font-size: var(--text-base); line-height: 18px; color: var(--text); overflow-wrap: anywhere; }
  .block { padding-block: var(--space-2); border-block-start: 1px solid var(--separator); }
  .block-label { display: block; font-size: var(--text-base); line-height: 20px; color: var(--text); }
  .num { inline-size: 96px; }
  .node, .portal, .item { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .item { stroke: var(--accent); }
  .back { opacity: 0.55; }
  .portal-bar { fill: none; stroke: var(--content-border); }
  .prompt { fill: none; stroke: var(--accent); stroke-width: 1.500; stroke-linecap: round; stroke-linejoin: round; }
  .sun { fill: var(--accent); }
  .ray { fill: none; stroke: var(--accent); stroke-width: 1.500; stroke-linecap: round; }
  .moon { fill: var(--tertiary); }
  .zz { fill: none; stroke: var(--tertiary); stroke-width: 1.200; stroke-linecap: round; stroke-linejoin: round; }
</style>
