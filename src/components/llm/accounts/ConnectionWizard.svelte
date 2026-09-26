<script lang="ts">
  // Stepper composition adapted from 21st progress-02; native Svelte controls and real IPC.
  import { onDestroy, onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/i18n';
  import { invoke } from '@tauri-apps/api/core';
  import { testConnection, parseConnectionDraft, CONNECTION_TOKEN_LIMIT, connectionMatchesIntent, diagnoseConnectionError, type ConnectionDiagnosis } from '$lib/llm/connection-setup';
  import { loadAccounts, type AccountView, type AccountsSnapshot, type CliDetected } from '$lib/stores/llm-accounts-store.svelte';
  import { loadRoster, selectAgent } from '$lib/stores/llm-store.svelte';
  import type { AgentDef } from '$lib/llm/types';
  let { initial = '' }: { initial?: string } = $props();
  type Kind = { id: string; name: string; base_url: string };
  let mode = $state('');
  let step = $state(0);
  let busy = $state(false);
  let error = $state('');
  // What the last failure means and what can be done about it (A06).
  let diagnosis = $derived<ConnectionDiagnosis | null>(error ? diagnoseConnectionError(error, mode) : null);
  let lastAction = $state<(() => Promise<void>) | null>(null);
  let clis = $state<CliDetected[]>([]);
  let accounts = $state<AccountView[]>([]);
  let cli = $state('claude');
  let account = $state('');
  let name = $state('');
  let provider = $state('openai');
  let kinds = $state<Kind[]>([]);
  let key = $state('');
  let models = $state<string[]>([]);
  let model = $state('');
  let loginOpened = $state(false);
  let verified = $state(false);
  let agentId = $state('');
  let attempt = $state('');
  let credentialId = $state('');
  let controller: AbortController | null = null;
  let hydrated = $state(false);
  onMount(() => {
    try {
      const saved = parseConnectionDraft(JSON.parse(localStorage.getItem('omniget-connection-setup') || 'null'));
      if (saved && ['subscription', 'api', 'local'].includes(saved.mode) && (!initial || initial === saved.mode)) {
        ({ mode, step, cli, account, name, provider, models, model, loginOpened, agentId, attempt, credentialId } = saved);
        void guard(async () => {
          if (mode === 'subscription') {
            clis = await invoke('llm_accounts_detect');
            accounts = (await invoke<AccountsSnapshot>('llm_accounts_list')).accounts;
          } else if (mode === 'api') kinds = await invoke('tool_keys_kinds');
        });
      } else if (initial) void choose(initial);
    } catch { /* A malformed draft can safely be discarded. */ }
    hydrated = true;
  });
  function persist() {
    const draft = { version: 1, mode, step, cli, account, name, provider, models, model, loginOpened, agentId, attempt, credentialId };
    try { localStorage.setItem('omniget-connection-setup', JSON.stringify(draft)); } catch { /* Persistence is optional when storage is unavailable. */ }
  }
  $effect(() => { if (hydrated) persist(); });
  function changed(providerChanged = false) {
    attempt = crypto.randomUUID(); agentId = ''; verified = false;
    if (providerChanged) { credentialId = ''; key = ''; models = []; model = ''; }
    persist();
  }
  onDestroy(() => controller?.abort());
  async function guard(fn: () => Promise<void>) {
    if (busy) return; busy = true; error = '';
    try { await fn(); } catch (e) { error = String(e); } finally { busy = false; }
  }
  async function choose(next: string) {
    mode = next; credentialId = ''; key = ''; step = 1; verified = false; agentId = ''; attempt = crypto.randomUUID(); models = []; model = ''; account = ''; loginOpened = false;
    await guard(async () => {
      if (mode === 'subscription') {
        const [detected, snapshot] = await Promise.all([invoke<CliDetected[]>('llm_accounts_detect'), invoke<AccountsSnapshot>('llm_accounts_list')]);
        clis = detected; accounts = snapshot.accounts;
      } else if (mode === 'api') { kinds = await invoke<Kind[]>('tool_keys_kinds'); provider = 'openai'; }
      else provider = 'ollama';
    });
  }
  async function connect() {
    lastAction = connect;
    persist();
    await guard(async () => {
      if (mode === 'subscription') {
        if (!account) {
          const snapshot = await invoke<AccountsSnapshot>('llm_accounts_create', { cli, label: name.trim() || cli, share: true, requestId: attempt });
          accounts = snapshot.accounts; account = `setup-${attempt}`; await loadAccounts();
        }
        await invoke('llm_accounts_login', { id: account }); loginOpened = true;
      } else {
        if (mode === 'api') {
          const kind = kinds.find(k => k.id === provider);
          if (!kind) throw new Error($t('llm.accounts.wizard.err_unknown_provider'));
          if (key) {
            credentialId ||= `setup-${attempt}`; persist();
            await invoke('tool_keys_save', { entry: { id: credentialId, name: name.trim() || kind.name, kind: provider, base_url: kind.base_url, key } });
            key = '';
          }
          const tested = await invoke<{ last_ok: boolean; error: string | null }>('tool_keys_test', { id: credentialId });
          if (!tested.last_ok) throw new Error(tested.error || $t('llm.accounts.wizard.err_credential_failed'));
        }
        const response = await invoke<{ models: string[] }>('llm_models_list', { provider: mode === 'api' ? credentialId : provider, refresh: true });
        models = response.models ?? []; model = models[0] ?? '';
        if (!model) throw new Error($t('llm.accounts.wizard.err_no_models'));
        step = 2;
      }
    });
  }
  function intendedAgent(): AgentDef {
        const selectedAccount = accounts.find(a => a.id === account);
        return {
          id: `connection-${attempt}`, name: name.trim() || (mode === 'subscription' ? selectedAccount?.label || cli : model), role: 'worker',
          system_prompt: '', tools: [], skills: [], budget: { tokens_per_turn: CONNECTION_TOKEN_LIMIT, max_tool_calls_per_turn: 0 },
          model: { policy: 'fixed', model: { provider: mode === 'subscription' ? (selectedAccount?.cli || cli) : mode === 'api' ? credentialId : provider, model: mode === 'subscription' ? 'default' : model } },
          runtime: mode === 'subscription' ? { kind: 'cli', cli: selectedAccount?.cli || cli, account } : { kind: 'native' },
        };
  }
  function matchesIntent(agent: AgentDef) {
    const expected = intendedAgent();
    return connectionMatchesIntent(agent, expected);
  }
  async function prepare() {
    await guard(async () => {
      const id = `connection-${attempt}`;
      persist();
      const roster = await invoke<AgentDef[]>('llm_roster_list');
      const existing = roster.find(a => a.id === id);
      if (existing && !matchesIntent(existing)) throw new Error($t('llm.accounts.wizard.err_agent_changed'));
      if (!existing) {
        const agent = intendedAgent();
        await invoke('llm_roster_create', { agent });
      }
      agentId = id; await loadRoster(true); step = 3;
    });
  }
  async function test() {
    lastAction = test;
    controller = new AbortController(); verified = false;
    await guard(async () => {
      const roster = await invoke<AgentDef[]>('llm_roster_list');
      const agent = roster.find(a => a.id === agentId);
      if (!agent || !matchesIntent(agent)) throw new Error($t('llm.accounts.wizard.err_config_changed'));
      await testConnection(agentId, controller!.signal); verified = true;
    });
  }
</script>

<section class="connection surface-card" aria-label={$t('llm.accounts.wizard.connect_title')}>
  <header><div><h2>{$t('llm.accounts.wizard.connect_title')}</h2><p>{$t('llm.accounts.wizard.connect_subtitle')}</p></div>
  {#if mode}<button class="button" disabled={busy} onclick={() => { mode = ''; step = 0; }}>{$t('llm.accounts.wizard.start_over')}</button>{/if}</header>
  {#if !mode}
    <div class="choices">
      {#each [['subscription', $t('llm.accounts.wizard.use_subscription'), 'Claude / Codex'], ['api', $t('llm.accounts.wizard.api_key'), $t('llm.accounts.wizard.provider_credential')], ['local', $t('llm.accounts.wizard.local_model'), 'Ollama / LM Studio / llama.cpp']] as choice}
        <button class="choice" onclick={() => choose(choice[0])}><strong>{choice[1]}</strong><span>{choice[2]}</span><span aria-hidden="true">→</span></button>
      {/each}
    </div>
  {:else}
    <ol class="steps" aria-label={$t('llm.accounts.wizard.progress')}>
      {#each [$t('llm.accounts.wizard.step_choose'), $t('llm.accounts.wizard.step_connect'), $t('llm.accounts.wizard.step_prepare'), $t('llm.accounts.wizard.step_test')] as label, i}
        <li class:current={step === i} class:done={step > i} aria-current={step === i ? 'step' : undefined}><span>{i + 1}</span>{label}</li>
      {/each}
    </ol>
    {#if step === 1}
      <label>{$t('llm.accounts.wizard.name_label')}<input class="input" bind:value={name} oninput={() => changed()} disabled={busy} /></label>
      {#if mode === 'subscription'}
        <p>{$t('llm.accounts.wizard.subscription_note')}</p>
        <label>{$t('llm.accounts.wizard.account_label')}<select class="input" bind:value={account} onchange={() => changed()} disabled={busy || loginOpened}><option value="">{$t('llm.accounts.wizard.account_new')}</option>{#each accounts.filter(a => !a.disabled) as a}<option value={a.id}>{a.label} · {a.cli}</option>{/each}</select></label>
        {#if !account}<label>CLI<select class="input" bind:value={cli} onchange={() => changed()} disabled={busy}><option>claude</option><option>codex</option></select></label>{/if}
        <p class="status">{clis.some(c => c.cli === (accounts.find(a => a.id === account)?.cli || cli)) ? $t('llm.accounts.wizard.cli_installed_unverified') : $t('llm.accounts.wizard.cli_not_detected')}</p>
        <div class="actions"><button class="button" disabled={busy} onclick={() => guard(async () => { clis = await invoke('llm_accounts_detect'); })}>{$t('llm.accounts.wizard.detect_again')}</button>
        <button class="button active" disabled={busy || !clis.some(c => c.cli === (accounts.find(a => a.id === account)?.cli || cli))} onclick={connect}>{$t('llm.accounts.wizard.open_terminal_login')}</button></div>
        {#if loginOpened}<p role="status">{$t('llm.accounts.wizard.terminal_opened')}</p>{/if}
        {#if loginOpened || account}<button class="button" disabled={busy} onclick={() => step = 2}>{$t('llm.accounts.wizard.signed_in_continue')}</button>{/if}
      {:else}
        <label>{$t('llm.accounts.wizard.provider_label')}<select class="input" bind:value={provider} onchange={() => changed(true)} disabled={busy}>{#if mode === 'api'}{#each kinds.filter(k => !['custom', 'newapi', 'ollama'].includes(k.id)) as k}<option value={k.id}>{k.name}</option>{/each}{:else}<option value="ollama">Ollama</option><option value="lmstudio">LM Studio</option><option value="llama-server">llama.cpp</option>{/if}</select></label>
        {#if mode === 'api'}<label>{$t('llm.accounts.wizard.api_key')}<input class="input" type="password" autocomplete="off" bind:value={key} disabled={busy} /></label><p>{$t('llm.accounts.wizard.key_vault_note')}</p>{:else}<p>{$t('llm.accounts.wizard.local_server_note')}</p><a href="/llm/local">{$t('llm.accounts.wizard.open_local_models')}</a>{/if}
        <button class="button active" disabled={busy} onclick={connect}>{$t('llm.accounts.wizard.check_connection')}</button>
      {/if}
    {:else if step === 2}
      <p>{$t('llm.accounts.wizard.new_agent_note')}</p>
      {#if mode !== 'subscription'}<label>{$t('llm.accounts.wizard.model_label')}<select class="input" bind:value={model} onchange={() => changed()}>{#each models as m}<option>{m}</option>{/each}</select></label>{/if}
      <div class="actions"><button class="button" disabled={busy} onclick={() => step = 1}>{$t('llm.accounts.wizard.back')}</button><button class="button active" disabled={busy} onclick={prepare}>{$t('llm.accounts.wizard.step_prepare')}</button></div>
    {:else}
      <p>{$t('llm.accounts.wizard.test_note')}</p>
      <p role="status">{verified ? $t('llm.accounts.wizard.verified') : busy ? $t('llm.accounts.wizard.waiting') : $t('llm.accounts.wizard.prepared_unverified')}</p>
      {#if mode === 'subscription'}<button class="button" disabled={busy} onclick={() => guard(async () => { await invoke('llm_accounts_login', { id: account }); verified = false; })}>{$t('llm.accounts.wizard.sign_in_again')}</button>{/if}
      <div class="actions"><button class="button active" disabled={busy} onclick={test}>{$t('llm.accounts.wizard.test_agent')}</button>{#if busy}<button class="button" onclick={() => controller?.abort()}>{$t('llm.accounts.wizard.cancel_test')}</button>{/if}{#if verified}<button class="button active" onclick={() => { selectAgent(agentId); goto('/llm'); }}>{$t('llm.accounts.wizard.open_conversation')}</button>{/if}</div>
    {/if}
    {#if error && diagnosis}
      <div class="problem" role="alert">
        <p class="error"><strong>{$t(diagnosis.titleKey)}</strong></p>
        <p>{$t(diagnosis.actionKey)}</p>
        <div class="actions">
          {#each diagnosis.actions as act (act)}
            {#if act === 'retry' && lastAction}<button class="button" disabled={busy} onclick={() => lastAction?.()}>{$t('assist.bots.connection.retry')}</button>
            {:else if act === 'sign_in' && mode === 'subscription' && account}<button class="button" disabled={busy} onclick={() => guard(async () => { await invoke('llm_accounts_login', { id: account }); loginOpened = true; verified = false; })}>{$t('llm.accounts.wizard.sign_in_again')}</button>
            {:else if act === 'switch'}<button class="button" disabled={busy} onclick={() => { error = ''; mode = ''; step = 0; }}>{$t('assist.bots.connection.switch')}</button>
            {:else if act === 'open_local'}<button class="button" onclick={() => goto('/llm/local')}>{$t('llm.accounts.wizard.open_local_models')}</button>
            {:else if act === 'change_model' && mode !== 'subscription' && models.length}<button class="button" disabled={busy} onclick={() => { error = ''; step = 2; }}>{$t('assist.bots.connection.change_model')}</button>{/if}
          {/each}
        </div>
        <details><summary>{$t('assist.bots.connection.details')}</summary><p class="detail">{diagnosis.detail}</p></details>
        <p>{$t('llm.accounts.wizard.preserved_retry')}</p>
      </div>
    {/if}
    {#if busy && step !== 3}<p role="status">{$t('llm.accounts.wizard.connecting')}</p>{/if}
  {/if}
</section>
<style>
  .connection { padding:24px; margin-bottom:24px; display:grid; gap:16px; }
  header,.actions { display:flex; align-items:center; justify-content:space-between; gap:12px; flex-wrap:wrap; }
  h2 { font-size:20px; margin:0 0 6px; } p { color:var(--text-muted); font-size:14px; max-width:75ch; margin:0; line-height:1.6; }
  .choices { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:12px; }
  .choice { display:flex; flex-direction:column; align-items:flex-start; gap:8px; text-align:left; padding:20px; border:1px solid var(--separator); border-radius:12px; background:var(--fill-quaternary); color:var(--text); cursor:pointer; }
  .choice:hover { background:var(--fill-tertiary); } .choice span { color:var(--text-muted); font-size:13px; }
  label { display:grid; gap:6px; font-size:13px; max-width:480px; } .input { width:100%; }
  .steps { display:flex; list-style:none; padding:0; margin:0 0 8px; gap:16px; flex-wrap:wrap; }
  .steps li { display:flex; align-items:center; gap:8px; font-size:12px; color:var(--text-muted); }
  .steps li span { display:grid; place-items:center; width:26px; height:26px; border:1px solid var(--separator); border-radius:50%; }
  .steps .current { color:var(--text); font-weight:600; } .steps .current span,.steps .done span { background:var(--fill-secondary); }
  .error { color:var(--error, #b42318); overflow-wrap:anywhere; }
  .problem { display:grid; gap:8px; padding:12px; border:1px solid var(--separator); border-radius:var(--radius-sm); }
  .problem summary { cursor:pointer; font-size:13px; } .detail { font-family:var(--font-mono); font-size:12px; overflow-wrap:anywhere; } .actions { justify-content:flex-start; }
  @media(max-width:650px) { .choices { grid-template-columns:1fr; } .connection { padding:16px; } }
</style>
