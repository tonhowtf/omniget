<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { fmtTime, lineAt } from "$lib/music/library";
  import { patchTracksLocal } from "$lib/music/library-store.svelte";
  import { plainLines, toLrc } from "$lib/music/lrc";
  import { player, seek } from "$lib/music/player-store.svelte";
  import type { LrclibRecord, LyricsPayload, Track } from "$lib/music/types";
  import MusicIcon from "./MusicIcon.svelte";

  let { big = false }: { big?: boolean } = $props();

  type Mode = "view" | "search" | "edit" | "sync";

  let data = $state<LyricsPayload | null>(null);
  let loading = $state(false);
  let mode = $state<Mode>("view");
  let offset = $state(0);
  let results = $state<LrclibRecord[]>([]);
  let searching = $state(false);
  let preview = $state<LrclibRecord | null>(null);
  let qArtist = $state("");
  let qTitle = $state("");
  let draft = $state("");
  let target = $state<"sidecar" | "tag" | "cache">("sidecar");
  let syncLines = $state<string[]>([]);
  let syncStamps = $state<number[]>([]);
  let follow = $state(true);
  let scroller: HTMLDivElement | undefined = $state();
  let loadedFor = $state<number | null>(null);

  let track = $derived(player.current);
  let lines = $derived(data?.lyrics.lines ?? []);
  let posMs = $derived(player.position * 1000 + offset);
  let idx = $derived(lineAt(lines, posMs));

  $effect(() => {
    const tr = track;
    if (!tr || tr.id === loadedFor) return;
    loadedFor = tr.id;
    void load(tr);
  });

  async function load(tr: Track) {
    loading = true;
    mode = "view";
    offset = 0;
    results = [];
    preview = null;
    qArtist = tr.artist;
    qTitle = tr.title;
    try {
      data = await invoke<LyricsPayload>("music_lyrics_get", { id: tr.id });
      // nothing local: look it up right away, it's what the user wants
      if (data.source === "none") void search(true);
    } catch {
      data = null;
    } finally {
      loading = false;
    }
  }

  async function search(auto = false) {
    if (!track) return;
    searching = true;
    try {
      results = await invoke<LrclibRecord[]>("music_lyrics_search", { id: track.id, artist: qArtist, title: qTitle });
      if (!auto) mode = "search";
      else if (results.length) {
        // an exact synced match on the first try is applied directly
        const best = results[0];
        const exact =
          best.syncedLyrics &&
          best.duration != null &&
          Math.abs(best.duration - track.duration_ms / 1000) <= 2 &&
          best.trackName.toLowerCase() === track.title.toLowerCase();
        if (exact) await apply(best);
        else mode = "search";
      }
    } catch (e) {
      if (!auto) showToast("error", String(e));
    } finally {
      searching = false;
    }
  }

  async function saveText(text: string) {
    if (!track) return;
    try {
      data = await invoke<LyricsPayload>("music_lyrics_save", { id: track.id, text, target });
      patchTracksLocal([track.id], { has_lyrics: text.trim().length > 0 });
      mode = "view";
      offset = 0;
      showToast("success", $t(`music.lyrics.saved_${data.source}`) as string);
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function apply(r: LrclibRecord) {
    const text = r.syncedLyrics || r.plainLyrics || "";
    if (!text) return;
    await saveText(text);
  }

  function saveOffset() {
    if (!data || !track) return;
    void saveText(toLrc(lines, { title: track.title, artist: track.artist, album: track.album }, -offset));
  }

  function startEdit() {
    draft = data?.raw ?? "";
    mode = "edit";
  }

  function startSync() {
    const source = data?.lyrics.plain ?? (lines.length ? lines.map((l) => l.text).join("\n") : draft);
    syncLines = plainLines(source || draft);
    syncStamps = [];
    mode = "sync";
    void seek(0);
  }

  function stampNext() {
    if (syncStamps.length >= syncLines.length) return;
    syncStamps = [...syncStamps, Math.round(player.position * 1000)];
  }

  function undoStamp() {
    syncStamps = syncStamps.slice(0, -1);
  }

  function finishSync() {
    if (!track) return;
    const ls = syncLines.slice(0, syncStamps.length).map((text, i) => ({ time_ms: syncStamps[i], text }));
    void saveText(toLrc(ls, { title: track.title, artist: track.artist, album: track.album }));
  }

  function syncKey(e: KeyboardEvent) {
    if (mode !== "sync") return;
    if (e.code === "Space" || e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      stampNext();
    } else if (e.key === "Backspace") {
      e.preventDefault();
      undoStamp();
    }
  }

  $effect(() => {
    void idx;
    if (!follow || mode !== "view" || !scroller) return;
    const el = scroller.querySelector<HTMLElement>(`[data-i="${idx}"]`);
    if (el) {
      const top = el.offsetTop - scroller.clientHeight / 2 + el.clientHeight / 2;
      scroller.scrollTo({ top, behavior: "smooth" });
    }
  });

  function wordProgress(i: number, wi: number): boolean {
    const w = lines[i]?.words?.[wi];
    return !!w && w.time_ms <= posMs;
  }
</script>

<svelte:window onkeydown={syncKey} />

<div class="lp" class:big>
  {#if !track}
    <div class="center dim">
      {#if player.radio}
        <MusicIcon name="radio" size={28} />
        <p>{player.radioMeta.title ? `${player.radioMeta.artist ?? ""} ${player.radioMeta.artist ? "·" : ""} ${player.radioMeta.title}` : player.radio.name}</p>
        <p class="small">{$t("music.lyrics.radio_hint")}</p>
      {:else}
        <MusicIcon name="lyrics" size={28} />
        <p>{$t("music.lyrics.nothing")}</p>
      {/if}
    </div>
  {:else if loading}
    <div class="center dim"><MusicIcon name="loader" size={22} spin /></div>
  {:else if mode === "search"}
    <div class="pane">
      <div class="bar">
        <button class="btn btn-sm btn-ghost" onclick={() => (mode = "view")}><MusicIcon name="back" size={13} /> {$t("music.back")}</button>
      </div>
      <form
        class="sform"
        onsubmit={(e) => {
          e.preventDefault();
          void search();
        }}
      >
        <input class="input" bind:value={qArtist} placeholder={$t("music.col.artist") as string} />
        <input class="input" bind:value={qTitle} placeholder={$t("music.col.title") as string} />
        <button class="btn btn-primary btn-sm" type="submit" disabled={searching}>
          {#if searching}<MusicIcon name="loader" size={13} spin />{:else}<MusicIcon name="search" size={13} />{/if}
          {$t("music.lyrics.search")}
        </button>
      </form>
      {#if preview}
        <div class="preview">
          <div class="bar">
            <button class="btn btn-sm btn-ghost" onclick={() => (preview = null)}><MusicIcon name="back" size={13} /></button>
            <span class="grow">{preview.artistName} — {preview.trackName}</span>
            <button class="btn btn-sm btn-primary" onclick={() => preview && void apply(preview)}>{$t("music.lyrics.use")}</button>
          </div>
          <pre>{preview.syncedLyrics || preview.plainLyrics}</pre>
        </div>
      {:else}
        <ul class="results">
          {#each results as r (r.id)}
            <li>
              <button class="res" onclick={() => (preview = r)}>
                <span class="res-t">{r.trackName}</span>
                <span class="res-s">{r.artistName}{r.albumName ? ` · ${r.albumName}` : ""}</span>
                <span class="res-tags">
                  {#if r.syncedLyrics}<span class="tag tag-accent">{$t("music.lyrics.synced")}</span>{:else if r.instrumental}<span class="tag">{$t("music.lyrics.instrumental")}</span>{:else}<span class="tag">{$t("music.lyrics.plain")}</span>{/if}
                  {#if r.duration}<span class="tag" class:tag-warning={Math.abs(r.duration - track.duration_ms / 1000) > 3}>{fmtTime(r.duration * 1000)}</span>{/if}
                </span>
              </button>
            </li>
          {:else}
            <li class="dim small">{searching ? "" : $t("music.lyrics.no_results")}</li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else if mode === "edit"}
    <div class="pane">
      <div class="bar">
        <button class="btn btn-sm btn-ghost" onclick={() => (mode = "view")}><MusicIcon name="back" size={13} /></button>
        <span class="grow small dim">{$t("music.lyrics.edit_hint")}</span>
      </div>
      <textarea class="input ed" bind:value={draft} spellcheck="false"></textarea>
      <div class="bar">
        <select class="input sel" bind:value={target} aria-label={$t("music.lyrics.save_where") as string}>
          <option value="sidecar">{$t("music.lyrics.target_sidecar")}</option>
          <option value="tag">{$t("music.lyrics.target_tag")}</option>
          <option value="cache">{$t("music.lyrics.target_cache")}</option>
        </select>
        <button class="btn btn-sm" onclick={startSync} disabled={!draft.trim()}><MusicIcon name="clock" size={13} /> {$t("music.lyrics.tap_sync")}</button>
        <button class="btn btn-sm btn-primary" onclick={() => void saveText(draft)}>{$t("music.save")}</button>
      </div>
    </div>
  {:else if mode === "sync"}
    <div class="pane">
      <div class="bar">
        <button class="btn btn-sm btn-ghost" onclick={() => (mode = "edit")}><MusicIcon name="back" size={13} /></button>
        <span class="grow small dim">{$t("music.lyrics.sync_hint")}</span>
      </div>
      <ol class="synclist">
        {#each syncLines as l, i (i)}
          <li class:done={i < syncStamps.length} class:next={i === syncStamps.length}>
            <span class="st">{i < syncStamps.length ? fmtTime(syncStamps[i]) : ""}</span>
            <span>{l || "♪"}</span>
          </li>
        {/each}
      </ol>
      <div class="bar">
        <button class="btn btn-sm" onclick={undoStamp} disabled={!syncStamps.length}>{$t("music.lyrics.undo")}</button>
        <button class="btn btn-sm btn-primary grow" onclick={stampNext} disabled={syncStamps.length >= syncLines.length}>{$t("music.lyrics.stamp")} (Space)</button>
        <button class="btn btn-sm" onclick={finishSync} disabled={!syncStamps.length}>{$t("music.save")}</button>
      </div>
    </div>
  {:else}
    <div class="bar top">
      <span class="src small dim">
        {#if data && data.source !== "none"}{$t(`music.lyrics.source_${data.source}`)}{/if}
      </span>
      {#if lines.length}
        <div class="offset" title={$t("music.lyrics.offset") as string}>
          <button class="btn btn-sm btn-ghost" onclick={() => (offset -= 250)} aria-label="-0.25s">−</button>
          <span class="small">{offset > 0 ? "+" : ""}{(offset / 1000).toFixed(2)}s</span>
          <button class="btn btn-sm btn-ghost" onclick={() => (offset += 250)} aria-label="+0.25s">+</button>
          {#if offset !== 0}<button class="btn btn-sm" onclick={saveOffset}>{$t("music.save")}</button>{/if}
        </div>
      {/if}
      <button class="btn btn-sm btn-ghost" onclick={() => void search()} title={$t("music.lyrics.search") as string} aria-label={$t("music.lyrics.search") as string}><MusicIcon name="search" size={14} /></button>
      <button class="btn btn-sm btn-ghost" onclick={startEdit} title={$t("music.lyrics.edit") as string} aria-label={$t("music.lyrics.edit") as string}><MusicIcon name="edit" size={14} /></button>
    </div>
    {#if lines.length}
      <div class="lines" role="group" aria-label={$t("music.lyrics.title") as string} bind:this={scroller} onwheel={() => (follow = false)} onpointerdown={() => (follow = false)}>
        <div class="pad"></div>
        {#each lines as l, i (i)}
          <button class="ln" class:on={i === idx} class:past={i < idx} data-i={i} onclick={() => { follow = true; void seek((l.time_ms - offset) / 1000); }}>
            {#if l.words?.length && i === idx}
              {#each l.words as w, wi (wi)}<span class="w" class:lit={wordProgress(i, wi)}>{w.text}</span>{/each}
            {:else}
              {l.text || "♪"}
            {/if}
          </button>
        {/each}
        <div class="pad"></div>
      </div>
      {#if !follow}
        <button class="btn btn-sm resume" onclick={() => (follow = true)}><MusicIcon name="lyrics" size={13} /> {$t("music.lyrics.follow")}</button>
      {/if}
    {:else if data?.lyrics.plain}
      <pre class="plain">{data.lyrics.plain}</pre>
      <div class="bar"><button class="btn btn-sm" onclick={startSync}><MusicIcon name="clock" size={13} /> {$t("music.lyrics.tap_sync")}</button></div>
    {:else}
      <div class="center dim">
        <MusicIcon name="lyrics" size={28} />
        <p>{searching ? $t("music.lyrics.searching") : $t("music.lyrics.none")}</p>
        <div class="row">
          <button class="btn btn-sm btn-primary" onclick={() => void search()} disabled={searching}>{$t("music.lyrics.search")}</button>
          <button class="btn btn-sm" onclick={startEdit}>{$t("music.lyrics.write")}</button>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .lp {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .center {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-5);
    text-align: center;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--text-xs);
  }
  .row {
    display: flex;
    gap: var(--space-2);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: var(--space-2);
  }
  .bar.top {
    border-bottom: var(--hairline) solid var(--content-border);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .src {
    flex: 1;
  }
  .offset {
    display: flex;
    align-items: center;
    gap: 2px;
    font-variant-numeric: tabular-nums;
  }
  .lines {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--space-4);
    mask-image: linear-gradient(to bottom, transparent, black 12%, black 88%, transparent);
  }
  .pad {
    height: 40%;
  }
  .ln {
    all: unset;
    display: block;
    width: 100%;
    padding: 6px 0;
    font-size: var(--text-lg);
    font-weight: 650;
    line-height: 1.3;
    color: var(--text-muted);
    opacity: 0.55;
    cursor: pointer;
    transition:
      opacity var(--duration-base, 0.2s),
      color var(--duration-base, 0.2s),
      transform var(--duration-base, 0.2s);
    transform-origin: left center;
  }
  .big .ln {
    font-size: var(--text-2xl);
    padding: 8px 0;
  }
  .ln.past {
    opacity: 0.35;
  }
  .ln.on {
    color: var(--text);
    opacity: 1;
    transform: scale(1.02);
  }
  .ln:hover {
    opacity: 0.9;
  }
  .ln:focus-visible {
    outline: var(--focus-ring);
  }
  .w {
    color: var(--text-muted);
    transition: color 0.12s;
  }
  .w.lit {
    color: var(--accent-hi);
  }
  .resume {
    position: absolute;
    bottom: var(--space-3);
    left: 50%;
    transform: translateX(-50%);
  }
  .plain,
  pre {
    flex: 1;
    min-height: 0;
    overflow: auto;
    margin: 0;
    padding: var(--space-3) var(--space-4);
    white-space: pre-wrap;
    font-family: inherit;
    font-size: var(--text-md);
    line-height: 1.6;
    color: var(--text);
  }
  .pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .sform {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 0 var(--space-2) var(--space-2);
  }
  .results {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0 var(--space-2);
  }
  .res {
    all: unset;
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    box-sizing: border-box;
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .res:hover,
  .res:focus-visible {
    background: var(--fill-1);
  }
  .res-t {
    font-weight: 600;
    font-size: var(--text-sm);
  }
  .res-s {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .res-tags {
    display: flex;
    gap: 4px;
  }
  .preview {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .preview pre {
    font-size: var(--text-sm);
    font-family: var(--font-mono);
  }
  .ed {
    flex: 1;
    margin: 0 var(--space-2);
    min-height: 200px;
    resize: none;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.5;
  }
  .sel {
    max-width: 160px;
    height: 24px;
    font-size: var(--text-sm);
  }
  .synclist {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 0 var(--space-3);
    list-style: none;
  }
  .synclist li {
    display: flex;
    gap: var(--space-2);
    padding: 3px 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .synclist li.done {
    color: var(--text);
  }
  .synclist li.next {
    color: var(--accent-hi);
    font-weight: 650;
  }
  .st {
    width: 44px;
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
  @media (prefers-reduced-motion: reduce) {
    .ln {
      transition: none;
    }
    .ln.on {
      transform: none;
    }
  }
</style>
