<script lang="ts">
  /**
   * The /llm header's view of all work (3.9, stage 1): "Needs you (N)" when
   * something waits for a person, else the strongest pill with its count.
   * Mounted by the /llm layout, so it is on every /llm route; it follows the
   * stores' events (no timer except the minute that ages "done recently"),
   * and it is where the one-time notifications are sent from.
   */
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { announce, getWorkItems, initWorkStore, tickWorkClock } from "$lib/stores/llm-work-store.svelte";
  import { summarize } from "$lib/llm/work";

  let items = $derived(getWorkItems());
  let summary = $derived(summarize(items));
  let path = $derived(page.url.pathname.replace(/\/$/, "") || "/llm");
  /** Without anything waiting: what runs, else what waits for review, else the queue. */
  let quiet = $derived(
    summary.running > 0
      ? { key: "llm.work.badge_running", count: summary.running, tone: "blue" }
      : summary.review > 0
        ? { key: "llm.work.badge_review", count: summary.review, tone: "orange" }
        : summary.queued > 0
          ? { key: "llm.work.badge_queued", count: summary.queued, tone: "grey" }
          : null,
  );

  onMount(() => {
    initWorkStore();
    const clock = setInterval(() => tickWorkClock(), 60_000);
    return () => clearInterval(clock);
  });

  // Once per (item, turn, status), persisted: see `announce`.
  $effect(() => {
    announce(items, path);
  });
</script>

{#if summary.needsYou > 0}
  <a class="work-badge orange" href="/llm/work" aria-live="polite" data-testid="work-needs-you">
    <span class="dot" aria-hidden="true"></span>{$t("llm.work.badge_needs_you", { count: summary.needsYou })}
  </a>
{:else if quiet}
  <a class="work-badge {quiet.tone}" href="/llm/work" aria-live="polite">{$t(quiet.key, { count: quiet.count })}</a>
{/if}

<style>
  .work-badge { display: inline-flex; align-items: center; gap: 6px; padding: 4px 10px; border-radius: 999px; font-size: var(--text-xs); font-weight: 600; text-decoration: none; white-space: nowrap; color: var(--text-muted); background: var(--fill-2); }
  .work-badge.orange { color: var(--orange); background: color-mix(in srgb, var(--orange) 16%, transparent); }
  .work-badge.blue { color: var(--blue); background: color-mix(in srgb, var(--blue) 14%, transparent); }
  .work-badge.red { color: var(--red); background: color-mix(in srgb, var(--red) 14%, transparent); }
  .work-badge:focus-visible { outline: var(--focus-ring); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
</style>
