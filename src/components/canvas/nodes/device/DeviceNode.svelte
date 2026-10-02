<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { Lod } from "$lib/canvas/lod";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import {
    actDevice,
    closeDevice,
    deviceState,
    frameSrc,
    listDevices,
    onDeviceFrame,
    onDeviceState,
    openDevice,
    toDevicePoint,
    watchDevice,
    type DeviceEntry,
    type DeviceInfo,
    type DevicePlatform,
  } from "$lib/canvas/devices/client";
  import Icon from "../../chrome/Icon.svelte";

  let {
    node,
    lod = "near",
  }: {
    node: CanvasNode;
    lod?: Lod;
  } = $props();

  const TAP_SLOP = 6;
  const VIS_MS = 500;

  const engine = useEngine();
  const platform = $derived((node.data.portalKind === "ios" ? "ios" : "android") as DevicePlatform);
  const savedId = $derived(typeof node.data.deviceId === "string" ? node.data.deviceId : "");

  let info = $state<DeviceInfo | null>(null);
  let frame = $state<{ file: string; seq: number; w: number; h: number; scale: number } | null>(null);
  let devices = $state<DeviceEntry[]>([]);
  let choice = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let screenEl = $state<HTMLDivElement | null>(null);
  let rootEl = $state<HTMLDivElement | null>(null);
  let watching = false;

  const canTouch = $derived(!!info?.capabilities?.tap);
  const mine = $derived(devices.filter((d) => d.platform === platform));
  const picked = $derived(mine.find((d) => d.id === choice));

  function useInfo(i: DeviceInfo | null) {
    info = i;
    if (i?.frame) frame = { file: i.frame.path, seq: i.frame.seq, w: i.frame.w, h: i.frame.h, scale: i.scale };
  }

  async function refreshList() {
    try {
      const l = await listDevices();
      devices = l.devices;
      if (!choice) choice = savedId || mine.find((d) => d.state === "booted")?.id || mine[0]?.id || "";
    } catch (e) {
      error = String(e);
    }
  }

  async function attach() {
    if (busy) return;
    busy = true;
    error = null;
    try {
      useInfo(await openDevice(node.id, platform, choice || savedId || undefined));
      watching = false;
      syncWatch();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function detach() {
    busy = true;
    try {
      await closeDevice(node.id);
      info = null;
      frame = null;
      watching = false;
      void refreshList();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function act(action: string, args: Record<string, unknown> = {}) {
    if (!info) return;
    actDevice(node.id, action, args).catch((e) => (error = String(e)));
  }

  function isVisible(): boolean {
    if (!info || !rootEl || lod === "far") return false;
    if (typeof document !== "undefined" && document.visibilityState !== "visible") return false;
    const r = rootEl.getBoundingClientRect();
    return r.width > 0 && r.bottom > 0 && r.right > 0 && r.top < window.innerHeight && r.left < window.innerWidth;
  }

  function syncWatch() {
    const v = isVisible();
    if (v === watching) return;
    watching = v;
    watchDevice(node.id, v).catch(() => {});
  }

  let down: { x: number; y: number; at: number; px: number; py: number } | null = null;

  function onDown(e: PointerEvent) {
    if (!canTouch || !frame || !screenEl || e.button !== 0) return;
    const p = toDevicePoint(screenEl.getBoundingClientRect(), frame, e.clientX, e.clientY);
    if (!p) return;
    e.preventDefault();
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    down = { ...p, at: performance.now(), px: e.clientX, py: e.clientY };
  }

  function onUp(e: PointerEvent) {
    const d = down;
    down = null;
    if (!d || !frame || !screenEl) return;
    e.stopPropagation();
    const moved = Math.hypot(e.clientX - d.px, e.clientY - d.py);
    if (moved < TAP_SLOP) {
      act("tap", { x: d.x, y: d.y });
      return;
    }
    const r = screenEl.getBoundingClientRect();
    const fit = Math.min(r.width / frame.w, r.height / frame.h) || 1;
    const s = frame.scale || 1;
    const x2 = Math.round(d.x + (e.clientX - d.px) / fit / s);
    const y2 = Math.round(d.y + (e.clientY - d.py) / fit / s);
    const durationMs = Math.min(2000, Math.max(80, Math.round(performance.now() - d.at)));
    const maxX = Math.floor(frame.w / s);
    const maxY = Math.floor(frame.h / s);
    act("swipe", {
      x: d.x,
      y: d.y,
      x2: Math.min(maxX, Math.max(0, x2)),
      y2: Math.min(maxY, Math.max(0, y2)),
      durationMs,
    });
  }

  const offs = [
    onDeviceFrame((f) => {
      if (f.nodeId !== node.id) return;
      frame = { file: f.file, seq: f.seq, w: f.w, h: f.h, scale: f.scale };
      error = null;
    }),
    onDeviceState((s) => {
      if (s.nodeId !== node.id) return;
      if (s.closed) {
        info = null;
        frame = null;
        watching = false;
      } else if (s.error) {
        error = s.error;
      } else if (s.platform) {
        useInfo(s as DeviceInfo);
      }
    }),
  ];

  let timer: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    void deviceState(node.id)
      .then((i) => {
        useInfo(i);
        syncWatch();
      })
      .catch(() => {});
    void refreshList();
    timer = setInterval(syncWatch, VIS_MS);
    document.addEventListener("visibilitychange", syncWatch);
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
    document.removeEventListener("visibilitychange", syncWatch);
    for (const off of offs) void off.then((f) => f()).catch(() => {});
    const id = node.id;
    if (!engine.store.node(id)) void closeDevice(id).catch(() => {});
    else if (watching) void watchDevice(id, false).catch(() => {});
  });
</script>

<div class="device" bind:this={rootEl}>
  {#if lod !== "far"}
    <div class="bar" class:attached={!!info} data-canvas-chrome>
      <span class="tag">{platform === "ios" ? "iOS" : "Android"}</span>
      {#if info}
        <span class="name" title={info.device.name}>{info.device.name}</span>
        <button
          type="button"
          class="nav"
          title={$t("canvas.portal.deviceNode.home", { default: "Home" })}
          aria-label={$t("canvas.portal.deviceNode.home", { default: "Home" })}
          disabled={!info.capabilities.home}
          onclick={() => act("home")}
        >
          <Icon d="M4 11l8-7 8 7 M6 10v10h12V10" size={14} />
        </button>
        {#if platform === "android"}
          <button
            type="button"
            class="nav"
            title={$t("canvas.portal.back", { default: "Back" })}
            aria-label={$t("canvas.portal.back", { default: "Back" })}
            onclick={() => act("key", { key: "back" })}
          >
            <Icon d="M15 6l-6 6 6 6" size={14} />
          </button>
        {/if}
        <button
          type="button"
          class="nav"
          title={$t("canvas.portal.deviceNode.detach", { default: "Detach" })}
          aria-label={$t("canvas.portal.deviceNode.detach", { default: "Detach" })}
          disabled={busy}
          onclick={detach}
        >
          <Icon d="M6 6l12 12 M18 6L6 18" size={14} />
        </button>
      {:else}
        <select
          class="pick"
          aria-label={$t("canvas.portal.deviceNode.pick", { default: "Device" })}
          bind:value={choice}
          onfocus={refreshList}
        >
          {#if !mine.length}
            <option value="">{$t("canvas.portal.deviceNode.none", { default: "No devices found" })}</option>
          {/if}
          {#each mine as d (d.id)}
            <option value={d.id}>{d.name}{d.os ? ` · ${d.os}` : ""}{d.state === "booted" ? " ●" : ""}</option>
          {/each}
        </select>
        <button type="button" class="go" disabled={busy || !mine.length} onclick={attach}>
          {busy
            ? $t("canvas.portal.deviceNode.starting", { default: "Starting…" })
            : picked && picked.state !== "booted"
              ? $t("canvas.portal.deviceNode.boot", { default: "Boot" })
              : $t("canvas.portal.deviceNode.attach", { default: "Attach" })}
        </button>
      {/if}
    </div>
  {/if}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="screen"
    class:touch={canTouch}
    bind:this={screenEl}
    onpointerdown={onDown}
    onpointerup={onUp}
    onpointercancel={() => (down = null)}
  >
    {#if frame}
      <img src={frameSrc(frame.file, frame.seq)} alt={info?.device.name ?? ""} draggable="false" />
    {:else}
      <div class="empty">
        <Icon d="M7 3h10v18H7z M11 18h2" size={28} />
        <div class="title">
          {platform === "ios"
            ? $t("canvas.portal.iosTitle", { default: "iOS Simulator" })
            : $t("canvas.portal.androidTitle", { default: "Android emulator" })}
        </div>
        {#if lod !== "far"}
          <div class="sub">
            {info
              ? $t("canvas.portal.loading", { default: "Loading…" })
              : $t("canvas.portal.deviceNode.hint", { default: "Pick a device and attach it" })}
          </div>
        {/if}
      </div>
    {/if}
    {#if info && !canTouch && lod === "near"}
      <div class="note">{$t("canvas.portal.deviceNode.noTouch", { default: "View only: install idb_companion for touch" })}</div>
    {/if}
    {#if error && lod !== "far"}
      <div class="error" title={error}>{error}</div>
    {/if}
  </div>
</div>

<style>
  .device {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    color: var(--text);
  }
  .bar {
    flex: none;
    height: 32px;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 6px;
    border-bottom: 1px solid var(--border);
    background: var(--popup-bg, var(--surface));
    font-size: 12px;
  }
  .bar.attached {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    z-index: 2;
    border-bottom: none;
    background: color-mix(in srgb, var(--popup-bg, var(--surface)) 86%, transparent);
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .device:hover .bar.attached,
  .bar.attached:focus-within {
    opacity: 1;
  }
  .tag {
    flex: none;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text-muted);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .nav {
    all: unset;
    flex: none;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .nav:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text) 8%, transparent);
    color: var(--text);
  }
  .nav:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .pick {
    flex: 1;
    min-width: 0;
    height: 22px;
    font-size: 11px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--input-bg, var(--surface));
    color: var(--text);
  }
  .go {
    all: unset;
    flex: none;
    cursor: pointer;
    padding: 2px 10px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 600;
    background: var(--accent);
    color: var(--on-accent, #fff);
  }
  .go:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .screen {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    background: transparent;
    user-select: none;
  }
  .screen.touch {
    cursor: pointer;
    touch-action: none;
  }
  .screen img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
    pointer-events: none;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 16px;
    text-align: center;
    color: var(--text-muted);
    background: var(--surface);
  }
  .title {
    font-weight: 600;
    color: var(--text);
  }
  .sub {
    font-size: 12px;
  }
  .note,
  .error {
    position: absolute;
    left: 8px;
    right: 8px;
    padding: 6px 8px;
    border-radius: 6px;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    top: 8px;
    background: color-mix(in srgb, var(--surface) 85%, transparent);
    color: var(--text-muted);
  }
  .error {
    bottom: 8px;
    background: color-mix(in srgb, var(--danger, #d33) 16%, var(--surface));
    color: var(--text);
  }
</style>
