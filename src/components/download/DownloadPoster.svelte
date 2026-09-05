<script lang="ts">
  import type { QueueKind } from "$lib/stores/download-store.svelte";

  type Props = {
    src?: string | null;
    kind?: QueueKind;
    loading?: boolean;
    durationSeconds?: number | null;
    size?: "md" | "sm";
  };

  let { src = null, kind = "generic", loading = false, durationSeconds = null, size = "md" }: Props = $props();

  // Arte padrão por tipo: quando não há poster (áudio, arquivo direto, thumb
  // que falhou) o card ainda diz o que está descendo, sem ficar um retângulo vazio.
  const ART: Record<string, string> = {
    video: "film_frames",
    audio: "headphone",
    image: "sparkles",
    pdf: "paperclip",
    book: "books",
    webpage: "link",
    telegram_media: "satellite_antenna",
    course_lesson: "clapper_board",
    generic: "package",
  };

  let failed = $state(false);
  $effect(() => {
    // Trocou a URL (thumb chegou depois do info fetch): tenta de novo.
    void src;
    failed = false;
  });

  let art = $derived(`/emoji/${ART[kind ?? "generic"] ?? "package"}.png`);
  let showImage = $derived(!!src && !failed);

  function fmtDuration(total: number): string {
    const s = Math.max(0, Math.round(total));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = s % 60;
    const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
    return `${h > 0 ? h + ":" : ""}${mm}:${String(sec).padStart(2, "0")}`;
  }
</script>

<div class="poster" class:sm={size === "sm"} data-kind={kind ?? "generic"} aria-hidden="true">
  {#if showImage}
    <img class="poster-img" src={src} alt="" loading="lazy" decoding="async" draggable="false" onerror={() => (failed = true)} />
  {:else}
    <div class="poster-fallback" class:shimmer={loading}>
      <img class="poster-art" src={art} alt="" width="34" height="34" draggable="false" />
    </div>
  {/if}
  {#if durationSeconds && durationSeconds > 0}
    <span class="poster-duration">{fmtDuration(durationSeconds)}</span>
  {/if}
</div>

<style>
  .poster {
    position: relative;
    flex-shrink: 0;
    width: 128px;
    aspect-ratio: 16 / 9;
    border-radius: 8px;
    overflow: hidden;
    background: var(--fill-2);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .poster.sm {
    width: 88px;
    border-radius: 6px;
  }

  .poster-img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .poster-fallback {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    background:
      radial-gradient(120% 90% at 20% 0%, var(--queue-kind-generic-bg) 0%, transparent 70%),
      linear-gradient(160deg, var(--fill-2), var(--fill-1));
  }

  .poster[data-kind="video"] .poster-fallback { background: radial-gradient(120% 90% at 20% 0%, var(--queue-kind-video-bg) 0%, transparent 70%), linear-gradient(160deg, var(--fill-2), var(--fill-1)); }
  .poster[data-kind="audio"] .poster-fallback { background: radial-gradient(120% 90% at 20% 0%, var(--queue-kind-audio-bg) 0%, transparent 70%), linear-gradient(160deg, var(--fill-2), var(--fill-1)); }
  .poster[data-kind="image"] .poster-fallback { background: radial-gradient(120% 90% at 20% 0%, var(--queue-kind-image-bg) 0%, transparent 70%), linear-gradient(160deg, var(--fill-2), var(--fill-1)); }
  .poster[data-kind="pdf"] .poster-fallback { background: radial-gradient(120% 90% at 20% 0%, var(--queue-kind-pdf-bg) 0%, transparent 70%), linear-gradient(160deg, var(--fill-2), var(--fill-1)); }
  .poster[data-kind="book"] .poster-fallback { background: radial-gradient(120% 90% at 20% 0%, var(--queue-kind-book-bg) 0%, transparent 70%), linear-gradient(160deg, var(--fill-2), var(--fill-1)); }
  .poster[data-kind="telegram_media"] .poster-fallback { background: radial-gradient(120% 90% at 20% 0%, var(--queue-kind-telegram_media-bg) 0%, transparent 70%), linear-gradient(160deg, var(--fill-2), var(--fill-1)); }
  .poster[data-kind="course_lesson"] .poster-fallback { background: radial-gradient(120% 90% at 20% 0%, var(--queue-kind-course_lesson-bg) 0%, transparent 70%), linear-gradient(160deg, var(--fill-2), var(--fill-1)); }

  .poster-art {
    width: 34px;
    height: 34px;
    opacity: 0.92;
    filter: drop-shadow(0 2px 4px rgba(0, 0, 0, 0.18));
  }

  .poster.sm .poster-art {
    width: 26px;
    height: 26px;
  }

  .shimmer::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, transparent 20%, color-mix(in srgb, var(--text) 8%, transparent) 50%, transparent 80%);
    background-size: 200% 100%;
    animation: poster-shimmer 1.4s linear infinite;
  }

  @keyframes poster-shimmer {
    from { background-position: 200% 0; }
    to { background-position: -200% 0; }
  }

  @media (prefers-reduced-motion: reduce) {
    .shimmer::after { animation: none; }
  }

  .poster-duration {
    position: absolute;
    right: 4px;
    bottom: 4px;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.62);
    color: #fff;
    font-size: 10px;
    font-weight: 600;
    line-height: 14px;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.01em;
  }
</style>
