<script lang="ts">
  // Presentation follows 21st AI Input by Hayden Bleasel (1887): one autosizing input and a compact action row. Original Svelte implementation with no simulated attachments or capabilities.
  /**
   * Composer: one auto-growing textarea, Enter sends, Shift+Enter breaks the
   * line. While a turn runs, Stop is offered and Send puts the message in a
   * visible queue (cancelable) instead of refusing it in silence. The status
   * line says what the work is really doing, from real events only. In a room,
   * `@` opens an accessible list of members (combobox pattern).
   */
  import { t } from "$lib/i18n";
  import { handleOf, mentionAt, type QueuedMessage, type TurnStatus } from "$lib/llm/types";

  type MemberOption = { id: string; name: string; role?: string };

  let {
    value = $bindable(""),
    disabled = false,
    running = false,
    status = "idle",
    queue = [],
    members = [],
    onsend,
    onstop,
    onremovequeued,
    onretryqueued,
  }: {
    value?: string;
    disabled?: boolean;
    running?: boolean;
    status?: TurnStatus;
    queue?: QueuedMessage[];
    members?: MemberOption[];
    onsend: (text: string) => boolean | Promise<boolean>;
    onstop: () => void;
    onremovequeued?: (id: string) => void;
    onretryqueued?: (id: string) => void;
  } = $props();

  const MAX_HEIGHT = 200;
  const LIST_ID = "composer-mentions";

  let sending = $state(false);
  let textarea = $state<HTMLTextAreaElement | null>(null);
  let mention = $state<{ start: number; query: string } | null>(null);
  let activeIndex = $state(0);

  $effect(() => { value; queueMicrotask(autosize); });

  let options = $derived.by<MemberOption[]>(() => {
    if (!mention || members.length === 0) return [];
    const q = mention.query;
    return members
      .filter((m) => !q || handleOf(m.name).startsWith(q) || m.id.toLowerCase().startsWith(q) || m.name.toLowerCase().includes(q))
      .slice(0, 8);
  });
  let listOpen = $derived(options.length > 0);

  let statusKey = $derived(
    status === "idle" ? null : `assist.conversation.status_${status}`,
  );

  function autosize() {
    const el = textarea;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
  }

  function updateMention() {
    const el = textarea;
    if (!el || members.length === 0) { mention = null; return; }
    mention = mentionAt(value, el.selectionStart ?? value.length);
    activeIndex = 0;
  }

  function pick(option: MemberOption) {
    const el = textarea;
    if (!mention || !el) return;
    const handle = handleOf(option.name) || option.id;
    const caret = el.selectionStart ?? value.length;
    value = `${value.slice(0, mention.start)}@${handle} ${value.slice(caret)}`;
    const at = mention.start + handle.length + 2;
    mention = null;
    queueMicrotask(() => { el.focus(); el.setSelectionRange(at, at); });
  }

  async function send() {
    const text = value.trim();
    if (!text || disabled || sending) return;
    sending = true;
    try {
      const accepted = await onsend(text);
      // Keep text typed during startup, and retain the entire draft on failure.
      if (accepted && value.trim() === text) value = "";
    } finally { sending = false; queueMicrotask(autosize); }
  }

  function onKeydown(event: KeyboardEvent) {
    if (listOpen) {
      if (event.key === "ArrowDown") { event.preventDefault(); activeIndex = (activeIndex + 1) % options.length; return; }
      if (event.key === "ArrowUp") { event.preventDefault(); activeIndex = (activeIndex - 1 + options.length) % options.length; return; }
      if ((event.key === "Enter" || event.key === "Tab") && !event.isComposing) { event.preventDefault(); pick(options[activeIndex]); return; }
      if (event.key === "Escape") { event.preventDefault(); mention = null; return; }
    }
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      send();
    }
  }
</script>

