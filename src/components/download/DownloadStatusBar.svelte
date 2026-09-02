<script lang="ts">
  import { onDestroy } from "svelte";
  import { t, locale } from "$lib/i18n";
  import NavIcon from "$components/shell/NavIcon.svelte";
  import DownloadSpeedGraph from "./DownloadSpeedGraph.svelte";
  import {
    getAggregate,
    getAggregateSpeedHistory,
    formatBytes,
    formatSpeed,
    formatEta,
  } from "$lib/stores/download-store.svelte";

  const GRACE_MS = 2000;

  let agg = $derived(getAggregate());
  let busy = $derived(agg.activeCount + agg.queuedCount + agg.pausedCount > 0);

  let visible = $state(false);
  let showComplete = $state(false);
  let graceTimer: ReturnType<typeof setTimeout> | null = null;
  let wasBusy = false;

  $effect(() => {
    if (busy) {
      if (graceTimer) {
        clearTimeout(graceTimer);
        graceTimer = null;
      }
      wasBusy = true;
      showComplete = false;
      visible = true;
    } else if (wasBusy) {
      wasBusy = false;
      showComplete = true;
      graceTimer = setTimeout(() => {
        graceTimer = null;
        visible = false;
        showComplete = false;
      }, GRACE_MS);
    }
  });

  onDestroy(() => {
    if (graceTimer) clearTimeout(graceTimer);
  });

  let allPaused = $derived(agg.activeCount === 0 && agg.pausedCount > 0);
  let etaText = $derived(formatEta(agg.etaSeconds));
  let percentRounded = $derived(agg.percent !== null ? Math.round(agg.percent) : null);
  let indeterminate = $derived(!showComplete && agg.percent === null && agg.activeCount > 0);
  let sizeLabel = $derived(
    percentRounded !== null ? `${percentRounded}%` : formatBytes(agg.downloadedBytes),
  );

  let countLabel = $derived(
    agg.activeCount > 0
      ? ($t("downloads.status_bar.active", { count: agg.activeCount }) as string)
      : allPaused
        ? ($t("downloads.status_bar.paused_all") as string)
        : ($t("downloads.status_bar.queued", { count: agg.queuedCount }) as string),
  );

  let announceKey = $derived(
    !visible
      ? ""
      : showComplete
        ? `done:${$locale}`
        : `${$locale}:${agg.activeCount}:${percentRounded === null ? "x" : Math.floor(percentRounded / 25)}`,
  );

  let announceText = $state("");
  let lastAnnounceKey = "";

  function buildAnnouncement(): string {
    if (showComplete) return $t("downloads.status_bar.complete") as string;
    const parts = [countLabel];
    if (percentRounded !== null) parts.push(`${percentRounded}%`);
    else if (agg.activeCount > 0) parts.push($t("downloads.status_bar.indeterminate") as string);
    if (etaText) parts.push($t("downloads.status_bar.eta", { eta: etaText }) as string);
    return parts.join(", ");
  }

  $effect(() => {
    const key = announceKey;
    if (key === lastAnnounceKey) return;
    lastAnnounceKey = key;
    announceText = key ? buildAnnouncement() : "";
  });
</script>

{#if visible}
  <div
    class="dl-status-bar"
    class:is-complete={showComplete}
    role="region"
    aria-label={$t("downloads.status_bar.region_label") as string}
  >
    <div
      class="progress dl-track"
      class:indeterminate={indeterminate}
      role="progressbar"
      aria-label={$t("downloads.status_bar.region_label") as string}
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={showComplete ? 100 : (percentRounded ?? undefined)}
      aria-valuetext={indeterminate
        ? ($t("downloads.status_bar.indeterminate") as string)
        : undefined}
    >
      <div
        class="progress-fill"
        class:success={showComplete}
        class:paused={!showComplete && allPaused}
        style:width={showComplete
          ? "100%"
          : percentRounded !== null
            ? `${agg.percent}%`
            : undefined}
      ></div>
    </div>

    <div class="dl-row">
      <NavIcon icon="downloads" size={14} />
      {#if showComplete}
        <span class="dl-label">{$t("downloads.status_bar.complete")}</span>
      {:else}
        <span class="dl-label">{countLabel}</span>
        {#if agg.activeCount > 0}
          <span class="dl-sep" aria-hidden="true">&middot;</span>
          <span class="dl-metric">{formatSpeed(agg.speedBps)}</span>
        {/if}
        {#if etaText}
          <span class="dl-sep" aria-hidden="true">&middot;</span>
          <span class="dl-metric">{$t("downloads.status_bar.eta", { eta: etaText })}</span>
        {/if}
        {#if agg.speedBps > 0}
          <span class="dl-graph">
            <DownloadSpeedGraph points={getAggregateSpeedHistory()} width={72} height={16} />
          </span>
        {/if}
        {#if agg.activeCount > 0 && agg.queuedCount > 0}
          <span class="dl-sep dl-opt" aria-hidden="true">&middot;</span>
          <span class="dl-metric dl-opt">
            {$t("downloads.status_bar.queued", { count: agg.queuedCount })}
          </span>
        {/if}
      {/if}
      <span class="dl-spacer"></span>
      {#if !showComplete}
        <span class="dl-metric dl-size">{sizeLabel}</span>
      {/if}
      <a class="dl-link" href="/downloads">{$t("downloads.status_bar.open")}</a>
    </div>

    <span class="dl-sr" aria-live="polite" aria-atomic="true">{announceText}</span>
  </div>
{/if}

<style>
  .dl-status-bar {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-4)
      calc(var(--space-2) + env(safe-area-inset-bottom, 0px));
    background: var(--surface-mut);
    border-top: var(--hairline) solid var(--border);
    box-shadow: var(--elev-1);
    animation: dl-status-bar-in var(--duration-base) var(--ease-out);
  }

  @keyframes dl-status-bar-in {
    from {
      opacity: 0;
      transform: translateY(100%);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  :global(body:has(.global-player-bar)) .dl-status-bar {
    margin-bottom: 80px;
  }

  .dl-track {
    height: 3px;
  }

  .dl-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    overflow: hidden;
    font-size: var(--text-xs);
    line-height: var(--leading-xs);
    color: var(--text-muted);
  }

  .dl-label {
    color: var(--text);
    font-weight: 500;
    white-space: nowrap;
  }

  .dl-metric {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .dl-sep {
    opacity: 0.45;
  }

  .dl-graph {
    display: inline-flex;
    align-items: center;
  }

  .dl-spacer {
    flex: 1 1 auto;
    min-width: var(--space-2);
  }

  .dl-size {
    color: var(--text);
    font-weight: 500;
  }

  .dl-link {
    flex: 0 0 auto;
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--accent);
    text-decoration: none;
    font-weight: 500;
    white-space: nowrap;
  }

  @media (hover: hover) {
    .dl-link:hover {
      background: var(--accent-soft);
    }
  }

  .dl-link:active {
    background: var(--accent-soft);
  }

  .dl-link:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .dl-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    border: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @media (max-width: 640px) {
    .dl-status-bar {
      padding-inline: var(--space-3);
    }

    .dl-graph,
    .dl-opt {
      display: none;
    }
  }

</style>
