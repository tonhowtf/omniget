<script lang="ts">
  /**
   * The floating Omni — the whole content of the `pet` window.
   *
   * Budget (docs/llm-world-execution-plan.md §1.2): at rest this page owns zero
   * timers and zero `requestAnimationFrame`. The single `drawImage` that put the
   * pet on screen already happened; nothing is scheduled until an intent asks
   * for a clip, and the clock stops itself again when the clip settles (see
   * `$lib/omni/sprite-player`). The frame rate is the clip's, capped at 12 fps,
   * driven by a chained `setTimeout` so it does not follow the display.
   *
   * The window itself (transparency, always-on-top, corner, click-through) is
   * built in Rust by `commands::pet`; this file only draws and offers the menu.
   */
  import { onMount } from "svelte";
  import { registerSemantic } from "$lib/native-agent/semantic";
  import { invoke } from "@tauri-apps/api/core";
  import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
  import { t } from "$lib/i18n";
  import { parseAtlas, type AtlasData } from "$lib/gfx/atlas";
  import {
    advance,
    createPlayer,
    currentFrame,
    fitScale,
    placeFrame,
    playClip,
    type PlayerState,
  } from "$lib/omni/sprite-player";
  import {
    CORNERS,
    MOOD_TINT,
    bubbleMs,
    cornerKey,
    isCorner,
    sanitizeIntent,
    type Corner,
    type Mood,
  } from "$lib/omni/pet-state";
  import {
    acceptAsk,
    cancelRequest,
    headAsk,
    resolveAsk,
    terminalRequestId,
    type PendingAsk,
  } from "$lib/omni/tool-ask-queue";
  import { balloonOf, type CompanionBalloon } from "$lib/omni/companion-balloon";

  /** Logical size of the window; must match `PET_W`/`PET_H` in Rust. */
  const BOX = 200;
  /** Pixels of air under the feet, so the pet does not sit on the window edge. */
  const GROUND_INSET = 28;
  /** Default skin. The profile's skin id would come from `profile_get`. */
  const SKIN = "omni-default";
  /**
   * Events the pet listens on. `llm://tool-ask` and `llm://turn` are emitted by
   * `commands::llm` to every window; `omni://tool-resolved` is the pet's own
   * note that a question is over (see the handoff: the backend is asked to emit
   * the same event from `llm_tool_answer`, and the pet works either way).
   */
  const EVENT_TOOL_ASK = "llm://tool-ask";
  const EVENT_TURN = "llm://turn";
  const EVENT_TOOL_RESOLVED = "omni://tool-resolved";
  /** Omni as the canvas companion: a summary of what an agent just did. */
  const EVENT_COMPANION = "canvas://companion";
  const EVENT_COMPANION_DISMISS = "canvas://companion-dismiss";

  let canvas: HTMLCanvasElement | undefined = $state();
  let ctx: CanvasRenderingContext2D | null = null;
  let sheet: HTMLImageElement | null = null;
  let atlas: AtlasData | null = null;
  let player: PlayerState | null = null;
  let scale = 3;

  let ready = $state(false);
  let failed = $state<string | null>(null);
  let line = $state<string | null>(null);
  let mood = $state<Mood>("Neutral");
  let menuOpen = $state(false);
  let cornerMenuOpen = $state(false);
  let corner = $state<Corner>("bottom-right");

  /**
   * Tool permissions waiting for an answer, oldest first. Every entry came from
   * an `llm://tool-ask` event; nothing here is polled, and an empty queue costs
   * exactly nothing.
   */
  let asks = $state<PendingAsk[]>([]);
  /** The question on screen: the oldest one. */
  let ask = $derived(headAsk(asks));
  /** Whether the two buttons are showing. A click on the pet opens them. */
  let askOpen = $state(false);
  /** Last value pushed to Rust, so the bridge is crossed only on a change. */
  let askPendingSent = false;

  /** The one animation timer. Null means the pet is at rest and costs nothing. */
  let frameTimer: ReturnType<typeof setTimeout> | null = null;
  let bubbleTimer: ReturnType<typeof setTimeout> | null = null;

  /** The companion's latest summary, until it expires, is acted on or dismissed. */
  let companion = $state<CompanionBalloon | null>(null);
  let companionTimer: ReturnType<typeof setTimeout> | null = null;

  function showCompanion(b: CompanionBalloon | null) {
    if (companionTimer !== null) {
      clearTimeout(companionTimer);
      companionTimer = null;
    }
    companion = b;
    if (!b) return;
    mood = b.mood;
    play(b.clip);
    companionTimer = setTimeout(() => {
      companion = null;
      companionTimer = null;
    }, b.ms);
  }

  /** Sends chip `index` to the agent (paste + Enter), or just dismisses. */
  async function companionAct(index: number | null) {
    const b = companion;
    if (!b) return;
    showCompanion(null);
    try {
      if (index === null) await invoke("canvas_companion_dismiss", { id: b.id });
      else await invoke("canvas_companion_act", { id: b.id, index, text: null });
    } catch (e) {
      say(String(e).slice(0, 80));
    }
  }

  function now(): number {
    return performance.now();
  }

  function draw() {
    if (!ctx || !sheet || !atlas || !player) return;
    const frame = currentFrame(player, atlas);
    const { dx, dy, dw, dh } = placeFrame(frame, BOX, BOX, scale, GROUND_INSET);
    ctx.clearRect(0, 0, BOX, BOX);
    if (player.flip) {
      ctx.save();
      ctx.translate(dx + dw, dy);
      ctx.scale(-1, 1);
      ctx.drawImage(sheet, frame.x, frame.y, frame.w, frame.h, 0, 0, dw, dh);
      ctx.restore();
    } else {
      ctx.drawImage(sheet, frame.x, frame.y, frame.w, frame.h, dx, dy, dw, dh);
    }
  }

  /**
   * Arm the next frame, or arm nothing. `player.nextDueMs === null` is the whole
   * idle budget: no timer exists while the pet is resting.
   */
  function schedule() {
    if (frameTimer !== null) {
      clearTimeout(frameTimer);
      frameTimer = null;
    }
    if (!player || player.nextDueMs === null) return;
    const delay = Math.max(0, player.nextDueMs - now());
    frameTimer = setTimeout(tick, delay);
  }

  function tick() {
    frameTimer = null;
    if (!player || !atlas) return;
    if (advance(player, atlas, now())) draw();
    schedule();
  }

  function play(clip: string) {
    if (!player || !atlas) return;
    try {
      playClip(player, atlas, clip, now());
    } catch {
      // An intent naming a clip this skin does not have leaves the pet alone.
      return;
    }
    draw();
    schedule();
  }

  function say(text: string | null) {
    if (bubbleTimer !== null) {
      clearTimeout(bubbleTimer);
      bubbleTimer = null;
    }
    line = text;
    if (!text) return;
    bubbleTimer = setTimeout(() => {
      line = null;
      bubbleTimer = null;
    }, bubbleMs(text));
  }

  async function loadArt(): Promise<void> {
    const manifest = await fetch(`/omni/skins/${SKIN}/pet.json`).then((r) => r.json());
    const atlasUrl: string = manifest.atlas ?? "/world/omni/atlas.json";
    const raw = await fetch(atlasUrl).then((r) => r.json());
    const parsed = parseAtlas(raw);
    const base = atlasUrl.slice(0, atlasUrl.lastIndexOf("/") + 1);
    const img = new Image();
    img.decoding = "async";
    img.src = base + parsed.pages[0];
    await img.decode();
    atlas = parsed;
    sheet = img;
    player = createPlayer(parsed);
    const first = currentFrame(player, parsed);
    scale = fitScale(first.w, first.h, BOX, BOX - GROUND_INSET);
  }

  function setupCanvas() {
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(BOX * dpr);
    canvas.height = Math.round(BOX * dpr);
    const c = canvas.getContext("2d", { alpha: true });
    if (!c) throw new Error("ERR_PET_NO_CANVAS2D");
    c.setTransform(dpr, 0, 0, dpr, 0, 0);
    // Pixel art: any smoothing turns the 48x64 sprite to mush at 3x.
    c.imageSmoothingEnabled = false;
    ctx = c;
  }

  /**
   * Keep the window's click-through in step with the queue. A pet the cursor
   * falls through cannot answer a question, so Rust suspends the preference
   * while something is pending and restores it after — one call per edge, never
   * a poll.
   */
  async function syncAskPending() {
    const pending = asks.length > 0;
    if (pending === askPendingSent) return;
    askPendingSent = pending;
    try {
      await invoke("pet_set_ask_pending", { pending });
    } catch (e) {
      // An older backend without the command leaves click-through as it was;
      // the bubble still works when the pet is not click-through.
      console.warn("[pet] set_ask_pending", e);
    }
  }

  /** Replace the queue and run the side effects a change implies. */
  function setAsks(next: PendingAsk[]) {
    if (next === asks) return;
    const hadHead = asks[0]?.toolCallId ?? null;
    asks = next;
    const head = asks[0]?.toolCallId ?? null;
    if (head !== hadHead) askOpen = false;
    if (head && !hadHead) {
      // First question: the mascot asks for attention. Same intent the pure
      // `rules` map `BusEvent::ToolAsk` to (Wave / Curious).
      mood = "Curious";
      play("wave");
    }
    void syncAskPending();
  }

  /**
   * Answer the oldest question. The broker keys on `tool_call_id`, so the pet
   * and the `/llm` route can both answer and the second one is a harmless
   * no-op: whatever `llm_tool_answer` says, the question leaves the pet.
   */
  async function answerAsk(target: PendingAsk, allow: boolean, always = false) {
    askOpen = false;
    setAsks(resolveAsk(asks, target.toolCallId));
    try {
      await invoke("llm_tool_answer", {
        requestId: target.requestId,
        toolCallId: target.toolCallId,
        allow,
        always,
      });
    } catch (e) {
      // Already answered elsewhere, or the turn is gone: either way the
      // question is over and the queue is right to have dropped it.
      console.warn("[pet] tool_answer", e);
    }
    // Tell the other surfaces (the rail avatar, a second pet) to drop it too.
    try {
      await emit(EVENT_TOOL_RESOLVED, {
        request_id: target.requestId,
        tool_call_id: target.toolCallId,
        allow,
      });
    } catch {
      // Cosmetic: the `/llm` route reconciles from the backend anyway.
    }
  }

  async function chooseCorner(next: Corner) {
    corner = next;
    menuOpen = false;
    cornerMenuOpen = false;
    try {
      await invoke("pet_set_corner", { corner: next });
    } catch (e) {
      console.warn("[pet] set_corner", e);
    }
  }

  async function hidePet() {
    menuOpen = false;
    try {
      await invoke("pet_close");
    } catch (e) {
      console.warn("[pet] close", e);
    }
  }

  /**
   * Bring the main window forward and ask the shell to go to /llm. The pet
   * window never navigates itself: it is 200 pixels wide.
   */
  async function openHub() {
    menuOpen = false;
    try {
      const main = await WebviewWindow.getByLabel("main");
      await main?.show();
      await main?.setFocus();
      await emit("omni://navigate", { path: "/llm" });
    } catch (e) {
      console.warn("[pet] open hub", e);
    }
  }

  function onContextMenu(event: MouseEvent) {
    event.preventDefault();
    menuOpen = !menuOpen;
    cornerMenuOpen = false;
  }

  onMount(() => {
    const stopSemantics = registerSemantic("pet", () => ({
      readiness: failed ? "error" : ready ? "ready" : "loading",
      skin: SKIN, mood, corner, pendingPermission: asks.length > 0,
    }));
    const root = document.documentElement;
    const prevHtml = root.style.background;
    const prevBody = document.body.style.background;
    root.style.background = "transparent";
    document.body.style.background = "transparent";

    const unlisteners: UnlistenFn[] = [];
    let disposed = false;

    /** Register a listener, or drop it if the window died while it was armed. */
    function keep(stop: UnlistenFn) {
      if (disposed) stop();
      else unlisteners.push(stop);
    }

    void (async () => {
      // The listeners go up before the art: a tool permission is a question the
      // user has to answer, and a skin that failed to load must not swallow it
      // (the bubble is HTML; `play` is a no-op while there is no sprite).
      keep(
        await listen("omni://intent", (event) => {
          const intent = sanitizeIntent(event.payload);
          if (!intent) return;
          mood = intent.mood;
          play(intent.clip);
          say(intent.line);
        }),
      );
      // A tool wants permission. Only the name travels to the bubble.
      keep(
        await listen(EVENT_TOOL_ASK, (event) => {
          setAsks(acceptAsk(asks, event.payload));
        }),
      );
      // Omni as the canvas companion: an agent finished, is blocked or failed.
      keep(
        await listen(EVENT_COMPANION, (event) => {
          const b = balloonOf(event.payload, (k, v) => $t(k, v) as string);
          if (b) showCompanion(b);
        }),
      );
      keep(
        await listen(EVENT_COMPANION_DISMISS, (event) => {
          const p = event.payload as { id?: unknown } | null;
          if (companion && p?.id === companion.id) showCompanion(null);
        }),
      );
      // Opened in the middle of a turn: catch up on what is already waiting.
      try {
        const waiting = (await invoke("llm_tool_asks_pending")) as unknown[];
        for (const ask of waiting ?? []) setAsks(acceptAsk(asks, ask));
      } catch {
        // Older backend or no LLM stack yet: the live event is enough.
      }
      // The turn finished, errored or was cancelled: its questions are moot.
      keep(
        await listen(EVENT_TURN, (event) => {
          const done = terminalRequestId(event.payload);
          if (done) setAsks(cancelRequest(asks, done));
        }),
      );
      // Answered somewhere else (the /llm route, another window).
      keep(
        await listen(EVENT_TOOL_RESOLVED, (event) => {
          const p = event.payload as { tool_call_id?: unknown } | null;
          if (typeof p?.tool_call_id === "string") {
            setAsks(resolveAsk(asks, p.tool_call_id));
          }
        }),
      );
      try {
        const state = (await invoke("pet_capabilities")) as { corner?: unknown };
        if (isCorner(state.corner)) corner = state.corner;
      } catch {
        // The corner only decorates the menu's check mark.
      }
      try {
        setupCanvas();
        await loadArt();
        if (disposed) return;
        draw();
        ready = true;
      } catch (e) {
        failed = e instanceof Error ? e.message : String(e);
      }
    })();

    return () => {
      stopSemantics();
      disposed = true;
      if (frameTimer !== null) clearTimeout(frameTimer);
      if (bubbleTimer !== null) clearTimeout(bubbleTimer);
      if (companionTimer !== null) clearTimeout(companionTimer);
      companionTimer = null;
      frameTimer = null;
      bubbleTimer = null;
      for (const stop of unlisteners) stop();
      unlisteners.length = 0;
      // Leave the window the way the user's preference wants it.
      if (askPendingSent) {
        askPendingSent = false;
        void invoke("pet_set_ask_pending", { pending: false }).catch(() => {});
      }
      root.style.background = prevHtml;
      document.body.style.background = prevBody;
    };
  });