<div class="composer">
  {#if queue.length > 0}
    <ul class="composer-queue" aria-label={$t("assist.conversation.queue_title")}>
      {#each queue as item (item.id)}
        <li class:failed={item.state === "failed"}>
          <span class="queue-state">{item.state === "failed" ? $t("assist.conversation.queue_failed") : $t("assist.conversation.queue_waiting")}</span>
          <span class="queue-text">{item.text}</span>
          {#if item.state === "failed" && onretryqueued}
            <button type="button" class="queue-btn" onclick={() => onretryqueued?.(item.id)}>{$t("assist.conversation.queue_retry")}</button>
          {/if}
          <button type="button" class="queue-btn" onclick={() => onremovequeued?.(item.id)} aria-label={$t("assist.conversation.queue_remove_label", { text: item.text.slice(0, 40) }) as string}>{$t("assist.conversation.queue_remove")}</button>
        </li>
      {/each}
    </ul>
  {/if}

  <div class="composer-field">
    <textarea
      bind:this={textarea}
      bind:value
      class="input composer-input"
      rows="1"
      {disabled}
      placeholder={members.length > 0 ? $t("assist.groups.composer_placeholder") : $t("llm.composer.placeholder")}
      aria-label={$t("llm.composer.placeholder")}
      role={members.length > 0 ? "combobox" : undefined}
      aria-autocomplete={members.length > 0 ? "list" : undefined}
      aria-expanded={members.length > 0 ? listOpen : undefined}
      aria-controls={members.length > 0 ? LIST_ID : undefined}
      aria-activedescendant={listOpen ? `${LIST_ID}-${activeIndex}` : undefined}
      oninput={() => { autosize(); updateMention(); }}
      onclick={updateMention}
      onkeyup={(e) => { if (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "Home" || e.key === "End") updateMention(); }}
      onblur={() => setTimeout(() => (mention = null), 120)}
      onkeydown={onKeydown}
    ></textarea>
    {#if members.length > 0}
      <ul id={LIST_ID} class="mention-list" role="listbox" aria-label={$t("assist.groups.mention_list")} hidden={!listOpen}>
        {#each options as option, i (option.id)}
          <li
            id={`${LIST_ID}-${i}`}
            role="option"
            aria-selected={i === activeIndex}
            class:active={i === activeIndex}
            onmousedown={(e) => { e.preventDefault(); pick(option); }}
          >
            <span class="mention-name">@{handleOf(option.name) || option.id}</span>
            <span class="mention-role">{option.role || option.name}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="composer-actions">
    {#if statusKey}
      <span class="composer-status status-{status}" role="status" aria-live="polite">{$t(statusKey)}</span>
    {:else}
      <span class="composer-hint">{$t("llm.composer.hint")}</span>
    {/if}
    <div class="composer-buttons">
      {#if running}
        <button type="button" class="button composer-send" onclick={onstop}>
          {$t("llm.conv.stop")}
        </button>
      {/if}
      <button
        type="button"
        class="button composer-send"
        class:primary={!running}
        disabled={disabled || sending || value.trim().length === 0}
        onclick={send}
        title={running ? ($t("assist.conversation.queue_hint") as string) : undefined}
      >
        {running ? $t("assist.conversation.queue_send") : $t("llm.composer.send")}
      </button>
    </div>
  </div>
</div>

<style>
  .composer {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-4);
    width: calc(100% - 40px);
    max-width: 760px;
    box-sizing: border-box;
    align-self: center;
    margin: 12px 20px 20px;
    border: 1px solid var(--separator);
    border-radius: var(--radius-xl);
    background: var(--fill-1);
    box-shadow: 0 2px 8px color-mix(in srgb, var(--text) 4%, transparent);
  }

  .composer-field { position: relative; }

  .composer-input {
    width: 100%;
    resize: none;
    border: none;
    box-shadow: none;
    background: transparent;
    font-size: 15px;
    min-height: 40px;
    max-height: 200px;
    line-height: var(--leading-base);
    padding: var(--space-2) var(--space-3);
  }

  .mention-list {
    position: absolute;
    left: 0;
    bottom: calc(100% + 4px);
    z-index: 20;
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    min-width: 220px;
    max-width: 100%;
    border: 1px solid var(--separator);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: var(--shadow-lg);
  }
  .mention-list li {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
    padding: 6px var(--space-2);
    border-radius: var(--radius-sm);
    cursor: pointer;
    min-width: 0;
  }
  .mention-list li.active { background: var(--accent-soft); }
  .mention-name { font-weight: 600; color: var(--text); white-space: nowrap; }
  .mention-role { font-size: var(--text-caption); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .composer-queue {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .composer-queue li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--fill-2, var(--accent-soft));
    font-size: var(--text-sm);
    min-width: 0;
  }
  .composer-queue li.failed { outline: 1px solid var(--danger); }
  .queue-state { font-size: var(--text-caption); color: var(--text-dim); white-space: nowrap; }
  .queue-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .queue-btn {
    border: 0;
    background: none;
    color: var(--accent-text, var(--accent-hi));
    font: inherit;
    font-size: var(--text-caption);
    cursor: pointer;
    padding: 2px 4px;
    border-radius: var(--radius-sm);
  }
  .queue-btn:focus-visible { outline: var(--focus-ring); }

  .composer-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .composer-buttons { display: flex; gap: var(--space-2); flex-shrink: 0; }

  @media (max-width: 480px) {
    .composer-actions { flex-wrap: wrap; }
    .composer-hint, .composer-status { flex: 1 1 140px; }
    .composer { width: calc(100% - 16px); margin: 8px; padding: var(--space-2) var(--space-3); }
  }

  .composer-hint {
    font-size: var(--text-caption);
    color: var(--text-dim);
  }

  .composer-status {
    font-size: var(--text-caption);
    color: var(--text-muted, var(--text-dim));
    overflow-wrap: anywhere;
  }
  .composer-status.status-failed, .composer-status.status-interrupted { color: var(--danger); }
  .composer-status.status-waiting_you { color: var(--accent-text, var(--accent-hi)); font-weight: 600; }

  .composer-send {
    min-width: 96px;
  }
</style>
