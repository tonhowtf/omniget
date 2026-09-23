<script lang="ts">
  /**
   * "Config health" tables: token weight of commands/rules/agents/AGENTS.md
   * (heaviest first), hooks per event with whether their program exists, and
   * MCP servers by category with complexity 1–5. Pure view of the
   * `guard_config_stats` result; the caller decides which files feed it.
   */
  import { t } from "$lib/i18n";
  import { formatTokens, type ConfigStats } from "$lib/central/guard";

  let { stats }: { stats: ConfigStats } = $props();

  type Tab = "docs" | "hooks" | "mcps";
  let tab = $state<Tab>("docs");
  let docSort = $state<"tokens" | "name" | "sections">("tokens");

  let docs = $derived(
    [...stats.docs].sort((a, b) =>
      docSort === "name"
        ? a.name.localeCompare(b.name)
        : docSort === "sections"
          ? b.sections - a.sections
          : b.tokens - a.tokens,
    ),
  );
  let maxTokens = $derived(Math.max(1, ...stats.docs.map((d) => d.tokens)));
  let events = $derived(Object.entries(stats.hooks_by_event).sort((a, b) => b[1].total - a[1].total));
  let categories = $derived(Object.entries(stats.mcp_by_category).sort((a, b) => b[1] - a[1]));

  function short(path: string): string {
    const parts = path.split(/[\\/]/);
    return parts.slice(-3).join("/");
  }

  function kb(bytes: number): string {
    return bytes >= 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${bytes} B`;
  }
</script>

<div class="stats">
  <div class="summary">
    <div class="tile">
      <span class="num">{formatTokens(stats.totals.doc_tokens)}</span>
      <span class="cap">{$t("llm.central.guard.stats_tokens_total", { n: String(stats.totals.docs) })}</span>
    </div>
    <div class="tile">
      <span class="num">{stats.totals.hooks_enabled}/{stats.totals.hooks}</span>
      <span class="cap">{$t("llm.central.guard.stats_hooks_enabled")}</span>
    </div>
    <div class="tile" class:bad={stats.totals.hooks_missing_program + stats.totals.hooks_fake_env > 0}>
      <span class="num">{stats.totals.hooks_missing_program + stats.totals.hooks_fake_env}</span>
      <span class="cap">{$t("llm.central.guard.stats_hooks_broken")}</span>
    </div>
    <div class="tile">
      <span class="num">{stats.totals.mcps_enabled}/{stats.totals.mcps}</span>
      <span class="cap">{$t("llm.central.guard.stats_mcps_enabled")}</span>
    </div>
  </div>

  <div class="tabs" role="tablist">
    {#each ["docs", "hooks", "mcps"] as const as id (id)}
      <button type="button" role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>
        {$t(`llm.central.guard.stats_tab_${id}`)}
      </button>
    {/each}
  </div>

  {#if tab === "docs"}
    {#if docs.length === 0}
      <p class="empty">{$t("llm.central.guard.stats_empty")}</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th><button type="button" class="sort" onclick={() => (docSort = "name")}>{$t("llm.central.guard.col_name")}</button></th>
            <th>{$t("llm.central.guard.col_kind")}</th>
            <th class="num-col"><button type="button" class="sort" onclick={() => (docSort = "tokens")}>{$t("llm.central.guard.col_tokens")}</button></th>
            <th class="num-col">{$t("llm.central.guard.col_words")}</th>
            <th class="num-col"><button type="button" class="sort" onclick={() => (docSort = "sections")}>{$t("llm.central.guard.col_sections")}</button></th>
            <th class="num-col">{$t("llm.central.guard.col_size")}</th>
          </tr>
        </thead>
        <tbody>
          {#each docs as d (d.path)}
            <tr>
              <td title={d.path}>
                <div class="name">{d.title ?? d.name}</div>
                <div class="path mono">{short(d.path)}</div>
              </td>
              <td>{d.kind}</td>
              <td class="num-col">
                <span class="bar" style:width="{Math.round((d.tokens / maxTokens) * 100)}%"></span>
                <span class="mono">{formatTokens(d.tokens)}</span>
              </td>
              <td class="num-col mono">{d.words}</td>
              <td class="num-col mono" class:warnc={d.sections > 20}>{d.sections}</td>
              <td class="num-col mono">{kb(d.bytes)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {:else if tab === "hooks"}
    {#if events.length > 0}
      <div class="chips">
        {#each events as [ev, c] (ev)}
          <span class="chip">{ev} <span class="mono">{c.enabled}/{c.total}</span></span>
        {/each}
      </div>
    {/if}
    {#if stats.hooks.length === 0}
      <p class="empty">{$t("llm.central.guard.stats_empty")}</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>{$t("llm.central.guard.col_event")}</th>
            <th>{$t("llm.central.guard.col_command")}</th>
            <th>{$t("llm.central.guard.col_status")}</th>
          </tr>
        </thead>
        <tbody>
          {#each stats.hooks as h, i (h.file + h.event + i)}
            <tr class:off={!h.enabled}>
              <td>
                <div class="name">{h.event}</div>
                {#if h.matcher}<div class="path mono">{h.matcher}</div>{/if}
              </td>
              <td>
                <pre class="cmd">{h.command}</pre>
                <div class="path mono" title={h.file}>{short(h.file)}{h.handler !== "command" ? ` · ${h.handler}` : ""}</div>
              </td>
              <td class="status">
                {#if !h.enabled}
                  <span class="pill">{$t("llm.central.guard.hook_disabled")}</span>
                {/if}
                {#if h.program_found === false}
                  <span class="pill bad">{$t("llm.central.guard.hook_missing_program", { program: h.program ?? "?" })}</span>
                {/if}
                {#if h.script_found === false}
                  <span class="pill bad">{$t("llm.central.guard.hook_missing_script", { script: h.script ?? "?" })}</span>
                {/if}
                {#if h.fake_env.length > 0}
                  <span class="pill bad" title={$t("llm.central.guard.hook_fake_env_hint")}>
                    {$t("llm.central.guard.hook_fake_env", { vars: h.fake_env.join(", ") })}
                  </span>
                {/if}
                {#if h.enabled && h.program_found !== false && h.script_found !== false && h.fake_env.length === 0}
                  <span class="pill">{$t("llm.central.guard.hook_ok")}</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {:else}
    {#if categories.length > 0}
      <div class="chips">
        {#each categories as [cat, n] (cat)}
          <span class="chip">{cat} <span class="mono">{n}</span></span>
        {/each}
      </div>
    {/if}
    {#if stats.mcps.length === 0}
      <p class="empty">{$t("llm.central.guard.stats_empty")}</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>{$t("llm.central.guard.col_name")}</th>
            <th>{$t("llm.central.guard.col_category")}</th>
            <th>{$t("llm.central.guard.col_transport")}</th>
            <th>{$t("llm.central.guard.col_complexity")}</th>
            <th class="num-col">{$t("llm.central.guard.col_args_env")}</th>
          </tr>
        </thead>
        <tbody>
          {#each stats.mcps as m, i (m.file + m.name + i)}
            <tr class:off={!m.enabled}>
              <td title={m.description ?? ""}>
                <div class="name">{m.name}</div>
                <div class="path mono">{m.command ?? ""}</div>
              </td>
              <td>{m.category}</td>
              <td class="mono">{m.transport}</td>
              <td>
                <span class="dots" aria-label={String(m.complexity)}>
                  {#each [1, 2, 3, 4, 5] as n (n)}<span class="d" class:on={n <= m.complexity}></span>{/each}
                </span>
              </td>
              <td class="num-col mono">{m.args}/{m.env}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}

  {#if stats.errors.length > 0}
    <ul class="errors">
      {#each stats.errors as e (e.path)}
        <li><span class="mono">{short(e.path)}</span> — {e.error}</li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .stats {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .summary {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: var(--space-2);
  }

  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-hi);
  }

  .tile .num {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: var(--text-md);
    font-weight: 600;
  }

  .tile.bad .num {
    color: var(--warning, var(--danger));
  }

  .tile .cap {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .tabs {
    display: flex;
    gap: var(--space-1);
    border-bottom: var(--hairline) solid var(--separator);
  }

  .tabs button {
    background: none;
    border: none;
    padding: var(--space-1) var(--space-2);
    font: inherit;
    font-size: var(--text-sm);
    color: var(--text-dim);
    border-bottom: 2px solid transparent;
    cursor: pointer;
  }

  .tabs button.active {
    color: inherit;
    border-bottom-color: var(--accent);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  th {
    text-align: left;
    font-weight: 500;
    font-size: var(--text-xs);
    color: var(--text-dim);
    padding: var(--space-1) var(--space-2);
    border-bottom: var(--hairline) solid var(--separator);
  }

  td {
    padding: var(--space-1) var(--space-2);
    border-bottom: var(--hairline) solid var(--separator);
    vertical-align: top;
  }

  .num-col {
    text-align: right;
    white-space: nowrap;
    position: relative;
  }

  .sort {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }

  .bar {
    position: absolute;
    right: 0;
    top: 20%;
    height: 60%;
    background: var(--accent-soft);
    border-radius: var(--radius-xs);
    z-index: 0;
  }

  .num-col .mono {
    position: relative;
    z-index: 1;
  }

  .name {
    font-weight: 500;
  }

  .path {
    font-size: var(--text-xs);
    color: var(--text-dim);
    word-break: break-all;
  }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .warnc {
    color: var(--warning, var(--danger));
  }

  .cmd {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 6em;
    overflow: auto;
  }

  .status {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .pill,
  .chip {
    display: inline-flex;
    gap: var(--space-1);
    padding: 0 var(--space-2);
    border-radius: var(--radius-full);
    border: var(--hairline) solid var(--separator);
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }

  .pill.bad {
    border-color: transparent;
    background: color-mix(in srgb, var(--warning, var(--danger)) 16%, transparent);
    color: var(--warning, var(--danger));
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  tr.off {
    opacity: 0.55;
  }

  .dots {
    display: inline-flex;
    gap: 3px;
  }

  .d {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--separator);
  }

  .d.on {
    background: var(--accent);
  }

  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .errors {
    margin: 0;
    padding-left: var(--space-4);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
