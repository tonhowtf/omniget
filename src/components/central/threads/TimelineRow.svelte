<script lang="ts" module>
  import type { PlanRow } from "$lib/central/threads/types";
  import type { Row } from "$lib/central/threads-ui/timeline";

  export type TimelineActions = {
    openFile: (path: string) => void;
    openDiff: (turnOrdinal: number | null, path?: string | null) => void;
    editFrom: (row: Extract<Row, { kind: "user" }>) => void;
    forkFrom: (turnOrdinal: number) => void;
    toggleFold: (id: string, open: boolean) => void;
    toggleGroup: (id: string) => void;
    implementPlan: (plan: PlanRow) => void;
    refinePlan: (plan: PlanRow) => void;
    implementPlanNew: (plan: PlanRow) => void;
    quote: (text: string) => void;
    canRevert: boolean;
    canFork: boolean;
    busy: boolean;
  };
</script>

<script lang="ts">
  // One timeline row, by kind. Rows are keyed by stable ids from
  // `deriveRows`, so the live row and folds keep their DOM while text streams.
  import { t } from "$lib/i18n";
  import ChangedFilesCard from "$components/central/vcs/ChangedFilesCard.svelte";
  import { formatClock, formatCost, formatDuration, formatFull, formatTokens } from "$lib/central/threads-ui/format";
  import { recallable } from "$lib/central/threads-ui/composer";
  import type { SummaryPart } from "$lib/central/threads-ui/work";
  import { entryVerbKey } from "$lib/central/threads-ui/work";
  import Markdown from "./Markdown.svelte";
  import WorkEntryRow from "./WorkEntryRow.svelte";
  import PlanCard from "./PlanCard.svelte";
  import Elapsed from "./Elapsed.svelte";
  import Icon from "./Icon.svelte";

  let {
    row,
    cwd,
    actions,
    expandedIds,
  }: { row: Row; cwd: string | null; actions: TimelineActions; expandedIds: Set<string> } = $props();

  let reasoningOpen = $state(false);
  let copied = $state(false);
  let showFull = $state(false);

  function summaryText(parts: SummaryPart[]): string {
    const words = parts.map((p, i) => {
      const s = $t(`llm.central.threads.summary.${p.key}_${p.count === 1 ? "one" : "other"}`, { count: p.count }) as string;
      return i === 0 ? s : s.charAt(0).toLowerCase() + s.slice(1);
    });
    if (words.length <= 1) return words[0] ?? "";
    return `${words.slice(0, -1).join(", ")} ${$t("llm.central.threads.summary.and")} ${words.at(-1)}`;
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => (copied = false), 1400);
    } catch {
      /* denied */
    }
  }

  function longMessage(text: string): boolean {
    return text.length > 600 || text.split("\n").length > 8;
  }
</script>

