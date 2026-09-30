<script lang="ts">
  // Web portal: a real browser (native webview, Rust `crate::portal`) placed
  // over the page area when the live policy allows it; otherwise the last
  // native snapshot of the page. The toolbar (URL, back/forward/reload,
  // device, storage) is DOM and belongs to the canvas chrome, so it answers
  // on the first click. iOS/Android portals are reserved: placeholder only.
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import type { CanvasNode } from "$lib/canvas/model/types";
  import {
    DEVICE_PRESETS,
    closePortal,
    copyPortalScreenshot,
    inspectPortal,
    lastSnapshot,
    navigatePortal,
    normalizeUrl,
    onPortalClosed,
    onPortalDrive,
    onPortalState,
    onSnapshot,
    openPortal,
    portalState,
    portalStates,
    setPortalUserAgent,
    trustPortalCert,
    type PortalDrive,
    type PortalState,
    type StorageScope,
  } from "$lib/canvas/portals/client";
  import { policyFor } from "$lib/canvas/portals/policy";
  import { designArm, designDisarm, onDesign, type DesignEvent } from "$lib/canvas/portals/design";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import Icon from "../chrome/Icon.svelte";
  import PortalDesignSheet from "./PortalDesignSheet.svelte";

  let {
    node,
    selected = false,
    focused = false,
    workspaceId = "",
    onpatch,
  }: {
    node: CanvasNode;
    selected?: boolean;
    focused?: boolean;
    zoom?: number;
    workspaceId?: string;
    onpatch?: (patch: Partial<CanvasNode>) => void;
  } = $props();

  /** Canvas node header + this toolbar, world px (Rust `NODE_HEADER`). */
  const TOOLBAR = 32;
  const DRIVE_BADGE_MS = 5000;

  const engine = useEngine();
  const policy = policyFor(engine);

  let hostEl = $state<HTMLDivElement | null>(null);
  let pageEl = $state<HTMLDivElement | null>(null);
  let live = $state(false);
  let pstate = $state<PortalState | null>(null);
  let snapshot = $state<string | undefined>(undefined);

  // What the host already knows about this portal (remount after scrolling away).
  $effect(() => {
    const id = node.id;
    pstate = portalState(id) ?? null;
    snapshot = lastSnapshot(id);
    // Events before this node mounted (a certificate refused while it was a
    // far-away card): ask the host once.
    void portalStates()
      .then((list) => {
        const s = list.find((x) => x.nodeId === id);
        if (s && node.id === id) pstate = s;
      })
      .catch(() => {});
  });
  let draft = $state("");
  let editing = $state(false);
  let drive = $state<PortalDrive | null>(null);
  let driveTimer: ReturnType<typeof setTimeout> | null = null;
  let error = $state<string | null>(null);

  const kind = $derived(String(node.data.portalKind ?? "web"));
  const url = $derived(String(node.data.url ?? ""));
  const scope = $derived((String(node.data.storageScope ?? "isolated")) as StorageScope);
  const shownUrl = $derived(pstate?.url && pstate.url !== "about:blank" ? pstate.url : url);
  const device = $derived(
    (node.data.device as { platform?: string } | undefined)?.platform ?? "responsive",
  );
  const certHost = $derived(pstate?.certError ?? "");
  const muted = $derived(node.data.muted === true);
  const chromeHidden = $derived(node.data.chromeHidden === true);
  const showSize = $derived(String(node.data.showSize ?? "never"));
  const sizeLabel = $derived.by(() => {
    const v = pstate?.viewport;
    const w = v ? v[0] : node.w;
    const h = v ? v[1] : node.h - TOOLBAR - 28;
    return `${Math.round(w)} × ${Math.round(h)}`;
  });
  const sizeShown = $derived(showSize === "always" || (showSize === "resize" && selected));
  const driver = $derived.by(() => {
    if (!drive) return "";
    const id = drive.by.startsWith("agent:") ? drive.by.slice(6) : drive.by;
    return engine.store.node(id)?.title ?? "";
  });

  $effect(() => {
    if (!editing) draft = shownUrl;
  });

  const offs = [
    onPortalState((s) => {
      if (s.nodeId !== node.id) return;
      pstate = s;
    }),
    onPortalClosed((id) => {
      if (id !== node.id) return;
      pstate = null;
      live = false;
    }),
    onSnapshot((id, img) => {
      if (id === node.id) snapshot = img;
    }),
    onPortalDrive((d) => {
      if (d.nodeId !== node.id) return;
      drive = d;
      if (driveTimer) clearTimeout(driveTimer);
      driveTimer = setTimeout(() => (drive = null), DRIVE_BADGE_MS);
    }),
  ];

  async function ensureOpen(): Promise<boolean> {
    if (kind !== "web" || !url) return false;
    if (pstate) return true;
    try {
      const body = pageEl?.getBoundingClientRect();
      const z = engine.isRaised(node.id) ? 1 : engine.camera.zoom || 1;
      pstate = await openPortal({
        nodeId: node.id,
        workspaceId,
        url,
        storageScope: scope,
        userAgent: typeof node.data.userAgent === "string" ? node.data.userAgent : undefined,
        width: body ? body.width / z : node.w,
        height: body ? body.height / z : node.h - TOOLBAR - 28,
      });
      error = null;
      return true;
    } catch (e) {
      error = String(e);
      return false;
    }
  }

  // Joins the live policy while this node is on screen ("near") with a URL.
  // A refused certificate shows the warning instead of the (blank) page; a
  // portal without chrome stays a snapshot until focused, so a drag moves it.
  $effect(() => {
    if (kind !== "web" || !url || certHost || (chromeHidden && !focused)) return;
    const off = policy.register({
      id: node.id,
      body: () => pageEl,
      host: () => hostEl?.closest<HTMLElement>("[data-node-id]") ?? hostEl,
      priority: () => {
        const r = pageEl?.getBoundingClientRect();
        const area = r ? r.width * r.height : 0;
        const f = engine.selection.focused === node.id ? 3e7 : engine.selection.has(node.id) ? 2e7 : 0;
        return f + area;
      },
      ensureOpen,
      onLive: (v) => (live = v),
      raised: () => engine.isRaised(node.id),
    });
    return () => {
      off();
      live = false;
    };
  });

  // Size, selection or focus changed: look again after it settles.
  $effect(() => {
    void node.x;
    void node.y;
    void node.w;
    void node.h;
    void node.z;
    void selected;
    void focused;
    policy.kick();
  });

  // Design mode (Maestri): brush in the toolbar arms an overlay in the page;
  // the pick comes back as `canvas://design-select` and opens the sheet.
  let designing = $state(false);
  let designEv = $state<DesignEvent | null>(null);
  const offDesign = onDesign((ev) => {
    if (ev.nodeId !== node.id) return;
    designing = false;
    if (ev.cancelled) {
      if (ev.error) error = ev.error;
      return;
    }
    designEv = ev;
  }).catch(() => () => {});

  async function toggleDesign() {
    if (designing) {
      designing = false;
      await designDisarm(node.id).catch(() => {});
      return;
    }
    if (!(await ensureOpen())) return;
    error = null;
    try {
      await designArm(
        node.id,
        $t("canvas.portal.design.hint", { default: "Drag a region or click an element · Esc cancels" }) as string,
      );
      designing = true;
      // The page needs the pointer: make sure the live browser is on top.
      engine.selection.set([node.id]);
      engine.selection.focus(node.id);
      policy.kick();
    } catch (e) {
      error = String(e);
    }
  }

  onDestroy(() => {
    void offDesign.then((f) => f());
    if (designing) void designDisarm(node.id).catch(() => {});
    for (const off of offs) off();
    if (driveTimer) clearTimeout(driveTimer);
  });

  function patchData(extra: Record<string, unknown>) {
    onpatch?.({ data: { ...node.data, ...extra } });
  }

  async function go(raw: string) {
    const next = normalizeUrl(raw);
    editing = false;
    if (!next) return;
    error = null;
    if (next !== url) patchData({ url: next });
    try {
      if (pstate) pstate = await navigatePortal(node.id, { url: next });
    } catch (e) {
      error = String(e);
    }
    policy.kick();
  }

  async function history(action: "back" | "forward" | "reload") {
    if (!url) return;
    try {
      pstate = await navigatePortal(node.id, { action });
    } catch (e) {
      error = String(e);
    }
  }

  function onUrlKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void go(draft);
      (e.currentTarget as HTMLInputElement).blur();
    } else if (e.key === "Escape") {
      e.preventDefault();
      editing = false;
      draft = shownUrl;
      (e.currentTarget as HTMLInputElement).blur();
    }
  }

  async function pickDevice(e: Event) {
    const id = (e.currentTarget as HTMLSelectElement).value;
    const p = DEVICE_PRESETS.find((d) => d.id === id);
    if (!p) {
      patchData({ device: null, userAgent: null });
      if (pstate) void setPortalUserAgent(node.id, null).catch(() => {});
      return;
    }
    onpatch?.({
      w: p.width,
      h: p.height + TOOLBAR + 28,
      data: { ...node.data, device: { platform: p.id, deviceName: p.label }, userAgent: p.ua ?? null },
    });
    if (pstate) {
      try {
        pstate = await setPortalUserAgent(node.id, p.ua);
      } catch (err) {
        error = String(err);
      }
    }
  }

  async function proceedCert() {
    error = null;
    try {
      pstate = await trustPortalCert(node.id);
    } catch (e) {
      error = String(e);
    }
    policy.kick();
  }

  function toggleMute() {
    patchData({ muted: !muted });
  }

  async function copyShot() {
    if (!(await ensureOpen())) return;
    try {
      await copyPortalScreenshot(node.id);
      showToast("info", $t("canvas.portal.copied", { default: "Screenshot copied" }) as string, 2500);
    } catch (e) {
      error = String(e);
    }
  }

  async function inspect() {
    if (!(await ensureOpen())) return;
    try {
      await inspectPortal(node.id);
    } catch (e) {
      error = String(e);
    }
  }

  async function pickScope(e: Event) {
    const next = (e.currentTarget as HTMLSelectElement).value as StorageScope;
    if (next === scope) return;
    patchData({ storageScope: next });
    // The store is chosen at creation: reload the browser in the new scope.
    if (pstate) {
      await closePortal(node.id).catch(() => {});
      pstate = null;
      live = false;
    }
    policy.kick();
  }
