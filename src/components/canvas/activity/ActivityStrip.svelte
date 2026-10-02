<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { switchTo } from "$lib/canvas/palette/navigate";
  import Icon from "../chrome/Icon.svelte";
  import { actorColor, aiActivity, initials, VERB_DEFAULTS, type ActivityEntry } from "$lib/canvas/activity/store.svelte";

  const engine = useEngine();
  const WINDOW_MS = 30_000;
  const STALE_MS = 10_000;
  const LIVE_MS = 4_000;
  const MAX_CHIPS = 3;
  const PULSE = "M3 12h4l3-8 4 16 3-8h4";

  let now = $state(Date.now());

  const recent = $derived(aiActivity.recent(now, WINDOW_MS));
  const shown = $derived(recent.slice(0, MAX_CHIPS));
  const more = $derived(Math.max(0, recent.length - MAX_CHIPS));
  const followName = $derived.by(() => {
    if (!aiActivity.follow) return null;
    if (!aiActivity.followOrigin) return $t("canvas.activity.anyAi", { default: "Any AI" });
    return aiActivity.actors().find((a) => a.origin === aiActivity.followOrigin)?.name ?? aiActivity.followOrigin;
  });

  function verb(e: ActivityEntry): string {
    return $t(`canvas.activity.verb.${e.verb}`, { default: VERB_DEFAULTS[e.verb] ?? e.verb });
  }

  function target(e: ActivityEntry): string {
    if (e.target) return `“${e.target}”`;
    if (e.count > 1) return $t("canvas.activity.nodes", { default: "{{count}} nodes", count: e.count });
    return $t(`canvas.activity.kind.${e.targetKind ?? "node"}`, { default: e.targetKind ?? "a node" });
  }

  async function go(e: ActivityEntry) {
    if (e.floorId && e.floorId !== engine.store.floorId) await switchTo(engine, e.workspaceId, e.floorId);
    const nodes = e.nodeIds.map((id) => engine.store.node(id)).filter((n) => !!n);
    if (!nodes.length) {
      aiActivity.show("activity");
      return;
    }
    const x0 = Math.min(...nodes.map((n) => n!.x));
    const y0 = Math.min(...nodes.map((n) => n!.y));
    const x1 = Math.max(...nodes.map((n) => n!.x + n!.w));
    const y1 = Math.max(...nodes.map((n) => n!.y + n!.h));
    engine.selection.set(nodes.map((n) => n!.id));
    engine.flyToRect({ x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, 1);
  }

  onMount(() => {
    const timer = setInterval(() => {
      if (recent.length || aiActivity.entries.length) now = Date.now();
    }, 1000);
    return () => clearInterval(timer);
  });
</script>

<div
  class="strip"
  class:busy={shown.length > 0}
  data-canvas-chrome
  data-canvas-occluder
  role="region"
  aria-label={$t("canvas.activity.strip.label", { default: "Who is working on the canvas" })}
>
  <button
    type="button"
    class="open"
    class:on={aiActivity.open}
    aria-expanded={aiActivity.open}
    title={$t("canvas.activity.strip.open", { default: "Open the activity of this workspace" })}
    onclick={() => aiActivity.toggle("activity")}
  >
    <Icon d={PULSE} size={15} />
    {#if !shown.length}<span class="idle">{$t("canvas.activity.title", { default: "Activity" })}</span>{/if}
  </button>
  {#each shown as a (a.origin)}
    {@const age = now - a.last.at}
    <button
      type="button"
      class="chip"
      class:stale={age > STALE_MS}
      class:live={age <= LIVE_MS}
      style:--c={actorColor(a.origin)}
      title={a.last.summary}
      onclick={() => void go(a.last)}
    >
      <span class="avatar" aria-hidden="true">{initials(a.name)}</span>
      <span class="who">{a.name}</span>
      <span class="what">{verb(a.last)} {target(a.last)}</span>
    </button>
  {/each}
  {#if more}
    <button type="button" class="more" onclick={() => aiActivity.show("activity")}>
      {$t("canvas.activity.strip.more", { default: "+{{count}}", count: more })}
    </button>
  {/if}
  {#if followName}
    <span class="follow">
      <span>{$t("canvas.activity.strip.following", { default: "Following {{name}}", name: followName })}</span>
      <button
        type="button"
        class="stop"
        aria-label={$t("canvas.activity.strip.stopFollow", { default: "Stop following" })}
        onclick={() => {
          aiActivity.follow = false;
          aiActivity.followOrigin = null;
        }}>×</button
      >
    </span>
  {/if}
</div>

<style>
  .strip {
    position: absolute;
    left: calc(var(--sidebar-w, 0px) + var(--dock-l, 0px) + 12px);
    bottom: 12px;
    z-index: 10;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: max(150px, calc(100% - var(--sidebar-w, 0px) - var(--dock-l, 0px) - var(--dock-r, 0px) - 420px));
    min-width: 0;
    padding: 3px;
    border-radius: 10px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.2);
    font-size: 12px;
    color: var(--text);
    overflow: hidden;
  }
  button {
    all: unset;
    box-sizing: border-box;
    cursor: pointer;
  }
  button:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
  }
  .open {
    flex: none;
    height: 28px;
    min-width: 28px;
    padding: 0 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border-radius: 6px;
    color: var(--text-muted);
  }
  .open:hover,
  .open.on {
    color: var(--text);
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
  }
  .idle {
    font-weight: 500;
    padding-right: 2px;
  }
  .chip {
    min-width: 0;
    height: 28px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px 0 3px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--c) 14%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c) 40%, transparent);
    opacity: 1;
    transition:
      opacity 600ms ease,
      background 200ms ease;
    animation: arrive 260ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }
  .chip:hover {
    background: color-mix(in srgb, var(--c) 24%, transparent);
  }
  .chip.stale {
    opacity: 0.55;
  }
  .avatar {
    position: relative;
    flex: none;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 9px;
    font-weight: 700;
    color: #fff;
    background: var(--c);
  }
  .chip.live .avatar::after {
    content: "";
    position: absolute;
    inset: -3px;
    border-radius: 50%;
    border: 2px solid var(--c);
    animation: ping 1.4s ease-out infinite;
  }
  .who {
    font-weight: 600;
    white-space: nowrap;
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .what {
    color: var(--text-muted);
    white-space: nowrap;
    max-width: 200px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .more {
    flex: none;
    height: 28px;
    padding: 0 8px;
    border-radius: 6px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .more:hover {
    color: var(--text);
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
  }
  .follow {
    flex: none;
    height: 28px;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0 2px 0 8px;
    border-radius: 7px;
    color: var(--accent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 50%, transparent);
    white-space: nowrap;
  }
  .stop {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 5px;
    font-size: 15px;
    line-height: 1;
  }
  .stop:hover {
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
  }
  @keyframes arrive {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
  @keyframes ping {
    from {
      opacity: 0.8;
      transform: scale(0.9);
    }
    to {
      opacity: 0;
      transform: scale(1.5);
    }
  }
  @media (max-width: 720px) {
    .what {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .chip,
    .chip.live .avatar::after {
      animation: none;
      transition: none;
    }
  }
</style>
