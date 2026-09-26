<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  /** Roster tab: the agent list, the editor and the team templates. */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import type { AgentDef } from "$lib/llm/types";
  import { agentTint, roleLabelKey, roleText, modelLabel } from "$lib/llm/types";
  import {
    applyTemplate,
    deleteAgent,
    getAgents,
    isRosterAvailable,
    loadRoster,
    saveAgent,
  } from "$lib/stores/llm-store.svelte";
  import { tintToCss } from "$lib/stores/profile-store.svelte";
  import RosterEditor from "$components/llm/RosterEditor.svelte";
  import TemplatePicker from "$components/llm/TemplatePicker.svelte";
  import CreateBot from "$components/llm/bots/CreateBot.svelte";
  import BotPanel from "$components/llm/bots/BotPanel.svelte";
  import { page } from "$app/state";

  let agents = $derived(getAgents());
  let editing = $state<AgentDef | null>(null);
  // "Create bot" (the guided flow) and the bot screen of one roster agent.
  let creating = $state(false);
  let selected = $state<string | null>(null);
  let selectedAgent = $derived(agents.find((a) => a.id === selected) ?? null);
  let query = $state("");
  let filtered = $derived(agents.filter(agent => `${agent.name} ${agent.system_prompt}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())));

  onMount(() => {
    void loadRoster();
    const q = page.url.searchParams;
    if (q.get("create") === "1") creating = true;
    if (q.get("bot")) selected = q.get("bot");
  });

  function blankAgent(): AgentDef {
    return {
      id: `agent-${Date.now().toString(36)}`,
      name: "",
      role: "worker",
      system_prompt: "",
      model: { policy: "fixed", model: { provider: "openai", model: "" } },
      tools: [],
      skills: [],
      budget: {},
      runtime: { kind: "native" },
      skin: { id: "omni-default", tint: [110, 139, 255] },
    };
  }

  let saving = $state(false);
  let saveError = $state(false);
  let saved = $state(false);
  async function onSave(agent: AgentDef) {
    if (saving) return;
    saving = true;
    saveError = false;
    saved = false;
    const ok = await saveAgent(agent);
    saving = false;
    if (ok) { editing = null; saved = true; }
    else saveError = true;
  }
  async function onTemplate(id: string) {
    if (saving) return;
    saving = true; saveError = false; saved = false;
    const ok = await applyTemplate(id);
    saving = false;
    saveError = !ok;
  }
  async function onDelete(id: string) {
    if (saving) return;
    saving = true;
    saveError = false;
    const ok = await deleteAgent(id);
    saving = false;
    if (ok) editing = null;
    else saveError = true;
  }
</script>

<svelte:head><title>{$t("llm.roster.title")}</title></svelte:head>

<div class="page page-wide roster-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.roster.title")}</h1>
      <p class="page-lede">{$t("llm.roster.roles_hint")}</p>
    </div>
    <div class="head-actions">
      <button type="button" class="button primary" disabled={saving} onclick={() => { saved = false; saveError = false; editing = null; selected = null; creating = true; }}>
        {$t("assist.bots.create.open")}
      </button>
      <button type="button" class="button" disabled={saving} onclick={() => { saved = false; saveError = false; creating = false; selected = null; editing = blankAgent(); }}>
        {$t("llm.roster.new")}
      </button>
    </div>
  </header>
  <SurfaceGuide text={$t("llm.surface.roster_hint")} href="/help?article=agent#guide" />

  {#if !isRosterAvailable()}
    <p class="notice" role="status">{$t("llm.roster.unavailable")}</p>
    <button type="button" class="button" onclick={() => void loadRoster(true)}>{$t("llm.surface.retry")}</button>
  {/if}

  {#if saveError}<p class="notice" role="alert">{$t("llm.roster.save_error")}</p>{/if}
  {#if saved}<p class="notice" role="status">{$t("llm.roster.saved")}</p>{/if}
  {#if creating}
    <CreateBot oncancel={() => (creating = false)} oncreated={(v) => { creating = false; selected = v.agent.id; }} />
  {:else if selectedAgent && !editing}
    <div class="bot-actions">
      <button type="button" class="button" onclick={() => (selected = null)}>‹ {$t("assist.bots.back")}</button>
      <button type="button" class="button" onclick={() => { editing = selectedAgent; }}>{$t("assist.bots.edit_advanced")}</button>
    </div>
    {#key selectedAgent.id}<BotPanel botId={selectedAgent.id} />{/key}
  {:else if editing}
    {#key editing.id}
    <RosterEditor
      agent={editing}
      busy={saving}
      onsave={onSave}
      oncancel={() => { editing = null; saveError = false; }}
      ondelete={agents.some((a) => a.id === editing?.id)
        ? onDelete
        : undefined}
    />
    {/key}
  {:else}
    <label class="agent-search"><span>{$t("llm.surface.search")}</span><input class="input" type="search" bind:value={query} /></label>
    <div class="group">
      {#each filtered as agent (agent.id)}
        <button type="button" class="group-row agent-row" onclick={() => (selected = agent.id)}>
          <span class="dot" style:background={tintToCss(agentTint(agent))}></span>
          <span class="group-row-content">
            <span class="group-row-title">{agent.name}</span>
            <span class="group-row-sub">
              {agent.system_prompt.split("\n")[0] || (typeof agent.role === "string" && ["worker", "coordinator", "advisor"].includes(agent.role) ? $t(`llm.surface.${agent.role}`) : roleText(agent.role))}
              <span class="agent-connection">{modelLabel(agent.model)}</span>
            </span>
          </span>
          <span class="group-row-trailing chevron">›</span>
        </button>
      {/each}
    </div>

    {#if filtered.length === 0 && query}<p role="status">{$t("llm.surface.no_matches")}</p>{/if}
    <details class="team-templates"><summary>{$t("llm.surface.team")}</summary><p>{$t("llm.surface.templates_impact")}</p><TemplatePicker disabled={saving} onapply={onTemplate} /></details>
  {/if}
</div>

<style>
  .head-actions, .bot-actions { display:flex; gap:8px; flex-wrap:wrap; }
  .bot-actions { margin-bottom:16px; }
  .agent-search { display:flex; flex-direction:column; gap:8px; max-width:400px; margin-bottom:20px; font-size:13px; color:var(--text-muted); }
  .agent-connection { display:block; margin-top:6px; color:var(--text-muted); }
  .team-templates summary { cursor:pointer; padding:16px 0; font-weight:600; }
  .team-templates p { color:var(--text-muted); font-size:13px; }
  /* Block layout on purpose: as a flex column the groups would shrink to fit
     the viewport and clip their own rows instead of letting the page scroll. */
  .roster-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .roster-page :global(.group) {
    margin-bottom: var(--space-5);
  }

  .notice {
    margin: 0 0 var(--space-4);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .agent-row {
    width: 100%;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
    color: inherit;
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .dot {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }
</style>
