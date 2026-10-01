<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { formatBytes } from "$lib/stores/download-store.svelte";
  import { getSettings } from "$lib/stores/settings-store.svelte";

  type Track = {
    id: string;
    label: string;
    language: string | null;
    height: number | null;
    bitrate_kbps: number | null;
    codec: string | null;
    has_audio: boolean;
  };
  type Tracks = { title: string; video: Track[]; audio: Track[]; subtitles: Track[]; protected: number };
  type Result = { path: string; bytes: number };
  type Progress = { percent: number; stage: string };

  let url = $state("");
  let loading = $state(false);
  let tracks = $state<Tracks | null>(null);
  let video = $state<string>("");
  let audio = $state<string[]>([]);
  let subs = $state<string[]>([]);
  let container = $state<"mkv" | "mp4">("mkv");
  let running = $state(false);
  let progress = $state<Progress | null>(null);
  let result = $state<Result | null>(null);
  let error = $state<string | null>(null);

  let unlisten: (() => void) | null = null;
  listen<Progress>("manifest-tracks-progress", (e) => {
    if (running) progress = e.payload;
  }).then((fn) => (unlisten = fn));
  onDestroy(() => unlisten?.());

  function describe(tr: Track) {
    return [tr.codec, tr.bitrate_kbps ? `${tr.bitrate_kbps} kbps` : null].filter(Boolean).join(" · ");
  }

  function fail(e: any) {
    const msg = typeof e === "string" ? e : (e?.message ?? "");
    return msg === "Cancelled" ? $t("downloads.tools.manifest_tracks.cancelled") : msg ? translateBackendError(msg, $t) : $t("common.error");
  }

  async function read() {
    const target = url.trim();
    if (!target || loading || running) return;
    loading = true;
    error = null;
    result = null;
    tracks = null;
    try {
      const got = await invoke<Tracks>("manifest_tracks_list", { url: target });
      tracks = got;
      video = got.video[0]?.id ?? "";
      const seen = new Set<string>();
      audio = got.audio
        .filter((a) => {
          const key = a.language ?? a.label;
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        })
        .map((a) => a.id);
      if (got.video[0]?.has_audio) audio = [];
      subs = got.subtitles.map((s) => s.id);
    } catch (e) {
      error = fail(e);
    } finally {
      loading = false;
    }
  }

  function toggle(list: string[], id: string) {
    return list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
  }

  async function download() {
    if (!tracks || running) return;
    const outputDir = getSettings()?.download.default_output_dir ?? "";
    if (!outputDir) {
      error = $t("downloads.tools.manifest_tracks.no_folder");
      return;
    }
    if (!video && !audio.length) {
      error = $t("downloads.tools.manifest_tracks.pick_one");
      return;
    }
    running = true;
    error = null;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("manifest_tracks_download", {
        url: url.trim(),
        video: video || null,
        audio,
        subtitles: subs,
        outputDir,
        container,
      });
    } catch (e) {
      error = fail(e);
    } finally {
      running = false;
    }
  }
</script>

