<script lang="ts">
  /**
   * Performance of each agent over the last 30 days, from runs, issues and
   * the ledger (`staff.scorecards`): runs, share that moved their issue,
   * issues finished, reviews sent back, spend and cost per finished issue.
   */
  import { t } from "$lib/i18n";
  import { onCompanyChange, rpc } from "$lib/company/rpc";
  import { onMount } from "svelte";

  type Scorecard = {
    staff_id: string; name: string; role: string; runs: number; succeeded: number; failed: number;
    with_progress: number; issues_done: number; reviews_sent_back: number; cost_usd: number;
    tokens: number; avg_run_s: number; cost_per_done_usd: number;
  };

  let { company }: { company: string } = $props();
  let cards = $state<Scorecard[]>([]);

  async function load() {
    try {
      cards = await rpc<Scorecard[]>("staff.scorecards", { company });
    } catch {
      cards = [];
    }
  }

  onMount(() => {
    void load();
    const un = onCompanyChange(company, (b) => {
      if (b.some((c) => c.entity === "run" || c.entity === "cost")) void load();
    });
    return () => void un.then((f) => f());
  });

  const pct = (a: number, b: number) => (b > 0 ? Math.round((a / b) * 100) : 0);
</script>

{#if cards.some((c) => c.runs > 0)}
  <section>
    <h3 class="group-label">{$t("company.team.scorecards.title")}</h3>
    <div class="group">
      {#each cards as c (c.staff_id)}
        <div class="sc-row">
          <span class="who"><span class="nm">{c.name}</span><span class="sub">{c.role}</span></span>
          <span class="col"><span class="col-label">{$t("company.team.scorecards.runs")}</span><span class="col-val">{c.runs}</span></span>
          <span class="col"><span class="col-label">{$t("company.team.scorecards.progress")}</span><span class="col-val">{pct(c.with_progress, c.runs)}%</span></span>
          <span class="col"><span class="col-label">{$t("company.team.scorecards.done")}</span><span class="col-val">{c.issues_done}</span></span>
          <span class="col"><span class="col-label">{$t("company.team.scorecards.sent_back")}</span><span class="col-val">{c.reviews_sent_back}</span></span>
          <span class="col"><span class="col-label">{$t("company.team.scorecards.failed")}</span><span class="col-val">{c.failed}</span></span>
          <span class="col"><span class="col-label">{$t("company.team.scorecards.cost")}</span><span class="col-val">US$ {c.cost_usd.toFixed(4)}</span></span>
          <span class="col"><span class="col-label">{$t("company.team.scorecards.per_done")}</span><span class="col-val">{c.issues_done ? `US$ ${c.cost_per_done_usd.toFixed(4)}` : "—"}</span></span>
        </div>
      {/each}
    </div>
    <p class="hint">{$t("company.team.scorecards.hint")}</p>
  </section>
{/if}

<style>
  .sc-row { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) var(--space-3); }
  .sc-row + .sc-row { border-top: var(--hairline) solid var(--separator); }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .nm { font-weight: 600; font-size: var(--text-sm); }
  .sub { font-size: var(--text-xs); color: var(--text-dim); }
  .col { display: flex; flex-direction: column; min-width: 72px; }
  .col-label { font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-faint); }
  .col-val { font-size: var(--text-xs); color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .hint { font-size: var(--text-xs); color: var(--text-dim); margin: var(--space-1) var(--space-1) 0; }
  @media (max-width: 720px) { .col:nth-child(n + 5) { display: none; } }
</style>
