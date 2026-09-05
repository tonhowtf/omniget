<script lang="ts">
  import { t } from "$lib/i18n";
  import type { CommandRecord, StreamInfo } from "$lib/stores/download-store.svelte";

  type Props = {
    phase: string;
    plannedFormats?: string[] | null;
    stream?: StreamInfo | null;
    streamsDone?: StreamInfo[];
    fragmentIndex?: number | null;
    fragmentCount?: number | null;
    command?: CommandRecord | null;
    downloadMode?: string | null;
  };

  let {
    phase,
    plannedFormats = null,
    stream = null,
    streamsDone = [],
    fragmentIndex = null,
    fragmentCount = null,
    command = null,
    downloadMode = null,
  }: Props = $props();

  type Step = "info" | "video" | "audio" | "download" | "merge" | "finish";

  const hasVideo = (s: StreamInfo | null | undefined) => !!s?.vcodec && s.vcodec !== "none";
  const hasAudio = (s: StreamInfo | null | undefined) => !!s?.acodec && s.acodec !== "none";

  // Dois streams (bv+ba) → trilho completo; um stream (muxado ou só áudio) →
  // trilho curto. Decidido pelo plano que o yt-dlp anunciou; se ainda não
  // anunciou, pelo que já vimos passar.
  let multi = $derived.by(() => {
    if (plannedFormats && plannedFormats.length > 0) return plannedFormats.length > 1;
    if (downloadMode === "audio") return false;
    return streamsDone.length > 0 && !!stream;
  });

  let steps = $derived<Step[]>(multi ? ["info", "video", "audio", "merge", "finish"] : ["info", "download", "finish"]);

  let activeIndex = $derived.by(() => {
    const last = steps.length - 1;
    switch (phase) {
      case "preparing":
      case "fetching_info":
      case "queued_starting":
      case "queued":
        return 0;
      case "starting":
      case "connecting":
      case "waiting_rate_limit":
        return 1;
      case "downloading_video":
        return 1;
      case "downloading_audio":
        return multi ? 2 : 1;
      case "downloading":
      case "stalled":
        if (multi) return hasAudio(stream) && !hasVideo(stream) ? 2 : streamsDone.some(hasVideo) ? 2 : 1;
        return 1;
      case "merging":
        return multi ? 3 : last;
      case "extracting_audio":
      case "embedding_subtitles":
      case "postprocessing":
      case "finalizing":
        return last;
      default:
        return 1;
    }
  });

  let phaseKey = $derived.by(() => {
    switch (phase) {
      case "preparing":
      case "fetching_info":
      case "starting":
      case "connecting":
      case "waiting_rate_limit":
      case "merging":
      case "extracting_audio":
      case "embedding_subtitles":
      case "postprocessing":
      case "finalizing":
      case "stalled":
      case "downloading_video":
      case "downloading_audio":
      case "queued_starting":
        return `downloads.phase_${phase}`;
      default:
        return "downloads.phase_downloading";
    }
  });

  let stepLabel = (s: Step) => $t(`downloads.steps.${s}`) as string;

  let details = $derived.by(() => {
    const out: string[] = [];
    if (fragmentCount && fragmentCount > 1) {
      out.push($t("downloads.detail.fragments", { index: String(fragmentIndex ?? 0), count: String(fragmentCount) }) as string);
    }
    if (command) {
      if (command.overridden) {
        out.push($t("downloads.detail.custom_command") as string);
      } else {
        if (command.connections > 1) {
          out.push($t("downloads.detail.connections", { count: String(command.connections) }) as string);
        }
        if (command.engine === "aria2c") out.push("aria2c");
        if (command.max_attempts > 1 && command.attempt > 1) {
          out.push($t("downloads.detail.attempt", { attempt: String(command.attempt), max: String(command.max_attempts) }) as string);
        }
        if (command.player_client && command.player_client !== "default") {
          out.push($t("downloads.detail.client", { client: command.player_client }) as string);
        }
      }
    }
    return out;
  });
</script>

<div class="phases" data-phase={phase}>
  <ol class="phase-rail" aria-label={$t(phaseKey)}>
    {#each steps as step, i (step)}
      <li class="phase-step" class:done={i < activeIndex} class:active={i === activeIndex} class:stalled={phase === "stalled" && i === activeIndex}>
        <span class="phase-dot" aria-hidden="true">
          {#if i < activeIndex}
            <svg viewBox="0 0 12 12" width="8" height="8" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 6.5l2.3 2.3L9.5 4" /></svg>
          {/if}
        </span>
        <span class="phase-label">{stepLabel(step)}</span>
      </li>
    {/each}
  </ol>
  <p class="phase-text">
    <span class="phase-now" class:phase-warn={phase === "stalled" || phase === "waiting_rate_limit"}>{$t(phaseKey)}</span>
    {#each details as d (d)}
      <span class="phase-sep" aria-hidden="true">·</span><span class="phase-detail">{d}</span>
    {/each}
  </p>
</div>

<style>
  .phases {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .phase-rail {
    display: flex;
    align-items: center;
    gap: 0;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow: hidden;
  }

  .phase-step {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    position: relative;
    padding-right: 18px;
    color: var(--text-faint);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .phase-step::after {
    content: "";
    position: absolute;
    right: 6px;
    top: 50%;
    width: 8px;
    height: var(--hairline);
    background: var(--content-border);
  }

  .phase-step:last-child {
    padding-right: 0;
  }

  .phase-step:last-child::after {
    display: none;
  }

  .phase-dot {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--on-status, #fff);
    flex-shrink: 0;
  }

  .phase-step.done { color: var(--text-dim); }
  .phase-step.done .phase-dot { background: var(--success); box-shadow: none; }

  .phase-step.active { color: var(--accent-hi); }
  .phase-step.active .phase-dot {
    background: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .phase-step.stalled { color: var(--warning); }
  .phase-step.stalled .phase-dot { background: var(--warning); box-shadow: 0 0 0 3px color-mix(in srgb, var(--warning) 20%, transparent); }

  @media (prefers-reduced-motion: no-preference) {
    .phase-step.active .phase-dot { animation: phase-pulse 1.6s ease-in-out infinite; }
    @keyframes phase-pulse {
      0%, 100% { box-shadow: 0 0 0 3px var(--accent-soft); }
      50% { box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 10%, transparent); }
    }
  }

  .phase-text {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .phase-now { color: var(--text-muted); font-weight: 500; }
  .phase-now.phase-warn { color: var(--warning); }
  .phase-sep { margin: 0 5px; color: var(--text-faint); }
  .phase-detail { font-variant-numeric: tabular-nums; }
</style>
