<script lang="ts">
  /**
   * One message. Assistant text is markdown (sanitised; see `$lib/llm/markdown`),
   * user text stays plain. Tool calls render as collapsed blocks the user can
   * open; the raw JSON input is shown as text, never as HTML.
   */
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { renderSafeMarkdownSync } from "$lib/llm/markdown";
  import { agentTint, type AgentDef, type ChatMessage, type ToolCallView } from "$lib/llm/types";
  import { tintToCss } from "$lib/stores/profile-store.svelte";
  import { getAgent } from "$lib/stores/llm-store.svelte";

  let {
    message,
    agent = null,
    streaming = false,
  }: {
    message: ChatMessage;
    agent?: AgentDef | null;
    streaming?: boolean;
  } = $props();

  const cache = new Map<string, string>();
  let version = $state(0);

  onDestroy(() => cache.clear());

  let isAssistant = $derived(message.role === "assistant");
  let authorKind = $derived(message.author?.kind ?? (isAssistant ? "bot" : "user"));
  let isSystem = $derived(authorKind === "system");
  // Rooms have several authors: the bubble names the one that wrote it.
  let author = $derived(message.author?.botId ? getAgent(message.author.botId) : agent);
  let authorName = $derived(author?.name ?? message.author?.botId ?? agent?.name ?? "");
  let delegated = $derived(authorKind === "bot" && !!message.taskId);
  let html = $derived.by(() => {
    version; // re-run once `marked` has loaded
    return isAssistant ? renderSafeMarkdownSync(message.text, cache, () => (version += 1)) : "";
  });
  let toolCalls = $derived<ToolCallView[]>(message.toolCalls ?? []);
  let name = $derived(
    isSystem ? $t("assist.groups.app_note") : authorKind === "user" ? $t("llm.conv.you") : authorName,
  );
</script>

<article class="bubble" class:assistant={isAssistant} class:system={isSystem}>
  <header class="bubble-head">
    {#if isAssistant && !isSystem}
      <span class="dot" style:background={tintToCss(agentTint(author))} aria-hidden="true"></span>
    {/if}
    <span class="who">{name}</span>
    {#if message.modelLabel}
      <span class="model">{message.modelLabel}</span>
    {/if}
  </header>

  {#if message.thinking}
    <details class="tool">
      <summary>{$t("llm.conv.thinking")}</summary>
      <pre class="tool-body">{message.thinking}</pre>
    </details>
  {/if}

  {#each toolCalls as call (call.id)}
    <details class="tool" open={!call.done}>
      <summary>
        <span class="tool-name">{$t("llm.conv.tool_call")}: {call.name}</span>
        {#if !call.done}<span class="tool-state">{$t("llm.conv.tool_running")}</span>{/if}
      </summary>
      <pre class="tool-body">{call.input}</pre>
    </details>
  {/each}

  {#if isSystem}
    <p class="body system-note">
      {#if message.author?.botId && message.status === "skipped"}{$t("assist.groups.skipped", { bot: authorName })} {/if}{message.text}
    </p>
  {:else if delegated}
    <details class="tool delegated">
      <summary>{$t("assist.groups.delegated_answer", { bot: authorName })}</summary>
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in $lib/llm/markdown -->
      <div class="body markdown">{@html html}</div>
    </details>
  {:else if isAssistant}
    {#if message.text && message.status !== "failed"}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in $lib/llm/markdown -->
      <div class="body markdown">{@html html}</div>
    {/if}
    {#if streaming}
      <span class="caret" aria-hidden="true"></span>
    {/if}
  {:else}
    <p class="body">{message.text}</p>
  {/if}

  {#if message.error}
    <div class="bubble-error" role="alert">
      <p>{$t("llm.conv.error")}: {message.error.code}</p>
      {#if message.error.message && message.error.message !== message.error.code}
        <p class="bubble-error-detail">{message.error.message}</p>
      {/if}
    </div>
  {:else if message.status === "interrupted"}
    <p class="bubble-note">{$t("assist.conversation.status_interrupted")}</p>
  {:else if message.finish === "cancelled"}
    <p class="bubble-note">{$t("llm.conv.cancelled")}</p>
  {/if}
</article>

<style>
  .bubble {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) 0;
    max-width: 78ch;
  }

  .bubble-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .dot {
    width: 12px;
    height: 12px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }

  .who {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .model {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }

  .body {
    margin: 0;
    font-size: var(--text-base);
    line-height: var(--leading-base);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .markdown {
    white-space: normal;
  }

  .markdown :global(p) {
    margin: 0 0 var(--space-2);
  }

  .markdown :global(pre) {
    margin: var(--space-2) 0;
    padding: var(--space-3);
    background: var(--fill-1);
    border-radius: var(--radius-md);
    overflow-x: auto;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .markdown :global(code) {
    font-family: var(--font-mono);
    font-size: 0.92em;
  }

  .markdown :global(ul),
  .markdown :global(ol) {
    margin: 0 0 var(--space-2);
    padding-left: var(--space-5);
  }

  .markdown :global(a) {
    color: var(--accent-hi);
  }

  .tool {
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-3);
    background: var(--fill-1);
  }

  .tool summary {
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-muted);
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .tool-name {
    font-weight: 600;
  }

  .tool-state {
    font-size: var(--text-caption);
    color: var(--accent-hi);
  }

  .tool-body {
    margin: var(--space-2) 0 0;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text-muted);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .caret {
    display: inline-block;
    width: 8px;
    height: 16px;
    background: var(--accent-hi);
    border-radius: 2px;
  }

  .bubble-error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--danger);
    min-width: 0;
  }
  .bubble-error p { margin: 0; }
  .bubble-error-detail {
    color: var(--text-muted, var(--text-dim));
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 12em;
    overflow-y: auto;
  }
  .bubble.system { padding: var(--space-1) 0; }
  .system-note { font-size: var(--text-sm); color: var(--text-dim); }
  .delegated .body { margin-top: var(--space-2); }

  .bubble-note {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
