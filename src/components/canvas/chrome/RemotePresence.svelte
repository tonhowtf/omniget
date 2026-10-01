<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";

  interface Presence {
    id: string;
    name: string;
    nodeId: string | null;
    x: number | null;
    y: number | null;
    at: number;
  }

  const engine = useEngine();
  const TTL_MS = 30_000;
  const COLORS = ["#ff6b6b", "#4dabf7", "#51cf66", "#fcc419", "#cc5de8", "#ff922b"];

  let devices = $state<Presence[]>([]);
  let spots = $state<Array<{ id: string; name: string; sx: number; sy: number; color: string }>>([]);

  function colorOf(id: string): string {
    let h = 0;
    for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
    return COLORS[h % COLORS.length];
  }

  function place() {
    const now = Date.now();
    const live = devices.filter((d) => now - d.at < TTL_MS);
    if (live.length !== devices.length) devices = live;
    const next: typeof spots = [];
    for (const d of live) {
      let wx: number | null = null;
      let wy: number | null = null;
      if (d.nodeId) {
        const n = engine.store.node(d.nodeId);
        if (n) {
          wx = n.x + n.w / 2;
          wy = n.y;
        }
      } else if (d.x != null && d.y != null) {
        wx = d.x;
        wy = d.y;
      }
      if (wx == null || wy == null) continue;
      const p = engine.camera.worldToScreen(wx, wy);
      next.push({ id: d.id, name: d.name, sx: p.x, sy: p.y, color: colorOf(d.id) });
    }
    const same =
      next.length === spots.length &&
      next.every((s, i) => s.id === spots[i].id && Math.abs(s.sx - spots[i].sx) < 0.5 && Math.abs(s.sy - spots[i].sy) < 0.5);
    if (!same) spots = next;
  }

  onMount(() => {
    const stopFrame = engine.onFrame(() => place());
    const timer = setInterval(() => {
      place();
      if (devices.length) engine.invalidate();
    }, 1000);
    const offs = [
      listen<{ device?: { id?: string; name?: string }; nodeId?: string | null; x?: number | null; y?: number | null }>(
        "canvas://presence",
        (e) => {
          const p = e.payload;
          const id = p?.device?.id;
          if (!id) return;
          const entry: Presence = {
            id,
            name: p.device?.name || id.slice(0, 6),
            nodeId: p.nodeId ?? null,
            x: typeof p.x === "number" ? p.x : null,
            y: typeof p.y === "number" ? p.y : null,
            at: Date.now(),
          };
          devices = [...devices.filter((d) => d.id !== id), entry];
          engine.invalidate();
        },
      ).catch(() => () => {}),
      listen<{ dx?: number; dy?: number; zoom?: number | null }>("canvas://remote-camera", (e) => {
        const p = e.payload ?? {};
        if (p.dx || p.dy) engine.panBy(p.dx ?? 0, p.dy ?? 0);
        if (typeof p.zoom === "number" && p.zoom > 0) engine.setZoom(engine.camera.zoom * p.zoom);
        engine.invalidate();
      }).catch(() => () => {}),
    ];
    return () => {
      stopFrame();
      clearInterval(timer);
      for (const o of offs) void o.then((f) => f());
    };
  });
</script>

{#each spots as s (s.id)}
  <div
    class="cursor"
    style:left="{s.sx}px"
    style:top="{s.sy}px"
    style:--c={s.color}
    aria-label={$t("canvas.remote.presenceOf", { default: "{{name}} is looking here", name: s.name })}
  >
    <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"><path d="M1 1l11 5-5 1.5L5.5 13z" fill="var(--c)" /></svg>
    <span>{s.name}</span>
  </div>
{/each}

<style>
  .cursor {
    position: absolute;
    z-index: 9;
    display: flex;
    align-items: center;
    gap: 4px;
    pointer-events: none;
    transform: translate(-2px, -16px);
  }
  .cursor span {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 6px;
    color: #fff;
    background: var(--c);
    white-space: nowrap;
  }
</style>
