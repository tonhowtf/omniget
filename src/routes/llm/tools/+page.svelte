<script lang="ts">
  /**
   * Ferramentas (Central): instalar, atualizar e logar os CLIs de código.
   *
   * Uma leitura da tabela ao abrir (sem processo), depois uma varredura com
   * cache (`clitools_detect`) e a versão mais nova só das instaladas (cache de
   * 1 h no backend). Nada fica em polling: o único listener é o log ao vivo
   * (`clitools://progress`), que morre com a página.
   *
   * Regras que esta tela só mostra (quem garante é o backend): o comando
   * aparece antes de rodar; atualizar só roda contra instalação com dono
   * provado; o login acontece no terminal, dentro da ferramenta, e o OmniGet
   * nunca lê a credencial.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import ToolCard from "$components/central/tools/ToolCard.svelte";
  import PlanPanel from "$components/central/tools/PlanPanel.svelte";
  import RunLog from "$components/central/tools/RunLog.svelte";
  import ToolHealthPanel, { type HealthTab } from "$components/central/tools/ToolHealthPanel.svelte";
  import { cachedUpdates, checkUpdates, onUpdates, type UpdatesReport } from "$components/central/tools/config-health";
  import AcpAgents from "$components/central/tools/AcpAgents.svelte";
  import {
    detectTools,
    doctorOf,
    latestOf,
    listTools,
    loginTool,
    onProgress,
    planFor,
    runPlan,
    type DoctorReport,
    type Latest,
    type LogEntry,
    type Plan,
    type Progress,
    type ToolRow,
  } from "$lib/central/clitools";

  type Filter = "all" | "installed" | "available" | "discontinued";
  type Account = { id: string; cli: string; label: string };

  let rows = $state<ToolRow[]>([]);
  let latest = $state<Record<string, Latest>>({});
  let filter = $state<Filter>("all");
  let query = $state("");
  let scanning = $state(false);
  let loadError = $state("");
  let busyTool = $state<string | null>(null);

  let planTool = $state<ToolRow | null>(null);
  let plans = $state<Plan[]>([]);
  let doctor = $state<DoctorReport | null>(null);
  let doctorBusy = $state(false);
  let healthTool = $state<ToolRow | null>(null);
  let healthTab = $state<HealthTab>("config");
  let updates = $state<UpdatesReport | null>(null);
  let updateOf = $derived(new Map((updates?.tools ?? []).map((u) => [u.id, u])));
  let checkingUpdates = $state(false);
  let loginFor = $state<{ row: ToolRow; accounts: Account[] } | null>(null);

  let logs = $state<LogEntry[]>([]);
  let activeLog = $state<string | null>(null);
  let shownLog = $derived(logs.find((l) => l.key === activeLog) ?? null);
  let runningKeys = $derived(new Set(logs.filter((l) => l.state === "running").map((l) => l.key)));

  let visible = $derived(
    rows.filter((r) => {
      const q = query.trim().toLowerCase();
      if (q && !r.tool.name.toLowerCase().includes(q) && !r.tool.id.includes(q)) return false;
      const installed = r.detection?.installed ?? false;
      switch (filter) {
        case "installed":
          return installed;
        case "available":
          return !installed && r.tool.status !== "discontinued";
        case "discontinued":
          return r.tool.status !== "active";
        default:
          return true;
      }
    }),
  );
  let counts = $derived({
    installed: rows.filter((r) => r.detection?.installed).length,
    behind: new Set([
      ...Object.values(latest).filter((l) => l.state === "behind_latest").map((l) => l.id),
      ...(updates?.tools ?? []).filter((u) => u.update_available).map((u) => u.id),
    ]).size,
  });

  async function refreshRows() {
    const list = await listTools();
    rows = list.tools;
  }

  async function scan(force: boolean) {
    scanning = true;
    loadError = "";
    try {
      await detectTools(force);
      await refreshRows();
      await loadLatest(force);
    } catch (e) {
      loadError = String(e);
    } finally {
      scanning = false;
    }
  }

  async function loadLatest(force = false) {
    const installed = rows.filter((r) => r.detection?.installed && r.detection.binary);
    await Promise.all(
      installed.map(async (r) => {
        try {
          latest[r.tool.id] = await latestOf(r.tool.id, force);
        } catch {
          /* sem rede: fica sem "mais nova" */
        }
      }),
    );
  }

  function logFor(key: string, title: string): LogEntry {
    let entry = logs.find((l) => l.key === key);
    if (!entry) {
      entry = { key, title, command: null, lines: [], state: "running", result: null };
      logs = [entry, ...logs].slice(0, 12);
      entry = logs[0];
    } else {
      entry.lines = [];
      entry.state = "running";
      entry.result = null;
      entry.command = null;
      entry.title = title;
    }
    activeLog = key;
    return entry;
  }

  function onEvent(p: Progress) {
    const entry = logs.find((l) => l.key === p.tool_id);
    if (!entry) return;
    if (p.kind === "start" && p.command) entry.command = p.command;
    if (p.line) entry.lines.push({ stream: p.stream, text: p.line });
    if (entry.lines.length > 2000) entry.lines.splice(0, entry.lines.length - 2000);
    if (p.kind === "end") entry.state = p.state;
  }

  async function openPlan(row: ToolRow, action: "install" | "update") {
    healthTool = null;
    doctor = null;
    loginFor = null;
    busyTool = row.tool.id;
    try {
      plans = await planFor(row.tool.id, action);
      planTool = row;
    } catch (e) {
      showToast("error", String(e));
    } finally {
      busyTool = null;
    }
  }

  async function run(plan: Plan) {
    const row = planTool;
    if (!row) return;
    planTool = null;
    const entry = logFor(row.tool.id, row.tool.name);
    entry.command = plan.command.display;
    try {
      const result = await runPlan(plan.id);
      entry.result = result;
      entry.state = result.state;
      showToast(
        result.state === "failed" ? "error" : "success",
        $t(`llm.central.tools.run_state.${result.state}`) + ` · ${row.tool.name}`,
      );
    } catch (e) {
      entry.state = "failed";
      entry.lines.push({ stream: "stderr", text: String(e) });
      showToast("error", String(e));
    }
    await refreshRows();
    try {
      latest[row.tool.id] = await latestOf(row.tool.id);
    } catch {
      /* ignora */
    }
  }

  async function login(row: ToolRow) {
    planTool = null;
    healthTool = null;
    doctor = null;
    if (row.tool.config_env && (row.tool.id === "claude" || row.tool.id === "codex")) {
      try {
        const snap = (await invoke("llm_accounts_list")) as { accounts: Account[] };
        const accounts = (snap.accounts ?? []).filter((a) => a.cli === row.tool.id);
        if (accounts.length > 0) {
          loginFor = { row, accounts };
          return;
        }
      } catch {
        /* sem contas: login na instância padrão */
      }
    }
    await doLogin(row, null);
  }

  async function doLogin(row: ToolRow, account: string | null) {
    loginFor = null;
    try {
      const launch = await loginTool(row.tool.id, account);
      showToast("success", $t("llm.central.tools.login_opened", { cmd: launch.display }) as string);
      if (launch.hint) showToast("info", launch.hint);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function openHealth(row: ToolRow, tab: HealthTab) {
    planTool = null;
    loginFor = null;
    if (healthTool?.tool.id !== row.tool.id) doctor = null;
    healthTool = row;
    healthTab = tab;
  }

  async function loadDoctor() {
    const row = healthTool;
    if (!row || doctorBusy) return;
    doctorBusy = true;
    busyTool = row.tool.id;
    try {
      doctor = await doctorOf(row.tool.id);
      latest[row.tool.id] = doctor.latest;
      await refreshRows();
    } catch (e) {
      showToast("error", String(e));
      healthTab = "config";
    } finally {
      doctorBusy = false;
      busyTool = null;
    }
  }

  function openDoctor(row: ToolRow) {
    openHealth(row, "doctor");
  }

  async function refreshUpdates(force: boolean) {
    checkingUpdates = true;
    try {
      updates = await checkUpdates({ force });
    } catch {
      /* sem rede: fica o último relatório */
    } finally {
      checkingUpdates = false;
    }
  }

  function acpStart(agentId: string, title: string) {
    logFor(`acp:${agentId}`, `ACP · ${title}`);
  }

  onMount(() => {
    let stop: (() => void) | null = null;
    let stopUpdates: (() => void) | null = null;
    onProgress(onEvent).then((fn) => (stop = fn));
    onUpdates((r) => (updates = r)).then((fn) => (stopUpdates = fn));
    cachedUpdates()
      .then((r) => {
        if (r && !updates) updates = r;
      })
      .catch(() => {});
    void (async () => {
      try {
        await refreshRows();
      } catch (e) {
        loadError = String(e);
      }
      await scan(false);
      await refreshUpdates(false);
    })();
    return () => {
      stop?.();
      stopUpdates?.();
    };
  });
</script>

<svelte:head><title>{$t("llm.central.tools.title")}</title></svelte:head>

<div class="page page-wide tools-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("llm.central.tools.title")}</h1>
      <p class="page-lede">{$t("llm.central.tools.lede")}</p>
    </div>
    <div class="head-actions">
      <span class="tag">{$t("llm.central.tools.count_installed", { n: counts.installed })}</span>
      {#if counts.behind > 0}
        <span class="tag tag-warning">{$t("llm.central.tools.count_behind", { n: counts.behind })}</span>
      {/if}
      <button class="button" type="button" disabled={checkingUpdates} onclick={() => refreshUpdates(true)} title={updates ? new Date(updates.checked_at * 1000).toLocaleString() : ""}>
        {checkingUpdates ? $t("llm.central.health.checking_updates") : $t("llm.central.health.check_updates")}
      </button>
      <button class="button" type="button" disabled={scanning} onclick={() => scan(true)}>
        {scanning ? $t("llm.central.tools.scanning") : $t("llm.central.tools.refresh")}
      </button>
    </div>
  </header>

  <div class="toolbar">
    <div class="segmented" role="tablist">
      {#each ["all", "installed", "available", "discontinued"] as f (f)}
        <button
          class="segmented-btn"
          class:active={filter === f}
          role="tab"
          aria-selected={filter === f}
          type="button"
          onclick={() => (filter = f as Filter)}
        >
          {$t(`llm.central.tools.filter.${f}`)}
        </button>
      {/each}
    </div>
    <input class="input input-search" type="search" placeholder={$t("llm.central.tools.search") as string} bind:value={query} />
  </div>

  {#if loadError}<p class="err" role="alert">{loadError}</p>{/if}

  {#if planTool}
    <PlanPanel
      toolName={planTool.tool.name}
      {plans}
      busy={runningKeys.has(planTool.tool.id)}
      onrun={run}
      onclose={() => (planTool = null)}
    />
  {/if}

  {#if loginFor}
    <section class="surface-card login">
      <h2>{$t("llm.central.tools.login_pick")} · {loginFor.row.tool.name}</h2>
      <div class="login-choices">
        <button class="button" type="button" onclick={() => loginFor && doLogin(loginFor.row, null)}>
          {$t("llm.central.tools.login_default")}
        </button>
        {#each loginFor.accounts as a (a.id)}
          <button class="button" type="button" onclick={() => loginFor && doLogin(loginFor.row, a.id)}>{a.label}</button>
        {/each}
        <button class="button" type="button" onclick={() => (loginFor = null)}>{$t("llm.central.tools.close")}</button>
      </div>
    </section>
  {/if}

  {#if healthTool}
    {#key healthTool.tool.id}
      <ToolHealthPanel
        row={healthTool}
        bind:tab={healthTab}
        {doctor}
        {doctorBusy}
        ondoctor={loadDoctor}
        onclose={() => {
          healthTool = null;
          doctor = null;
        }}
      />
    {/key}
  {/if}

  {#if shownLog}
    <RunLog entry={shownLog} onclear={() => (activeLog = null)} />
  {/if}

  {#if visible.length === 0}
    <div class="empty-state">
      <div class="empty-state-title">{$t("llm.central.tools.empty")}</div>
    </div>
  {:else}
    <div class="grid">
      {#each visible as row (row.tool.id)}
        <ToolCard
          {row}
          latest={latest[row.tool.id] ?? null}
          busy={busyTool === row.tool.id}
          running={runningKeys.has(row.tool.id)}
          oninstall={() => openPlan(row, "install")}
          onupdate={() => openPlan(row, "update")}
          onlogin={() => login(row)}
          ondoctor={() => openDoctor(row)}
          update={updateOf.get(row.tool.id) ?? null}
          onconfig={() => openHealth(row, "config")}
        />
      {/each}
    </div>
  {/if}

  {#if logs.length > 1}
    <div class="log-tabs">
      <span class="dim">{$t("llm.central.tools.log_title")}:</span>
      {#each logs as l (l.key)}
        <button class="tag" class:tag-accent={l.key === activeLog} type="button" onclick={() => (activeLog = l.key)}>
          {l.title}
        </button>
      {/each}
    </div>
  {/if}

  <AcpAgents onstart={acpStart} />
</div>

<style>
  .tools-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .tools-page :global(.page-head) {
    flex-wrap: wrap;
  }
  .head-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .toolbar .input {
    width: 240px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-3);
  }
  .login {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .login h2 {
    margin: 0;
    font-size: var(--text-md);
  }
  .login-choices {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .log-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .log-tabs .tag {
    border: none;
    cursor: pointer;
  }
  .dim {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }
  .err {
    color: var(--danger);
    font-size: var(--text-sm);
  }
</style>
