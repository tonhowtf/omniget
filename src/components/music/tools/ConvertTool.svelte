<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { searchTracks } from "$lib/music/library";
  import { library, loadLibrary } from "$lib/music/library-store.svelte";
  import type { ConvertJob, ConvertOptions, Preset } from "$lib/music/types";
  import MusicIcon from "../MusicIcon.svelte";

  let { ids = [] }: { ids?: number[] } = $props();

  const PRESETS: Preset[] = ["mp3_v0", "mp3_320", "mp3_192", "aac256", "opus160", "opus96", "vorbis", "flac", "alac", "wav"];
  const TEMPLATES = ["", "{artist} - {title}", "{album_artist}/{album}/{track} - {title}", "{genre}/{artist} - {title}"];

  let preset = $state<Preset>("mp3_v0");
  let outDir = $state("");
  let template = $state("");
  let filter = $state("");
  let opts = $state<ConvertOptions>({ sample_rate: null, trim_silence: false, mono: false, fade_in_s: null, fade_out_s: null });
  let bake = $state(false);
  let addLib = $state(false);
  let jobs = $state<ConvertJob[]>([]);
  let un: UnlistenFn | null = null;
  let reloadTimer: ReturnType<typeof setTimeout> | null = null;

  let lib = $derived(library());
  let targets = $derived(ids.length ? lib.tracks.filter((x) => ids.includes(x.id)) : filter.trim() ? searchTracks(lib.tracks, filter) : []);

  onMount(async () => {
    jobs = await invoke<ConvertJob[]>("music_convert_jobs").catch(() => []);
    un = await listen<ConvertJob>("music-convert-progress", (e) => {
      const j = e.payload;
      const i = jobs.findIndex((x) => x.id === j.id);
      jobs = i >= 0 ? jobs.map((x) => (x.id === j.id ? j : x)) : [...jobs, j];
      if (j.state === "done" && addLib) {
        if (reloadTimer) clearTimeout(reloadTimer);
        reloadTimer = setTimeout(() => void loadLibrary(true), 800);
      }
    });
  });
  onDestroy(() => un?.());

  async function pickDir() {
    const d = await open({ directory: true });
    if (typeof d === "string") outDir = d;
  }

  async function start() {
    if (!targets.length) return;
    try {
      const created = await invoke<ConvertJob[]>("music_convert", {
        request: {
          track_ids: targets.map((x) => x.id),
          preset,
          options: $state.snapshot(opts),
          out_dir: outDir,
          template,
          bake_replaygain: bake,
          add_to_library: addLib,
          overwrite: false,
        },
      });
      jobs = [...jobs, ...created];
    } catch (e) {
      showToast("error", String(e));
    }
  }

  let running = $derived(jobs.filter((j) => j.state === "running" || j.state === "queued").length);
</script>

