<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getDownloads, knownPercent } from "$lib/stores/download-store.svelte";
  import { fmtTime } from "$lib/music/library";
  import { library } from "$lib/music/library-store.svelte";
  import { clearFinishedMusicDownloads, musicDownloads, startMusicDownload } from "$lib/music/downloads.svelte";
  import { playTracks } from "$lib/music/player-store.svelte";
  import EmptyState from "../EmptyState.svelte";
  import MusicIcon from "../MusicIcon.svelte";

  let { q = "" }: { q?: string } = $props();

  type Result = { id: string; title: string; author: string; duration: number | null; thumbnail_url: string | null; url: string; platform: string };

  let input = $state("");
  let platform = $state<"youtube" | "soundcloud">("youtube");
  let results = $state<Result[]>([]);
  let searching = $state(false);
  let started = $state(new Set<string>());

  let urls = $derived(
    input
      .split(/\s+/)
      .map((x) => x.trim())
      .filter((x) => /^https?:\/\//i.test(x)),
  );
  let isQuery = $derived(!urls.length && input.trim().length > 1);
  let dls = $derived(getDownloads());
  let dir = $derived(library().status?.download_dir ?? "");

  onMount(() => {
    if (q) {
      input = q;
      void search();
    }
  });

  async function search() {
    if (!input.trim()) return;
    searching = true;
    try {
      results = await invoke<Result[]>("search_videos", { query: `${input.trim()}${platform === "youtube" ? " audio" : ""}`, platform, maxResults: 15 });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      searching = false;
    }
  }

  async function download(url: string, title = "", thumb: string | null = null) {
    try {
      await startMusicDownload(url, title, thumb);
      started = new Set([...started, url]);
    } catch (e) {
      const msg = String(e);
      showToast("error", msg.startsWith("PathTooLong") ? ($t("music.download.path_too_long") as string) : msg);
    }
  }

  async function submit() {
    if (urls.length) {
      for (const u of urls) await download(u);
      input = "";
    } else if (isQuery) await search();
  }
</script>

<div class="dv">
  <header>
    <h1>{$t("music.nav.download")}</h1>
    <p class="sub">{$t("music.download.subtitle")} <code title={dir}>{dir}</code></p>
  </header>

  <form
    class="bar"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <label class="find">
      <MusicIcon name={urls.length ? "link" : "search"} size={16} />
      <input class="input input-lg" bind:value={input} placeholder={$t("music.download.placeholder") as string} aria-label={$t("music.download.placeholder") as string} />
    </label>
    {#if !urls.length}
      <div class="mac-segmented">
        <button type="button" class="mac-segmented-btn" class:active={platform === "youtube"} onclick={() => (platform = "youtube")}>YouTube</button>
        <button type="button" class="mac-segmented-btn" class:active={platform === "soundcloud"} onclick={() => (platform = "soundcloud")}>SoundCloud</button>
      </div>
    {/if}
    <button class="btn btn-primary btn-lg" type="submit" disabled={searching || (!urls.length && !isQuery)}>
      {#if searching}<MusicIcon name="loader" size={15} spin />{:else}<MusicIcon name={urls.length ? "download" : "search"} size={15} />{/if}
      {urls.length ? $t("music.download.get", { count: urls.length }) : $t("music.search.go")}
    </button>
  </form>

  {#if results.length}
    <section>
      <h2>{$t("music.download.results")}</h2>
      <ul class="res">
        {#each results as r (r.id)}
          <li>
            {#if r.thumbnail_url}<img src={r.thumbnail_url} alt="" loading="lazy" />{:else}<span class="ph"><MusicIcon name="note" size={18} /></span>{/if}
            <div class="rm">
              <span class="t1">{r.title}</span>
              <span class="t2">{r.author}{r.duration ? ` · ${fmtTime(r.duration * 1000)}` : ""}</span>
            </div>
            <button class="btn" onclick={() => void download(r.url, r.title, r.thumbnail_url)} disabled={started.has(r.url)}>
              <MusicIcon name={started.has(r.url) ? "check" : "download"} size={14} />
              {started.has(r.url) ? $t("music.download.queued") : $t("music.download.one")}
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <section>
    <div class="sh">
      <h2>{$t("music.download.recent")}</h2>
      {#if musicDownloads.items.some((x) => x.state !== "running")}
        <button class="btn btn-sm btn-ghost" onclick={clearFinishedMusicDownloads}>{$t("music.download.clear")}</button>
      {/if}
    </div>
    {#if !musicDownloads.items.length && !results.length}
      <EmptyState art="download" title={$t("music.download.empty_title") as string} body={$t("music.download.empty_body") as string} compact />
    {:else}
      <ul class="jobs">
        {#each musicDownloads.items as md (md.id)}
          {@const d = dls.get(md.id)}
          {@const pct = d ? knownPercent(d.percent) : null}
          <li class="job {md.state}">
            {#if md.thumb}<img src={md.thumb} alt="" loading="lazy" />{:else}<span class="ph"><MusicIcon name="note" size={16} /></span>{/if}
            <div class="rm">
              <span class="t1">{d?.name || md.title}</span>
              <span class="t2">
                {#if md.state === "done"}{$t("music.download.in_library")}
                {:else if md.state === "failed"}<span class="err">{md.error || $t("music.download.failed")}</span>
                {:else if d?.status === "queued"}{$t("music.download.waiting")}
                {:else}{d?.phase || $t("music.download.running")}{pct != null ? ` · ${Math.round(pct)}%` : ""}{/if}
              </span>
              {#if md.state === "running"}
                <span class="prog"><i style:width="{pct ?? 8}%" class:indet={pct == null}></i></span>
              {/if}
            </div>
            {#if md.state === "done" && md.trackIds?.length}
              <button class="btn btn-sm" onclick={() => md.trackIds && void playTracks(md.trackIds)}><MusicIcon name="play" size={13} /> {$t("music.play")}</button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

<style>
  .dv {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding-bottom: var(--space-7);
    max-width: 920px;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  .sub {
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--fill-2);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1 1 360px;
    color: var(--text-muted);
  }
  .find input {
    flex: 1;
  }
  h2 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-lg);
    font-weight: 700;
  }
  .sh {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .res,
  .jobs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .res li,
  .job {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 6px 8px;
    border-radius: var(--radius-md);
  }
  .res li:hover {
    background: var(--fill-1);
  }
  img,
  .ph {
    width: 64px;
    height: 40px;
    border-radius: 6px;
    object-fit: cover;
    flex-shrink: 0;
    background: var(--fill-2);
    display: grid;
    place-items: center;
    color: var(--text-muted);
  }
  .rm {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .t1 {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .t2 {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .err {
    color: var(--danger);
  }
  .job.done .t2 {
    color: var(--success);
  }
  .prog {
    height: 3px;
    border-radius: 2px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .prog i {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.3s;
  }
  .prog i.indet {
    animation: indet 1.2s ease-in-out infinite;
  }
  @keyframes indet {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(1200%);
    }
  }
</style>
