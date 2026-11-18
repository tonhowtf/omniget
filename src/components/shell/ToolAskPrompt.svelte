<script lang="ts">
  import { agentActions } from "$lib/native-agent/actions";
  import ApprovalCard from "./ApprovalCard.svelte";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t, locale } from "$lib/i18n";
  import { acceptAsk, resolveAsk, type PendingAsk } from "$lib/omni/tool-ask-queue";

  type Decision = "accept" | "accept_session" | "accept_always" | "decline" | "cancel";

  let asks = $state<PendingAsk[]>([]);
  let previews = $state<Record<string, string>>({});
  let busy = $state(false);
  let failed = $state(false);
  let now = $state(Date.now());
  let current = $derived(asks[0] ?? null);
  let preview = $derived(current ? (previews[current.toolCallId] ?? "") : "");
  let agentName = $derived(current?.agent || $t("tool_ask.agent_fallback"));
  let timing = $derived(current ? timingText(current, now, $locale) : "");

  function span(ms: number, lang: string): string {
    const seconds = Math.max(0, Math.round(ms / 1000));
    const minutes = seconds >= 90;
    try {
      return new Intl.NumberFormat(lang, { style: "unit", unit: minutes ? "minute" : "second", unitDisplay: "narrow" })
        .format(minutes ? Math.round(seconds / 60) : seconds);
    } catch {
      return minutes ? `${Math.round(seconds / 60)} min` : `${seconds} s`;
    }
  }

  function timingText(ask: PendingAsk, at: number, lang: string): string {
    if (ask.deadlineMs !== null && ask.deadlineMs > at) return $t("tool_ask.expires", { time: span(ask.deadlineMs - at, lang) });
    if (ask.askedMs !== null && at >= ask.askedMs) return $t("tool_ask.waiting", { time: span(at - ask.askedMs, lang) });
    return "";
  }

  $effect(() => {
    if (!current || (current.askedMs === null && current.deadlineMs === null)) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  function take(raw: unknown) {
    asks = acceptAsk(asks, raw).map((a) => {
      if (a.askedMs !== null) return a;
      arrived[a.toolCallId] ??= Date.now();
      return { ...a, askedMs: arrived[a.toolCallId] };
    });
    const o = raw as { tool_call_id?: unknown; preview?: unknown } | null;
    if (o && typeof o.tool_call_id === "string" && typeof o.preview === "string") {
      previews = { ...previews, [o.tool_call_id]: o.preview.slice(0, 2000) };
    }
  }

  let arrived: Record<string, number> = {};
  let guarded = $state(false);
  let guardedFor = "";

  async function sync() {
    const waiting = await invoke<unknown[]>("llm_tool_asks_pending").catch(() => null);
    if (!waiting) return;
    for (const ask of waiting) take(ask);
    const timed = waiting.flatMap((ask) => acceptAsk([], ask));
    asks = asks.map((a) => {
      const fresh = timed.find((f) => f.toolCallId === a.toolCallId);
      return fresh ? { ...a, askedMs: fresh.askedMs ?? a.askedMs, deadlineMs: fresh.deadlineMs ?? a.deadlineMs } : a;
    });
  }

  $effect(() => {
    const id = current?.toolCallId ?? "";
    if (id === guardedFor) return;
    const replaced = guardedFor !== "";
    guardedFor = id;
    if (!id || !replaced) return;
    guarded = true;
    const timer = setTimeout(() => (guarded = false), 400);
    return () => clearTimeout(timer);
  });

  function drop(toolCallId: string) {
    failed = false;
    const full = asks.length >= 16;
    asks = resolveAsk(asks, toolCallId);
    delete arrived[toolCallId];
    if (full) void sync();
    const { [toolCallId]: _, ...rest } = previews;
    previews = rest;
  }

  async function answer(decision: Decision) {
    const target = current;
    if (!target || busy || guarded) return;
    busy = true;
    failed = false;
    try {
      await invoke("llm_tool_answer", { requestId: target.requestId, toolCallId: target.toolCallId, decision });
      drop(target.toolCallId);
    } catch {
      const waiting = await invoke<unknown[]>("llm_tool_asks_pending").catch(() => null);
      const still = (waiting ?? []).some((w) => (w as { tool_call_id?: unknown } | null)?.tool_call_id === target.toolCallId);
      if (waiting === null || still) failed = true;
      else drop(target.toolCallId);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    const stops: UnlistenFn[] = [];
    let disposed = false;
    const keep = (p: Promise<UnlistenFn>) => p.then((fn) => (disposed ? fn() : stops.push(fn))).catch(() => {});
    keep(listen("llm://tool-ask", (e) => { take(e.payload); void sync(); }));
    keep(listen("omni://tool-resolved", (e) => {
      const id = (e.payload as { tool_call_id?: unknown })?.tool_call_id;
      if (typeof id === "string") drop(id);
    }));
    void sync();
    return () => { disposed = true; stops.forEach((fn) => fn()); };
  });
</script>

<div aria-live="assertive">
{#if current}
  <ApprovalCard
    titleId="tool-ask-title"
    title={$t(`tool_ask.title_${current.kind}`, { agent: agentName }) as string}
    describedBy={preview ? "tool-ask-body tool-ask-preview" : "tool-ask-body"}
    error={failed ? ($t("tool_ask.failed") as string) : null}
  >
    {#snippet aside()}
      {#if asks.length > 1}<span class="ask-count">{$t("tool_ask.count", { index: 1, total: asks.length })}</span>{/if}
    {/snippet}
    <p id="tool-ask-body" class="ask-meta">
      <span class="ask-kind" data-kind={current.kind}>{$t(`tool_ask.kind_${current.kind}`)}</span>
      <bdi class="ask-tool">{current.tool}</bdi>
      {#if timing}<span class="ask-timing">{timing}</span>{/if}
    </p>
    {#if current.impact}
      <p class="ask-impact">
        <span>{$t("tool_ask.impact", { files: current.impact.files, added: current.impact.added, removed: current.impact.removed })}</span>
        {#if current.impact.outside}<span class="ask-outside">{$t("tool_ask.impact_outside")}</span>{/if}
      </p>
    {/if}
    {#if preview}<pre id="tool-ask-preview" class="ask-preview" dir="ltr">{preview}</pre>{/if}
    <div class="ask-actions">
      <button use:agentActions={[]} type="button" class="btn btn-secondary btn-sm" disabled={busy || guarded} onclick={() => answer("decline")}>{$t("tool_ask.deny")}</button>
      <button use:agentActions={[]} type="button" class="btn btn-primary btn-sm" disabled={busy || guarded} onclick={() => answer("accept")}>{$t("tool_ask.once")}</button>
    </div>
    <div class="ask-more">
      <button use:agentActions={[]} type="button" class="ask-link" disabled={busy || guarded} onclick={() => answer("accept_session")}>{$t(current.kind === "command" ? "tool_ask.session_command" : "tool_ask.session", { tool: current.tool })}</button>
      {#if current.canAlways}<button use:agentActions={[]} type="button" class="ask-link" disabled={busy || guarded} onclick={() => answer("accept_always")}>{$t(current.kind === "command" ? "tool_ask.always_command" : "tool_ask.always", { tool: current.tool })}</button>{/if}
      <button use:agentActions={[]} type="button" class="ask-link" disabled={busy || guarded} onclick={() => answer("cancel")}>{$t("tool_ask.stop")}</button>
    </div>
    <p class="ask-note">{$t("tool_ask.note")}</p>
  </ApprovalCard>
{/if}
</div>

<style>
  .ask-impact {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-sm);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
  .ask-outside {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0 var(--space-2);
    border-radius: var(--radius-full);
    box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, currentColor);
    font-size: var(--text-xs);
  }
  .ask-outside::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--agent-blocked);
  }
  .ask-count { flex: none; font-size: var(--text-xs); color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .ask-note { margin: 0; font-size: var(--text-xs); color: var(--text-muted); overflow-wrap: anywhere; }
  .ask-preview {
    margin: 0;
    max-height: 160px;
    overflow: auto;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    font-size: var(--text-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .ask-meta { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); margin: 0; font-size: var(--text-xs); color: var(--text-muted); }
  .ask-kind {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 1px var(--space-2);
    border-radius: var(--radius-full);
    box-shadow: inset 0 0 0 var(--hairline, 1px) var(--content-border, currentColor);
    color: var(--text);
    font-weight: 600;
  }
  .ask-kind::before { content: ""; width: 6px; height: 6px; border-radius: var(--radius-full); background: var(--warning); }
  .ask-kind[data-kind="read"]::before { background: var(--success); }
  .ask-tool { font-family: var(--font-mono, ui-monospace, monospace); overflow-wrap: anywhere; }
  .ask-timing { font-variant-numeric: tabular-nums; }
  .ask-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--space-2); padding-block-start: var(--space-1); }
  .ask-more { display: flex; flex-direction: column; align-items: flex-end; gap: var(--space-1); }
  .ask-link {
    all: unset;
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-muted);
    text-align: end;
    text-decoration: underline;
    text-underline-offset: 2px;
    border-radius: var(--radius-xs);
  }
  .ask-link:hover:not(:disabled) { color: var(--text); }
  .ask-link:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  .ask-link:disabled { cursor: default; opacity: 0.5; }
</style>
