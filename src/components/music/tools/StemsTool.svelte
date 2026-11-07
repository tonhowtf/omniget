<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { searchTracks } from "$lib/music/library";
  import { library, trackById, upsertTracks } from "$lib/music/library-store.svelte";
  import { karaoke, loadStemSets } from "$lib/music/karaoke.svelte";
  import { playTracks } from "$lib/music/player-store.svelte";
  import { go } from "$lib/music/ui-store.svelte";
  import {
    fileName,
    isActive,
    MEDIA_EXTENSIONS,
    MODEL_EVENT,
    mergeJob,
    PROGRESS_EVENT,
    STEM_FORMATS,
    stemsApi,
    type ModelProgress,
    type StemFormat,
    type StemJob,
    type StemSet,
    type StemsStatus,
  } from "$lib/music/stems";
  import type { Track } from "$lib/music/types";
  import MusicIcon from "../MusicIcon.svelte";

  let { ids = [] }: { ids?: number[] } = $props();

  let status = $state<StemsStatus | null>(null);
  let installing = $state(false);
  let dl = $state<ModelProgress | null>(null);
  let filter = $state("");
  let files = $state<string[]>([]);
  let format = $state<StemFormat>("flac");
  let outDir = $state("");
  let jobs = $state<StemJob[]>([]);
  let un: UnlistenFn[] = [];

  let lib = $derived(library());
  let targets = $derived(ids.length ? lib.tracks.filter((x) => ids.includes(x.id)) : filter.trim() ? searchTracks(lib.tracks, filter) : []);
  let model = $derived(status?.models.find((m) => m.default) ?? status?.models[0] ?? null);
  let ready = $derived(!!model?.downloaded && !!status?.runtime_installed);
  let unsupported = $derived(!!status && !status.runtime_installed && !status.runtime_can_download);
  let count = $derived(targets.length + files.length);
  let running = $derived(jobs.filter(isActive).length);
  let mb = $derived(model ? Math.round(model.bytes / 1_000_000) : 0);

  async function refreshStatus() {
    status = await stemsApi.status().catch(() => null);
  }

  async function onDone(j: StemJob) {
    const outIds = j.outputs.map((o) => o.track_id).filter((x): x is number => x != null);
    if (outIds.length) upsertTracks(await invoke<Track[]>("music_tracks_by_ids", { ids: outIds }).catch(() => [] as Track[]));
    void refreshStatus();
  }

  onMount(async () => {
    void refreshStatus();
    void loadStemSets();
    jobs = await stemsApi.jobs().catch(() => []);
    un = [
      await listen<StemJob>(PROGRESS_EVENT, (e) => {
        jobs = mergeJob(jobs, e.payload);
        if (e.payload.state === "done") void onDone(e.payload);
      }),
      await listen<ModelProgress>(MODEL_EVENT, (e) => {
        dl = e.payload;
      }),
    ];
  });
  onDestroy(() => un.forEach((u) => u()));

  async function install() {
    installing = true;
    try {
      status = await stemsApi.installModel();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      installing = false;
      dl = null;
    }
  }

  async function removeModel() {
    try {
      status = await stemsApi.removeModel();
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function pickFiles() {
    const picked = await open({ multiple: true, filters: [{ name: $t("music.stems.media_files") as string, extensions: MEDIA_EXTENSIONS }] });
    const list = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    files = [...new Set([...files, ...list])];
  }

  async function pickDir() {
    const d = await open({ directory: true });
    if (typeof d === "string") outDir = d;
  }

  async function start() {
    if (!count) return;
    try {
      const created = await stemsApi.start({
        track_ids: targets.map((x) => x.id),
        paths: files,
        format,
        out_dir: outDir,
        labels: { vocals: $t("music.stems.label_vocals") as string, instrumental: $t("music.stems.label_instrumental") as string },
      });
      jobs = [...jobs, ...created];
      files = [];
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function clearFinished() {
    await stemsApi.clear().catch(() => {});
    jobs = jobs.filter(isActive);
  }

  async function sing(set: StemSet) {
    if (set.instrumental_id == null) return;
    await playTracks([set.instrumental_id], set.instrumental_id, $t("music.karaoke.title") as string);
    go({ kind: "nowplaying" });
  }

  function play(id: number | null) {
    if (id != null) void playTracks([id], id);
  }

  function setTitle(set: StemSet): string {
    const src = set.source_id != null ? trackById(set.source_id) : undefined;
    return src?.title || fileName(set.source);
  }

  function jobSet(j: StemJob): StemSet | undefined {
    const inst = j.outputs.find((o) => o.stem === "instrumental")?.path;
    return karaoke.sets.find((s) => s.instrumental === inst);
  }

  function dlPct(p: ModelProgress | null): number {
    return p?.total ? Math.round((p.done / p.total) * 100) : 0;
  }
</script>

<div class="st">
  <p class="lead">{$t("music.stems.lead")}</p>

  <section class="model" class:ok={ready}>
    <div class="mi">
      <b>{model?.label ?? $t("music.stems.model_title")}</b>
      <small>
        {#if ready}
          {$t("music.stems.model_ready")}
        {:else}
          {$t("music.stems.model_needed", { size: mb })}
        {/if}
        · {$t("music.stems.model_license", { license: model?.license ?? "MIT" })}
      </small>
      {#if installing && dl}
        <span class="bar"><i style:width="{dlPct(dl)}%"></i></span>
      {/if}
    </div>
    {#if unsupported}
      <span class="warn">{$t("music.stems.no_runtime")}</span>
    {:else if !ready}
      <button class="btn btn-sm" onclick={install} disabled={installing}>
        <MusicIcon name={installing ? "loader" : "download"} size={13} />
        {installing ? $t("music.stems.downloading", { pct: dlPct(dl) }) : $t("music.stems.download_model")}
      </button>
    {:else}
      <button class="btn btn-sm btn-ghost" onclick={removeModel} disabled={running > 0}>{$t("music.stems.remove_model")}</button>
    {/if}
  </section>

  <div class="grid">
    <section class="col">
      <h3>{$t("music.stems.what")}</h3>
      {#if ids.length}
        <p class="dim">{$t("music.convert.selected", { count: targets.length })}</p>
      {:else}
        <input class="input" bind:value={filter} placeholder={$t("music.stems.filter_placeholder") as string} />
        <p class="dim small">{$t("music.convert.matches", { count: targets.length })}</p>
      {/if}
      <button class="btn btn-sm" onclick={pickFiles}><MusicIcon name="folder" size={13} /> {$t("music.stems.pick_files")}</button>
      {#if files.length}
        <ul class="files">
          {#each files as f (f)}
            <li>
              <span title={f}>{fileName(f)}</span>
              <button class="btn btn-sm btn-ghost" onclick={() => (files = files.filter((x) => x !== f))} aria-label={$t("music.remove") as string}><MusicIcon name="close" size={12} /></button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="col">
      <h3>{$t("music.convert.format")}</h3>
      <div class="formats">
        {#each STEM_FORMATS as f (f)}
          <button class="pp" class:on={format === f} onclick={() => (format = f)} aria-pressed={format === f}>
            <b>{$t(`music.stems.f.${f}`)}</b>
            <small>{$t(`music.stems.fd.${f}`)}</small>
          </button>
        {/each}
      </div>
      <h3>{$t("music.convert.where")}</h3>
      <div class="row">
        <input class="input" value={outDir || ($t("music.convert.next_to_source") as string)} readonly />
        <button class="btn" onclick={pickDir} aria-label={$t("music.stems.pick_folder") as string}><MusicIcon name="folder" size={14} /></button>
        {#if outDir}<button class="btn btn-ghost" onclick={() => (outDir = "")} aria-label={$t("music.convert.next_to_source") as string}><MusicIcon name="close" size={14} /></button>{/if}
      </div>
      <p class="dim small">{$t("music.stems.where_hint")}</p>
    </section>
  </div>

  <div class="go">
    <button class="btn btn-primary btn-lg" onclick={start} disabled={!count || unsupported}>
      <MusicIcon name="split" size={15} /> {$t("music.stems.start", { count })}
    </button>
    {#if running}<button class="btn" onclick={() => void stemsApi.cancel(null)}>{$t("music.convert.cancel_all")}</button>{/if}
    {#if jobs.some((j) => !isActive(j))}
      <button class="btn btn-ghost" onclick={clearFinished}>{$t("music.download.clear")}</button>
    {/if}
  </div>
  {#if count}<p class="dim small">{$t("music.stems.time_hint")}</p>{/if}

  {#if jobs.length}
    <ul class="jobs">
      {#each [...jobs].reverse() as j (j.id)}
        {@const set = j.state === "done" ? jobSet(j) : undefined}
        <li class={j.state}>
          <div class="jm">
            <span class="jt">{j.title}</span>
            <span class="jo" title={j.error ?? j.source}>
              {#if j.error}{j.error}{:else if j.state === "preparing"}{$t("music.stems.preparing")}{:else}{fileName(j.source)}{/if}
            </span>
            {#if isActive(j)}<span class="bar"><i style:width="{j.progress * 100}%"></i></span>{/if}
          </div>
          <span class="stt">{$t(`music.stems.state_${j.state}`)}{#if j.state === "running"} · {Math.round(j.progress * 100)}%{/if}</span>
          {#if isActive(j)}
            <button class="btn btn-sm btn-ghost" onclick={() => void stemsApi.cancel(j.id)} aria-label={$t("music.cancel") as string}><MusicIcon name="close" size={12} /></button>
          {:else if j.state === "done"}
            {#if set}<button class="btn btn-sm" onclick={() => void sing(set)}><MusicIcon name="lyrics" size={12} /> {$t("music.karaoke.sing")}</button>{/if}
            <button class="btn btn-sm btn-ghost" onclick={() => void invoke("reveal_file", { path: j.outputs[0]?.path })} aria-label={$t("music.menu.reveal") as string}><MusicIcon name="folder" size={12} /></button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if karaoke.sets.length}
    <h3>{$t("music.stems.ready_title")}</h3>
    <ul class="sets">
      {#each karaoke.sets as s (s.instrumental)}
        <li>
          <span class="jt">{setTitle(s)}</span>
          <div class="acts">
            <button class="btn btn-sm btn-primary" onclick={() => void sing(s)} disabled={s.instrumental_id == null}><MusicIcon name="lyrics" size={12} /> {$t("music.karaoke.sing")}</button>
            <button class="btn btn-sm" onclick={() => play(s.instrumental_id)} disabled={s.instrumental_id == null}>{$t("music.stems.play_instrumental")}</button>
            <button class="btn btn-sm" onclick={() => play(s.vocals_id)} disabled={s.vocals_id == null}>{$t("music.stems.play_vocals")}</button>
            <button class="btn btn-sm btn-ghost" onclick={() => void invoke("reveal_file", { path: s.instrumental })} aria-label={$t("music.menu.reveal") as string}><MusicIcon name="folder" size={12} /></button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .st {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .lead {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
    max-width: 70ch;
  }
  .model {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 10px 12px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .model.ok {
    box-shadow: inset 0 0 0 var(--hairline) color-mix(in srgb, var(--success) 50%, transparent);
  }
  .mi {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }
  .mi b {
    font-size: var(--text-sm);
  }
  .mi small {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .warn {
    font-size: var(--text-xs);
    color: var(--warning);
    max-width: 40ch;
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
    align-items: stretch;
  }
  .col > .btn {
    align-self: flex-start;
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
  .files,
  .jobs,
  .sets {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .files li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    font-size: var(--text-sm);
    padding: 2px 4px 2px 10px;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
  }
  .files span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .formats {
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
  }
  .row .input {
    flex: 1;
  }
  .go {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .jobs li,
  .sets li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 10px;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .sets li {
    justify-content: space-between;
    flex-wrap: wrap;
  }
  .acts {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
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
  .stt {
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .done .stt {
    color: var(--success);
  }
  .bar {
    display: block;
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
