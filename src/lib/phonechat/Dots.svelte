<!-- Ported from "Dots" by loading-ui (21st.dev). MIT License. -->
<script lang="ts">
  let { label, dots = 3 }: { label: string; dots?: number } = $props();
</script>

<span class="dots" role="status">
  {#each { length: dots } as _, index (index)}
    <span class="dot" aria-hidden="true" style:animation-delay="{0.2 * index}s"></span>
  {/each}
  <span class="sr-only">{label}</span>
</span>

<style>
  @keyframes dots-blink {
    0%,
    100% {
      opacity: 0.2;
    }

    20% {
      opacity: 1;
    }
  }

  .dots {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 12%;
    width: 16px;
    height: 10px;
  }

  .dot {
    flex-grow: 1;
    aspect-ratio: 1;
    border-radius: 999px;
    background: currentColor;
    animation: dots-blink 1.4s infinite both;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .dot {
      animation: none;
      opacity: 0.6;
    }
  }
</style>
