<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { acceptAsk, resolveAsk, type PendingAsk } from "$lib/omni/tool-ask-queue";

  let asks = $state<PendingAsk[]>([]);
  let previews = $state<Record<string, string>>({});
  let busy = $state(false);
  let current = $derived(asks[0] ?? null);
  let preview = $derived(current ? (previews[current.toolCallId] ?? "") : "");

  function take(raw: unknown) {
    asks = acceptAsk(asks, raw);
    const o = raw as { tool_call_id?: unknown; preview?: unknown } | null;
    if (o && typeof o.tool_call_id === "string" && typeof o.preview === "string") {
      previews = { ...previews, [o.tool_call_id]: o.preview.slice(0, 600) };
    }
  }

  function drop(toolCallId: string) {
    asks = resolveAsk(asks, toolCallId);
    const { [toolCallId]: _, ...rest } = previews;
    previews = rest;
  }

  async function answer(allow: boolean, always = false) {
    const target = current;
    if (!target || busy) return;
    busy = true;
    try {
      await invoke("llm_tool_answer", { requestId: target.requestId, toolCallId: target.toolCallId, allow, always });
    } catch {
    } finally {
      drop(target.toolCallId);
      busy = false;
    }
  }

  onMount(() => {
    const stops: UnlistenFn[] = [];
    let disposed = false;
    const keep = (p: Promise<UnlistenFn>) => p.then((fn) => (disposed ? fn() : stops.push(fn))).catch(() => {});
    keep(listen("llm://tool-ask", (e) => take(e.payload)));
    keep(listen("omni://tool-resolved", (e) => {
      const id = (e.payload as { tool_call_id?: unknown })?.tool_call_id;
      if (typeof id === "string") drop(id);
    }));
    invoke<unknown[]>("llm_tool_asks_pending")
      .then((waiting) => { for (const ask of waiting ?? []) take(ask); })
      .catch(() => {});
    return () => { disposed = true; stops.forEach((fn) => fn()); };
  });
</script>

{#if current}
  <div class="ask" role="alertdialog" aria-labelledby="tool-ask-title" aria-describedby="tool-ask-body">
    <div class="ask-head">
      <h2 id="tool-ask-title">{$t("pet.ask.title")}</h2>
      {#if asks.length > 1}<span class="ask-count">1/{asks.length}</span>{/if}
    </div>
    <p id="tool-ask-body" class="ask-tool"><code>{current.tool}</code>{#if current.agent}<span> · {current.agent}</span>{/if}</p>
    {#if preview}<pre class="ask-preview">{preview}</pre>{/if}
    <div class="ask-actions">
      <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => answer(false)}>{$t("pet.ask.deny")}</button>
      <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => answer(true, true)}>{$t("pet.ask.always")}</button>
      <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => answer(true)}>{$t("pet.ask.allow")}</button>
    </div>
  </div>
{/if}

<style>
  .ask {
    position: fixed;
    inset-inline-end: var(--space-5);
    inset-block-end: var(--space-5);
    z-index: 900;
    width: min(380px, calc(100vw - 32px));
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    border-radius: var(--radius-lg, 12px);
    background: var(--surface-hi, var(--surface));
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.28), inset 0 0 0 var(--hairline, 1px) var(--content-border, transparent);
    color: var(--text);
  }
  .ask-head { display: flex; align-items: baseline; gap: var(--space-2); }
  h2 { flex: 1; margin: 0; font-size: var(--text-md); font-weight: 600; }
  .ask-count { font-size: var(--text-sm); color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .ask-tool { margin: 0; font-size: var(--text-sm); color: var(--text-muted); overflow-wrap: anywhere; }
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
  .ask-actions { display: flex; justify-content: flex-end; gap: var(--space-2); }
</style>
