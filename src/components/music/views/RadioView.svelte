<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { player, playStation, toggle } from "$lib/music/player-store.svelte";
  import { go } from "$lib/music/ui-store.svelte";
  import type { RbStation, Station } from "$lib/music/types";
  import EmptyState from "../EmptyState.svelte";
  import MusicIcon from "../MusicIcon.svelte";
  import StationIcon from "../StationIcon.svelte";
  import Visualizer from "../Visualizer.svelte";

  type Tab = "favorites" | "discover" | "url";
  let tab = $state<Tab>("discover");
  let favorites = $state<Station[]>([]);
  let results = $state<RbStation[]>([]);
  let tags = $state<{ name: string; stationcount: number }[]>([]);
  let q = $state("");
  let tag = $state("");
  let country = $state("");
  let loading = $state(false);
  let error = $state("");
  let customName = $state("");
  let customUrl = $state("");

  const COUNTRIES = ["", "BR", "US", "GB", "DE", "FR", "ES", "PT", "IT", "NL", "JP", "KR", "MX", "AR", "CA", "AU", "RU", "IN", "PL", "SE"];

  onMount(async () => {
    await loadFavorites();
    if (favorites.length) tab = "favorites";
    void search();
    tags = await invoke<{ name: string; stationcount: number }[]>("music_radio_tags").catch(() => []);
  });

  async function loadFavorites() {
    favorites = await invoke<Station[]>("music_radio_stations").catch(() => []);
  }

  async function search() {
    loading = true;
    error = "";
    try {
      results = await invoke<RbStation[]>("music_radio_search", {
        query: { name: q, tag, countrycode: country, order: q || tag ? "votes" : "clickcount", limit: 80 },
      });
    } catch (e) {
      error = String(e);
      results = [];
    } finally {
      loading = false;
    }
  }

  function toStation(s: RbStation): Station {
    return {
      id: s.stationuuid,
      name: s.name.trim(),
      url: s.url_resolved || s.url,
      homepage: s.homepage,
      favicon: s.favicon,
      tags: s.tags,
      country: s.country,
      codec: s.codec,
      bitrate: s.bitrate,
      hls: s.hls === 1,
    };
  }

  let favIds = $derived(new Set(favorites.map((f) => f.id)));

  async function toggleFav(st: Station) {
    if (favIds.has(st.id)) await invoke("music_radio_remove", { id: st.id });
    else await invoke("music_radio_save", { station: st });
    await loadFavorites();
  }

  async function addCustom() {
    const url = customUrl.trim();
    if (!/^https?:\/\//i.test(url)) {
      showToast("error", $t("music.radio.bad_url") as string);
      return;
    }
    const st: Station = {
      id: `url:${url}`,
      name: customName.trim() || url,
      url,
      homepage: "",
      favicon: "",
      tags: "",
      country: "",
      codec: "",
      bitrate: 0,
      hls: /\.m3u8(\?|$)/i.test(url),
    };
    await invoke("music_radio_save", { station: st });
    customName = "";
    customUrl = "";
    await loadFavorites();
    tab = "favorites";
    void playStation(st);
  }

  function isOn(st: Station) {
    return player.radio?.id === st.id;
  }

  function play(st: Station) {
    if (isOn(st)) void toggle();
    else void playStation(st);
  }

  function findSong() {
    const m = player.radioMeta;
    const text = [m.artist, m.title].filter(Boolean).join(" ");
    if (text) go({ kind: "download", q: text });
  }
</script>

