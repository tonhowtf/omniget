<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { paceSide } from "#components/limits-strip/pace.ts";

  type Win = {
    id: string;
    label: string;
    used: number;
    resets_at: number | null;
    runs_out_at: number | null;
    expected: number | null;
    level: "ok" | "warn" | "critical";
  };
  type Account = {
    id: string;
    cli: string;
    name: string;
    email: string | null;
    plan: string | null;
    status: "pending" | "ok" | "absent" | "needs_auth" | "rate_limited" | "error";
    message: string | null;
    session: Win | null;
    weekly: Win | null;
    others: Win[];
    read_at: number | null;
    spend_today: number | null;
  };
  type View = {
    accounts: Account[];
    headline: [number, string] | null;
    shows: string;
    warn: number;
    critical: number;
  };
  type CatalogItem = { id: string; cli: string; name: string; shown: boolean };
  type Panel = { view: View | null; catalog: CatalogItem[] };

  const R = 22;
  const CIRC = 2 * Math.PI * R;

  let view = $state<View | null>(null);
  let catalog = $state<CatalogItem[]>([]);
  let choosing = $state(false);
  let now = $state(Date.now());
  let theme = $state<"dark" | "light">("dark");
  let box: HTMLElement | undefined = $state();

  let empty = $derived(!view || view.accounts.length === 0);
  let showChooser = $derived(choosing || empty);

  function clock(ms: number): string {
    const d = new Date(ms);
    const sameDay = d.toDateString() === new Date(now).toDateString();
    const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    return sameDay ? hm : `${d.toLocaleDateString([], { weekday: "short" })} ${hm}`;
  }

  function until(ms: number): string {
    const mins = Math.round((ms - now) / 60_000);
    if (mins <= 0) return $t("usage_tray.panel.now");
    if (mins < 60) return $t("usage_tray.panel.in_min", { count: mins });
    if (mins < 48 * 60)
      return $t("usage_tray.panel.in_hours", {
        hours: Math.floor(mins / 60),
        minutes: String(mins % 60).padStart(2, "0"),
      });
    return $t("usage_tray.panel.in_days", { count: Math.round(mins / 1440) });
  }

  function money(v: number): string {
    return v.toLocaleString([], { style: "currency", currency: "USD", maximumFractionDigits: 2 });
  }

  function pct(w: Win): string {
    return `${Math.round(w.used * 100)}%`;
  }

  function paceText(w: Win): string | null {
    if (w.expected == null) return null;
    const side = paceSide(w.used - w.expected);
    const delta = Math.round(Math.abs(w.used - w.expected) * 100);
    return side === "on"
      ? $t("agents.limits.strip.pace.on")
      : $t(`agents.limits.strip.pace.${side}`, { delta });
  }

  function tick(at: number) {
    const a = Math.max(0, Math.min(1, at)) * 2 * Math.PI - Math.PI / 2;
    const [from, to] = [R - 4.5, R + 4.5];
    return { x1: 28 + from * Math.cos(a), y1: 28 + from * Math.sin(a), x2: 28 + to * Math.cos(a), y2: 28 + to * Math.sin(a) };
  }

  function statusLine(a: Account): string | null {
    if (a.status === "needs_auth") return $t("usage_tray.panel.needs_auth");
    if (a.status === "rate_limited") return $t("usage_tray.panel.rate_limited");
    if (a.status === "error") return a.message ?? $t("usage_tray.panel.error");
    if (a.status === "pending") return $t("usage_tray.panel.pending");
    return null;
  }

  async function load() {
    try {
      const p = await invoke<Panel>("usage_tray_view");
      view = p.view;
      catalog = p.catalog;
    } catch {
    }
  }

  async function toggle(id: string) {
    await invoke("usage_tray_toggle_account", { id }).catch(() => {});
    await load();
  }

  function refresh() {
    void invoke("usage_tray_refresh").catch(() => {});
  }

  function openApp() {
    void invoke("usage_tray_open_app").catch(() => {});
  }

  function openClaude(id: string | null, add: boolean) {
    void invoke("usage_tray_open_claude", { id, add }).catch(() => {});
    void invoke("usage_tray_close_panel").catch(() => {});
  }

  onMount(() => {
    const root = document.documentElement;
    root.style.background = "transparent";
    document.body.style.background = "transparent";

    const unlisteners: UnlistenFn[] = [];
    let disposed = false;
    void listen<View>("usage-tray://view", (e) => {
      view = e.payload;
    }).then((un) => (disposed ? un() : unlisteners.push(un)));
    void load();

    const dark = matchMedia("(prefers-color-scheme: dark)");
    const readTheme = () => {
      const set = root.getAttribute("data-theme");
      theme = set === "light" || set === "dark" ? set : dark.matches ? "dark" : "light";
    };
    readTheme();
    dark.addEventListener("change", readTheme);

    const tick = setInterval(() => (now = Date.now()), 30_000);
    const onFocus = () => {
      now = Date.now();
      void load();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void invoke("usage_tray_close_panel").catch(() => {});
    };
    window.addEventListener("focus", onFocus);
    window.addEventListener("keydown", onKey);

    const ro = new ResizeObserver(() => {
      if (!box) return;
      const height = Math.ceil(box.getBoundingClientRect().height) + 2;
      void invoke("usage_tray_resize", { height }).catch(() => {});
    });
    if (box) ro.observe(box);

    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
      clearInterval(tick);
      dark.removeEventListener("change", readTheme);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("keydown", onKey);
      ro.disconnect();
    };
  });