<div class="mt" data-agent-readiness={loading || running ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.manifest_tracks.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      read();
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.manifest_tracks.placeholder")}
      aria-label={$t("downloads.tools.manifest_tracks.placeholder")}
      spellcheck="false"
      autocomplete="off"
      disabled={loading || running}
    />
    <button class="button" type="submit" disabled={!url.trim() || loading || running}>
      {loading ? $t("downloads.tools.manifest_tracks.reading") : $t("downloads.tools.manifest_tracks.read")}
    </button>
  </form>

  {#if tracks}
    <p class="title">{tracks.title}</p>
    <div class="groups">
      {#if tracks.video.length}
        <fieldset>
          <legend>{$t("downloads.tools.manifest_tracks.video")}</legend>
          <label><input type="radio" name="mt-video" value="" bind:group={video} disabled={running} /> {$t("downloads.tools.manifest_tracks.no_video")}</label>
          {#each tracks.video as tr (tr.id)}
            <label>
              <input type="radio" name="mt-video" value={tr.id} bind:group={video} disabled={running} />
              {tr.label} <span class="dim">{describe(tr)}{tr.has_audio ? ` · ${$t("downloads.tools.manifest_tracks.with_sound")}` : ""}</span>
            </label>
          {/each}
        </fieldset>
      {/if}
      {#if tracks.audio.length}
        <fieldset>
          <legend>{$t("downloads.tools.manifest_tracks.audio")}</legend>
          {#each tracks.audio as tr (tr.id)}
            <label>
              <input type="checkbox" checked={audio.includes(tr.id)} onchange={() => (audio = toggle(audio, tr.id))} disabled={running} />
              {tr.label}{#if tr.language && tr.language !== tr.label} <span class="dim">({tr.language})</span>{/if} <span class="dim">{describe(tr)}</span>
            </label>
          {/each}
        </fieldset>
      {/if}
      {#if tracks.subtitles.length}
        <fieldset>
          <legend>{$t("downloads.tools.manifest_tracks.subtitles")}</legend>
          {#each tracks.subtitles as tr (tr.id)}
            <label>
              <input type="checkbox" checked={subs.includes(tr.id)} onchange={() => (subs = toggle(subs, tr.id))} disabled={running} />
              {tr.label}{#if tr.label !== tr.id} <span class="dim">({tr.id})</span>{/if}
            </label>
          {/each}
        </fieldset>
      {/if}
    </div>
    {#if tracks.protected}
      <p class="dim">{$t("downloads.tools.manifest_tracks.protected", { count: tracks.protected })}</p>
    {/if}
    <div class="actions">
      <label>
        {$t("downloads.tools.manifest_tracks.container")}
        <select bind:value={container} disabled={running}>
          <option value="mkv">MKV</option>
          <option value="mp4">MP4</option>
        </select>
      </label>
      {#if running}
        <button class="button" type="button" onclick={() => void invoke("manifest_tracks_cancel")}>{$t("downloads.tools.manifest_tracks.cancel")}</button>
      {:else}
        <button class="button" type="button" onclick={download}>{$t("downloads.tools.manifest_tracks.download")}</button>
      {/if}
    </div>
  {/if}

  {#if running}
    <p class="status" role="status">
      {#if progress?.stage === "merge"}
        {$t("downloads.tools.manifest_tracks.merging")}
      {:else}
        {$t("downloads.tools.manifest_tracks.progress", { stage: progress?.stage ?? "1", percent: Math.round(progress?.percent ?? 0) })}
      {/if}
    </p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if result}
    <div class="done">
      <p>{$t("downloads.tools.manifest_tracks.done", { size: formatBytes(result.bytes) })}</p>
      <button class="link" onclick={() => void invoke("reveal_file", { path: result!.path })}>{$t("downloads.tools.manifest_tracks.show")}</button>
    </div>
  {/if}
  <p class="dim small">{$t("downloads.tools.manifest_tracks.note")}</p>
</div>

<style>
  .mt {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .lead,
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .small {
    font-size: var(--text-xs, 11px);
  }
  .title {
    margin: 0;
    font-weight: 600;
    font-size: var(--text-sm);
  }
  .row {
    display: flex;
    gap: var(--space-2);
  }
  .row input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-md, 8px);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    color: var(--text);
    font-size: var(--text-sm);
  }
  .row input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .groups {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--space-3);
  }
  fieldset {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    font-size: var(--text-sm);
  }
  legend {
    padding: 0;
    color: var(--text-dim);
    font-size: var(--text-xs, 11px);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  fieldset label {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }
  .actions label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  .actions select {
    background: var(--surface);
    color: var(--text);
    border: none;
    border-radius: var(--radius-md, 8px);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    padding: 2px var(--space-2);
  }
  .status,
  .done p {
    margin: 0;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }
  .done {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }
  .error {
    margin: 0;
    color: var(--danger, #d33);
    font-size: var(--text-sm);
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: var(--text-sm);
    cursor: pointer;
    padding: 2px var(--space-1);
    border-radius: var(--radius-full);
  }
  .link:hover {
    background: var(--accent-soft);
  }
</style>
