<script lang="ts">
  type MascotEmotion = "idle" | "downloading" | "error" | "stalled" | "queue";

  function emotionToSrc(e: MascotEmotion): string {
    if (e === "queue") return "/mascot/downloading.png";
    return `/mascot/${e}.png`;
  }

  let { emotion = "idle" }: { emotion?: MascotEmotion } = $props();
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
    }, 300);
  });

  function onCurrentLoad() {
    showCurrent = true;
    errored = false;
  }

  function onCurrentError() {
    if (!showCurrent) errored = true;
  }
</script>

<div class="mascot">
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
      width="152"
      height="152"
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
</div>

<style>
  .mascot {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 152px;
    position: relative;
  }

  .mascot-img {
    height: 152px;
    width: auto;
    opacity: 0;
    transition: opacity 0.3s ease;
    pointer-events: none;
    user-select: none;
  }

  .mascot-img.visible {
    opacity: 1;
  }

  .mascot-fallback {
    color: var(--gray);
    opacity: 0.5;
    pointer-events: none;
  }
</style>
