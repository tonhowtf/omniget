<script lang="ts">
  // Who is doing what, beside the canvas. The pose comes from the replica the
  // canvas already keeps (`agents`), the reason for it from the bus through
  // `world://activity`, the job from the jobs store. Nothing here polls.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { energyPercent, rowState, type AgentRow, type AgentWork } from "$lib/world/activity";
  import { getJobs, initJobsStore, type Job } from "$lib/stores/llm-jobs-store.svelte";

  interface Props {
    /** Residents as the replica sees them, refreshed by the canvas. */
    agents: AgentRow[];
    /** Centre the camera on that agent. */
    onfocus: (ent: number) => void;
    /** Put the residents to work with scripted jobs. */
    ondemo: () => void;
    demoBusy?: boolean;
    loading?: boolean;
  }
  let { agents, onfocus, ondemo, demoBusy = false, loading = false }: Props = $props();

  let work = $state<Record<string, AgentWork>>({});
  let jobs = $derived(getJobs());

  /** The job an agent is on: the newest one that has not finished. */
  function jobOf(agent: string): Job | null {
    return (
      jobs.find(
        (j) => j.agent_id === agent && (j.state === "running" || j.state === "waiting_approval" || j.state === "queued"),
      ) ?? null
    );
  }

  function firstLine(text: string): string {
    return text.split("\n")[0] ?? "";
  }

  onMount(() => {
    initJobsStore();
    let stop: (() => void) | null = null;
    let dead = false;
    void (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const { listen } = await import("@tauri-apps/api/event");
        const unlisten = await listen<AgentWork>("world://activity", (e) => {
          work = { ...work, [e.payload.agent]: e.payload };
        });
        if (dead) unlisten();
        else stop = unlisten;
        const now = (await invoke("world_activity")) as AgentWork[];
        const next: Record<string, AgentWork> = {};
        for (const w of now) next[w.agent] = w;
        work = { ...next, ...work };
      } catch {
        // Outside the app there is no bus: the rows still show the poses.
      }
    })();
    return () => {
      dead = true;
      stop?.();
    };
  });
</script>

<aside class="activity" aria-label={$t("world.activity.title") as string}>
  <header class="head">
    <h2>{$t("world.activity.title")}</h2>
    <button type="button" class="button" disabled={demoBusy} onclick={ondemo} title={$t("world.activity.demo_hint") as string}>
      {demoBusy ? $t("world.activity.demo_running") : $t("world.activity.demo")}
    </button>
  </header>

  {#if loading}
    <p class="empty" role="status">{$t("common.loading")}</p>
  {:else if agents.length === 0}
    <p class="empty">{$t("world.activity.empty")}</p>
  {:else}
    <ul class="rows">
      {#each agents as agent (agent.id)}
        {@const w = work[agent.name]}
        {@const state = rowState(agent, w)}
        {@const job = jobOf(agent.name)}
        {@const energy = energyPercent(agent.energy)}
        <li class="row" class:busy={state === "tool" || state === "thinking"} class:asking={state === "asking"}>
          <button type="button" class="row-main" onclick={() => onfocus(agent.id)}>
            <span class="dot" data-state={state} aria-hidden="true"></span>
            <span class="who">
              <span class="name">{agent.name}</span>
              <span class="state">{$t(`world.activity.state.${state}`)}</span>
            </span>
            <span class="energy" title={$t("world.activity.energy") as string}>
              <span class="bar"><span class="fill" class:low={energy < 25} style:width="{energy}%"></span></span>
              <span class="pct">{energy}%</span>
            </span>
          </button>
          {#if w?.working || w?.asking}
            <p class="detail mono">{w.caption || w.tool || $t("world.activity.no_tool")}</p>
          {/if}
          {#if job}
            <a class="detail job" href="/llm/jobs" title={$t("world.activity.open_job") as string}>
              <span class="kind">{job.kind}</span>
              <span class="line">{firstLine(job.prompt)}</span>
            </a>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</aside>

<style>
  .activity {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 280px;
    flex-shrink: 0;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .head h2 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
  }
  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .row + .row {
    border-top: var(--hairline) solid var(--separator);
  }
  .row.busy,
  .row.asking {
    background: var(--fill-1);
  }
  .row-main {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    background: none;
    border: 0;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .row-main:hover {
    background: var(--fill-1);
  }
  .row-main:focus-visible {
    outline: var(--focus-ring);
    outline-offset: calc(-1 * var(--focus-ring-offset));
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
    opacity: 0.5;
  }
  .dot[data-state="tool"],
  .dot[data-state="thinking"] {
    background: var(--success);
    opacity: 1;
  }
  .dot[data-state="asking"] {
    background: var(--accent);
    opacity: 1;
  }
  .dot[data-state="tired"] {
    background: var(--warning);
    opacity: 1;
  }
  .who {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .name {
    font-size: var(--text-sm);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .state {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .energy {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
  }
  .bar {
    width: 36px;
    height: 4px;
    border-radius: 2px;
    background: var(--fill-1);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--success);
  }
  .fill.low {
    background: var(--warning);
  }
  .pct {
    width: 2.6em;
    text-align: right;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .detail {
    display: flex;
    gap: var(--space-2);
    margin: 0;
    padding: 0 var(--space-3) var(--space-2) calc(var(--space-3) + 8px + var(--space-2));
    font-size: var(--text-xs);
    color: var(--text-muted);
    min-width: 0;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .job {
    color: var(--accent-text);
    text-decoration: none;
  }
  .job:hover .line {
    text-decoration: underline;
  }
  .kind {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .line {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  @media (max-width: 900px) {
    .activity {
      width: 100%;
    }
  }
</style>
