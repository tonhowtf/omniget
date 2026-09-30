<script lang="ts">
  // Artist page: full-width photo fading into the background, huge name,
  // subscribers, Shuffle / Radio / Subscribe, then the artist shelves.
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import { thumbUrl } from "$lib/tube/format";
  import { playPlaylist, playRadio } from "$lib/tube/music.svelte";
  import type { Route } from "$lib/tube/ui.svelte";
  import type { ArtistPage } from "$lib/tube/types";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import MusicShelf from "../../music/MusicShelf.svelte";
  import TIcon from "../../TIcon.svelte";
  import MusicPage from "./MusicPage.svelte";
  import MusicSkeleton from "./MusicSkeleton.svelte";
  import MusicState from "./MusicState.svelte";
  import { copyLink, songFromItem } from "./shared.svelte";

  let { route }: { route: Extract<Route, { name: "m_artist" }> } = $props();

  let artist = $state<ArtistPage | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let subBusy = $state(false);
  let descOpen = $state(false);
  let seq = 0;

  let photo = $derived(artist ? thumbUrl(artist.thumbs, 1440) : "");
  let firstSong = $derived.by(() => {
    for (const s of artist?.shelves ?? []) for (const it of s.items) {
      const song = songFromItem(it);
      if (song) return song;
    }
    return null;
  });

  async function load() {
    const my = ++seq;
    loading = true;
    error = null;
    descOpen = false;
    try {
      const res = await api.musicArtist(route.id);
      if (my === seq) artist = res;
    } catch (e) {
      if (my === seq) error = String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  $effect(() => {
    void route.id;
    void load();
  });

  function shuffle() {
    if (!artist) return;
    if (artist.shuffle_playlist_id) void playPlaylist(artist.shuffle_playlist_id, true);
    else if (artist.radio_playlist_id) void playRadio(firstSong?.id ?? "", artist.radio_playlist_id);
  }

  function radio() {
    if (!artist) return;
    if (artist.radio_playlist_id) void playRadio(firstSong?.id ?? "", artist.radio_playlist_id);
    else if (firstSong) void playRadio(firstSong.id);
  }

  async function subscribe() {
    if (!artist || subBusy) return;
    subBusy = true;
    const next = !artist.subscribed;
    try {
      await api.action(next ? "subscribe" : "unsubscribe", {
        channel_id: artist.channel_id ?? artist.id,
        channel_name: artist.name,
        avatar: artist.thumbs[0]?.url ?? null,
      });
      artist.subscribed = next;
    } catch (e) {
      showToast("error", String(e));
    } finally {
      subBusy = false;
    }
  }
</script>

<MusicPage readiness={loading ? "loading" : error ? "error" : "ready"} label={artist?.name ?? ($t("tube.music.kind_artist") as string)}>
  {#snippet hero()}
    {#if artist && !loading}
      <div class="ar-hero" class:no-photo={!photo}>
        {#if photo}<img class="ar-photo" src={photo} alt="" decoding="async" referrerpolicy="no-referrer" />{/if}
        <div class="ar-fade" aria-hidden="true"></div>
        <div class="ar-hero-inner">
          <h1 class="ar-name">{artist.name}</h1>
          {#if artist.description}
            <p class="ar-desc" class:open={descOpen}>{artist.description}</p>
            {#if artist.description.length > 180}
              <button class="ar-desc-more" onclick={() => (descOpen = !descOpen)}>{descOpen ? $t("tube.music.less") : $t("tube.music.more_text")}</button>
            {/if}
          {/if}
          <div class="ar-actions">
            {#if artist.shuffle_playlist_id || artist.radio_playlist_id}
              <button class="yt-btn ar-shuffle" onclick={shuffle}><TIcon name="shuffle" size={20} />{$t("tube.music.shuffle")}</button>
            {/if}
            {#if artist.radio_playlist_id || firstSong}
              <button class="yt-btn ar-radio" onclick={radio}><TIcon name="radio" size={20} />{$t("tube.music.radio")}</button>
            {/if}
            <button class="yt-btn ar-sub" class:is-on={artist.subscribed} onclick={subscribe} disabled={subBusy} aria-pressed={!!artist.subscribed}>
              {artist.subscribed ? $t("tube.music.subscribed") : $t("tube.music.subscribe")}
              {#if artist.subscribers_text}<span class="ar-subs">{artist.subscribers_text}</span>{/if}
            </button>
            <button class="ar-icon" onclick={() => artist && copyLink(`https://music.youtube.com/channel/${artist.id}`)} aria-label={$t("tube.music.copy_link") as string} title={$t("tube.music.copy_link") as string}><TIcon name="share-arrow" size={20} /></button>
          </div>
        </div>
      </div>
    {/if}
  {/snippet}

  {#if loading}
    <MusicSkeleton variant="artist" />
  {:else if error || !artist}
    <MusicState icon="radio" error title={$t("tube.music.error_title") as string} text={error ?? ""} onretry={load} />
  {:else if !artist.shelves.length}
    <MusicState icon="radio" title={$t("tube.music.artist_empty") as string} />
  {:else}
    {#each artist.shelves as shelf, i (shelf.id + i)}
      <MusicShelf {shelf} listLimit={shelf.kind === "list" ? 5 : 0} />
    {/each}
  {/if}
</MusicPage>

<style>
  .ar-hero {
    position: relative;
    height: clamp(320px, 42cqi, 560px);
    display: flex; align-items: flex-end;
    overflow: hidden;
    margin-bottom: 8px;
  }
  .ar-hero.no-photo {
    height: auto; min-height: 240px;
    background: radial-gradient(ellipse at 20% 0%, color-mix(in srgb, var(--yt-accent) 25%, transparent), transparent 60%);
  }
  .ar-photo { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; object-position: center 25%; }
  .ar-fade {
    position: absolute; inset: 0;
    background:
      linear-gradient(to bottom, rgba(0, 0, 0, .35) 0%, transparent 30%, transparent 45%, var(--ytm-bg, var(--yt-bg)) 100%),
      linear-gradient(to right, color-mix(in srgb, var(--ytm-bg, #000) 55%, transparent), transparent 65%);
  }
  .ar-hero-inner {
    position: relative; width: 100%;
    max-width: calc(1478px + 2 * var(--ytm-pad));
    margin: 0 auto; padding: 0 var(--ytm-pad) 24px;
  }
  .ar-name {
    margin: 0 0 12px;
    font: 700 clamp(40px, 7cqi, 88px)/1 var(--yt-font-display);
    letter-spacing: -.02em;
    color: #fff; text-shadow: 0 2px 24px rgba(0, 0, 0, .35);
    word-break: break-word;
  }
  .ar-hero.no-photo .ar-name { color: var(--ytm-text, var(--yt-text)); text-shadow: none; }
  .ar-desc {
    margin: 0; max-width: 640px; font: var(--yt-type-meta); color: rgba(255, 255, 255, .8);
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
  }
  .ar-hero.no-photo .ar-desc { color: var(--ytm-text-2, var(--yt-text-2)); }
  .ar-desc.open { display: block; }
  .ar-desc-more { border: 0; padding: 0; margin-top: 2px; background: none; font: 500 14px/20px var(--yt-font); color: var(--ytm-text, #fff); cursor: pointer; }
  .ar-desc-more:hover { text-decoration: underline; }
  .ar-actions { display: flex; align-items: center; gap: 12px; margin-top: 20px; flex-wrap: wrap; }
  .ar-shuffle { background: #fff; color: #000; }
  .ar-shuffle:hover { background: #e5e5e5; }
  .ar-radio, .ar-sub {
    background: rgba(255, 255, 255, .1); color: #fff;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, .2); backdrop-filter: blur(8px);
  }
  .ar-hero.no-photo .ar-radio, .ar-hero.no-photo .ar-sub { color: var(--ytm-text, var(--yt-text)); }
  .ar-radio:hover, .ar-sub:hover { background: rgba(255, 255, 255, .2); }
  .ar-sub.is-on { background: rgba(255, 255, 255, .05); color: var(--ytm-text-2, #aaa); }
  .ar-subs { margin-left: 4px; font-weight: 400; opacity: .75; }
  .ar-icon {
    display: grid; place-items: center; width: 36px; height: 36px; border-radius: 50%;
    border: 0; background: rgba(255, 255, 255, .1); color: #fff; cursor: pointer;
  }
  .ar-icon:hover { background: rgba(255, 255, 255, .2); }
  .ar-icon:focus-visible, .ar-desc-more:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }
</style>
