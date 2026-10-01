<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { modelLabel } from "$lib/llm/types";
  import { t } from "$lib/i18n";
  import type { AgentDef, AgentRole, Candidate, ModelRef } from "$lib/llm/types";
  import { getSkills, loadSkills } from "$lib/stores/llm-skills-store.svelte";
  import ModelPicker from "./ModelPicker.svelte";

  let {
    agent,
    busy = false,
    onsave,
    oncancel,
    ondelete,
    onconnect,
    initialStep = 0,
  }: {
    agent: AgentDef;
    busy?: boolean;
    onsave: (agent: AgentDef) => void;
    oncancel: () => void;
    ondelete?: (id: string) => void;
    onconnect?: (draft: AgentDef) => void;
    initialStep?: number;
  } = $props();

  const policyGroup = $props.id();
  let step = $state(untrack(() => initialStep));
  let confirmingDelete = $state(false);
  let steps = $derived([$t("llm.surface.identity"), $t("llm.surface.model"), $t("llm.surface.review")]);
  const ROLES = ["coordinator", "worker", "advisor"] as const;

  function cloneAgent(source: AgentDef): AgentDef {
    const copy = structuredClone($state.snapshot(source)) as AgentDef;
    copy.budget ??= {};
    copy.tools ??= [];
    copy.skills ??= [];
    return copy;
  }

  let draft = $state<AgentDef>(untrack(() => cloneAgent(agent)));
  let roleValue = $state(untrack(() => (typeof agent.role === "string" ? agent.role : "custom")));
  let customRole = $state(untrack(() => (typeof agent.role === "string" ? "" : agent.role.custom)));

  let validModel = $derived(draft.model.policy === "fixed" ? !!draft.model.model.provider.trim() && !!draft.model.model.model.trim() : draft.model.chain.length > 0 && draft.model.chain.every(candidate => !!candidate.model.trim() && (candidate.runtime === "native" ? !!candidate.provider.trim() : !!candidate.account_id.trim())));

  let isRoute = $derived(draft.model.policy === "route");

  onMount(() => {
    void loadSkills();
  });

  function toggleSkill(name: string, on: boolean) {
    const current = draft.skills ?? [];
    draft.skills = on ? [...current, name] : current.filter((s) => s !== name);
  }

  function setPolicy(next: "fixed" | "route") {
    if (next === draft.model.policy) return;
    draft.model =
      next === "fixed"
        ? { policy: "fixed", model: { provider: "", model: "" } }
        : { policy: "route", chain: [] };
  }

  function setFixedModel(ref: ModelRef) {
    draft.model = { policy: "fixed", model: ref };
  }

  function addCandidate() {
    if (draft.model.policy !== "route") return;
    const candidate: Candidate = {
      runtime: "native",
      provider: "",
      model: "",
      max_cost_per_1k: null,
      min_context: 0,
    };
    draft.model = { policy: "route", chain: [...draft.model.chain, candidate] };
  }

  function removeCandidate(index: number) {
    if (draft.model.policy !== "route") return;
    draft.model = { policy: "route", chain: draft.model.chain.filter((_, i) => i !== index) };
  }

  function role(): AgentRole {
    return roleValue === "custom" ? { custom: customRole.trim() || "custom" } : (roleValue as AgentRole);
  }

  function save() {
    onsave({ ...($state.snapshot(draft) as AgentDef), role: role() });
  }

  function preserveForConnection() {
    onconnect?.({ ...($state.snapshot(draft) as AgentDef), role: role() });
  }
</script>

