<script lang="ts">
  // A book or series cover with a typographic fallback (title on a tint
  // derived from the title) while loading or when there is no image.
  let {
    src = "",
    title = "",
    ratio = 1.5,
    radius = 6,
  }: { src?: string; title?: string; ratio?: number; radius?: number } = $props();

  let failed = $state(false);
  $effect(() => {
    void src;
    failed = false;
  });

  let hue = $derived([...title].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 7));
</script>

<div class="cv" style:aspect-ratio="1 / {ratio}" style:border-radius="{radius}px" style:--h={hue}>
  {#if !src || failed}
    <span class="fb">{title}</span>
  {/if}
  {#if src && !failed}
    <img {src} alt="" loading="lazy" decoding="async" draggable="false" onerror={() => (failed = true)} />
  {/if}
</div>

<style>
  .cv {
    position: relative;
    width: 100%;
    overflow: hidden;
    background: linear-gradient(160deg, hsl(var(--h) 38% 42%), hsl(calc(var(--h) + 40) 42% 26%));
    box-shadow:
      0 1px 2px rgb(0 0 0 / 0.18),
      inset 0 0 0 var(--hairline) rgb(255 255 255 / 0.08);
  }
  .fb {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: flex-end;
    padding: 10%;
    font-family: var(--font-display);
    font-weight: 700;
    font-size: clamp(11px, 1.1vw, 15px);
    line-height: 1.2;
    color: #fff;
    overflow: hidden;
    overflow-wrap: anywhere;
    text-shadow: 0 1px 2px rgb(0 0 0 / 0.4);
  }
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>
