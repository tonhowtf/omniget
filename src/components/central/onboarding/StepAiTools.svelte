<script lang="ts">
  /**
   * Passo "Suas ferramentas de IA" do onboarding: detecta os CLIs de código
   * (`clitools_detect`), mostra quais têm sessões (`sessions_sources`),
   * oferece trazer as sessões recentes como threads (`threads_import_external`,
   * só leitura, idempotente) e uma stack inicial em 1 clique (MCP de fetch +
   * skill de depuração + perfil de permissões "dev") planejada pelo agentkit
   * para as ferramentas detectadas: o plano aparece antes de aplicar.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { detectTools, listTools, type ToolRow } from "$lib/central/clitools";
  import { sessionsList, sessionsSources, type SourceInfo } from "$lib/central/sessions";
  import {
    agentkitApply,
    agentkitParsePreview,
    agentkitPlan,
    agentkitTargets,
    type ApplyReport,
    type InstallPlan,
  } from "$lib/central/catalog";

  /** Stack inicial: ids do catálogo (skill + perfil dev) + MCP de fetch próprio. */
  const STARTER_CATALOG_IDS = [
    "cct:skills/development/systematic-debugging",
    "cct:settings/permissions/development-mode",
  ];
  // O item `cct:mcps/web/web-fetch` do catálogo aponta para um pacote npm que
  // não existe; o servidor oficial de fetch é o Python `mcp-server-fetch` (uvx).
  const FETCH_MCP = JSON.stringify(
    { mcpServers: { fetch: { command: "uvx", args: ["mcp-server-fetch"], description: "Fetch web pages as Markdown" } } },
    null,
    2,
  );
  const IMPORT_LIMIT = 20;

  let rows = $state<ToolRow[]>([]);
  let sources = $state<SourceInfo[]>([]);
  let loading = $state(true);
  let error = $state("");

  let importing = $state<string | null>(null);
  let imported = $state<Record<string, number>>({});
  let importMissing = $state(false);

  let planning = $state(false);
  let plan = $state<InstallPlan | null>(null);
  let applying = $state(false);
  let applied = $state<ApplyReport | null>(null);
  let targetIds = $state<string[]>([]);

  let installed = $derived(rows.filter((r) => r.detection?.installed));
  let sessionsOf = $derived(new Map(sources.filter((s) => s.found).map((s) => [s.tool, s])));
  let withSessions = $derived(installed.filter((r) => (sessionsOf.get(r.tool.id)?.sessions ?? 0) > 0));
  let planCounts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const u of plan?.units ?? []) c[u.status] = (c[u.status] ?? 0) + 1;
    return c;
  });
  let planCommands = $derived([...new Set((plan?.units ?? []).flatMap((u) => u.commands))]);

  onMount(() => {
    void (async () => {
      try {
        await detectTools(false);
        const [list, src, targets] = await Promise.all([
          listTools(),
          sessionsSources().catch(() => [] as SourceInfo[]),
          agentkitTargets().catch(() => []),
        ]);
        rows = list.tools;
        sources = src;
        const known = new Set(targets.map((x) => x.id));
        targetIds = list.tools.filter((r) => r.detection?.installed && known.has(r.tool.id)).map((r) => r.tool.id);
      } catch (e) {
        error = String(e);
      } finally {
        loading = false;
      }
    })();
  });

  async function importSessions(tool: string) {
    importing = tool;
    error = "";
    try {
      const page = await sessionsList({ tool, limit: IMPORT_LIMIT, sort: "recent" });
      let n = 0;
      for (const s of page.sessions) {
        try {
          await invoke("threads_import_external", { tool, sessionId: s.id });
          n++;
        } catch (e) {
          if (String(e).includes("not found")) {
            importMissing = true;
            break;
          }
        }
      }
      imported[tool] = n;
    } catch (e) {
      error = String(e);
    } finally {
      importing = null;
    }
  }

  async function planStack() {
    planning = true;
    error = "";
    applied = null;
    try {
      const fetchMcp = await agentkitParsePreview({ kind: "mcp", entry: "fetch.json", files: { "fetch.json": FETCH_MCP } });
      plan = await agentkitPlan({
        components: [fetchMcp],
        catalogIds: STARTER_CATALOG_IDS,
        targets: targetIds,
        scope: "global",
        policy: "skip",
      });
    } catch (e) {
      error = String(e);
    } finally {
      planning = false;
    }
  }

  async function applyStack() {
    if (!plan) return;
    applying = true;
    error = "";
    try {
      applied = await agentkitApply(plan.id);
      plan = null;
    } catch (e) {
      error = String(e);
    } finally {
      applying = false;
    }
  }
