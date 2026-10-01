<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import { thumbUrl } from "$lib/tube/format";
  import TIcon from "../../TIcon.svelte";
  import type { Thumb } from "$lib/tube/types";

  let {
    thumbs,
    title,
    meta,
    stats = "",
    description = "",
    special = null,
    playing = false,
    saved = null,
    onplay,
    onshuffle,
    onsave,
    ondownload,
    onmore,
  }: {
    thumbs: Thumb[];
    title: string;
    meta?: Snippet;
    stats?: string;
    description?: string | null;
    special?: "liked" | null;
    playing?: boolean;
    saved?: boolean | null;
    onplay: () => void;
    onshuffle?: () => void;
    onsave?: () => void;
    ondownload?: () => void;
    onmore?: (e: MouseEvent) => void;
  } = $props();

  let art = $derived(thumbUrl(thumbs, 264));
  let bg = $derived(thumbUrl(thumbs, 120));
  let descOpen = $state(false);
</script>

<header class="ch">
  {#if bg}
    <div class="ch-bg" aria-hidden="true"><img src={bg} alt="" decoding="async" referrerpolicy="no-referrer" /></div>
  {:else if special === "liked"}
    <div class="ch-bg ch-bg--liked" aria-hidden="true"></div>
  {/if}
  <div class="ch-body">
    <div class="ch-art" class:liked={special === "liked"}>
      {#if special === "liked"}
        <TIcon name="thumbs-up-fill" size={96} />
      {:else if art}
        <img src={art} alt="" decoding="async" referrerpolicy="no-referrer" />
      {:else}
        <TIcon name="album" size={72} />
      {/if}
    </div>
    <div class="ch-info">
      <h1 class="ch-title">{title}</h1>
      {#if meta}<p class="ch-meta">{@render meta()}</p>{/if}
      {#if stats}<p class="ch-stats">{stats}</p>{/if}
      {#if description}
        <div class="ch-desc-wrap">
          <p class="ch-desc" class:open={descOpen}>{description}</p>
          {#if description.length > 160}
            <button class="ch-desc-more" onclick={() => (descOpen = !descOpen)}>{descOpen ? $t("tube.music.less") : $t("tube.music.more_text")}</button>
          {/if}
        </div>
      {/if}
      <div class="ch-actions">
        {#if ondownload}
          <button class="ch-round" onclick={ondownload} aria-label={$t("tube.music.download_all") as string} title={$t("tube.music.download_all") as string}><TIcon name="download" size={22} /></button>
        {/if}
        {#if onsave && saved !== null}
          <button class="ch-round" class:is-on={saved} onclick={onsave} aria-pressed={!!saved} aria-label={(saved ? $t("tube.music.remove_from_library") : $t("tube.music.save_to_library")) as string} title={(saved ? $t("tube.music.remove_from_library") : $t("tube.music.save_to_library")) as string}>
            <TIcon name={saved ? "check" : "plus"} size={22} />
          </button>
        {/if}
        <button class="ch-play" onclick={onplay} aria-label={(playing ? $t("tube.music.pause") : $t("tube.music.play")) as string}>
          <TIcon name={playing ? "pause" : "play"} size={30} />
        </button>
        {#if onshuffle}
          <button class="ch-round" onclick={onshuffle} aria-label={$t("tube.music.shuffle") as string} title={$t("tube.music.shuffle") as string}><TIcon name="shuffle" size={22} /></button>
        {/if}
        {#if onmore}
          <button class="ch-round" onclick={onmore} aria-label={$t("tube.music.more_actions") as string} aria-haspopup="menu"><TIcon name="more-vert" size={22} /></button>
        {/if}
      </div>
    </div>
  </div>
</header>

<style>
  .ch { position: relative; margin: 0 0 32px; padding-top: 16px; }
  .ch-bg {
    position: absolute; left: 50%; top: -24px; width: 100cqi; height: 520px; transform: translateX(-50%);
    overflow: hidden; pointer-events: none; z-index: 0;
  }
  .ch-bg img { width: 100%; height: 100%; object-fit: cover; filter: blur(80px) saturate(1.3); opacity: .45; transform: scale(1.2); }
  .ch-bg::after { content: ""; position: absolute; inset: 0; background: linear-gradient(to bottom, rgba(0, 0, 0, .2), var(--ytm-bg, var(--yt-bg)) 92%); }
  .ch-bg--liked { background: radial-gradient(ellipse at 30% 20%, rgba(98, 60, 234, .45), transparent 60%), radial-gradient(ellipse at 80% 0%, rgba(62, 166, 255, .3), transparent 55%); }
  .ch-bg--liked::after { background: linear-gradient(to bottom, transparent, var(--ytm-bg, var(--yt-bg)) 92%); }

  .ch-body { position: relative; z-index: 1; display: flex; align-items: flex-end; gap: 32px; }
  .ch-art {
    flex: none; width: var(--ytm-header-art, 264px); height: var(--ytm-header-art, 264px);
    border-radius: 8px; overflow: hidden; display: grid; place-items: center;
    background: var(--ytm-bg-raised, var(--yt-skeleton)); color: var(--ytm-text-3, var(--yt-text-2));
    box-shadow: 0 16px 48px rgba(0, 0, 0, .5);
  }
  .ch-art img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .ch-art.liked { background: linear-gradient(135deg, #623cea, #3ea6ff); color: #fff; }

  .ch-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .ch-title {
    margin: 0 0 8px; font: 700 clamp(28px, 4.2cqi, 45px)/1.15 var(--yt-font-display);
    color: var(--ytm-text, var(--yt-text));
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; word-break: break-word;
  }
  .ch-meta, .ch-stats { margin: 0; font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); }
  .ch-meta { color: var(--ytm-text, var(--yt-text)); font-size: 15px; }
  .ch-meta :global(button) { border: 0; padding: 0; background: none; font: inherit; color: inherit; cursor: pointer; }
  .ch-meta :global(button:hover) { text-decoration: underline; }
  .ch-meta :global(button:focus-visible) { outline: 2px solid var(--yt-link); border-radius: 2px; }
  .ch-desc-wrap { max-width: 640px; }
  .ch-desc { margin: 4px 0 0; font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; white-space: pre-line; }
  .ch-desc.open { -webkit-line-clamp: unset; line-clamp: unset; display: block; }
  .ch-desc-more { border: 0; padding: 0; background: none; font: 500 14px/20px var(--yt-font); color: var(--ytm-text, var(--yt-text)); cursor: pointer; }
  .ch-desc-more:hover { text-decoration: underline; }

  .ch-actions { display: flex; align-items: center; gap: 16px; margin-top: 16px; }
  .ch-round {
    display: grid; place-items: center; width: 40px; height: 40px; border-radius: 50%;
    border: 0; padding: 0; color: var(--ytm-text, var(--yt-text));
    background: var(--ytm-chip, var(--yt-tonal)); cursor: pointer;
    transition: background var(--yt-dur-base), transform var(--yt-dur-fast);
  }
  .ch-round:hover { background: var(--ytm-chip-hover, var(--yt-tonal-hover)); }
  .ch-round.is-on { background: var(--ytm-text, var(--yt-text)); color: var(--ytm-bg, var(--yt-bg)); }
  .ch-play {
    display: grid; place-items: center; width: 56px; height: 56px; border-radius: 50%;
    border: 0; padding: 0; color: #000; background: #fff; cursor: pointer;
    box-shadow: 0 4px 16px rgba(0, 0, 0, .35);
    transition: transform var(--yt-dur-base) var(--yt-ease-standard), background var(--yt-dur-base);
  }
  .ch-play:hover { transform: scale(1.06); background: #f1f1f1; }
  .ch-play:active, .ch-round:active { transform: scale(.95); }
  .ch-play:focus-visible, .ch-round:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 3px; }

  @container omnitube (max-width: 720px) {
    .ch-body { flex-direction: column; align-items: center; text-align: center; gap: 20px; }
    .ch-art { width: 200px; height: 200px; }
    .ch-actions { justify-content: center; }
    .ch-desc-wrap { margin: 0 auto; }
  }
  @media (prefers-reduced-motion: reduce) { .ch-play, .ch-round { transition: none; } }
</style>
