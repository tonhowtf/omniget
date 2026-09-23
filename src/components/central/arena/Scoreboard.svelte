<script lang="ts">
  // Placar por driver/modelo ao longo de todas as arenas: vitórias, votos,
  // testes que passaram, falhas e médias de tempo, custo, tokens e linhas.
  import { t } from "$lib/i18n";
  import { driverName, driverTint } from "$lib/central/threads-ui/drivers";
  import { formatCost, formatMs, formatTokens, type ScoreRow } from "$lib/central/arena";

  let { rows }: { rows: ScoreRow[] } = $props();

  let maxWins = $derived(Math.max(1, ...rows.map((r) => r.wins)));
</script>

<div class="wrap">
  <table>
    <thead>
      <tr>
        <th>{$t("llm.central.arena.score.tool")}</th>
        <th class="n">{$t("llm.central.arena.score.runs")}</th>
        <th class="n">{$t("llm.central.arena.score.wins")}</th>
        <th class="n">{$t("llm.central.arena.score.win_rate")}</th>
        <th class="n">{$t("llm.central.arena.score.votes")}</th>
        <th class="n">{$t("llm.central.arena.score.tests")}</th>
        <th class="n">{$t("llm.central.arena.score.failures")}</th>
        <th class="n">{$t("llm.central.arena.score.avg_time")}</th>
        <th class="n">{$t("llm.central.arena.score.avg_cost")}</th>
        <th class="n">{$t("llm.central.arena.score.avg_tokens")}</th>
        <th class="n">{$t("llm.central.arena.score.avg_lines")}</th>
      </tr>
    </thead>
    <tbody>
      {#each rows as r (`${r.driver}:${r.model ?? ""}`)}
        <tr>
          <td>
            <span class="who" style:--tint={driverTint(r.driver)}>
              <span class="dot"></span>
              <span><strong>{driverName(r.driver)}</strong>{#if r.model}<span class="model"> · {r.model}</span>{/if}</span>
            </span>
          </td>
          <td class="n">{r.runs}</td>
          <td class="n">
            <span class="bar" style:--w="{(r.wins / maxWins) * 100}%"></span>{r.wins}
          </td>
          <td class="n">{r.runs ? Math.round((r.wins / r.runs) * 100) : 0}%</td>
          <td class="n">{r.votes}</td>
          <td class="n">{r.testsRun ? `${r.testsPassed}/${r.testsRun}` : "—"}</td>
          <td class="n">{r.failures}</td>
          <td class="n">{formatMs(r.avgDurationMs)}</td>
          <td class="n">{formatCost(r.avgCostUsd)}</td>
          <td class="n">{formatTokens(r.avgTokens)}</td>
          <td class="n">{r.avgLines == null ? "—" : Math.round(r.avgLines)}</td>
        </tr>
      {:else}
        <tr><td colspan="11" class="empty">{$t("llm.central.arena.score.empty")}</td></tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .wrap { overflow-x: auto; border: 1px solid var(--separator); border-radius: var(--radius-lg); }
  table { width: 100%; border-collapse: collapse; font-size: var(--text-base); }
  th { text-align: left; font-size: var(--text-xs); font-weight: 600; color: var(--text-muted); padding: 8px 10px; border-bottom: 1px solid var(--separator); white-space: nowrap; }
  td { padding: 8px 10px; border-bottom: 1px solid var(--separator); white-space: nowrap; }
  tr:last-child td { border-bottom: 0; }
  .n { text-align: right; font-variant-numeric: tabular-nums; }
  .who { display: inline-flex; align-items: center; gap: 7px; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--tint); }
  .model { color: var(--text-muted); font-family: var(--font-mono); font-size: var(--text-sm); }
  .bar { display: inline-block; vertical-align: middle; height: 6px; width: calc(var(--w) * 0.5); max-width: 50px; margin-right: 6px; border-radius: 3px; background: var(--success); }
  .empty { text-align: center; color: var(--text-muted); padding: 24px; }
</style>