<form class="editor" aria-busy={busy} onsubmit={(e) => { e.preventDefault(); if (!busy && step === 2 && draft.name.trim() && validModel) save(); }}>
  <ol class="setup-steps" aria-label={$t("llm.surface.configure")}>
    {#each steps as label, index}<li class:current={step === index} class:complete={step > index} aria-current={step === index ? "step" : undefined}><span>{index + 1}</span>{label}</li>{/each}
  </ol>
  <fieldset class="editor-fields" disabled={busy}>
  {#if step === 0}
  <label class="field">
    <span class="field-label">{$t("llm.roster.name")}</span>
    <input class="input" bind:value={draft.name} required maxlength="48" />
  </label>

  <label class="field">
    <span class="field-label">{$t("llm.roster.role")}</span>
    <select class="input" bind:value={roleValue}>
      {#each ROLES as r (r)}
        <option value={r}>{$t(`llm.surface.${r}`)}</option>
      {/each}
      <option value="custom">{$t("llm.role.custom")}</option>
    </select>
  </label>

  {#if roleValue === "custom"}
    <label class="field">
      <span class="field-label">{$t("llm.role.custom")}</span>
      <input class="input" bind:value={customRole} maxlength="32" />
    </label>
  {/if}

  <label class="field">
    <span class="field-label">{$t("llm.surface.purpose")}</span>
    <textarea class="input prompt" rows="4" bind:value={draft.system_prompt}></textarea>
  </label>

  {/if}
  {#if step === 1}
  <fieldset class="group-block">
    <legend class="field-label">{$t("llm.roster.model_policy")}</legend>
    <div class="policy-choice">
      <label><input type="radio" name={policyGroup} checked={!isRoute} onchange={() => setPolicy("fixed")} />{$t("llm.roster.fixed")}</label>
      <label><input type="radio" name={policyGroup} checked={isRoute} onchange={() => setPolicy("route")} />{$t("llm.roster.route")}</label>
    </div>

    {#if draft.model.policy === "fixed"}
      <ModelPicker value={draft.model.model} onchange={setFixedModel} onconnect={preserveForConnection} connectHref="/llm/accounts?guide=api&returnTo=editor" />
    {:else}
      <ol class="chain">
        {#each draft.model.chain as candidate, index (candidate)}
          <li class="chain-row">
            <div class="candidate-model">
              {#if candidate.runtime === "native"}
                <ModelPicker value={{ provider: candidate.provider, model: candidate.model }} onchange={ref => { if (candidate.runtime === "native") { candidate.provider = ref.provider; candidate.model = ref.model; } }} onconnect={preserveForConnection} connectHref="/llm/accounts?guide=api&returnTo=editor" />
              {:else}
                <span class="hint">{candidate.account_id}</span>
                <label class="field">
                  <span class="field-label">{$t("llm.surface.model")}</span>
                  <input class="input" bind:value={candidate.model} required />
                </label>
              {/if}
            </div>
            <button type="button" class="button" onclick={() => removeCandidate(index)}>
              {$t("llm.roster.remove")}
            </button>
          </li>
        {/each}
      </ol>
      <button type="button" class="button" onclick={addCandidate}>
        {$t("llm.roster.add_candidate")}
      </button>
    {/if}
  </fieldset>

  {/if}
  {#if step === 2}
  <section class="agent-review"><h2>{draft.name}</h2><p>{draft.system_prompt || $t("llm.surface.worker")}</p><span>{modelLabel(draft.model)}</span></section>
  <details class="agent-advanced"><summary>{$t("llm.surface.customize")}</summary>
  <fieldset class="group-block">
    <legend class="field-label">{$t("llm.roster.tools")}</legend>
    {#if (draft.tools ?? []).length === 0}
      <p class="hint">
        {$t("llm.roster.tools_empty")}
        <a class="mcp-link" href="/llm/mcp">{$t("llm.mcp.grants")}</a>
      </p>
    {:else}
      <ul class="tools">
        {#each draft.tools ?? [] as grant, index (index)}
          <li>
            <span class="mono">
              {grant.source === "mcp" ? `${grant.server}/${grant.tool}` : grant.name}
            </span>
            <select class="input mode" aria-label={`${grant.source === "mcp" ? grant.tool : grant.name}: ${$t("llm.roster.tools")}`} bind:value={grant.mode}>
              <option value="auto">{$t("llm.inspector.grant_auto")}</option>
              <option value="ask">{$t("llm.inspector.grant_ask")}</option>
              <option value="deny">{$t("llm.inspector.grant_deny")}</option>
            </select>
          </li>
        {/each}
      </ul>
      <a class="mcp-link" href="/llm/mcp">{$t("llm.mcp.grants")}</a>
    {/if}
  </fieldset>

  <fieldset class="group-block">
    <legend class="field-label">{$t("llm.roster.skills")}</legend>
    {#if getSkills().length === 0}
      <p class="hint">{$t("llm.roster.skills_empty")}</p>
    {:else}
      <ul class="tools">
        {#each getSkills() as skill (skill.name)}
          <li>
            <label class="skill-row">
              <input
                type="checkbox"
                checked={(draft.skills ?? []).includes(skill.name)}
                onchange={(e) => toggleSkill(skill.name, e.currentTarget.checked)}
              />
              <span class="mono">{skill.name}</span>
            </label>
          </li>
        {/each}
      </ul>
    {/if}
  </fieldset>

  <fieldset class="group-block">
    <legend class="field-label">{$t("llm.roster.budget")}</legend>
    <div class="budget">
      <label class="field">
        <span class="field-label">{$t("llm.inspector.usd_per_day")}</span>
        <input class="input" type="number" min="0" step="0.1" bind:value={draft.budget!.usd_per_day} />
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.inspector.tokens_per_turn")}</span>
        <input class="input" type="number" min="0" step="1000" bind:value={draft.budget!.tokens_per_turn} />
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.inspector.max_tool_calls")}</span>
        <input class="input" type="number" min="0" max="255" bind:value={draft.budget!.max_tool_calls_per_turn} />
      </label>
    </div>
  </fieldset>

  </details>
  {/if}
  <div class="actions">
    {#if step > 0}<button type="button" class="button" onclick={() => step -= 1}>{$t("llm.mcp.back")}</button>{/if}
    {#if step < 2}<button type="button" class="button primary" disabled={step === 0 ? !draft.name.trim() : !validModel} onclick={() => step += 1}>{steps[step + 1]}</button>
    {:else}<button type="submit" class="button primary">{$t("llm.roster.save")}</button>{/if}
    <button type="button" class="button" onclick={oncancel}>{$t("llm.roster.cancel")}</button>
    {#if ondelete}
      {#if confirmingDelete}
        <div class="delete-confirm" role="group" aria-label={$t("llm.roster.delete") as string}>
          <p>{$t("llm.roster.delete_warning", { name: draft.name })}</p>
          <button type="button" class="button danger" disabled={busy} onclick={() => ondelete?.(draft.id)}>
            {$t("llm.roster.delete_confirm")}
          </button>
          <button type="button" class="button" disabled={busy} onclick={() => confirmingDelete = false}>
            {$t("llm.roster.cancel")}
          </button>
        </div>
      {:else}
        <button type="button" class="button danger" disabled={busy} onclick={() => confirmingDelete = true}>
          {$t("llm.roster.delete")}
        </button>
      {/if}
    {/if}
  </div>
  </fieldset>
</form>

<style>
  .policy-choice { display: flex; gap: var(--space-4); flex-wrap: wrap; }
  .policy-choice label { display: flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); }
  .candidate-model { flex: 1; min-width: 0; }
  .setup-steps { display:flex; list-style:none; gap:16px; margin:0 0 12px; padding:0; flex-wrap:wrap; }
  .setup-steps li { display:flex; align-items:center; gap:8px; color:var(--text-muted); font-size:13px; }
  .setup-steps li span { display:grid; place-items:center; width:28px; height:28px; border:1px solid var(--separator); border-radius:50%; }
  .setup-steps .current { color:var(--text); font-weight:600; } .setup-steps .current span, .setup-steps .complete span { background:var(--accent-soft); border-color:var(--accent); }
  .agent-review { padding:20px; border:1px solid var(--separator); border-radius:var(--radius-lg); }
  .delete-confirm { display:flex; align-items:center; flex-wrap:wrap; gap:var(--space-2); flex:1 1 100%; padding:var(--space-3); border:1px solid var(--separator); border-radius:var(--radius-md); }
  .delete-confirm p { flex:1 1 100%; margin:0; color:var(--text-dim); font-size:var(--text-sm); line-height:var(--leading-sm); }
  .agent-review h2 { font-size:18px; margin:0 0 12px; } .agent-review p { white-space:pre-wrap; overflow-wrap:anywhere; font-size:14px; line-height:1.6; } .agent-review span { color:var(--text-muted); font-size:13px; }
  .agent-advanced summary { cursor:pointer; padding:12px 0; font-weight:600; }
  .agent-advanced .group-block { margin-top:16px; }

  .editor-fields { border: 0; padding: 0; margin: 0; min-width: 0; display: flex; flex-direction: column; gap: var(--space-4); }
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 760px;
    width: 100%;
  }

  .group-block {
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .prompt {
    resize: vertical;
    line-height: var(--leading-base);
  }

  .chain {
    list-style: decimal;
    margin: 0;
    padding-left: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .chain-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .tools {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .tools li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .mode {
    width: 120px;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .skill-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
  }

  .mcp-link {
    font-size: var(--text-sm);
    color: var(--accent-hi);
    text-decoration: none;
    align-self: flex-start;
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .budget {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: var(--space-3);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
