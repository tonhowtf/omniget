<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { getDownloads } from "$lib/stores/download-store.svelte";
  import { musicDownloads, syncMusicDownloads } from "$lib/music/downloads.svelte";
  import { library, loadLibrary } from "$lib/music/library-store.svelte";
  import { hydrateCurrent, initPlayer, player, syncCurrent } from "$lib/music/player-store.svelte";
  import { shellLayout } from "$lib/stores/shell-layout.svelte";
  import { registerSemantic } from "$lib/native-agent/semantic";
  import MiniPlayer from "./MiniPlayer.svelte";

  const MINI_PLAYER_RESERVED_INSET = 80;
  const NATIVE_ACTIVITY_RESERVED_INSET = 56;
  const STACKED_OVERLAYS_RESERVED_INSET = 120;

  onMount(() =>
    registerSemantic("music-player", () => ({
      ready: true,
      playing: player.playing,
      buffering: player.buffering,
      error: player.error || null,
      track: player.current ? { id: player.current.id, title: player.current.title, artist: player.current.artist } : null,
      radio: player.radio ? { name: player.radio.name, nowPlaying: player.radioMeta.title || null, artist: player.radioMeta.artist || null } : null,
      position: Math.round(player.position * 10) / 10,
      duration: Math.round(player.duration * 10) / 10,
      queueLength: player.queue.length,
      queueIndex: player.index,
    })),
  );

  onMount(() => {
    initPlayer();
    if (player.queue.length || musicDownloads.items.some((x) => x.state === "running")) {
      const id = setTimeout(async () => {
        await loadLibrary();
        hydrateCurrent();
      }, 2500);
      return () => clearTimeout(id);
    }
  });

  onMount(() => () => { shellLayout.bottomInset = 0; });

  $effect(() => {
    const map = getDownloads();
    if (musicDownloads.items.some((x) => x.state === "running")) syncMusicDownloads(map);
  });

  $effect(() => {
    void library().version;
    syncCurrent();
  });

  let onMusic = $derived(page.url.pathname.startsWith("/music"));
  let show = $derived(!onMusic && (!!player.radio || (!!player.current && (player.playing || player.position > 0))));

  $effect(() => {
    shellLayout.bottomInset = shellLayout.overlayBottomInset
      ? (show ? STACKED_OVERLAYS_RESERVED_INSET : NATIVE_ACTIVITY_RESERVED_INSET)
      : (show ? MINI_PLAYER_RESERVED_INSET : 0);
  });
</script>

{#if show}<MiniPlayer />{/if}
