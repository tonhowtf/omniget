<script lang="ts">
  import { t } from "$lib/i18n";
  import { mcpErrorKey, type McpServerRow, type McpTestResult } from "$lib/llm/mcp";
  import AgentOrb from "../AgentOrb.svelte";

  let {
    rows,
    testingId = null,
    resultOf,
    onedit,
    ontest,
    ongrants,
    onremove,
  }: {
    rows: McpServerRow[];
    testingId?: string | null;
    resultOf: (id: string) => McpTestResult | null;
    onedit: (row: McpServerRow) => void;
    ontest: (id: string) => void;
    ongrants: (row: McpServerRow) => void;
    onremove: (id: string) => void;
  } = $props();

  function target(row: McpServerRow): string {
    const transport = row.config.transport;
    return transport.kind === "http"
      ? transport.url
      : [transport.command, ...transport.args].join(" ");
  }
</script>

{#if rows.length === 0}
  <p class="hint">{$t("llm.mcp.no_servers")}</p>
{:else}
  <div class="group">
    {#each rows as row (row.config.id)}
      {@const result = resultOf(row.config.id)}
      <div class="group-row mcp-row">
        <div class="group-row-content">
          <div class="group-row-title">
            {row.config.name}
            <span class="tag">{$t(`llm.mcp.transport.${row.config.transport.kind}`)}</span>
            {#if !row.config.enabled}
              <span class="tag">{$t("llm.mcp.disabled")}</span>
            {/if}
          </div>
          <div class="group-row-sub mono">{target(row)}</div>
          <div class="group-row-sub">
            {#if row.last_error}
              <span class="danger">{$t(mcpErrorKey(row.last_error.code))}</span>
              <span class="mono">{row.last_error.message}</span>
            {:else if result?.ok}
              {$t("llm.mcp.probe_ok", { count: result.tool_count, ms: result.elapsed_ms })}
            {:else if row.tools.length > 0}
              {$t("llm.mcp.tools_known", { count: row.tools.length })}
            {:else}
              {$t("llm.mcp.never_probed")}
            {/if}
          </div>
        </div>
        <div class="group-row-trailing actions">
          <button
            type="button"
            class="button"
            disabled={testingId !== null}
            onclick={() => ontest(row.config.id)}
          >
            {#if testingId === row.config.id}<AgentOrb activity="connecting" />{/if}
            {testingId === row.config.id ? $t("llm.mcp.testing") : $t("llm.mcp.test")}
          </button>
          <button type="button" class="button" onclick={() => ongrants(row)}>
            {$t("llm.mcp.grants")}
          </button>
          <button type="button" class="button" onclick={() => onedit(row)}>
            {$t("llm.mcp.edit")}
          </button>
          <button type="button" class="button danger-btn" onclick={() => onremove(row.config.id)}>
            {$t("llm.mcp.remove")}
          </button>
        </div>
      </div>
    {/each}
  </div>
{/if}

<style>
  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    overflow-wrap: anywhere;
  }

  .danger {
    color: var(--danger);
  }

  .danger-btn {
    color: var(--danger);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .group-row-title .tag {
    margin-left: var(--space-2);
  }

  @media (max-width: 720px) {
    .mcp-row {
      flex-direction: column;
      align-items: stretch;
    }

    .actions {
      justify-content: flex-start;
    }
  }
</style>
