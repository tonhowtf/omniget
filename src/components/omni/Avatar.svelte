<script lang="ts">
  import { OMNI_STRIP, moodPose, stripAnimationCss } from "$lib/omni/skins";
  import { tintToCss } from "$lib/stores/profile-store.svelte";

  let {
    tint,
    size = 44,
    animate = "auto",
    label,
    compact = false,
  }: {
    tint: readonly number[] | null | undefined;
    size?: number;
    compact?: boolean;
    animate?: "auto" | "always" | "never";
    label?: string;
  } = $props();

  const anim = stripAnimationCss(OMNI_STRIP, moodPose("neutral"));
  let scale = $derived(compact ? size / 72 : size / 44);
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
  data-animate={animate}
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
        circle at 50% 24%,
        color-mix(in srgb, var(--omni-tint, var(--accent)) 36%, transparent),
        transparent 72%
      ),
      color-mix(in srgb, var(--omni-tint, var(--accent)) 18%, var(--surface-hi));
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--omni-tint, var(--accent)) 52%, var(--content-border));
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

  .omni-avatar[data-animate="always"] .omni-sprite {
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
    .omni-avatar[data-animate="auto"]:hover .omni-sprite {
      animation: none;
      background-position: var(--omni-from) 0;
    }
  }
</style>