{#if row.kind === "user"}
  {@const text = recallable(row.message.text)}
  <div class="user-row">
    <h3 class="sr-only">{$t("llm.central.threads.you")}</h3>
    <div class="bubble" class:clamped={longMessage(text) && !showFull}>
      <p class="user-text">{text}</p>
    </div>
    {#if longMessage(text)}
      <button type="button" class="show-full" onclick={() => (showFull = !showFull)}>{showFull ? $t("llm.central.threads.show_less") : $t("llm.central.threads.show_full")}</button>
    {/if}
    <div class="user-foot">
      <time datetime={row.message.createdAt} title={formatFull(row.message.createdAt)}>{formatClock(row.message.createdAt)}</time>
      {#if row.revertTo != null && actions.canRevert}
        <button type="button" class="ghost" disabled={actions.busy} title={$t("llm.central.threads.edit_from_here")} onclick={() => actions.editFrom(row)}>
          <Icon name="arrow-counter-clockwise" size={12} />{$t("llm.central.threads.edit_from_here")}
        </button>
      {/if}
      {#if row.turn && actions.canFork}
        <button type="button" class="ghost" title={$t("llm.central.threads.fork_here")} onclick={() => actions.forkFrom(row.turn!.turn.ordinal)}>
          <Icon name="arrows-split" size={12} />{$t("llm.central.threads.fork_here")}
        </button>
      {/if}
      <button type="button" class="ghost" aria-label={$t("llm.central.threads.copy")} onclick={() => copy(text)}><Icon name="copy" size={12} /></button>
    </div>
  </div>
{:else if row.kind === "assistant"}
  <div class="assistant" class:commentary={!row.terminal}>
    <h3 class="sr-only">{$t("llm.central.threads.agent")}</h3>
    {#if row.message.text}
      <Markdown text={row.message.text} streaming={row.message.streaming} onopenfile={actions.openFile} />
    {:else if !row.message.streaming}
      <p class="muted">{$t("llm.central.threads.empty_response")}</p>
    {/if}
    {#if row.message.streaming}<span class="caret" aria-hidden="true"></span>{/if}
  </div>
{:else if row.kind === "reasoning"}
  <div class="reasoning">
    <button type="button" class="reason-head" aria-expanded={reasoningOpen} onclick={() => (reasoningOpen = !reasoningOpen)}>
      <Icon name="brain" size={13} />
      <span class="reason-label" class:shimmer={row.message.streaming}>{row.message.streaming ? $t("llm.central.threads.thinking") : $t("llm.central.threads.thought")}</span>
      {#if !reasoningOpen && row.preview}<span class="preview">{row.preview}</span>{/if}
      <span class="chev" class:open={reasoningOpen}><Icon name="caret-right" size={10} /></span>
    </button>
    {#if reasoningOpen}<div class="reason-body"><Markdown text={row.message.text} streaming={row.message.streaming} /></div>{/if}
  </div>
{:else if row.kind === "work"}
  <WorkEntryRow
    entry={row.entry}
    depth={row.depth}
    {cwd}
    onopenfile={actions.openFile}
    onopendiff={(p) => actions.openDiff(null, p)}
    expanded={expandedIds.has(row.id)}
    ontoggle={() => actions.toggleGroup(row.id)}
  />
{:else if row.kind === "group"}
  <button type="button" class="toggle" class:failed={row.failed} style:--depth={row.depth} aria-expanded={row.open} onclick={() => actions.toggleGroup(row.id)}>
    <Icon name="hammer" size={13} />
    <span class="toggle-text">{summaryText(row.summary)}</span>
    {#if row.failed}<span class="fail-dot" aria-label={$t("llm.central.threads.work.failed")}></span>{/if}
    <span class="chev" class:open={row.open}><Icon name="caret-right" size={10} /></span>
  </button>
{:else if row.kind === "live"}
  <button type="button" class="toggle live" aria-expanded={row.open} onclick={() => actions.toggleGroup(row.id)}>
    <span class="live-ico"><Icon name="circle-notch" size={13} /></span>
    {#if row.current}
      <span class="toggle-text"><span class="shimmer">{$t(entryVerbKey(row.current))}</span> <span class="mono">{row.current.label}</span></span>
    {/if}
    {#if row.entries.length > 1}<span class="count">{row.entries.length}</span>{/if}
    <span class="chev" class:open={row.open}><Icon name="caret-right" size={10} /></span>
  </button>
{:else if row.kind === "fold"}
  <button type="button" class="fold" aria-expanded={row.open} onclick={() => actions.toggleFold(row.id, row.open)}>
    <span class="fold-text">
      {#if row.interrupted}
        {$t("llm.central.threads.fold.stopped", { duration: formatDuration(row.durationMs) })}
      {:else if row.failed}
        {$t("llm.central.threads.fold.failed", { duration: formatDuration(row.durationMs) })}
      {:else}
        {$t("llm.central.threads.fold.worked", { duration: formatDuration(row.durationMs) })}
      {/if}
    </span>
    {#if row.summary.length && !row.open}<span class="fold-sum">· {summaryText(row.summary)}</span>{/if}
    <span class="chev" class:open={row.open}><Icon name="caret-right" size={10} /></span>
  </button>
{:else if row.kind === "working"}
  <div class="working" aria-live="polite">
    <span class="live-ico"><Icon name="circle-notch" size={13} /></span>
    <span class="shimmer">
      {#if row.phase === "pending"}{$t("llm.central.threads.connecting")}{:else if row.phase === "thinking"}{$t("llm.central.threads.thinking")}{:else}{$t("llm.central.threads.working")}{/if}
    </span>
    <span class="muted"><Elapsed since={row.since} /></span>
  </div>
{:else if row.kind === "agents"}
  {@const running = row.entries.filter((e) => e.status === "running").length}
  <div class="agents">
    <button type="button" class="toggle" aria-expanded={row.open} onclick={() => actions.toggleGroup(row.id)}>
      <Icon name="robot" size={13} />
      <span class="toggle-text" class:shimmer={row.live}>
        {row.live ? $t("llm.central.threads.agents.live", { count: row.entries.length }) : $t("llm.central.threads.agents.done", { count: row.entries.length })}
      </span>
      {#if running}<span class="count">{$t("llm.central.threads.agents.working", { count: running })}</span>{/if}
      <span class="chev" class:open={row.open}><Icon name="caret-right" size={10} /></span>
    </button>
    {#if row.open}
      <ul class="members">
        {#each row.entries as e (e.id)}
          <li>
            <span class="m-title">{e.agent?.title ?? e.label}</span>
            <span class="m-status mono" data-status={e.status}>
              {e.status === "running" ? $t("llm.central.threads.status.working") : e.status}{e.agent?.tokens ? ` · ${formatTokens(e.agent.tokens)} tok` : ""}
            </span>
            {#if e.agent?.summary}<p class="m-sum">{e.agent.summary.split("\n")[0]}</p>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{:else if row.kind === "question"}
  <div class="question">
    <Icon name="chat-circle-dots" size={13} />
    <span>{$t("llm.central.threads.question.submitted")}</span>
    <span class="muted answer">{typeof row.input.answers === "string" ? row.input.answers : JSON.stringify(row.input.answers ?? row.input.resolution ?? "")}</span>
  </div>
{:else if row.kind === "plan"}
  <PlanCard
    plan={row.plan}
    actionable={row.actionable}
    onimplement={actions.implementPlan}
    onrefine={actions.refinePlan}
    onimplementnew={actions.implementPlanNew}
    onopenfile={actions.openFile}
  />
{:else if row.kind === "compaction"}
  <div class="compaction" role="separator"><span>{$t("llm.central.threads.compacted")}</span></div>
{:else if row.kind === "error"}
  <div class="error-row" class:warning={row.entry.kind === "warning"} role={row.entry.kind === "error" ? "alert" : undefined}>
    <Icon name={row.entry.kind === "warning" ? "warning" : "warning-circle"} size={14} />
    <div>
      <p class="err-title">{row.entry.label}</p>
      {#if row.entry.detail}<pre class="err-detail">{row.entry.detail}</pre>{/if}
    </div>
  </div>
{:else if row.kind === "meta"}
  <div class="meta">
    {#if row.files.length}
      <ChangedFilesCard
        files={row.files}
        onopenfile={(p) => actions.openDiff(row.turn.turn.ordinal, p)}
        onopendiff={() => actions.openDiff(row.turn.turn.ordinal, null)}
      />
    {/if}
    <div class="meta-line">
      {#if row.answer}
        <button type="button" class="ghost" aria-label={$t("llm.central.threads.copy")} onclick={() => copy(row.answer)}>
          <Icon name={copied ? "check" : "copy"} size={12} />
        </button>
      {/if}
      {#if row.turn.turn.completedAt}<time datetime={row.turn.turn.completedAt} title={formatFull(row.turn.turn.completedAt)}>{formatClock(row.turn.turn.completedAt)}</time>{/if}
      {#if row.usage?.model ?? row.turn.turn.model}<span class="mono">{row.usage?.model ?? row.turn.turn.model}</span>{/if}
      {#if row.usage}
        <span title={$t("llm.central.threads.usage.tooltip", { input: row.usage.inputTokens, cached: row.usage.cachedInputTokens, output: row.usage.outputTokens, reasoning: row.usage.reasoningOutputTokens })}>
          {formatTokens(row.usage.inputTokens + row.usage.cachedInputTokens + row.usage.cacheWriteTokens)} → {formatTokens(row.usage.outputTokens + row.usage.reasoningOutputTokens)} tok
        </span>
        {#if row.usage.costUsd != null}<span class="cost">{formatCost(row.usage.costUsd)}</span>{/if}
      {/if}
      {#if row.durationMs}<span>{formatDuration(row.durationMs)}</span>{/if}
      {#if row.error}<span class="err">{row.error}</span>{/if}
    </div>
  </div>
{/if}

<style>
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; margin: -1px; padding: 0; border: 0; }
  .muted { color: var(--text-muted); }
  .mono { font-family: var(--font-mono); font-size: 12px; }
  .user-row { display: flex; flex-direction: column; align-items: flex-end; gap: 4px; padding: 8px 0 4px; }
  .bubble { max-width: 80%; background: var(--accent-soft); color: var(--text); border-radius: 18px; padding: 10px 14px; position: relative; overflow: hidden; }
  .bubble.clamped { max-height: 11rem; mask-image: linear-gradient(#000 calc(100% - 28px), transparent); }
  .user-text { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-size: 14px; line-height: 1.55; user-select: text; }
  .show-full { border: 0; background: transparent; color: var(--accent-hi); font-size: 12px; cursor: pointer; }
  .user-foot { display: flex; align-items: center; gap: 4px; font-size: 11px; color: var(--text-muted); opacity: 0; transition: opacity 120ms; min-height: 22px; }
  .user-row:hover .user-foot, .user-row:focus-within .user-foot { opacity: 1; }
  @media (pointer: coarse) { .user-foot { opacity: 1; } }
  .ghost { display: inline-flex; align-items: center; gap: 4px; border: 0; background: transparent; color: var(--text-muted); font-size: 11.5px; padding: 3px 6px; border-radius: 6px; cursor: pointer; }
  .ghost:hover:not(:disabled) { background: var(--fill-2); color: var(--text); }
  .ghost:disabled { opacity: 0.4; cursor: default; }
  .assistant { padding: 6px 0 12px; position: relative; }
  .assistant.commentary { padding-bottom: 6px; }
  .caret { display: inline-block; width: 7px; height: 15px; background: var(--accent); border-radius: 2px; vertical-align: text-bottom; animation: blink 1s steps(2) infinite; }
  @keyframes blink { 50% { opacity: 0; } }
  .reasoning { padding: 2px 0; }
  .reason-head, .toggle, .fold { display: flex; align-items: center; gap: 7px; width: 100%; min-height: 28px; border: 0; background: transparent; color: var(--text-muted); font-size: 13px; padding: 3px 8px; border-radius: 8px; cursor: pointer; text-align: left; }
  .toggle { margin-left: calc(var(--depth, 0) * 18px); width: calc(100% - var(--depth, 0) * 18px); }
  .reason-head:hover, .toggle:hover, .fold:hover { background: var(--fill-1); }
  .reason-head:focus-visible, .toggle:focus-visible, .fold:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .reason-label { color: var(--text); font-weight: 500; }
  .preview { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .reason-body { margin: 2px 0 6px 28px; max-height: 24rem; overflow: auto; color: var(--text-muted); font-size: 13px; }
  .reason-body :global(.md) { color: var(--text-muted); font-size: 13px; }
  .chev { display: inline-flex; margin-left: auto; transition: transform 150ms; }
  .chev.open { transform: rotate(90deg); }
  .toggle-text { color: var(--text); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .toggle.failed .toggle-text { color: var(--text-muted); }
  .fail-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--red, #ef4444); }
  .count { font-size: 11px; background: var(--fill-2); border-radius: 999px; padding: 0 7px; color: var(--text-muted); }
  .live-ico { display: inline-flex; color: var(--blue, var(--accent)); animation: spin 1.1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .shimmer { background: linear-gradient(90deg, var(--text-muted) 0%, var(--text) 40%, var(--text-muted) 60%) 0 0 / 200% 100%; -webkit-background-clip: text; background-clip: text; color: transparent; animation: shine 2.2s steps(30) infinite; }
  @keyframes shine { from { background-position: 100% 0; } to { background-position: -100% 0; } }
  .fold { color: var(--text-muted); font-weight: 500; }
  .fold-text { color: var(--text); }
  .fold-sum { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 400; }
  .working { display: flex; align-items: center; gap: 8px; min-height: 30px; padding: 3px 8px; font-size: 13px; }
  .agents .members { list-style: none; margin: 2px 0 6px 28px; padding: 0; display: grid; gap: 4px; }
  .members li { display: grid; grid-template-columns: 1fr auto; gap: 2px 10px; font-size: 12.5px; }
  .m-title { color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .m-status { color: var(--text-muted); font-size: 11px; }
  .m-status[data-status="failed"] { color: var(--red, #ef4444); }
  .m-sum { grid-column: 1 / -1; margin: 0; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .question { display: flex; align-items: center; gap: 7px; font-size: 13px; padding: 4px 8px; color: var(--text); }
  .question .answer { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .compaction { display: flex; align-items: center; gap: 10px; color: var(--text-muted); font-size: 11.5px; margin: 10px 0; }
  .compaction::before, .compaction::after { content: ""; flex: 1; height: 1px; background: var(--separator); }
  .error-row { display: flex; gap: 8px; align-items: flex-start; padding: 8px 10px; margin: 4px 0; border-radius: 10px; color: var(--red, #ef4444); background: color-mix(in srgb, var(--red, #ef4444) 9%, transparent); font-size: 13px; }
  .error-row.warning { color: var(--orange, #f59e0b); background: color-mix(in srgb, var(--orange, #f59e0b) 10%, transparent); }
  .err-title { margin: 0; font-weight: 600; overflow-wrap: anywhere; }
  .err-detail { margin: 4px 0 0; white-space: pre-wrap; font-size: 11.5px; font-family: var(--font-mono); max-height: 10rem; overflow: auto; color: var(--text-muted); }
  .meta { display: grid; gap: 8px; padding: 2px 0 18px; }
  .meta-line { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; font-size: 11.5px; color: var(--text-muted); font-variant-numeric: tabular-nums; min-height: 22px; }
  .cost { color: var(--text); font-weight: 600; }
  .err { color: var(--red, #ef4444); }
  @media (prefers-reduced-motion: reduce) { .shimmer, .live-ico, .caret { animation: none; } .shimmer { color: var(--text); background: none; } }
</style>
