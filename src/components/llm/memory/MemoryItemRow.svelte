<script lang="ts">
  /**
   * One memory: what it says, "why?" (where it came from, readable), edit as
   * a new version, confirm or discard a guess, history, forget. A failed
   * action keeps the draft and says why.
   */
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { getAgent, getConversations, selectConversation } from "$lib/stores/llm-store.svelte";
  import {
    authorBot,
    confirmMemory,
    correctMemory,
    forgetMemory,
    memoryHistory,
    retractMemory,
    type MemoryItem,
  } from "$lib/stores/assist-memory-store.svelte";

  let { item }: { item: MemoryItem } = $props();

  let why = $state(false);
  let editing = $state(false);
  let draft = $state("");
  let confirmingForget = $state(false);
  let busy = $state(false);
  let errorKey = $state<string | null>(null);
  let history = $state<MemoryItem[] | null>(null);

  const date = (ms: number) => new Date(ms).toLocaleDateString();
  const dateTime = (ms: number) => new Date(ms).toLocaleString();

  let conversation = $derived(
    item.source.conversation ? getConversations().find((c) => c.id === item.source.conversation) ?? null : null,
  );
  let botName = $derived.by(() => {
    const id = authorBot(item);
    return id ? getAgent(id)?.name ?? id : null;
  });

  function startEdit() {
    draft = item.content;
    errorKey = null;
    editing = true;
  }

  async function saveEdit() {
    const text = draft.trim();
    if (!text || busy) return;
    busy = true;
    const out = await correctMemory(item, text);
    busy = false;
    if (out.ok) {
      editing = false;
      errorKey = null;
    } else {
      errorKey = out.errorKey; // the draft stays in the box
    }
  }

  async function act(fn: () => Promise<{ ok: boolean; errorKey?: string }>) {
    if (busy) return;
    busy = true;
    const out = await fn();
    busy = false;
    errorKey = out.ok ? null : (out as { errorKey: string }).errorKey;
  }

  async function toggleHistory() {
    if (history) {
      history = null;
      return;
    }
    const out = await memoryHistory(item.id);
    if (out.ok) history = out.value;
    else errorKey = out.errorKey;
  }

  function openConversation() {
    if (!conversation) return;
    selectConversation(conversation.id);
    void goto("/llm");
  }
</script>

