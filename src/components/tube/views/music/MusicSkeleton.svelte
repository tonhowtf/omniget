<script lang="ts">
  let {
    variant = "home",
    idle = false,
  }: {
    variant?: "home" | "collection" | "artist" | "grid" | "list" | "shelves";
    idle?: boolean;
  } = $props();
  const cards = Array.from({ length: 6 }, (_, i) => i);
  const rows = Array.from({ length: 8 }, (_, i) => i);
</script>

<div class="msk" aria-busy={idle ? undefined : "true"} aria-live={idle ? undefined : "polite"}>
  {#if variant === "home"}
    <div class="msk-chips">{#each cards as i (i)}<span class="yt-skel msk-chip" style:width="{72 + ((i * 23) % 40)}px"></span>{/each}</div>
    <span class="yt-skel msk-strap"></span>
    <span class="yt-skel msk-h"></span>
    <div class="msk-qp">
      {#each Array.from({ length: 12 }, (_, i) => i) as i (i)}
        <div class="msk-pick"><span class="yt-skel msk-pc"></span><span class="msk-lines"><span class="yt-skel msk-l1"></span><span class="yt-skel msk-l2"></span></span></div>
      {/each}
    </div>
  {/if}
  {#if variant === "collection"}
    <div class="msk-col">
      <span class="yt-skel msk-cover"></span>
      <div class="msk-meta">
        <span class="yt-skel msk-t"></span>
        <span class="yt-skel msk-l1 w50"></span>
        <span class="yt-skel msk-l2 w40"></span>
        <span class="yt-skel msk-l2 w70"></span>
        <div class="msk-btns"><span class="yt-skel msk-b"></span><span class="yt-skel msk-b"></span><span class="yt-skel msk-play"></span><span class="yt-skel msk-b"></span></div>
      </div>
    </div>
  {/if}
  {#if variant === "artist"}
    <div class="yt-skel msk-hero"></div>
  {/if}
  {#if variant === "collection" || variant === "list" || variant === "artist"}
    <div class="msk-rows">
      {#each rows as i (i)}
        <div class="msk-row"><span class="yt-skel msk-rc"></span><span class="yt-skel msk-l1" style:width="{30 + ((i * 17) % 25)}%"></span><span class="yt-skel msk-l2 w20"></span></div>
      {/each}
    </div>
  {/if}
  {#if variant === "home" || variant === "shelves" || variant === "artist"}
    {#each [0, 1] as s (s)}
      <span class="yt-skel msk-h"></span>
      <div class="msk-cards">
        {#each cards as i (i)}<div class="msk-card"><span class="yt-skel msk-sq"></span><span class="yt-skel msk-l1"></span><span class="yt-skel msk-l2 w60"></span></div>{/each}
      </div>
    {/each}
  {/if}
  {#if variant === "grid"}
    <div class="msk-grid">
      {#each Array.from({ length: 12 }, (_, i) => i) as i (i)}<div class="msk-card"><span class="yt-skel msk-sq"></span><span class="yt-skel msk-l1"></span><span class="yt-skel msk-l2 w60"></span></div>{/each}
    </div>
  {/if}
</div>

<style>
  .msk { display: flex; flex-direction: column; gap: 16px; padding-top: 8px; }
  .msk :global(.yt-skel) { display: block; background: var(--ytm-bg-raised, var(--yt-skeleton)); }
  .msk-chips { display: flex; gap: 12px; margin-bottom: 16px; }
  .msk-chip { height: 32px; border-radius: 8px; }
  .msk-strap { width: 96px; height: 12px; }
  .msk-h { width: 220px; height: 28px; margin-top: 8px; }
  .msk-qp { display: grid; grid-auto-flow: column; grid-template-rows: repeat(4, 56px); grid-auto-columns: minmax(280px, 420px); gap: 8px 24px; overflow: hidden; margin-bottom: 32px; }
  .msk-pick { display: flex; align-items: center; gap: 16px; }
  .msk-pc { width: 48px; height: 48px; border-radius: 4px; flex: none; }
  .msk-lines { display: flex; flex-direction: column; gap: 8px; flex: 1; }
  .msk-l1 { height: 14px; width: 70%; border-radius: 4px; }
  .msk-l2 { height: 12px; width: 45%; border-radius: 4px; }
  .w20 { width: 20% !important; } .w40 { width: 40% !important; } .w50 { width: 50% !important; } .w60 { width: 60% !important; } .w70 { width: 70% !important; }
  .msk-cards { display: flex; gap: 16px; overflow: hidden; margin-bottom: 32px; }
  .msk-card { width: 180px; flex: none; display: flex; flex-direction: column; gap: 8px; }
  .msk-grid .msk-card { width: auto; }
  .msk-sq { aspect-ratio: 1; width: 100%; }
  .msk-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 32px 16px; }
  .msk-col { display: flex; gap: 32px; align-items: flex-end; margin: 24px 0 16px; }
  .msk-cover { width: var(--ytm-header-art, 264px); height: var(--ytm-header-art, 264px); flex: none; }
  .msk-meta { flex: 1; display: flex; flex-direction: column; gap: 12px; }
  .msk-t { height: 40px; width: 55%; }
  .msk-btns { display: flex; gap: 12px; align-items: center; margin-top: 12px; }
  .msk-b { width: 40px; height: 40px; border-radius: 50%; }
  .msk-play { width: 56px; height: 56px; border-radius: 50%; }
  .msk-hero { height: 360px; border-radius: 0; margin: 0 calc(-1 * var(--ytm-pad, 0px)) 16px; }
  .msk-rows { display: flex; flex-direction: column; }
  .msk-row { display: flex; align-items: center; gap: 16px; height: 56px; }
  .msk-rc { width: 40px; height: 40px; border-radius: 2px; flex: none; }
  @container omnitube (max-width: 720px) {
    .msk-col { flex-direction: column; align-items: center; }
    .msk-cover { width: 200px; height: 200px; }
  }
</style>
