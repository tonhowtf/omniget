<script lang="ts">
  /**
   * The limits strip: the whole content of the `limits-strip` window.
   *
   * The window is built, sized and parked in Rust (`limits_strip::commands`);
   * it is exactly as big as what is drawn here, because a transparent window
   * still swallows clicks. Collapsed it is the pill; a click on a ring asks
   * Rust to grow the window and the card appears next to the pill.
   *
   * At rest this page owns no timer: it redraws when `limits://state`
   * arrives. The only clock is a 30 s one for "resets in…", alive while a
   * card is open.
   *
   * The snapshot's `look` says how to draw: everything is zoomed by its
   * scale, and Rust already sized the window for that scale and for the
   * percentage band under the rings (`limits_strip::placement`).
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { t } from "$lib/i18n";

  import LimitRing from "$components/limits-strip/LimitRing.svelte";
  import { bot } from "$components/limits-strip/bot";
  import { paceOf, paceSide } from "$components/limits-strip/pace";
  import {
    arcs,
    bodyOf,
    DEFAULT_LOOK,
    groups,
    paceTick,
    tone,
    toolOf,
    worst,
    type LimitWindow,
    type LocalModel,
    type Ring,
    type Snapshot,
  } from "$components/limits-strip/limits";

  let snap = $state<Snapshot>({ open: true, edge: "top", look: DEFAULT_LOOK, rings: [] });
  let openId = $state<string | null>(null);
  let initialLoading = $state(true);
  let loadFailed = $state(false);
  let openingApp = $state(false);

  async function openConnections() {
    if (openingApp) return;
    openingApp = true;
    try { await invoke("usage_tray_open_app"); }
    catch { loadFailed = true; }
    finally { openingApp = false; }
  }
  /** From Rust, in real window pixels: divided by the scale inside the zoomed stage. */
  let offset = $state(0);
  let now = $state(Date.now());
  let pulsing = $state<string | null>(null);
  let solid = $state(false);
  /** The glass follows the app theme, or the system when the app sets none. */
  let theme = $state<"dark" | "light">("dark");
  /** The window is hidden or minimised: every CSS loop holds still. */
  let asleep = $state(false);

  let vertical = $derived(snap.edge === "left" || snap.edge === "right");
  let look = $derived(snap.look ?? DEFAULT_LOOK);
  let scale = $derived(Number.isFinite(look.scale) && look.scale > 0 ? look.scale : 1);
  let openRing = $derived(snap.rings.find((r) => r.id === openId) ?? null);
  let stacks = $derived(groups(snap.rings));
  /** The one ring that most needs a look: the fullest at 80 % or more. */
  let critical = $derived.by(() => {
    let best: { id: string; used: number } | null = null;
    for (const r of snap.rings) {
      const u = worst(r);
      if (u != null && u >= 0.8 && (!best || u > best.used)) best = { id: r.id, used: u };
    }
    return best?.id ?? null;
  });
  /** Other logins of the open ring's tool, to hop between without closing. */
  let siblings = $derived(
    openRing ? snap.rings.filter((r) => toolOf(r.id) === toolOf(openRing.id)) : [],
  );

  function percent(used: number): number {
    return Math.round(used * 100);
  }

  function span(ms: number): string {
    const minutes = Math.max(1, Math.round(ms / 60000));
    const d = Math.floor(minutes / 1440);
    const h = Math.floor((minutes % 1440) / 60);
    const m = minutes % 60;
    const parts: string[] = [];
    if (d) parts.push(`${d} ${$t("llm.limits.strip.d")}`);
    if (h) parts.push(`${h} ${$t("llm.limits.strip.h")}`);
    if (m && !d) parts.push(`${m} ${$t("llm.limits.strip.min")}`);
    return parts.join(" ");
  }

  function resets(at: number | null): string | null {
    if (at == null) return null;
    return at <= now
      ? ($t("llm.limits.strip.resets_now") as string)
      : ($t("llm.limits.strip.resets_in", { time: span(at - now) }) as string);
  }

  /** "20% ahead of pace · runs out in 1 h 20 min", or null with nothing to say yet. */
  function paceLine(w: LimitWindow): { text: string; warn: boolean } | null {
    const p = paceOf(w, now);
    if (!p) return null;
    const side = paceSide(p.delta);
    const delta = percent(Math.abs(p.delta));
    if (side === "on") return { text: $t("llm.limits.strip.pace.on") as string, warn: false };
    if (side === "under") return { text: $t("llm.limits.strip.pace.under", { delta }) as string, warn: false };
    const out = p.runsOutAt != null && p.runsOutAt > now;
    return out
      ? { text: $t("llm.limits.strip.pace.ahead_out", { delta, time: span(p.runsOutAt! - now) }) as string, warn: true }
      : { text: $t("llm.limits.strip.pace.ahead", { delta }) as string, warn: false };
  }

  function amount(n: number, unit: string | null): string {
    const digits = unit === "usd" ? 2 : 0;
    return n.toLocaleString(undefined, { maximumFractionDigits: digits, minimumFractionDigits: digits });
  }

  function absolute(w: LimitWindow): string | null {
    if (w.used_abs == null) return null;
    const unit = w.unit ?? "";
    return w.limit_abs != null
      ? ($t("llm.limits.strip.of", { used: amount(w.used_abs, w.unit), limit: amount(w.limit_abs, w.unit), unit }) as string)
      : ($t("llm.limits.strip.count", { used: amount(w.used_abs, w.unit), unit }) as string);
  }

  function gib(bytes: number | null): string | null {
    return bytes ? `${(bytes / 1073741824).toFixed(1)} GB` : null;
  }

  function modelLine(m: LocalModel): string {
    const left =
      m.expires_at && m.expires_at > now
        ? ($t("llm.limits.strip.unloads_in", { time: span(m.expires_at - now) }) as string)
        : null;
    return [m.params, m.quant, gib(m.vram_bytes ?? m.size_bytes), left].filter(Boolean).join(" · ");
  }

  /** "Max pessoal · me@x.com", or null for a tool with a single login. */
  function who(ring: Ring): string | null {
    const parts = [ring.account?.label, ring.account?.email].filter(Boolean);
    return parts.length ? parts.join(" · ") : null;
  }

  /** Two letters in the ring: the account's initials when a tool has several. */
  function glyph(ring: Ring): string {
    const several = snap.rings.filter((r) => r.label === ring.label).length > 1;
    const name = several ? (ring.account?.label ?? ring.account?.email ?? "") : "";
    const words = name.replace(/\(.*?\)/g, "").split(/[^\p{L}\p{N}]+/u).filter(Boolean);
    if (!words.length) return ring.label.slice(0, 2);
    return (words.length > 1 ? words[0][0] + words[1][0] : words[0].slice(0, 2)).toUpperCase();
  }

  function tooltip(ring: Ring): string {
    const used = worst(ring);
    const name = who(ring) ? `${ring.label} (${who(ring)})` : ring.label;
    let head = used != null ? `${name} · ${$t("llm.limits.strip.used", { percent: percent(used) })}` : name;
    if (tone(used) === "red") head = `${head} · ${$t("llm.limits.strip.near_limit")}`;
    return ring.activity === "idle" ? head : `${head} · ${$t(`llm.limits.strip.state.${ring.activity}`)}`;
  }

  function readAgo(ring: Ring): string | null {
    if (ring.read_at == null) return null;
    const ms = now - ring.read_at;
    return ms < 60000
      ? ($t("llm.limits.strip.just_now") as string)
      : ($t("llm.limits.strip.read_ago", { time: span(ms) }) as string);
  }

  /** The bot mirrors the account: hops while its agent works, holds still when it cannot read. */
  function botOf(ring: Ring) {
    const stuck = ring.status === "needs_auth" || ring.status === "error" || ring.status === "absent";
    return {
      type: bodyOf(ring, snap.rings),
      state: (ring.activity === "working" || ring.activity === "waiting" ? "working" : "default") as
        | "working"
        | "default",
      size: 48,
      paused: stuck || asleep,
      label: tooltip(ring),
    };
  }

  async function expand(id: string | null) {
    try {
      const res = (await invoke("limits_strip_set_expanded", { expanded: id !== null })) as { offset: number };
      offset = res.offset;
    } catch {
      offset = 0;
    }
    now = Date.now();
    openId = id;
  }

  function toggle(id: string) {
    void expand(openId === id ? null : id);
  }

  /** Another login of the same tool: the window is already open, only the card changes. */
  function switchTo(id: string) {
    now = Date.now();
    openId = id;
  }

  function refresh(id: string) {
    void invoke("limits_strip_refresh", { providerId: id }).catch(() => {});
  }

  function hide() {
    void invoke("limits_strip_close").catch(() => {});
  }

  function grab(event: MouseEvent) {
    if (event.button !== 0 || openId) return;
    void getCurrentWindow().startDragging().catch(() => {});
  }

  // The 30 s clock lives only while a card shows a countdown.
  $effect(() => {
    if (!openId) return;
    const timer = setInterval(() => (now = Date.now()), 30000);
    return () => clearInterval(timer);
  });

  // The ring whose card is open was switched off: fold the window back.
  $effect(() => {
    if (openId && !snap.rings.some((r) => r.id === openId)) void expand(null);
  });

  onMount(() => {
    solid = new URLSearchParams(location.search).has("solid");
    const root = document.documentElement;
    const prev = [root.style.background, document.body.style.background];
    if (!solid) {
      root.style.background = "transparent";
      document.body.style.background = "transparent";
    }

    const unlisteners: UnlistenFn[] = [];
    let disposed = false;
    const arm = (p: Promise<UnlistenFn>) =>
      void p.then((un) => (disposed ? un() : unlisteners.push(un))).catch(() => {});

    // The ticks follow the snapshot's clock: no timer at rest.
    arm(
      listen<Snapshot>("limits://state", (e) => {
        now = Date.now();
        snap = e.payload;
        initialLoading = false;
        loadFailed = false;
      }),
    );
    arm(
      listen<{ provider: string }>("limits://chime", (e) => {
        pulsing = e.payload.provider;
        setTimeout(() => (pulsing = null), 2400);
      }),
    );
    void invoke<Snapshot>("limits_strip_state")
      .then((s) => {
        now = Date.now();
        snap = s;
      })
      .catch(() => { loadFailed = true; })
      .finally(() => { initialLoading = false; });

    const dark = matchMedia("(prefers-color-scheme: dark)");
    const readTheme = () => {
      const set = root.getAttribute("data-theme");
      theme = set === "light" || set === "dark" ? set : dark.matches ? "dark" : "light";
    };
    readTheme();
    dark.addEventListener("change", readTheme);
    const themeWatch = new MutationObserver(readTheme);
    themeWatch.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    const onVisibility = () => (asleep = document.visibilityState === "hidden");
    onVisibility();
    document.addEventListener("visibilitychange", onVisibility);

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && openId) void expand(null);
    };
    const onBlur = () => {
      if (openId) void expand(null);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
      dark.removeEventListener("change", readTheme);
      themeWatch.disconnect();
      document.removeEventListener("visibilitychange", onVisibility);
      [root.style.background, document.body.style.background] = prev;
    };
  });
