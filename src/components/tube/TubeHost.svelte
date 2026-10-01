<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { player as localMusic } from "$lib/music/player-store.svelte";
  import { registerSemantic } from "$lib/native-agent/semantic";
  import { shellLayout } from "$lib/stores/shell-layout.svelte";
  import { yieldAll } from "$lib/tube/media-session";
  import { attachAudio, currentSong, detachAudio, mplayer } from "$lib/tube/music.svelte";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import { vplayer } from "$lib/tube/video.svelte";
  import "./tube.css";
  import MiniVideo from "./music/MiniVideo.svelte";
  import PlayerBar from "./music/PlayerBar.svelte";
  import { look, watchScheme } from "./shell/look.svelte";

  const BAR_INSET = 88;
  const MINI_INSET = 0;

  let audio = $state<HTMLAudioElement>();

  onMount(() => {
    watchScheme();
    if (audio) attachAudio(audio);
    return () => {
      if (audio) detachAudio(audio);
    };
  });

  $effect(() => {
    if (localMusic.playing) yieldAll();
  });

  let onTube = $derived(page.url.pathname.startsWith("/youtube"));
  let song = $derived(currentSong());
  let showMini = $derived(!!vplayer.videoId && vplayer.mini && (vplayer.active || !song));
  let showBar = $derived(!onTube && !!song && (mplayer.playing || mplayer.position > 0) && !showMini);
  let face = $derived(showBar ? "music" : "video");
  let prefs = $derived(tubePrefs());
  let scheme = $derived(face === "music" && prefs.look === "classic" ? "dark" : look.scheme);

  let localMiniShown = $derived(!page.url.pathname.startsWith("/music") && !!localMusic.current && (localMusic.playing || localMusic.position > 0));

  let reserved = false;
  $effect(() => {
    const need = showBar ? BAR_INSET : MINI_INSET;
    if (need > 0) {
      reserved = true;
      if (shellLayout.bottomInset < need) shellLayout.bottomInset = need;
    } else if (reserved) {
      reserved = false;
      if (!localMiniShown) shellLayout.bottomInset = 0;
    }
  });

  onMount(() =>
    registerSemantic("tube-player", () => ({
      ready: true,
      video: vplayer.videoId
        ? { id: vplayer.videoId, playing: vplayer.playing, mini: vplayer.mini, position: Math.round(vplayer.position), duration: Math.round(vplayer.duration), error: vplayer.error }
        : null,
      music: song ? { id: song.id, title: song.title, artist: song.artists.map((a) => a.name).join(", "), playing: mplayer.playing, error: mplayer.error, queueLength: mplayer.queue.length, queueIndex: mplayer.index } : null,
    })),
  );
</script>

<audio bind:this={audio} preload="auto" hidden></audio>

{#if showMini || showBar}
  <div class="omnitube tube-host" data-look={prefs.look} data-scheme={scheme} data-face={face} style:--overlay-inset="{shellLayout.overlayBottomInset}px">
    {#if showMini}<MiniVideo />{/if}
    {#if showBar}<PlayerBar floating />{/if}
  </div>
{/if}

<style>
  .tube-host {
    position: fixed;
    inset: 0;
    z-index: 300;
    pointer-events: none;
    background: transparent;
  }
</style>
