<script lang="ts">
  import { t } from "$lib/i18n";
  import { grantModeOf, toolSignature, type McpServerRow } from "$lib/llm/mcp";
  import type { AgentDef, GrantMode } from "$lib/llm/types";

  let {
    row,
    agents,
    busy = false,
    onset,
  }: {
    row: McpServerRow;
    agents: AgentDef[];
    busy?: boolean;
    onset: (agent: AgentDef, tool: string, mode: GrantMode | null) => void;
  } = $props();

  const MODES: (GrantMode | "none")[] = ["none", "auto", "ask", "deny"];

  function change(agent: AgentDef, tool: string, value: string) {
    onset(agent, tool, value === "none" ? null : (value as GrantMode));
  }
</script>

{#if agents.length === 0}
  <p class="hint">{$t("llm.mcp.no_agents")}</p>
{:else if row.tools.length === 0}
  <p class="hint">{$t("llm.mcp.no_tools")}</p>
{:else}
  <div class="wrap">
    <table class="grid">
      <thead>
        <tr>
          <th scope="col">{$t("llm.mcp.tool")}</th>
          {#each agents as agent (agent.id)}
            <th scope="col">{agent.name}</th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each row.tools as tool (tool.name)}
          <tr>
            <th scope="row">
              <span class="mono">{tool.name}</span>
              <span class="desc">{tool.description}</span>
              {#if toolSignature(tool.inputSchema)}
                <span class="mono sig">{toolSignature(tool.inputSchema)}</span>
              {/if}
            </th>
            {#each agents as agent (agent.id)}
              <td>
                <select
                  class="input mode"
                  disabled={busy}
                  aria-label={`${agent.name} · ${tool.name}`}
                  value={grantModeOf(agent, row.config.id, tool.name) ?? "none"}
                  onchange={(e) => change(agent, tool.name, e.currentTarget.value)}
                >
                  {#each MODES as mode (mode)}
                    <option value={mode}>{$t(`llm.mcp.mode.${mode}`)}</option>
                  {/each}
                </select>
              </td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .wrap {
    overflow-x: auto;
  }

  .grid {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  .grid th,
  .grid td {
    text-align: left;
    padding: var(--space-2);
    border-bottom: var(--hairline) solid var(--separator);
    vertical-align: top;
  }

  .grid thead th {
    color: var(--text-muted);
    font-weight: 600;
    white-space: nowrap;
  }

  .grid tbody th {
    font-weight: 500;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 220px;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .desc,
  .sig {
    color: var(--text-dim);
    font-size: var(--text-xs);
    font-weight: 400;
  }

  .mode {
    width: 110px;
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
