<script lang="ts">
  /**
   * Left rail: the roster grouped by role, with the agents that have no
   * conversation yet under "Unassigned", and a footer with the Marketplace
   * link and the local profile.
   *
   * The list is windowed on fixed row heights (header 28px, item 56px), the
   * same idea as `components/omnidisc/VirtualList.svelte` but without the
   * measuring pass: every row here has a known height, so the visible range is
   * arithmetic and 50 agents cost the same as 5. No observer, no timer.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import type { AgentDef } from "$lib/llm/types";
  import {
    getActiveAgentId,
    getConversations,
    isDemoRoster,
    lastSpokenBy,
  } from "$lib/stores/llm-store.svelte";
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
    | { kind: "agent"; key: string; agent: AgentDef };

  let viewport = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let viewportHeight = $state(480);
  let query = $state("");

  let conversations = $derived(getConversations());
  let activeAgentId = $derived(getActiveAgentId());

  let filtered = $derived(
    query.trim()
      ? agents.filter((a) => a.name.toLowerCase().includes(query.trim().toLowerCase()))
      : agents,
  );

  /** Agents with at least one conversation are "in the team"; the rest are unassigned. */
  let rows = $derived.by<Row[]>(() => {
    const assigned = new Set(conversations.map((c) => c.agentId));
    const team = filtered.filter((a) => assigned.has(a.id));
    const unassigned = filtered.filter((a) => !assigned.has(a.id));
    const out: Row[] = [];
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
      bind:value={query}
      placeholder={$t("llm.rail.search")}
      aria-label={$t("llm.rail.search")}
    />
    <button type="button" class="button primary rail-new" disabled={!agents.length} onclick={onnew}>
      {$t("llm.rail.new_chat")}
    </button>
  </div>

  {#if isDemoRoster()}
    <p class="rail-demo" role="status">{$t("llm.demo_banner")}</p>
  {/if}

  <div class="rail-scroll" bind:this={viewport} onscroll={onScroll}>
    {#if rows.length === 0}
      <p class="rail-empty">{$t("llm.rail.empty")}</p>
    {:else}
      <div style:padding-top="{padTop}px" style:padding-bottom="{padBottom}px">
        {#each visible as row (row.key)}
          {#if row.kind === "header"}
            <div class="rail-group">{row.label}</div>
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

  .rail-new {
    width: 100%;
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
