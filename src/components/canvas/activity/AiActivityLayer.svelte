<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { Rect } from "$lib/canvas/model/types";
  import {
    ACTIVITY_EVENT,
    actorColor,
    aiActivity,
    baseOrigin,
    isAi,
    reducedMotion,
    type ActivityEntry,
  } from "$lib/canvas/activity/store.svelte";
  import ActivityPanel from "./ActivityPanel.svelte";
  import ActivityStrip from "./ActivityStrip.svelte";

  const engine = useEngine();
  const PRESENCE_MS = 8_000;
  const FADE_MS = 600;
  const GLOW_MS = 3_200;
  const FOLLOW_MS = 1_200;

  interface Mark {
    key: string;
    name: string;
    color: string;
    nodeId: string;
    at: number;
  }

  let presence = new Map<string, Mark>();
  let glows = new Map<string, Mark>();
  let cursors = $state<Array<{ key: string; name: string; color: string; x: number; y: number; fading: boolean }>>([]);
  let rings = $state<Array<{ key: string; name: string; color: string; x: number; y: number; w: number; h: number; fading: boolean }>>([]);
  let lastFly = 0;
  let pendingFly: ReturnType<typeof setTimeout> | null = null;
  let still = $state(reducedMotion());

  function here(e: ActivityEntry): boolean {
    return e.workspaceId === engine.store.workspaceId && e.floorId === engine.store.floorId;
  }

  function onEntry(e: ActivityEntry) {
    if (!aiActivity.push(e) || !isAi(e) || !here(e)) return;
    const ids = e.nodeIds.filter((id) => !!engine.store.node(id));
    const key = baseOrigin(e.origin);
    const color = actorColor(e.origin);
    const now = Date.now();
    if (aiActivity.presence) {
      if (ids.length) presence.set(key, { key, name: e.actor, color, nodeId: ids[ids.length - 1], at: now });
      if (e.verb !== "deleted") for (const id of ids) glows.set(id, { key: id, name: e.actor, color, nodeId: id, at: now });
    }
    if (aiActivity.follow && ids.length && (!aiActivity.followOrigin || aiActivity.followOrigin === key)) follow(ids);
    place();
    engine.invalidate();
  }

  function follow(ids: string[]) {
    const rects: Rect[] = ids.map((id) => engine.store.node(id)).filter((n) => !!n).map((n) => ({ x: n!.x, y: n!.y, w: n!.w, h: n!.h }));
    if (!rects.length) return;
    const x0 = Math.min(...rects.map((r) => r.x));
    const y0 = Math.min(...rects.map((r) => r.y));
    const x1 = Math.max(...rects.map((r) => r.x + r.w));
    const y1 = Math.max(...rects.map((r) => r.y + r.h));
    const target = { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
    const go = () => {
      lastFly = Date.now();
      pendingFly = null;
      if (still) {
        engine.camera.set(engine.measureChrome().fitRect(target, 64, 1));
        engine.invalidate();
      } else {
        engine.flyToRect(target, 1);
      }
    };
    if (pendingFly) clearTimeout(pendingFly);
    const wait = FOLLOW_MS - (Date.now() - lastFly);
    if (wait <= 0) go();
    else pendingFly = setTimeout(go, wait);
  }

  function place() {
    const now = Date.now();
    const z = engine.camera.zoom;
    for (const [k, m] of presence) if (now - m.at > PRESENCE_MS + FADE_MS || !engine.store.node(m.nodeId)) presence.delete(k);
    for (const [k, m] of glows) if (now - m.at > GLOW_MS + FADE_MS || !engine.store.node(m.nodeId)) glows.delete(k);
    const nextCursors: typeof cursors = [];
    const taken = new Map<string, number>();
    for (const m of presence.values()) {
      const n = engine.store.node(m.nodeId);
      if (!n) continue;
      const stack = taken.get(n.id) ?? 0;
      taken.set(n.id, stack + 1);
      const p = engine.camera.worldToScreen(n.x + n.w, n.y);
      nextCursors.push({ key: m.key, name: m.name, color: m.color, x: p.x - 6, y: p.y + 6 + stack * 24, fading: now - m.at > PRESENCE_MS });
    }
    const nextRings: typeof rings = [];
    for (const m of glows.values()) {
      const n = engine.store.node(m.nodeId);
      if (!n) continue;
      const p = engine.camera.worldToScreen(n.x, n.y);
      nextRings.push({ key: m.key, name: m.name, color: m.color, x: p.x, y: p.y, w: n.w * z, h: n.h * z, fading: now - m.at > GLOW_MS });
    }
    const sameC =
      nextCursors.length === cursors.length &&
      nextCursors.every((c, i) => c.key === cursors[i].key && c.fading === cursors[i].fading && Math.abs(c.x - cursors[i].x) < 0.5 && Math.abs(c.y - cursors[i].y) < 0.5);
    if (!sameC) cursors = nextCursors;
    const sameR =
      nextRings.length === rings.length &&
      nextRings.every((r, i) => r.key === rings[i].key && r.fading === rings[i].fading && Math.abs(r.x - rings[i].x) < 0.5 && Math.abs(r.y - rings[i].y) < 0.5 && Math.abs(r.w - rings[i].w) < 0.5);
    if (!sameR) rings = nextRings;
  }

  $effect(() => {
    if (aiActivity.presence) return;
    presence = new Map();
    glows = new Map();
    cursors = [];
    rings = [];
  });

  $effect(() => {
    const ws = engine.store.workspaceId;
    presence = new Map();
    glows = new Map();
    cursors = [];
    rings = [];
    if (ws) void aiActivity.load(ws);
  });

  onMount(() => {
    const media = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    const onMotion = () => (still = !!media?.matches);
    media?.addEventListener?.("change", onMotion);
    const stopFrame = engine.onFrame(() => place());
    const timer = setInterval(() => {
      if (presence.size || glows.size || cursors.length || rings.length) place();
    }, 250);
    const off = listen<ActivityEntry>(ACTIVITY_EVENT, (e) => onEntry(e.payload)).catch(() => () => {});
    return () => {
      media?.removeEventListener?.("change", onMotion);
      stopFrame();
      clearInterval(timer);
      if (pendingFly) clearTimeout(pendingFly);
      void off.then((f) => f());
    };
  });
</script>

{#each rings as r (r.key)}
  <div
    class="ring"
    class:fading={r.fading}
    class:still
    style:left="{r.x}px"
    style:top="{r.y}px"
    style:width="{r.w}px"
    style:height="{r.h}px"
    style:--c={r.color}
    aria-hidden="true"
  >
    <span class="by">{$t("canvas.activity.editedBy", { default: "edited by {{name}}", name: r.name })}</span>
  </div>
{/each}

{#each cursors as c (c.key)}
  <div
    class="cursor"
    class:fading={c.fading}
    class:still
    style:left="{c.x}px"
    style:top="{c.y}px"
    style:--c={c.color}
    role="status"
    aria-label={$t("canvas.activity.workingHere", { default: "{{name}} is working here", name: c.name })}
  >
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><path d="M2 1.5l12 5.2-5.3 1.6L6.9 14z" fill="var(--c)" stroke="#fff" stroke-width="1.2" stroke-linejoin="round" /></svg>
    <span>{c.name}</span>
  </div>
{/each}

<ActivityStrip />

{#if aiActivity.open}
  <div class="dock" data-canvas-chrome data-canvas-occluder>
    <ActivityPanel />
  </div>
{/if}

<style>
  .ring {
    position: absolute;
    z-index: 8;
    pointer-events: none;
    border-radius: 12px;
    box-shadow:
      0 0 0 2px var(--c),
      0 0 18px 2px color-mix(in srgb, var(--c) 55%, transparent);
    opacity: 1;
    transition: opacity 600ms ease;
    animation: pulse 1.2s ease-out 1;
  }
  .ring.fading {
    opacity: 0;
  }
  .ring .by {
    position: absolute;
    left: 0;
    bottom: calc(100% + 6px);
    font-size: 11px;
    font-weight: 600;
    line-height: 1;
    padding: 3px 7px;
    border-radius: 999px;
    color: #fff;
    background: var(--c);
    white-space: nowrap;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cursor {
    position: absolute;
    z-index: 9;
    display: flex;
    align-items: flex-start;
    gap: 2px;
    pointer-events: none;
    opacity: 1;
    transition:
      left 420ms cubic-bezier(0.2, 0.7, 0.2, 1),
      top 420ms cubic-bezier(0.2, 0.7, 0.2, 1),
      opacity 600ms ease;
  }
  .cursor.fading {
    opacity: 0;
  }
  .cursor span {
    margin-top: 10px;
    font-size: 11px;
    font-weight: 600;
    padding: 3px 7px;
    border-radius: 6px;
    color: #fff;
    background: var(--c);
    white-space: nowrap;
    box-shadow: 0 2px 8px rgb(0 0 0 / 0.25);
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .still,
  .still.fading {
    transition: none;
    animation: none;
  }
  .dock {
    position: absolute;
    top: 64px;
    right: calc(var(--dock-r, 0px) + 12px);
    z-index: 24;
    max-height: calc(100% - 140px);
    display: flex;
  }
  @keyframes pulse {
    from {
      box-shadow:
        0 0 0 6px color-mix(in srgb, var(--c) 35%, transparent),
        0 0 28px 6px color-mix(in srgb, var(--c) 60%, transparent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ring,
    .cursor {
      transition: none;
      animation: none;
    }
  }
</style>
