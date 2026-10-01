<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/washmybelly/api";
  import type { PlanPreview, Progress, ReceiptSummary } from "$lib/washmybelly/api";
  import { errorText, explainDetail, formatBytes, formatCount, shortenPath } from "$lib/washmybelly/format";
  import { confirmKey, execSteps, fraction, freesSpace, groupRows, hasRisk, reviewTitleKey, riskRows, splitByStatus } from "$lib/washmybelly/ui";
  import { onProgress } from "./progress";
  import Icon from "./Icon.svelte";
  import RiskBadge from "./RiskBadge.svelte";
  import Sheet from "./Sheet.svelte";

  let {
    kind,
    itemIds,
    open = $bindable(false),
    onDone,
    onRestored,
    groupLabel = (key: string) => key,
    title,
    confirmLabel,
  }: {
    kind: string;
    itemIds: string[];
    open?: boolean;
    onDone?: (r: ReceiptSummary) => void;
    onRestored?: (receiptId: string) => void;
    groupLabel?: (key: string) => string;
    title?: string;
    confirmLabel?: string;
  } = $props();

  type Phase = "loading" | "review" | "running" | "result" | "restored" | "error";

  let phase = $state<Phase>("loading");
  let preview = $state<PlanPreview | null>(null);
  let result = $state<ReceiptSummary | null>(null);
  let restoreLines = $state<{ label: string; ok: boolean; detail: string }[]>([]);
  let error = $state<string | null>(null);
  let progress = $state<Progress | null>(null);
  let restoring = $state(false);
  let copyState = $state<"idle" | "busy" | "copied" | "error">("idle");
  let copyError = $state<string | null>(null);
  let showItems = $state(false);

  let risks = $derived(riskRows(preview?.by_risk));
  let groups = $derived(groupRows(preview?.by_group));
  let destructive = $derived(hasRisk(preview?.by_risk, "destructive") || hasRisk(preview?.by_risk, "extreme"));
  let extreme = $derived(hasRisk(preview?.by_risk, "extreme"));
  let frees = $derived(freesSpace(kind));
  let steps = $derived(execSteps(progress?.stage ?? null, !!preview?.needs_admin, phase === "error" && !!progress));
  let split = $derived(splitByStatus(result?.items));
  let maxGroupBytes = $derived(Math.max(1, ...groups.map((g) => g.bytes)));
  let heading = $derived(
    phase === "result" || phase === "restored"
      ? (phase === "restored" ? $t("washmybelly.plan.restored_title") : $t("washmybelly.plan.done_title"))
      : phase === "running"
        ? $t("washmybelly.plan.running_title")
        : (title ?? $t(reviewTitleKey(kind))),
  );

  $effect(() => {
    if (!open) return;
    const ids = itemIds;
    const k = kind;
    phase = "loading";
    preview = null;
    result = null;
    error = null;
    progress = null;
    restoreLines = [];
    copyState = "idle";
    showItems = false;
    if (!ids.length) {
      phase = "error";
      error = $t("washmybelly.plan.nothing_selected") as string;
      return;
    }
    let alive = true;
    api
      .plan(k, ids)
      .then((p) => {
        if (!alive) return;
        preview = p;
        phase = "review";
      })
      .catch((e) => {
        if (!alive) return;
        error = errorText(e);
        phase = "error";
      });
    return () => {
      alive = false;
    };
  });

  async function confirm() {
    if (!preview) return;
    phase = "running";
    progress = null;
    const stop = await onProgress("execute", (p) => (progress = p)).catch(() => null);
    try {
      result = await api.execute(preview.plan_id);
      phase = "result";
      onDone?.(result);
    } catch (e) {
      error = errorText(e);
      phase = "error";
    } finally {
      stop?.();
    }
  }

  async function undo() {
    if (!result) return;
    restoring = true;
    try {
      const r = await api.restore(result.receipt_id);
      restoreLines = r.items;
      phase = "restored";
      onRestored?.(result.receipt_id);
    } catch (e) {
      error = errorText(e);
    } finally {
      restoring = false;
    }
  }

  async function copyCommands() {
    if (!preview) return;
    copyState = "busy";
    copyError = null;
    try {
      const r = await api.ui<{ commands: string[] }>("plan.copy_commands", { plan_id: preview.plan_id });
      await navigator.clipboard.writeText((r.commands ?? []).join("\n"));
      copyState = "copied";
    } catch (e) {
      copyError = errorText(e);
      copyState = "error";
    }
  }

  function reason(detail: string): string {
    const x = explainDetail(detail);
    if (!x.key) return x.text;
    const friendly = $t(x.key) as string;
    return friendly === x.key ? x.text || detail : friendly;
  }

  function itemPath(it: PlanPreview["items"][number]): string {
    return it.action?.path ? shortenPath(it.action.path, 60) : (it.detail ?? "");
  }

  function close() {
    open = false;
  }