</script>

{#if designEv}
  <PortalDesignSheet
    event={designEv}
    {workspaceId}
    onclose={() => {
      designEv = null;
      policy.kick();
    }}
    onsent={(name) =>
      showToast("info", $t("canvas.portal.design.sent", { default: "Sent to {{name}}", name }) as string, 4000)}
  />
{/if}
<div class="portal" class:driving={!!drive} bind:this={hostEl}>
  {#if kind !== "web"}
    <div class="soon">
      <Icon d="M7 3h10v18H7z M11 18h2" size={28} />
      <div class="soon-title">
        {kind === "ios"
          ? $t("canvas.portal.iosTitle", { default: "iOS Simulator" })
          : $t("canvas.portal.androidTitle", { default: "Android emulator" })}
      </div>
      <div class="soon-sub">{$t("canvas.portal.comingSoon", { default: "Coming soon" })}</div>
    </div>
  {:else}
    {#if !chromeHidden}
    <div class="bar" data-canvas-chrome>
      <button
        type="button"
        class="nav"
        title={$t("canvas.portal.back", { default: "Back" })}
        aria-label={$t("canvas.portal.back", { default: "Back" })}
        disabled={!pstate}
        onclick={() => history("back")}
      >
        <Icon d="M15 6l-6 6 6 6" size={14} />
      </button>
      <button
        type="button"
        class="nav"
        title={$t("canvas.portal.forward", { default: "Forward" })}
        aria-label={$t("canvas.portal.forward", { default: "Forward" })}
        disabled={!pstate}
        onclick={() => history("forward")}
      >
        <Icon d="M9 6l6 6-6 6" size={14} />
      </button>
      <button
        type="button"
        class="nav"
        class:spin={pstate?.loading}
        title={$t("canvas.portal.reload", { default: "Reload" })}
        aria-label={$t("canvas.portal.reload", { default: "Reload" })}
        disabled={!url}
        onclick={() => history("reload")}
      >
        <Icon d="M20 11a8 8 0 1 0-2.3 5.7 M20 5v6h-6" size={14} />
      </button>
      <input
        class="url"
        type="text"
        spellcheck="false"
        autocomplete="off"
        placeholder={$t("canvas.portal.urlPlaceholder", { default: "Enter a URL" })}
        aria-label={$t("canvas.portal.url", { default: "Address" })}
        bind:value={draft}
        onfocus={(e) => {
          editing = true;
          (e.currentTarget as HTMLInputElement).select();
        }}
        onblur={() => (editing = false)}
        onkeydown={onUrlKey}
      />
      <button
        type="button"
        class="nav"
        class:on={designing}
        title={$t("canvas.portal.design.toggle", { default: "Design mode: ask an agent about part of the page" })}
        aria-label={$t("canvas.portal.design.toggle", { default: "Design mode: ask an agent about part of the page" })}
        aria-pressed={designing}
        disabled={!url}
        onclick={toggleDesign}
      >
        <Icon d="M18 3l3 3-9 9-3-3z M9 12l-4 4c-1 1-1 3-2 5 2-1 4-1 5-2l4-4" size={14} />
      </button>
      {#if drive}
        <span class="drive" aria-live="polite" title={drive.method}>
          <span class="pulse"></span>
          {driver
            ? $t("canvas.portal.drivingBy", { default: "{{name}} is driving", name: driver })
            : $t("canvas.portal.driving", { default: "Agent is driving" })}
        </span>
      {/if}
      {#if sizeShown}
        <span class="size" aria-hidden="true">{sizeLabel}</span>
      {/if}
      <select
        class="pick"
        title={$t("canvas.portal.device.label", { default: "Device size" })}
        aria-label={$t("canvas.portal.device.label", { default: "Device size" })}
        value={device}
        onchange={pickDevice}
      >
        <option value="responsive">{$t("canvas.portal.device.responsive", { default: "Responsive" })}</option>
        {#each DEVICE_PRESETS as p (p.id)}
          <option value={p.id}>{$t(p.key, { default: p.label })} {p.width}×{p.height}</option>
        {/each}
      </select>
      <select
        class="pick"
        title={$t("canvas.portal.storage.label", { default: "Storage" })}
        aria-label={$t("canvas.portal.storage.label", { default: "Storage" })}
        value={scope}
        onchange={pickScope}
      >
        <option value="isolated">{$t("canvas.portal.storage.isolated", { default: "Isolated" })}</option>
        <option value="workspace">{$t("canvas.portal.storage.workspace", { default: "Workspace" })}</option>
        <option value="global">{$t("canvas.portal.storage.global", { default: "Global" })}</option>
      </select>
      <button
        type="button"
        class="nav"
        class:on={muted}
        title={muted ? $t("canvas.portal.unmute", { default: "Unmute" }) : $t("canvas.portal.mute", { default: "Mute" })}
        aria-label={muted ? $t("canvas.portal.unmute", { default: "Unmute" }) : $t("canvas.portal.mute", { default: "Mute" })}
        aria-pressed={muted}
        onclick={toggleMute}
      >
        <Icon d={muted ? "M4 9h4l5-4v14l-5-4H4z M17 9l4 6 M21 9l-4 6" : "M4 9h4l5-4v14l-5-4H4z M17 8a5 5 0 0 1 0 8"} size={14} />
      </button>
      <button
        type="button"
        class="nav"
        title={$t("canvas.portal.copyScreenshot", { default: "Copy screenshot" })}
        aria-label={$t("canvas.portal.copyScreenshot", { default: "Copy screenshot" })}
        disabled={!url}
        onclick={copyShot}
      >
        <Icon d="M4 7h3l2-2h6l2 2h3v12H4z M12 10a3 3 0 1 0 0 6 3 3 0 0 0 0-6" size={14} />
      </button>
      <button
        type="button"
        class="nav"
        title={$t("canvas.portal.inspector", { default: "Web Inspector" })}
        aria-label={$t("canvas.portal.inspector", { default: "Web Inspector" })}
        disabled={!url}
        onclick={inspect}
      >
        <Icon d="M8 6l-6 6 6 6 M16 6l6 6-6 6" size={14} />
      </button>
    </div>
    {/if}
    <div class="page" class:live bind:this={pageEl}>
      {#if !live}
        {#if snapshot}
          <img class="snap" src={snapshot} alt={pstate?.title || shownUrl} draggable="false" />
        {:else if url}
          <div class="empty">
            {pstate?.loading
              ? $t("canvas.portal.loading", { default: "Loading…" })
              : $t("canvas.portal.sleeping", { default: "Zoom in to load the page" })}
          </div>
        {:else}
          <div class="empty">{$t("canvas.portal.empty", { default: "Type an address above to open a page" })}</div>
        {/if}
      {/if}
      {#if certHost}
        <div class="cert" role="alert">
          <div class="cert-title">{$t("canvas.portal.cert.title", { default: "This connection is not private" })}</div>
          <div class="cert-sub">
            {$t("canvas.portal.cert.body", {
              default: "{{host}} uses a certificate this computer does not trust (self-signed or expired).",
              host: certHost,
            })}
          </div>
          <button type="button" class="cert-go" onclick={proceedCert}>
            {$t("canvas.portal.cert.proceed", { default: "Proceed in this portal" })}
          </button>
        </div>
      {/if}
      {#if error}
        <div class="error" title={error}>{error}</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .portal {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    color: var(--text);
  }
  .portal.driving {
    box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent) 70%, transparent);
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
  .nav.on {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--accent);
  }
  .nav:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .nav.spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .url {
    flex: 1;
    min-width: 0;
    height: 22px;
    box-sizing: border-box;
    padding: 0 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--input-bg, var(--surface));
    color: var(--text);
    font-size: 12px;
  }
  .url:focus {
    outline: none;
    border-color: var(--accent);
  }
  .pick {
    flex: none;
    max-width: 96px;
    height: 22px;
    font-size: 11px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--input-bg, var(--surface));
    color: var(--text-muted);
  }
  .page {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    background: #fff;
  }
  .page.live {
    background: transparent;
  }
  .snap {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: top left;
    display: block;
    user-select: none;
    pointer-events: none;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 16px;
    text-align: center;
    font-size: 13px;
    color: var(--text-muted);
    background: var(--surface);
  }
  .error {
    position: absolute;
    left: 8px;
    right: 8px;
    bottom: 8px;
    padding: 6px 8px;
    border-radius: 6px;
    font-size: 11px;
    background: color-mix(in srgb, var(--danger, #d33) 16%, var(--surface));
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cert {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 16px;
    text-align: center;
    background: var(--surface);
    color: var(--text);
  }
  .cert-title {
    font-weight: 600;
    font-size: 14px;
  }
  .cert-sub {
    font-size: 12px;
    color: var(--text-muted);
    max-width: 360px;
  }
  .cert-go {
    all: unset;
    cursor: pointer;
    padding: 5px 12px;
    border-radius: 6px;
    font-size: 12px;
    border: 1px solid var(--border);
    color: var(--text);
  }
  .cert-go:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .size {
    flex: none;
    padding: 2px 6px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    pointer-events: none;
  }
  .drive {
    flex: none;
    max-width: 140px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    background: var(--accent);
    color: var(--on-accent, #fff);
    pointer-events: none;
  }
  .pulse {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    animation: pulse 1s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse,
    .nav.spin :global(svg) {
      animation: none;
    }
  }
  .soon {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    color: var(--text-muted);
  }
  .soon-title {
    font-weight: 600;
    color: var(--text);
  }
  .soon-sub {
    font-size: 12px;
  }
</style>