</script>

<svelte:window onblur={() => (menuOpen = false)} />

<!-- The drag region is the whole window: the pet is grabbed anywhere. -->
<div
  class="pet"
  class:compact={!!companion && !ask}
  data-tauri-drag-region
  role="presentation"
  oncontextmenu={onContextMenu}
  ondblclick={openHub}
>
  {#if ask}
    <!--
      A pending tool permission owns the bubble: it outranks a line the agent
      said, it never expires on a timer, and it shows the tool's NAME and
      nothing else — no arguments, no paths, no secret.
    -->
    <div
      class="bubble ask"
      role="alertdialog"
      aria-labelledby="pet-ask-title"
      style="--mood: {MOOD_TINT.Curious}"
    >
      <span id="pet-ask-title" class="ask-title">{$t("pet.ask.title")}</span>
      <span class="ask-tool">{ask.tool}</span>
      {#if asks.length > 1}
        <span class="ask-queue">+{asks.length - 1}</span>
      {/if}
      {#if askOpen}
        <div class="ask-actions">
          <button type="button" class="allow" onclick={() => void answerAsk(ask!, true)}>
            {$t("pet.ask.allow")}
          </button>
          <button type="button" class="deny" onclick={() => void answerAsk(ask!, true, true)}>
            {$t("pet.ask.always")}
          </button>
          <button type="button" class="deny" onclick={() => void answerAsk(ask!, false)}>
            {$t("pet.ask.deny")}
          </button>
        </div>
      {:else}
        <!-- The pet itself opens the actions; this is the keyboard way in. -->
        <button type="button" class="ask-hint" onclick={() => (askOpen = true)}>
          {$t("pet.ask.hint")}
        </button>
      {/if}
    </div>
  {:else if companion}
    <!-- The companion's summary: short text and up to three steps to send. -->
    <div class="bubble companion" role="status" style="--mood: {MOOD_TINT[companion.mood]}">
      <div class="companion-head">
        <span class="companion-name">{companion.name}</span>{#if companion.via}<span class="companion-via">{companion.via}</span>{/if}
        <button
          type="button"
          class="companion-x"
          aria-label={$t("canvas.companion.dismiss", { default: "Dismiss" }) as string}
          onclick={() => void companionAct(null)}>✕</button
        >
      </div>
      <span class="companion-text">{companion.text}</span>
      {#each companion.chips as chip, i (i)}
        <button type="button" class="chip" onclick={() => void companionAct(i)}>{chip}</button>
      {/each}
    </div>
  {:else if line}
    <div class="bubble" style="--mood: {MOOD_TINT[mood]}">{line}</div>
  {/if}

  <canvas
    bind:this={canvas}
    class="sprite"
    data-agent-renderer="pet"
    style="width: {companion && !ask ? BOX / 2 : BOX}px; height: {companion && !ask ? BOX / 2 : BOX}px"
    aria-label={$t("pet.a11y.canvas") as string}
    onclick={() => (ask ? (askOpen = !askOpen) : play("wave"))}
  ></canvas>

  {#if failed}
    <p class="failed">{$t("pet.error.art")}</p>
  {:else if !ready}
    <p class="loading">{$t("pet.loading")}</p>
  {/if}

  {#if menuOpen}
    <div class="menu" role="menu">
      <button type="button" role="menuitem" onclick={() => { play("wave"); menuOpen = false; }}>
        {$t("pet.menu.wave")}
      </button>
      <button type="button" role="menuitem" onclick={openHub}>{$t("pet.menu.open_llm")}</button>
      <button
        type="button"
        role="menuitem"
        aria-expanded={cornerMenuOpen}
        onclick={() => (cornerMenuOpen = !cornerMenuOpen)}
      >
        {$t("pet.menu.corner")}
      </button>
      {#if cornerMenuOpen}
        <div class="submenu">
          {#each CORNERS as c (c)}
            <button
              type="button"
              role="menuitemradio"
              aria-checked={corner === c}
              class:selected={corner === c}
              onclick={() => chooseCorner(c)}
            >
              {$t(cornerKey(c))}
            </button>
          {/each}
        </div>
      {/if}
      <button type="button" role="menuitem" class="danger" onclick={hidePet}>
        {$t("pet.menu.hide")}
      </button>
    </div>
  {/if}
</div>

<style>
  :global(html),
  :global(body) {
    background: transparent !important;
    margin: 0;
    overflow: hidden;
  }

  .pet {
    position: relative;
    width: 200px;
    height: 200px;
    /* Nothing but the sprite and the menu is opaque: the rest is desktop. */
    background: transparent;
    user-select: none;
    -webkit-user-select: none;
    cursor: grab;
  }

  .sprite {
    position: absolute;
    inset: 0;
    display: block;
    image-rendering: pixelated;
  }

  .bubble {
    position: absolute;
    top: 4px;
    left: 8px;
    right: 8px;
    max-height: 64px;
    overflow: hidden;
    padding: 6px 10px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--mood, #8e8e93) 22%, rgba(28, 28, 30, 0.92));
    box-shadow:
      0 2px 10px rgba(0, 0, 0, 0.35),
      inset 0 0 0 1px color-mix(in srgb, var(--mood, #8e8e93) 55%, transparent);
    color: #fff;
    font-size: 12px;
    line-height: 1.3;
    text-align: center;
    pointer-events: none;
  }

  /* The ask bubble is the one piece of the pet that must catch clicks. */
  .bubble.ask {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    max-height: none;
    pointer-events: auto;
    cursor: default;
  }

  /*
   * Companion summary: the sprite shrinks to half size in the bottom-right
   * corner and the balloon takes the space above it, its tail pointing at
   * Omni, so the text never sits on top of the character.
   */
  .pet.compact .sprite {
    inset: auto;
    right: 0;
    bottom: 0;
  }

  .bubble.companion {
    display: flex;
    flex-direction: column;
    gap: 3px;
    top: 4px;
    left: 4px;
    right: 4px;
    max-height: 128px;
    overflow: visible;
    text-align: left;
    pointer-events: auto;
    cursor: default;
    font-size: 11px;
  }

  .bubble.companion::after {
    content: "";
    position: absolute;
    right: 44px;
    bottom: -6px;
    width: 10px;
    height: 10px;
    transform: rotate(45deg);
    background: inherit;
    box-shadow: inset -1px -1px 0 0 color-mix(in srgb, var(--mood, #8e8e93) 55%, transparent);
  }

  .companion-text {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
    line-height: 1.25;
  }

  .companion-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 4px;
  }

  .companion-name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .companion-x,
  .chip {
    appearance: none;
    border: 0;
    color: inherit;
    cursor: default;
    font: inherit;
  }

  .companion-x {
    background: transparent;
    padding: 0 2px;
    opacity: 0.7;
  }

  .chip {
    text-align: left;
    padding: 2px 6px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.14);
    font-size: 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip:hover {
    background: rgba(255, 255, 255, 0.24);
  }

  .ask-title {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    opacity: 0.75;
  }

  .ask-tool {
    font-family:
      ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 12px;
    font-weight: 600;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ask-queue {
    font-size: 10px;
    opacity: 0.7;
  }

  .ask-hint {
    appearance: none;
    border: 0;
    background: transparent;
    padding: 0;
    color: inherit;
    font-size: 10px;
    opacity: 0.6;
    cursor: default;
  }

  .ask-actions {
    display: flex;
    gap: 6px;
    margin-top: 2px;
  }

  .ask-actions button {
    appearance: none;
    border: 0;
    border-radius: 7px;
    padding: 3px 10px;
    font-size: 11px;
    font-weight: 600;
    color: #fff;
    cursor: default;
  }

  .ask-actions .allow {
    background: #30d158;
    color: #0b1f10;
  }

  .ask-actions .deny {
    background: rgba(255, 255, 255, 0.16);
  }

  .ask-actions button:hover {
    filter: brightness(1.12);
  }

  .loading,
  .failed {
    position: absolute;
    bottom: 4px;
    left: 0;
    right: 0;
    margin: 0;
    text-align: center;
    font-size: 11px;
    color: rgba(255, 255, 255, 0.72);
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
  }

  .menu {
    position: absolute;
    right: 6px;
    bottom: 6px;
    z-index: 2;
    display: flex;
    flex-direction: column;
    min-width: 132px;
    padding: 4px;
    border-radius: 10px;
    background: rgba(38, 38, 40, 0.96);
    box-shadow:
      0 8px 24px rgba(0, 0, 0, 0.45),
      inset 0 0 0 1px rgba(255, 255, 255, 0.08);
  }

  .submenu {
    display: flex;
    flex-direction: column;
    margin-left: 8px;
  }

  .menu button {
    appearance: none;
    border: 0;
    background: transparent;
    color: #fff;
    font-size: 12px;
    text-align: left;
    padding: 5px 8px;
    border-radius: 6px;
    cursor: default;
  }

  .menu button:hover {
    background: rgba(255, 255, 255, 0.12);
  }

  .menu button.selected::after {
    content: " ✓";
  }

  .menu button.danger {
    color: #ff6961;
  }
  .companion-via {
    margin-left: 6px;
    font-size: 10px;
    opacity: 0.7;
  }
</style>