</script>

{#snippet ring(w: Win | null, label: string)}
  <div class="meter">
    <svg viewBox="0 0 56 56" class="ring {w?.level ?? 'none'}" aria-hidden="true">
      <circle cx="28" cy="28" r={R} class="track" />
      {#if w}
        <circle
          cx="28"
          cy="28"
          r={R}
          class="fill"
          stroke-dasharray={CIRC}
          stroke-dashoffset={CIRC * (1 - Math.min(1, Math.max(0, w.used)))}
          transform="rotate(-90 28 28)"
        />
        {#if w.expected != null}
          {@const k = tick(w.expected)}
          <line class="tick-under" x1={k.x1} y1={k.y1} x2={k.x2} y2={k.y2} />
          <line class="tick" x1={k.x1} y1={k.y1} x2={k.x2} y2={k.y2} />
        {/if}
      {/if}
      <text x="28" y="32" text-anchor="middle">{w ? pct(w) : "–"}</text>
    </svg>
    <div class="meter-text">
      <span class="meter-label">{label}</span>
      {#if w?.resets_at}
        <span class="dim">{$t("usage_tray.panel.resets_at", { when: `${clock(w.resets_at)} · ${until(w.resets_at)}` })}</span>
      {/if}
      {#if w?.runs_out_at}
        <span class="forecast {w.level}">{$t("usage_tray.panel.runs_out", { when: clock(w.runs_out_at) })}</span>
      {:else if w && w.resets_at}
        <span class="dim">{$t("usage_tray.panel.lasts")}</span>
      {/if}
      {#if w && paceText(w)}
        <span class="pace" class:ahead={w.runs_out_at != null && paceSide(w.used - (w.expected ?? 0)) === "ahead"}>{paceText(w)}</span>
      {/if}
    </div>
  </div>
{/snippet}

<main class="panel theme-{theme}" bind:this={box}>
  <header>
    <img src="/usage-tray-loop.png" alt="" class="loop" />
    <h1>{$t("usage_tray.panel.title")}</h1>
    <button class="icon-btn" onclick={refresh} title={$t("usage_tray.panel.refresh")} aria-label={$t("usage_tray.panel.refresh")}>
      <svg viewBox="0 0 16 16" width="14" height="14"><path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5v3h-3" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
    </button>
  </header>

  {#if view && view.accounts.length > 0}
    <section class="accounts">
      {#each view.accounts as a (a.id)}
        <article class="card">
          <div class="who">
            <span class="badge {a.cli}">{a.cli === "codex" ? "Codex" : "Claude"}</span>
            <span class="name">{a.name}</span>
            {#if a.plan}<span class="plan">{a.plan}</span>{/if}
          </div>
          {#if statusLine(a)}
            <p class="status {a.status}">{statusLine(a)}</p>
          {/if}
          <div class="meters">
            {@render ring(a.session, $t("usage_tray.panel.session"))}
            {@render ring(a.weekly, $t("usage_tray.panel.weekly"))}
          </div>
          {#if a.others.length}
            <ul class="others">
              {#each a.others as w (w.id)}
                <li title={paceText(w) ?? undefined}>
                  <span>{w.label}</span>
                  <span class="bar">
                    <span class="bar-fill {w.level}" style:width="{Math.min(100, w.used * 100)}%"></span>
                    {#if w.expected != null}<i class="bar-tick" style:left="{Math.min(1, w.expected) * 100}%"></i>{/if}
                  </span>
                  <span class="num">{pct(w)}</span>
                </li>
              {/each}
            </ul>
          {/if}
          <div class="foot">
            {#if a.spend_today != null}
              <span>{$t("usage_tray.panel.spent_today", { amount: money(a.spend_today) })}</span>
            {/if}
            {#if a.read_at}
              <span class="dim">{$t("usage_tray.panel.updated", { when: clock(a.read_at) })}</span>
            {/if}
            {#if a.cli === "claude"}
              <button class="link" onclick={() => openClaude(a.id, false)}>{$t("usage_tray.panel.open_claude")}</button>
            {/if}
          </div>
        </article>
      {/each}
    </section>
  {/if}

  {#if showChooser}
    <section class="chooser">
      {#if empty}
        <h2>{$t("usage_tray.panel.empty_title")}</h2>
        <p class="dim">{$t("usage_tray.panel.empty_body")}</p>
      {/if}
      {#if catalog.length === 0}
        <p class="dim">{$t("usage_tray.panel.no_logins")}</p>
      {/if}
      {#each catalog as c (c.id)}
        <label class="pick">
          <input type="checkbox" checked={c.shown} onchange={() => toggle(c.id)} />
          <span>{c.name}</span>
        </label>
      {/each}
    </section>
  {/if}

  <footer>
    <button onclick={openApp}>{$t("usage_tray.panel.open_app")}</button>
    <button onclick={() => openClaude(null, true)}>{$t("usage_tray.panel.add_claude_account")}</button>
    {#if !empty}
      <button class:on={choosing} onclick={() => (choosing = !choosing)}>{$t("usage_tray.panel.choose")}</button>
    {/if}
  </footer>
</main>

<style>
  .panel {
    --calm: #5cc45d;
    --amber: #f59e35;
    --red: #ff5a36;
    --ink: rgba(255, 255, 255, 0.93);
    --ink-dim: rgba(255, 255, 255, 0.58);
    --glass: rgba(28, 28, 30, 0.97);
    --card: rgba(255, 255, 255, 0.05);
    --edge: rgba(255, 255, 255, 0.1);
    --track: rgba(255, 255, 255, 0.13);
    --hover: rgba(255, 255, 255, 0.09);
    box-sizing: border-box;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-radius: 12px;
    background: var(--glass);
    box-shadow: inset 0 0 0 1px var(--edge);
    color: var(--ink);
    font: 12px/1.35 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
    user-select: none;
    -webkit-user-select: none;
    cursor: default;
  }
  .panel.theme-light {
    --calm: #2f8f3a;
    --amber: #d9780f;
    --red: #d23500;
    --ink: rgba(17, 17, 17, 0.92);
    --ink-dim: rgba(17, 17, 17, 0.56);
    --glass: rgba(253, 246, 236, 0.98);
    --card: rgba(29, 90, 60, 0.06);
    --edge: rgba(17, 17, 17, 0.1);
    --track: rgba(17, 17, 17, 0.1);
    --hover: rgba(17, 17, 17, 0.07);
  }

  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .loop {
    width: 26px;
    height: 26px;
  }
  h1 {
    flex: 1;
    margin: 0;
    font-size: 14px;
    font-weight: 650;
  }
  h2 {
    margin: 0 0 4px;
    font-size: 13px;
    font-weight: 600;
  }
  p {
    margin: 0;
  }
  .dim {
    color: var(--ink-dim);
  }

  button {
    font: inherit;
    color: inherit;
    background: var(--card);
    border: 0;
    border-radius: 7px;
    padding: 5px 9px;
    box-shadow: inset 0 0 0 1px var(--edge);
    cursor: default;
  }
  button:hover,
  button.on {
    background: var(--hover);
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
  }
  .link {
    margin-left: auto;
    padding: 2px 7px;
    color: var(--calm);
    background: transparent;
    box-shadow: none;
  }

  .accounts {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: 10px;
    background: var(--card);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .plan {
    margin-left: auto;
    color: var(--ink-dim);
    text-transform: capitalize;
  }
  .badge {
    flex: none;
    font-size: 10px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 99px;
    background: rgba(217, 119, 87, 0.2);
    color: #e08a67;
  }
  .badge.codex {
    background: rgba(120, 140, 255, 0.18);
    color: #8fa0ff;
  }
  .status {
    font-size: 11px;
    color: var(--ink-dim);
  }
  .status.needs_auth,
  .status.error {
    color: var(--red);
  }

  .meters {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .meter {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .ring {
    flex: none;
    width: 48px;
    height: 48px;
  }
  .ring circle {
    fill: none;
    stroke-width: 5;
  }
  .ring .track {
    stroke: var(--track);
  }
  .ring .fill {
    stroke: var(--calm);
    stroke-linecap: round;
    transition: stroke-dashoffset 0.4s ease;
  }
  .ring.warn .fill {
    stroke: var(--amber);
  }
  .ring.critical .fill {
    stroke: var(--red);
  }
  .ring line {
    stroke-linecap: round;
  }
  .ring .tick {
    stroke: var(--ink);
    stroke-width: 1.4;
  }
  .ring .tick-under {
    stroke: var(--glass);
    stroke-width: 3;
  }
  .ring text {
    fill: var(--ink);
    font-size: 12px;
    font-weight: 650;
  }
  .meter-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    font-size: 11px;
  }
  .meter-label {
    font-weight: 600;
    font-size: 12px;
  }
  .forecast {
    color: var(--amber);
  }
  .forecast.critical {
    color: var(--red);
  }
  .pace {
    color: var(--ink-dim);
  }
  .pace.ahead {
    color: var(--amber);
  }

  .others {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
  }
  .others li {
    display: grid;
    grid-template-columns: 1fr 70px 34px;
    align-items: center;
    gap: 6px;
  }
  .bar {
    position: relative;
    height: 5px;
    border-radius: 3px;
    background: var(--track);
    overflow: hidden;
  }
  .bar-fill {
    display: block;
    height: 100%;
    background: var(--calm);
  }
  .bar-fill.warn {
    background: var(--amber);
  }
  .bar-fill.critical {
    background: var(--red);
  }
  .bar-tick {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: var(--ink);
    box-shadow: 0 0 0 1px var(--glass);
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
  }

  .chooser {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    border-radius: 10px;
    background: var(--card);
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
  }
  .pick input {
    accent-color: var(--calm);
  }

  footer {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
