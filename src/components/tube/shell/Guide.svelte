<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { agentActions } from "$lib/native-agent/actions";
  import { bestAvatar, thumbUrl } from "$lib/tube/format";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import { current, go, goRoot, ui } from "$lib/tube/ui.svelte";
  import TIcon from "../TIcon.svelte";
  import { FOOT, MUSIC_MAIN, VIDEO_MAIN, VIDEO_YOU, activeId, type NavEntry } from "./nav";
  import { session } from "./session.svelte";

  let { onnavigate }: { onnavigate?: () => void } = $props();

  const SUBS_SHOWN = 7;
  let allSubs = $state(false);

  let music = $derived(ui.face === "music");
  let active = $derived.by(() => {
    void ui.nav;
    return activeId(current());
  });
  let prefs = $derived(tubePrefs());
  let main = $derived(music ? MUSIC_MAIN : VIDEO_MAIN.filter((e) => !(e.id === "shorts" && prefs.hide_shorts)));
  let likedOn = $derived(active === "mpl:LM");
  let subs = $derived(allSubs ? session.subs : session.subs.slice(0, SUBS_SHOWN));

  function pick(e: NavEntry) {
    if (e.href) void goto(e.href);
    else if (e.route) (e.root ? goRoot : go)(e.route);
    onnavigate?.();
  }
</script>

{#snippet entry(e: NavEntry)}
  {@const on = active === e.id}
  <button use:agentActions={["click"]} class="it" class:on aria-current={on ? "page" : undefined} onclick={() => pick(e)}>
    <TIcon name={on && e.iconOn ? e.iconOn : e.icon} size={24} />
    <span class="lbl">{$t(e.label)}</span>
  </button>
{/snippet}

<nav class="guide yt-scroll" class:music aria-label={$t("tube.shell.guide") as string}>
  <div class="sec">
    {#each main as e (e.id)}{@render entry(e)}{/each}
  </div>

  {#if !music}
    <div class="sec">
      <button class="head" onclick={() => pick(VIDEO_MAIN[3])}>
        {$t("tube.shell.you")}<TIcon name="chevron-right" size={18} />
      </button>
      {#each VIDEO_YOU as e (e.id)}{@render entry(e)}{/each}
    </div>

    {#if session.subs.length}
      <div class="sec">
        <h3 class="title">{$t("tube.shell.subscriptions")}</h3>
        {#each subs as s (s.channel_id)}
          {@const on = active === `ch:${s.channel_id}`}
          <button class="it ch" class:on onclick={() => (go({ name: "channel", id: s.channel_id }), onnavigate?.())}>
            {#if s.avatar}
              <img class="yt-avatar" src={bestAvatar(s.avatar, 24)} alt="" width="24" height="24" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
            {:else}
              <span class="ini">{s.name.charAt(0).toUpperCase()}</span>
            {/if}
            <span class="lbl">{s.name}</span>
          </button>
        {/each}
        {#if session.subs.length > SUBS_SHOWN}
          <button class="it" onclick={() => (allSubs = !allSubs)} aria-expanded={allSubs}>
            <TIcon name={allSubs ? "chevron-up" : "chevron-down"} size={24} />
            <span class="lbl">{allSubs ? $t("tube.shell.show_less") : $t("tube.shell.show_more")}</span>
          </button>
        {/if}
      </div>
    {/if}
  {:else if session.account?.logged_in}
    <div class="sec">
      <button class="it" class:on={likedOn} onclick={() => (go({ name: "m_playlist", id: "LM" }), onnavigate?.())}>
        <TIcon name={likedOn ? "heart-fill" : "heart"} size={24} />
        <span class="lbl">{$t("tube.shell.liked_music")}</span>
      </button>
      {#each session.musicPlaylists as p (p.id)}
        {@const on = active === `mpl:${p.id}`}
        <button class="it pl" class:on onclick={() => (go({ name: "m_playlist", id: p.id }), onnavigate?.())}>
          <img src={thumbUrl(p.thumbs, 24)} alt="" width="24" height="24" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
          <span class="two">
            <span class="lbl">{p.title}</span>
            {#if p.channel?.name}<span class="own">{p.channel.name}</span>{/if}
          </span>
        </button>
      {/each}
    </div>
  {/if}

  <div class="sec">
    {#each FOOT as e (e.id)}{@render entry(e)}{/each}
  </div>
  <p class="fine">{$t("tube.shell.footer")}</p>
</nav>

<style>
  .guide {
    width: var(--yt-guide-w);
    height: 100%;
    padding: 12px 0 16px;
    overflow-y: auto;
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
  }
  .sec {
    display: flex;
    flex-direction: column;
    padding: 0 12px 12px;
    margin-bottom: 12px;
    border-bottom: 1px solid var(--yt-outline);
  }
  .sec:last-of-type {
    border-bottom: 0;
  }
  .it {
    display: flex;
    align-items: center;
    gap: 24px;
    width: 100%;
    min-height: 40px;
    padding: 0 12px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: var(--yt-text);
    font: var(--yt-type-meta);
    text-align: left;
    cursor: pointer;
    transition: background var(--yt-dur-fast);
  }
  .it:hover {
    background: var(--yt-hover);
  }
  .it:active {
    background: var(--yt-press);
  }
  .it.on {
    background: var(--yt-tonal);
    font-weight: 500;
  }
  .it.on:hover {
    background: var(--yt-tonal-hover);
  }
  .it:focus-visible,
  .head:focus-visible {
    outline: 2px solid var(--yt-link);
    outline-offset: -2px;
  }
  .lbl {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ch img,
  .ini,
  .pl img {
    width: 24px;
    height: 24px;
    flex: none;
  }
  .pl img {
    border-radius: 4px;
    object-fit: cover;
  }
  .pl {
    min-height: 48px;
  }
  .two {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .own {
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ini {
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--yt-tonal-hover);
    font: 500 12px/1 var(--yt-font);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 12px;
    border: 0;
    border-radius: 10px;
    background: none;
    color: var(--yt-text);
    font: 500 16px/22px var(--yt-font);
    cursor: pointer;
  }
  .head:hover {
    background: var(--yt-hover);
  }
  .title {
    margin: 0;
    padding: 6px 12px 4px;
    font: 500 16px/22px var(--yt-font);
  }
  .fine {
    margin: 0;
    padding: 0 24px;
    font: 400 12px/18px var(--yt-font);
    color: var(--yt-text-disabled);
  }
  .music .it {
    min-height: 48px;
    border-radius: 8px;
  }
  .music .it.on {
    background: var(--ytm-chip);
  }
  .music .it:hover {
    background: var(--ytm-chip-hover);
  }
  .music .sec {
    border-color: var(--ytm-divider);
  }
</style>
