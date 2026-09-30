<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy } from "svelte";
  import { t } from "$lib/i18n";
  import { translateBackendError } from "$lib/error-translate";
  import { getSettings } from "$lib/stores/settings-store.svelte";

  type Result = { show_dir: string; show: string; added: string[] };
  type Progress = { item: number; total: number; percent: number };

  const SERVERS = [
    { id: "jellyfin", label: "Jellyfin" },
    { id: "plex", label: "Plex" },
    { id: "emby", label: "Emby" },
    { id: "kodi", label: "Kodi" },
  ];
  const LIMITS = [0, 5, 10, 25, 50];
  const QUALITIES = [2160, 1080, 720, 480];

  let url = $state("");
  let server = $state("jellyfin");
  let limit = $state(10);
  let quality = $state(1080);
  let genre = $state("");
  let running = $state(false);
  let progress = $state<Progress | null>(null);
  let result = $state<Result | null>(null);
  let error = $state<string | null>(null);

  let unlisten: (() => void) | null = null;
  listen<Progress>("tv-library-progress", (e) => {
    if (running) progress = e.payload;
  }).then((fn) => (unlisten = fn));
  onDestroy(() => unlisten?.());

  async function start() {
    const target = url.trim();
    if (!target || running) return;
    const libraryDir = getSettings()?.download.default_output_dir ?? "";
    if (!libraryDir) {
      error = $t("downloads.tools.tv_library.no_folder");
      return;
    }
    running = true;
    error = null;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tv_library_sync", {
        url: target,
        libraryDir,
        server,
        genre: genre.trim() || null,
        maxItems: limit || null,
        maxHeight: quality,
      });
    } catch (e: any) {
      const msg = typeof e === "string" ? e : (e?.message ?? "");
      error = msg === "Cancelled" ? $t("downloads.tools.tv_library.cancelled") : msg ? translateBackendError(msg, $t) : $t("common.error");
    } finally {
      running = false;
    }
  }
</script>

<div class="tv" data-agent-readiness={running ? "loading" : "ready"}>
  <p class="lead">{$t("downloads.tools.tv_library.lead")}</p>
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      start();
    }}
  >
    <input
      type="url"
      bind:value={url}
      placeholder={$t("downloads.tools.tv_library.placeholder")}
      aria-label={$t("downloads.tools.tv_library.placeholder")}
      spellcheck="false"
      autocomplete="off"
      disabled={running}
    />
    {#if running}
      <button class="button" type="button" onclick={() => void invoke("tv_library_cancel")}>{$t("downloads.tools.tv_library.cancel")}</button>
    {:else}
      <button class="button" type="submit" disabled={!url.trim()}>{$t("downloads.tools.tv_library.start")}</button>
    {/if}
  </form>

  <div class="opts">
    <label>
      {$t("downloads.tools.tv_library.server")}
      <select bind:value={server} disabled={running}>
        {#each SERVERS as s (s.id)}
          <option value={s.id}>{s.label}</option>
        {/each}
      </select>
    </label>
    <label>
      {$t("downloads.tools.tv_library.latest")}
      <select bind:value={limit} disabled={running}>
        {#each LIMITS as n (n)}
          <option value={n}>{n ? $t("downloads.tools.tv_library.latest_n", { count: n }) : $t("downloads.tools.tv_library.all")}</option>
        {/each}
      </select>
    </label>
    <label>
      {$t("downloads.tools.tv_library.quality")}
      <select bind:value={quality} disabled={running}>
        {#each QUALITIES as q (q)}
          <option value={q}>{q}p</option>
        {/each}
      </select>
    </label>
    <label>
      {$t("downloads.tools.tv_library.genre")}
      <input class="genre" type="text" bind:value={genre} placeholder={$t("downloads.tools.tv_library.genre_placeholder")} disabled={running} />
    </label>
  </div>

  {#if running}
    <p class="status" role="status">
      {#if progress}
        {$t("downloads.tools.tv_library.progress", { item: progress.item, total: progress.total, percent: Math.round(progress.percent) })}
      {:else}
        {$t("downloads.tools.tv_library.starting")}
      {/if}
    </p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if result}
    <div class="done">
      <p>
        {#if result.added.length}
          {$t("downloads.tools.tv_library.done", { count: result.added.length, show: result.show })}
        {:else}
          {$t("downloads.tools.tv_library.nothing_new")}
        {/if}
      </p>
      {#if result.added.length}
        <button class="link" onclick={() => void invoke("reveal_file", { path: result!.show_dir })}>{$t("downloads.tools.tv_library.show")}</button>
      {/if}
    </div>
  {/if}
  <p class="dim small">{$t("downloads.tools.tv_library.note")}</p>
</div>

<style>
  .tv {
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
  .row {
    display: flex;
    gap: var(--space-2);
  }
  .row input,
  .genre {
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
  .genre {
    width: 120px;
    flex: none;
    padding: 2px var(--space-2);
  }
  .row input:focus-visible,
  .genre:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .opts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-sm);
  }
  .opts label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  .opts select {
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
