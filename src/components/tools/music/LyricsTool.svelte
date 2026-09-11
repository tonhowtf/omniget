<script lang="ts">
  /** Letra sincronizada (.lrc) pelo LRCLIB, um arquivo ou uma pasta inteira. */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { baseName, errText, onToolProgress, pct, pickDir, pickFiles, reveal, type ToolProgress } from "$lib/tools/rt";

  type Track = {
    path: string;
    artist: string;
    title: string;
    status: string;
    lrc_path: string | null;
    txt_path: string | null;
    lines: number;
    message: string | null;
  };
  type Result = { total: number; synced: number; plain: number; estimated: number; not_found: number; tracks: Track[] };

  let files = $state<string[]>([]);
  let dir = $state<string | null>(null);
  let outDir = $state<string | null>(null);
  let estimate = $state(false);
  let overwrite = $state(false);
  let writePlain = $state(true);
  let busy = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<Result | null>(null);
  let unlisten: (() => void) | null = null;

  const ready = $derived(files.length > 0 || !!dir);
  const STATUS: Record<string, string> = {
    synced: "tools.lrc.st_synced",
    plain: "tools.lrc.st_plain",
    estimated: "tools.lrc.st_estimated",
    not_found: "tools.lrc.st_not_found",
    skipped: "tools.lrc.st_skipped",
    error: "tools.lrc.st_error",
  };

  onMount(async () => {
    unlisten = await onToolProgress((p) => {
      if (p.id === "music-lyrics") progress = p;
    });
  });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!ready || busy) return;
    busy = true;
    result = null;
    progress = null;
    try {
      result = await invoke<Result>("tool_lyrics_sync", {
        opts: { files, dir, out_dir: outDir, estimate, overwrite, write_plain: writePlain },
      });
      showToast("success", `${result.synced}/${result.total}`);
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = false;
    }
  }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.lrc.files")}</div>
          <div class="group-row-sub mono">{files.map(baseName).join(" · ") || "—"}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if files.length}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (files = [])}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const f = await pickFiles([{ name: "Audio", extensions: ["mp3", "m4a", "flac", "wav", "ogg", "opus", "aac"] }]); if (f.length) files = f; }}>{$t("tools.common.add")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.lrc.folder")}</div>
          <div class="group-row-sub mono">{dir ?? $t("tools.lrc.folder_hint")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if dir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (dir = null)}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) dir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.common.output_folder")}</div>
          <div class="group-row-sub mono">{outDir ?? $t("tools.lrc.beside")}</div>
        </div>
        <div class="group-row-trailing btn-row">
          {#if outDir}<button class="btn btn-ghost btn-sm" type="button" onclick={() => (outDir = null)}>×</button>{/if}
          <button class="btn btn-secondary btn-sm" type="button" onclick={async () => { const d = await pickDir(); if (d) outDir = d; }}>{$t("tools.common.choose")}</button>
        </div>
      </div>
    </div>
  </section>

  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content">
          <div class="group-row-title">{$t("tools.lrc.estimate")}</div>
          <div class="group-row-sub">{$t("tools.lrc.estimate_hint")}</div>
        </div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={estimate} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lrc.write_plain")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={writePlain} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.lrc.overwrite")}</div></div>
        <div class="group-row-trailing"><input type="checkbox" bind:checked={overwrite} /></div>
      </div>
      <div class="group-row">
        <div class="group-row-content">
          {#if busy}
            <div class="group-row-sub mono">{progress?.message ?? "…"}</div>
            <div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>
          {/if}
        </div>
        <div class="group-row-trailing">
          <button class="btn btn-primary" type="button" disabled={busy || !ready} onclick={run}>{busy ? $t("tools.common.working") : $t("tools.lrc.run")}</button>
        </div>
      </div>
    </div>
  </section>

  {#if result}
    <section>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content">
            <div class="group-row-title">{result.synced} {$t("tools.lrc.synced")}</div>
            <div class="group-row-sub">{result.plain} {$t("tools.lrc.plain")} · {result.estimated} {$t("tools.lrc.estimated")} · {result.not_found} {$t("tools.lrc.not_found")}</div>
          </div>
        </div>
        {#each result.tracks as tr (tr.path)}
          <div class="group-row">
            <div class="group-row-content">
              <div class="group-row-sub">
                <span class="tag {tr.status === 'synced' ? 'tag-success' : ''}">{$t(STATUS[tr.status] ?? "tools.lrc.st_error")}</span>
                {tr.artist ? `${tr.artist} — ` : ""}{tr.title}
                {#if tr.lines}<span class="dim">{tr.lines}</span>{/if}
              </div>
              {#if tr.message}<div class="group-row-sub mono">{tr.message}</div>{/if}
            </div>
            <div class="group-row-trailing">
              {#if tr.lrc_path}<button class="btn btn-ghost btn-sm" type="button" onclick={() => reveal(tr.lrc_path!)}>↗</button>{/if}
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .dim { color: var(--text-dim); font-weight: 400; }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
</style>
