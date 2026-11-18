<script lang="ts">
  import { agentActions } from "$lib/native-agent/actions";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { locale, t } from "$lib/i18n";
  import { pickMeter, raiseTo, type Meter, type PolicyStatus } from "$lib/canvas/attention/budget";

  const WINDOWS = ["turn", "day", "week", "5h", "7d", "run"];

  let meter = $state<Meter | null>(null);
  let open = $state(false);
  let busy = $state(false);
  let failed = $state(false);
  let host = $state<HTMLDivElement | null>(null);
  let reload: () => Promise<void> = async () => {};

  let raise = $derived(meter && meter.tone === "paused" ? raiseTo(meter) : null);
  let windowText = $derived(meter && WINDOWS.includes(meter.window) ? ($t(`canvas.budget.w_${meter.window}`) as string) : "");

  function amount(value: number, metric: string, lang: string): string {
    try {
      if (metric === "usd") return new Intl.NumberFormat(lang, { style: "currency", currency: "USD" }).format(value);
      if (metric === "quota_pct") return new Intl.NumberFormat(lang, { style: "percent", maximumFractionDigits: 0 }).format(value / 100);
      return $t("canvas.budget.tokens", { count: new Intl.NumberFormat(lang, { notation: "compact", maximumFractionDigits: 1 }).format(value) }) as string;
    } catch {
      return String(value);
    }
  }

  function toggle() {
    open = !open;
    if (open) void reload();
  }

  async function answer(action: "raise" | "keep_paused") {
    if (!meter?.incidentId || busy) return;
    busy = true;
    failed = false;
    try {
      await invoke("llm_budget_resolve", { id: meter.incidentId, action, amount: action === "raise" ? raise : null });
      open = false;
    } catch {
      failed = true;
    } finally {
      busy = false;
      await reload();
    }
  }

  onMount(() => {
    let alive = true;
    let run = 0;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let off: (() => void) | null = null;
    reload = async () => {
      const mine = ++run;
      let next: Meter | null = null;
      try {
        const state = (await invoke("llm_budget_state")) as { policies?: PolicyStatus[] } | null;
        next = pickMeter(state?.policies ?? []);
      } catch {
        next = null;
      }
      if (!alive || mine !== run) return;
      meter = next;
      if (!next) open = false;
    };
    const soon = () => {
      if (timer) return;
      timer = setTimeout(() => {
        timer = null;
        void reload();
      }, 300);
    };
    void listen("llm://budget", soon)
      .then((un) => (alive ? (off = un) : un()))
      .catch(() => {});
    const close = (e: PointerEvent) => {
      if (open && host && !host.contains(e.target as Node)) open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (open && e.key === "Escape") open = false;
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", esc);
    void reload();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
      off?.();
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", esc);
    };
  });
</script>

{#if meter}
  {@const spent = amount(meter.spent, meter.metric, $locale)}
  {@const limit = amount(meter.limit, meter.metric, $locale)}
  <div class="budget" data-tone={meter.tone} bind:this={host}>
    <button type="button" class="pill" class:on={open} aria-expanded={open} title={$t("canvas.budget.label") as string} onclick={toggle}>
      <span class="dot" aria-hidden="true"></span>
      <span class="text" role="status">
        {meter.tone === "paused" ? $t("canvas.budget.paused_short") : $t("canvas.budget.usage", { spent, limit })}
      </span>
      {#if meter.tone !== "paused"}
        <span class="track" aria-hidden="true"><span class="bar" style:width="{Math.round(meter.ratio * 100)}%"></span></span>
      {/if}
    </button>
    {#if open}
      <div class="pop" role="group" aria-label={$t("canvas.budget.label") as string}>
        {#if meter.tone === "paused"}
          <p class="title">{$t("canvas.budget.paused_title", { limit })}</p>
          <p class="body">{$t("canvas.budget.paused_body")}</p>
        {:else}
          <p class="title">{$t("canvas.budget.usage", { spent, limit })}{windowText ? ` · ${windowText}` : ""}</p>
          {#if meter.tone === "warn"}<p class="body">{$t("canvas.budget.near")}</p>{/if}
        {/if}
        <div class="track wide" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(meter.ratio * 100)} aria-label={$t("canvas.budget.label") as string}>
          <span class="bar" style:width="{Math.round(meter.ratio * 100)}%"></span>
        </div>
        {#if failed}<p class="error" role="alert">{$t("canvas.budget.failed")}</p>{/if}
        {#if meter.tone === "paused"}
          <div class="actions">
            <button use:agentActions={[]} type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => void answer("keep_paused")}>{$t("canvas.budget.keep")}</button>
            {#if raise !== null}
              <button use:agentActions={[]} type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => void answer("raise")}>
                {$t("canvas.budget.raise", { limit: amount(raise, meter.metric, $locale) })}
              </button>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .budget {
    position: relative;
    --tone: var(--agent-success);
  }
  .budget[data-tone="warn"] {
    --tone: var(--agent-working);
  }
  .budget[data-tone="paused"] {
    --tone: var(--agent-blocked);
  }
  .pill {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 30px;
    padding: 0 10px;
    border-radius: 999px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.06),
      0 4px 14px rgba(0, 0, 0, 0.1);
    font-size: 12px;
    font-weight: 500;
    color: var(--text);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    cursor: pointer;
  }
  .pill:hover,
  .pill.on {
    background: color-mix(in srgb, var(--popup-bg, var(--surface)) 88%, var(--text) 12%);
  }
  .pill:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--tone);
  }
  .track {
    display: block;
    width: 36px;
    height: 4px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--text) 14%, transparent);
    overflow: hidden;
  }
  .track.wide {
    width: 100%;
  }
  .bar {
    display: block;
    height: 100%;
    border-radius: var(--radius-full);
    background: var(--tone);
  }
  .pop {
    position: absolute;
    right: 0;
    bottom: calc(100% + 8px);
    width: 280px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: var(--radius-lg, 12px);
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: var(--elev-3);
    z-index: 2;
  }
  .title {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .body,
  .error {
    margin: 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .error {
    color: var(--agent-blocked);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