<div class="ct">
  <div class="grid">
    <section class="col">
      <h3>{$t("music.convert.what")}</h3>
      {#if ids.length}
        <p class="dim">{$t("music.convert.selected", { count: targets.length })}</p>
      {:else}
        <input class="input" bind:value={filter} placeholder={$t("music.convert.filter_placeholder") as string} />
        <p class="dim small">{$t("music.convert.matches", { count: targets.length })}</p>
      {/if}

      <h3>{$t("music.convert.format")}</h3>
      <div class="presets">
        {#each PRESETS as p (p)}
          <button class="pp" class:on={preset === p} onclick={() => (preset = p)}>
            <b>{$t(`music.convert.p.${p}`)}</b>
            <small>{$t(`music.convert.pd.${p}`)}</small>
          </button>
        {/each}
      </div>
    </section>

    <section class="col">
      <h3>{$t("music.convert.where")}</h3>
      <div class="row">
        <input class="input" value={outDir || ($t("music.convert.next_to_source") as string)} readonly />
        <button class="btn" onclick={pickDir}><MusicIcon name="folder" size={14} /></button>
        {#if outDir}<button class="btn btn-ghost" onclick={() => (outDir = "")}><MusicIcon name="close" size={14} /></button>{/if}
      </div>
      <label class="field">
        <span class="field-label">{$t("music.convert.naming")}</span>
        <select class="input" bind:value={template}>
          {#each TEMPLATES as tp (tp)}<option value={tp}>{tp || $t("music.convert.keep_name")}</option>{/each}
        </select>
        <input class="input mono" bind:value={template} placeholder={"{artist} - {title}"} />
        <span class="field-hint">{$t("music.convert.tokens")}</span>
      </label>

      <h3>{$t("music.convert.options")}</h3>
      <label class="chk"><input type="checkbox" bind:checked={opts.trim_silence} /> {$t("music.convert.trim")}</label>
      <label class="chk"><input type="checkbox" bind:checked={bake} /> {$t("music.convert.bake_rg")}</label>
      <label class="chk"><input type="checkbox" bind:checked={opts.mono} /> {$t("music.convert.mono")}</label>
      <label class="chk"><input type="checkbox" bind:checked={addLib} /> {$t("music.convert.add_library")}</label>
      <div class="row">
        <label class="mini">{$t("music.convert.rate")}
          <select class="input" bind:value={opts.sample_rate}>
            <option value={null}>{$t("music.convert.keep")}</option>
            <option value={44100}>44.1 kHz</option>
            <option value={48000}>48 kHz</option>
            <option value={96000}>96 kHz</option>
          </select>
        </label>
        <label class="mini">{$t("music.convert.fade_in")}<input class="input" type="number" min="0" step="0.5" bind:value={opts.fade_in_s} /></label>
        <label class="mini">{$t("music.convert.fade_out")}<input class="input" type="number" min="0" step="0.5" bind:value={opts.fade_out_s} /></label>
      </div>
    </section>
  </div>

  <div class="go">
    <button class="btn btn-primary btn-lg" onclick={start} disabled={!targets.length}><MusicIcon name="convert" size={15} /> {$t("music.convert.start", { count: targets.length })}</button>
    {#if running}<button class="btn" onclick={() => void invoke("music_convert_cancel", { id: null })}>{$t("music.convert.cancel_all")}</button>{/if}
    {#if jobs.some((j) => j.state !== "running" && j.state !== "queued")}
      <button class="btn btn-ghost" onclick={async () => { await invoke("music_convert_clear"); jobs = jobs.filter((j) => j.state === "running" || j.state === "queued"); }}>{$t("music.download.clear")}</button>
    {/if}
  </div>

  {#if jobs.length}
    <ul class="jobs">
      {#each [...jobs].reverse() as j (j.id)}
        <li class={j.state}>
          <div class="jm">
            <span class="jt">{j.title}</span>
            <span class="jo" title={j.output}>{j.error ?? j.output}</span>
            {#if j.state === "running" || j.state === "queued"}<span class="bar"><i style:width="{j.progress * 100}%"></i></span>{/if}
          </div>
          <span class="st">{$t(`music.convert.state_${j.state}`)}</span>
          {#if j.state === "running" || j.state === "queued"}
            <button class="btn btn-sm btn-ghost" onclick={() => void invoke("music_convert_cancel", { id: j.id })} aria-label={$t("music.cancel") as string}><MusicIcon name="close" size={12} /></button>
          {:else if j.state === "done"}
            <button class="btn btn-sm btn-ghost" onclick={() => void invoke("reveal_file", { path: j.output })} aria-label={$t("music.menu.reveal") as string}><MusicIcon name="folder" size={12} /></button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .ct {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
    gap: var(--space-5);
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  h3 {
    margin: var(--space-2) 0 0;
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
  }
  .dim {
    color: var(--text-dim);
    margin: 0;
  }
  .small {
    font-size: var(--text-xs);
  }
  .presets {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 6px;
  }
  .pp {
    all: unset;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    cursor: pointer;
  }
  .pp.on {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .pp:focus-visible {
    outline: var(--focus-ring);
  }
  .pp b {
    font-size: var(--text-sm);
  }
  .pp small {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .row {
    display: flex;
    gap: 6px;
    align-items: flex-end;
    flex-wrap: wrap;
  }
  .row .input {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
  }
  .chk input {
    accent-color: var(--accent);
  }
  .mini {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    flex: 1;
    min-width: 90px;
  }
  .go {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .jobs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .jobs li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .jm {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .jt {
    font-weight: 600;
    font-size: var(--text-sm);
  }
  .jo {
    font-size: var(--text-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .failed .jo {
    color: var(--danger);
  }
  .st {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .done .st {
    color: var(--success);
  }
  .bar {
    height: 3px;
    border-radius: 2px;
    background: var(--fill-2);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.3s;
  }
</style>
