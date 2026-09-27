<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { fmtBytes, fmtTime } from "$lib/music/library";
  import { trackById, upsertTracks } from "$lib/music/library-store.svelte";
  import { syncCurrent } from "$lib/music/player-store.svelte";
  import type { CoverInput, MbCandidate, TagDetail, TagPatch, Track } from "$lib/music/types";
  import Cover from "./Cover.svelte";
  import Modal from "./Modal.svelte";
  import MusicIcon from "./MusicIcon.svelte";

  let { ids, onclose }: { ids: number[]; onclose: () => void } = $props();

  type Field = keyof TagPatch;
  const FIELDS: { key: Field; wide?: boolean; num?: boolean; multi?: boolean }[] = [
    { key: "title", wide: true },
    { key: "artist", wide: true },
    { key: "album", wide: true },
    { key: "album_artist", wide: true },
    { key: "genre" },
    { key: "composer" },
    { key: "year", num: true },
    { key: "bpm", num: true },
    { key: "track", num: true },
    { key: "track_total", num: true },
    { key: "disc", num: true },
    { key: "disc_total", num: true },
    { key: "musical_key" },
    { key: "comment", wide: true },
  ];

  let tracks = $derived(ids.map((id) => trackById(id)).filter(Boolean) as Track[]);
  let single = $derived(tracks.length === 1);
  let detail = $state<TagDetail | null>(null);
  let values = $state<Record<string, string>>({});
  let mixed = $state<Record<string, boolean>>({});
  let touched = $state<Record<string, boolean>>({});
  let cover = $state<CoverInput>({ kind: "keep" });
  let coverPreview = $state<string | null>(null);
  let coverUrlInput = $state("");
  let saving = $state(false);
  let mb = $state<MbCandidate[] | null>(null);
  let mbBusy = $state(false);

  function fromTrack(t: Track): Record<string, string> {
    return {
      title: t.title,
      artist: t.artist,
      album: t.album,
      album_artist: t.album_artist,
      genre: t.genre,
      composer: t.composer,
      year: t.year?.toString() ?? "",
      bpm: t.bpm ? String(Math.round(t.bpm)) : "",
      track: t.track_no?.toString() ?? "",
      track_total: "",
      disc: t.disc_no?.toString() ?? "",
      disc_total: "",
      musical_key: t.musical_key,
      comment: "",
    };
  }

  onMount(async () => {
    if (single) {
      try {
        detail = await invoke<TagDetail>("music_tags_read", { id: tracks[0].id });
        const m = detail.meta;
        values = {
          title: m.title,
          artist: m.artist,
          album: m.album,
          album_artist: m.album_artist,
          genre: m.genre,
          composer: m.composer,
          year: m.year?.toString() ?? "",
          bpm: m.bpm ? String(Math.round(m.bpm)) : "",
          track: m.track?.toString() ?? "",
          track_total: m.track_total?.toString() ?? "",
          disc: m.disc?.toString() ?? "",
          disc_total: m.disc_total?.toString() ?? "",
          musical_key: m.musical_key,
          comment: m.comment,
        };
      } catch (e) {
        showToast("error", String(e));
        values = fromTrack(tracks[0]);
      }
    } else {
      const all = tracks.map(fromTrack);
      const v: Record<string, string> = {};
      const mx: Record<string, boolean> = {};
      for (const f of FIELDS) {
        const set = new Set(all.map((a) => a[f.key] ?? ""));
        v[f.key] = set.size === 1 ? [...set][0] : "";
        mx[f.key] = set.size > 1;
      }
      values = v;
      mixed = mx;
    }
  });

  function edit(k: string, v: string) {
    values = { ...values, [k]: v };
    touched = { ...touched, [k]: true };
  }

  async function pickCover() {
    const p = await open({ multiple: false, filters: [{ name: "Image", extensions: ["jpg", "jpeg", "png", "webp"] }] });
    if (typeof p === "string") {
      cover = { kind: "file", path: p };
      coverPreview = null;
    }
  }

  function coverFromUrl(url: string) {
    if (!/^https?:\/\//i.test(url)) return;
    cover = { kind: "url", url };
    coverPreview = url;
  }

  async function lookup() {
    mbBusy = true;
    try {
      mb = await invoke<MbCandidate[]>("music_mb_search", {
        id: tracks[0].id,
        artist: values.artist || null,
        title: values.title || null,
        album: values.album || null,
      });
    } catch (e) {
      showToast("error", String(e));
    } finally {
      mbBusy = false;
    }
  }

  function applyMb(c: MbCandidate) {
    const set = (k: string, v: string | number | null | undefined) => {
      if (v != null && v !== "") edit(k, String(v));
    };
    set("title", c.title);
    set("artist", c.artist);
    set("album", c.album);
    set("album_artist", c.album_artist);
    set("year", c.year);
    set("track", c.track);
    set("track_total", c.track_total);
    if (c.cover_url) coverFromUrl(c.cover_url);
    mb = null;
  }

  async function saveAll() {
    const patch: TagPatch = {};
    for (const f of FIELDS) {
      if (single ? values[f.key] !== undefined : touched[f.key]) patch[f.key] = values[f.key] ?? "";
    }
    if (!single && !Object.keys(patch).length && cover.kind === "keep") {
      onclose();
      return;
    }
    saving = true;
    try {
      const r = await invoke<{ updated: Track[]; failed: [number, string][] }>("music_tags_write", { ids, patch, cover });
      upsertTracks(r.updated);
      syncCurrent();
      if (r.failed.length) showToast("error", $t("music.tags.partial", { ok: r.updated.length, failed: r.failed.length, err: r.failed[0][1] }) as string);
      else showToast("success", $t("music.tags.saved", { count: r.updated.length }) as string);
      onclose();
    } catch (e) {
      showToast("error", String(e));
    } finally {
      saving = false;
    }
  }
</script>

<Modal title={single ? ($t("music.tags.title_one") as string) : ($t("music.tags.title_many", { count: tracks.length }) as string)} width={760} {onclose}>
  <div class="te">
    <aside class="side">
      <div class="art">
        {#if cover.kind === "remove"}
          <Cover hash={null} seed={values.album || values.title} size="100%" />
        {:else if coverPreview}
          <img src={coverPreview} alt="" />
        {:else if cover.kind === "file"}
          <div class="picked"><MusicIcon name="image" size={28} /><span>{cover.path.split(/[\\/]/).pop()}</span></div>
        {:else}
          <Cover hash={tracks[0]?.cover} seed={values.album || values.title} size="100%" full />
        {/if}
      </div>
      <div class="cacts">
        <button class="btn btn-sm" onclick={pickCover}><MusicIcon name="image" size={13} /> {$t("music.tags.choose_image")}</button>
        <button class="btn btn-sm btn-ghost" onclick={() => { cover = { kind: "remove" }; coverPreview = null; }}>{$t("music.tags.remove_cover")}</button>
        {#if cover.kind !== "keep"}<button class="btn btn-sm btn-ghost" onclick={() => { cover = { kind: "keep" }; coverPreview = null; }}>{$t("music.tags.keep_cover")}</button>{/if}
      </div>
      <form
        class="urlf"
        onsubmit={(e) => {
          e.preventDefault();
          coverFromUrl(coverUrlInput.trim());
        }}
      >
        <input class="input" bind:value={coverUrlInput} placeholder={$t("music.tags.cover_url") as string} />
      </form>
      {#if single && detail}
        <dl class="info">
          <dt>{$t("music.tags.format")}</dt><dd>{detail.meta.codec} {detail.meta.bitrate ? `· ${detail.meta.bitrate} kbps` : ""}</dd>
          <dt>{$t("music.tags.sample")}</dt><dd>{detail.meta.sample_rate ?? "?"} Hz · {detail.meta.bit_depth ? `${detail.meta.bit_depth}-bit · ` : ""}{detail.meta.channels ?? "?"} ch</dd>
          <dt>{$t("music.tags.length")}</dt><dd>{fmtTime(detail.meta.duration_ms)} · {fmtBytes(detail.size)}</dd>
          <dt>{$t("music.tags.file")}</dt><dd class="path" title={detail.path}>{detail.path}</dd>
        </dl>
      {/if}
    </aside>

    <div class="main">
      {#if single}
        <div class="mbbar">
          <button class="btn btn-sm" onclick={lookup} disabled={mbBusy}>{#if mbBusy}<MusicIcon name="loader" size={13} spin />{:else}<MusicIcon name="globe" size={13} />{/if} {$t("music.tags.lookup")}</button>
          <span class="hint">{$t("music.tags.lookup_hint")}</span>
        </div>
        {#if mb}
          <ul class="mb">
            {#each mb as c (c.recording_id + c.release_id)}
              <li>
                <button onclick={() => applyMb(c)}>
                  <span class="mt"><b>{c.title}</b> — {c.artist}</span>
                  <span class="ms">{c.album || "—"}{c.year ? ` · ${c.year}` : ""}{c.release_type ? ` · ${c.release_type}` : ""}{c.length_ms ? ` · ${fmtTime(c.length_ms)}` : ""} · {c.score}%</span>
                </button>
              </li>
            {:else}
              <li class="hint">{$t("music.lyrics.no_results")}</li>
            {/each}
          </ul>
        {/if}
      {/if}

      <div class="fields">
        {#each FIELDS as f (f.key)}
          <label class="fl" class:wide={f.wide}>
            <span>{$t(`music.tags.f.${f.key}`)}</span>
            <input
              class="input"
              inputmode={f.num ? "numeric" : undefined}
              value={values[f.key] ?? ""}
              placeholder={mixed[f.key] && !touched[f.key] ? ($t("music.tags.multiple") as string) : ""}
              oninput={(e) => edit(f.key, (e.currentTarget as HTMLInputElement).value)}
            />
          </label>
        {/each}
      </div>
      {#if !single}<p class="hint">{$t("music.tags.batch_hint")}</p>{/if}
    </div>
  </div>
  <div class="dialog-actions">
    <button class="btn" onclick={onclose}>{$t("music.cancel")}</button>
    <button class="btn btn-primary" onclick={saveAll} disabled={saving}>{#if saving}<MusicIcon name="loader" size={13} spin />{/if}{$t("music.save")}</button>
  </div>
</Modal>

<style>
  .te {
    display: grid;
    grid-template-columns: 200px 1fr;
    gap: var(--space-4);
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .art {
    width: 200px;
    height: 200px;
    border-radius: 10px;
    overflow: hidden;
    background: var(--fill-1);
  }
  .art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .art :global(.cover) {
    width: 100% !important;
    height: 100% !important;
  }
  .picked {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    text-align: center;
    padding: 8px;
    box-sizing: border-box;
    word-break: break-all;
  }
  .cacts {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .urlf .input {
    font-size: var(--text-xs);
  }
  .info {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 8px;
    margin: 0;
    font-size: var(--text-xs);
  }
  .info dt {
    color: var(--text-muted);
  }
  .info dd {
    margin: 0;
    color: var(--text-dim);
  }
  .path {
    word-break: break-all;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  .mbbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .hint {
    font-size: var(--text-xs);
    color: var(--text-dim);
    margin: 0;
  }
  .mb {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 180px;
    overflow: auto;
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .mb button {
    all: unset;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    width: 100%;
    padding: 6px 10px;
    cursor: pointer;
  }
  .mb button:hover,
  .mb button:focus-visible {
    background: var(--fill-2);
  }
  .mt {
    font-size: var(--text-sm);
  }
  .ms {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .fields {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-2);
  }
  .fl {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: var(--text-xs);
    color: var(--text-dim);
    min-width: 0;
  }
  .fl.wide {
    grid-column: span 2;
  }
  @media (max-width: 720px) {
    .te {
      grid-template-columns: 1fr;
    }
    .fields {
      grid-template-columns: repeat(2, 1fr);
    }
  }
</style>
