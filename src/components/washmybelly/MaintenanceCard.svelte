<script lang="ts">
  import { t } from "$lib/i18n";
  import { durationLabel, taskKey, type MaintenanceTask } from "$lib/washmybelly/maintenance";
  import Icon from "./ui/Icon.svelte";

  /// One maintenance task. Ported from "Project Item Card" (sean0205,
  let {
    task,
    busy = false,
    onRun,
  }: {
    task: MaintenanceTask;
    busy?: boolean;
    onRun: (task: MaintenanceTask) => void;
  } = $props();

  let copied = $state(false);
  let dur = $derived(durationLabel(task.minutes));

  function text(field: "title" | "what", fallback: string): string {
    const k = taskKey(task.id, field);
    const s = $t(k) as string;
    return s === k ? fallback : s;
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(task.command);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      copied = false;
    }
  }
</script>

<article class="mt-card" class:off={!task.available}>
  <header class="mt-head">
    <span class="mt-group">{$t(`washmybelly.maintenance.group.${task.group}`)}</span>
    <span class="mt-tags">
      <span class="mt-tag" class:long={task.long}><Icon name="clock" size={11} />{$t(dur.key, { count: dur.count })}</span>
      {#if task.admin}
        <span class="mt-tag"><Icon name="lock" size={11} />{$t("washmybelly.maintenance.admin")}</span>
      {/if}
    </span>
  </header>
  <h3>{text("title", task.title)}</h3>
  <p class="mt-what">{text("what", task.what)}</p>
  {#if task.long}
    <p class="mt-warn"><Icon name="warning" size={11} />{$t("washmybelly.maintenance.long_warning")}</p>
  {/if}
  <footer class="mt-foot">
    <button class="link mt-copy" type="button" onclick={() => void copy()} title={task.command}>
      <Icon name={copied ? "check" : "copy"} size={12} />{$t(copied ? "washmybelly.maintenance.copied" : "washmybelly.maintenance.copy")}
    </button>
    {#if task.available}
      <button class="btn btn-primary btn-sm" type="button" disabled={busy} onclick={() => onRun(task)}>{$t("washmybelly.maintenance.run")}</button>
    {:else}
      <span class="mt-unavail">{$t("washmybelly.maintenance.unavailable")}</span>
    {/if}
  </footer>
</article>

<style>
  .mt-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    border-radius: var(--radius-lg, 12px);
    background: var(--surface-1, var(--fill-1));
    box-shadow: inset 0 0 0 0.5px var(--separator, rgb(0 0 0 / 0.1));
    min-width: 0;
  }
  .mt-card.off {
    opacity: 0.6;
  }
  .mt-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .mt-group {
    font-size: var(--text-caption);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .mt-tags {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .mt-tag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 1px 7px;
    border-radius: var(--radius-full);
    font-size: var(--text-caption);
    font-weight: 600;
    background: var(--fill-2);
    color: var(--text-muted);
  }
  .mt-tag.long {
    background: color-mix(in srgb, var(--warning, #ff9f0a) 17%, transparent);
    color: var(--warning, #ff9f0a);
  }
  h3 {
    margin: 0;
    font-size: var(--text-base, 15px);
    font-weight: 600;
    color: var(--text);
  }
  .mt-what {
    flex: 1;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.45;
  }
  .mt-warn {
    display: flex;
    align-items: center;
    gap: 5px;
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .mt-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .mt-unavail {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