<li class="memory-item" class:candidate={item.category === "inference"}>
  {#if editing}
    <form class="edit" onsubmit={(e) => { e.preventDefault(); void saveEdit(); }}>
      <label class="sr" for={`edit-${item.id}`}>{$t("assist.memory.edit_label")}</label>
      <textarea id={`edit-${item.id}`} class="input" rows="2" bind:value={draft}></textarea>
      <p class="dim">{$t("assist.memory.edit_hint")}</p>
      <div class="actions">
        <button type="submit" class="button primary" disabled={busy || !draft.trim()}>{$t("assist.memory.save")}</button>
        <button type="button" class="button" onclick={() => { editing = false; errorKey = null; }}>{$t("assist.memory.cancel")}</button>
      </div>
    </form>
  {:else}
    <p class="content">{item.content}</p>
    {#if item.category === "inference"}
      <p class="dim">{$t("assist.memory.candidate_note", { confidence: Math.round(item.confidence * 100) })}</p>
    {/if}
    {#if item.category === "temporary" && item.valid_until}
      <p class="dim">{$t("assist.memory.until", { when: dateTime(item.valid_until) })}</p>
    {/if}
    <div class="actions">
      <button type="button" class="link" aria-expanded={why} onclick={() => (why = !why)}>{$t("assist.memory.why")}</button>
      {#if item.status === "active"}
        {#if item.category === "inference"}
          <button type="button" class="button small" disabled={busy} onclick={() => act(() => confirmMemory(item))}>{$t("assist.memory.confirm")}</button>
          <button type="button" class="button small" disabled={busy} onclick={() => act(() => retractMemory(item))}>{$t("assist.memory.discard")}</button>
        {/if}
        <button type="button" class="button small" disabled={busy} onclick={startEdit}>{$t("assist.memory.edit")}</button>
      {/if}
      {#if item.version > 1 || item.status !== "active"}
        <button type="button" class="link" aria-expanded={history !== null} onclick={toggleHistory}>{$t("assist.memory.history")}</button>
      {/if}
      {#if confirmingForget}
        <span class="confirm" role="group" aria-label={$t("assist.memory.forget")}>
          <span>{$t("assist.memory.forget_confirm")}</span>
          <button type="button" class="button small danger" disabled={busy} onclick={() => act(() => forgetMemory(item))}>{$t("assist.memory.forget_yes")}</button>
          <button type="button" class="button small" onclick={() => (confirmingForget = false)}>{$t("assist.memory.cancel")}</button>
        </span>
      {:else}
        <button type="button" class="button small" disabled={busy} onclick={() => (confirmingForget = true)}>{$t("assist.memory.forget")}</button>
      {/if}
    </div>
  {/if}

  {#if why}
    <dl class="why">
      <dt>{$t("assist.memory.why_from")}</dt>
      <dd>
        {#if item.source.author === "user"}
          {$t("assist.memory.from_you", { date: date(item.source.at_ms) })}
        {:else if conversation}
          <button type="button" class="link" onclick={openConversation}>
            {$t("assist.memory.from_conversation", { title: conversation.title || $t("assist.memory.untitled"), date: date(item.source.at_ms) })}
          </button>
        {:else if item.source.conversation}
          {$t("assist.memory.from_old_conversation", { date: date(item.source.at_ms) })}
        {:else if item.source.kind === "import"}
          {$t("assist.memory.from_import", { date: date(item.source.at_ms) })}
        {:else}
          {$t("assist.memory.from_unknown", { date: date(item.source.at_ms) })}
        {/if}
      </dd>
      {#if botName}
        <dt>{$t("assist.memory.why_by")}</dt>
        <dd>{botName}</dd>
      {/if}
      {#if item.evidence}
        <dt>{$t("assist.memory.why_evidence")}</dt>
        <dd>{item.evidence}</dd>
      {/if}
      <dt>{$t("assist.memory.why_kind")}</dt>
      <dd>{$t(`assist.memory.kind_help.${item.category}`)}</dd>
    </dl>
  {/if}

  {#if history}
    <ol class="history" aria-label={$t("assist.memory.history")}>
      {#each history as v (v.id)}
        <li class:current={v.status === "active"}>
          <span class="dim">v{v.version} · {date(v.created_ms)} · {$t(`assist.memory.status.${v.status}`)}</span>
          <span>{v.content}</span>
        </li>
      {/each}
    </ol>
  {/if}

  {#if errorKey}
    <p class="notice error" role="alert">{$t(errorKey)}</p>
  {/if}
</li>

<style>
  .memory-item { list-style: none; padding: 12px 0; border-bottom: 1px solid var(--separator); display: grid; gap: 6px; }
  .memory-item.candidate .content { font-style: italic; }
  .content { margin: 0; color: var(--text); font-size: 14px; line-height: 1.5; overflow-wrap: anywhere; }
  .dim { margin: 0; color: var(--text-muted); font-size: 12px; }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .link { background: none; border: 0; padding: 4px 0; color: var(--accent-text); font-size: 13px; cursor: pointer; border-radius: var(--radius-sm); text-align: left; }
  .link:hover { text-decoration: underline; }
  .link:focus-visible, .button:focus-visible, textarea:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  .danger { color: var(--danger, #c0392b); }
  .confirm { display: inline-flex; flex-wrap: wrap; gap: 6px; align-items: center; font-size: 13px; }
  .why { display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; margin: 4px 0 0; padding: 8px 12px; border-left: 2px solid var(--separator); font-size: 13px; }
  .why dt { color: var(--text-muted); }
  .why dd { margin: 0; overflow-wrap: anywhere; }
  .history { margin: 4px 0 0; padding-left: 20px; display: grid; gap: 4px; font-size: 13px; }
  .history li { display: grid; gap: 2px; }
  .history li.current span:last-child { font-weight: 600; }
  .edit { display: grid; gap: 6px; }
  .edit textarea { width: 100%; resize: vertical; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
  @media (max-width: 560px) { .why { grid-template-columns: 1fr; } }
</style>
