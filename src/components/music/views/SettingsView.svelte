<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { addLibraryFolder, library, loadLibrary, refreshStatus, rescan } from "$lib/music/library-store.svelte";
  import { ui } from "$lib/music/ui-store.svelte";
  import MusicIcon from "../MusicIcon.svelte";

  let lib = $derived(library());

  onMount(() => void refreshStatus());

  let typedPath = $state("");

  async function addTyped() {
    const path = typedPath.trim();
    if (!path) return;
    try {
      await invoke("music_roots_add", { path });
      typedPath = "";
      await refreshStatus();
      await loadLibrary(true);
    } catch (e) {
      showToast("error", String(e));
    }
  }
  let roots = $derived(lib.status?.roots ?? []);

  function removeRoot(path: string) {
    ui.confirm = {
      title: $t("music.settings.remove_folder") as string,
      body: $t("music.settings.remove_folder_body", { path }) as string,
      ok: $t("music.remove") as string,
      run: async () => {
        await invoke("music_roots_remove", { path, forgetTracks: true });
        await refreshStatus();
        await loadLibrary(true);
      },
    };
  }

  function clearHistory() {
    ui.confirm = {
      title: $t("music.settings.clear_history") as string,
      body: $t("music.settings.clear_history_body") as string,
      danger: true,
      ok: $t("music.settings.clear") as string,
      run: async () => {
        await invoke("music_history_clear");
        showToast("success", $t("music.settings.history_cleared") as string);
      },
    };
  }

  const KEYS: [string, string][] = [
    ["Space", "music.keys.play"],
    ["Shift + ← / →", "music.keys.track"],
    ["← / →", "music.keys.seek"],
    ["↑ / ↓", "music.keys.volume"],
    ["M", "music.keys.mute"],
    ["S", "music.keys.shuffle"],
    ["R", "music.keys.repeat"],
    ["L", "music.keys.like"],
    ["0 – 9", "music.keys.jump"],
    ["/", "music.keys.search"],
    ["Ctrl/⌘ + A", "music.keys.select_all"],
  ];
</script>

<div class="set">
  <h1>{$t("music.nav.settings")}</h1>

  <div class="sec">
    <h2>{$t("music.settings.folders")}</h2>
    <div class="card">
      {#each roots as r (r)}
        <div class="row">
          <MusicIcon name="folder" size={16} />
          <span class="p" title={r}>{r}</span>
          <button class="btn btn-sm btn-ghost" onclick={() => void invoke("reveal_file", { path: r })} aria-label={$t("music.menu.reveal") as string}><MusicIcon name="external" size={13} /></button>
          <button class="btn btn-sm btn-ghost" onclick={() => removeRoot(r)} aria-label={$t("music.remove") as string}><MusicIcon name="trash" size={13} /></button>
        </div>
      {:else}
        <p class="dim">{$t("music.no_folders")}</p>
      {/each}
    </div>
    <div class="acts">
      <button class="btn btn-primary" onclick={() => void addLibraryFolder()}><MusicIcon name="plus" size={14} /> {$t("music.empty.add_folder")}</button>
      <button class="btn" onclick={() => void rescan(false)} disabled={!roots.length || !!lib.scan}><MusicIcon name="refresh" size={14} /> {$t("music.settings.rescan")}</button>
      <button class="btn" onclick={() => void rescan(true)} disabled={!roots.length || !!lib.scan}>{$t("music.settings.full_rescan")}</button>
      {#if lib.scan}
        <span class="dim">{$t("music.scan.progress", { done: lib.scan.done, total: lib.scan.total })}</span>
        <button class="btn btn-sm" onclick={() => void invoke("music_scan_cancel")}>{$t("music.cancel")}</button>
      {/if}
    </div>
    <form
      class="acts"
      onsubmit={(e) => {
        e.preventDefault();
        void addTyped();
      }}
    >
      <input class="input path-in" bind:value={typedPath} placeholder={$t("music.settings.path_placeholder") as string} aria-label={$t("music.settings.path_placeholder") as string} />
      <button class="btn" type="submit" disabled={!typedPath.trim()}>{$t("music.settings.add_path")}</button>
    </form>
    <p class="hint">{$t("music.settings.folders_hint")}</p>
  </div>

  <div class="sec">
    <h2>{$t("music.settings.downloads")}</h2>
    <p class="hint">{$t("music.settings.downloads_hint")} <code>{lib.status?.download_dir}</code></p>
    <p class="hint">{$t("music.settings.format_hint")}</p>
  </div>

  <div class="sec">
    <h2>{$t("music.settings.playback")}</h2>
    <div class="acts">
      <button class="btn" onclick={() => (ui.side = "eq")}><MusicIcon name="eq" size={14} /> {$t("music.settings.open_sound")}</button>
      <button class="btn" onclick={clearHistory}><MusicIcon name="history" size={14} /> {$t("music.settings.clear_history")}</button>
    </div>
    <p class="hint">
      ffmpeg: {#if lib.status?.ffmpeg}<span class="ok">{$t("music.settings.ffmpeg_ok")}</span>{:else}<span class="warn">{$t("music.tools.no_ffmpeg")}</span>{/if}
    </p>
  </div>

  <div class="sec">
    <h2>{$t("music.settings.shortcuts")}</h2>
    <dl class="keys">
      {#each KEYS as [k, label] (k)}
        <dt><kbd>{k}</kbd></dt>
        <dd>{$t(label)}</dd>
      {/each}
    </dl>
  </div>
</div>

<style>
  .set {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    max-width: 760px;
    padding-bottom: var(--space-7);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 750;
  }
  h2 {
    margin: 0;
    font-size: var(--text-xs);
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .sec {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .card {
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    padding: 4px var(--space-2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 4px;
    color: var(--accent-hi);
  }
  .row + .row {
    border-top: var(--hairline) solid var(--content-border);
  }
  .p {
    flex: 1;
    min-width: 0;
    color: var(--text);
    font-size: var(--text-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .path-in {
    flex: 1;
    min-width: 220px;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .dim,
  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .hint {
    font-size: var(--text-xs);
  }
  code {
    font-family: var(--font-mono);
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--fill-2);
  }
  .ok {
    color: var(--success);
  }
  .warn {
    color: var(--warning);
  }
  .keys {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px var(--space-4);
    margin: 0;
    font-size: var(--text-sm);
  }
  dd {
    margin: 0;
    color: var(--text-dim);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--fill-2);
    box-shadow: inset 0 -1px 0 var(--content-border);
  }
</style>
