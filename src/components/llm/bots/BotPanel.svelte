<script lang="ts">
  /**
   * The bot screen: who it is (purpose, capabilities, memory policy), the
   * connection it runs on, its skills with their real state, what it can
   * really do this turn (from the backend manifest, never from the boxes),
   * what it remembers and its reading journeys.
   */
  import { untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { modelLabel } from "$lib/llm/types";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import { getSkills, loadSkills } from "$lib/stores/llm-skills-store.svelte";
  import {
    CAPABILITIES, acceptSkillVersion, bindSkill, capStateKey, capabilities as loadReport, capabilityHintKey,
    capabilityKey, connectionAgents, errorText, fixPlan, getBot, reasonKey, reinstallSkill, reloadSkills,
    saveProfile, setConnection, setSkillGrants, skillReads, unbindSkill,
    type BotView, type Capability, type CapabilityReport, type Fix, type SkillRead,
  } from "$lib/stores/assist-bots-store.svelte";
  import SkillBindingRow from "./SkillBindingRow.svelte";
  import BotMemoryPanel from "$components/llm/memory/BotMemoryPanel.svelte";
  import ReadingPanel from "$components/llm/reading/ReadingPanel.svelte";
  import BotLearning from "./BotLearning.svelte";

  let { botId, conversationId = null }: { botId: string; conversationId?: string | null } = $props();

  let view = $state<BotView | null>(null);
  let report = $state<CapabilityReport | null>(null);
  let reads = $state<SkillRead[]>([]);
  let busy = $state(false);
  let error = $state("");
  let saved = $state(false);
  let addSkill = $state("");
  let purpose = $state("");
  let instructions = $state("");
  let caps = $state<Capability[]>([]);
  let memoryPolicy = $state<"remember" | "ask" | "read_only">("remember");
  let connectionSelect: HTMLSelectElement | undefined = $state();

  let connections = $derived(connectionAgents(getAgents()).filter((a) => a.id !== botId));
  let unbound = $derived(getSkills().filter((s) => !view?.bindings.some((b) => b.skill === s.name)));
  let dirty = $derived(!!view && (purpose !== view.profile.purpose || instructions !== view.profile.instructions
    || memoryPolicy !== view.profile.memory_policy || [...caps].sort().join() !== [...view.profile.capabilities].sort().join()));

  async function refresh() {
    error = "";
    try {
      const [v, r, tr] = await Promise.all([getBot(botId), loadReport(botId, conversationId), skillReads(botId, 15).catch(() => [])]);
      view = v; report = r; reads = tr;
      purpose = v.profile.purpose; instructions = v.profile.instructions;
      caps = [...v.profile.capabilities]; memoryPolicy = v.profile.memory_policy;
    } catch (err) { error = errorText(err); }
  }

  $effect(() => {
    void botId;
    // Forced: a skill installed elsewhere (the reading panel, the Skills page)
    // must show up here to be linked, not wait for a stale session cache.
    untrack(() => { void loadSkills(true); void loadRoster(); void refresh(); });
  });

  async function run(fn: () => Promise<unknown>) {
    if (busy) return;
    busy = true; error = ""; saved = false;
    try { await fn(); await refresh(); } catch (err) { error = errorText(err); } finally { busy = false; }
  }

  function save() {
    if (!view) return;
    const profile = { ...view.profile, purpose, instructions, capabilities: caps, memory_policy: memoryPolicy };
    return run(async () => { await saveProfile(botId, profile); saved = true; });
  }

  function toggleCap(cap: Capability, on: boolean) {
    caps = on ? [...new Set([...caps, cap])] : caps.filter((c) => c !== cap);
  }

  function fix(f: Fix, skill?: string) {
    const plan = fixPlan(f);
    const name = f.target ?? skill ?? "";
    switch (f.action) {
      case "enable_capability":
        if (f.capability) { toggleCap(f.capability, true); void save(); }
        return;
      case "allow_scripts": {
        const s = report?.skills.find((x) => x.skill === name);
        if (s) void run(() => setSkillGrants(botId, name, s.allow_read, true));
        return;
      }
      case "allow_read": {
        const s = report?.skills.find((x) => x.skill === name);
        if (s) void run(() => setSkillGrants(botId, name, true, s.allow_scripts));
        return;
      }
      case "accept_version": void run(() => acceptSkillVersion(name)); return;
      case "reinstall": void run(() => reinstallSkill(name)); return;
      case "unbind": void run(() => unbindSkill(botId, name)); return;
      case "reload_skills": void run(() => reloadSkills()); return;
      case "switch_connection": connectionSelect?.focus(); return;
      case "check_capability": document.getElementById(`cap-${f.capability}`)?.focus(); return;
    }
    if (plan.route) void goto(plan.route);
  }

  function capStatus(cap: Capability) {
    return report?.capabilities.find((c) => c.id === cap);
  }
</script>

<section class="bot-panel" aria-busy={busy}>
  {#if !view}
    {#if error}<p class="error" role="alert">{error}</p><button class="button" onclick={refresh}>{$t("assist.bots.retry")}</button>
    {:else}<p class="hint" role="status">{$t("assist.bots.loading")}</p>{/if}
  {:else}
    <header>
      <h2>{view.agent.name}</h2>
      <p class="hint">{$t("assist.bots.identity_note")}</p>
    </header>

    <div class="block">
      <label class="field">
        <span class="field-label">{$t("assist.bots.create.purpose")}</span>
        <textarea class="input" rows="3" bind:value={purpose}></textarea>
      </label>
      <h3>{$t("assist.bots.capabilities_title")}</h3>
      <ul class="caps">
        {#each CAPABILITIES as cap (cap)}
          {@const st = capStatus(cap)}
          <li>
            <label class="cap">
              <input id={`cap-${cap}`} type="checkbox" checked={caps.includes(cap)} onchange={(e) => toggleCap(cap, e.currentTarget.checked)} disabled={busy} />
              <span><strong>{$t(capabilityKey(cap))}</strong><small>{$t(capabilityHintKey(cap))}</small></span>
            </label>
            {#if st}
              <span class="state {st.state}">{$t(capStateKey(st))}</span>
              {#if st.state !== "available" && st.state !== "not_requested" && reasonKey(st.reason)}
                <span class="reason">{$t(reasonKey(st.reason)!)}</span>
                {#if st.fix && st.fix.action !== "enable_capability"}<button class="button small" type="button" disabled={busy} onclick={() => fix(st.fix!)}>{$t(fixPlan(st.fix).labelKey)}</button>{/if}
              {/if}
            {/if}
          </li>
        {/each}
      </ul>
      {#if dirty}<p class="hint">{$t("assist.bots.unsaved")}</p>{/if}
      <details class="advanced">
        <summary>{$t("assist.bots.advanced")}</summary>
        <label class="field">
          <span class="field-label">{$t("assist.bots.create.instructions")}</span>
          <textarea class="input" rows="5" bind:value={instructions}></textarea>
        </label>
        <label class="field">
          <span class="field-label">{$t("assist.bots.memory_policy.label")}</span>
          <select class="input" bind:value={memoryPolicy}>
            <option value="remember">{$t("assist.bots.memory_policy.remember")}</option>
            <option value="ask">{$t("assist.bots.memory_policy.ask")}</option>
            <option value="read_only">{$t("assist.bots.memory_policy.read_only")}</option>
          </select>
        </label>
      </details>
      <div class="actions">
        <button class="button primary" type="button" disabled={busy || !dirty} onclick={save}>{$t("assist.bots.save")}</button>
        {#if saved}<span class="hint" role="status">{$t("assist.bots.saved")}</span>{/if}
      </div>
    </div>

    <div class="block">
      <h3>{$t("assist.bots.connection_title")}</h3>
      <p class="hint">{$t("assist.bots.connection_now")}: {modelLabel(view.agent.model)}</p>
      {#if connections.length}
        <label class="field">
          <span class="field-label">{$t("assist.bots.connection_switch")}</span>
          <select class="input" bind:this={connectionSelect} disabled={busy}
            onchange={(e) => { const id = e.currentTarget.value; if (id) void run(() => setConnection(botId, id)); }}>
            <option value="">—</option>
            {#each connections as a (a.id)}<option value={a.id}>{a.name} · {modelLabel(a.model)}</option>{/each}
          </select>
        </label>
      {/if}
      <p class="hint">{$t("assist.bots.connection_keeps")}</p>
    </div>

    <div class="block">
      <h3>{$t("assist.bots.skills.title")}</h3>
      {#if report?.skills.length}
        <ul class="skills">
          {#each report.skills as s (s.skill)}
            <SkillBindingRow status={s} {busy}
              onfix={(f) => fix(f, s.skill)}
              ongrants={(r, x) => void run(() => setSkillGrants(botId, s.skill, r, x))}
              onunbind={() => void run(() => unbindSkill(botId, s.skill))} />
          {/each}
        </ul>
      {:else}
        <p class="hint">{$t("assist.bots.skills.none")}</p>
      {/if}
      {#if unbound.length}
        <div class="actions">
          <select class="input narrow" bind:value={addSkill} aria-label={$t("assist.bots.skills.add")}>
            <option value="">{$t("assist.bots.skills.add")}</option>
            {#each unbound as s (s.name)}<option value={s.name}>{s.name}</option>{/each}
          </select>
          <button class="button" type="button" disabled={busy || !addSkill} onclick={() => { const n = addSkill; addSkill = ""; void run(() => bindSkill(botId, n)); }}>{$t("assist.bots.skills.bind")}</button>
        </div>
      {:else if getSkills().length === 0}
        <a class="link" href="/llm/skills">{$t("assist.bots.create.skill_install")}</a>
      {/if}
      {#if reads.length}
        <details class="advanced">
          <summary>{$t("assist.bots.skills.reads")}</summary>
          <ul class="reads">
            {#each reads as r (r.id)}
              <li class:failed={!r.ok}><span class="mono">{r.skill}/{r.file}</span> <span class="mono">{r.hash.slice(0, 8)}</span> <span>{new Date(r.at).toLocaleString()}</span>{#if r.error}<span> · {r.error}</span>{/if}</li>
            {/each}
          </ul>
        </details>
      {/if}
    </div>

    {#if report}
      <details class="advanced block">
        <summary>{$t("assist.bots.details")}</summary>
        <p class="hint">{$t("assist.bots.details_runtime")}: {report.runtime.label}{report.runtime.version ? ` ${report.runtime.version}` : ""} ({report.runtime_kind}) · {report.projectless ? $t("assist.bots.details_projectless") : $t("assist.bots.details_project")}</p>
        <p class="hint">{$t("assist.bots.details_offered")}:</p>
        <p class="mono small">{report.offered_tools.join(", ") || "—"}</p>
        {#if report.unavailable_grants.length}
          <p class="hint">{$t("assist.bots.details_unavailable")}:</p>
          <ul class="reads">{#each report.unavailable_grants as g (g.tool)}<li class="mono">{g.tool} · {$t(reasonKey(g.reason) ?? "assist.bots.reason.other")}</li>{/each}</ul>
        {/if}
      </details>
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <div class="block"><BotMemoryPanel {botId} /></div>
    <div class="block"><ReadingPanel {botId} /></div>
    <div class="block"><BotLearning {botId} /></div>
  {/if}
</section>

<style>
  .bot-panel { display: flex; flex-direction: column; gap: 16px; max-width: 820px; width: 100%; }
  header h2 { margin: 0 0 4px; font-size: 20px; }
  h3 { margin: 0; font-size: 15px; }
  .block { border: 1px solid var(--separator); border-radius: var(--radius-sm); padding: 14px; display: flex; flex-direction: column; gap: 12px; }
  .hint { margin: 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .caps, .skills, .reads { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
  .caps li { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .cap { display: flex; gap: 10px; align-items: flex-start; flex: 1 1 260px; cursor: pointer; }
  .cap span { display: flex; flex-direction: column; gap: 2px; }
  .cap small { color: var(--text-muted); font-size: 12px; }
  .state { font-size: 12px; padding: 2px 8px; border-radius: 999px; background: var(--fill-tertiary); }
  .state.available { color: var(--success, #067647); }
  .state.partial, .state.missing { color: var(--warning, #b54708); }
  .reason { font-size: 12px; color: var(--text-muted); flex-basis: 100%; }
  .actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .narrow { max-width: 260px; }
  .advanced summary { cursor: pointer; font-weight: 600; }
  .reads li { font-size: 12px; }
  .reads li.failed { color: var(--warning, #b54708); }
  .mono { font-family: var(--font-mono); overflow-wrap: anywhere; }
  .small { font-size: 12px; margin: 0; }
  .link { color: var(--accent-text); font-size: 13px; }
  .error { color: var(--error, #b42318); margin: 0; overflow-wrap: anywhere; }
  .button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible, summary:focus-visible { outline: var(--focus-ring, 2px solid var(--accent-text)); outline-offset: 2px; }
</style>
