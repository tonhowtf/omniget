<script lang="ts">
  type MascotEmotion = "idle" | "downloading" | "error" | "stalled" | "queue" | "complete" | "amazed";

  function emotionToSrc(e: MascotEmotion): string {
    return `/mascot/${e}.png`;
  }

  let {
    emotion = "idle",
    compact = false,
    bubbleText,
  }: {
    emotion?: MascotEmotion;
    compact?: boolean;
    bubbleText?: string;
  } = $props();

  let currentSrc = $state("/mascot/idle.png");
  let nextSrc = $state("");
  let showCurrent = $state(false);
  let showNext = $state(false);
  let errored = $state(false);
  let transitioning = $state(false);

  $effect(() => {
    const target = emotionToSrc(emotion);
    if (target === currentSrc && !transitioning) return;
    if (transitioning) return;

    if (!showCurrent) {
      currentSrc = target;
      return;
    }

    transitioning = true;
    nextSrc = target;
    showNext = false;
    showCurrent = false;

    setTimeout(() => {
      currentSrc = target;
      nextSrc = "";
      showCurrent = false;
      transitioning = false;
    }, 200);
  });

  function onCurrentLoad() {
    showCurrent = true;
    errored = false;
  }

  function onCurrentError() {
    if (!showCurrent) errored = true;
  }
</script>

<div class="mascot" class:compact>
  {#if !errored}
    <img
      src={currentSrc}
      alt="OmniGet mascot"
      class="mascot-img"
      class:visible={showCurrent}
      onload={onCurrentLoad}
      onerror={onCurrentError}
      draggable="false"
    />
  {/if}
  {#if errored}
    <svg
      class="mascot-fallback"
      viewBox="0 0 64 64"
      width="100"
      height="100"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <rect x="12" y="8" width="40" height="48" rx="6" />
      <path d="M32 22v14m0 0l-6-6m6 6l6-6" />
      <path d="M20 48h24" />
    </svg>
  {/if}
  {#if bubbleText}
    <div class="mascot-bubble" role="status" aria-live="polite">
      <span>{bubbleText}</span>
    </div>
  {/if}
</div>

<style>
  .mascot {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    height: 100px;
    position: relative;
    transition: height 0.2s ease;
  }

  .mascot.compact {
    height: 72px;
  }

  .mascot-img {
    height: 100px;
    width: auto;
    opacity: 0;
    transform: scale(0.85);
    transition:
      opacity var(--duration-base) var(--ease-out),
      transform var(--duration-bounce) var(--ease-spring),
      height var(--duration-base) var(--ease-out);
    pointer-events: none;
    user-select: none;
  }

  .mascot.compact .mascot-img {
    height: 72px;
  }

  .mascot-img.visible {
    opacity: 1;
    transform: scale(1);
  }

  .mascot-fallback {
    color: var(--gray);
    opacity: 0.5;
    pointer-events: none;
  }

  .mascot-bubble {
    background: var(--surface-hi);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-4);
    font-size: var(--text-sm);
    color: var(--text);
    max-width: 280px;
    text-align: center;
    margin-top: var(--space-2);
    position: relative;
    box-shadow: var(--elev-1);
    animation: bubbleFadeIn var(--duration-base) var(--ease-out);
  }

  .mascot-bubble::before {
    content: '';
    position: absolute;
    top: -6px;
    left: 50%;
    transform: translateX(-50%);
    width: 12px;
    height: 6px;
    background: var(--surface-hi);
    clip-path: polygon(50% 0%, 0% 100%, 100% 100%);
  }

  @keyframes bubbleFadeIn {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  @media (prefers-reduced-motion: reduce) {
    .mascot-bubble {
      animation: none;
    }

    .mascot,
    .mascot-img {
      transition: opacity var(--duration-base) var(--ease-out);
      transform: none;
    }
  }
</style>
