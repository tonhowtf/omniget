<script lang="ts">
  // Tasks surface: every subagent / task the thread launched, live first.
  import { t } from "$lib/i18n";
  import type { WorkEntry } from "$lib/central/threads-ui/work";
  import { formatTokens, formatAge } from "$lib/central/threads-ui/format";
  import Icon from "./Icon.svelte";

  let { tasks }: { tasks: WorkEntry[] } = $props();
  let sorted = $derived([...tasks].sort((a, b) => (a.status === "running" ? 0 : 1) - (b.status === "running" ? 0 : 1) || b.updatedAt.localeCompare(a.updatedAt)));
  let open = $state<string | null>(null);
</script>

<div class="tasks">
  {#each sorted as e (e.id)}
    <div class="task" data-status={e.status}>
      <button type="button" class="head" aria-expanded={open === e.id} onclick={() => (open = open === e.id ? null : e.id)}>
        <span class="ico"><Icon name={e.status === "running" ? "circle-notch" : e.status === "failed" ? "warning-circle" : "robot"} size={14} /></span>
        <span class="title">{e.agent?.title ?? e.label}</span>
        <span class="meta">
          {e.status === "running" ? $t("llm.central.threads.status.working") : e.status}{e.agent?.tokens ? ` · ${formatTokens(e.agent.tokens)} tok` : ""} · {formatAge(e.updatedAt)}
        </span>
      </button>
      {#if open === e.id}
        <div class="body">
          {#if e.agent?.model}<p class="muted mono">{e.agent.model}</p>{/if}
          {#if e.agent?.summary}<pre>{e.agent.summary}</pre>{/if}
          {#if e.output}<pre>{e.output}</pre>{/if}
        </div>
      {/if}
    </div>
  {:else}
    <p class="note">{$t("llm.central.threads.tasks.none")}</p>
  {/each}
</div>

<style>
  .tasks { padding: 10px; display: grid; gap: 4px; overflow: auto; height: 100%; box-sizing: border-box; align-content: start; }
  .task { border-radius: 10px; }
  .head { display: grid; grid-template-columns: auto 1fr; gap: 2px 8px; width: 100%; border: 0; background: transparent; padding: 8px; border-radius: 10px; cursor: pointer; text-align: left; color: var(--text); }
  .head:hover { background: var(--fill-1); }
  .ico { grid-row: span 2; display: inline-flex; color: var(--text-muted); padding-top: 1px; }
  [data-status="running"] .ico { color: var(--blue, var(--accent)); }
  [data-status="failed"] .ico { color: var(--red, #ef4444); }
  .title { font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: 11.5px; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .body { padding: 0 8px 8px 30px; display: grid; gap: 6px; }
  pre { margin: 0; max-height: 14rem; overflow: auto; font-family: var(--font-mono); font-size: 11.5px; white-space: pre-wrap; background: var(--fill-1); border-radius: 8px; padding: 8px; }
  .muted { margin: 0; color: var(--text-muted); font-size: 11.5px; }
  .mono { font-family: var(--font-mono); }
  .note { color: var(--text-muted); font-size: 13px; text-align: center; margin-top: 24px; }
</style>