</script>

<div class="step-ai">
  <h2>{$t("llm.central.onboarding.title")}</h2>
  <p class="desc">{$t("llm.central.onboarding.desc")}</p>

  {#if loading}
    <div class="row"><span class="spinner"></span> {$t("llm.central.onboarding.detecting")}</div>
  {:else if installed.length === 0}
    <p class="desc">{$t("llm.central.onboarding.none")}</p>
    <a class="btn btn-secondary btn-sm" href="/llm/tools">{$t("llm.central.onboarding.open_tools")}</a>
  {:else}
    <ul class="tools">
      {#each installed as r (r.tool.id)}
        {@const src = sessionsOf.get(r.tool.id)}
        <li>
          <span class="dot" aria-hidden="true"></span>
          <span class="name">{r.tool.name}</span>
          <span class="ver">{r.detection?.version ?? r.detection?.app_version ?? ""}</span>
          {#if src && src.sessions > 0}
            <span class="count">{$t("llm.central.onboarding.sessions", { n: src.sessions })}</span>
            {#if imported[r.tool.id] !== undefined}
              <span class="ok">{$t("llm.central.onboarding.imported", { n: imported[r.tool.id] })}</span>
            {:else}
              <button class="btn btn-ghost btn-sm" type="button" disabled={importing !== null || importMissing} onclick={() => importSessions(r.tool.id)}>
                {importing === r.tool.id ? $t("llm.central.onboarding.importing") : $t("llm.central.onboarding.import", { n: Math.min(IMPORT_LIMIT, src.sessions) })}
              </button>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
    {#if importMissing}<p class="note">{$t("llm.central.onboarding.import_unavailable")}</p>{/if}
    {#if withSessions.length === 0}<p class="note">{$t("llm.central.onboarding.no_sessions")}</p>{/if}

    <div class="stack">
      <strong>{$t("llm.central.onboarding.stack_title")}</strong>
      <p class="note">{$t("llm.central.onboarding.stack_desc")}</p>
      {#if applied}
        <p class="ok">{$t("llm.central.onboarding.stack_done", { n: applied.installed.length, files: applied.files_written.length })}</p>
      {:else if plan}
        <p class="note">
          {#each Object.entries(planCounts) as [status, n] (status)}
            <span class="pill">{$t(`llm.central.onboarding.unit.${status}`)}: {n}</span>
          {/each}
        </p>
        {#if planCommands.length}
          <p class="note">{$t("llm.central.onboarding.stack_commands")}</p>
          {#each planCommands as c (c)}<code>{c}</code>{/each}
        {/if}
        <div class="row">
          <button class="btn btn-primary btn-sm" type="button" disabled={applying} onclick={applyStack}>
            {applying ? $t("llm.central.onboarding.applying") : $t("llm.central.onboarding.apply", { n: plan.files.filter((f) => f.action !== "unchanged").length })}
          </button>
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (plan = null)}>{$t("onboarding.back")}</button>
        </div>
      {:else}
        <button class="btn btn-secondary btn-sm" type="button" disabled={planning || targetIds.length === 0} onclick={planStack}>
          {planning ? $t("llm.central.onboarding.planning") : $t("llm.central.onboarding.stack_plan", { n: targetIds.length })}
        </button>
      {/if}
    </div>
  {/if}

  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  .step-ai {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    text-align: left;
    width: 100%;
  }
  h2 {
    margin: 0;
    text-align: center;
  }
  .desc {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-sm);
    text-align: center;
  }
  .tools {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 200px;
    overflow-y: auto;
  }
  .tools li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    flex-wrap: wrap;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success);
    flex-shrink: 0;
  }
  .name {
    font-weight: 600;
  }
  .ver,
  .count {
    color: var(--text-dim);
    font-size: var(--text-caption);
  }
  .count {
    margin-left: auto;
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: var(--space-3);
    border-radius: var(--radius-lg, 12px);
    background: var(--fill-1);
  }
  .note {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-caption);
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .pill {
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent-soft);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    word-break: break-all;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .ok {
    margin: 0;
    color: var(--success);
    font-size: var(--text-caption);
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--text-caption);
  }
</style>
