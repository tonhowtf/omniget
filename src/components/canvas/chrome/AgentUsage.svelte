<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { goto } from "$app/navigation";
  import { agentActions } from "$lib/native-agent/actions";

  type Win = { id: string; label: string; used: number; resets_at: number | null; level: "ok" | "warn" | "critical" };
  type Account = {
    id: string;
    cli: string;
    name: string;
    plan: string | null;
    status: string;
    message: string | null;
    session: Win | null;
    weekly: Win | null;
    others: Win[];
  };
  type View = { accounts: Account[] };

  const tr = (key: string, fallback: string, vars?: Record<string, unknown>) =>
    $t(key, { default: fallback, ...(vars ?? {}) }) as string;

  let accounts = $state<Account[]>([]);
  let open = $state(false);
  let now = $state(Date.now());
  let host = $state<HTMLDivElement | null>(null);

  onMount(() => {
    let disposed = false;
    const offs: Array<() => void> = [];
    invoke<{ view: View | null }>("usage_tray_view")
      .then((p) => {
        if (!disposed) accounts = p.view?.accounts ?? [];
      })
      .catch(() => {});
    void listen<View>("usage-tray://view", (e) => (accounts = e.payload?.accounts ?? []))
      .then((un) => (disposed ? un() : offs.push(un)))
      .catch(() => {});
    const tick = setInterval(() => (now = Date.now()), 30_000);
    const close = (e: PointerEvent) => {
      if (open && host && !host.contains(e.target as Node)) open = false;
    };
    window.addEventListener("pointerdown", close, true);
    return () => {
      disposed = true;
      offs.forEach((f) => f());
      clearInterval(tick);
      window.removeEventListener("pointerdown", close, true);
    };
  });

  const COLORS: Record<string, string> = { claude: "#d97757", codex: "#10a37f", gemini: "#4f8df5" };
  const color = (a: Account) => COLORS[a.cli] ?? "var(--accent)";
  const used = (w: Win | null) => Math.min(1, Math.max(0, w?.used ?? 0));
  const limited = (a: Account) =>
    a.status === "rate_limited" || [a.session, a.weekly, ...a.others].some((w) => w && (w.level === "critical" || w.used >= 1));
  const cliName = (a: Account) => (a.cli === "codex" ? "Codex" : a.cli === "gemini" ? "Gemini" : "Claude Code");

  function until(ms: number | null): string {
    if (!ms) return "";
    const d = ms - now;
    if (d <= 60_000) return tr("usage_tray.panel.now", "now");
    const min = Math.round(d / 60_000);
    if (min < 60) return tr("usage_tray.panel.in_min", "in {{count}} min", { count: min });
    const h = Math.floor(min / 60);
    if (h < 48) return tr("usage_tray.panel.in_hours", "in {{hours}}h{{minutes}}", { hours: h, minutes: String(min % 60).padStart(2, "0") });
    return tr("usage_tray.panel.in_days", "in {{count}} days", { count: Math.round(h / 24) });
  }

  function arc(r: number, frac: number): string {
    const c = 2 * Math.PI * r;
    return `${c * frac} ${c}`;
  }

  const SEGMENTS = 30;
  const windows = (a: Account) =>
    [
      a.session ? { ...a.session, label: tr("usage_tray.panel.session", "Session") } : null,
      a.weekly ? { ...a.weekly, label: tr("usage_tray.panel.weekly", "Weekly") } : null,
      ...a.others,
    ].filter((w): w is Win => !!w);
</script>

