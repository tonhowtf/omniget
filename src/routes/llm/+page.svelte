<script lang="ts">
  /**
   * Chat: rail, conversation, inspector. The roster loads once when the page
   * mounts; nothing here polls, and the only timer that can exist belongs to a
   * running turn.
   */
  import { setLlmFocus } from "$lib/llm/ui-focus.svelte";
  import { onMount, untrack, tick } from "svelte";
  import { page } from "$app/stores";
  import { t } from "$lib/i18n";
  import {
    getActiveTurn,
    getRosterLoadError,
    getConversationListLoadError,
    getAgent,
    getAgents,
    getActiveAgentId,
    getActiveConversation,
    getRooms,
    loadConversations,
    loadRooms,
    loadRoster,
    getConversations,
    selectConversation,
    newConversation,
    selectAgent,
  } from "$lib/stores/llm-store.svelte";
  import { loadProfile } from "$lib/stores/profile-store.svelte";
  import Rail from "$components/llm/Rail.svelte";
  import Conversation from "$components/llm/Conversation.svelte";
  import Inspector from "$components/llm/Inspector.svelte";
  import RoomEditor from "$components/llm/groups/RoomEditor.svelte";

  let detailsOpen = $state(false);
  let creatingRoom = $state(false);
  let rosterReady = $state(false);
  let rosterError = $state(false);
  let reloading = $state(false);
  let loadError = $derived(getRosterLoadError() ?? getConversationListLoadError());
  let detailsTrigger = $state<HTMLButtonElement>();
  let detailsClose = $state<HTMLButtonElement>();
  let detailsPanel = $state<HTMLElement>();

  async function toggleDetails() {
    detailsOpen = !detailsOpen;
    await tick();
    if (detailsOpen) detailsClose?.focus();
    else detailsTrigger?.focus();
  }

  function detailsKeydown(event: KeyboardEvent) {
    if (!detailsOpen || event.defaultPrevented) return;
    if (event.key === "Escape") {
      event.preventDefault();
      void toggleDetails();
    } else if (detailsPanel && event.key === "Tab" && matchMedia("(max-width: 1100px)").matches) {
      const controls = Array.from(detailsPanel.querySelectorAll<HTMLElement>('button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]')).filter(el => el.getClientRects().length > 0);
      const first = controls[0], last = controls.at(-1);
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
  }

  async function retryLoading() {
    if (reloading) return;
    reloading = true;
    rosterError = false;
    try { await Promise.all([loadRoster(true), loadConversations(), loadRooms()]); rosterReady = true; }
    catch { rosterError = true; }
    finally { reloading = false; }
  }
  let agents = $derived(getAgents());
  let activeAgent = $derived(getAgent(getActiveAgentId()));
  let active = $derived(getActiveConversation());
  // The conversation on screen: a "needs you" about its own turn is not sent
  // while the window has the focus (see $lib/llm/ui-focus).
  $effect(() => {
    setLlmFocus({ conversationId: active?.id ?? null });
    return () => setLlmFocus({ conversationId: null });
  });
  let rooms = $derived(getRooms());
  let turn = $derived(getActiveTurn());

  onMount(() => {
    // Saved chats (old ones reopen as direct chats, same ids) and rooms load
    // with the roster; nothing here polls.
    void Promise.all([loadRoster(), loadConversations(), loadRooms()]).then(() => {
      rosterReady = true;
    }).catch(() => { rosterError = true; });
    void loadProfile();
  });

  $effect(() => {
    const requestedAgent = $page.url.searchParams.get("agent");
    if (rosterReady && requestedAgent && agents.some(agent => agent.id === requestedAgent)) {
      untrack(() => selectAgent(requestedAgent));
    }
  });

  // `?conversation=<id>` (a chat turn listed under /llm/work) opens that chat.
  let followedConversation: string | null = null;
  $effect(() => {
    const id = $page.url.searchParams.get("conversation");
    if (!rosterReady || !id || id === followedConversation) return;
    if (getConversations().some((c) => c.id === id)) {
      followedConversation = id;
      untrack(() => selectConversation(id));
    }
  });

  function onNew() {
    // "New chat" from a room starts a direct chat with the room's answerer.
    const id = getActiveAgentId() ?? agents[0]?.id;
    if (id) newConversation(id);
  }

  function onCompactPick(value: string) {
    if (value.startsWith("room:")) selectConversation(value.slice(5));
    else selectAgent(value);
  }
</script>

<svelte:head><title>{$t("llm.title")}</title></svelte:head>
<svelte:window onkeydown={detailsKeydown} />

{#if loadError || rosterError}
  <div class="load-error" role="alert">
    <span>{$t(loadError ?? "llm.surface.roster_load_error")}</span>
    <button type="button" class="button" disabled={reloading} onclick={retryLoading}>{$t("common.retry")}</button>
  </div>
{/if}

<div class="chat-toolbar">
  <div class="compact-agents">
    <label for="chat-agent">{$t("llm.rail.title")}</label>
    <select
      id="chat-agent"
      class="input"
      value={active?.kind === "group" ? `room:${active.id}` : (activeAgent?.id ?? "")}
      onchange={(e) => onCompactPick(e.currentTarget.value)}
    >
      {#each agents as agent (agent.id)}<option value={agent.id}>{agent.name}</option>{/each}
      {#if rooms.length > 0}
        <optgroup label={$t("assist.groups.rail_title") as string}>
          {#each rooms as room (room.id)}<option value={`room:${room.id}`}>{room.title}</option>{/each}
        </optgroup>
      {/if}
    </select>
    <button class="button" type="button" onclick={onNew} disabled={!agents.length}>{$t("llm.rail.new_chat")}</button>
    <button class="button" type="button" onclick={() => (creatingRoom = true)} disabled={!agents.length}>{$t("assist.groups.new")}</button>
  </div>
  <button bind:this={detailsTrigger} type="button" class="button details-toggle" aria-expanded={detailsOpen} aria-controls="agent-details" onclick={toggleDetails}>{$t("llm.chat.details")}</button>
</div>
<div class="llm-chat" class:with-details={detailsOpen} data-agent-readiness={rosterError ? "error" : !rosterReady ? "loading" : turn ? "streaming" : "ready"}>
  <div class="agent-rail"><Rail {agents} speakingAgentId={turn?.agentId ?? null} onselect={selectAgent} onnew={onNew} /></div>
  <Conversation agent={activeAgent} />
  {#if detailsOpen}<aside bind:this={detailsPanel} id="agent-details" class="agent-details" aria-label={$t("llm.chat.details")}><button bind:this={detailsClose} type="button" class="button details-close" onclick={toggleDetails}>{$t("common.close")}</button><Inspector agent={activeAgent} /></aside>{/if}
</div>

{#if creatingRoom}<RoomEditor {agents} onclose={() => (creatingRoom = false)} />{/if}

<style>
  .chat-toolbar { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) var(--space-4); border-bottom: 1px solid var(--separator); flex-wrap: wrap; }
  .details-toggle { margin-left: auto; }
  .compact-agents { display: none; align-items: center; gap: var(--space-2); min-width: 0; flex-wrap: wrap; }
  .compact-agents select { max-width: 220px; }
  .llm-chat { flex: 1; min-height: 0; min-width: 0; display: flex; position: relative; }
  .agent-rail, .agent-details { display: flex; min-height: 0; flex-shrink: 0; }
  .agent-details { overflow: auto; flex-direction: column; }
  .details-close { align-self: flex-end; margin: var(--space-2); }
  .load-error { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); padding: var(--space-3); background: var(--accent-soft); }
  @media (max-width: 1100px) {
    .agent-details { position: absolute; right: 0; top: 0; bottom: 0; z-index: 10; background: var(--surface); box-shadow: var(--shadow-lg); max-width: 100%; }
  }
  @media (max-width: 820px) {
    .agent-rail { display: none; }
    .compact-agents { display: flex; }
  }
</style>
