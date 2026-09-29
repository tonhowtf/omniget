<script lang="ts">
  import { agentActions } from "$lib/native-agent/actions";
  /**
   * Left rail: switch between the agent roster and saved direct conversations.
   * Rooms remain in the roster view and the footer keeps the local profile.
   *
   * The list is windowed on fixed row heights (header 28px, item 56px) with no
   * measuring pass: every row here has a known height, so the visible range is
   * arithmetic and 50 agents cost the same as 5. No observer, no timer.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import type { AgentDef, Conversation, Room } from "$lib/llm/types";
  import {
    getActiveConversation,
    getConversations,
    getRooms,
    isDemoRoster,
    lastSpokenBy,
    selectConversation,
  } from "$lib/stores/llm-store.svelte";
  import RoomEditor from "./groups/RoomEditor.svelte";
  import RoomRailItem from "./groups/RoomRailItem.svelte";
  import { getProfile, isProfileAvailable } from "$lib/stores/profile-store.svelte";
  import RailItem from "./RailItem.svelte";

  let {
    agents,
    speakingAgentId = null,
    onselect,
    onnew,
  }: {
    agents: AgentDef[];
    speakingAgentId?: string | null;
    onselect: (id: string) => void;
    onnew: () => void;
  } = $props();

  const HEADER_H = 28;
  const ITEM_H = 56;
  const OVERSCAN = 4;

  type Row =
    | { kind: "header"; key: string; label: string }
    | { kind: "agent"; key: string; agent: AgentDef }
    | { kind: "room"; key: string; room: Room }
    | { kind: "conversation"; key: string; conversation: Conversation };

  let viewport = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let viewportHeight = $state(480);
  let query = $state("");
  let view = $state<"agents" | "chats">("agents");

  let conversations = $derived(getConversations());
  let active = $derived(getActiveConversation());
  // A room's answerer is not "the selected agent": only a direct chat highlights one.
  let activeAgentId = $derived(active && active.kind !== "group" ? active.agentId : null);
  let activeRoomId = $derived(active?.kind === "group" ? active.id : null);
  let rooms = $derived(getRooms());
  let creatingRoom = $state(false);
  let newRoomButton = $state<HTMLButtonElement | null>(null);

  let filtered = $derived(
    query.trim()
      ? agents.filter((a) => a.name.toLowerCase().includes(query.trim().toLowerCase()))
      : agents,
  );

  /** Agents with at least one conversation are "in the team"; the rest are unassigned. */
  let rows = $derived.by<Row[]>(() => {
    if (view === "chats") {
      const q = query.trim().toLocaleLowerCase();
      const chats = conversations
        .filter((conversation) => conversation.kind !== "group")
        .filter((conversation) => {
          if (!q) return true;
          const owner = agents.find((agent) => agent.id === conversation.agentId)?.name ?? conversation.agentId;
          return conversation.title.toLocaleLowerCase().includes(q) || owner.toLocaleLowerCase().includes(q);
        })
        .sort((a, b) => b.updatedAtMs - a.updatedAtMs);
      if (chats.length === 0) return [];
      return [
        { kind: "header", key: "h-conversations", label: $t("llm.rail.view_chats") },
        ...chats.map((conversation) => ({ kind: "conversation" as const, key: `conversation-${conversation.id}`, conversation })),
      ];
    }

    const assigned = new Set(conversations.filter((c) => c.kind !== "group").map((c) => c.agentId));
    const team = filtered.filter((a) => assigned.has(a.id));
    const unassigned = filtered.filter((a) => !assigned.has(a.id));
    const out: Row[] = [];
    const q = query.trim().toLowerCase();
    const shownRooms = q ? rooms.filter((r) => r.title.toLowerCase().includes(q)) : rooms;
    if (shownRooms.length > 0) {
      out.push({ kind: "header", key: "h-rooms", label: $t("assist.groups.rail_title") });
      for (const room of shownRooms) out.push({ kind: "room", key: `room-${room.id}`, room });
    }
    if (team.length > 0) {
      out.push({ kind: "header", key: "h-team", label: $t("llm.rail.team") });
      for (const agent of team) out.push({ kind: "agent", key: agent.id, agent });
    }
    if (unassigned.length > 0) {
      out.push({ kind: "header", key: "h-unassigned", label: $t("llm.rail.unassigned") });
      for (const agent of unassigned) out.push({ kind: "agent", key: agent.id, agent });
    }
    return out;
  });

  let offsets = $derived.by(() => {
    const out = new Array<number>(rows.length + 1);
    let acc = 0;
    for (let i = 0; i < rows.length; i++) {
      out[i] = acc;
      acc += rows[i].kind === "header" ? HEADER_H : ITEM_H;
    }
    out[rows.length] = acc;
    return out;
  });

  let total = $derived(offsets[offsets.length - 1] ?? 0);

  function indexAt(value: number): number {
    let lo = 0;
    let hi = rows.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (offsets[mid + 1] <= value) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  let range = $derived.by(() => {
    if (rows.length === 0) return { start: 0, end: 0 };
    const start = Math.max(0, indexAt(scrollTop) - OVERSCAN);
    const end = Math.min(rows.length, indexAt(scrollTop + viewportHeight) + 1 + OVERSCAN);
    return { start, end };
  });

  let visible = $derived(rows.slice(range.start, range.end));
  let padTop = $derived(offsets[range.start] ?? 0);
  let padBottom = $derived(Math.max(0, total - (offsets[range.end] ?? total)));

  let profile = $derived(getProfile());

  function onScroll() {
    if (viewport) scrollTop = viewport.scrollTop;
  }

  onMount(() => {
    // One read on mount and one per window resize: no ResizeObserver loop.
    const measure = () => {
      if (viewport) viewportHeight = viewport.clientHeight;
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  });
</script>

<aside class="llm-rail" aria-label={$t("llm.rail.title")}>
  <div class="rail-head">
    <input
      class="input input-search rail-search"
      type="search"
      data-agent-actions="fill"
      use:agentActions={["fill"]}
      bind:value={query}
      placeholder={$t(view === "chats" ? "llm.rail.search_chats" : "llm.rail.search")}
      aria-label={$t(view === "chats" ? "llm.rail.search_chats" : "llm.rail.search")}
    />
    <div class="rail-view-switch">
      <button type="button" data-agent-actions="click" use:agentActions={["click"]} aria-pressed={view === "agents"} class:active={view === "agents"} onclick={() => { view = "agents"; query = ""; }}>
        {$t("llm.rail.view_agents")}
      </button>
      <button type="button" data-agent-actions="click" use:agentActions={["click"]} aria-pressed={view === "chats"} class:active={view === "chats"} onclick={() => { view = "chats"; query = ""; }}>
        {$t("llm.rail.view_chats")}
      </button>
    </div>
    <div class="rail-actions">
      <button type="button" class="button primary rail-new" disabled={!agents.length} onclick={onnew}>
        {$t("llm.rail.new_chat")}
      </button>
      <button
        type="button"
        class="button rail-new"
        bind:this={newRoomButton}
        disabled={agents.length === 0}
        onclick={() => (creatingRoom = true)}
        title={$t("assist.groups.new_hint") as string}
      >
        {$t("assist.groups.new")}
      </button>
    </div>
  </div>

  {#if isDemoRoster()}
    <p class="rail-demo" role="status">{$t("llm.demo_banner")}</p>
  {/if}

  <div data-agent-actions="scroll" use:agentActions={["scroll"]} data-agent-region="agent-list" class="rail-scroll" bind:this={viewport} onscroll={onScroll}>
    {#if rows.length === 0}
      <p class="rail-empty">{$t(view === "chats" ? (query.trim() ? "llm.rail.no_chat_matches" : "llm.rail.no_chats") : "llm.rail.empty")}</p>
    {:else}
      <div style:padding-top="{padTop}px" style:padding-bottom="{padBottom}px">
        {#each visible as row (row.key)}
          {#if row.kind === "header"}
            <div class="rail-group">{row.label}</div>
          {:else if row.kind === "room"}
            <RoomRailItem room={row.room} active={row.room.id === activeRoomId} onselect={selectConversation} />
          {:else if row.kind === "conversation"}
            <button
              type="button"
              class="rail-chat-item"
              class:active={row.conversation.id === active?.id}
              aria-current={row.conversation.id === active?.id ? "true" : undefined}
              data-agent-actions="click"
              data-agent-region={`conversation-history-${row.conversation.id}`}
              use:agentActions={["click"]}
              onclick={() => selectConversation(row.conversation.id)}
            >
              <span class="rail-chat-title">{row.conversation.title || $t("llm.rail.new_chat")}</span>
              <span class="rail-chat-meta">
                {agents.find((agent) => agent.id === row.conversation.agentId)?.name ?? row.conversation.agentId}
                <span aria-hidden="true"> · </span>
                {new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(new Date(row.conversation.updatedAtMs))}
              </span>
            </button>
          {:else}
            <RailItem
              agent={row.agent}
              active={row.agent.id === activeAgentId}
              speaking={row.agent.id === speakingAgentId}
              preview={lastSpokenBy(row.agent.id)}
              {onselect}
            />
          {/if}
        {/each}
      </div>
    {/if}
  </div>

  <footer class="rail-foot">
    <a class="rail-foot-link" href="/llm/roster">{$t("llm.roster.new")}</a>
    {#if profile}
      <div class="rail-profile">
        <span class="rail-profile-dot" style:background={`rgb(${profile.skin.tint.join(",")})`}></span>
        <span class="rail-profile-nick">{profile.nickname}</span>
      </div>
    {:else if !isProfileAvailable()}
      <span class="rail-profile-nick dim">{$t("llm.rail.no_profile")}</span>
    {/if}
  </footer>
</aside>

{#if creatingRoom}
  <RoomEditor
    {agents}
    onclose={() => {
      creatingRoom = false;
      queueMicrotask(() => newRoomButton?.focus());
    }}
  />
{/if}

<style>
  .llm-rail {
    width: 264px;
    min-width: 264px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: var(--hairline) solid var(--separator);
    background: var(--surface);
  }

  .rail-head {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
  }

  .rail-search {
    width: 100%;
  }

  .rail-view-switch { display:flex; gap:2px; padding:2px; border:1px solid var(--separator); border-radius:var(--radius-md); background:var(--fill-1); }
  .rail-view-switch button { flex:1; min-width:0; border:0; border-radius:var(--radius-sm); padding:5px 8px; background:transparent; color:var(--text-dim); font:inherit; font-size:var(--text-sm); cursor:pointer; }
  .rail-view-switch button.active { background:var(--surface); color:var(--text); box-shadow:0 0 0 1px var(--separator); }
  .rail-view-switch button:focus-visible,.rail-chat-item:focus-visible { outline:var(--focus-ring); outline-offset:var(--focus-ring-offset); }
  .rail-chat-item { display:flex; flex-direction:column; justify-content:center; gap:2px; width:100%; height:56px; padding:0 var(--space-2); border:0; border-radius:var(--radius-md); background:transparent; color:var(--text); text-align:left; cursor:pointer; }
  .rail-chat-item:hover { background:var(--fill-2); }
  .rail-chat-item.active { background:var(--accent-soft); }
  .rail-chat-title,.rail-chat-meta { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .rail-chat-title { font-size:var(--text-sm); font-weight:600; }
  .rail-chat-meta { color:var(--text-dim); font-size:var(--text-caption); }

  .rail-actions {
    display: flex;
    gap: var(--space-2);
  }

  .rail-new {
    flex: 1;
    min-width: 0;
  }

  .rail-demo {
    margin: 0 var(--space-3) var(--space-2);
    font-size: var(--text-caption);
    color: var(--text-dim);
  }

  .rail-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--space-2) var(--space-2);
    contain: layout paint;
  }

  .rail-group {
    height: 28px;
    display: flex;
    align-items: center;
    padding: 0 var(--space-2);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: var(--track-wide, 0.04em);
    text-transform: uppercase;
    color: var(--text-dim);
  }

  .rail-empty {
    padding: var(--space-4) var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .rail-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3);
    border-top: var(--hairline) solid var(--separator);
  }

  .rail-foot-link {
    font-size: var(--text-sm);
    color: var(--accent-hi);
    text-decoration: none;
  }

  .rail-profile {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .rail-profile-dot {
    width: 18px;
    height: 18px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }

  .rail-profile-nick {
    font-size: var(--text-sm);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rail-profile-nick.dim {
    color: var(--text-dim);
  }
</style>
