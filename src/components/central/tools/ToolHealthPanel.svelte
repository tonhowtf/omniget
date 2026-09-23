<script module lang="ts">
  export type HealthTab = "config" | "security" | "doctor";
</script>

<script lang="ts">
  /**
   * Saúde da config de uma ferramenta (F8): abas Configuração (arquivos reais
   * por escopo + estatísticas do guard), Segurança (varredura do instalado)
   * e Diagnóstico (doctor). "Otimizar com um agente" cria um Job com o
   * runner escolhido e um prompt de revisão dos arquivos listados.
   *
   * Nada é aberto aqui: a lista vem de `clitools_config_files` (só caminho e
   * tamanho, credenciais puladas); quem lê conteúdo é o guard, e só os
   * caminhos que a lista mandou.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import { t, locale } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import { recentProjects, rememberProject, workspaceFolder, baseName } from "$lib/central/catalog";
  import {
    guardConfigStats,
    guardScanInstalled,
    formatTokens,
    summarize,
    type ConfigStats,
    type GuardReport,
  } from "$lib/central/guard";
  import type { DoctorReport, ToolRow } from "$lib/central/clitools";
  import ConfigStatsTable from "$components/central/guard/ConfigStatsTable.svelte";
  import GuardBadge from "$components/central/guard/GuardBadge.svelte";
  import GuardReportDialog from "$components/central/guard/GuardReportDialog.svelte";
  import DoctorPanel from "./DoctorPanel.svelte";
  import {
    byScope,
    configFiles,
    formatBytes,
    mergeStatsInput,
    optimizePrompt,
    type ConfigFiles,
  } from "./config-health";

  let {
    row,
    tab = $bindable("config"),
    doctor = null,
    doctorBusy = false,
    ondoctor,
    onclose,
  }: {
    row: ToolRow;
    tab?: HealthTab;
    doctor?: DoctorReport | null;
    doctorBusy?: boolean;
    ondoctor: () => void;
    onclose: () => void;
  } = $props();

  let project = $state<string | null>(null);
  let projects = $state<string[]>([]);
  let files = $state<ConfigFiles | null>(null);
  let loading = $state(false);
  let error = $state("");

  let statsScope = $state<"all" | "global" | "project">("all");
  let stats = $state<ConfigStats | null>(null);
  let statsBusy = $state(false);

  let reports = $state<GuardReport[]>([]);
  let scanned = $state(false);
  let scanBusy = $state(false);
  let openReport = $state<GuardReport | null>(null);
  let summary = $derived(summarize(reports));

  let agents = $derived(getAgents());
  let agentId = $state("");
  let submitting = $state(false);

  let groups = $derived(files ? byScope(files.entries) : []);
  let existing = $derived(files ? files.entries.filter((e) => e.exists) : []);

  function scopesFor(which: typeof statsScope): string[] {
    if (which === "global") return ["global"];
    if (which === "project") return ["project", "local"];
    return ["global", "project", "local"];
  }

  async function load() {
    loading = true;
    error = "";
    stats = null;
    reports = [];
    scanned = false;
    try {
      files = await configFiles(row.tool.id, project);
      await loadStats();
    } catch (e) {
      error = String(e);
      files = null;
    } finally {
      loading = false;
    }
  }

  async function loadStats() {
    if (!files) return;
    const input = mergeStatsInput(files.stats_input, scopesFor(statsScope));
    if (Object.keys(input).length === 0) {
      stats = null;
      return;
    }
    statsBusy = true;
    try {
      stats = await guardConfigStats(input, project);
    } catch (e) {
      error = String(e);
    } finally {
      statsBusy = false;
    }
  }

  async function scan() {
    if (!files) return;
    scanBusy = true;
    error = "";
    const out: GuardReport[] = [];
    try {
      for (const [scope, paths] of Object.entries(files.scan_paths)) {
        if (!paths.length) continue;
        const r = await guardScanInstalled({
          targetId: files.target ?? row.tool.id,
          scope,
          projectDir: scope === "global" ? null : project,
          paths,
        });
        out.push(...r);
      }
      const rank = { block: 0, warn: 1, ok: 2 } as const;
      out.sort((a, b) => rank[a.level] - rank[b.level] || a.score - b.score);
      reports = out;
      scanned = true;
    } catch (e) {
      error = String(e);
    } finally {
      scanBusy = false;
    }
  }

  async function pickProject() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const dir = await open({ directory: true, multiple: false });
      if (!dir || Array.isArray(dir)) return;
      setProject(String(dir));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  function setProject(dir: string | null) {
    project = dir;
    if (dir) {
      rememberProject(dir);
      projects = [dir, ...projects.filter((p) => p !== dir)];
    }
    void load();
  }

  function facts(): string[] {
    const out: string[] = [];
    if (stats) {
      out.push(`${stats.totals.docs} docs, ~${formatTokens(stats.totals.doc_tokens)} tokens`);
      if (stats.totals.hooks) out.push(`${stats.totals.hooks} hooks (${stats.totals.hooks_missing_program} sem programa, ${stats.totals.hooks_fake_env} com variável inexistente)`);
      if (stats.totals.mcps) out.push(`${stats.totals.mcps} MCPs`);
      const heavy = [...stats.docs].sort((a, b) => b.tokens - a.tokens).slice(0, 5);
      for (const d of heavy) out.push(`${d.path}: ~${formatTokens(d.tokens)} tokens`);
    }
    for (const r of reports.filter((r) => r.level !== "ok").slice(0, 8)) {
      out.push(`guard ${r.level} (${r.score}): ${r.root ?? r.label} — ${r.findings.slice(0, 3).map((f) => f.code).join(", ")}`);
    }
    return out;
  }

  async function optimize() {
    if (!files || !agentId) return;
    submitting = true;
    try {
      const prompt = optimizePrompt({ toolName: row.tool.name, files: existing, facts: facts(), locale: $locale ?? "en" });
      const job = await invoke<{ id: string }>("llm_job_submit", { agentId, prompt, workspace: project });
      showToast("success", $t("llm.central.health.optimize_started", { id: job.id.slice(0, 8) }) as string);
    } catch (e) {
      showToast("error", String(e));
    } finally {
      submitting = false;
    }
  }

  $effect(() => {
    if (agents.length && !agentId) {
      const own = agents.find((a) => a.runtime.kind === "cli" && a.runtime.cli === row.tool.id);
      agentId = (own ?? agents[0]).id;
    }
  });

  $effect(() => {
    if (tab === "doctor" && !doctor && !doctorBusy) ondoctor();
  });

  onMount(() => {
    void loadRoster();
    projects = recentProjects();
    void (async () => {
      const ws = await workspaceFolder();
      if (ws && !projects.includes(ws)) projects = [ws, ...projects];
      await load();
    })();
  });

  const TABS: HealthTab[] = ["config", "security", "doctor"];
  function runnerLabel(a: (typeof agents)[number]): string {
    const r = a.runtime;
    return r.kind === "cli" ? `${a.name} · ${r.cli}` : r.kind === "acp" ? `${a.name} · ACP` : a.name;
  }
</script>

<section class="surface-card health" aria-label={$t("llm.central.health.title")}>
  <header class="head">
    <div class="title">
      <h2>{row.tool.name}</h2>
      <div class="segmented" role="tablist">
        {#each TABS as k (k)}
          <button class="segmented-btn" class:active={tab === k} role="tab" aria-selected={tab === k} type="button" onclick={() => (tab = k)}>
            {$t(`llm.central.health.tab.${k}`)}
          </button>
        {/each}
      </div>
    </div>
    <button class="button" type="button" onclick={onclose}>{$t("llm.central.tools.close")}</button>
  </header>

  {#if tab !== "doctor"}
    <div class="project-row">
      <span class="dim">{$t("llm.central.health.project")}</span>
      <select class="input" value={project ?? ""} onchange={(e) => setProject((e.target as HTMLSelectElement).value || null)}>
        <option value="">{$t("llm.central.health.project_none")}</option>
        {#each projects as p (p)}<option value={p} title={p}>{baseName(p)}</option>{/each}
      </select>
      <button class="button" type="button" onclick={pickProject}>{$t("llm.central.health.project_pick")}</button>
      <button class="button" type="button" disabled={loading} onclick={load}>{loading ? $t("llm.central.tools.scanning") : $t("llm.central.tools.refresh")}</button>
    </div>
  {/if}

  {#if error}<p class="err" role="alert">{error}</p>{/if}

  {#if tab === "config"}
    {#if files}
      <p class="dim">
        {$t("llm.central.health.files_summary", { n: existing.length, size: formatBytes(files.total_bytes) })}
        {#if files.skipped.length}· {$t("llm.central.health.skipped", { n: files.skipped.length })}{/if}
      </p>
      {#each groups as [scope, list] (scope)}
        <h3>{$t(`llm.central.health.scope.${scope}`)}</h3>
        <ul class="files">
          {#each list as f (f.path + f.key)}
            <li class:missing={!f.exists}>
              <span class="mark" aria-hidden="true">{f.exists ? "✓" : "—"}</span>
              <span class="tag">{f.key}{f.surface ? ` · ${f.surface}` : ""}</span>
              <code title={f.template}>{f.path}</code>
              {#if f.exists && !f.container}
                <span class="dim">{f.is_dir ? $t("llm.central.health.dir_files", { n: f.files }) + (f.truncated ? "+" : "") + " · " : ""}{formatBytes(f.size)}</span>
              {/if}
              {#if f.alt}<span class="dim">{$t("llm.central.health.alt")}</span>{/if}
              {#if f.container}<span class="dim">{$t("llm.central.health.container")}</span>{/if}
            </li>
          {/each}
        </ul>
      {/each}

      <div class="stats-head">
        <h3>{$t("llm.central.health.stats")}</h3>
        <div class="segmented" role="tablist">
          {#each ["all", "global", "project"] as s (s)}
            <button class="segmented-btn" class:active={statsScope === s} type="button" role="tab" aria-selected={statsScope === s} disabled={s === "project" && !project} onclick={() => { statsScope = s as typeof statsScope; void loadStats(); }}>
              {$t(`llm.central.health.stats_scope.${s}`)}
            </button>
          {/each}
        </div>
      </div>
      {#if statsBusy}
        <p class="dim">{$t("llm.central.tools.scanning")}</p>
      {:else if stats}
        <ConfigStatsTable {stats} />
      {:else}
        <p class="dim">{$t("llm.central.health.stats_empty")}</p>
      {/if}

      <div class="optimize">
        <div>
          <strong>{$t("llm.central.health.optimize")}</strong>
          <p class="dim">{$t("llm.central.health.optimize_hint")}</p>
        </div>
        {#if agents.length === 0}
          <a class="button" href="/llm/roster">{$t("llm.central.health.optimize_no_agent")}</a>
        {:else}
          <select class="input" bind:value={agentId} aria-label={$t("llm.central.health.optimize_runner")}>
            {#each agents as a (a.id)}<option value={a.id}>{runnerLabel(a)}</option>{/each}
          </select>
          <button class="button active" type="button" disabled={submitting || !agentId || existing.length === 0} onclick={optimize}>
            {$t("llm.central.health.optimize_run")}
          </button>
          <button class="button" type="button" onclick={() => goto("/llm/jobs")}>{$t("llm.central.health.open_jobs")}</button>
        {/if}
      </div>
    {:else if loading}
      <p class="dim">{$t("llm.central.tools.scanning")}</p>
    {/if}
  {:else if tab === "security"}
    <div class="scan-head">
      <p class="dim">{$t("llm.central.health.scan_hint")}</p>
      <button class="button active" type="button" disabled={scanBusy || !files} onclick={scan}>
        {scanBusy ? $t("llm.central.health.scanning") : $t("llm.central.health.scan")}
      </button>
    </div>
    {#if scanned}
      <p class="summary">
        <span class="tag tag-success">{$t("llm.central.health.level.ok", { n: summary.ok })}</span>
        <span class="tag tag-warning">{$t("llm.central.health.level.warn", { n: summary.warn })}</span>
        <span class="tag tag-danger">{$t("llm.central.health.level.block", { n: summary.block })}</span>
      </p>
      {#if reports.length === 0}
        <p class="dim">{$t("llm.central.health.scan_empty")}</p>
      {:else}
        <ul class="reports">
          {#each reports as r, i (i)}
            <li>
              <GuardBadge report={r} compact onclick={() => (openReport = r)} />
              <button class="linkish" type="button" onclick={() => (openReport = r)}>{r.label}</button>
              <span class="dim">{r.kind}</span>
              {#if r.root}<code class="root">{r.root}</code>{/if}
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  {:else if doctor}
    <DoctorPanel report={doctor} {onclose} />
  {:else}
    <p class="dim">{$t("llm.central.tools.scanning")}</p>
  {/if}
</section>

{#if openReport}
  <GuardReportDialog report={openReport} onclose={() => (openReport = null)} />
{/if}

<style>
  .health {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .title {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
    font-size: var(--text-md);
  }
  h3 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
  }
  .project-row,
  .stats-head,
  .scan-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .stats-head,
  .scan-head {
    justify-content: space-between;
  }
  .project-row .input {
    max-width: 260px;
  }
  .files,
  .reports {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--text-sm);
  }
  .files li,
  .reports li {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex-wrap: wrap;
    min-width: 0;
  }
  .files li.missing {
    opacity: 0.55;
  }
  .mark {
    width: 1em;
    color: var(--text-dim);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    word-break: break-all;
    user-select: text;
  }
  .root {
    color: var(--text-dim);
  }
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-caption);
  }
  .optimize {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    padding: var(--space-3);
    border-radius: var(--radius-md, 10px);
    background: var(--accent-soft);
  }
  .optimize > div {
    flex: 1 1 240px;
  }
  .optimize .input {
    max-width: 260px;
  }
  .summary {
    display: flex;
    gap: 6px;
    margin: 0;
  }
  .linkish {
    border: none;
    background: none;
    padding: 0;
    color: var(--text);
    cursor: pointer;
    font: inherit;
    text-align: left;
  }
  .linkish:hover {
    color: var(--accent);
  }
  .err {
    color: var(--danger);
    font-size: var(--text-sm);
    margin: 0;
  }
</style>
