<script lang="ts">
  import { t } from "$lib/i18n";
  import type { SponsorBlockSegment } from "$lib/study-bridge";

  type Props = {
    segments: SponsorBlockSegment[];
    currentTimeMs: number;
    autoSkip?: boolean;
    onSkip: (toMs: number, category: string) => void;
    onDismiss?: (uuid: string) => void;
  };

  let {
    segments,
    currentTimeMs,
    autoSkip = false,
    onSkip,
    onDismiss,
  }: Props = $props();

  const CATEGORY_LABELS = $derived<Record<string, string>>({
    sponsor: $t("study.player.sponsor.sponsor"),
    selfpromo: $t("study.player.sponsor.selfpromo"),
    intro: $t("study.player.sponsor.intro"),
    outro: $t("study.player.sponsor.outro"),
    interaction: $t("study.player.sponsor.interaction"),
    preview: $t("study.player.sponsor.preview"),
    music_offtopic: $t("study.player.sponsor.music_offtopic"),
    filler: $t("study.player.sponsor.filler"),
  });

  let dismissed = $state<Set<string>>(new Set());
  let autoSkipped = $state<Set<string>>(new Set());

  const activeSegment = $derived.by(() => {
    for (const seg of segments) {
      if (dismissed.has(seg.uuid)) continue;
      if (
        currentTimeMs >= seg.start_ms &&
        currentTimeMs < seg.start_ms + 2_000 &&
        currentTimeMs < seg.end_ms
      ) {
        return seg;
      }
    }
    return null;
  });

  $effect(() => {
    if (!autoSkip) return;
    if (!activeSegment) return;
    if (autoSkipped.has(activeSegment.uuid)) return;
    autoSkipped.add(activeSegment.uuid);
    autoSkipped = new Set(autoSkipped);
    onSkip(activeSegment.end_ms, activeSegment.category);
  });

  $effect(() => {
    if (segments.length === 0) {
      dismissed = new Set();
      autoSkipped = new Set();
    }
  });

  function label(category: string): string {
    return CATEGORY_LABELS[category] ?? category;
  }

  function handleSkip() {
    if (!activeSegment) return;
    onSkip(activeSegment.end_ms, activeSegment.category);
  }

  function handleDismiss() {
    if (!activeSegment) return;
    dismissed.add(activeSegment.uuid);
    dismissed = new Set(dismissed);
    onDismiss?.(activeSegment.uuid);
  }
</script>

{#if activeSegment && !autoSkip}
  <div class="sb-overlay" role="region" aria-label="SponsorBlock">
    <button type="button" class="skip-btn" onclick={handleSkip}>
      <span>Pular {label(activeSegment.category)}</span>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <polyline points="13 17 18 12 13 7" />
        <polyline points="6 17 11 12 6 7" />
      </svg>
    </button>
    <button type="button" class="dismiss-btn" onclick={handleDismiss} aria-label="Dispensar">
      <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    </button>
  </div>
{/if}

<style>
  .sb-overlay {
    position: absolute;
    bottom: 64px;
    right: 16px;
    display: inline-flex;
    align-items: stretch;
    gap: 0;
    z-index: 5;
    animation: slide-in 200ms ease-out;
  }

  .skip-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    background: color-mix(in oklab, black 75%, transparent);
    color: white;
    border: 1px solid color-mix(in oklab, white 20%, transparent);
    border-right: none;
    border-radius: 6px 0 0 6px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    backdrop-filter: blur(6px);
    box-shadow: 0 4px 12px color-mix(in oklab, black 30%, transparent);
  }

  .skip-btn:hover {
    background: color-mix(in oklab, var(--accent) 60%, black);
    border-color: var(--accent);
  }

  .dismiss-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 10px;
    background: color-mix(in oklab, black 75%, transparent);
    color: color-mix(in oklab, white 70%, transparent);
    border: 1px solid color-mix(in oklab, white 20%, transparent);
    border-radius: 0 6px 6px 0;
    cursor: pointer;
    backdrop-filter: blur(6px);
    box-shadow: 0 4px 12px color-mix(in oklab, black 30%, transparent);
  }

  .dismiss-btn:hover {
    color: white;
    background: color-mix(in oklab, black 60%, transparent);
  }

  @keyframes slide-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .sb-overlay {
      animation: none;
    }
  }
</style>
