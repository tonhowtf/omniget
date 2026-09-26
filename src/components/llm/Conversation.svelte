<script lang="ts">
  // Presentation follows 21st Agent Chat by Serafim (12402): a bounded reading column, contextual empty state and an in-flow composer. Original Svelte implementation; backend contracts stay local.
  import WorkspaceChip from "./WorkspaceChip.svelte";
  import MissionChatBar from "./missions/MissionChatBar.svelte";
  /**
   * Centre column: header (agent, effective model, switch-model, new chat),
   * the messages and the composer. The streaming answer is a bubble built from
   * the live turn, so it follows the same layout as a committed message.
   */
  import { tick } from "svelte";
  import { t } from "$lib/i18n";
  import type { AgentDef, ChatMessage, ModelRef } from "$lib/llm/types";
  import { effectiveModelLabel } from "$lib/llm/types";
  import {
    cancelRoom,
    cancelTurn,
    getActiveRoom,
    getAgents,
    getLastAsk,
    getQueue,
    getRoomLive,
    getTurnStatus,
    refreshRunState,
    removeQueued,
    retryQueued,
    getActiveConversation,
    getActiveTurn,
    getMessages,
    getErrorKey,
    getToolAsk,
    answerTool,
    sendMessage,
    switchModel,
  } from "$lib/stores/llm-store.svelte";
  import Composer from "./Composer.svelte";
  import MessageBubble from "./MessageBubble.svelte";
  import ModelPicker from "./ModelPicker.svelte";
  import RoomActivity from "./groups/RoomActivity.svelte";
  import RoomEditor from "./groups/RoomEditor.svelte";

  let { agent = null }: { agent?: AgentDef | null } = $props();

  let scroller = $state<HTMLDivElement | null>(null);
  let pickerOpen = $state(false);
  let draft = $state("");
  let sendFailed = $state(false);
  let actionFailed = $state(false);
  let answering = $state(false);
  let runtimeError = $derived(getErrorKey());
  let switchFailed = $state(false);
  let switching = $state(false);
  let pickerTrigger = $state<HTMLButtonElement | null>(null);
  let submittedText = "";
  let sendGeneration = 0;
  let cancelledGeneration = 0;

  let conversation = $derived(getActiveConversation());
  let messages = $derived(getMessages());
  let turn = $derived(getActiveTurn());
  let ask = $derived(getToolAsk());
  let modelText = $derived(effectiveModelLabel(conversation, agent));
  let room = $derived(getActiveRoom());
  let roomLive = $derived(getRoomLive(room?.id));
  let status = $derived(getTurnStatus(conversation?.id));
  let queued = $derived(getQueue(conversation?.id));
  let lastAsk = $derived(getLastAsk(conversation?.id));
  let editingRoom = $state(false);
  let allAgents = $derived(getAgents());
  let memberOptions = $derived(
    room
      ? room.members.map((m) => ({ id: m.bot, name: allAgents.find((a) => a.id === m.bot)?.name ?? m.bot, role: m.role }))
      : [],
  );
  let running = $derived(room ? roomLive.length > 0 : turn !== null);

  let streamingMessage = $derived<ChatMessage | null>(
    turn && !room && turn.conversationId === conversation?.id
      ? {
          id: `turn-${turn.requestId}`,
          role: "assistant",
          text: turn.text,
          toolCalls: turn.toolCalls.length > 0 ? turn.toolCalls : undefined,
          thinking: turn.thinking || undefined,
          modelLabel: modelText,
          error: turn.error,
        }
      : null,
  );

  async function stickToBottom() {
    await tick();
    const el = scroller;
    if (el) el.scrollTop = el.scrollHeight;
  }

  // A permission request must never be born below the fold, and a streaming
  // turn keeps the newest line in view while the reader is already at the end.
  $effect(() => {
    if (ask) void stickToBottom();
  });
  $effect(() => {
    void turn?.text;
    void turn?.toolCalls.length;
    const el = scroller;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 160) void stickToBottom();
  });

  async function onSend(text: string): Promise<boolean> {
    const generation = ++sendGeneration;
    sendFailed = false;
    submittedText = text;
    const sent = await sendMessage(text);
    sendFailed = !sent && generation !== cancelledGeneration;
    if (sent) void stickToBottom();
    return sent;
  }

  async function onStop() {
    if (room) {
      actionFailed = !(await cancelRoom(room.id));
      return;
    }
    cancelledGeneration = sendGeneration;
    sendFailed = false;
    const interruptedText = submittedText;
    actionFailed = false;
    try {
      await cancelTurn();
      if (!draft.trim()) draft = interruptedText;
    } catch { actionFailed = true; }
  }

  async function onAnswer(allow: boolean, always = false) {
    if (!ask || answering) return;
    answering = true;
    actionFailed = false;
    try { await answerTool(ask.tool_call_id, allow, always); }
    catch { actionFailed = true; }
    finally { answering = false; }
  }

  async function onSwitch(ref: ModelRef) {
    if (switching) return;
    switching = true;
    switchFailed = false;
    try {
      if (await switchModel(ref)) {
        pickerOpen = false;
        await tick();
        pickerTrigger?.focus();
      } else switchFailed = true;
    } finally { switching = false; }
  }

  function onRemoveQueued(id: string) {
    const text = removeQueued(id);
    // A cancelled queued message goes back to the draft when that is empty.
    if (text && !draft.trim()) draft = text;
  }

  // Follow the stream without a timer: the text length is the only trigger.
  $effect(() => {
    conversation?.id;
    void stickToBottom();
  });

  // After a restart the durable run record says whether work was interrupted.
  $effect(() => {
    const id = conversation?.id;
    if (id && conversation?.kind !== "group") void refreshRunState(id);
  });
