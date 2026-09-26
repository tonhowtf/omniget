<script lang="ts">
  /**
   * Chat: rail, conversation, inspector. The roster loads once when the page
   * mounts; nothing here polls, and the only timer that can exist belongs to a
   * running turn.
   */
  import { onMount, untrack } from "svelte";
  import { page } from "$app/stores";
  import { t } from "$lib/i18n";
  import {
    getActiveTurn,
    getAgent,
    getAgents,
    getActiveAgentId,
    getActiveConversation,
    getRooms,
    loadConversations,
    loadRooms,
    loadRoster,
    selectConversation,
    seedDemoConversation,
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
  let agents = $derived(getAgents());
  let activeAgent = $derived(getAgent(getActiveAgentId()));
  let active = $derived(getActiveConversation());
  let rooms = $derived(getRooms());
  let turn = $derived(getActiveTurn());

  onMount(() => {
    // Saved chats (old ones reopen as direct chats, same ids) and rooms load
    // with the roster; nothing here polls.
    void Promise.all([loadRoster(), loadConversations(), loadRooms()]).then(() => {
      seedDemoConversation();
      rosterReady = true;
    });
    void loadProfile();
  });

  $effect(() => {
    const requestedAgent = $page.url.searchParams.get("agent");
    if (rosterReady && requestedAgent && agents.some(agent => agent.id === requestedAgent)) {
      untrack(() => selectAgent(requestedAgent));
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
  <button type="button" class="button details-toggle" aria-expanded={detailsOpen} aria-controls="agent-details" onclick={() => detailsOpen = !detailsOpen}>{$t("llm.chat.details")}</button>
</div>
<div class="llm-chat" class:with-details={detailsOpen}>
  <div class="agent-rail"><Rail {agents} speakingAgentId={turn?.agentId ?? null} onselect={selectAgent} onnew={onNew} /></div>
  <Conversation agent={activeAgent} />
  {#if detailsOpen}<div id="agent-details" class="agent-details"><Inspector agent={activeAgent} /></div>{/if}
</div>

{#if creatingRoom}<RoomEditor {agents} onclose={() => (creatingRoom = false)} />{/if}

<style>
  .chat-toolbar { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-2) var(--space-4); border-bottom: 1px solid var(--separator); flex-wrap: wrap; }
  .details-toggle { margin-left: auto; }
  .compact-agents { display: none; align-items: center; gap: var(--space-2); min-width: 0; flex-wrap: wrap; }
  .compact-agents select { max-width: 220px; }
  .llm-chat { flex: 1; min-height: 0; min-width: 0; display: flex; position: relative; }
  .agent-rail, .agent-details { display: flex; min-height: 0; flex-shrink: 0; }
  .agent-details { overflow: auto; }
  @media (max-width: 1100px) {
    .agent-details { position: absolute; right: 0; top: 0; bottom: 0; z-index: 10; background: var(--surface); box-shadow: var(--shadow-lg); max-width: 100%; }
  }
  @media (max-width: 820px) {
    .agent-rail { display: none; }
    .compact-agents { display: flex; }
  }
</style>