</script>

<main
  class="stage edge-{snap.edge} theme-{theme}"
  class:vertical
  class:solid
  class:asleep
  class:labels={look.show_percent}
  class:contrast={look.contrast}
  style:zoom={scale === 1 ? null : scale}
  oncontextmenu={(e) => e.preventDefault()}
>
  <div
    class="pill"
    class:vertical
    role="toolbar"
    aria-label={$t("llm.limits.strip.aria") as string}
    style={vertical ? `margin-top:${offset / scale}px` : `margin-left:${offset / scale}px`}
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <span class="grip" onmousedown={grab} aria-hidden="true"><i></i><i></i><i></i></span>
    {#each stacks as stack (stack.rings[0].id)}
      <div class="stack" class:many={stack.rings.length > 1}>
        {#each stack.rings as ring (ring.id)}
          {@const used = worst(ring)}
          {@const arc = arcs(ring)}
          <button
            type="button"
            class="ring tone-{tone(used)} act-{ring.activity}"
            class:open={openId === ring.id}
            class:pulse={pulsing === ring.id}
            class:critical={critical === ring.id}
            class:dim={ring.status !== "ok" && !ring.reading}
            title={tooltip(ring)}
            aria-label={tooltip(ring)}
            aria-expanded={openId === ring.id}
            onclick={() => toggle(ring.id)}
          >
            <LimitRing
              outer={arc.outer}
              inner={arc.inner}
              glyph={ring.local ? String(ring.reading?.local_models.length ?? 0) : glyph(ring)}
              activity={ring.activity}
              {theme}
              pace={look.show_pace ? paceTick(ring, now) : null}
            />
            {#if look.show_percent}
              <span class="pct" class:none={used == null}>{used != null ? `${percent(used)}%` : "–"}</span>
            {/if}
            {#if ring.activity === "waiting" || ring.activity === "done"}
              <span class="dot dot-{ring.activity}"></span>
            {:else if ring.status === "needs_auth" || ring.status === "error" || ring.status === "rate_limited"}
              <span class="dot dot-problem"></span>
            {/if}
          </button>
        {/each}
      </div>
    {:else}
      {#if initialLoading}
        <span class="empty" role="status" aria-label={$t("common.loading")}>…</span>
      {:else}
        <button class="empty empty-action" type="button" disabled={openingApp} onclick={openConnections}
          aria-label={$t(loadFailed ? "llm.limits.strip.open_after_error" : "llm.limits.strip.connect_hint")}
          title={$t(loadFailed ? "llm.limits.strip.open_after_error" : "llm.limits.strip.connect_hint")}>+</button>
      {/if}
    {/each}
  </div>

  {#if openRing}
    {@const ring = openRing}
    {@const used = worst(ring)}
    <section class="card" aria-label={ring.label}>
      <header class="card-head">
        <span class="avatar" class:alert={tone(used) === "red"} use:bot={botOf(ring)}></span>
        <div class="head-main">
          <div class="head-row">
            <span class="card-title">{ring.label}</span>
            {#if ring.beta}<span class="tag" title={$t("llm.limits.beta_hint") as string}>{$t("llm.limits.beta")}</span>{/if}
            {#if ring.local}<span class="tag">{$t("llm.limits.local")}</span>{/if}
          </div>
          {#if who(ring)}<p class="card-who" title={who(ring)}>{who(ring)}</p>{/if}
          <div class="head-row chips">
            {#if ring.activity !== "idle"}
              <span class="chip chip-{ring.activity}">{$t(`llm.limits.strip.state.${ring.activity}`)}</span>
            {/if}
            {#if used != null}
              <span class="chip tone-{tone(used)} chip-used">
                {tone(used) === "red" ? $t("llm.limits.strip.near_limit") : $t("llm.limits.strip.used", { percent: percent(used) })}
              </span>
            {/if}
          </div>
        </div>
      </header>

      {#if siblings.length > 1}
        <nav class="accounts" aria-label={$t("llm.limits.strip.accounts") as string}>
          {#each siblings as other (other.id)}
            {@const oa = arcs(other)}
            <button
              type="button"
              class="acct"
              class:on={other.id === ring.id}
              title={tooltip(other)}
              aria-pressed={other.id === ring.id}
              onclick={() => switchTo(other.id)}
            >
              <LimitRing small outer={oa.outer} inner={oa.inner} glyph="" activity={other.activity} {theme} />
              <span class="acct-name">{other.account?.label ?? other.account?.email ?? other.label}</span>
            </button>
          {/each}
        </nav>
      {/if}

      <div class="card-body">
        {#if ring.status !== "ok"}
          <p class="status">
            {$t(`llm.limits.strip.status.${ring.status}`)}
            {#if ring.message}<span class="status-detail">{ring.message}</span>{/if}
          </p>
        {/if}

        {#if ring.reading}
          {#each ring.reading.windows as w (w.id)}
            {@const abs = absolute(w)}
            {@const when = resets(w.resets_at)}
            {@const pace = look.show_pace ? paceLine(w) : null}
            {@const at = look.show_pace ? paceOf(w, now)?.expected : null}
            <div class="win tone-{tone(w.used)}">
              <div class="win-top">
                <span class="win-label">{w.group ? `${w.group} · ${w.label}` : w.label}</span>
                {#if w.used != null}<span class="win-pct">{percent(w.used)}%</span>{/if}
              </div>
              <div class="bar">
                <span style={`width:${Math.min(100, percent(w.used ?? 0))}%`}></span>
                {#if at != null}<i class="bar-tick" style:left="{at * 100}%"></i>{/if}
              </div>
              {#if abs || when}
                <div class="win-sub">{[abs, when].filter(Boolean).join(" · ")}</div>
              {/if}
              {#if pace}
                <div class="win-sub pace" class:warn={pace.warn}>{pace.text}</div>
              {/if}
            </div>
          {/each}

          {#if ring.reading.local_models.length}
            <h3 class="sub">{$t("llm.limits.strip.models")}</h3>
            {#each ring.reading.local_models as m (m.name)}
              <div class="model">
                <span class="model-name">{m.name}</span>
                <span class="win-sub">{modelLine(m)}</span>
              </div>
            {/each}
          {/if}

          {#if ring.reading.plan}
            <p class="meta">{$t("llm.limits.strip.plan")}: {ring.reading.plan}</p>
          {/if}
          {#if ring.reading.account}<p class="meta">{ring.reading.account}</p>{/if}
          {#if ring.reading.note}<p class="meta">{ring.reading.note}</p>{/if}
        {/if}
      </div>

      <footer class="card-foot">
        <span class="meta">{readAgo(ring) ?? ""}</span>
        <span class="foot-actions">
          {#if ring.id !== "omniget"}
            <button type="button" class="link" onclick={() => refresh(ring.id)}>{$t("llm.limits.strip.refresh")}</button>
          {/if}
          <button type="button" class="link" onclick={hide}>{$t("llm.limits.strip.hide")}</button>
        </span>
      </footer>
    </section>
  {/if}
</main>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    overflow: hidden;
  }

  /* Loop's palette: hood green, carrot orange, scarf red. */
  .stage {
    --calm: #4aa84b;
    --amber: #f59e35;
    --red: #e03a00;
    --work: #4aa84b;
    --ink: rgba(255, 255, 255, 0.93);
    --ink-dim: rgba(255, 255, 255, 0.6);
    --glass: rgba(24, 24, 26, 0.94);
    --edge: rgba(255, 255, 255, 0.09);
    --track: rgba(255, 255, 255, 0.14);
    --hover: rgba(255, 255, 255, 0.1);
    --stack: rgba(255, 255, 255, 0.055);
    --chip: rgba(255, 255, 255, 0.1);
    --shadow: 0 8px 24px rgba(0, 0, 0, 0.28);
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-start;
    color: var(--ink);
    font: 12px/1.35 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
    user-select: none;
    -webkit-user-select: none;
    cursor: default;
  }
  .stage.theme-dark {
    --calm: #5cc45d;
  }
  .stage.theme-light {
    --calm: #2f8f3a;
    --amber: #d9780f;
    --red: #d23500;
    --ink: rgba(17, 17, 17, 0.92);
    --ink-dim: rgba(17, 17, 17, 0.58);
    --glass: rgba(253, 246, 236, 0.95);
    --edge: rgba(17, 17, 17, 0.1);
    --track: rgba(17, 17, 17, 0.1);
    --hover: rgba(17, 17, 17, 0.07);
    --stack: rgba(29, 90, 60, 0.07);
    --chip: rgba(17, 17, 17, 0.07);
    --shadow: 0 8px 24px rgba(60, 40, 10, 0.14);
  }
  /* High contrast: opaque plates, stronger tracks and text, deeper tones on
     light, brighter ones on dark. */
  .stage.contrast {
    --calm: #6fdc70;
    --amber: #ffb35c;
    --red: #ff6a3d;
    --work: #6fdc70;
    --ink: #fff;
    --ink-dim: rgba(255, 255, 255, 0.84);
    --glass: #111113;
    --edge: rgba(255, 255, 255, 0.34);
    --track: rgba(255, 255, 255, 0.3);
    --stack: rgba(255, 255, 255, 0.1);
    --chip: rgba(255, 255, 255, 0.16);
  }
  .stage.contrast.theme-light {
    --calm: #1b6f27;
    --amber: #a85400;
    --red: #b02a00;
    --work: #1b6f27;
    --ink: #000;
    --ink-dim: rgba(0, 0, 0, 0.78);
    --glass: #fffaf3;
    --edge: rgba(0, 0, 0, 0.4);
    --track: rgba(0, 0, 0, 0.24);
    --stack: rgba(29, 90, 60, 0.12);
    --chip: rgba(0, 0, 0, 0.1);
  }
  /* No alpha channel to trust: a square backdrop instead of see-through corners. */
  .stage.solid {
    background: #1c1c1e;
  }
  .stage.solid.theme-light {
    background: #fdf6ec;
  }
  .stage.edge-bottom {
    flex-direction: column-reverse;
  }
  .stage.edge-left {
    flex-direction: row;
  }
  .stage.edge-right {
    flex-direction: row-reverse;
  }
  /* Hidden window: no CSS loop keeps ticking. */
  .stage.asleep,
  .stage.asleep * {
    animation-play-state: paused !important;
  }

  .pill {
    flex: none;
    box-sizing: border-box;
    height: 44px;
    padding: 0 8px;
    display: flex;
    align-items: center;
    border-radius: 14px;
    background: var(--glass);
    box-shadow: inset 0 0 0 1px var(--edge);
  }
  .pill.vertical {
    height: auto;
    width: 44px;
    padding: 8px 0;
    flex-direction: column;
  }
  /* The percentage band: 12 px more across a horizontal strip, 12 px more
     per ring along a vertical one (`placement::LABEL`). */
  .stage.labels .pill:not(.vertical) {
    height: 56px;
  }

  .grip {
    flex: none;
    width: 14px;
    height: 28px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    cursor: grab;
  }
  .pill.vertical .grip {
    width: 28px;
    height: 14px;
    flex-direction: row;
  }
  .grip i {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--ink-dim);
    opacity: 0.6;
  }

  /* Several logins of one tool sit on one shared plate, no extra width. */
  .stack {
    position: relative;
    display: flex;
    flex-direction: inherit;
  }
  .stack.many::before {
    content: "";
    position: absolute;
    inset: 2px 1px;
    border-radius: 12px;
    background: var(--stack);
    box-shadow: inset 0 0 0 1px var(--edge);
  }
  .pill.vertical .stack {
    flex-direction: column;
  }
  .pill.vertical .stack.many::before {
    inset: 1px 2px;
  }

  .ring {
    flex: none;
    position: relative;
    width: 36px;
    height: 36px;
    padding: 2px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: inherit;
    cursor: pointer;
    transition:
      background 160ms ease,
      transform 160ms ease;
  }
  .ring:hover,
  .ring.open {
    background: var(--hover);
  }
  .ring:active {
    transform: scale(0.94);
  }
  .ring:focus-visible {
    outline: 2px solid var(--calm);
    outline-offset: -2px;
  }
  .ring.dim {
    opacity: 0.5;
  }
  .stage.labels .ring {
    height: 48px;
  }
  .pct {
    display: block;
    height: 12px;
    font-size: 9.5px;
    font-weight: 700;
    line-height: 12px;
    text-align: center;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.01em;
    color: var(--tone);
    white-space: nowrap;
  }
  .pct.none {
    color: var(--ink-dim);
  }
  .stage.contrast .pct {
    font-weight: 800;
  }
  .tone-calm {
    --tone: var(--calm);
  }
  .tone-amber {
    --tone: var(--amber);
  }
  .tone-red {
    --tone: var(--red);
  }

  /* A soft halo behind the ring: steady for the fullest account, breathing
     while an agent works. Opacity and transform only, so it stays on the
     compositor. */
  .ring::before {
    content: "";
    position: absolute;
    inset: 3px;
    border-radius: 50%;
    box-shadow:
      0 0 7px 1px color-mix(in srgb, var(--halo, var(--tone)) 75%, transparent),
      inset 0 0 5px 0 color-mix(in srgb, var(--halo, var(--tone)) 45%, transparent);
    opacity: 0;
    transform: scale(0.9);
    transition:
      opacity 260ms ease,
      transform 260ms ease;
    pointer-events: none;
  }
  .stage.labels .ring::before {
    inset: 3px 3px 15px;
  }
  .ring.critical::before {
    opacity: 0.8;
    transform: scale(1);
    animation: alarm 1.1s ease-in-out 3;
  }
  .ring.act-working::before {
    --halo: var(--work);
    animation: breathe 2.4s ease-in-out infinite;
  }
  .ring.act-done :global(.lr) {
    animation: pop 520ms cubic-bezier(0.3, 1.6, 0.5, 1) 1;
  }
  .ring.pulse::before {
    animation: alarm 0.8s ease-in-out 3;
  }
  @keyframes breathe {
    0%,
    100% {
      opacity: 0.25;
      transform: scale(0.86);
    }
    50% {
      opacity: 0.9;
      transform: scale(1.08);
    }
  }
  @keyframes alarm {
    0%,
    100% {
      opacity: 0.85;
      transform: scale(1);
    }
    50% {
      opacity: 1;
      transform: scale(1.2);
    }
  }
  @keyframes pop {
    40% {
      transform: scale(1.14);
    }
  }

  .dot {
    position: absolute;
    right: 2px;
    bottom: 2px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    box-shadow: 0 0 0 2px var(--glass);
    animation: dot-in 320ms cubic-bezier(0.3, 1.6, 0.5, 1) 1;
  }
  .stage.labels .dot {
    bottom: 14px;
  }
  .dot-waiting {
    background: var(--amber);
  }
  .dot-done {
    background: var(--calm);
  }
  .dot-problem {
    background: var(--ink-dim);
  }
  @keyframes dot-in {
    from {
      transform: scale(0);
    }
  }

  .empty {
    width: 36px;
    text-align: center;
    color: var(--ink-dim);
  }

  .empty-action { min-height: 32px; border: none; border-radius: 8px; background: transparent; font: inherit; font-size: 21px; cursor: pointer; }
  .empty-action:hover { background: var(--fill-1); color: var(--text); }
  .empty-action:focus-visible { outline: var(--focus-ring); }

  .card {
    flex: 1;
    align-self: stretch;
    min-width: 0;
    min-height: 0;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    border-radius: 16px;
    background: var(--glass);
    box-shadow: inset 0 0 0 1px var(--edge);
    user-select: text;
    -webkit-user-select: text;
    animation: card-in 220ms cubic-bezier(0.2, 0.9, 0.3, 1.1) 1;
  }
  .stage.edge-top .card {
    transform-origin: top left;
  }
  .stage.edge-bottom .card {
    transform-origin: bottom left;
  }
  @keyframes card-in {
    from {
      opacity: 0;
      transform: scale(0.96) translateY(-4px);
    }
  }

  .card-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 12px 8px;
  }
  .avatar {
    flex: none;
    position: relative;
    width: 48px;
    height: 48px;
    display: grid;
    place-items: center;
  }
  .avatar::before {
    content: "";
    position: absolute;
    inset: -6px;
    border-radius: 50%;
    background: radial-gradient(circle, color-mix(in srgb, var(--red) 45%, transparent) 25%, transparent 70%);
    opacity: 0;
    transition: opacity 300ms ease;
    pointer-events: none;
  }
  .avatar.alert::before {
    opacity: 1;
    animation: alarm 1.1s ease-in-out 3;
  }
  .head-main {
    min-width: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .head-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .card-title {
    font-size: 14px;
    font-weight: 700;
  }
  .tag {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    padding: 1px 5px;
    border-radius: 999px;
    background: var(--chip);
    color: var(--ink-dim);
  }
  .card-who {
    margin: 0;
    font-size: 11px;
    color: var(--ink-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chips {
    margin-top: 2px;
  }
  .chip {
    font-size: 10.5px;
    font-weight: 650;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--chip);
    color: var(--ink-dim);
    white-space: nowrap;
  }
  .chip-working {
    color: var(--work);
    background: color-mix(in srgb, var(--work) 16%, transparent);
  }
  .chip-waiting {
    color: var(--amber);
    background: color-mix(in srgb, var(--amber) 16%, transparent);
  }
  .chip-done {
    color: var(--calm);
    background: color-mix(in srgb, var(--calm) 16%, transparent);
  }
  .chip-used {
    color: var(--tone);
    background: color-mix(in srgb, var(--tone) 14%, transparent);
    transition:
      color 260ms ease,
      background 260ms ease;
  }

  .accounts {
    display: flex;
    gap: 4px;
    padding: 0 10px 8px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .accounts::-webkit-scrollbar {
    display: none;
  }
  .acct {
    flex: none;
    display: flex;
    align-items: center;
    gap: 5px;
    max-width: 120px;
    padding: 3px 8px 3px 4px;
    border: none;
    border-radius: 999px;
    background: var(--chip);
    color: var(--ink-dim);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    transition:
      background 160ms ease,
      color 160ms ease;
  }
  .acct:hover {
    color: var(--ink);
  }
  .acct.on {
    color: var(--ink);
    background: color-mix(in srgb, var(--calm) 20%, transparent);
  }
  .acct-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .status {
    margin: 0;
    color: var(--amber);
  }
  .status-detail {
    display: block;
    color: var(--ink-dim);
  }
  .win-top {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .win-label {
    font-weight: 600;
  }
  .win-pct {
    font-variant-numeric: tabular-nums;
    color: var(--tone);
    font-weight: 700;
    transition: color 260ms ease;
  }
  .bar {
    position: relative;
    height: 5px;
    margin: 3px 0;
    border-radius: 3px;
    background: var(--track);
    overflow: hidden;
  }
  /* Where the bar would be if the window were spent evenly. */
  .bar-tick {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: var(--ink);
    box-shadow: 0 0 0 1px var(--glass);
  }
  .bar span {
    display: block;
    height: 100%;
    border-radius: 3px;
    background: var(--tone);
    transform-origin: left;
    transition:
      width 700ms cubic-bezier(0.22, 1.2, 0.36, 1),
      background 260ms ease;
    animation: grow 700ms cubic-bezier(0.22, 1, 0.36, 1) 1;
  }
  @keyframes grow {
    from {
      transform: scaleX(0);
    }
  }
  .win-sub,
  .meta {
    margin: 0;
    font-size: 11px;
    color: var(--ink-dim);
  }
  .pace.warn {
    color: var(--amber);
    font-weight: 600;
  }
  .stage.contrast .win-label,
  .stage.contrast .card-title {
    font-weight: 800;
  }
  .sub {
    margin: 2px 0 0;
    font-size: 11px;
    font-weight: 700;
    color: var(--ink-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .model {
    display: flex;
    flex-direction: column;
  }
  .model-name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 12px 8px;
  }
  .foot-actions {
    display: flex;
    gap: 10px;
  }
  .link {
    border: none;
    padding: 0;
    background: none;
    font: inherit;
    font-size: 11px;
    color: var(--calm);
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }

  @media (prefers-reduced-motion: reduce) {
    .ring,
    .ring::before,
    .bar span,
    .chip-used,
    .acct {
      transition: none;
    }
    .ring::before,
    .ring :global(.lr),
    .avatar::before,
    .dot,
    .card,
    .bar span {
      animation: none !important;
    }
  }
</style>