</script>

<section class="conv">
  <header class="conv-head">
    <div class="conv-title">
      {#if room}
        <h2>{room.title}</h2>
        <span class="conv-members" aria-label={$t("assist.groups.members") as string}>
          {#each memberOptions as m (m.id)}<span class="member-chip" class:coordinator={room.coordinator === m.id}>@{m.name}</span>{/each}
        </span>
      {:else}
        <h2>{agent?.name ?? $t("llm.conv.no_agent")}</h2>
        {#if modelText}<span class="conv-model">{modelText}</span>{/if}
      {/if}
      <WorkspaceChip conversationId={conversation?.id ?? null} refreshKey={turn ? 1 : 0} />
      <MissionChatBar mode="chip" conversationId={conversation?.id ?? null} />
    </div>
    <div class="conv-actions">
      {#if room}
        <button type="button" class="button" onclick={() => (editingRoom = true)}>{$t("assist.groups.edit")}</button>
      {/if}
      <button
        type="button"
        class="button"
        bind:this={pickerTrigger}
        onclick={() => (pickerOpen = !pickerOpen)}
        aria-expanded={pickerOpen}
        hidden={!!room}
        disabled={!conversation}
      >
        {$t("llm.conv.switch_model")}
      </button>

    </div>
  </header>

  {#if pickerOpen}
    <div class="conv-picker">
      <ModelPicker value={conversation?.model ?? (agent?.model.policy === "fixed" ? agent.model.model : null)} onchange={onSwitch} explicit busy={switching} />
      {#if switchFailed}<p role="alert">{$t("llm.surface.unavailable")}</p>{/if}
    </div>
  {/if}

  <div class="conv-scroll" bind:this={scroller}>
    <div class="reading-column">
    {#if messages.length === 0 && !streamingMessage && roomLive.length === 0}
      <div class="empty-state">
        <p class="empty-state-title">{room ? $t("assist.groups.empty_title") : $t("llm.conv.empty_title")}</p>
        <p class="empty-state-body">{room ? $t("assist.groups.empty_body") : $t("llm.conv.empty_body")}</p>
        {#if room}
          <!-- a room teaches @mentions instead of generic suggestions -->
        {:else if !agent}
          <a class="button primary" href="/llm/accounts">{$t("llm.surface.connect")}</a>
        {:else}
          <div class="task-suggestions">
            {#each [$t("llm.surface.suggestion_1"), $t("llm.surface.suggestion_2"), $t("llm.surface.suggestion_3")] as suggestion}
              <button type="button" onclick={() => draft = suggestion}>{suggestion}</button>
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      {#each messages as message (message.id)}
        <MessageBubble {message} {agent} />
      {/each}
      {#if streamingMessage}
        <MessageBubble message={streamingMessage} {agent} streaming />
      {/if}
      {#each roomLive as live (live.runId)}
        <MessageBubble
          message={{ id: `live-${live.runId}`, role: "assistant", text: live.text, author: { kind: "bot", botId: live.botId } }}
          {agent}
          streaming
        />
      {/each}
    {/if}
    {#if room}<RoomActivity roomId={room.id} />{/if}
    {#if lastAsk && !ask}
      <p class="ask-ended" role="status">
        {lastAsk.state === "expired"
          ? $t("assist.conversation.ask_expired", { tool: lastAsk.tool })
          : $t("assist.conversation.ask_cancelled", { tool: lastAsk.tool })}
      </p>
    {/if}

    {#if ask}
      <div class="tool-ask" role="region" aria-labelledby="llm-tool-ask-title">
        <div class="tool-ask-text">
          <span id="llm-tool-ask-title" class="tool-ask-title" role="status">{$t("llm.conv.tool_ask")}</span>
          <span class="tool-ask-tool">{ask.tool}</span>
          {#if ask.preview}<pre class="tool-ask-preview">{ask.preview}</pre>{/if}
        </div>
        <div class="tool-ask-actions">
          <button
            type="button"
            class="button primary"
            disabled={answering}
            onclick={() => void onAnswer(true)}
          >
            {$t("llm.conv.allow")}
          </button>
          {#if ask.tool !== "external_directory"}
            <button
              type="button"
              class="button"
              title={$t("llm.conv.always_hint") as string}
              disabled={answering}
              onclick={() => void onAnswer(true, true)}
            >
              {$t("llm.conv.always")}
            </button>
          {/if}
          <button
            type="button"
            class="button"
            disabled={answering}
            onclick={() => void onAnswer(false)}
          >
            {$t("llm.conv.deny")}
          </button>
        </div>
      </div>
    {/if}
    </div>
  </div>
  {#if actionFailed || (runtimeError && turn)}<p class="send-error" role="alert">{$t("llm.surface.action_error")}</p>{/if}
  {#if sendFailed}<p class="send-error" role="alert">{$t("llm.surface.error")}</p>{/if}

  <MissionChatBar mode="action" conversationId={conversation?.id ?? null} botId={room ? null : (agent?.id ?? null)} roomId={room?.id ?? null} {draft} />
  <Composer
    bind:value={draft}
    disabled={!agent && !room}
    {running}
    {status}
    queue={queued}
    members={memberOptions}
    onsend={onSend}
    onstop={() => void onStop()}
    onremovequeued={onRemoveQueued}
    onretryqueued={retryQueued}
  />
</section>

{#if editingRoom && room}
  <RoomEditor {room} agents={allAgents} onclose={() => (editingRoom = false)} />
{/if}

<style>
  .reading-column { width:100%; max-width:760px; margin:0 auto; }
  .task-suggestions { display:flex; flex-direction:column; align-items:center; gap:8px; margin-top:24px; }
  .task-suggestions button { border:1px solid var(--separator); border-radius:var(--radius-lg); padding:12px 16px; color:var(--text); background:var(--fill-1); text-align:left; font:inherit; cursor:pointer; max-width:100%; transition:background 150ms; }
  .task-suggestions button:hover { background:var(--accent-soft); }
  .task-suggestions button:focus-visible { outline:var(--focus-ring); }
  .conv-members { display:flex; flex-wrap:wrap; gap:4px; min-width:0; }
  .member-chip { font-size:var(--text-caption); color:var(--text-dim); background:var(--fill-1); border-radius:var(--radius-full); padding:1px 8px; white-space:nowrap; }
  .member-chip.coordinator { color:var(--text); font-weight:600; }
  .ask-ended { margin:var(--space-2) 0; font-size:var(--text-sm); color:var(--text-dim); overflow-wrap:anywhere; }
  .send-error { color:var(--text); background:var(--accent-soft); padding:12px 16px; margin:0 20px; border-radius:var(--radius-md); font-size:13px; }
  @media(max-width:680px) { .conv-head { flex-wrap:wrap; } .tool-ask { flex-direction:column; align-items:stretch; } .tool-ask-actions { flex-wrap:wrap; } }

  .tool-ask-preview {
    margin: var(--space-2) 0 0;
    padding: var(--space-2);
    max-height: 220px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-quaternary, rgba(127, 127, 127, 0.12));
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs, 11px);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .conv {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .conv-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-5);
    border-bottom: var(--hairline) solid var(--separator);
  }

  .conv-title {
    display: flex;
    align-items: baseline;
    /* The workspace chip carries Undo: on a narrow window it drops to a line
       of its own instead of sliding under the actions on the right. */
    flex-wrap: wrap;
    row-gap: var(--space-1);
    gap: var(--space-3);
    min-width: 0;
    flex: 1 1 0;
  }

  .conv-title h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    /* The name is the last thing to give way: the model label shrinks first. */
    flex: 0 0 auto;
    max-width: 45%;
  }

  .conv-model {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--text-dim);
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .conv-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .conv-picker {
    padding: var(--space-3) var(--space-5);
    border-bottom: var(--hairline) solid var(--separator);
    background: var(--fill-1);
  }

  .conv-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3) var(--space-5);
    overscroll-behavior: contain;
  }

  .empty-state {
    padding: var(--space-7) 0;
    text-align: center;
  }

  .tool-ask {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin: var(--space-3) 0;
    padding: var(--space-3);
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    background: var(--accent-soft);
  }

  .tool-ask-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .tool-ask-title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
  }

  .tool-ask-tool {
    font-family: var(--font-mono);
    font-size: var(--text-caption);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .tool-ask-actions {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .empty-state-body {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
