<script lang="ts">
  import type { Tone } from "$lib/washmybelly/game";

  /// Pill with a status dot. Ported from diceui "Status" (21st.dev): rounded
  let { label, tone = "default" }: { label: string; tone?: Tone } = $props();
</script>

<span class="gs-pill tone-{tone}">
  <span class="gs-dot" class:live={tone === "success"} aria-hidden="true"></span>
  <span class="gs-label">{label}</span>
</span>

<style>
  .gs-pill {
    --c: var(--text-dim);
    display: inline-flex;
    align-items: center;
    gap: 6px;
    width: fit-content;
    padding: 3px 10px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--c) 22%, transparent);
    background: color-mix(in srgb, var(--c) 11%, transparent);
    color: var(--c);
    font-size: var(--text-sm);
    font-weight: 500;
    white-space: nowrap;
    line-height: 1.2;
  }
  .tone-success {
    --c: var(--success);
  }
  .tone-info {
    --c: var(--blue, var(--accent));
  }
  .tone-warning {
    --c: var(--warning);
  }
  .gs-dot {
    position: relative;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--c);
    flex-shrink: 0;
  }
  .gs-dot.live::before {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: inherit;
    animation: gs-ping 1.6s cubic-bezier(0, 0, 0.2, 1) infinite;
  }
  @keyframes gs-ping {
    75%,
    100% {
      transform: scale(2.2);
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .gs-dot.live::before {
      animation: none;
    }
  }
</style>
