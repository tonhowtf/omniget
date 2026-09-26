<script lang="ts">
  /**
   * What the specialists did in this room: delegated tasks (who asked whom,
   * what, state, result) and member runs. Collapsed by default; the answers
   * themselves stay in the transcript with their authors.
   */
  import { t } from "$lib/i18n";
  import { getAgent, getRoomActivity } from "$lib/stores/llm-store.svelte";

  let { roomId }: { roomId: string } = $props();

  let activity = $derived(getRoomActivity(roomId));
  let tasks = $derived([...activity.tasks].reverse());
  let running = $derived(activity.runs.filter((r) => r.state === "running"));

  function name(id: string): string {
    return getAgent(id)?.name ?? id;
  }
</script>

{#if activity.tasks.length > 0 || activity.runs.length > 0}
  <details class="activity">
    <summary>
      {$t("assist.groups.activity_title")}
      <span class="count">{$t("assist.groups.activity_count", { tasks: activity.tasks.length, running: running.length })}</span>
    </summary>
    {#if tasks.length === 0}
      <p class="dim">{$t("assist.groups.activity_no_tasks")}</p>
    {:else}
      <ol class="task-list">
        {#each tasks as task (task.id)}
          <li>
            <div class="task-head">
              <span class="who">{name(task.creator_bot)} → {name(task.recipient)}</span>
              <span class="state state-{task.state}">{$t(`assist.groups.task_${task.state}`)}</span>
            </div>
            <p class="question">{task.question}</p>
            {#if task.result}
              <details class="result">
                <summary>{$t("assist.groups.task_result")}</summary>
                <p>{task.result}</p>
              </details>
            {/if}
            {#if task.error}<p class="error">{task.error}</p>{/if}
          </li>
        {/each}
      </ol>
    {/if}
  </details>
{/if}

<style>
  .activity {
    margin: var(--space-2) 0;
    padding: var(--space-2) var(--space-3);
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    background: var(--fill-1);
    font-size: var(--text-sm);
  }
  .activity > summary { cursor: pointer; color: var(--text-muted, var(--text-dim)); display: flex; gap: var(--space-2); flex-wrap: wrap; }
  .activity summary:focus-visible { outline: var(--focus-ring); }
  .count { color: var(--text-dim); font-size: var(--text-caption); }
  .task-list { margin: var(--space-2) 0 0; padding-left: var(--space-4); display: flex; flex-direction: column; gap: var(--space-2); }
  .task-head { display: flex; gap: var(--space-2); flex-wrap: wrap; align-items: baseline; }
  .who { font-weight: 600; color: var(--text); }
  .state { font-size: var(--text-caption); color: var(--text-dim); }
  .state-failed, .state-interrupted { color: var(--danger); }
  .state-completed { color: var(--success, var(--text)); }
  .question, .result p { margin: 2px 0 0; color: var(--text); overflow-wrap: anywhere; white-space: pre-wrap; }
  .result summary { cursor: pointer; color: var(--text-dim); font-size: var(--text-caption); }
  .error { margin: 2px 0 0; color: var(--danger); overflow-wrap: anywhere; }
  .dim { color: var(--text-dim); margin: var(--space-2) 0 0; }
</style>
