<script lang="ts">
  /**
   * "Create bot": name, purpose, an optional installed skill, capabilities in
   * plain words, and the connection it runs on. No code or shell access by
   * default; instructions and memory policy sit under "advanced".
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getAgents, getRosterLoadError, loadRoster } from "$lib/stores/llm-store.svelte";
  import { getSkills, loadSkills } from "$lib/stores/llm-skills-store.svelte";
  import {
    CAPABILITIES, capabilityHintKey, capabilityKey, connectionAgents, createBot, defaultCapabilities,
    errorText, validateNewBot, type BotView, type Capability, type MemoryPolicy,
  } from "$lib/stores/assist-bots-store.svelte";
  import { modelLabel } from "$lib/llm/types";

  let { oncreated, oncancel, onadvanced }: { oncreated: (view: BotView) => void; oncancel: () => void; onadvanced?: (identity: { name: string; purpose: string }) => void } = $props();

  let name = $state("");
  let purpose = $state("");
  let instructions = $state("");
  let skill = $state("");
  let caps = $state<Capability[]>(defaultCapabilities());
  let memoryPolicy = $state<MemoryPolicy>("remember");
  let connection = $state("");
  let busy = $state(false);
  let errorKey = $state<string | null>(null);
  let errorDetail = $state("");

  const DRAFT_KEY = "omniget-create-agent-draft";
  const connectionHint = $props.id();
  let hydrated = $state(false);
  let keepDraft = true;
  let connectionsReady = $state(false);
  let connections = $derived(connectionAgents(getAgents()));
  let connectionLoadError = $derived(getRosterLoadError());

  onMount(() => {
    try {
      const saved = JSON.parse(sessionStorage.getItem(DRAFT_KEY) ?? "null");
      if (saved?.version === 1) {
        name = typeof saved.name === "string" ? saved.name.slice(0, 48) : "";
        purpose = typeof saved.purpose === "string" ? saved.purpose.slice(0, 10000) : "";
        instructions = typeof saved.instructions === "string" ? saved.instructions.slice(0, 20000) : "";
        skill = typeof saved.skill === "string" ? saved.skill : "";
        connection = typeof saved.connection === "string" ? saved.connection : "";
        caps = Array.isArray(saved.caps) ? saved.caps.filter((cap: Capability) => CAPABILITIES.includes(cap)) : defaultCapabilities();
        memoryPolicy = ["remember", "ask", "read_only"].includes(saved.memoryPolicy) ? saved.memoryPolicy : "remember";
      }
    } catch { /* Invalid drafts do not block creating an agent. */ }
    hydrated = true;
    void loadRoster().finally(() => { connectionsReady = true; });
    void loadSkills();
  });

  // This window's draft survives visiting Connections, without becoming an agent.
  $effect(() => {
    if (!hydrated) return;
    const draft = { version: 1, name, purpose, instructions, skill, caps: [...caps], memoryPolicy, connection };
    if (!keepDraft) return;
    try { sessionStorage.setItem(DRAFT_KEY, JSON.stringify(draft)); } catch { /* Storage can be unavailable. */ }
  });

  function clearDraft() {
    keepDraft = false;
    try { sessionStorage.removeItem(DRAFT_KEY); } catch { /* Optional storage. */ }
  }

  function cancel() { clearDraft(); oncancel(); }

  async function retryConnections() {
    connectionsReady = false;
    await loadRoster(true);
    connectionsReady = true;
  }

  $effect(() => {
    if (connectionsReady && !connections.some(a => a.id === connection)) connection = connections[0]?.id ?? "";
  });

  function toggle(cap: Capability, on: boolean) {
    // Reading records availability evidence only from pages it really fetched,
    // so it brings web search along; turning web off turns reading off too.
    const add: Capability[] = on && cap === "reading" ? [cap, "web" as Capability] : [cap];
    const drop: Capability[] = !on && cap === "web" ? [cap, "reading" as Capability] : [cap];
    caps = on ? [...new Set([...caps, ...add])] : caps.filter((c) => !drop.includes(c));
  }

  async function submit(e: Event) {
    e.preventDefault();
    if (busy || !connectionsReady || connectionLoadError) return;
    const bot = {
      name, purpose, instructions, capabilities: caps, memory_policy: memoryPolicy,
      skills: skill ? [skill] : [], connection_agent_id: connection,
    };
    errorKey = validateNewBot(bot);
    errorDetail = "";
    if (errorKey) return;
    busy = true;
    try {
      const view = await createBot(bot);
      await loadRoster(true);
      clearDraft();
      oncreated(view);
    } catch (err) {
      errorKey = "assist.bots.create.err_failed";
      errorDetail = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<form class="create-bot surface-card" onsubmit={submit} aria-busy={busy}>
  <header>
    <h2>{$t("assist.bots.create.title")}</h2>
    <p>{$t("assist.bots.create.lede")}</p>
  </header>

  <section class="connection-first" aria-labelledby={connectionHint}>
    <h3 id={connectionHint}>{$t("assist.bots.create.connection")}</h3>
    {#if !connectionsReady}
      <p class="hint" role="status">{$t("llm.models.loading")}</p>
    {:else if connectionLoadError}
      <p id={`${connectionHint}-status`} class="hint" role="alert">{$t("llm.surface.roster_load_error")}</p>
      <button type="button" class="button" onclick={retryConnections}>{$t("llm.surface.retry")}</button>
    {:else if connections.length === 0}
      <p id={`${connectionHint}-status`} class="hint" role="status">{$t("assist.bots.create.no_connection")}</p>
      <a class="button" href="/llm/accounts?returnTo=agent">{$t("assist.bots.create.connect_first")}</a>
      <p class="hint">{$t("assist.bots.create.draft_session")}</p>
    {:else}
      <label class="field">
        <span class="field-label">{$t("llm.surface.setup")}</span>
        <select class="input" bind:value={connection}>
          {#each connections as a (a.id)}<option value={a.id}>{a.name} · {modelLabel(a.model)}</option>{/each}
        </select>
        <span class="hint">{$t("assist.bots.create.connection_note")}</span>
      </label>
    {/if}
  </section>

  <label class="field">
    <span class="field-label">{$t("assist.bots.create.name")}</span>
    <input class="input" bind:value={name} maxlength="48" disabled={busy} required placeholder={$t("assist.bots.create.name_placeholder")} />
  </label>

  <label class="field">
    <span class="field-label">{$t("assist.bots.create.purpose")}</span>
    <textarea class="input" rows="3" maxlength="10000" bind:value={purpose} placeholder={$t("assist.bots.create.purpose_placeholder")}></textarea>
  </label>

  <fieldset class="block">
    <legend class="field-label">{$t("assist.bots.create.capabilities")}</legend>
    {#each CAPABILITIES as cap (cap)}
      <label class="cap">
        <input type="checkbox" checked={caps.includes(cap)} onchange={(e) => toggle(cap, e.currentTarget.checked)} />
        <span><strong>{$t(capabilityKey(cap))}</strong><small>{$t(capabilityHintKey(cap))}</small></span>
      </label>
    {/each}
    <p class="hint">{$t("assist.bots.create.capabilities_note")}</p>
  </fieldset>

  <label class="field">
    <span class="field-label">{$t("assist.bots.create.skill")}</span>
    <select class="input" bind:value={skill}>
      <option value="">{$t("assist.bots.create.skill_none")}</option>
      {#each getSkills() as s (s.name)}<option value={s.name}>{s.name}</option>{/each}
    </select>
    {#if getSkills().length === 0}<a class="hint link" href="/llm/skills">{$t("assist.bots.create.skill_install")}</a>{/if}
  </label>


  <details class="advanced">
    <summary>{$t("assist.bots.advanced")}</summary>
    <label class="field">
      <span class="field-label">{$t("assist.bots.create.instructions")}</span>
      <textarea class="input" rows="5" maxlength="20000" bind:value={instructions}></textarea>
    </label>
    <label class="field">
      <span class="field-label">{$t("assist.bots.memory_policy.label")}</span>
      <select class="input" bind:value={memoryPolicy}>
        <option value="remember">{$t("assist.bots.memory_policy.remember")}</option>
        <option value="ask">{$t("assist.bots.memory_policy.ask")}</option>
        <option value="read_only">{$t("assist.bots.memory_policy.read_only")}</option>
      </select>
    </label>
    {#if onadvanced}<button type="button" class="button" disabled={busy} onclick={() => onadvanced?.({ name, purpose })}>{$t("assist.bots.create.advanced_setup")}</button>{/if}
  </details>

  {#if errorKey}<p class="error" role="alert">{$t(errorKey)}{#if errorDetail}<span class="detail">{errorDetail}</span>{/if}</p>{/if}

  <div class="actions">
    <button type="submit" class="button primary" disabled={busy || !connectionsReady || !!connectionLoadError || !connection} aria-describedby={connectionsReady && (connectionLoadError || !connection) ? `${connectionHint}-status` : undefined}>{busy ? $t("assist.bots.create.creating") : $t("assist.bots.create.submit")}</button>
    <button type="button" class="button" disabled={busy} onclick={cancel}>{$t("assist.bots.cancel")}</button>
  </div>
</form>

<style>
  .connection-first { display: grid; gap: var(--space-2); padding-bottom: var(--space-4); border-bottom: 1px solid var(--separator); }
  .connection-first h3 { margin: 0; font-size: var(--text-sm); }
  .connection-first .button { justify-self: start; }
  .create-bot { display: flex; flex-direction: column; gap: var(--space-4, 16px); padding: 20px; max-width: 760px; }
  header h2 { margin: 0 0 6px; font-size: 18px; }
  header p, .hint { margin: 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .block { border: 1px solid var(--separator); border-radius: var(--radius-sm); padding: 12px; margin: 0; display: flex; flex-direction: column; gap: 10px; }
  .cap { display: flex; gap: 10px; align-items: flex-start; cursor: pointer; }
  .cap span { display: flex; flex-direction: column; gap: 2px; }
  .cap small { color: var(--text-muted); font-size: 12px; }
  .cap input:focus-visible, .input:focus-visible, .button:focus-visible, summary:focus-visible { outline: var(--focus-ring, 2px solid var(--accent-text)); outline-offset: 2px; }
  .advanced summary { cursor: pointer; padding: 8px 0; font-weight: 600; }
  .advanced { display: flex; flex-direction: column; gap: 12px; }
  .link { color: var(--accent-text); }
  .error { color: var(--error, #b42318); margin: 0; overflow-wrap: anywhere; }
  .detail { display: block; font-size: 12px; color: var(--text-muted); }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  @media (max-width: 600px) { .create-bot { padding: 14px; } }
</style>
