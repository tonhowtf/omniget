<script lang="ts">
  type Emotion = "smile" | "error" | "thinking";

  let { emotion = "smile" }: { emotion?: Emotion } = $props();
  let loaded = $state(false);
  let errored = $state(false);
</script>

<div class="mascot">
  {#if !errored}
    <img
      src="/mascot/{emotion}.png"
      alt="OmniGet mascot"
      class="mascot-img"
      class:visible={loaded}
      onload={() => (loaded = true)}
      onerror={() => (errored = true)}
      draggable="false"
    />
  {/if}
  {#if errored || !loaded}
    <svg
      class="mascot-fallback"
      class:hidden={loaded && !errored}
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
    position: absolute;
    pointer-events: none;
  }

  .mascot-fallback.hidden {
    display: none;
  }
</style>
