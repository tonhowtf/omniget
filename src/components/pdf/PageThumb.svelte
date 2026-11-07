<script lang="ts">
  import type { PageItem } from "$lib/pdf/pages";
  import type { Thumbs } from "$lib/pdf/thumbs";

  let { item, thumbs, width = 132 }: { item: PageItem; thumbs: Thumbs; width?: number } = $props();

  let host = $state<HTMLElement | null>(null);
  let visible = $state(false);
  let url = $state<string | null>(null);
  let failed = $state(false);

  let rotate = $derived(item.kind === "page" ? item.rotate : 0);
  let blankRatio = $derived(item.kind === "blank" ? item.height / item.width : 1.414);

  $effect(() => {
    if (!host) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          visible = true;
          io.disconnect();
        }
      },
      { rootMargin: "400px" },
    );
    io.observe(host);
    return () => io.disconnect();
  });

  $effect(() => {
    if (!visible || item.kind !== "page") return;
    let alive = true;
    failed = false;
    thumbs
      .render(item.path, item.page, width)
      .then((u) => {
        if (alive) url = u;
      })
      .catch(() => {
        if (alive) failed = true;
      });
    return () => {
      alive = false;
    };
  });
</script>

<div class="pt" bind:this={host} style:--w="{width}px">
  {#if item.kind === "blank"}
    <div class="pt-sheet pt-blank" style:aspect-ratio="1 / {blankRatio}"></div>
  {:else if url}
    <img class="pt-img" class:sideways={rotate % 180 !== 0} src={url} alt="" draggable="false" style:transform="rotate({rotate}deg)" />
  {:else if failed}
    <div class="pt-sheet pt-failed" aria-hidden="true">!</div>
  {:else}
    <div class="pt-sheet pt-loading" aria-hidden="true"></div>
  {/if}
</div>

<style>
  .pt {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--w);
    height: calc(var(--w) * 1.3);
    pointer-events: none;
  }

  .pt-img {
    display: block;
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: 3px;
    background: #fff;
    box-shadow: 0 1px 3px rgb(0 0 0 / 0.18), 0 0 0 var(--hairline) rgb(0 0 0 / 0.08);
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .pt-img.sideways {
    max-width: calc(var(--w) * 1.3);
    max-height: var(--w);
  }

  .pt-sheet {
    width: calc(var(--w) * 0.78);
    aspect-ratio: 1 / 1.414;
    max-height: 100%;
    border-radius: 3px;
    background: #fff;
    box-shadow: 0 1px 3px rgb(0 0 0 / 0.18), 0 0 0 var(--hairline) rgb(0 0 0 / 0.08);
  }

  .pt-loading {
    background: var(--surface-2, var(--surface));
    animation: pt-pulse 1.4s ease-in-out infinite;
  }

  .pt-failed {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
    font-weight: 700;
  }

  @keyframes pt-pulse {
    50% {
      opacity: 0.55;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pt-img {
      transition: none;
    }

    .pt-loading {
      animation: none;
    }
  }
</style>