<div class="rv">
  <header>
    <h1>{$t("music.nav.radio")}</h1>
    <div class="mac-segmented">
      <button class="mac-segmented-btn" class:active={tab === "favorites"} onclick={() => (tab = "favorites")}>{$t("music.radio.favorites")} {favorites.length ? `(${favorites.length})` : ""}</button>
      <button class="mac-segmented-btn" class:active={tab === "discover"} onclick={() => (tab = "discover")}>{$t("music.radio.discover")}</button>
      <button class="mac-segmented-btn" class:active={tab === "url"} onclick={() => (tab = "url")}>{$t("music.radio.custom")}</button>
    </div>
  </header>

  {#if player.radio}
    <section class="now">
      <div class="now-viz"><Visualizer mode="bars" bars={72} /></div>
      <div class="now-body">
        <StationIcon src={player.radio.favicon} name={player.radio.name} size={72} radius={12} />
        <div class="now-meta">
          <span class="kick"><span class="live">{$t("music.radio.live")}</span> {player.radio.name}</span>
          <span class="song">{player.radioMeta.title || (player.buffering ? $t("music.radio.connecting") : $t("music.radio.no_title"))}</span>
          {#if player.radioMeta.artist}<span class="art">{player.radioMeta.artist}</span>{/if}
          {#if player.error.startsWith("radio_failed")}<span class="err">{$t("music.radio.failed")}</span>{/if}
        </div>
        <div class="now-acts">
          <button class="btn btn-primary" onclick={() => void toggle()}><MusicIcon name={player.playing ? "pause" : "play"} size={14} /> {$t(player.playing ? "music.pause" : "music.play")}</button>
          {#if player.radioMeta.title}
            <button class="btn" onclick={findSong}><MusicIcon name="download" size={14} /> {$t("music.radio.get_song")}</button>
          {/if}
        </div>
      </div>
    </section>
  {/if}

  {#if tab === "discover"}
    <form
      class="filters"
      onsubmit={(e) => {
        e.preventDefault();
        void search();
      }}
    >
      <label class="find">
        <MusicIcon name="search" size={14} />
        <input class="input input-search" bind:value={q} placeholder={$t("music.radio.search_placeholder") as string} />
      </label>
      <select class="input sel" bind:value={country} onchange={search} aria-label={$t("music.radio.country") as string}>
        {#each COUNTRIES as c (c)}<option value={c}>{c || $t("music.radio.any_country")}</option>{/each}
      </select>
      <button class="btn" type="submit" disabled={loading}>{#if loading}<MusicIcon name="loader" size={13} spin />{/if}{$t("music.search.go")}</button>
    </form>
    {#if tags.length}
      <div class="tags">
        <button class="btn btn-pill btn-sm" class:active={!tag} onclick={() => { tag = ""; void search(); }}>{$t("music.radio.popular")}</button>
        {#each tags.slice(0, 28) as tg (tg.name)}
          <button class="btn btn-pill btn-sm" class:active={tag === tg.name} onclick={() => { tag = tg.name; void search(); }}>{tg.name}</button>
        {/each}
      </div>
    {/if}
    {#if error}
      <p class="err">{error}</p>
    {/if}
    <div class="grid">
      {#each results as r (r.stationuuid)}
        {@const st = toStation(r)}
        <div class="st" class:on={isOn(st)}>
          <button class="st-play" onclick={() => play(st)} aria-label={$t("music.play") as string}>
            <StationIcon src={r.favicon} name={st.name} />
            <span class="ov"><MusicIcon name={isOn(st) && player.playing ? "pause" : "play"} size={16} /></span>
          </button>
          <div class="st-meta">
            <span class="n">{st.name}</span>
            <span class="d">{[r.countrycode, r.codec, r.bitrate ? `${r.bitrate} kbps` : "", r.tags.split(",").slice(0, 2).join(", ")].filter(Boolean).join(" · ")}</span>
          </div>
          <button class="fv" class:on={favIds.has(st.id)} onclick={() => void toggleFav(st)} aria-label={$t(favIds.has(st.id) ? "music.radio.unfavorite" : "music.radio.favorite") as string}>
            <MusicIcon name={favIds.has(st.id) ? "star-fill" : "star"} size={15} />
          </button>
        </div>
      {:else}
        {#if !loading}<p class="dim">{$t("music.radio.no_results")}</p>{/if}
      {/each}
    </div>
  {:else if tab === "favorites"}
    {#if !favorites.length}
      <EmptyState art="radio" title={$t("music.radio.empty_title") as string} body={$t("music.radio.empty_body") as string} compact>
        <button class="btn btn-primary" onclick={() => (tab = "discover")}>{$t("music.radio.discover")}</button>
      </EmptyState>
    {:else}
      <div class="grid">
        {#each favorites as st (st.id)}
          <div class="st" class:on={isOn(st)}>
            <button class="st-play" onclick={() => play(st)} aria-label={$t("music.play") as string}>
              <StationIcon src={st.favicon} name={st.name} />
              <span class="ov"><MusicIcon name={isOn(st) && player.playing ? "pause" : "play"} size={16} /></span>
            </button>
            <div class="st-meta">
              <span class="n">{st.name}</span>
              <span class="d">{[st.country, st.codec, st.bitrate ? `${st.bitrate} kbps` : "", st.play_count ? $t("music.radio.plays", { count: st.play_count }) : ""].filter(Boolean).join(" · ")}</span>
            </div>
            <button class="fv on" onclick={() => void toggleFav(st)} aria-label={$t("music.radio.unfavorite") as string}><MusicIcon name="star-fill" size={15} /></button>
          </div>
        {/each}
      </div>
    {/if}
  {:else}
    <form
      class="custom"
      onsubmit={(e) => {
        e.preventDefault();
        void addCustom();
      }}
    >
      <p class="dim">{$t("music.radio.custom_hint")}</p>
      <input class="input" bind:value={customName} placeholder={$t("music.radio.custom_name") as string} />
      <input class="input" bind:value={customUrl} placeholder="https://stream.example.com/live.mp3" />
      <button class="btn btn-primary" type="submit"><MusicIcon name="play" size={14} /> {$t("music.radio.add_play")}</button>
    </form>
  {/if}
</div>

<style>
  .rv {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding-bottom: var(--space-7);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  .now {
    position: relative;
    overflow: hidden;
    border-radius: var(--radius-xl);
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 22%, var(--surface)), var(--surface));
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .now-viz {
    position: absolute;
    inset: auto 0 0 0;
    height: 60%;
    opacity: 0.35;
  }
  .now-body {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-4);
    flex-wrap: wrap;
  }
  .now-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 200px;
  }
  .kick {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .song {
    font-size: var(--text-xl);
    font-weight: 700;
  }
  .art {
    color: var(--text-dim);
  }
  .now-acts {
    display: flex;
    gap: var(--space-2);
  }
  .live {
    display: inline-block;
    padding: 0 5px;
    border-radius: 3px;
    background: #e5484d;
    color: white;
    font-size: var(--text-caption);
    font-weight: 700;
    letter-spacing: 0.05em;
  }
  .filters {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    align-items: center;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1 1 260px;
    color: var(--text-muted);
  }
  .find input {
    flex: 1;
  }
  .sel {
    width: auto;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-2);
  }
  .st {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 8px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .st:hover {
    background: var(--fill-2);
  }
  .st.on {
    background: var(--accent-soft);
  }
  .st-play {
    all: unset;
    position: relative;
    cursor: pointer;
    border-radius: 8px;
  }
  .st-play:focus-visible {
    outline: var(--focus-ring);
  }
  .ov {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    border-radius: 8px;
    background: rgba(0, 0, 0, 0.45);
    color: white;
    opacity: 0;
    transition: opacity var(--duration-fast);
  }
  .st:hover .ov,
  .st.on .ov,
  .st-play:focus-visible .ov {
    opacity: 1;
  }
  .st-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .n {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .d {
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fv {
    all: unset;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    color: var(--text-muted);
    cursor: pointer;
  }
  .fv.on {
    color: #ffb340;
  }
  .fv:focus-visible {
    outline: var(--focus-ring);
  }
  .custom {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 520px;
  }
  .dim {
    color: var(--text-dim);
  }
  .err {
    color: var(--danger);
    font-size: var(--text-sm);
  }
</style>
