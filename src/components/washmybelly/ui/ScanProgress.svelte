<script lang="ts">
  import Icon from "./Icon.svelte";
  import { fraction } from "$lib/washmybelly/ui";
  import { formatCount } from "$lib/washmybelly/format";

  /// A scan in steps: the list of stage labels, which one runs now (index or
  /// label), a determinate bar when `total` is known, and the current item.
  /// Feed it from `onProgress(task, …)`.
  let {
    stages = [],
    current = 0,
    done = 0,
    total = 0,
    message = "",
    title = "",
  }: { stages?: string[]; current?: number | string; done?: number; total?: number; message?: string; title?: string } = $props();

  let at = $derived(typeof current === "number" ? current : Math.max(0, stages.indexOf(current)));
  let frac = $derived(fraction(done, total));
  let determinate = $derived(total > 0);
</script>

<div class="wmb-scan" role="status" aria-live="polite">
  {#if title}<div class="wmb-scan-title"><span class="spinner" aria-hidden="true"></span>{title}</div>{/if}

  {#if stages.length > 1}
    <ol class="wmb-scan-steps">
      {#each stages as s, i (i)}
        <li class:done={i < at} class:active={i === at}>
          <span class="mark" aria-hidden="true">
            {#if i < at}<Icon name="check" size={14} />{:else}<span class="num">{i + 1}</span>{/if}
          </span>
          <span class="lbl">{s}</span>
        </li>
      {/each}
    </ol>
  {/if}

  <div class="progress" class:indeterminate={!determinate} aria-hidden="true">
    <div class="progress-fill" style:width={determinate ? `${(frac * 100).toFixed(1)}%` : undefined}></div>
  </div>

  <div class="wmb-scan-foot">
    <span class="msg" title={message}>{message}</span>
    {#if determinate}<span class="count">{formatCount(done)} / {formatCount(total)}</span>{/if}
  </div>
</div>

<style>
  .wmb-scan {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .wmb-scan-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-weight: 600;
    color: var(--text);
  }
  .wmb-scan-steps {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .wmb-scan-steps li {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .wmb-scan-steps li.active {
    color: var(--text);
    font-weight: 600;
  }
  .wmb-scan-steps li.done {
    color: var(--text-muted);
  }
  .mark {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-dim);
  }
  li.active .mark {
    background: var(--accent);
    color: var(--on-accent);
  }
  li.done .mark {
    background: transparent;
    color: var(--success);
  }
  .num {
    font-size: 10px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .wmb-scan-foot {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-dim);
    min-width: 0;
  }
  .msg {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: ltr;
  }
  .count {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
</style>
