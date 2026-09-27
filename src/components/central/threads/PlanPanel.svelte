<script lang="ts">
  // Plan surface: the live step list (Codex plan / TodoWrite) with a
  // segmented bar, and the latest proposed plan card.
  import { t } from "$lib/i18n";
  import type { PlanRow } from "$lib/central/threads/types";
  import PlanCard from "./PlanCard.svelte";
  import Icon from "./Icon.svelte";

  let {
    steps,
    proposed,
    actionable = false,
    onimplement,
    onrefine,
    onimplementnew,
    onopenfile,
  }: {
    steps: PlanRow | null;
    proposed: PlanRow | null;
    actionable?: boolean;
    onimplement: (p: PlanRow) => void;
    onrefine: (p: PlanRow) => void;
    onimplementnew: (p: PlanRow) => void;
    onopenfile: (path: string) => void;
  } = $props();

  function norm(s: string): "done" | "active" | "pending" {
    const v = s.toLowerCase();
    if (/complete|done/.test(v)) return "done";
    if (/progress|active|running/.test(v)) return "active";
    return "pending";
  }

  let list = $derived((steps?.steps ?? []).map((s) => ({ step: s.step, st: norm(s.status) })));
  let done = $derived(list.filter((s) => s.st === "done").length);
</script>

<div class="plan-panel">
  {#if list.length}
    <section>
      <header>
        <h4>{$t("llm.central.threads.plan.steps")}</h4>
        <span class="count" class:all={done === list.length}>{done}/{list.length}</span>
      </header>
      {#if list.length >= 2}
        <div class="bar" aria-hidden="true">{#each list as s, i (i)}<span data-st={s.st}></span>{/each}</div>
      {/if}
      {#if steps?.explanation}<p class="expl">{steps.explanation}</p>{/if}
      <ol>
        {#each list as s, i (i)}
          <li data-st={s.st}>
            <Icon name={s.st === "done" ? "check-circle" : s.st === "active" ? "circle-notch" : "clock"} size={14} />
            <span>{s.step}</span>
          </li>
        {/each}
      </ol>
    </section>
  {/if}
  {#if proposed}
    <PlanCard plan={proposed} {actionable} {onimplement} {onrefine} {onimplementnew} {onopenfile} />
  {/if}
  {#if !list.length && !proposed}
    <p class="note">{$t("llm.central.threads.plan.none")}</p>
  {/if}
</div>

<style>
  .plan-panel { padding: 12px; display: grid; gap: 14px; overflow: auto; height: 100%; box-sizing: border-box; align-content: start; }
  header { display: flex; align-items: center; justify-content: space-between; }
  h4 { margin: 0; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.03em; color: var(--text-muted); }
  .count { font-size: 12px; font-variant-numeric: tabular-nums; color: var(--text-muted); }
  .count.all { color: var(--green, #22c55e); font-weight: 600; }
  .bar { display: flex; gap: 2px; margin: 8px 0; }
  .bar span { flex: 1; height: 3px; border-radius: 2px; background: var(--fill-3, var(--fill-2)); }
  .bar span[data-st="done"] { background: var(--green, #22c55e); }
  .bar span[data-st="active"] { background: var(--accent); }
  .expl { margin: 0 0 6px; font-size: 12.5px; color: var(--text-muted); }
  ol { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  li { display: flex; gap: 8px; align-items: flex-start; font-size: 13px; }
  li[data-st="done"] { color: var(--text-muted); }
  li[data-st="done"] :global(svg) { color: var(--green, #22c55e); }
  li[data-st="active"] :global(svg) { color: var(--accent); }
  .note { color: var(--text-muted); font-size: 13px; text-align: center; margin-top: 24px; }
</style>