</script>

<Sheet bind:open label={heading as string} locked={phase === "running" || restoring} width={560}>
  <div class="wmb-pr">
    <header class="wmb-pr-head">
      <h3>{heading}</h3>
      {#if phase === "review" && preview}
        <p class="lede">
          {#if frees && preview.total_bytes > 0}
            <span class="big">{formatBytes(preview.total_bytes)}</span>
            <span>{$t("washmybelly.plan.items", { count: preview.item_count })}</span>
          {:else}
            <span class="big">{formatCount(preview.item_count)}</span>
            <span>{$t("washmybelly.plan.changes", { count: preview.item_count })}</span>
          {/if}
        </p>
      {/if}
    </header>

    {#if phase === "loading"}
      <div class="wmb-pr-skel" aria-busy="true">
        <div class="skeleton" style:width="40%" style:height="28px"></div>
        <div class="skeleton" style:width="70%"></div>
        <div class="skeleton" style:height="96px"></div>
      </div>
    {:else if phase === "error"}
      <div class="notice notice-danger" role="alert">
        <Icon name="error" size={18} />
        <div class="notice-body">
          <span class="notice-title">{$t("washmybelly.plan.error_title")}</span>
          <span class="notice-text">{error ? reason(error) : ""}</span>
        </div>
      </div>
    {:else if phase === "review" && preview}
      {#if risks.length}
        <div class="wmb-pr-risks">
          {#each risks as r (r.risk)}<RiskBadge risk={r.risk} count={r.count} />{/each}
        </div>
      {/if}

      {#if extreme}
        <div class="notice notice-danger">
          <Icon name="warning" size={18} />
          <div class="notice-body">
            <span class="notice-title">{$t("washmybelly.plan.extreme_title")}</span>
            <span class="notice-text">{$t("washmybelly.plan.extreme_body")}</span>
          </div>
        </div>
      {:else if destructive}
        <div class="notice notice-warning">
          <Icon name="warning" size={18} />
          <div class="notice-body">
            <span class="notice-title">{$t("washmybelly.plan.destructive_title")}</span>
            <span class="notice-text">{$t("washmybelly.plan.destructive_body", { count: preview.by_risk.destructive?.count ?? 0 })}</span>
          </div>
        </div>
      {/if}

      {#if preview.needs_admin}
        <div class="notice notice-info">
          <Icon name="lock" size={18} />
          <div class="notice-body">
            <span class="notice-title">{$t("washmybelly.plan.needs_admin")}</span>
            <span class="notice-text">{$t("washmybelly.plan.needs_admin_body")}</span>
            <button class="btn btn-sm btn-ghost wmb-pr-copy" disabled={copyState === "busy"} onclick={copyCommands}>
              <Icon name={copyState === "copied" ? "check" : "copy"} size={13} />
              {copyState === "copied" ? $t("washmybelly.plan.copied") : $t("washmybelly.plan.copy_command")}
            </button>
            {#if copyState === "error" && copyError}<span class="notice-text err">{reason(copyError)}</span>{/if}
          </div>
        </div>
      {/if}

      {#if groups.length}
        <div class="group wmb-pr-groups">
          {#each groups as g (g.group)}
            <div class="group-row">
              <div class="group-row-content">
                <span class="group-row-title">{groupLabel(g.group)}</span>
                <span class="bar" aria-hidden="true"><span style:width="{Math.max(2, (g.bytes / maxGroupBytes) * 100)}%"></span></span>
              </div>
              <div class="group-row-trailing num">
                <span class="dim">{formatCount(g.count)}</span>
                {#if frees}<span>{formatBytes(g.bytes)}</span>{/if}
              </div>
            </div>
          {/each}
        </div>
      {/if}

      <button class="wmb-pr-toggle" aria-expanded={showItems} onclick={() => (showItems = !showItems)}>
        <span class="chev" class:open={showItems} aria-hidden="true"><Icon name="chevron" size={12} /></span>
        {showItems ? $t("washmybelly.plan.hide_items") : $t("washmybelly.plan.show_items", { count: preview.item_count })}
      </button>
      {#if showItems}
        <ul class="group wmb-pr-items">
          {#each preview.items as it (it.id)}
            <li>
              <span class="text">
                <span class="label" title={it.label}>{it.label}</span>
                {#if itemPath(it)}<span class="path" title={it.action?.path ?? it.detail ?? ""}>{itemPath(it)}</span>{/if}
              </span>
              {#if it.admin}<span class="lock" title={$t("washmybelly.plan.needs_admin") as string}><Icon name="lock" size={12} /></span>{/if}
              <RiskBadge risk={it.risk} compact />
              {#if frees}<span class="num">{formatBytes(it.bytes)}</span>{/if}
            </li>
          {/each}
          {#if preview.items_truncated}
            <li class="more">{$t("washmybelly.plan.more_items", { count: preview.item_count - preview.items.length })}</li>
          {/if}
        </ul>
      {/if}
    {:else if phase === "running"}
      <ol class="wmb-pr-steps" aria-live="polite">
        {#each steps as s (s.id)}
          <li class="st-{s.state}">
            <span class="mark" aria-hidden="true">
              {#if s.state === "done"}<Icon name="check" size={18} />
              {:else if s.state === "active"}<span class="spinner"></span>
              {:else if s.state === "failed"}<Icon name="error" size={18} />
              {:else}<span class="ring"></span>{/if}
            </span>
            <span>{$t(`washmybelly.plan.step.${s.id}`)}</span>
          </li>
        {/each}
      </ol>
      <div class="progress" class:indeterminate={!progress?.total} aria-hidden="true">
        <div class="progress-fill" style:width={progress?.total ? `${fraction(progress.done, progress.total) * 100}%` : undefined}></div>
      </div>
      <p class="wmb-pr-msg" title={progress?.message ?? ""}>
        {progress?.stage === "admin" ? $t("washmybelly.plan.waiting_admin") : (progress?.message ?? $t("washmybelly.plan.starting"))}
      </p>
    {:else if phase === "result" && result}
      <div class="wmb-pr-result">
        <span class="ok-mark" class:partial={result.failed > 0} aria-hidden="true"><Icon name={result.failed > 0 ? "warning" : "check"} size={34} /></span>
        {#if frees}
          <span class="big">{formatBytes(result.bytes_freed)}</span>
          <span class="dim">{$t("washmybelly.plan.freed_label")}</span>
        {:else}
          <span class="big">{formatCount(result.done)}</span>
          <span class="dim">{$t("washmybelly.plan.applied_label")}</span>
        {/if}
        <div class="counts">
          <span class="tag tag-success">{$t("washmybelly.plan.done_count", { count: result.done })}</span>
          {#if result.skipped}<span class="tag">{$t("washmybelly.plan.skipped", { count: result.skipped })}</span>{/if}
          {#if result.failed}<span class="tag tag-danger">{$t("washmybelly.plan.failed", { count: result.failed })}</span>{/if}
        </div>
      </div>
      {#if split.failed.length || split.skipped.length}
        <ul class="group wmb-pr-items">
          {#each [...split.failed, ...split.skipped] as it (it.item_id)}
            <li>
              <span class="st-icon" class:bad={it.status === "failed"} aria-hidden="true"><Icon name={it.status === "failed" ? "error" : "skip"} size={14} /></span>
              <span class="text">
                <span class="label" title={it.label}>{it.label}</span>
                <span class="path">{reason(it.detail)}</span>
              </span>
            </li>
          {/each}
        </ul>
      {/if}
      {#if result.restorable}
        <p class="wmb-pr-hint">{$t("washmybelly.plan.undo_hint")}</p>
      {/if}
      {#if error}<p class="err" role="alert">{reason(error)}</p>{/if}
    {:else if phase === "restored"}
      <ul class="group wmb-pr-items">
        {#each restoreLines as l, i (i)}
          <li>
            <span class="st-icon" class:bad={!l.ok} class:good={l.ok} aria-hidden="true"><Icon name={l.ok ? "check" : "error"} size={14} /></span>
            <span class="text">
              <span class="label">{l.label}</span>
              <span class="path">{l.ok ? l.detail : reason(l.detail)}</span>
            </span>
          </li>
        {:else}
          <li class="more">{$t("washmybelly.plan.restored_nothing")}</li>
        {/each}
      </ul>
    {/if}
  </div>

  {#snippet footer()}
    {#if phase === "result" || phase === "restored"}
      {#if phase === "result" && result?.restorable}
        <button class="btn btn-secondary" disabled={restoring} onclick={undo}>
          {#if restoring}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name="undo" size={14} />{/if}
          {$t("washmybelly.plan.restore")}
        </button>
      {/if}
      <button class="btn btn-primary" data-autofocus onclick={close}>{$t("washmybelly.plan.close")}</button>
    {:else if phase === "error"}
      <button class="btn btn-primary" data-autofocus onclick={close}>{$t("washmybelly.plan.close")}</button>
    {:else}
      <button class="btn btn-secondary" disabled={phase === "running"} onclick={close}>{$t("washmybelly.plan.cancel")}</button>
      <button
        class="btn {extreme ? 'wmb-btn-danger' : 'btn-primary'}"
        disabled={phase !== "review" || !preview || preview.item_count === 0}
        data-autofocus
        onclick={confirm}
      >
        {#if phase === "running"}<span class="spinner" aria-hidden="true"></span>{/if}
        {confirmLabel ?? $t(confirmKey(kind))}
      </button>
    {/if}
  {/snippet}
</Sheet>

<style>
  .wmb-pr {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .wmb-pr-head h3 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 700;
    letter-spacing: var(--track-snug);
  }
  .lede {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    margin: var(--space-1) 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .big {
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .wmb-pr-skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .wmb-pr-risks {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .wmb-pr-copy {
    display: flex;
    width: fit-content;
    margin-top: var(--space-2);
    margin-left: calc(var(--space-2) * -1);
  }
  .wmb-pr-groups .group-row {
    min-height: 44px;
  }
  .bar {
    display: block;
    height: 4px;
    margin-top: 4px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    overflow: hidden;
  }
  .bar > span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
  }
  .num {
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .wmb-pr-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    padding: 2px 0;
    border: none;
    background: transparent;
    color: var(--accent-hi);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
  }
  .wmb-pr-toggle:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 2px;
    border-radius: var(--radius-xs);
  }
  .chev {
    display: flex;
    transition: transform var(--duration-fast) var(--ease-out);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .wmb-pr-items {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 240px;
    overflow-y: auto;
  }
  .wmb-pr-items li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 7px var(--space-3);
    position: relative;
  }
  .wmb-pr-items li + li::before {
    content: "";
    position: absolute;
    top: 0;
    left: var(--space-3);
    right: 0;
    height: var(--hairline);
    background: var(--separator);
  }
  .wmb-pr-items .more {
    justify-content: center;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .label {
    font-size: var(--text-sm);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .path {
    font-size: var(--text-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lock {
    display: flex;
    color: var(--info);
  }
  .st-icon {
    display: flex;
    color: var(--text-dim);
  }
  .st-icon.bad {
    color: var(--danger);
  }
  .st-icon.good {
    color: var(--success);
  }
  .wmb-pr-steps {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .wmb-pr-steps li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-base);
    color: var(--text-dim);
  }
  .wmb-pr-steps .st-active {
    color: var(--text);
    font-weight: 600;
  }
  .wmb-pr-steps .st-done {
    color: var(--text-muted);
  }
  .wmb-pr-steps .st-failed {
    color: var(--danger);
  }
  .mark {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
  }
  .st-done .mark {
    color: var(--success);
  }
  .ring {
    width: 12px;
    height: 12px;
    border-radius: var(--radius-full);
    box-shadow: inset 0 0 0 2px var(--fill-3);
  }
  .wmb-pr-msg {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wmb-pr-result {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: var(--space-2) 0 var(--space-1);
    text-align: center;
  }
  .ok-mark {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    margin-bottom: var(--space-2);
    border-radius: var(--radius-full);
    color: var(--success);
    background: color-mix(in srgb, var(--success) 14%, transparent);
  }
  .ok-mark.partial {
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 14%, transparent);
  }
  .counts {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .wmb-pr-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    text-align: center;
  }
  .err {
    color: var(--danger);
    font-size: var(--text-sm);
    margin: 0;
  }
</style>
