<script lang="ts">
  import Confetti from "$components/celebrate/Confetti.svelte";
  import type { HomeArt } from "$lib/home/omnibox-controller";

  let {
    art,
    size = 132,
    celebrate = false,
  }: {
    art: HomeArt;
    size?: number;
    celebrate?: boolean;
  } = $props();
</script>

<!-- Loop sticker art (Higgsfield, same recipe as the sidebar icons). Decorative:
     the state is always said in text next to it. -->
<div class="home-hero" style:--hero-size="{size}px">
  {#key art}
    <img
      class="hero-art"
      src="/mascot/home/{art}.webp"
      srcset="/mascot/home/{art}.webp 1x, /mascot/home/{art}@2x.webp 2x"
      width={size}
      height={size}
      alt=""
      draggable="false"
    />
  {/key}
  <Confetti active={celebrate} />
</div>

<style>
  .home-hero {
    position: relative;
    display: grid;
    place-items: center;
    width: var(--hero-size);
    height: var(--hero-size);
    transition: width var(--duration-slow) var(--ease-out), height var(--duration-slow) var(--ease-out);
  }

  .hero-art {
    grid-area: 1 / 1;
    width: 100%;
    height: 100%;
    object-fit: contain;
    user-select: none;
    -webkit-user-drag: none;
    filter: drop-shadow(0 10px 18px rgba(var(--shadow-ink), var(--elev-alpha-1)));
    animation: art-in 280ms var(--ease-out);
  }

  @keyframes art-in {
    from { opacity: 0; transform: translateY(6px) scale(0.94); filter: blur(4px); }
    to { opacity: 1; transform: none; filter: drop-shadow(0 10px 18px rgba(var(--shadow-ink), var(--elev-alpha-1))); }
  }

  @media (prefers-reduced-motion: reduce) {
    .home-hero { transition: none; }
    .hero-art { animation: none; }
  }
</style>
