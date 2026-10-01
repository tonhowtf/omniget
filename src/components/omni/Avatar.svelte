<script lang="ts">
  import {
    OMNI_STRIP,
    moodPose,
    stripAnimationCss,
    type Mood,
    type OmniPose,
  } from "$lib/omni/skins";
  import { tintToCss } from "$lib/stores/profile-store.svelte";

  let {
    tint,
    size = 44,
    mood = "neutral",
    speaking = false,
    animate = "auto",
    pose,
    label,
    compact = false,
  }: {
    tint: readonly number[] | null | undefined;
    size?: number;
    compact?: boolean;
    mood?: Mood | string;
    speaking?: boolean;
    animate?: "auto" | "always" | "never";
    pose?: OmniPose;
    label?: string;
  } = $props();

  let activePose = $derived(pose ?? moodPose(mood, speaking));
  let anim = $derived(stripAnimationCss(OMNI_STRIP, activePose));
  let scale = $derived(compact ? size / 72 : size / 40);
</script>

<span
  class="omni-avatar"
  style:--omni-size="{size}px"
  style:--omni-tint={tintToCss(tint)}
  style:--omni-image="url({anim.image})"
  style:--omni-sheet-w="{anim.widthPx}px"
  style:--omni-frame-w="{anim.frameW}px"
  style:--omni-frame-h="{anim.frameH}px"
  style:--omni-from="{anim.fromPx}px"
  style:--omni-to="{anim.toPx}px"
  style:--omni-steps={anim.steps}
  style:--omni-duration="{anim.durationMs}ms"
  style:--omni-scale={scale}
  data-mood={mood}
  data-pose={activePose}
  data-animate={animate}
  data-speaking={speaking ? "true" : "false"}
  role={label ? "img" : "presentation"}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  <span class="omni-sprite"></span>
</span>

<style>
  .omni-avatar {
    position: relative;
    display: inline-block;
    width: var(--omni-size);
    height: var(--omni-size);
    flex-shrink: 0;
    border-radius: var(--radius-full);
    overflow: hidden;
    background:
      radial-gradient(
        circle at 50% 22%,
        rgba(255, 255, 255, 0.34),
        rgba(255, 255, 255, 0) 62%
      ),
      var(--omni-tint, #6e8bff);
    box-shadow:
      inset 0 0 0 var(--hairline) rgba(0, 0, 0, 0.18),
      var(--elev-1);
  }

  .omni-sprite {
    position: absolute;
    left: 50%;
    bottom: 0;
    width: var(--omni-frame-w);
    height: var(--omni-frame-h);
    transform-origin: bottom center;
    transform: translateX(-50%) scale(var(--omni-scale));
    background-image: var(--omni-image);
    background-repeat: no-repeat;
    background-size: var(--omni-sheet-w) var(--omni-frame-h);
    background-position: var(--omni-from) 0;
    image-rendering: pixelated;
    pointer-events: none;
  }

  @keyframes omni-walk {
    from {
      background-position-x: var(--omni-from);
    }
    to {
      background-position-x: var(--omni-to);
    }
  }

  .omni-avatar[data-animate="always"] .omni-sprite,
  .omni-avatar[data-animate="auto"][data-speaking="true"] .omni-sprite {
    animation: omni-walk var(--omni-duration) steps(var(--omni-steps)) infinite;
  }

  @media (hover: hover) {
    .omni-avatar[data-animate="auto"]:hover .omni-sprite {
      animation: omni-walk var(--omni-duration) steps(var(--omni-steps)) infinite;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .omni-avatar .omni-sprite,
    .omni-avatar[data-animate="always"] .omni-sprite,
    .omni-avatar[data-animate="auto"][data-speaking="true"] .omni-sprite,
    .omni-avatar[data-animate="auto"]:hover .omni-sprite {
      animation: none;
      background-position: var(--omni-from) 0;
    }
  }
</style>