{#snippet rings(a: Account, size: number, stroke: number)}
  {@const r1 = size / 2 - stroke / 2}
  {@const r2 = r1 - stroke - 1.5}
  <svg width={size} height={size} viewBox="0 0 {size} {size}" aria-hidden="true" style:--c={color(a)}>
    <circle cx={size / 2} cy={size / 2} r={r1} class="track" stroke-width={stroke} />
    <circle cx={size / 2} cy={size / 2} r={r1} class="fill" stroke-width={stroke} stroke-dasharray={arc(r1, used(a.session))} transform="rotate(-90 {size / 2} {size / 2})" />
    {#if r2 > stroke}
      <circle cx={size / 2} cy={size / 2} r={r2} class="track" stroke-width={stroke} />
      <circle cx={size / 2} cy={size / 2} r={r2} class="fill" stroke-width={stroke} stroke-dasharray={arc(r2, used(a.weekly))} transform="rotate(-90 {size / 2} {size / 2})" />
    {/if}
  </svg>
{/snippet}

<div class="usage" bind:this={host} data-canvas-chrome>
  {#if open}
    <div class="pop" role="region" aria-label={tr("canvas.usage.title", "Agent usage")}>
      <header>
        <span>{tr("canvas.usage.title", "Agent usage")}</span>
        <button type="button" class="icon" title={tr("usage_tray.panel.refresh", "Refresh")} aria-label={tr("usage_tray.panel.refresh", "Refresh")} onclick={() => void invoke("usage_tray_refresh").catch(() => {})}>
          <svg viewBox="0 0 16 16" width="14" height="14"><path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5v3h-3" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
      </header>
      <div class="body">
        {#each accounts as a (a.id)}
          <section>
            <div class="who">
              {@render rings(a, 56, 7)}
              <div class="names">
                <strong>{cliName(a)}</strong>
                <span class="muted">{a.plan ?? a.name}</span>
                {#if limited(a)}<span class="warn">⚠ {tr("canvas.usage.limit", "Usage limit reached")}</span>{/if}
              </div>
            </div>
            <div class="plan-title">{tr("canvas.usage.planLimits", "Plan limits")}</div>
            {#each windows(a) as w (w.id + w.label)}
              {@const on = Math.round(used(w) * SEGMENTS)}
              <div class="win">
                <div class="row"><span>{w.label}</span><span>{Math.round(used(w) * 100)}%</span></div>
                <div class="segs" style:--c={color(a)}>
                  {#each Array(SEGMENTS) as _, i (i)}<i class:on={i < on}></i>{/each}
                </div>
                {#if w.resets_at}<div class="muted small">{tr("usage_tray.panel.resets_at", "resets {{when}}", { when: until(w.resets_at) })}</div>{/if}
              </div>
            {/each}
          </section>
        {:else}
          <div class="empty">
            <strong>{tr("canvas.usage.emptyTitle", "See how much of each plan your agents have used")}</strong>
            <p class="muted">{tr("canvas.usage.emptyBody", "Pick your Claude Code or Codex accounts and their limits show up here.")}</p>
            <button
              type="button"
              class="choose"
              onclick={() => {
                open = false;
                void goto("/settings?tab=general&section=usage-tray");
              }}>{tr("canvas.usage.choose", "Choose accounts…")}</button
            >
          </div>
        {/each}
      </div>
    </div>
  {/if}
  <button type="button" class="pill" use:agentActions={["click"]} class:on={open} title={tr("canvas.usage.title", "Agent usage")} aria-label={tr("canvas.usage.title", "Agent usage")} aria-expanded={open} onclick={() => (open = !open)}>
    {#each accounts.slice(0, 3) as a (a.id)}
      <span class="mini">
        {@render rings(a, 20, 3)}
        {#if limited(a)}<span class="alert"></span>{/if}
      </span>
    {:else}
      <svg width="20" height="20" viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="8" class="track" stroke-width="3" /></svg>
    {/each}
  </button>
</div>

<style>
  .usage {
    position: relative;
  }
  .pill {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 30px;
    padding: 0 7px;
    border-radius: 999px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.06),
      0 4px 14px rgba(0, 0, 0, 0.1);
    cursor: pointer;
  }
  .pill:hover,
  .pill.on {
    background: color-mix(in srgb, var(--popup-bg, var(--surface)) 88%, var(--text) 12%);
  }
  .mini {
    position: relative;
    display: inline-flex;
  }
  .alert {
    position: absolute;
    right: -1px;
    bottom: -1px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--warning, #f59e35);
    box-shadow: 0 0 0 1.5px var(--popup-bg, var(--surface));
  }
  .track {
    fill: none;
    stroke: color-mix(in srgb, var(--c, var(--text)) 22%, transparent);
  }
  .fill {
    fill: none;
    stroke: var(--c);
    stroke-linecap: round;
  }
  .pop {
    position: absolute;
    right: -40px;
    bottom: calc(100% + 10px);
    width: 300px;
    max-height: min(520px, 70vh);
    display: flex;
    flex-direction: column;
    border-radius: 14px;
    background: var(--popup-bg, var(--surface));
    backdrop-filter: blur(20px) saturate(1.4);
    -webkit-backdrop-filter: blur(20px) saturate(1.4);
    border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    box-shadow: 0 18px 48px rgba(0, 0, 0, 0.22);
    color: var(--text);
    z-index: 20;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px 10px;
    font-weight: 600;
    font-size: 14px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .icon {
    all: unset;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    cursor: pointer;
    color: var(--text-muted);
  }
  .icon:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .body {
    overflow-y: auto;
    padding: 10px 14px 14px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .names {
    display: flex;
    flex-direction: column;
    gap: 1px;
    font-size: 13px;
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .small {
    font-size: 11px;
    margin-top: 3px;
  }
  .warn {
    font-size: 12px;
    color: var(--warning, #f59e35);
  }
  .plan-title {
    margin: 12px 0 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
  }
  .win {
    margin-bottom: 10px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    font-size: 12.5px;
    margin-bottom: 4px;
  }
  .segs {
    display: flex;
    gap: 2px;
    height: 14px;
  }
  .segs i {
    flex: 1;
    border-radius: 2px;
    background: color-mix(in srgb, var(--c) 16%, transparent);
  }
  .segs i.on {
    background: var(--c);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 4px 0;
    font-size: 13px;
  }
  .empty p {
    margin: 0;
  }
  .choose {
    all: unset;
    margin-top: 4px;
    padding: 5px 12px;
    border-radius: 7px;
    font-size: 12.5px;
    background: var(--accent);
    color: var(--on-accent, #fff);
    cursor: pointer;
  }
</style>
